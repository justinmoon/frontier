//! High-level automation client for driving the Frontier automation host in tests.
//! Tests should rely on this crate instead of hand-rolling HTTP calls so the API
//! remains stable while the automation host evolves.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::{Client, Response};
use reqwest::Url;
use serde::{Deserialize, Serialize};

pub use crate::automation::full_app::PointerOffset;
pub use crate::automation::{
    ElementSelector, KeyboardAction, PointerAction, PointerButton, PointerTarget,
};

/// Default automation session id – the host currently supports a single active session.
const SESSION_ID: &str = "frontier";

/// Top-level handle that owns the automation host process and HTTP client.
pub struct AutomationHost {
    child: Child,
    _reader: BufReader<std::process::ChildStdout>,
    base_url: Url,
    client: Client,
    artifact_root: PathBuf,
}

impl AutomationHost {
    /// Spawn a fresh automation host process.
    ///
    /// The binary is located via `CARGO_BIN_EXE_automation_host`, so integration tests must
    /// ensure the executable is built (Cargo takes care of this automatically).
    pub fn spawn(config: AutomationHostConfig) -> Result<Self> {
        let AutomationHostConfig {
            bind_address,
            initial_target,
            asset_root,
            artifact_root,
        } = config;
        let binary = match std::env::var("CARGO_BIN_EXE_automation_host") {
            Ok(path) => PathBuf::from(path),
            Err(_) => fallback_automation_host_path()
                .context("automation host binary not built; run cargo test to compile binaries")?,
        };
        let mut command = Command::new(&binary);
        let asset_root = asset_root.unwrap_or_else(default_asset_root);
        let artifact_root = artifact_root.unwrap_or_else(default_artifact_root);
        command
            .env("AUTOMATION_ASSET_ROOT", asset_root.display().to_string())
            .env(
                "AUTOMATION_BIND",
                bind_address.unwrap_or_else(|| "127.0.0.1:0".into()),
            )
            .env(
                "AUTOMATION_INITIAL",
                initial_target.unwrap_or_else(|| "about:blank".to_string()),
            )
            .env(
                "AUTOMATION_ARTIFACT_ROOT",
                artifact_root.display().to_string(),
            )
            .stderr(Stdio::inherit())
            .stdout(Stdio::piped())
            .stdin(Stdio::null());

        let mut child = command.spawn().context("spawn automation host process")?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("automation host stdout unavailable"))?;
        let mut reader = BufReader::new(stdout);
        let mut banner = String::new();
        let addr = loop {
            banner.clear();
            let bytes_read = reader
                .read_line(&mut banner)
                .context("read automation host banner")?;
            if bytes_read == 0 {
                return Err(anyhow!("automation host exited before reporting readiness"));
            }
            let trimmed = banner.trim();
            if let Some(addr) = trimmed.strip_prefix("AUTOMATION_HOST_READY ") {
                break addr.to_string();
            }
            // Forward any early stdout noise to stderr so the caller can diagnose issues.
            if !trimmed.is_empty() {
                eprintln!("automation_host: {trimmed}");
            }
        };

        let base_url =
            Url::parse(&format!("http://{addr}")).context("parse automation base url")?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build automation http client")?;

        std::fs::create_dir_all(&artifact_root)
            .with_context(|| format!("create artifact directory {}", artifact_root.display()))?;

        Ok(Self {
            child,
            _reader: reader,
            base_url,
            client,
            artifact_root,
        })
    }

    /// Create a session by navigating to an asset relative to the asset root.
    pub fn session_from_asset(&self, file: impl AsRef<str>) -> Result<AutomationSession<'_>> {
        let payload = CreateSessionPayload {
            url: None,
            file: Some(file.as_ref().to_string()),
        };
        self.create_session(payload)
    }

    /// Create a session by navigating to an absolute URL.
    pub fn session_from_url(&self, url: impl AsRef<str>) -> Result<AutomationSession<'_>> {
        let payload = CreateSessionPayload {
            url: Some(url.as_ref().to_string()),
            file: None,
        };
        self.create_session(payload)
    }

    fn create_session(&self, payload: CreateSessionPayload) -> Result<AutomationSession<'_>> {
        self.post("/session", &payload)?
            .error_for_status()
            .context("create session response")?;
        Ok(AutomationSession {
            host: self,
            session_id: SESSION_ID.to_string(),
            artifact_dir: self.host_artifact_dir(),
        })
    }

    fn post<T: Serialize>(&self, path: &str, body: &T) -> Result<Response> {
        let url = self.base_url.join(path).context("build request url")?;
        self.client
            .post(url)
            .json(body)
            .send()
            .context("execute automation POST")
    }

    fn get(&self, path: &str) -> Result<Response> {
        let url = self.base_url.join(path).context("build request url")?;
        self.client
            .get(url)
            .send()
            .context("execute automation GET")
    }

    /// Directory where command artifacts should be written. The host populates it on demand.
    pub fn artifact_root(&self) -> &Path {
        &self.artifact_root
    }

    fn host_artifact_dir(&self) -> PathBuf {
        self.artifact_root.join(SESSION_ID)
    }
}

