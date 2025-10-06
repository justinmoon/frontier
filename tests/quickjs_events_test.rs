use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use frontier::js::environment::JsDomEnvironment;

#[tokio::test]
async fn test_add_event_listener() {
    let html = "<!DOCTYPE html><html><body><button id=\"btn\">Click</button></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    // Add an event listener that modifies the DOM
    environment
        .eval(
            r#"
            const btn = document.getElementById('btn');
            btn.addEventListener('click', function() {
                btn.textContent = 'Clicked!';
            });

            // Dispatch the event
            const event = new Event('click');
            btn.dispatchEvent(event);
        "#,
            "event-test.js",
        )
        .expect("evaluate script");

    // Verify the listener was called by checking DOM
    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("Clicked!"),
        "Event listener should have updated button text"
    );
}

#[tokio::test]
async fn test_remove_event_listener() {
    let html = "<!DOCTYPE html><html><body><button id=\"btn\">Click</button></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    environment
        .eval(
            r#"
            const btn = document.getElementById('btn');

            function handler() {
                btn.textContent = btn.textContent + 'X';
            }

            btn.addEventListener('click', handler);

            // Dispatch once
            btn.dispatchEvent(new Event('click'));

            // Remove listener
            btn.removeEventListener('click', handler);

            // Dispatch again
            btn.dispatchEvent(new Event('click'));
        "#,
            "remove-event-test.js",
        )
        .expect("evaluate script");

    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("ClickX") && !html.contains("ClickXX"),
        "Listener should only fire once before removal: {}",
        html
    );
}

#[tokio::test]
async fn test_multiple_listeners() {
    let html = "<!DOCTYPE html><html><body><button id=\"btn\">Click</button></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    environment
        .eval(
            r#"
            const btn = document.getElementById('btn');

            btn.addEventListener('click', function() {
                btn.textContent = btn.textContent + 'A';
            });

            btn.addEventListener('click', function() {
                btn.textContent = btn.textContent + 'B';
            });

            btn.dispatchEvent(new Event('click'));
        "#,
            "multiple-listeners-test.js",
        )
        .expect("evaluate script");

    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("ClickAB"),
        "Both listeners should fire in order: {}",
        html
    );
}

#[tokio::test]
async fn test_prevent_default() {
    let html = "<!DOCTYPE html><html><body><button id=\"btn\">Click</button></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
    environment.attach_document(&mut document);

    environment
        .eval(
            r#"
            const btn = document.getElementById('btn');

            btn.addEventListener('click', function(event) {
                event.preventDefault();
                if (event.defaultPrevented) {
                    btn.textContent = 'Prevented';
                }
            });

            const event = new Event('click', { cancelable: true });
            const result = btn.dispatchEvent(event);

            // Mark the dispatch result in the DOM
            if (!result) {
                btn.textContent = btn.textContent + '-NotDefault';
            }
        "#,
            "prevent-default-test.js",
        )
        .expect("evaluate script");

    let html = environment.document_html().expect("serialize");
    assert!(
        html.contains("Prevented-NotDefault"),
        "Event should be prevented and dispatchEvent should return false: {}",
        html
    );
}
