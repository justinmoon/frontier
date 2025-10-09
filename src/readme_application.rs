use std::sync::Arc;

use crate::js::processor::ScriptExecutionSummary;
use crate::js::runtime_document::RuntimeDocument;
use crate::js::session::JsPageRuntime;
use crate::navigation::{
    execute_fetch, prepare_navigation, FetchRequest, FetchedDocument, NavigationPlan,
};
use crate::WindowRenderer;
use anyhow::Context;
use blitz_dom::net::Resource;
use blitz_dom::{local_name, Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_shell::{BlitzApplication, BlitzShellEvent, View, WindowConfig};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use html_escape::encode_text;
use tokio::runtime::Handle;
use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::event::{Modifiers, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Theme, WindowId};

#[derive(Debug, Clone)]
pub enum ReadmeEvent {
    Refresh,
    Navigation(Box<NavigationMessage>),
}

#[derive(Debug, Clone)]
pub enum NavigationMessage {
    Completed {
        document: Box<FetchedDocument>,
        retain_scroll: bool,
    },
    Failed {
        message: String,
    },
}

pub struct ReadmeApplication {
    inner: BlitzApplication<WindowRenderer>,
    handle: Handle,
    net_provider: Arc<Provider<Resource>>,
    navigation_provider: Arc<dyn NavigationProvider>,
    keyboard_modifiers: Modifiers,
    current_input: String,
    current_document: Option<FetchedDocument>,
    current_js_runtime: Option<JsPageRuntime>,
    prepared_document: Option<HtmlDocument>,
    pending_script_summary: Option<ScriptExecutionSummary>,
    pending_document_reset: bool,
    chrome_handles: Option<DocumentChromeHandles>,
    url_history: Vec<String>,
}

impl ReadmeApplication {
    pub fn new(
        proxy: EventLoopProxy<BlitzShellEvent>,
        initial_input: String,
        net_provider: Arc<Provider<Resource>>,
        navigation_provider: Arc<dyn NavigationProvider>,
    ) -> Self {
        Self {
            inner: BlitzApplication::new(proxy),
            handle: Handle::current(),
            net_provider,
            navigation_provider,
            keyboard_modifiers: Default::default(),
            current_input: initial_input,
            current_document: None,
            current_js_runtime: None,
            prepared_document: None,
            pending_script_summary: None,
            pending_document_reset: false,
            chrome_handles: None,
            url_history: Vec::new(),
        }
    }

    pub fn add_window(&mut self, window_config: WindowConfig<WindowRenderer>) {
        self.inner.add_window(window_config);
    }

    pub fn prepare_initial_state(&mut self, document: FetchedDocument) {
        self.set_document(document);
    }

    pub fn take_initial_document(&mut self) -> Box<dyn Document> {
        let (base_url, contents) = {
            let current = self
                .current_document
                .as_ref()
                .expect("prepare_initial_state must be called first");
            (current.base_url.clone(), current.contents.clone())
        };

        let mut doc = self
            .prepared_document
            .take()
            .unwrap_or_else(|| self.build_document_with_chrome(&contents, &base_url));

        if self.chrome_handles.is_none() {
            match DocumentChromeHandles::compute(&mut doc) {
                Ok(handles) => self.chrome_handles = Some(handles),
                Err(err) => {
                    error!(
                        target = "quickjs",
                        url = %base_url,
                        error = %err,
                        "failed to compute chrome handles"
                    );
                }
            }
        }

        let boxed_document: Box<dyn Document> =
            if let Some(runtime) = self.current_js_runtime.as_ref() {
                let environment = runtime.environment();
                // Moving the document into RuntimeDocument changes its memory location.
                // We need to box it first to get its final heap location, then reattach.
                let mut boxed = Box::new(RuntimeDocument::new(doc, environment.clone()));
                // Get mutable reference to the BaseDocument at its final location
                use std::ops::DerefMut;
                environment.reattach_document(boxed.deref_mut());
                boxed as Box<dyn Document>
            } else {
                Box::new(doc) as Box<dyn Document>
            };

        if let Some(summary) = self.pending_script_summary.take() {
            self.log_script_summary(&base_url, &summary);
        }

        self.pending_document_reset = false;
        boxed_document
    }

    fn set_document(&mut self, document: FetchedDocument) {
        self.current_js_runtime = None;
        self.prepared_document = None;
        self.pending_script_summary = None;
        self.pending_document_reset = true;
        self.chrome_handles = None;

        self.current_input = document.display_url.clone();

        if !document.scripts.is_empty() {
            match JsPageRuntime::new(
                &document.contents,
                &document.scripts,
                Some(document.base_url.as_str()),
            ) {
                Ok(Some(runtime)) => {
                    self.current_js_runtime = Some(runtime);
                }
                Ok(None) => {}
                Err(err) => {
                    error!(
                        target = "quickjs",
                        url = %document.base_url,
                        error = %err,
                        "failed to initialize page runtime"
                    );
                }
            }
        }

        let base_url = document.base_url.clone();
        let contents = document.contents.clone();

        let mut prepared_doc = self.build_document_with_chrome(&contents, &base_url);

        if let Some(runtime) = self.current_js_runtime.as_mut() {
            runtime.attach_document(&mut prepared_doc);
            match runtime.run_blocking_scripts() {
                Ok(Some(summary)) => {
                    self.pending_script_summary = Some(summary);
                }
                Ok(None) => {}
                Err(err) => {
                    error!(
                        target = "quickjs",
                        url = %base_url,
                        error = %err,
                        "failed to execute blocking scripts"
                    );
                }
            }
        }

        match DocumentChromeHandles::compute(&mut prepared_doc) {
            Ok(handles) => {
                self.chrome_handles = Some(handles);
            }
            Err(err) => {
                error!(
                    target = "quickjs",
                    url = %base_url,
                    error = %err,
                    "failed to compute chrome handles"
                );
                self.chrome_handles = None;
            }
        }

        self.prepared_document = Some(prepared_doc);
        self.current_document = Some(document);
    }

    fn log_script_summary(&self, base_url: &str, summary: &ScriptExecutionSummary) {
        info!(
            target = "quickjs",
            url = %base_url,
            scripts = summary.executed_scripts,
            dom_mutations = summary.dom_mutations,
            "executed blocking inline scripts"
        );
    }

    fn window_mut(&mut self) -> &mut View<WindowRenderer> {
        self.inner
            .windows
            .values_mut()
            .next()
            .expect("window available")
    }

    fn build_document_with_chrome(&self, contents: &str, base_url: &str) -> HtmlDocument {
        let html = crate::wrap_with_url_bar(contents, &self.current_input, None);
        HtmlDocument::from_html(
            &html,
            DocumentConfig {
                base_url: Some(base_url.to_string()),
                ua_stylesheets: None,
                net_provider: Some(self.net_provider.clone()),
                navigation_provider: Some(self.navigation_provider.clone()),
                ..Default::default()
            },
        )
    }

    fn render_current_document(&mut self, retain_scroll: bool) {
        if self.current_document.is_none() {
            return;
        }

        if self.pending_document_reset {
            let (base_url, contents) = {
                let current = self
                    .current_document
                    .as_ref()
                    .expect("current_document must be set");
                (current.base_url.clone(), current.contents.clone())
            };

            let mut doc = self
                .prepared_document
                .take()
                .unwrap_or_else(|| self.build_document_with_chrome(&contents, &base_url));

            if self.chrome_handles.is_none() {
                match DocumentChromeHandles::compute(&mut doc) {
                    Ok(handles) => self.chrome_handles = Some(handles),
                    Err(err) => {
                        error!(
                            target = "quickjs",
                            url = %base_url,
                            error = %err,
                            "failed to compute chrome handles"
                        );
                    }
                }
            }

            let boxed_document: Box<dyn Document> = if let Some(environment) = self
                .current_js_runtime
                .as_ref()
                .map(|runtime| runtime.environment())
            {
                Box::new(RuntimeDocument::new(doc, environment)) as Box<dyn Document>
            } else {
                Box::new(doc) as Box<dyn Document>
            };

            self.window_mut()
                .replace_document(boxed_document, retain_scroll);

            if let Some(summary) = self.pending_script_summary.take() {
                self.log_script_summary(&base_url, &summary);
            }

            self.pending_document_reset = false;
            return;
        }

        {
            let view = self.window_mut();
            view.poll();
            view.request_redraw();
        }
    }

    fn reload_document(&mut self, retain_scroll: bool) {
        let input = self.current_input.clone();
        self.spawn_navigation(input, retain_scroll);
    }

    fn spawn_navigation(&mut self, input: String, retain_scroll: bool) {
        let net_provider = Arc::clone(&self.net_provider);
        let proxy = self.inner.proxy.clone();

        self.handle.spawn(async move {
            match prepare_navigation(&input).await {
                Ok(NavigationPlan::Fetch(request)) => {
                    let proxy_clone = proxy.clone();
                    run_fetch_task(request, net_provider, proxy_clone, retain_scroll).await;
                }
                Err(err) => {
                    let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Failed {
                        message: err.to_string(),
                    }));
                    let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                }
            }
        });
    }

    fn handle_navigation_message(&mut self, message: NavigationMessage) {
        match message {
            NavigationMessage::Completed {
                document,
                retain_scroll,
            } => {
                self.set_document(*document);
                self.render_current_document(retain_scroll);
            }
            NavigationMessage::Failed { message } => {
                self.show_error(&message);
            }
        }
    }

    fn show_error(&mut self, message: &str) {
        let escaped = encode_text(message);
        let html = format!(
            "<section class=\"error\"><h2>Navigation failed</h2><p>{escaped}</p></section>"
        );
        let document = FetchedDocument {
            base_url: "about:error".into(),
            contents: html,
            file_path: None,
            display_url: self.current_input.clone(),
            scripts: Vec::new(),
        };
        self.set_document(document);
        self.render_current_document(false);
    }

    fn toggle_theme(&mut self) {
        let window = self.window_mut();
        let new_theme = match window.current_theme() {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        };
        window.set_theme_override(Some(new_theme));
    }

    fn navigate(&mut self, options: NavigationOptions) {
        let url = options.url.clone();
        let url_str = url.to_string();
        let target = if url_str.contains("?url=") {
            if let Some(query) = url.query() {
                ::url::form_urlencoded::parse(query.as_bytes())
                    .find(|(key, _)| key == "url")
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or(url_str)
            } else {
                url_str
            }
        } else {
            url_str
        };

        let previous = self.current_input.clone();
        if previous != target {
            self.url_history.push(previous);
        }
        self.current_input = target.clone();
        self.spawn_navigation(target, false);
    }
}

