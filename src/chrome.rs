pub fn wrap_with_url_bar(content: &str, display_url: &str, overlay_html: Option<&str>) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Frontier Browser - {display_url}</title>
    <style>
        * {{
            box-sizing: border-box;
        }}

        html, body {{
            margin: 0;
            padding: 0;
            width: 100%;
            height: 100%;
            display: flex;
            flex-direction: column;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
        }}

        #url-bar-container {{
            position: fixed;
            top: 0;
            left: 0;
            right: 0;
            height: 50px;
            background: #f6f8fa;
            border-bottom: 1px solid #d0d7de;
            display: flex;
            align-items: center;
            padding: 8px 12px;
            gap: 8px;
            z-index: 1000;
        }}

        .nav-button {{
            width: 32px;
            height: 32px;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            background: white;
            color: #24292f;
            font-size: 18px;
            line-height: 1;
            display: flex;
            align-items: center;
            justify-content: center;
            cursor: pointer;
        }}

        .nav-button:hover {{
            background: #eaeef2;
        }}

        .nav-button:active {{
            background: #d0d7de;
        }}

        .nav-button:disabled {{
            opacity: 0.4;
            cursor: not-allowed;
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

        
        #nns-overlay {{
            position: fixed;
            top: 60px;
            left: 50%;
            transform: translateX(-50%);
            width: min(560px, 92%);
            background: #ffffff;
            border: 1px solid #d0d7de;
            border-radius: 12px;
            box-shadow: 0 12px 32px rgba(15, 23, 42, 0.18);
            padding: 16px 18px;
            z-index: 1200;
        }}

        #nns-overlay header {{
            margin-bottom: 12px;
        }}

        #nns-overlay h2 {{
            margin: 0;
            font-size: 18px;
            font-weight: 600;
        }}

        #nns-overlay p {{
            margin: 4px 0 0;
            font-size: 13px;
            color: #57606a;
        }}

        #nns-overlay ul {{
            list-style: none;
            margin: 12px 0 0;
            padding: 0;
            max-height: 340px;
            overflow-y: auto;
        }}

        .overlay-option {{
            padding: 12px;
            border-radius: 8px;
            border: 1px solid transparent;
            margin-bottom: 8px;
            cursor: pointer;
            background: #f9fafb;
        }}

        .overlay-option:last-child {{
            margin-bottom: 0;
        }}

        .overlay-option:hover,
        .overlay-option.selected {{
            background: #f0f6ff;
            border-color: #0969da;
        }}

        .overlay-line {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            font-weight: 600;
            font-size: 14px;
        }}

        .overlay-ip {{
            font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
        }}

        .overlay-pubkey {{
            color: #57606a;
            font-size: 12px;
            margin-left: 12px;
        }}

        .overlay-meta {{
            font-size: 12px;
            color: #57606a;
            margin-top: 6px;
        }}

        .overlay-note {{
            display: block;
            margin-top: 8px;
            font-size: 13px;
            color: #1f2328;
        }}

        #go-button:active {{
            background: #298e46;
        }}

        #content {{
            margin-top: 50px;
            padding: 20px;
        }}
    </style>
</head>
<body>
    <nav id="url-bar-container" role="navigation" aria-label="Browser navigation">
        <button id="back-button" class="nav-button" title="Back" aria-label="Go back" type="button">&larr;</button>
        <button id="forward-button" class="nav-button" title="Forward" aria-label="Go forward" type="button">&rarr;</button>
        <form id="url-form" style="display: flex; flex: 1; gap: 8px;" role="search">
            <label for="url-input" class="sr-only" style="position: absolute; left: -10000px;">
                Enter website URL
            </label>
            <input
                type="url"
                id="url-input"
                name="url"
                value="{display_url}"
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
        {content}
    </main>
    <div id="overlay-host">
        {overlay}
    </div>
    <script>
        (function() {{
            const form = document.getElementById('url-form');
            const input = document.getElementById('url-input');
            const goButton = document.getElementById('go-button');
            const backButton = document.getElementById('back-button');
            const forwardButton = document.getElementById('forward-button');

            const navigate = (target) => {{
                if (!target) {{
                    return;
                }}
                window.location.href = target;
            }};

            form?.addEventListener('submit', (event) => {{
                event.preventDefault();
                navigate(input?.value || '');
            }});

            goButton?.addEventListener('click', (event) => {{
                event.preventDefault();
                navigate(input?.value || '');
            }});

            backButton?.addEventListener('click', (event) => {{
                event.preventDefault();
                navigate('frontier://back');
            }});

            forwardButton?.addEventListener('click', (event) => {{
                event.preventDefault();
                navigate('frontier://forward');
            }});
        }})();
    </script>
</body>
</html>"#,
        display_url = display_url,
        content = content,
        overlay = overlay_html.unwrap_or("")
    )
}
