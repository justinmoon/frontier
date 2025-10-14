use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::automation::{
    AutomationCommand, AutomationEvent, AutomationResponse, AutomationResult, AutomationStateHandle,
};
use crate::chrome::wrap_with_url_bar;
use crate::js::processor::ScriptExecutionSummary;
use crate::js::runtime_document::RuntimeDocument;
use crate::js::session::JsPageRuntime;
use crate::navigation::{
    execute_fetch, prepare_navigation, FetchRequest, FetchedDocument, NavigationPlan,
};
use crate::WindowRenderer;
use anyhow::{anyhow, Context};
use blitz_dom::net::Resource;
use blitz_dom::{local_name, Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_shell::{BlitzApplication, BlitzShellEvent, View, WindowConfig};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use html_escape::encode_text;
use keyboard_types::Modifiers;
use tokio::runtime::Handle;
use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalPosition;
use winit::event::{
    DeviceId, ElementState, Ime, Modifiers as WinitModifiers, MouseButton, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Theme, WindowId};

#[derive(Debug, Clone)]
pub enum ReadmeEvent {
    Refresh,
    Navigation(Box<NavigationMessage>),
}

fn runtime_document_with_environment(
    runtime: &JsPageRuntime,
    doc: HtmlDocument,
) -> RuntimeDocument {
    let environment = runtime.environment();
    RuntimeDocument::new(doc, environment.clone())
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

struct AutomationBindings {
    state: AutomationStateHandle,
}

pub struct ReadmeApplication {
    inner: BlitzApplication<WindowRenderer>,
    handle: Handle,
    net_provider: Arc<Provider<Resource>>,
    navigation_provider: Arc<dyn NavigationProvider>,
    keyboard_modifiers: WinitModifiers,
    current_input: String,
    current_document: Option<FetchedDocument>,
    current_js_runtime: Option<JsPageRuntime>,
    prepared_document: Option<HtmlDocument>,
    pending_document_reset: bool,
    chrome_handles: Option<DocumentChromeHandles>,
    back_history: Vec<String>,
    forward_history: Vec<String>,
    automation: Option<AutomationBindings>,
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
            pending_document_reset: false,
            chrome_handles: None,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            automation: None,
        }
    }

    #[allow(dead_code)]
    pub fn attach_automation(&mut self, state: AutomationStateHandle) {
        self.automation = Some(AutomationBindings { state });
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
            if let Some(runtime) = self.current_js_runtime.as_mut() {
                let runtime_doc = runtime_document_with_environment(runtime, doc);
                let mut boxed = Box::new(runtime_doc);
                // Attach after boxing to ensure bridge pointer is valid at final heap location
                runtime.attach_document(&mut boxed);
                // Run blocking scripts now that document is attached
                match runtime.run_blocking_scripts() {
                    Ok(Some(summary)) => {
                        self.log_script_summary(&base_url, &summary);
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
                boxed
            } else {
                Box::new(doc)
            };

        self.pending_document_reset = false;
        boxed_document
    }

    fn set_document(&mut self, document: FetchedDocument) {
        self.current_js_runtime = None;
        self.prepared_document = None;
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

        // Note: We don't attach the document here because it will be moved/boxed later.
        // The attachment happens when creating the final RuntimeDocument to ensure
        // the bridge pointer is valid at the document's final heap location.
        // Scripts will be run after the document is properly attached and boxed.

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
        let html = wrap_with_url_bar(contents, &self.current_input, None);
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

            let boxed_document: Box<dyn Document> =
                if let Some(runtime) = self.current_js_runtime.as_mut() {
                    let runtime_doc = runtime_document_with_environment(runtime, doc);
                    let mut boxed = Box::new(runtime_doc);
                    // Attach after boxing to ensure bridge pointer is valid at final heap location
                    runtime.attach_document(&mut boxed);
                    // Run blocking scripts now that document is attached
                    match runtime.run_blocking_scripts() {
                        Ok(Some(summary)) => {
                            self.log_script_summary(&base_url, &summary);
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
                    boxed
                } else {
                    Box::new(doc)
                };

            self.window_mut()
                .replace_document(boxed_document, retain_scroll);

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

        if url_str == "frontier://back" {
            self.go_back();
            return;
        }

        if url_str == "frontier://forward" {
            self.go_forward();
            return;
        }

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
            self.back_history.push(previous);
            self.forward_history.clear();
        }
        self.current_input = target.clone();
        self.spawn_navigation(target, false);
    }

    fn go_back(&mut self) {
        if let Some(target) = self.back_history.pop() {
            let current = self.current_input.clone();
            self.forward_history.push(current);
            self.current_input = target.clone();
            self.spawn_navigation(target, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(target) = self.forward_history.pop() {
            let current = self.current_input.clone();
            self.back_history.push(current);
            self.current_input = target.clone();
            self.spawn_navigation(target, false);
        }
    }

    fn process_automation_commands(&mut self, event_loop: &ActiveEventLoop) {
        let state: AutomationStateHandle = match self.automation.as_ref() {
            Some(bindings) => Arc::clone(&bindings.state),
            None => return,
        };

        loop {
            let task = state.pop();
            let Some(task) = task else { break };
            let (command, responder) = task.into_parts();
            let result = self.execute_automation_command(event_loop, command);
            let _ = responder.send(result);
        }
    }

    fn execute_automation_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        command: AutomationCommand,
    ) -> AutomationResult {
        match command {
            AutomationCommand::Click { selector } => {
                let (window_id, x, y) = self.automation_pointer_for_selector(&selector)?;
                self.automation_dispatch_cursor_move(event_loop, window_id, x, y);
                self.automation_dispatch_mouse_button(event_loop, window_id, ElementState::Pressed);
                self.automation_dispatch_mouse_button(
                    event_loop,
                    window_id,
                    ElementState::Released,
                );
                Ok(AutomationResponse::None)
            }
            AutomationCommand::TypeText { selector, text } => {
                let (window_id, x, y) = self.automation_pointer_for_selector(&selector)?;
                self.automation_dispatch_cursor_move(event_loop, window_id, x, y);
                self.automation_dispatch_mouse_button(event_loop, window_id, ElementState::Pressed);
                self.automation_dispatch_mouse_button(
                    event_loop,
                    window_id,
                    ElementState::Released,
                );

                self.current_input = text.clone();
                for ch in text.chars() {
                    let mut buffer = [0u8; 4];
                    let committed = ch.encode_utf8(&mut buffer).to_string();
                    self.inner.window_event(
                        event_loop,
                        window_id,
                        WindowEvent::Ime(Ime::Commit(committed)),
                    );
                }
                Ok(AutomationResponse::None)
            }
            AutomationCommand::GetText { selector } => {
                let text = self.automation_element_text(&selector)?;
                Ok(AutomationResponse::Text(text))
            }
            AutomationCommand::Pump { duration_ms } => {
                self.automation_pump_for(Duration::from_millis(duration_ms));
                Ok(AutomationResponse::None)
            }
            AutomationCommand::Navigate { target } => {
                self.spawn_navigation(target, false);
                Ok(AutomationResponse::None)
            }
            AutomationCommand::Shutdown => {
                event_loop.exit();
                Ok(AutomationResponse::None)
            }
        }
    }

    fn automation_first_window_id(&self) -> Option<WindowId> {
        self.inner.windows.keys().next().copied()
    }

    fn automation_node_for_selector(
        &mut self,
        selector: &str,
    ) -> anyhow::Result<(WindowId, usize)> {
        let window_id = self
            .automation_first_window_id()
            .ok_or_else(|| anyhow!("automation window not ready"))?;
        let node_id = {
            let view = self
                .inner
                .windows
                .get_mut(&window_id)
                .ok_or_else(|| anyhow!("automation window missing"))?;
            Self::lookup_node(view.doc.as_mut(), selector)?
        };
        Ok((window_id, node_id))
    }

    fn automation_pointer_for_selector(
        &mut self,
        selector: &str,
    ) -> anyhow::Result<(WindowId, f64, f64)> {
        let (window_id, node_id) = self.automation_node_for_selector(selector)?;
        let (x, y) = {
            let view = self
                .inner
                .windows
                .get_mut(&window_id)
                .ok_or_else(|| anyhow!("automation window missing"))?;
            let node = view
                .doc
                .get_node(node_id)
                .ok_or_else(|| anyhow!("automation node disappeared"))?;
            let synthetic = node.synthetic_click_event_data(Modifiers::default());
            (synthetic.x as f64, synthetic.y as f64)
        };
        Ok((window_id, x, y))
    }

    fn automation_element_text(&mut self, selector: &str) -> anyhow::Result<String> {
        let (window_id, node_id) = self.automation_node_for_selector(selector)?;
        let view = self
            .inner
            .windows
            .get_mut(&window_id)
            .ok_or_else(|| anyhow!("automation window missing"))?;
        let text = view
            .doc
            .get_node(node_id)
            .map(|node| node.text_content())
            .unwrap_or_default();
        Ok(text)
    }

    fn automation_dispatch_cursor_move(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        x: f64,
        y: f64,
    ) {
        let logical = LogicalPosition::new(x, y);
        let physical = {
            let scale = self
                .inner
                .windows
                .get(&window_id)
                .map(|view| view.window.scale_factor())
                .unwrap_or(1.0);
            logical.to_physical(scale)
        };
        self.inner.window_event(
            event_loop,
            window_id,
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: physical,
            },
        );
    }

    fn automation_dispatch_mouse_button(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        state: ElementState,
    ) {
        self.inner.window_event(
            event_loop,
            window_id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            },
        );
    }

    fn automation_pump_for(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            for view in self.inner.windows.values_mut() {
                view.poll();
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn lookup_node(doc: &mut dyn Document, selector: &str) -> anyhow::Result<usize> {
        let id = selector
            .strip_prefix('#')
            .ok_or_else(|| anyhow!("only id selectors are supported for automation"))?;

        let mut result = None;
        let root = doc.root_node().id;
        doc.iter_subtree_mut(root, |node_id, document| {
            if result.is_some() {
                return;
            }
            if let Some(node) = document.get_node(node_id) {
                if node.attr(local_name!("id")) == Some(id) {
                    result = Some(node_id);
                }
            }
        });

        result.ok_or_else(|| anyhow!("automation selector not found: {selector}"))
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
                    PhysicalKey::Code(KeyCode::KeyB) => self.go_back(),
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
                    return;
                }

                if event.downcast_ref::<AutomationEvent>().is_some() {
                    self.process_automation_commands(event_loop);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_default_event_loop;
    use crate::navigation::{execute_fetch, FetchRequest, FetchSource};
    use crate::WindowConfig;
    use crate::WindowRenderer;
    use blitz_traits::net::DummyNetCallback;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::runtime::Builder;
    use url::Url;
    use winit::window::WindowAttributes;

    struct NoopNavigationProvider;

    impl NavigationProvider for NoopNavigationProvider {
        fn navigate_to(&self, _opts: NavigationOptions) {}
    }

    #[test]
    #[cfg_attr(
        any(target_os = "macos", target_os = "linux"),
        ignore = "Winit requires the event loop to be created on the main thread"
    )]
    fn react_demo_navigation_replaces_document() {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let (index_doc, timer_doc, display_url, net_provider) = runtime.block_on(async {
            let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
            let index_url = Url::from_file_path(asset_root.join("index.html")).unwrap();
            let timer_url = Url::from_file_path(asset_root.join("timer.html")).unwrap();

            let net_provider = Arc::new(Provider::new(Arc::new(DummyNetCallback)));

            let fetch_index = FetchRequest {
                source: FetchSource::Url(index_url.clone()),
                display_url: index_url.to_string(),
            };
            let index_doc = execute_fetch(&fetch_index, Arc::clone(&net_provider))
                .await
                .expect("fetch index");

            let fetch_timer = FetchRequest {
                source: FetchSource::Url(timer_url.clone()),
                display_url: timer_url.to_string(),
            };
            let timer_doc = execute_fetch(&fetch_timer, Arc::clone(&net_provider))
                .await
                .expect("fetch timer");

            (index_doc, timer_doc, fetch_index.display_url, net_provider)
        });

        let event_loop = create_default_event_loop();
        let proxy = event_loop.create_proxy();
        let nav_provider = Arc::new(NoopNavigationProvider);

        let mut app =
            ReadmeApplication::new(proxy, display_url, Arc::clone(&net_provider), nav_provider);
        app.prepare_initial_state(index_doc);
        let initial_document = app.take_initial_document();
        let renderer = WindowRenderer::new();
        let attrs = WindowAttributes::default().with_title("React demos test harness");
        let window = WindowConfig::with_attributes(initial_document, renderer, attrs);
        app.add_window(window);
        app.render_current_document(false);

        app.handle_navigation_message(NavigationMessage::Completed {
            document: Box::new(timer_doc),
            retain_scroll: false,
        });
        app.render_current_document(false);
    }
}
