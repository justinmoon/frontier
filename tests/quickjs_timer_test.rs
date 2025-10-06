use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use frontier::js::environment::JsDomEnvironment;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_set_timeout() {
    let html = "<!DOCTYPE html><html><body><div id=\"target\">Initial</div></body></html>";

    // Create environment inside tokio context
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Set up a timeout that modifies the DOM
    environment
        .eval(
            r#"
            setTimeout(function() {
                document.getElementById('target').textContent = 'Updated by timeout';
            }, 100);
        "#,
            "timeout-test.js",
        )
        .expect("evaluate script");

    // Wait for the timeout to fire (async sleep)
    sleep(Duration::from_millis(150)).await;

    // Poll timers to execute the callback
    let executed = environment.poll_timers().expect("poll timers");
    assert_eq!(executed, 1, "should execute 1 timer");

    // Check that the DOM was updated
    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("Updated by timeout"),
        "DOM should be updated by timeout callback"
    );
}

#[tokio::test]
async fn test_set_interval() {
    let html = "<!DOCTYPE html><html><body><div id=\"counter\">0</div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Set up an interval that increments a counter
    environment
        .eval(
            r#"
            let count = 0;
            const intervalId = setInterval(function() {
                count++;
                document.getElementById('counter').textContent = String(count);
            }, 50);
            // Store intervalId globally so we can clear it
            globalThis.testIntervalId = intervalId;
        "#,
            "interval-test.js",
        )
        .expect("evaluate script");

    // Wait for multiple ticks
    sleep(Duration::from_millis(200)).await;

    // Poll timers multiple times
    let mut total_executed = 0;
    for _ in 0..5 {
        total_executed += environment.poll_timers().expect("poll timers");
        sleep(Duration::from_millis(10)).await;
    }

    assert!(
        total_executed >= 3,
        "should execute interval at least 3 times, got {}",
        total_executed
    );

    // Clear the interval
    environment
        .eval(
            "clearInterval(globalThis.testIntervalId);",
            "clear-interval.js",
        )
        .expect("clear interval");
}

#[tokio::test]
async fn test_clear_timeout() {
    let html = "<!DOCTYPE html><html><body><div id=\"target\">Initial</div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Set up a timeout then immediately clear it
    environment
        .eval(
            r#"
            const timeoutId = setTimeout(function() {
                document.getElementById('target').textContent = 'Should not see this';
            }, 100);
            clearTimeout(timeoutId);
        "#,
            "clear-timeout-test.js",
        )
        .expect("evaluate script");

    // Wait for when the timeout would have fired
    sleep(Duration::from_millis(150)).await;

    // Poll timers
    let executed = environment.poll_timers().expect("poll timers");
    assert_eq!(executed, 0, "should not execute any timers");

    // Check that the DOM was NOT updated
    let html = environment.document_html().expect("serialize");
    assert!(
        !html.contains("Should not see this"),
        "DOM should not be updated after clearing timeout"
    );
}

#[tokio::test]
async fn test_queue_microtask() {
    let html = "<!DOCTYPE html><html><body><div id=\"result\">Initial</div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Queue a microtask that modifies the DOM
    environment
        .eval(
            r#"
            queueMicrotask(function() {
                document.getElementById('result').textContent = 'Microtask executed';
            });
        "#,
            "microtask-test.js",
        )
        .expect("evaluate script");

    // The microtask should have executed immediately after eval
    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("Microtask executed"),
        "Microtask should execute immediately after script"
    );
}

#[tokio::test]
async fn test_microtask_execution_order() {
    let html = "<!DOCTYPE html><html><body><div id=\"result\"></div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Queue multiple microtasks
    environment
        .eval(
            r#"
            let result = '';
            queueMicrotask(function() {
                result += '1';
            });
            queueMicrotask(function() {
                result += '2';
            });
            queueMicrotask(function() {
                result += '3';
                document.getElementById('result').textContent = result;
            });
        "#,
            "microtask-order-test.js",
        )
        .expect("evaluate script");

    // All microtasks should have executed in order
    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains(">123<"),
        "Microtasks should execute in order: {}",
        html
    );
}

#[tokio::test]
async fn test_nested_microtasks() {
    let html = "<!DOCTYPE html><html><body><div id=\"result\"></div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Test microtask that queues another microtask
    environment
        .eval(
            r#"
            let result = '';
            queueMicrotask(function() {
                result += 'A';
                queueMicrotask(function() {
                    result += 'C';
                    document.getElementById('result').textContent = result;
                });
                result += 'B';
            });
        "#,
            "nested-microtask-test.js",
        )
        .expect("evaluate script");

    // Nested microtask should also execute
    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains(">ABC<"),
        "Nested microtasks should execute: {}",
        html
    );
}
