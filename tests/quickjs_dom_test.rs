use frontier::js::processor;
use frontier::js::session::JsPageRuntime;
use frontier::navigation::FetchedDocument;

#[test]
fn quickjs_demo_executes_script_and_mutates_dom() {
    let html = std::fs::read_to_string("assets/quickjs-demo.html").expect("demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");
    assert_eq!(scripts.len(), 1, "demo asset contains one inline script");

    let mut runtime = JsPageRuntime::new(&html, &scripts)
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
