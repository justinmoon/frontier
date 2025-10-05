use anyhow::{anyhow, Context as AnyhowContext, Result};
use kuchiki::parse_html;
use kuchiki::traits::*;
use tracing::{debug, error};

use super::environment::JsDomEnvironment;
use super::script::{ScriptDescriptor, ScriptExecution, ScriptKind, ScriptSource};
use crate::navigation::FetchedDocument;

#[derive(Debug, Clone, Copy)]
pub struct ScriptExecutionSummary {
    pub executed_scripts: usize,
    pub dom_mutations: usize,
}

#[allow(dead_code)]
pub fn execute_inline_scripts(
    document: &mut FetchedDocument,
) -> Result<Option<ScriptExecutionSummary>> {
    if document.scripts.is_empty() {
        document.scripts = collect_scripts(&document.contents)?;
    }

    let inline_scripts = filter_inline_classic(&document.scripts);
    if inline_scripts.is_empty() {
        return Ok(None);
    }

    let environment = JsDomEnvironment::new(&document.contents)
        .context("failed to initialize QuickJS environment")?;
    let summary = run_inline_scripts(&environment, &inline_scripts)?;

    finalize_environment(document, &environment, summary)
}

pub fn collect_scripts(html: &str) -> Result<Vec<ScriptDescriptor>> {
    let parsed = parse_html().one(html);
    let mut collected = Vec::new();
    let selector = parsed
        .select("script")
        .map_err(|_| anyhow!("failed to compile selector"))?;

    for (index, script) in selector.enumerate() {
        let attributes = script.attributes.borrow();
        let kind = classify_kind(attributes.get("type"));
        let execution = determine_execution(&attributes, kind);

        if let Some(src) = attributes
            .get("src")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            collected.push(ScriptDescriptor {
                index,
                kind,
                execution,
                source: ScriptSource::External {
                    src: src.to_string(),
                },
            });
            continue;
        }

        drop(attributes);
        let code = script.text_contents();
        if code.trim().is_empty() {
            continue;
        }
        collected.push(ScriptDescriptor::inline(index, code, kind));
    }

    Ok(collected)
}

fn classify_kind(script_type: Option<&str>) -> ScriptKind {
    match script_type {
        Some(value) => {
            let lowered = value.trim().to_ascii_lowercase();
            match lowered.as_str() {
                "" | "text/javascript" | "application/javascript" => ScriptKind::Classic,
                "module" | "text/javascript+module" => ScriptKind::Module,
                other => {
                    if other == "text/ecmascript" || other == "application/ecmascript" {
                        ScriptKind::Classic
                    } else {
                        ScriptKind::Unknown
                    }
                }
            }
        }
        None => ScriptKind::Classic,
    }
}

fn determine_execution(attributes: &kuchiki::Attributes, kind: ScriptKind) -> ScriptExecution {
    if attributes.get("async").is_some() {
        return ScriptExecution::Async;
    }
    if attributes.get("defer").is_some() {
        return ScriptExecution::Defer;
    }
    match kind {
        ScriptKind::Module => ScriptExecution::Defer,
        _ => ScriptExecution::Blocking,
    }
}

pub(super) fn filter_inline_classic(scripts: &[ScriptDescriptor]) -> Vec<ScriptDescriptor> {
    scripts
        .iter()
        .filter(|descriptor| matches!(descriptor.source, ScriptSource::Inline { .. }))
        .filter(|descriptor| descriptor.execution == ScriptExecution::Blocking)
        .cloned()
        .collect()
}

pub(super) fn run_inline_scripts(
    environment: &JsDomEnvironment,
    scripts: &[ScriptDescriptor],
) -> Result<ScriptExecutionSummary> {
    let mut executed = 0usize;

    for descriptor in scripts {
        let filename = format!("inline-script-{}.js", descriptor.index);
        let source = match &descriptor.source {
            ScriptSource::Inline { code } => code,
            ScriptSource::External { .. } => continue,
        };

        match environment.eval(source, &filename) {
            Ok(_) => executed += 1,
            Err(err) => {
                error!(target = "quickjs", %filename, error = %err, "inline script execution failed");
            }
        }
    }

    Ok(ScriptExecutionSummary {
        executed_scripts: executed,
        dom_mutations: environment.drain_mutations().len(),
    })
}

fn finalize_environment(
    document: &mut FetchedDocument,
    environment: &JsDomEnvironment,
    summary: ScriptExecutionSummary,
) -> Result<Option<ScriptExecutionSummary>> {
    let mutated_html = environment
        .document_html()
        .context("failed to serialize DOM after script execution")?;

    if mutated_html != document.contents {
        document.contents = mutated_html;
        debug!(
            target = "quickjs",
            scripts = summary.executed_scripts,
            dom_mutations = summary.dom_mutations,
            "applied inline script mutations"
        );
    }

    Ok(Some(summary))
}