impl Drop for AutomationHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Launch configuration for the automation host.
#[derive(Default)]
pub struct AutomationHostConfig {
    bind_address: Option<String>,
    initial_target: Option<String>,
    asset_root: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
}

impl AutomationHostConfig {
    pub fn with_initial_target(mut self, target: impl Into<String>) -> Self {
        self.initial_target = Some(target.into());
        self
    }

    pub fn with_asset_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.asset_root = Some(path.into());
        self
    }

    pub fn with_artifact_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.artifact_root = Some(path.into());
        self
    }
}

/// Active automation session that exposes higher-level helpers for driving the host.
pub struct AutomationSession<'host> {
    host: &'host AutomationHost,
    session_id: String,
    artifact_dir: PathBuf,
}

impl<'host> AutomationSession<'host> {
    fn post<T: Serialize>(&self, path: &str, payload: &T) -> Result<Response> {
        let full_path = format!(
            "/session/{}/{}",
            self.session_id,
            path.trim_start_matches('/')
        );
        self.host.post(&full_path, payload)
    }

    fn get(&self, path: &str) -> Result<Response> {
        let full_path = format!(
            "/session/{}/{}",
            self.session_id,
            path.trim_start_matches('/')
        );
        self.host.get(&full_path)
    }

    /// Click a selector.
    pub fn click(&self, selector: &ElementSelector) -> Result<()> {
        self.post(
            "click",
            &SelectorPayloadOwned {
                selector: selector.clone(),
            },
        )?
        .error_for_status()
        .context("click response")?;
        Ok(())
    }

    /// Convenience wrapper for clicking using a CSS selector string.
    pub fn click_css(&self, selector: &str) -> Result<()> {
        self.click(&ElementSelector::css(selector.to_string()))
    }

    /// Type text into the given selector (clicks first to focus).
    pub fn type_text(&self, selector: &ElementSelector, text: &str) -> Result<()> {
        self.post(
            "type",
            &TypePayload {
                selector: selector.clone(),
                text: text.to_string(),
            },
        )?
        .error_for_status()
        .context("type response")?;
        Ok(())
    }

    pub fn type_text_css(&self, selector: &str, text: &str) -> Result<()> {
        self.type_text(&ElementSelector::css(selector.to_string()), text)
    }

    /// Navigate to the provided URL.
    pub fn navigate_url(&self, url: &str) -> Result<()> {
        self.post(
            "navigate",
            &NavigatePayload {
                url: Some(url.to_string()),
                file: None,
            },
        )?
        .error_for_status()
        .context("navigate response")?;
        Ok(())
    }

    /// Navigate to an asset relative to the asset root.
    pub fn navigate_asset(&self, file: &str) -> Result<()> {
        self.post(
            "navigate",
            &NavigatePayload {
                url: None,
                file: Some(file.to_string()),
            },
        )?
        .error_for_status()
        .context("navigate response")?;
        Ok(())
    }

    /// Pump the event loop for the specified duration.
    pub fn pump(&self, duration: Duration) -> Result<()> {
        self.post(
            "pump",
            &PumpPayload {
                milliseconds: duration.as_millis() as u64,
            },
        )?
        .error_for_status()
        .context("pump response")?;
        Ok(())
    }

    /// Wait for text to appear on the node identified by `selector`.
    pub fn wait_for_text(&self, selector: &ElementSelector, opts: WaitOptions) -> Result<String> {
        let end = Instant::now() + opts.timeout;
        let encoded = encode_selector_query(selector);
        let path = format!("text?{}", encoded);
        let mut last_error: Option<anyhow::Error> = None;
        while Instant::now() <= end {
            match self.get(&path) {
                Ok(response) if response.status().is_success() => {
                    let parsed: TextResponse =
                        response.json().context("parse text response body")?;
                    return Ok(parsed.value);
                }
                Ok(response) => {
                    last_error = Some(anyhow!("unexpected status {}", response.status()));
                }
                Err(err) => last_error = Some(err),
            }
            self.pump(opts.poll_interval)?;
        }
        Err(last_error.unwrap_or_else(|| anyhow!("wait_for_text timed out")))
    }

