use anyhow::{Context as AnyhowContext, Result};
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

use super::environment::JsDomEnvironment;
use super::processor::{filter_inline_classic, run_inline_scripts, ScriptExecutionSummary};
use super::script::ScriptDescriptor;

/// Owns the JavaScript runtime for a page and coordinates script execution.
///
/// This runtime maintains a persistent QuickJS context and owns the BaseDocument
/// that the JS code mutates, keeping them alive across renders.
pub struct JsPageRuntime {
    environment: JsDomEnvironment,
    #[allow(dead_code)]
    document: Box<HtmlDocument>,
    scripts: Vec<ScriptDescriptor>,
    executed_blocking: bool,
}

impl JsPageRuntime {
    /// Construct a runtime for the supplied HTML/script manifest and base URL.
    pub fn new(
        html: &str,
        scripts: &[ScriptDescriptor],
        config: DocumentConfig,
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
            executed_blocking: false,
        }))
    }

    /// Execute all classic blocking scripts in document order.
    pub fn run_blocking_scripts(&mut self) -> Result<Option<ScriptExecutionSummary>> {
        if self.executed_blocking {
            return Ok(None);
        }

        let inline_scripts = filter_inline_classic(&self.scripts);
        if inline_scripts.is_empty() {
            self.executed_blocking = true;
            return Ok(None);
        }

        let summary = run_inline_scripts(&self.environment, &inline_scripts)?;
        self.executed_blocking = true;
        Ok(Some(summary))
    }

    /// Serialize the current document tree managed by the runtime.
    pub fn document_html(&self) -> Result<String> {
        self.environment
            .document_html()
            .context("failed to serialize runtime document")
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
}
