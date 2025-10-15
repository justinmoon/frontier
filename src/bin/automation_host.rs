#![allow(clippy::disallowed_types)]

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use frontier::automation::full_app::{AutomationState, AutomationTask};
use frontier::automation::{
    AutomationCommand, AutomationEvent, AutomationResponse, AutomationResult,
    AutomationStateHandle, ElementSelector, KeyboardAction, PointerAction,
};
use frontier::{create_default_event_loop, wrap_with_url_bar, ReadmeApplication};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::info;
use url::Url;
use winit::event_loop::EventLoopProxy;

use blitz_net::Provider;
use blitz_shell::{BlitzShellEvent, BlitzShellNetCallback, WindowConfig};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use frontier::navigation::{execute_fetch, prepare_navigation, FetchedDocument, NavigationPlan};
use frontier::WindowRenderer;

const SESSION_ID: &str = "frontier";

#[derive(Clone)]
struct HostState {
    automation: AutomationStateHandle,
    proxy: EventLoopProxy<BlitzShellEvent>,
    asset_root: PathBuf,
    session_active: Arc<Mutex<bool>>,
    artifact_root: PathBuf,
    command_counter: Arc<Mutex<u64>>,
    session_artifacts: Arc<Mutex<Option<PathBuf>>>,
}

impl HostState {
    fn prepare_session_artifacts(&self, session_id: &str) -> Result<(), StatusCode> {
        let dir = self.artifact_root.join(session_id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
        std::fs::create_dir_all(&dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        *self.command_counter.lock().unwrap() = 0;
        *self.session_artifacts.lock().unwrap() = Some(dir);
        Ok(())
    }

    fn next_artifact_path(&self, label: &str) -> Option<PathBuf> {
        let mut counter = self.command_counter.lock().unwrap();
        let session_dir = {
            let guard = self.session_artifacts.lock().unwrap();
            guard.clone()
        }?;
        let index = *counter;
        *counter += 1;
        let sanitized = label
            .chars()
            .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
            .collect::<String>();
        Some(session_dir.join(format!("{index:04}_{}", sanitized)))
    }
}

#[derive(Deserialize)]
struct CreateSessionPayload {
    url: Option<String>,
    file: Option<String>,
}

#[derive(Serialize)]
struct CreateSessionResponse {
    session_id: String,
}

#[derive(Deserialize)]
struct SelectorBody {
    selector: ElementSelector,
}

#[derive(Deserialize)]
struct TypePayload {
    selector: ElementSelector,
    text: String,
}

#[derive(Deserialize)]
struct PumpPayload {
    milliseconds: u64,
}

#[derive(Deserialize)]
struct NavigatePayload {
    url: Option<String>,
    file: Option<String>,
}

#[derive(Deserialize)]
struct TextQuery {
    kind: Option<String>,
    selector: Option<String>,
    role: Option<String>,
    name: Option<String>,
}

#[derive(Serialize)]
struct TextResponse {
    value: String,
}

#[derive(Serialize)]
struct ExistsResponse {
    exists: bool,
}

#[derive(Deserialize)]
struct PointerPayload {
    actions: Vec<PointerAction>,
}

#[derive(Deserialize)]
struct KeyboardPayload {
    actions: Vec<KeyboardAction>,
}

impl TextQuery {
    fn into_selector(self) -> Result<ElementSelector, StatusCode> {
        match self.kind.as_deref() {
            Some("css") => self
                .selector
                .map(ElementSelector::css)
                .ok_or(StatusCode::BAD_REQUEST),
            Some("role") => self
                .role
                .map(|role| ElementSelector::role(role, self.name))
                .ok_or(StatusCode::BAD_REQUEST),
            Some(_) => Err(StatusCode::BAD_REQUEST),
            None => {
                if let Some(selector) = self.selector {
                    Ok(ElementSelector::css(selector))
                } else if let Some(role) = self.role {
                    Ok(ElementSelector::role(role, self.name))
                } else {
                    Err(StatusCode::BAD_REQUEST)
                }
            }
        }
    }
}

fn main() -> Result<()> {
    setup_tracing();

    let config = HostConfig::from_env()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("automation host runtime")?;
    let _guard = runtime.enter();

    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();

    let automation_state = Arc::new(AutomationState::new());
    let _ = std::fs::create_dir_all(&config.artifact_root);
    let host_state = HostState {
        automation: Arc::clone(&automation_state),
        proxy: proxy.clone(),
        asset_root: config.asset_root.clone(),
        session_active: Arc::new(Mutex::new(false)),
        artifact_root: config.artifact_root.clone(),
        command_counter: Arc::new(Mutex::new(0)),
        session_artifacts: Arc::new(Mutex::new(None)),
    };

    // Spin up HTTP server after binding listener
    let (server_ready_tx, server_ready_rx) = oneshot::channel::<Result<SocketAddr>>();
    runtime.spawn(start_http_server(
        config.bind_addr,
        host_state.clone(),
        server_ready_tx,
    ));
    let bound_addr = runtime
        .block_on(server_ready_rx)
        .context("automation server failed to bind")??;
    println!("AUTOMATION_HOST_READY {bound_addr}");

    // Prepare initial application state on the current thread (main)
    let net_callback = BlitzShellNetCallback::shared(proxy.clone());
    let net_provider = Arc::new(Provider::new(net_callback));

    let initial_plan = runtime
        .block_on(prepare_navigation(&config.initial_target))
        .context("prepare initial navigation")?;

    let initial_document = match initial_plan {
        NavigationPlan::Fetch(request) => {
            match runtime.block_on(execute_fetch(&request, Arc::clone(&net_provider))) {
                Ok(doc) => doc,
                Err(err) => {
                    tracing::error!(
                        target = "automation_host",
                        error = %err,
                        target = %config.initial_target,
                        "failed to load initial document, falling back to blank"
                    );
                    fallback_document(&config.initial_target)
                }
            }
        }
    };

    let navigation_provider: Arc<dyn NavigationProvider> = Arc::new(MainNavigationProvider {
        proxy: proxy.clone(),
    });

    let initial_input = config.initial_target.clone();
    let mut application = ReadmeApplication::new(
        proxy.clone(),
        initial_input,
        Arc::clone(&net_provider),
        navigation_provider,
    );
    application.attach_automation(Arc::clone(&automation_state));
    application.prepare_initial_state(initial_document);

    let document = application.take_initial_document();
    let renderer = WindowRenderer::new();
    let attrs = winit::window::WindowAttributes::default()
        .with_visible(false)
        .with_decorations(false)
        .with_title("Frontier Automation Host");
    let window = WindowConfig::with_attributes(document, renderer, attrs);
    application.add_window(window);

    info!(target: "automation_host", %bound_addr, "host ready");

    event_loop
        .run_app(&mut application)
        .expect("automation host event loop");

    Ok(())
}

fn setup_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
}

struct HostConfig {
    bind_addr: SocketAddr,
    initial_target: String,
    asset_root: PathBuf,
    artifact_root: PathBuf,
}

impl HostConfig {
    fn from_env() -> Result<Self> {
        let bind_addr = std::env::var("AUTOMATION_BIND")
            .unwrap_or_else(|_| "127.0.0.1:0".into())
            .parse::<SocketAddr>()
            .context("parse AUTOMATION_BIND")?;

        let initial_target =
            std::env::var("AUTOMATION_INITIAL").unwrap_or_else(|_| "about:blank".into());

        Ok(Self {
            bind_addr,
            initial_target,
            asset_root: std::env::var("AUTOMATION_ASSET_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")),
            artifact_root: std::env::var("AUTOMATION_ARTIFACT_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("target")
                        .join("automation-artifacts")
                }),
        })
    }
}

