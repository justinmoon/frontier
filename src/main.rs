mod input;
mod js;
mod navigation;
mod readme_application;

#[cfg(feature = "gpu")]
use anyrender_vello::VelloWindowRenderer as WindowRenderer;
#[cfg(feature = "cpu-base")]
use anyrender_vello_cpu::VelloCpuWindowRenderer as WindowRenderer;

use anyhow::{anyhow, Context as AnyhowContext};
use blitz_dom::{BaseDocument, Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use notify::{Error as NotifyError, Event as NotifyEvent, RecursiveMode, Watcher as _};
use readme_application::{ReadmeApplication, ReadmeEvent};

use crate::js::processor;
use crate::js::runtime_document::RuntimeDocument;
use crate::js::script::{ScriptDescriptor, ScriptSource};
use crate::js::session::JsPageRuntime;
use crate::navigation::{execute_fetch, prepare_navigation};
use blitz_shell::{
    create_default_event_loop, BlitzApplication, BlitzShellEvent, BlitzShellNetCallback,
    WindowConfig,
};
use blitz_traits::events::UiEvent;
use std::any::Any;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Context as TaskContext;
use std::thread;
use std::time::Duration as StdDuration;
use tracing_subscriber::EnvFilter;
use url::Url;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowAttributes;

enum LaunchMode {
    Standard(String),
    ReactDemo(PathBuf),
}

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
    let mut args = std::env::args().skip(1);
    let launch_mode = match args.next().as_deref() {
        Some("--react-demo") | Some("react-demo") => {
            let demo_path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/react-counter/index.html");
            LaunchMode::ReactDemo(demo_path)
        }
        Some(value) => LaunchMode::Standard(value.to_string()),
        None => LaunchMode::Standard(String::from("https://example.com")),
    };

    let subscriber_result = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
    if subscriber_result.is_err() {
        // tracing was already initialised; continue silently
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let _guard = rt.enter();

    match launch_mode {
        LaunchMode::ReactDemo(path) => {
            if let Err(err) = run_react_demo(&rt, &path) {
                eprintln!("Failed to launch React demo ({}): {err:?}", path.display());
                std::process::exit(1);
            }
        }
        LaunchMode::Standard(raw_input) => {
            if let Err(err) = run_standard_browser(&rt, raw_input) {
                eprintln!("Frontier exited with error: {err:?}");
                std::process::exit(1);
            }
        }
    }
}

fn run_standard_browser(rt: &tokio::runtime::Runtime, raw_input: String) -> anyhow::Result<()> {
    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();

    let net_callback = BlitzShellNetCallback::shared(proxy.clone());
    let net_provider = Arc::new(Provider::new(net_callback));

    let initial_request = rt
        .block_on(prepare_navigation(&raw_input))
        .unwrap_or_else(|err| {
            eprintln!("Failed to prepare initial navigation target: {err}");
            std::process::exit(1);
        });

    let initial_document = rt
        .block_on(execute_fetch(&initial_request, Arc::clone(&net_provider)))
        .unwrap_or_else(|err| {
            eprintln!("Failed to load initial document: {err}");
            std::process::exit(1);
        });

    let title = String::from("Frontier Browser");

    let navigation_provider: Arc<dyn NavigationProvider> = Arc::new(ReadmeNavigationProvider {
        proxy: event_loop.create_proxy(),
    });

    let mut application = ReadmeApplication::new(
        proxy.clone(),
        raw_input.clone(),
        Arc::clone(&net_provider),
        Arc::clone(&navigation_provider),
    );

    let html = application.prepare_initial_state(initial_document.clone());

    let mut doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some(initial_document.base_url.clone()),
            ua_stylesheets: None,
            ..Default::default()
        },
    );

    doc.set_net_provider(net_provider.clone());
    doc.set_navigation_provider(navigation_provider.clone());
    let renderer = WindowRenderer::new();
    let attrs = WindowAttributes::default().with_title(title);
    let window = WindowConfig::with_attributes(Box::new(doc) as _, renderer, attrs);

    application.add_window(window);

    if let Some(path) = initial_document.file_path.clone() {
        let watcher_proxy = proxy.clone();
        let mut watcher =
            notify::recommended_watcher(move |_: Result<NotifyEvent, NotifyError>| {
                let event = ReadmeEvent::Refresh;
                let _ = watcher_proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
            })
            .unwrap();
        watcher.watch(&path, RecursiveMode::NonRecursive).unwrap();
        Box::leak(Box::new(watcher));
    }

    event_loop.run_app(&mut application).unwrap();
    Ok(())
}

