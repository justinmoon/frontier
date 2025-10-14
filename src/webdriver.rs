use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::automation::{HeadlessSession, HeadlessSessionBuilder};

#[derive(Clone)]
pub struct WebDriverConfig {
    pub asset_root: PathBuf,
}

impl Default for WebDriverConfig {
    fn default() -> Self {
        Self {
            asset_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        }
    }
}

pub struct WebDriverHandle {
    pub addr: SocketAddr,
    shutdown_tx: oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
}

impl WebDriverHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.server_handle.await;
    }
}

pub async fn start_webdriver(addr: SocketAddr, config: WebDriverConfig) -> Result<WebDriverHandle> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<CommandMessage>(32);
    spawn_worker(config.clone(), cmd_rx);

    let state = Arc::new(WebDriverState { command_tx: cmd_tx });

    let router = Router::new()
        .route("/session", post(create_session))
        .route(
            "/session/:id/url",
            get(get_session_url).post(navigate_session),
        )
        .route("/session/:id", delete(delete_session))
        .route("/session/:id/source", get(get_session_source))
        .route("/session/:id/element", post(find_element))
        .route("/session/:id/element/:element/click", post(click_element))
        .route("/session/:id/element/:element/text", post(element_text))
        .route("/session/:id/frontier/pump", post(pump_session))
        .with_state(state);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let graceful =
        axum::serve(listener, router.into_make_service()).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });

    let handle = tokio::spawn(async move {
        if let Err(err) = graceful.await {
            tracing::error!(target = "webdriver", error = %err, "webdriver server error");
        }
    });

    Ok(WebDriverHandle {
        addr: local_addr,
        shutdown_tx,
        server_handle: handle,
    })
}

fn spawn_worker(config: WebDriverConfig, mut rx: mpsc::Receiver<CommandMessage>) {
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("webdriver worker runtime");
        runtime.block_on(async move {
            let mut sessions: HashMap<Uuid, SessionEntry> = HashMap::new();
            while let Some(message) = rx.recv().await {
                let response =
                    handle_command(&config.asset_root, &mut sessions, message.command).await;
                let _ = message.respond_to.send(response);
            }
        });
    });
}

async fn handle_command(
    asset_root: &Path,
    sessions: &mut HashMap<Uuid, SessionEntry>,
    command: Command,
) -> Result<serde_json::Value, String> {
    match command {
        Command::CreateSession(target) => {
            let session = open_target(asset_root, &target)
                .await
                .map_err(|err| err.to_string())?;
            let id = Uuid::new_v4();
            sessions.insert(id, SessionEntry::new(session));
            Ok(json!({
                "sessionId": id.to_string(),
                "capabilities": {
                    "browserName": "frontier",
                    "frontier:headless": true
                }
            }))
        }
        Command::Navigate { session_id, target } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            session
                .session
                .navigate_to_target(asset_root, &target)
                .await
                .map_err(|e| e.to_string())?;
            session.elements.clear();
            Ok(json!(null))
        }
        Command::FindElement {
            session_id,
            selector,
        } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            session
                .session
                .ensure_selector(&selector)
                .map_err(|e| e.to_string())?;
            let element_id = Uuid::new_v4().to_string();
            session.elements.insert(element_id.clone(), selector);
            Ok(json!({"element-6066-11e4-a52e-4f735466cecf": element_id}))
        }
        Command::ClickElement {
            session_id,
            element,
        } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            let selector = session
                .elements
                .get(&element)
                .cloned()
                .ok_or_else(|| "unknown element".to_string())?;
            session
                .session
                .click(&selector)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!(null))
        }
        Command::ElementText {
            session_id,
            element,
        } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            let selector = session
                .elements
                .get(&element)
                .cloned()
                .ok_or_else(|| "unknown element".to_string())?;
            let text = session
                .session
                .inner_text(&selector)
                .map_err(|e| e.to_string())?;
            Ok(json!(text))
        }
        Command::GetUrl { session_id } => {
            let session = sessions
                .get(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            Ok(json!(session.session.current_url().to_string()))
        }
        Command::GetSource { session_id } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            let html = session.session.document_html().map_err(|e| e.to_string())?;
            Ok(json!(html))
        }
        Command::DeleteSession { session_id } => {
            sessions.remove(&session_id);
            Ok(json!(null))
        }
        Command::Pump {
            session_id,
            duration,
        } => {
            let session = sessions
                .get_mut(&session_id)
                .ok_or_else(|| "unknown session".to_string())?;
            session.session.pump_for(duration).await;
            Ok(json!(null))
        }
    }
}

async fn open_target(asset_root: &Path, target: &SessionTarget) -> Result<HeadlessSession> {
    match target {
        SessionTarget::Url(url) => HeadlessSession::navigate(url)
            .await
            .map_err(|err| anyhow!("failed to navigate: {err}")),
        SessionTarget::File(path) => {
            let builder = HeadlessSessionBuilder::default().with_base_dir(asset_root.to_path_buf());
            builder
                .open_file(path)
                .await
                .map_err(|err| anyhow!("failed to open file {path}: {err}"))
        }
    }
}

