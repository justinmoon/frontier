use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

/// Basic smoke test: URL bar renders with correct structure
#[test]
fn test_url_bar_basics() {
    let html = r#"
        <nav id="url-bar-container">
            <form id="url-form">
                <input type="url" id="url-input" name="url" value="https://example.com" />
                <input type="submit" id="go-button" value="Go" />
            </form>
        </nav>
        <main id="content"><h1>Page Content</h1></main>
    "#;

    let doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url: Some("https://example.com".to_string()),
            ..Default::default()
        },
    );

    // Verify basic structure exists
    assert!(doc.query_selector("#url-bar-container").unwrap().is_some());
    assert!(doc.query_selector("#url-input").unwrap().is_some());
    assert!(doc.query_selector("#go-button").unwrap().is_some());
}
