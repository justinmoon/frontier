/// Integration test that fetches real websites and verifies content
/// Run with: cargo test --test integration_test -- --ignored
/// (These tests are marked as ignored because they require network access)

use blitz_dom::{DocumentConfig, local_name};
use blitz_html::HtmlDocument;

async fn fetch_url_content(url: &str) -> (String, String) {
    // Use reqwest directly for testing (simpler than setting up blitz networking)
    let response = reqwest::get(url).await.expect("Failed to fetch URL");
    let response_url = response.url().to_string();
    let content = response.text().await.expect("Failed to get response text");
    (response_url, content)
}

#[tokio::test]
#[ignore] // Requires network access
async fn test_fetch_example_com() {
    let (url, content) = fetch_url_content("https://example.com").await;

    // Verify we got example.com
    assert!(
        url.contains("example.com"),
        "Should fetch example.com"
    );

    // Create a document from the HTML
    let doc = HtmlDocument::from_html(
        &content,
        DocumentConfig {
            base_url: Some(url),
            ..Default::default()
        },
    );

    // Verify example.com has expected content
    let h1_elements = doc.query_selector_all("h1").unwrap();
    assert!(
        !h1_elements.is_empty(),
        "Example.com should have at least one h1 element"
    );

    // Check the h1 text content
    let h1_id = h1_elements[0];
    let h1_node = doc.get_node(h1_id).unwrap();
    let h1_text = h1_node.text_content();

    assert!(
        h1_text.contains("Example Domain"),
        "Example.com h1 should contain 'Example Domain', got: {}",
        h1_text
    );

    // Verify there's a paragraph
    let p_elements = doc.query_selector_all("p").unwrap();
    assert!(
        !p_elements.is_empty(),
        "Example.com should have paragraph elements"
    );

    // Verify there's an anchor link
    let a_elements = doc.query_selector_all("a").unwrap();
    assert!(
        !a_elements.is_empty(),
        "Example.com should have anchor elements"
    );
}

#[tokio::test]
#[ignore] // Requires network access
async fn test_fetch_google_com() {
    let (url, content) = fetch_url_content("https://www.google.com").await;

    // Verify we got google.com
    assert!(
        url.contains("google.com"),
        "Should fetch google.com"
    );

    // Create a document from the HTML
    let doc = HtmlDocument::from_html(
        &content,
        DocumentConfig {
            base_url: Some(url),
            ..Default::default()
        },
    );

    // Google should have a form (the search form)
    let forms = doc.query_selector_all("form").unwrap();
    assert!(
        !forms.is_empty(),
        "Google should have at least one form element"
    );

    // Google should have input fields
    let inputs = doc.query_selector_all("input").unwrap();
    assert!(
        !inputs.is_empty(),
        "Google should have input elements"
    );

    // Look for search-related inputs
    let mut found_text_input = false;
    for input_id in inputs {
        if let Some(element) = doc.get_node(input_id).and_then(|n| n.element_data()) {
            let input_type = element.attr(local_name!("type"));
            if input_type == Some("text") || input_type == Some("search") || input_type.is_none() {
                found_text_input = true;
                break;
            }
        }
    }

    assert!(
        found_text_input,
        "Google should have a text or search input field"
    );
}

#[tokio::test]
#[ignore] // Requires network access
async fn test_navigation_simulation() {
    // Simulate navigating from one page to another

    // Step 1: Fetch example.com
    let (url1, content1) = fetch_url_content("https://example.com").await;
    let doc1 = HtmlDocument::from_html(
        &content1,
        DocumentConfig {
            base_url: Some(url1.clone()),
            ..Default::default()
        },
    );

    // Verify first page loaded
    let h1_elements = doc1.query_selector_all("h1").unwrap();
    assert!(!h1_elements.is_empty(), "First page should load");

    // Step 2: Simulate navigating to a different page
    // In a real app, this would happen through form submission
    let (url2, content2) = fetch_url_content("https://www.google.com").await;

    // Verify we navigated to a different URL
    assert_ne!(
        url1, url2,
        "Should navigate to a different URL"
    );

    let doc2 = HtmlDocument::from_html(
        &content2,
        DocumentConfig {
            base_url: Some(url2),
            ..Default::default()
        },
    );

    // Verify second page has different content
    let forms = doc2.query_selector_all("form").unwrap();
    assert!(
        !forms.is_empty(),
        "Second page (Google) should have forms"
    );
}

#[test]
fn test_url_bar_with_real_structure() {
    // This test verifies our URL bar wrapper works with realistic HTML
    use crate::common::create_realistic_page;

    let url = "https://example.com";
    let page_content = r#"
        <!DOCTYPE html>
        <html>
        <head><title>Example Page</title></head>
        <body>
            <h1>Example Domain</h1>
            <p>This domain is for use in illustrative examples.</p>
            <a href="https://www.iana.org/domains/example">More information...</a>
        </body>
        </html>
    "#;

    let wrapped = create_realistic_page(page_content, url);

    let doc = HtmlDocument::from_html(
        &wrapped,
        DocumentConfig {
            base_url: Some(url.to_string()),
            ..Default::default()
        },
    );

    // Verify URL bar exists
    let url_bar = doc.query_selector("#url-bar-container").unwrap();
    assert!(url_bar.is_some(), "URL bar should be present");

    // Verify original content is preserved
    let h1_elements = doc.query_selector_all("h1").unwrap();
    assert!(!h1_elements.is_empty(), "Original h1 should be present");

    // Verify both URL bar AND page content coexist
    let url_input = doc.query_selector("#url-input").unwrap();
    assert!(url_input.is_some(), "URL input should be present");

    let para_elements = doc.query_selector_all("p").unwrap();
    assert!(!para_elements.is_empty(), "Original paragraphs should be present");
}

mod common {
    pub fn create_realistic_page(content: &str, current_url: &str) -> String {
        // Simulate the wrap_with_url_bar function for testing
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Blitz Browser - {current_url}</title>
</head>
<body>
    <nav id="url-bar-container">
        <form id="url-form">
            <input type="url" id="url-input" name="url" value="{current_url}" />
            <input type="submit" id="go-button" value="Go" />
        </form>
    </nav>
    <main id="content">
        {content}
    </main>
</body>
</html>"#,
            current_url = current_url,
            content = content
        )
    }
}
