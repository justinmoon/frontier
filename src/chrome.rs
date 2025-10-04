/// Browser chrome (UI) management
/// This module handles the browser chrome (URL bar, etc.)
///
/// Architecture:
/// - Chrome is a fixed HTML document with a #content div
/// - Fetched HTML is injected into #content using set_inner_html()
/// - Content stays pure, chrome handles layout
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

#[allow(dead_code)]
pub const CHROME_HEIGHT: f64 = 50.0;

/// Creates a chrome document with empty content area
/// Use set_content() to inject the fetched HTML
#[allow(dead_code)]
pub fn create_chrome_document(current_url: &str) -> HtmlDocument {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Frontier Browser Chrome</title>
    <style>
        * {{
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }}

        html, body {{
            width: 100%;
            height: {chrome_height}px;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
            background: #f6f8fa;
            border-bottom: 1px solid #d0d7de;
        }}

        #url-bar-container {{
            width: 100%;
            height: 100%;
            display: flex;
            align-items: center;
            padding: 8px 12px;
            gap: 8px;
        }}

        #url-form {{
            width: 100%;
            display: flex;
            gap: 8px;
        }}

        #url-input {{
            flex: 1;
            height: 34px;
            padding: 0 12px;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            font-size: 14px;
            line-height: 34px;
            outline: none;
            background: white;
        }}

        #url-input:focus {{
            border-color: #0969da;
            box-shadow: 0 0 0 3px rgba(9, 105, 218, 0.3);
        }}

        #go-button {{
            height: 34px;
            padding: 0 16px;
            background: #2da44e;
            color: white;
            border: 1px solid rgba(27, 31, 36, 0.15);
            border-radius: 6px;
            font-size: 14px;
            font-weight: 500;
            line-height: 34px;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
        }}

        #go-button:hover {{
            background: #2c974b;
        }}

        #go-button:active {{
            background: #298e46;
        }}

        #content {{
            position: absolute;
            top: {chrome_height}px;
            left: 0;
            right: 0;
            bottom: 0;
            width: 100%;
            height: calc(100vh - {chrome_height}px);
            overflow: auto;
            padding: 20px;
        }}
    </style>
</head>
<body>
    <nav id="url-bar-container" role="navigation" aria-label="Browser navigation">
        <form id="url-form" style="display: flex; flex: 1; gap: 8px;" role="search">
            <label for="url-input" class="sr-only" style="position: absolute; left: -10000px;">
                Enter website URL
            </label>
            <input
                type="url"
                id="url-input"
                name="url"
                value="{current_url}"
                autofocus
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
        <!-- Content injected here via set_inner_html() -->
    </main>
</body>
</html>"#,
        chrome_height = CHROME_HEIGHT,
        current_url = current_url
    );

    HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some("chrome://browser".to_string()),
            ..Default::default()
        },
    )
}

/// Inject content HTML into the chrome document's #content div
#[allow(dead_code)]
pub fn set_content(doc: &mut HtmlDocument, content_html: &str) {
    let content_node = doc
        .query_selector("#content")
        .expect("Chrome document should have #content div")
        .expect("Should find #content");
    doc.mutate().set_inner_html(content_node, content_html);
}
