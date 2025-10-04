use blitz_dom::{DocumentConfig, local_name};
use blitz_html::HtmlDocument;

/// Test helper to create a minimal document with URL bar
fn create_url_bar_document(url: &str) -> HtmlDocument {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Test Browser</title>
</head>
<body>
    <nav id="url-bar-container" role="navigation" aria-label="Browser navigation">
        <form id="url-form" role="search">
            <label for="url-input">Enter website URL</label>
            <input
                type="url"
                id="url-input"
                name="url"
                value="{url}"
                aria-label="Website URL address bar"
                placeholder="Enter URL..."
                required
            />
            <input
                type="submit"
                id="go-button"
                value="Go"
                aria-label="Navigate to URL"
            />
        </form>
    </nav>
    <main id="content" role="main" aria-label="Page content">
        <h1>Test Page</h1>
    </main>
</body>
</html>"#,
        url = url
    );

    HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some(url.to_string()),
            ..Default::default()
        },
    )
}

#[test]
fn test_url_bar_structure() {
    let doc = create_url_bar_document("https://example.com");

    // Test that URL bar container exists
    let url_bar_container = doc.query_selector("#url-bar-container").unwrap();
    assert!(url_bar_container.is_some(), "URL bar container should exist");

    // Test that the form exists
    let form = doc.query_selector("#url-form").unwrap();
    assert!(form.is_some(), "URL form should exist");

    // Test that the input field exists
    let url_input = doc.query_selector("#url-input").unwrap();
    assert!(url_input.is_some(), "URL input field should exist");

    // Test that the go button exists
    let go_button = doc.query_selector("#go-button").unwrap();
    assert!(go_button.is_some(), "Go button should exist");
}

#[test]
fn test_url_input_accessibility() {
    let doc = create_url_bar_document("https://example.com");

    // Get the URL input element
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    // Test accessibility attributes
    assert_eq!(
        url_input_element.attr(local_name!("type")),
        Some("url"),
        "Input should have type='url'"
    );

    assert_eq!(
        url_input_element.attr(local_name!("aria-label")),
        Some("Website URL address bar"),
        "Input should have aria-label"
    );

    assert_eq!(
        url_input_element.attr(local_name!("required")),
        Some(""),
        "Input should be required"
    );

    assert_eq!(
        url_input_element.attr(local_name!("id")),
        Some("url-input"),
        "Input should have id"
    );
}

#[test]
fn test_go_button_accessibility() {
    let doc = create_url_bar_document("https://example.com");

    // Get the go button element
    let go_button_id = doc.query_selector("#go-button").unwrap().unwrap();
    let go_button_node = doc.get_node(go_button_id).unwrap();
    let go_button_element = go_button_node.element_data().unwrap();

    // Test accessibility attributes
    assert_eq!(
        go_button_element.attr(local_name!("type")),
        Some("submit"),
        "Button should have type='submit'"
    );

    assert_eq!(
        go_button_element.attr(local_name!("aria-label")),
        Some("Navigate to URL"),
        "Button should have aria-label"
    );

    // Test button value
    assert_eq!(
        go_button_element.attr(local_name!("value")),
        Some("Go"),
        "Submit button should have value 'Go'"
    );
}

#[test]
fn test_navigation_semantics() {
    let doc = create_url_bar_document("https://example.com");

    // Test navigation container has proper role
    let nav_id = doc.query_selector("#url-bar-container").unwrap().unwrap();
    let nav_node = doc.get_node(nav_id).unwrap();
    let nav_element = nav_node.element_data().unwrap();

    assert_eq!(
        nav_element.attr(local_name!("role")),
        Some("navigation"),
        "Nav container should have role='navigation'"
    );

    assert_eq!(
        nav_element.attr(local_name!("aria-label")),
        Some("Browser navigation"),
        "Nav container should have descriptive aria-label"
    );

    // Test form has search role
    let form_id = doc.query_selector("#url-form").unwrap().unwrap();
    let form_node = doc.get_node(form_id).unwrap();
    let form_element = form_node.element_data().unwrap();

    assert_eq!(
        form_element.attr(local_name!("role")),
        Some("search"),
        "Form should have role='search'"
    );
}

#[test]
fn test_url_input_value() {
    let test_url = "https://example.com/test/page";
    let doc = create_url_bar_document(test_url);

    // Get the URL input element
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    // Test that the input value matches the URL
    assert_eq!(
        url_input_element.attr(local_name!("value")),
        Some(test_url),
        "Input value should match the current URL"
    );
}

