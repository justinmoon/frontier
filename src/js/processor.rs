use anyhow::{anyhow, Context as AnyhowContext, Result};
use kuchiki::parse_html;
use kuchiki::traits::*;
use tracing::{debug, error};

use super::environment::JsDomEnvironment;
use crate::navigation::FetchedDocument;

#[derive(Debug, Clone, Copy)]
pub struct ScriptExecutionSummary {
    pub executed_scripts: usize,
    pub dom_mutations: usize,
}

pub fn execute_inline_scripts(
    document: &mut FetchedDocument,
) -> Result<Option<ScriptExecutionSummary>> {
    let scripts = extract_inline_scripts(&document.contents)?;
    if scripts.is_empty() {
        return Ok(None);
    }

    let environment = JsDomEnvironment::new(&document.contents)
        .context("failed to initialize QuickJS environment")?;

    let mut executed = 0usize;

    for (idx, script) in scripts.iter().enumerate() {
        let filename = format!("inline-script-{idx}.js");
        match environment.eval(script, &filename) {
            Ok(_) => {
                executed += 1;
            }
            Err(err) => {
                error!(target = "quickjs", %filename, error = %err, "inline script execution failed");
            }
        }
    }

    let dom_mutations = environment.drain_mutations().len();
    let mutated_html = environment
        .document_html()
        .context("failed to serialize DOM after script execution")?;

    if mutated_html != document.contents {
        document.contents = mutated_html;
        debug!(
            target = "quickjs",
            scripts = executed,
            dom_mutations,
            "applied inline script mutations"
        );
    }

    Ok(Some(ScriptExecutionSummary {
        executed_scripts: executed,
        dom_mutations,
    }))
}

fn extract_inline_scripts(html: &str) -> Result<Vec<String>> {
    let parsed = parse_html().one(html);
    let mut collected = Vec::new();
    let selector = parsed
        .select("script")
        .map_err(|_| anyhow!("failed to compile selector"))?;

    for script in selector {
        let attributes = script.attributes.borrow();
        if attributes.get("src").is_some() {
            continue;
        }
        if let Some(script_type) = attributes.get("type") {
            let ty = script_type.trim().to_ascii_lowercase();
            if !ty.is_empty() && ty != "text/javascript" && ty != "application/javascript" {
                continue;
            }
        }
        drop(attributes);
        let code = script.text_contents();
        if code.trim().is_empty() {
            continue;
        }
        collected.push(code);
    }

    Ok(collected)
}