    pub fn wait_for_element(&self, selector: &ElementSelector, opts: WaitOptions) -> Result<()> {
        let end = Instant::now() + opts.timeout;
        let encoded = encode_selector_query(selector);
        let path = format!("exists?{}", encoded);
        let mut last_error: Option<anyhow::Error> = None;
        while Instant::now() <= end {
            match self.get(&path) {
                Ok(response) if response.status().is_success() => {
                    let parsed: ExistsResponse =
                        response.json().context("parse exists response")?;
                    if parsed.exists {
                        return Ok(());
                    }
                }
                Ok(response) => {
                    last_error = Some(anyhow!("unexpected status {}", response.status()));
                }
                Err(err) => last_error = Some(err),
            }
            self.pump(opts.poll_interval)?;
        }
        Err(last_error.unwrap_or_else(|| anyhow!("wait_for_element timed out")))
    }

    pub fn pointer_sequence(&self, actions: Vec<PointerAction>) -> Result<()> {
        self.post("pointer", &PointerPayload { actions })?
            .error_for_status()
            .context("pointer sequence response")?;
        Ok(())
    }

    pub fn keyboard_sequence(&self, actions: Vec<KeyboardAction>) -> Result<()> {
        self.post("keyboard", &KeyboardPayload { actions })?
            .error_for_status()
            .context("keyboard sequence response")?;
        Ok(())
    }

    pub fn focus(&self, selector: &ElementSelector) -> Result<()> {
        self.post(
            "focus",
            &SelectorPayloadOwned {
                selector: selector.clone(),
            },
        )?
        .error_for_status()
        .context("focus response")?;
        Ok(())
    }

    pub fn scroll_into_view(&self, selector: &ElementSelector) -> Result<()> {
        self.post(
            "scroll",
            &SelectorPayloadOwned {
                selector: selector.clone(),
            },
        )?
        .error_for_status()
        .context("scroll response")?;
        Ok(())
    }

    pub fn artifact_dir(&self) -> &Path {
        &self.artifact_dir
    }
}

/// Wait configuration shared by helpers.
#[derive(Clone, Copy)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl WaitOptions {
    pub fn new(timeout: Duration, poll_interval: Duration) -> Self {
        Self {
            timeout,
            poll_interval,
        }
    }

    pub fn default_text_wait() -> Self {
        Self::new(Duration::from_secs(5), Duration::from_millis(200))
    }
}

#[derive(Serialize)]
struct SelectorPayloadOwned {
    selector: ElementSelector,
}

#[derive(Serialize)]
struct TypePayload {
    selector: ElementSelector,
    text: String,
}

#[derive(Serialize)]
struct PumpPayload {
    milliseconds: u64,
}

#[derive(Serialize)]
struct NavigatePayload {
    url: Option<String>,
    file: Option<String>,
}

#[derive(Deserialize)]
struct TextResponse {
    value: String,
}

#[derive(Deserialize)]
struct ExistsResponse {
    exists: bool,
}

#[derive(Serialize)]
struct PointerPayload {
    actions: Vec<PointerAction>,
}

#[derive(Serialize)]
struct KeyboardPayload {
    actions: Vec<KeyboardAction>,
}

fn encode_selector_query(selector: &ElementSelector) -> String {
    let mut params: Vec<(String, String)> = Vec::new();
    match selector {
        ElementSelector::Css { selector } => {
            params.push(("kind".into(), "css".into()));
            params.push(("selector".into(), selector.clone()));
        }
        ElementSelector::Role { role, name } => {
            params.push(("kind".into(), "role".into()));
            params.push(("role".into(), role.clone()));
            if let Some(name) = name {
                params.push(("name".into(), name.clone()));
            }
        }
    }
    serde_urlencoded::to_string(params).expect("serialize selector query")
}

#[derive(Serialize)]
struct CreateSessionPayload {
    url: Option<String>,
    file: Option<String>,
}

fn default_asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn default_artifact_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("automation-artifacts")
}

fn fallback_automation_host_path() -> anyhow::Result<PathBuf> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push(if cfg!(windows) {
        "automation_host.exe"
    } else {
        "automation_host"
    });
    if path.exists() {
        Ok(path)
    } else {
        Err(anyhow!(
            "automation host binary not found at {}",
            path.display()
        ))
    }
}