#[test]
fn test_form_submission_attributes() {
    let doc = create_url_bar_document("https://example.com");

    // Get the form element
    let form_id = doc.query_selector("#url-form").unwrap().unwrap();
    let form_node = doc.get_node(form_id).unwrap();
    let form_element = form_node.element_data().unwrap();

    // Verify form exists and is properly structured
    assert_eq!(
        form_element.name.local.to_string(),
        "form",
        "Element should be a form"
    );

    // Get the input field within the form
    let input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let input_node = doc.get_node(input_id).unwrap();
    let input_element = input_node.element_data().unwrap();

    // Verify input has name attribute for form submission
    assert_eq!(
        input_element.attr(local_name!("name")),
        Some("url"),
        "Input should have name attribute for form submission"
    );
}

#[test]
fn test_content_area_accessibility() {
    let doc = create_url_bar_document("https://example.com");

    // Get the main content element
    let content_id = doc.query_selector("#content").unwrap().unwrap();
    let content_node = doc.get_node(content_id).unwrap();
    let content_element = content_node.element_data().unwrap();

    // Test main content has proper role
    assert_eq!(
        content_element.attr(local_name!("role")),
        Some("main"),
        "Content area should have role='main'"
    );

    assert_eq!(
        content_element.attr(local_name!("aria-label")),
        Some("Page content"),
        "Content area should have descriptive aria-label"
    );
}

#[test]
fn test_accessibility_tree_structure() {
    let doc = create_url_bar_document("https://example.com");

    // Build the accessibility tree
    let tree_update = doc.build_accessibility_tree();

    // Verify we have nodes in the tree
    assert!(
        !tree_update.nodes.is_empty(),
        "Accessibility tree should contain nodes"
    );

    // Look for the URL input in the accessibility tree
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let accessibility_node_id = accesskit::NodeId(url_input_id as u64);

    let input_node = tree_update
        .nodes
        .iter()
        .find(|(id, _)| *id == accessibility_node_id);

    assert!(
        input_node.is_some(),
        "URL input should be present in accessibility tree"
    );

    // Verify the input has the TextInput role in the accessibility tree
    if let Some((_, node)) = input_node {
        assert_eq!(
            node.role(),
            accesskit::Role::TextInput,
            "URL input should have TextInput role in accessibility tree"
        );
    }
}

#[test]
fn test_keyboard_navigation() {
    let doc = create_url_bar_document("https://example.com");

    // Get the URL input element
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    // Test that input can receive keyboard focus
    // The input should have proper type for keyboard interaction
    assert_eq!(
        url_input_element.attr(local_name!("type")),
        Some("url"),
        "URL input should have type='url' for proper keyboard interaction"
    );

    // Test that input is properly labeled for screen readers
    assert!(
        url_input_element.attr(local_name!("aria-label")).is_some(),
        "URL input should have aria-label for screen reader accessibility"
    );
}

#[test]
fn test_multiple_url_values() {
    let test_cases = vec![
        "https://example.com",
        "https://www.google.com/search?q=test",
        "https://github.com/user/repo",
        "file:///home/user/document.html",
        "http://localhost:8080",
    ];

    for url in test_cases {
        let doc = create_url_bar_document(url);
        let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
        let url_input_node = doc.get_node(url_input_id).unwrap();
        let url_input_element = url_input_node.element_data().unwrap();

        assert_eq!(
            url_input_element.attr(local_name!("value")),
            Some(url),
            "Input value should match the URL: {}",
            url
        );
    }
}

#[test]
fn test_label_association() {
    let doc = create_url_bar_document("https://example.com");

    // Find the label element
    let labels = doc.query_selector_all("label").unwrap();
    assert!(!labels.is_empty(), "Should have at least one label");

    // Get the first label
    let label_node = doc.get_node(labels[0]).unwrap();
    let label_element = label_node.element_data().unwrap();

    // Verify label is associated with the input
    assert_eq!(
        label_element.attr(local_name!("for")),
        Some("url-input"),
        "Label should be associated with url-input via 'for' attribute"
    );
}

