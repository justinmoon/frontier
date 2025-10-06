/// Test that replicates exactly what the GUI does:
/// Uses JsPageRuntime.dispatch_event() to send click events
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_dom::net::Resource;
use frontier::js::processor;
use frontier::js::session::JsPageRuntime;
use std::fs;
use std::sync::Arc;

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
async fn test_dispatch_event_like_gui() {
    // Set up tracing
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    println!("\n🧪 Testing dispatch_event (GUI simulation)\n");

    // Load React counter HTML
    let html = fs::read_to_string("assets/react-sync-counter.html")
        .expect("Failed to load react-sync-counter.html");

    let scripts = processor::collect_scripts(&html).expect("collect scripts");
    println!("✓ Found {} scripts", scripts.len());

    let provider = create_file_provider();
    let config = DocumentConfig {
        base_url: Some(
            "file:///Users/justin/code/frontier/worktrees/dom-api-milestones-claude/assets/"
                .to_string(),
        ),
        ..Default::default()
    };

    let mut runtime = JsPageRuntime::new(&html, &scripts, config, Some(provider))
        .await
        .expect("create runtime")
        .expect("runtime available");

    println!("✓ Created runtime");

    // Execute scripts
    let summary = runtime
        .run_blocking_scripts()
        .expect("execute scripts")
        .expect("summary");

    println!("✓ Executed {} scripts", summary.executed_scripts);

    // Get initial HTML
    let initial_html = runtime.document_html().expect("initial html");
    let doc = HtmlDocument::from_html(&initial_html, DocumentConfig::default());

    // Find the button
    let button_id = doc
        .query_selector("#increment-btn")
        .expect("query works")
        .expect("button exists");

    println!("✓ Found button at node_id={}", button_id);

    // Find count span
    let count_span = doc
        .query_selector("#count")
        .expect("query works")
        .expect("count exists");

    let initial_count: i32 = doc
        .tree()
        .get(count_span)
        .expect("node exists")
        .text_content()
        .trim()
        .parse()
        .expect("count is number");

    println!("✓ Initial count: {}", initial_count);

    // First check if __frontier_dispatch_event exists
    println!("\n🔍 Checking if __frontier_dispatch_event exists...");
    runtime.eval(
        r#"
        console.log('typeof __frontier_dispatch_event: ' + typeof __frontier_dispatch_event);
        "#,
        "check-dispatch.js"
    ).ok();

    // THIS IS WHAT THE GUI DOES: dispatch_event
    println!("\n📍 Dispatching click to node_id={} (like GUI does)", button_id);

    // Debug: See how the working test dispatches
    println!("\n🔍 Testing working approach (via element.dispatchEvent):");
    runtime.eval(&format!(
        r#"
        var btn = document.getElementById('increment-btn');
        var countSpan = document.getElementById('count');
        var beforeText = countSpan.textContent;
        console.log('Before via btn.dispatchEvent: ' + beforeText);

        var event = new MouseEvent('click', {{ bubbles: true, cancelable: true }});
        btn.dispatchEvent(event);

        var afterText = countSpan.textContent;
        console.log('After via btn.dispatchEvent: ' + afterText);
        "#
    ), "btn-dispatchEvent.js").expect("btn dispatch works");

    println!("\n🔍 Now testing __frontier_dispatch_event:");
    // Try calling directly via eval to see what happens
    runtime.eval(&format!(
        r#"
        var countSpan = document.getElementById('count');
        var beforeText = countSpan.textContent;
        console.log('Before via __frontier_dispatch: ' + beforeText);

        var handle = '{}';
        var event = new MouseEvent('click', {{ bubbles: true, cancelable: true }});
        __frontier_dispatch_event(handle, event);

        var afterText = countSpan.textContent;
        console.log('After via __frontier_dispatch: ' + afterText);
        "#,
        button_id
    ), "manual-dispatch.js").expect("manual dispatch works");

    runtime
        .dispatch_event(button_id, "click")
        .expect("dispatch works");

    // Get updated HTML
    let updated_html = runtime.document_html().expect("updated html");
    let updated_doc = HtmlDocument::from_html(&updated_html, DocumentConfig::default());

    // Check updated count
    let updated_count_span = updated_doc
        .query_selector("#count")
        .expect("query works")
        .expect("count exists");

    let updated_count: i32 = updated_doc
        .tree()
        .get(updated_count_span)
        .expect("node exists")
        .text_content()
        .trim()
        .parse()
        .expect("count is number");

    println!("  Updated count: {}", updated_count);

    assert_eq!(
        updated_count,
        initial_count + 1,
        "Counter should increment when using dispatch_event"
    );

    println!("\n✅ TEST PASSED - dispatch_event works!\n");
}
