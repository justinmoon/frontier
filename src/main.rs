mod automation;
#[allow(dead_code)]
mod chrome;
mod input;
mod js;
mod navigation;
mod readme_application;

#[cfg(feature = "gpu")]
use anyrender_vello::VelloWindowRenderer as WindowRenderer;
#[cfg(feature = "cpu-base")]
use anyrender_vello_cpu::VelloCpuWindowRenderer as WindowRenderer;

use anyhow::Result;
use blitz_net::Provider;
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use notify::{Error as NotifyError, Event as NotifyEvent, RecursiveMode, Watcher as _};
use readme_application::{ReadmeApplication, ReadmeEvent};

use crate::navigation::{execute_fetch, prepare_navigation, NavigationPlan};
use blitz_shell::{
    create_default_event_loop, BlitzShellEvent, BlitzShellNetCallback, WindowConfig,
};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
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
    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("https://example.com"));

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

    if let Err(err) = run_standard_browser(&rt, target) {
        eprintln!("Frontier exited with error: {err:?}");
        std::process::exit(1);
    }
}

fn run_standard_browser(rt: &tokio::runtime::Runtime, raw_input: String) -> Result<()> {
    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();

    let net_callback = BlitzShellNetCallback::shared(proxy.clone());
    let net_provider = Arc::new(Provider::new(net_callback));

    let initial_plan = rt
        .block_on(prepare_navigation(&raw_input))
        .unwrap_or_else(|err| {
            eprintln!("Failed to prepare initial navigation target: {err}");
            std::process::exit(1);
        });

    let initial_document = match initial_plan {
        NavigationPlan::Fetch(request) => rt
            .block_on(execute_fetch(&request, Arc::clone(&net_provider)))
            .unwrap_or_else(|err| {
                eprintln!("Failed to load initial document: {err}");
                std::process::exit(1);
            }),
    };

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

    application.prepare_initial_state(initial_document.clone());

    let doc = application.take_initial_document();
    let renderer = WindowRenderer::new();
    let attrs = WindowAttributes::default().with_title(title);
    let window = WindowConfig::with_attributes(doc, renderer, attrs);

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
