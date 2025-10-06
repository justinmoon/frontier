use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as AnyhowContext, Result};
use blitz_dom::net::Resource;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_net::Provider;

use super::environment::JsDomEnvironment;
use super::processor::ScriptExecutionSummary;
use super::script::{ScriptDescriptor, ScriptSource};
use super::script_fetcher::ScriptFetcher;

/// Owns the JavaScript runtime for a page and coordinates script execution.
///
/// This runtime maintains a persistent QuickJS context and owns the BaseDocument
/// that the JS code mutates, keeping them alive across renders.
pub struct JsPageRuntime {
    environment: JsDomEnvironment,
    #[allow(dead_code)]
    document: Box<HtmlDocument>,
    scripts: Vec<ScriptDescriptor>,
    fetched_external: HashMap<usize, String>,
    executed_blocking: bool,
}

impl JsPageRuntime {
    /// Fetch external scripts asynchronously (Send-safe).
    /// This is separated from runtime creation to allow spawning on multi-threaded executors.
    pub async fn fetch_external_scripts(
        scripts: &[ScriptDescriptor],
        base_url: &str,
        net_provider: Arc<Provider<Resource>>,
    ) -> Result<HashMap<usize, String>> {
        let fetcher = ScriptFetcher::new(net_provider);
        let fetched = fetcher.fetch_all_external(scripts, base_url).await?;
        Ok(fetched.into_iter().collect())
    }

    /// Construct a runtime for the supplied HTML/script manifest.
    /// For async workflows, call `fetch_external_scripts` first, then pass the result here.
    #[allow(dead_code)] // Used in tests
    pub fn new_with_fetched(
        html: &str,
        scripts: &[ScriptDescriptor],
        config: DocumentConfig,
        fetched_external: HashMap<usize, String>,
    ) -> Result<Option<Self>> {
        if scripts.is_empty() {
            return Ok(None);
        }

        let environment = JsDomEnvironment::new(html)
            .context("failed to create QuickJS environment for page runtime")?;

        let mut document = Box::new(HtmlDocument::from_html(html, config));
        environment.attach_document(&mut document);

        Ok(Some(Self {
            environment,
            document,
            scripts: scripts.to_vec(),
            fetched_external,
            executed_blocking: false,
        }))
    }

    /// Construct a runtime for the supplied HTML/script manifest and base URL.
    /// Fetches all external scripts during initialization.
    #[allow(dead_code)] // Used in tests
    pub async fn new(
        html: &str,
        scripts: &[ScriptDescriptor],
        config: DocumentConfig,
        net_provider: Option<Arc<Provider<Resource>>>,
    ) -> Result<Option<Self>> {
        if scripts.is_empty() {
            return Ok(None);
        }

        // Extract base URL and fetch scripts
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "about:blank".to_string());

        let fetched_external = if let Some(provider) = net_provider.clone() {
            Self::fetch_external_scripts(scripts, &base_url, provider).await?
        } else {
            HashMap::new()
        };

        // Create runtime with fetched scripts
        Self::new_with_fetched(html, scripts, config, fetched_external)
    }

    /// Execute all classic blocking scripts in document order.
    /// This includes both inline and external scripts that were fetched during initialization.
    pub fn run_blocking_scripts(&mut self) -> Result<Option<ScriptExecutionSummary>> {
        if self.executed_blocking {
            return Ok(None);
        }

        // Collect all blocking classic scripts in document order
        let blocking_scripts: Vec<_> = self
            .scripts
            .iter()
            .filter(|s| {
                s.kind == super::script::ScriptKind::Classic
                    && s.execution == super::script::ScriptExecution::Blocking
            })
            .collect();

        tracing::info!(
            target: "script_exec",
            total_scripts = self.scripts.len(),
            blocking_scripts = blocking_scripts.len(),
            "preparing to execute blocking scripts"
        );

        if blocking_scripts.is_empty() {
            self.executed_blocking = true;
            return Ok(None);
        }

        let mut executed_count = 0;

        // Execute scripts in document order
        for script in blocking_scripts {
            tracing::debug!(
                target: "script_exec",
                index = script.index,
                is_inline = matches!(script.source, ScriptSource::Inline { .. }),
                "processing script"
            );
            let source_code = match &script.source {
                ScriptSource::Inline { code } => code.clone(),
                ScriptSource::External { src } => {
                    if let Some(content) = self.fetched_external.get(&script.index) {
                        content.clone()
                    } else {
                        tracing::warn!(
                            target: "script_exec",
                            index = script.index,
                            src = %src,
                            "external script not fetched, skipping"
                        );
                        continue;
                    }
                }
            };

            let filename = match &script.source {
                ScriptSource::Inline { .. } => format!("inline-{}.js", script.index),
                ScriptSource::External { src } => src.clone(),
            };

            match self.environment.eval(&source_code, &filename) {
                Ok(()) => {
                    executed_count += 1;
                    tracing::debug!(
                        target: "script_exec",
                        index = script.index,
                        filename = %filename,
                        "script executed successfully"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "script_exec",
                        index = script.index,
                        filename = %filename,
                        error = %e,
                        "script execution failed"
                    );
                    // Also print to stdout for debugging
                    eprintln!("❌ Script {} execution failed:", script.index);
                    eprintln!("   File: {}", filename);
                    eprintln!("   Error: {}", e);
                }
            }
        }

        self.executed_blocking = true;
        Ok(Some(ScriptExecutionSummary {
            executed_scripts: executed_count,
            dom_mutations: 0, // TODO: Track mutations from bridge
        }))
    }

    /// Serialize the current document tree managed by the runtime.
    pub fn document_html(&self) -> Result<String> {
        self.environment
            .document_html()
            .context("failed to serialize runtime document")
    }

    /// Evaluate JavaScript code in this runtime's persistent environment.
    #[allow(dead_code)]
    pub fn eval(&self, source: &str, filename: &str) -> Result<()> {
        self.environment
            .eval(source, filename)
            .with_context(|| format!("failed to evaluate {}", filename))
    }

    /// Get a reference to the document owned by this runtime.
    /// The document is mutated directly by JS code through the bridge.
    #[allow(dead_code)]
    pub fn document(&self) -> &HtmlDocument {
        &self.document
    }

    /// Get a mutable reference to the document owned by this runtime.
    #[allow(dead_code)]
    pub fn document_mut(&mut self) -> &mut HtmlDocument {
        &mut self.document
    }

    /// Take ownership of the document from this runtime.
    /// This consumes the runtime and returns the document.
    #[allow(dead_code)]
    pub fn into_document(self) -> Box<HtmlDocument> {
        self.document
    }

    /// Dispatch a DOM event from native GUI (e.g., click events from Blitz)
    pub fn dispatch_event(&mut self, node_id: usize, event_type: &str) -> Result<()> {
        self.environment.dispatch_event(node_id, event_type, "")?;

        // No need to apply mutations - they're already applied to the internal document
        // The caller should call document_html() to get the updated HTML

        Ok(())
    }
}