struct MainNavigationProvider {
    proxy: EventLoopProxy<BlitzShellEvent>,
}

impl NavigationProvider for MainNavigationProvider {
    fn navigate_to(&self, opts: NavigationOptions) {
        let _ = self
            .proxy
            .send_event(BlitzShellEvent::Navigate(Box::new(opts)));
    }
}

async fn start_http_server(
    bind_addr: SocketAddr,
    host_state: HostState,
    ready_tx: oneshot::Sender<Result<SocketAddr>>,
) {
    let listener = match TcpListener::bind(bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            let _ = ready_tx.send(Err(anyhow!(err)));
            return;
        }
    };
    let actual_addr = listener.local_addr().expect("listener address");
    let _ = ready_tx.send(Ok(actual_addr));

    let app = Router::new()
        .route("/session", post(create_session))
        .route("/session/:id/click", post(click_element))
        .route("/session/:id/type", post(type_text))
        .route("/session/:id/pump", post(pump_session))
        .route("/session/:id/text", get(get_text))
        .route("/session/:id/exists", get(element_exists))
        .route("/session/:id/navigate", post(navigate_to))
        .route("/session/:id/pointer", post(pointer_sequence))
        .route("/session/:id/keyboard", post(keyboard_sequence))
        .route("/session/:id/focus", post(focus_element))
        .route("/session/:id/scroll", post(scroll_element))
        .with_state(host_state);

    if let Err(err) = axum::serve(listener, app).await {
        tracing::error!(target = "automation_host", error = %err, "server error");
    }
}

async fn create_session(
    State(state): State<HostState>,
    Json(payload): Json<CreateSessionPayload>,
) -> Result<Json<CreateSessionResponse>, StatusCode> {
    {
        let mut guard = state.session_active.lock().unwrap();
        if *guard {
            return Err(StatusCode::BAD_REQUEST);
        }
        *guard = true;
    }

    if let Some(target) = resolve_target(&state.asset_root, payload.url, payload.file)? {
        send_command(&state, AutomationCommand::Navigate { target })
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    state
        .prepare_session_artifacts(SESSION_ID)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CreateSessionResponse {
        session_id: SESSION_ID.into(),
    }))
}

