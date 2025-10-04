mod readme_application;

mod markdown {
    pub(crate) const GITHUB_MD_STYLES: &str = include_str!("../assets/github-markdown.css");
    pub(crate) const BLITZ_MD_STYLES: &str = include_str!("../assets/blitz-markdown-overrides.css");

    #[cfg(feature = "comrak")]
    mod comrak;
    #[cfg(feature = "comrak")]
    pub(crate) use comrak::*;

    #[cfg(feature = "pulldown_cmark")]
    mod pulldown_cmark;
    #[cfg(feature = "pulldown_cmark")]
    pub(crate) use pulldown_cmark::*;
}

#[cfg(feature = "gpu")]
use anyrender_vello::VelloWindowRenderer as WindowRenderer;
#[cfg(feature = "cpu-base")]
use anyrender_vello_cpu::VelloCpuWindowRenderer as WindowRenderer;

use blitz_dom::DocumentConfig;
use blitz_dom::net::Resource;
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use blitz_traits::net::Request;
use markdown::{BLITZ_MD_STYLES, GITHUB_MD_STYLES, markdown_to_html};
use notify::{Error as NotifyError, Event as NotifyEvent, RecursiveMode, Watcher as _};
use readme_application::{ReadmeApplication, ReadmeEvent};

use blitz_shell::{
    BlitzShellEvent, BlitzShellNetCallback, WindowConfig, create_default_event_loop,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot;
use url::Url;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowAttributes;

struct ReadmeNavigationProvider {
    proxy: EventLoopProxy<BlitzShellEvent>,
}

impl NavigationProvider for ReadmeNavigationProvider {
    fn navigate_to(&self, opts: NavigationOptions) {
        let _ = self
            .proxy
            .send_event(BlitzShellEvent::Navigate(Box::new(opts)));
    }
}

fn main() {
    let raw_url = std::env::args().nth(1).unwrap_or_else(|| {
        String::from("https://example.com")
    });

    // Turn on the runtime and enter it
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let _guard = rt.enter();

    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();

    let net_callback = BlitzShellNetCallback::shared(proxy.clone());
    let net_provider = Arc::new(Provider::new(net_callback));

    let (base_url, contents, is_md, file_path) =
        rt.block_on(fetch(&raw_url, Arc::clone(&net_provider)));

    // Process markdown if necessary
    let title = String::from("Blitz Browser");
    let html = wrap_with_url_bar(&contents, &base_url, is_md);

    let proxy = event_loop.create_proxy();
    let navigation_provider = ReadmeNavigationProvider {
        proxy: proxy.clone(),
    };
    let navigation_provider = Arc::new(navigation_provider);

    let doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some(base_url.clone()),
            ua_stylesheets: None,
            net_provider: Some(net_provider.clone()),
            navigation_provider: Some(navigation_provider.clone()),
            ..Default::default()
        },
    );
    let renderer = WindowRenderer::new();
    let attrs = WindowAttributes::default().with_title(title);
    let window = WindowConfig::with_attributes(Box::new(doc) as _, renderer, attrs);

    // Create application
    let mut application = ReadmeApplication::new(
        proxy.clone(),
        raw_url.clone(),
        net_provider,
        navigation_provider,
    );
    application.add_window(window);

    if let Some(path) = file_path {
        let mut watcher =
            notify::recommended_watcher(move |_: Result<NotifyEvent, NotifyError>| {
                let event = BlitzShellEvent::Embedder(Arc::new(ReadmeEvent));
                proxy.send_event(event).unwrap();
            })
            .unwrap();

        // Add a path to be watched. All files and directories at that path and
        // below will be monitored for changes.
        watcher.watch(&path, RecursiveMode::NonRecursive).unwrap();

        // Leak watcher to ensure it continues watching. Leaking is unproblematic here as we only create
        // one and we want it to last the entire duration of the program
        Box::leak(Box::new(watcher));
    }

    // Run event loop
    event_loop.run_app(&mut application).unwrap()
}

