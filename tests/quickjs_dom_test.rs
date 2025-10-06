use blitz_dom::{local_name, BaseDocument, DocumentConfig};
use blitz_html::HtmlDocument;
use frontier::js::environment::JsDomEnvironment;
use frontier::js::processor;
use frontier::js::session::JsPageRuntime;
use frontier::navigation::FetchedDocument;

#[tokio::test]
async fn quickjs_demo_executes_script_and_mutates_dom() {
    let html = std::fs::read_to_string("assets/quickjs-demo.html").expect("demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");
    assert_eq!(scripts.len(), 1, "demo asset contains one inline script");

    let mut runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");
    let runtime_summary = runtime
        .run_blocking_scripts()
        .expect("runtime execution")
        .expect("runtime executed script");
    assert!(runtime_summary.executed_scripts > 0);

    let mutated = runtime.document_html().expect("serialize runtime dom");
    assert!(mutated.contains("Hello from QuickJS!"));
    assert!(mutated.contains("data-origin=\"quickjs-demo\""));

    let mut document = FetchedDocument {
        base_url: "file://demo".into(),
        contents: html,
        file_path: None,
        display_url: "file://demo/quickjs-demo.html".into(),
        blossom: None,
        scripts: scripts.clone(),
    };
    let summary = processor::execute_inline_scripts(&mut document)
        .expect("processor execution")
        .expect("processor ran script");

    assert_eq!(summary.executed_scripts, runtime_summary.executed_scripts);
    assert!(document.contents.contains("Hello from QuickJS!"));
    assert!(document.contents.contains("data-origin=\"quickjs-demo\""));
}

#[tokio::test]
async fn dom_bridge_updates_live_document() {
    let html = "<!DOCTYPE html><html><body><h1 id=\"message\">Loading…</h1></body></html>";

    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

    environment.attach_document(&mut document);
    environment
        .eval(
            "document.getElementById('message').textContent = 'Updated';",
            "bridge-test.js",
        )
        .expect("evaluate script");

    let mut updated = None;
    {
        let base: &mut BaseDocument = &mut document;
        let root_id = base.root_node().id;
        base.iter_subtree_mut(root_id, |node_id, doc| {
            if updated.is_some() {
                return;
            }
            if let Some(node) = doc.get_node(node_id) {
                if node.attr(local_name!("id")) == Some("message") {
                    updated = Some(node.text_content());
                }
            }
        });
    }

    assert_eq!(updated.as_deref(), Some("Updated"));
}
