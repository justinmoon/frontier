use blitz_dom::DocumentConfig;
use frontier::js::processor;
use frontier::js::session::JsPageRuntime;

#[tokio::test]
async fn react_counter_dom_access_works() {
    // Simple test to verify getElementById works
    let html = std::fs::read_to_string("assets/react-counter-demo.html")
        .expect("react counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    // Try basic DOM access
    let result = runtime.eval(
        r#"
        const root = document.getElementById('root');
        if (!root) throw new Error('root not found');
        root.textContent = 'Test';
        "#,
        "test-dom.js",
    );

    eprintln!("DOM access result: {:?}", result);
    assert!(
        result.is_ok(),
        "should be able to access DOM: {:?}",
        result.err()
    );

    let rendered = runtime.document_html().expect("serialize dom");
    assert!(
        rendered.contains("Test"),
        "should have modified root element"
    );
}

#[tokio::test]
#[ignore] // innerHTML not working yet - needs investigation
async fn react_counter_inner_html_works() {
    // Test innerHTML with template literals
    let html = std::fs::read_to_string("assets/react-counter-demo.html")
        .expect("react counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    eprintln!("Testing innerHTML assignment");
    let result = runtime.eval(
        r#"
        const root = document.getElementById('root');
        if (!root) throw new Error('root is null');
        const initialHTML = root.innerHTML;
        console.log('Initial innerHTML: ' + initialHTML);

        root.innerHTML = '<span>Test Value</span>';

        const afterHTML = root.innerHTML;
        console.log('After innerHTML: ' + afterHTML);

        if (!afterHTML || afterHTML === initialHTML) {
            throw new Error('innerHTML did not change');
        }
        "#,
        "test-innerHTML.js",
    );

    eprintln!("innerHTML result: {:?}", result);
    assert!(
        result.is_ok(),
        "should be able to use innerHTML: {:?}",
        result.err()
    );

    let rendered = runtime.document_html().expect("serialize dom");
    eprintln!(
        "Rendered HTML snippet: {}",
        &rendered[rendered.find("root").unwrap_or(0)
            ..rendered.len().min(rendered.find("root").unwrap_or(0) + 200)]
    );
    assert!(
        rendered.contains("Test Value"),
        "should have rendered innerHTML: {}",
        rendered
    );
}

#[tokio::test]
async fn react_counter_initializes_and_renders() {
    let html = std::fs::read_to_string("assets/react-counter-demo.html")
        .expect("react counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let mut runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    let summary = runtime.run_blocking_scripts().expect("runtime execution");

    assert!(summary.is_some(), "should have executed scripts");
    let summary = summary.unwrap();
    assert!(
        summary.executed_scripts > 0,
        "should execute counter script"
    );

    let rendered = runtime.document_html().expect("serialize dom");

    // Verify initial render
    assert!(
        rendered.contains("data-initialized=\"true\""),
        "counter should be initialized: {}",
        rendered
    );
    assert!(rendered.contains("Count:"), "should render counter text");
    assert!(
        rendered.contains("id=\"count-value\""),
        "should have count span"
    );
    assert!(
        rendered.contains("Increment"),
        "should have increment button"
    );
    assert!(rendered.contains(">0<"), "should show initial count of 0");
}

#[tokio::test]
async fn react_counter_handles_click_events() {
    let html = std::fs::read_to_string("assets/react-counter-demo.html")
        .expect("react counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let mut runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    runtime
        .run_blocking_scripts()
        .expect("runtime execution")
        .expect("runtime executed script");

    // Simulate click event from within JS
    runtime
        .eval(
            r#"
            (function() {
                const btn = document.getElementById('increment-btn');
                const event = new Event('click');
                btn.dispatchEvent(event);
            })();
            "#,
            "simulate-click.js",
        )
        .expect("simulate click");

    let rendered = runtime.document_html().expect("serialize dom");

    // After click, count should be incremented to 1
    assert!(
        rendered.contains(">1<"),
        "count should be incremented after click: {}",
        rendered
    );
}

#[tokio::test]
async fn react_counter_state_persists_across_evaluations() {
    let html = std::fs::read_to_string("assets/react-counter-demo.html")
        .expect("react counter demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");

    let mut runtime = JsPageRuntime::new(&html, &scripts, DocumentConfig::default(), None)
        .await
        .expect("create runtime")
        .expect("runtime available for scripts");

    runtime.run_blocking_scripts().expect("runtime execution");

    // Click multiple times
    for _ in 0..3 {
        runtime
            .eval(
                r#"
                (function() {
                    const btn = document.getElementById('increment-btn');
                    btn.dispatchEvent(new Event('click'));
                })();
                "#,
                "multi-click.js",
            )
            .expect("simulate click");
    }

    let rendered = runtime.document_html().expect("serialize dom");

    // After 3 clicks, count should be 3
    assert!(
        rendered.contains(">3<") || rendered.contains("Counter: 3"),
        "count should persist and accumulate across clicks: {}",
        rendered
    );
}