pub fn wrap_with_url_bar(content: &str, current_url: &str, is_md: bool) -> String {
    let mut body_content = content.to_string();
    let mut styles = String::new();

    if is_md {
        body_content = markdown_to_html(body_content);
        styles.push_str(GITHUB_MD_STYLES);
        styles.push_str("\n");
        styles.push_str(BLITZ_MD_STYLES);
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Blitz Browser - {current_url}</title>
    <style>
        {styles}

        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}

        body {{
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

        #url-input {{
            flex: 1;
            height: 34px;
            padding: 0 12px;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            font-size: 14px;
            outline: none;
        }}

        #url-input:focus {{
            border-color: #0969da;
            box-shadow: 0 0 0 3px rgba(9, 105, 218, 0.1);
        }}

        #go-button {{
            height: 34px;
            padding: 0 16px;
            background: #2da44e;
            color: white;
            border: none;
            border-radius: 6px;
            font-size: 14px;
            font-weight: 500;
            cursor: pointer;
        }}

        #go-button:hover {{
            background: #2c974b;
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
        {body_content}
    </main>
</body>
</html>"#,
        styles = styles,
        current_url = current_url,
        body_content = body_content
    )
}

async fn fetch(
    raw_url: &str,
    net_provider: Arc<Provider<Resource>>,
) -> (String, String, bool, Option<PathBuf>) {
    if let Ok(url) = Url::parse(raw_url) {
        match url.scheme() {
            "file" => fetch_file_path(url.path()),
            _ => fetch_url(url, net_provider).await,
        }
    } else if fs::exists(raw_url).unwrap() {
        fetch_file_path(raw_url)
    } else if let Ok(url) = Url::parse(&format!("https://{raw_url}")) {
        fetch_url(url, net_provider).await
    } else {
        eprintln!("Cannot parse {raw_url} as url or find it as a file");
        std::process::exit(1);
    }
}

async fn fetch_url(
    url: Url,
    net_provider: Arc<Provider<Resource>>,
) -> (String, String, bool, Option<PathBuf>) {
    let (tx, rx) = oneshot::channel();

    let request = Request::get(url);
    net_provider.fetch_with_callback(
        request,
        Box::new(move |result| {
            let result = result.unwrap();
            tx.send(result).unwrap();
        }),
    );

    let (response_url, bytes) = rx.await.unwrap();

    // Detect markdown file
    // let content_type = response
    //     .headers()
    //     .get(HeaderName::from_static("content-type"));
    // || content_type
    //     .is_some_and(|ct| ct.to_str().is_ok_and(|ct| ct.starts_with("text/markdown")));
    let is_md = response_url.ends_with(".md");

    // Get the file content
    let file_content = std::str::from_utf8(&bytes).unwrap().to_string();

    (response_url, file_content, is_md, None)
}

fn fetch_file_path(raw_path: &str) -> (String, String, bool, Option<PathBuf>) {
    let path = std::path::absolute(Path::new(&raw_path)).unwrap();

    // If path is a directory, search for README.md in that directory or any parent directories
    let path = if path.is_dir() {
        let mut maybe_dir: Option<&Path> = Some(path.as_ref());
        loop {
            match maybe_dir {
                Some(dir) => {
                    let rdme_path = dir.join("README.md");
                    if fs::exists(&rdme_path).unwrap() {
                        break rdme_path;
                    }
                    maybe_dir = dir.parent()
                }
                None => {
                    eprintln!("Could not find README.md file in the current directory");
                    std::process::exit(1);
                }
            }
        }
    } else {
        path
    };

    let base_url_path = path.parent().unwrap().to_string_lossy();
    let base_url = format!("file://{base_url_path}/");
    let is_md = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));

    // Read file
    let file_content = std::fs::read_to_string(&path).unwrap();

    (base_url, file_content, is_md, Some(path))
}