async fn click_element(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<SelectorBody>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::Click {
            selector: payload.selector,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn type_text(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<TypePayload>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::TypeText {
            selector: payload.selector,
            text: payload.text,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pump_session(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<PumpPayload>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::Pump {
            duration_ms: payload.milliseconds,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn navigate_to(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<NavigatePayload>,
) -> Result<StatusCode, StatusCode> {
    let target = resolve_target(&state.asset_root, payload.url, payload.file)?
        .ok_or(StatusCode::BAD_REQUEST)?;
    send_command(&state, AutomationCommand::Navigate { target })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pointer_sequence(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<PointerPayload>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::PointerSequence {
            actions: payload.actions,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn keyboard_sequence(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<KeyboardPayload>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::KeyboardSequence {
            actions: payload.actions,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn focus_element(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<SelectorBody>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::Focus {
            selector: payload.selector,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn scroll_element(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Json(payload): Json<SelectorBody>,
) -> Result<StatusCode, StatusCode> {
    send_command(
        &state,
        AutomationCommand::ScrollIntoView {
            selector: payload.selector,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

fn command_label(command: &AutomationCommand) -> &'static str {
    match command {
        AutomationCommand::Click { .. } => "click",
        AutomationCommand::TypeText { .. } => "type",
        AutomationCommand::GetText { .. } => "get_text",
        AutomationCommand::ElementExists { .. } => "exists",
        AutomationCommand::Pump { .. } => "pump",
        AutomationCommand::Navigate { .. } => "navigate",
        AutomationCommand::PointerSequence { .. } => "pointer",
        AutomationCommand::KeyboardSequence { .. } => "keyboard",
        AutomationCommand::Focus { .. } => "focus",
        AutomationCommand::ScrollIntoView { .. } => "scroll",
        AutomationCommand::Shutdown => "shutdown",
    }
}

fn persist_artifacts(
    path: &Path,
    command: &AutomationCommand,
    result: &AutomationResult,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::write(path.join("command.txt"), format!("{command:#?}"))?;

    match result {
        Ok(reply) => {
            std::fs::write(
                path.join("reply.json"),
                serde_json::to_string_pretty(reply)?,
            )?;
            if let Some(artifacts) = &reply.artifacts {
                if let Some(dom) = &artifacts.dom_html {
                    std::fs::write(path.join("dom.html"), dom)?;
                }
            }
        }
        Err(err) => {
            std::fs::write(path.join("error.txt"), format!("{err:#}"))?;
        }
    }

    Ok(())
}

async fn get_text(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Query(query): Query<TextQuery>,
) -> Result<Json<TextResponse>, StatusCode> {
    let selector = query.into_selector()?;
    let reply = send_command(&state, AutomationCommand::GetText { selector })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let AutomationResponse::Text(value) = reply.response else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok(Json(TextResponse { value }))
}

async fn element_exists(
    State(state): State<HostState>,
    AxumPath(_id): AxumPath<String>,
    Query(query): Query<TextQuery>,
) -> Result<Json<ExistsResponse>, StatusCode> {
    let selector = query.into_selector()?;
    let reply = send_command(&state, AutomationCommand::ElementExists { selector })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let AutomationResponse::Bool(exists) = reply.response else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok(Json(ExistsResponse { exists }))
}

async fn send_command(state: &HostState, command: AutomationCommand) -> AutomationResult {
    eprintln!("AUTOMATION_CMD queue {:?}", command);
    let label = command_label(&command);
    let artifact_path = state.next_artifact_path(label);

    let (tx, rx) = oneshot::channel();
    state
        .automation
        .enqueue(AutomationTask::new(command.clone(), tx));
    state
        .proxy
        .send_event(BlitzShellEvent::Embedder(Arc::new(AutomationEvent)))
        .map_err(|_| anyhow!("event loop closed"))?;

    let result = rx
        .await
        .map_err(|_| anyhow!("automation response dropped"))?;

    if let Some(path) = artifact_path {
        if let Err(err) = persist_artifacts(&path, &command, &result) {
            tracing::warn!(
                target = "automation_host",
                error = %err,
                artifact = %path.display(),
                "failed to persist automation artifacts"
            );
        }
    }

    eprintln!("AUTOMATION_CMD done {:?} -> {:?}", command, result);
    result
}

fn resolve_target(
    asset_root: &Path,
    url: Option<String>,
    file: Option<String>,
) -> Result<Option<String>, StatusCode> {
    if let Some(url) = url {
        return Ok(Some(url));
    }

    if let Some(file) = file {
        let joined = asset_root.join(file);
        let url = Url::from_file_path(&joined)
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .to_string();
        return Ok(Some(url));
    }

    Ok(None)
}

fn fallback_document(target: &str) -> FetchedDocument {
    let content = "<main id=\"content\"></main>";
    let wrapped = wrap_with_url_bar(content, target, None);
    FetchedDocument {
        base_url: target.to_string(),
        contents: wrapped,
        file_path: None,
        display_url: target.to_string(),
        scripts: Vec::new(),
    }
}
