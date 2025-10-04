use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

/// Real-world test: Can we load actual webpages?
#[tokio::test]
#[ignore] // Run with: cargo test --test online_test -- --ignored
async fn test_load_webpage() {
    // Fetch a real website
    let response = reqwest::get("https://example.com")
        .await
        .expect("Failed to fetch example.com");
    let url = response.url().to_string();
    let html = response.text().await.expect("Failed to get HTML");

    // Parse it with blitz
    let doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some(url),
            ..Default::default()
        },
    );

    // Verify we got content
    let h1_elements = doc.query_selector_all("h1").unwrap();
    assert!(
        !h1_elements.is_empty(),
        "Should have loaded example.com with h1 elements"
    );
}