impl ApplicationHandler<BlitzShellEvent> for ReadmeApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::ModifiersChanged(new_state) = &event {
            self.keyboard_modifiers = *new_state;
        }

        if let WindowEvent::KeyboardInput { event, .. } = &event {
            let mods = self.keyboard_modifiers.state();
            if !event.state.is_pressed() && (mods.control_key() || mods.super_key()) {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::KeyR) => self.reload_document(true),
                    PhysicalKey::Code(KeyCode::KeyT) => self.toggle_theme(),
                    PhysicalKey::Code(KeyCode::KeyB) => {
                        if let Some(url) = self.url_history.pop() {
                            self.current_input = url;
                            self.reload_document(false);
                        }
                    }
                    _ => {}
                }
            }
        }

        self.inner.window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: BlitzShellEvent) {
        match event {
            BlitzShellEvent::Embedder(event) => {
                if let Some(event) = event.downcast_ref::<ReadmeEvent>() {
                    match event {
                        ReadmeEvent::Refresh => self.reload_document(true),
                        ReadmeEvent::Navigation(message) => {
                            self.handle_navigation_message((**message).clone())
                        }
                    }
                }
            }
            BlitzShellEvent::Navigate(options) => {
                self.navigate(*options);
            }
            other => self.inner.user_event(event_loop, other),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DocumentChromeHandles {
    #[allow(dead_code)]
    content_root: usize,
    #[allow(dead_code)]
    url_input: usize,
}

impl DocumentChromeHandles {
    fn compute(document: &mut HtmlDocument) -> anyhow::Result<Self> {
        let content_root =
            find_node_by_id(document, "content").context("content container missing")?;
        let url_input = find_node_by_id(document, "url-input").context("url input missing")?;

        Ok(Self {
            content_root,
            url_input,
        })
    }
}

fn find_node_by_id(document: &mut HtmlDocument, target: &str) -> Option<usize> {
    let mut result = None;
    let root_id = document.root_node().id;
    document.iter_subtree_mut(root_id, |node_id, doc| {
        if result.is_some() {
            return;
        }
        if let Some(node) = doc.get_node(node_id) {
            if let Some(id_attr) = node.attr(local_name!("id")) {
                if id_attr == target {
                    result = Some(node_id);
                }
            }
        }
    });
    result
}

async fn run_fetch_task(
    request: FetchRequest,
    net_provider: Arc<Provider<Resource>>,
    proxy: EventLoopProxy<BlitzShellEvent>,
    retain_scroll: bool,
) {
    match execute_fetch(&request, net_provider).await {
        Ok(document) => {
            let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Completed {
                document: Box::new(document),
                retain_scroll,
            }));
            let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
        }
        Err(err) => {
            let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Failed {
                message: err.to_string(),
            }));
            let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
        }
    }
}