fn run_react_demo(_rt: &tokio::runtime::Runtime, demo_path: &Path) -> anyhow::Result<()> {
    let html = std::fs::read_to_string(demo_path)
        .with_context(|| format!("reading demo HTML from {}", demo_path.display()))?;
    let scripts = processor::collect_scripts(&html).context("collecting scripts for React demo")?;

    let mut runtime = JsPageRuntime::new(&html, &scripts)
        .context("initialising React demo runtime")?
        .ok_or_else(|| anyhow!("React demo produced no executable scripts"))?;

    let file_url = Url::from_file_path(demo_path)
        .map_err(|_| anyhow!("React demo path is not a valid file URL"))?;

    let mut html_doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url: Some(file_url.to_string()),
            ..Default::default()
        },
    );

    runtime.attach_document(&mut html_doc);
    let environment = runtime.environment();
    let base_dir = demo_path.parent().unwrap_or_else(|| Path::new("."));
    load_external_scripts(environment.as_ref(), &scripts, base_dir)
        .context("loading external React assets")?;
    if let Some(summary) = runtime
        .run_blocking_scripts()
        .context("executing React demo scripts")?
    {
        tracing::info!(
            target = "quickjs",
            scripts = summary.executed_scripts,
            dom_mutations = summary.dom_mutations,
            "executed React demo scripts"
        );
    }

    for _ in 0..10 {
        runtime
            .environment()
            .pump()
            .context("pumping React demo event loop")?;
        thread::sleep(StdDuration::from_millis(5));
    }

    let document = ReactRuntimeDocument::new(runtime, html_doc);

    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();
    let mut app = BlitzApplication::new(proxy);

    let attrs = WindowAttributes::default().with_title("React Counter Demo");
    let renderer = WindowRenderer::new();
    let window =
        WindowConfig::with_attributes(Box::new(document) as Box<dyn Document>, renderer, attrs);
    app.add_window(window);

    event_loop
        .run_app(&mut app)
        .context("running React counter demo")
}

struct ReactRuntimeDocument {
    #[allow(dead_code)]
    runtime: JsPageRuntime,
    inner: RuntimeDocument,
}

impl ReactRuntimeDocument {
    fn new(runtime: JsPageRuntime, html_doc: HtmlDocument) -> Self {
        let environment = runtime.environment();
        let inner = RuntimeDocument::new(html_doc, environment);
        Self { runtime, inner }
    }
}

impl Deref for ReactRuntimeDocument {
    type Target = BaseDocument;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for ReactRuntimeDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Document for ReactRuntimeDocument {
    fn handle_ui_event(&mut self, event: UiEvent) {
        self.inner.handle_ui_event(event);
    }

    fn poll(&mut self, task_context: Option<TaskContext>) -> bool {
        self.inner.poll(task_context)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner.as_any_mut()
    }

    fn id(&self) -> usize {
        self.inner.id()
    }
}

fn load_external_scripts(
    environment: &crate::js::environment::JsDomEnvironment,
    scripts: &[ScriptDescriptor],
    base_dir: &Path,
) -> anyhow::Result<()> {
    for descriptor in scripts {
        if let ScriptSource::External { src } = &descriptor.source {
            if src.starts_with("http://") || src.starts_with("https://") {
                continue;
            }
            let script_path = if Path::new(src).is_absolute() {
                PathBuf::from(src)
            } else {
                base_dir.join(src)
            };
            let code = std::fs::read_to_string(&script_path)
                .with_context(|| format!("reading external script {}", script_path.display()))?;
            let filename = script_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("external-script.js");
            environment
                .eval(&code, filename)
                .with_context(|| format!("executing external script {}", script_path.display()))?;
        }
    }

    Ok(())
}

pub fn wrap_with_url_bar(content: &str, display_url: &str) -> String {
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
</body>
</html>"#,
        display_url = display_url,
        content = content
    )
}
