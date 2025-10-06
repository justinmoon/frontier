use blitz_dom::net::Resource;
use blitz_dom::DocumentConfig;
use blitz_net::Provider;
use frontier::js::processor;
use frontier::js::session::JsPageRuntime;
use std::sync::Arc;

// Create a simple file-based network provider for tests
fn create_file_provider() -> Arc<Provider<Resource>> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let callback = Arc::new(
        move |_doc_id: usize, resource: Result<Resource, Option<String>>| {
            let _ = tx.send((_doc_id, resource));
        },
    );
    Arc::new(Provider::new(callback))
}

#[tokio::test]
async fn react_local_counter_script_collection() {
    let html = std::fs::read_to_string("assets/react-local-counter.html")
        .expect("react local counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    eprintln!("Collected {} scripts from React local demo", scripts.len());
    for (i, script) in scripts.iter().enumerate() {
        match &script.source {
            frontier::js::script::ScriptSource::External { src } => {
                eprintln!("Script {}: External - {}", i, src);
            }
            frontier::js::script::ScriptSource::Inline { code } => {
                eprintln!("Script {}: Inline - {} chars", i, code.len());
            }
        }
    }

    assert_eq!(
        scripts.len(),
        3,
        "should have 2 external (React bundles) + 1 inline script"
    );
}

#[tokio::test]
async fn react_local_counter_loads_and_runs() {
    let html = std::fs::read_to_string("assets/react-local-counter.html")
        .expect("react local counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let provider = create_file_provider();
    let config = DocumentConfig {
        base_url: Some(
            "file:///Users/justin/code/frontier/worktrees/dom-api-milestones-claude/assets/"
                .to_string(),
        ),
        ..Default::default()
    };

    eprintln!("\nScript details:");
    for (i, script) in scripts.iter().enumerate() {
        eprintln!(
            "  Script {}: kind={:?}, execution={:?}, source={:?}",
            i,
            script.kind,
            script.execution,
            match &script.source {
                frontier::js::script::ScriptSource::External { src } =>
                    format!("External({})", src),
                frontier::js::script::ScriptSource::Inline { code } =>
                    format!("Inline({} chars)", code.len()),
            }
        );
    }

    let mut runtime = JsPageRuntime::new(&html, &scripts, config, Some(provider))
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    let summary = runtime.run_blocking_scripts().expect("runtime execution");

    eprintln!("\nExecution summary: {:?}", summary);

    if let Some(sum) = summary {
        eprintln!("Executed {} scripts", sum.executed_scripts);
        // We expect all 3 scripts to execute (2 React bundles + 1 inline)
        assert!(
            sum.executed_scripts > 0,
            "should have executed React scripts"
        );
    }

    let rendered = runtime.document_html().expect("serialize dom");

    eprintln!("Rendered HTML length: {} bytes", rendered.len());

    // Check if React initialized
    if rendered.contains("data-react-initialized=\"true\"") {
        eprintln!("✓ React initialization marker found!");
    } else {
        eprintln!("✗ React initialization marker NOT found");
        eprintln!(
            "Rendered HTML snippet: {}",
            &rendered[..rendered.len().min(500)]
        );
    }
}
