use frontier::js::processor;
use frontier::navigation::FetchedDocument;

#[test]
fn quickjs_demo_executes_script_and_mutates_dom() {
    let html = std::fs::read_to_string("assets/quickjs-demo.html").expect("demo asset");
    let mut document = FetchedDocument {
        base_url: "file://demo".into(),
        contents: html,
        file_path: None,
        display_url: "file://demo/quickjs-demo.html".into(),
        blossom: None,
    };

    let summary = processor::execute_inline_scripts(&mut document)
        .expect("script execution")
        .expect("scripts executed");

    assert!(summary.executed_scripts > 0);
    assert!(document.contents.contains("Hello from QuickJS!"));
    assert!(document.contents.contains("data-origin=\"quickjs-demo\""));
}
