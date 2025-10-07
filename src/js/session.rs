use std::rc::Rc;

use anyhow::{Context as AnyhowContext, Result};
use blitz_dom::BaseDocument;

use super::environment::JsDomEnvironment;
use super::processor::{filter_inline_classic, run_inline_scripts, ScriptExecutionSummary};
use super::script::ScriptDescriptor;

/// Owns the JavaScript runtime for a page and coordinates script execution.
pub struct JsPageRuntime {
    environment: Rc<JsDomEnvironment>,
    scripts: Vec<ScriptDescriptor>,
    executed_blocking: bool,
    bridge_attached: bool,
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
            environment: Rc::new(environment),
            scripts: scripts.to_vec(),
            executed_blocking: false,
            bridge_attached: false,
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
        self.environment.pump()?;
        self.executed_blocking = true;
        Ok(Some(summary))
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