#[derive(Clone)]
struct WebDriverState {
    command_tx: mpsc::Sender<CommandMessage>,
}

#[derive(Clone, Deserialize)]
struct NewSessionPayload {
    url: Option<String>,
    file: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn create_session(
    State(state): State<Arc<WebDriverState>>,
    Json(payload): Json<NewSessionPayload>,
) -> Response {
    let target = match session_target_from_payload(&payload) {
        Ok(target) => target,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"value": ErrorResponse { error: err }})),
            )
                .into_response();
        }
    };
    send_command(&state.command_tx, Command::CreateSession(target)).await
}

#[derive(Deserialize)]
struct NavigatePayload {
    url: String,
}

async fn navigate_session(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<NavigatePayload>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(
        &state.command_tx,
        Command::Navigate {
            session_id,
            target: SessionTarget::Url(payload.url),
        },
    )
    .await
}

async fn get_session_url(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(&state.command_tx, Command::GetUrl { session_id }).await
}

async fn get_session_source(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(&state.command_tx, Command::GetSource { session_id }).await
}

#[derive(Deserialize)]
struct FindElementPayload {
    using: String,
    value: String,
}

async fn find_element(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<FindElementPayload>,
) -> Response {
    if payload.using != "css selector" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"value": ErrorResponse { error: "unsupported locator".into() }})),
        )
            .into_response();
    }
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(
        &state.command_tx,
        Command::FindElement {
            session_id,
            selector: payload.value,
        },
    )
    .await
}

async fn click_element(
    State(state): State<Arc<WebDriverState>>,
    AxumPath((id, element)): AxumPath<(String, String)>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(
        &state.command_tx,
        Command::ClickElement {
            session_id,
            element,
        },
    )
    .await
}

async fn element_text(
    State(state): State<Arc<WebDriverState>>,
    AxumPath((id, element)): AxumPath<(String, String)>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(
        &state.command_tx,
        Command::ElementText {
            session_id,
            element,
        },
    )
    .await
}

async fn delete_session(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    send_command(&state.command_tx, Command::DeleteSession { session_id }).await
}

#[derive(Deserialize)]
struct PumpPayload {
    milliseconds: u64,
}

async fn pump_session(
    State(state): State<Arc<WebDriverState>>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<PumpPayload>,
) -> Response {
    let session_id = match Uuid::parse_str(&id) {
        Ok(session_id) => session_id,
        Err(_) => return invalid_session_response(&id),
    };
    let duration = Duration::from_millis(payload.milliseconds);
    send_command(
        &state.command_tx,
        Command::Pump {
            session_id,
            duration,
        },
    )
    .await
}

fn session_target_from_payload(payload: &NewSessionPayload) -> Result<SessionTarget, String> {
    if let Some(url) = payload.url.clone() {
        Ok(SessionTarget::Url(url))
    } else if let Some(file) = payload.file.clone() {
        Ok(SessionTarget::File(file))
    } else {
        Err("missing 'url' or 'file' in request".into())
    }
}

async fn send_command(tx: &mpsc::Sender<CommandMessage>, command: Command) -> Response {
    let (respond_to, rx) = oneshot::channel();
    if tx
        .send(CommandMessage {
            command,
            respond_to,
        })
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    match rx.await {
        Ok(Ok(value)) => Json(json!({"value": value})).into_response(),
        Ok(Err(err)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"value": ErrorResponse { error: err }})),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn invalid_session_response(id: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"value": ErrorResponse { error: format!("invalid session id: {id}") } })),
    )
        .into_response()
}

enum SessionTarget {
    Url(String),
    File(String),
}

enum Command {
    CreateSession(SessionTarget),
    Navigate {
        session_id: Uuid,
        target: SessionTarget,
    },
    FindElement {
        session_id: Uuid,
        selector: String,
    },
    ClickElement {
        session_id: Uuid,
        element: String,
    },
    ElementText {
        session_id: Uuid,
        element: String,
    },
    GetUrl {
        session_id: Uuid,
    },
    GetSource {
        session_id: Uuid,
    },
    DeleteSession {
        session_id: Uuid,
    },
    Pump {
        session_id: Uuid,
        duration: Duration,
    },
}

struct CommandMessage {
    command: Command,
    respond_to: oneshot::Sender<Result<serde_json::Value, String>>,
}

struct SessionEntry {
    session: HeadlessSession,
    elements: HashMap<String, String>,
}

impl SessionEntry {
    fn new(session: HeadlessSession) -> Self {
        Self {
            session,
            elements: HashMap::new(),
        }
    }
}

impl HeadlessSession {
    async fn navigate_to_target(
        &mut self,
        asset_root: &Path,
        target: &SessionTarget,
    ) -> Result<(), anyhow::Error> {
        match target {
            SessionTarget::Url(url) => self.navigate_to(url).await.map_err(|e| anyhow!(e)),
            SessionTarget::File(path) => {
                let builder =
                    HeadlessSessionBuilder::default().with_base_dir(asset_root.to_path_buf());
                let mut new_session = builder.open_file(path).await?;
                std::mem::swap(self, &mut new_session);
                Ok(())
            }
        }
    }
}
