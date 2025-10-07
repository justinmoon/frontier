use std::env;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use blitz_dom::BaseDocument;
use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use tracing::{error, warn};
use url::Url;

use super::environment::JsDomEnvironment;
use super::processor::ScriptExecutionSummary;
use super::script::{ScriptDescriptor, ScriptExecution, ScriptKind, ScriptSource};

/// Owns the JavaScript runtime for a page and coordinates script execution.
pub struct JsPageRuntime {
    environment: Rc<JsDomEnvironment>,
    scripts: Vec<ScriptDescriptor>,
    base_url: Option<Url>,
    executed_blocking: bool,
    bridge_attached: bool,
}

impl JsPageRuntime {
    /// Construct a runtime for the supplied HTML/script manifest.
    pub fn new(
        html: &str,
        scripts: &[ScriptDescriptor],
        base_url: Option<&str>,
    ) -> Result<Option<Self>> {
        if scripts.is_empty() {
            return Ok(None);
        }

        let environment = JsDomEnvironment::new(html)
            .context("failed to create QuickJS environment for page runtime")?;

        let base_url = base_url.and_then(|raw| match Url::parse(raw) {
            Ok(url) => Some(url),
            Err(err) => {
                warn!(
                    target = "quickjs",
                    base = %raw,
                    error = %err,
                    "failed to parse base URL for page runtime"
                );
                None
            }
        });

        Ok(Some(Self {
            environment: Rc::new(environment),
            scripts: scripts.to_vec(),
            base_url,
            executed_blocking: false,
            bridge_attached: false,
        }))
    }

    /// Execute all classic blocking scripts in document order.
    pub fn run_blocking_scripts(&mut self) -> Result<Option<ScriptExecutionSummary>> {
        if self.executed_blocking {
            return Ok(None);
        }

        let mut executed = 0usize;
        let mut saw_blocking = false;

        for descriptor in self.scripts.iter().filter(|descriptor| {
            descriptor.execution == ScriptExecution::Blocking
                && descriptor.kind == ScriptKind::Classic
        }) {
            saw_blocking = true;
            match self.evaluate_blocking_script(descriptor) {
                Ok(()) => executed += 1,
                Err(err) => {
                    error!(
                        target = "quickjs",
                        script_index = descriptor.index,
                        source = ?descriptor.source,
                        error = %err,
                        "blocking script execution failed"
                    );
                }
            }
        }

        if !saw_blocking {
            self.executed_blocking = true;
            return Ok(None);
        }

        self.environment.pump()?;
        let dom_mutations = self.environment.drain_mutations().len();
        self.executed_blocking = true;
        Ok(Some(ScriptExecutionSummary {
            executed_scripts: executed,
            dom_mutations,
        }))
    }

    fn evaluate_blocking_script(&self, descriptor: &ScriptDescriptor) -> Result<()> {
        match &descriptor.source {
            ScriptSource::Inline { code } => {
                let filename = format!("inline-script-{}.js", descriptor.index);
                self.environment.eval(code, &filename)
            }
            ScriptSource::External { src } => {
                let (code, filename) = self.load_external_script(src)?;
                self.environment.eval(&code, &filename)
            }
        }
    }

    fn load_external_script(&self, src: &str) -> Result<(String, String)> {
        let url = self.resolve_script_url(src)?;
        match url.scheme() {
            "file" => self.read_script_from_file(&url),
            "http" | "https" => self.fetch_script_over_http(&url),
            "data" => self.decode_data_url(&url),
            other => Err(anyhow!("unsupported script scheme: {other}")),
        }
    }

    fn resolve_script_url(&self, src: &str) -> Result<Url> {
        if src.trim().is_empty() {
            return Err(anyhow!("script src attribute cannot be empty"));
        }

        match Url::parse(src) {
            Ok(url) => Ok(url),
            Err(_) => {
                if let Some(base) = &self.base_url {
                    if let Ok(joined) = base.join(src) {
                        return Ok(joined);
                    }
                }
                self.path_to_file_url(src)
            }
        }
    }

    fn path_to_file_url(&self, src: &str) -> Result<Url> {
        let path = Path::new(src);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(base) = &self.base_url {
            if base.scheme() == "file" {
                match base.to_file_path() {
                    Ok(mut base_path) => {
                        base_path.pop();
                        base_path.push(path);
                        base_path
                    }
                    Err(_) => env::current_dir()
                        .context("resolving relative script path")?
                        .join(path),
                }
            } else {
                env::current_dir()
                    .context("resolving relative script path")?
                    .join(path)
            }
        } else {
            env::current_dir()
                .context("resolving relative script path")?
                .join(path)
        };

        Url::from_file_path(&candidate).map_err(|_| {
            anyhow!(
                "failed to convert script path '{}' into a file URL",
                candidate.display()
            )
        })
    }

    fn read_script_from_file(&self, url: &Url) -> Result<(String, String)> {
        let path = url
            .to_file_path()
            .map_err(|_| anyhow!("invalid file URL for script: {url}"))?;
        let code = fs::read_to_string(&path)
            .with_context(|| format!("reading external script {}", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| path.display().to_string());
        Ok((code, filename))
    }

    fn fetch_script_over_http(&self, url: &Url) -> Result<(String, String)> {
        let client = Client::builder()
            .build()
            .context("building HTTP client for external script")?;
        let response = client
            .get(url.clone())
            .send()
            .with_context(|| format!("fetching external script {}", url))?
            .error_for_status()
            .with_context(|| format!("fetching external script {}", url))?;
        let code = response
            .text()
            .with_context(|| format!("reading external script body {}", url))?;
        Ok((code, url.to_string()))
    }

    fn decode_data_url(&self, url: &Url) -> Result<(String, String)> {
        let raw = url.as_str();
        let without_scheme = raw
            .strip_prefix("data:")
            .ok_or_else(|| anyhow!("invalid data URL: {raw}"))?;
        let (metadata, payload) = without_scheme
            .split_once(',')
            .ok_or_else(|| anyhow!("data URL missing payload: {raw}"))?;
        let is_base64 = metadata.ends_with(";base64");
        let mime_type = metadata.trim_end_matches(";base64");

        let decoded_bytes = if is_base64 {
            let normalized = payload.replace('\n', "");
            BASE64_STANDARD
                .decode(normalized.as_bytes())
                .with_context(|| format!("decoding base64 data URL {raw}"))?
        } else {
            percent_decode_str(payload)
                .decode_utf8()
                .with_context(|| format!("percent-decoding data URL {raw}"))?
                .into_owned()
                .into_bytes()
        };

        let code = String::from_utf8(decoded_bytes)
            .with_context(|| format!("data URL payload is not UTF-8: {raw}"))?;

        let filename = if mime_type.is_empty() {
            "data:application/javascript".to_string()
        } else {
            format!("data:{mime_type}")
        };
        Ok((code, filename))
    }

    /// Serialize the current document tree managed by the runtime.
    #[allow(dead_code)]
    pub fn document_html(&self) -> Result<String> {
        self.environment
            .document_html()
            .context("failed to serialize runtime document")
    }

    /// Attach the runtime to the live Blitz document so subsequent mutations
    /// operate on the rendered tree.
    pub fn attach_document(&mut self, document: &mut BaseDocument) {
        if self.bridge_attached {
            return;
        }
        self.environment.attach_document(document);
        self.bridge_attached = true;
    }

    pub fn environment(&self) -> Rc<JsDomEnvironment> {
        Rc::clone(&self.environment)
    }
}
