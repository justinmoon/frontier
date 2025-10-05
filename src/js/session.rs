use anyhow::{Context as AnyhowContext, Result};

use super::environment::JsDomEnvironment;
use super::processor::{filter_inline_classic, run_inline_scripts, ScriptExecutionSummary};
use super::script::ScriptDescriptor;

/// Owns the JavaScript runtime for a page and coordinates script execution.
pub struct JsPageRuntime {
    environment: JsDomEnvironment,
    scripts: Vec<ScriptDescriptor>,
    executed_blocking: bool,
}

impl JsPageRuntime {
    /// Construct a runtime for the supplied HTML/scrip manifest.
    pub fn new(html: &str, scripts: &[ScriptDescriptor]) -> Result<Option<Self>> {
        if scripts.is_empty() {
            return Ok(None);
        }

        let environment = JsDomEnvironment::new(html)
            .context("failed to create QuickJS environment for page runtime")?;

        Ok(Some(Self {
            environment,
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

    /// Expose the script manifest associated with this runtime.
    pub fn scripts(&self) -> &[ScriptDescriptor] {
        &self.scripts
    }
}