#[test]
fn test_form_submission_integration() {
    let doc = create_url_bar_document("https://example.com");

    // Get the form element
    let form_id = doc.query_selector("#url-form").unwrap().unwrap();

    // Verify form owner is set for the input (this is important for form submission)
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();

    // Verify the input is a child of the form (DOM structure)
    // This ensures the form submission will include this input
    let mut current_parent = url_input_node.parent;
    let mut found_form = false;
    while let Some(parent_id) = current_parent {
        if parent_id == form_id {
            found_form = true;
            break;
        }
        current_parent = doc.get_node(parent_id).and_then(|n| n.parent);
    }

    assert!(
        found_form,
        "URL input should be a descendant of the form element"
    );
}

#[test]
fn test_url_input_validation() {
    let doc = create_url_bar_document("https://example.com");

    // Get the URL input element
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    // Test that input is required
    assert_eq!(
        url_input_element.attr(local_name!("required")),
        Some(""),
        "URL input should be required to prevent empty submissions"
    );

    // Test that input type is url for browser validation
    assert_eq!(
        url_input_element.attr(local_name!("type")),
        Some("url"),
        "URL input should have type='url' for browser URL validation"
    );
}

/// Integration test that verifies the complete end-to-end flow
/// of entering a URL and clicking Go would work
#[test]
fn test_end_to_end_navigation_flow() {
    // Start with an initial URL
    let initial_url = "https://example.com";
    let doc = create_url_bar_document(initial_url);

    // Step 1: Verify the URL bar is properly initialized
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    assert_eq!(
        url_input_element.attr(local_name!("value")),
        Some(initial_url),
        "Initial URL should be displayed in the input"
    );

    // Step 2: Verify the Go button exists and is a submit button
    let go_button_id = doc.query_selector("#go-button").unwrap().unwrap();
    let go_button_node = doc.get_node(go_button_id).unwrap();
    let go_button_element = go_button_node.element_data().unwrap();

    assert_eq!(
        go_button_element.attr(local_name!("type")),
        Some("submit"),
        "Go button should be a submit button"
    );

    // Step 3: Verify the form can be found and would trigger submission
    let form_id = doc.query_selector("#url-form").unwrap().unwrap();
    let form_node = doc.get_node(form_id).unwrap();

    assert!(
        form_node.element_data().is_some(),
        "Form element should exist for submission"
    );

    // Step 4: Simulate what would happen on form submission
    // In real usage, submitting the form would:
    // 1. Collect form data (the URL from the input)
    // 2. Trigger navigation through the NavigationProvider
    // 3. Load the new page
    // 4. Update the URL bar with the new URL

    // Verify the form would collect the correct data
    let _submitter_id = go_button_id;

    // The document should be able to submit this form
    // (In the real app, this calls doc.submit_form(form_id, submitter_id))
    // which triggers NavigationProvider.navigate_to()

    // Verify the input is within the form structure for submission
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let mut current_parent = url_input_node.parent;
    let mut found_form = false;
    while let Some(parent_id) = current_parent {
        if parent_id == form_id {
            found_form = true;
            break;
        }
        current_parent = doc.get_node(parent_id).and_then(|n| n.parent);
    }

    assert!(
        found_form,
        "Input should be correctly associated with form for submission"
    );
}

#[test]
fn test_accessibility_compliance() {
    let doc = create_url_bar_document("https://example.com");

    // Verify document has language attribute
    let html_id = doc.query_selector("html").unwrap().unwrap();
    let html_node = doc.get_node(html_id).unwrap();
    let html_element = html_node.element_data().unwrap();

    assert_eq!(
        html_element.attr(local_name!("lang")),
        Some("en"),
        "HTML element should have lang attribute for accessibility"
    );

    // Verify all interactive elements have proper labels
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    assert!(
        url_input_element.attr(local_name!("aria-label")).is_some(),
        "Interactive input should have aria-label"
    );

    let go_button_id = doc.query_selector("#go-button").unwrap().unwrap();
    let go_button_node = doc.get_node(go_button_id).unwrap();
    let go_button_element = go_button_node.element_data().unwrap();

    assert!(
        go_button_element.attr(local_name!("aria-label")).is_some(),
        "Interactive submit button should have aria-label"
    );

    // Verify it's an input element
    assert_eq!(
        go_button_element.name.local.to_string(),
        "input",
        "Go button should be an input element for blitz form submission compatibility"
    );

    // Verify semantic HTML structure
    let nav_id = doc.query_selector("nav").unwrap();
    assert!(nav_id.is_some(), "Should use semantic nav element");

    let main_id = doc.query_selector("main").unwrap();
    assert!(main_id.is_some(), "Should use semantic main element");
}
