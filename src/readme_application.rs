use std::sync::Arc;

use crate::blossom::BlossomFetcher;
use crate::js::processor::ScriptExecutionSummary;
use crate::js::runtime_document::RuntimeDocument;
use crate::js::session::JsPageRuntime;
use crate::navigation::{
    execute_fetch, prepare_navigation, BlossomFetchRequest, FetchRequest, FetchSource,
    FetchedDocument, NavigationPlan, SelectionPrompt,
};
use crate::nns::{ClaimLocation, NnsClaim, NnsResolver};
use crate::storage::unix_timestamp;
use crate::WindowRenderer;
use ::url::Url;
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
        prompt: Option<SelectionPrompt>,
        retain_scroll: bool,
    },
    Prompt {
        prompt: Box<SelectionPrompt>,
        retain_scroll: bool,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct SelectionOverlayState {
    prompt: SelectionPrompt,
    highlighted: usize,
}

pub struct ReadmeApplication {
    inner: BlitzApplication<WindowRenderer>,
    handle: Handle,
    net_provider: Arc<Provider<Resource>>,
    resolver: Arc<NnsResolver>,
    blossom: Arc<BlossomFetcher>,
    navigation_provider: Arc<dyn NavigationProvider>,
    keyboard_modifiers: Modifiers,
    current_input: String,
    current_document: Option<FetchedDocument>,
    current_js_runtime: Option<JsPageRuntime>,
    prepared_document: Option<HtmlDocument>,
    pending_script_summary: Option<ScriptExecutionSummary>,
    pending_document_reset: bool,
    chrome_handles: Option<DocumentChromeHandles>,
    last_overlay_markup: Option<String>,
    selection_overlay: Option<SelectionOverlayState>,
    url_history: Vec<String>,
}

impl ReadmeApplication {
    pub fn new(
        proxy: EventLoopProxy<BlitzShellEvent>,
        initial_input: String,
        net_provider: Arc<Provider<Resource>>,
        navigation_provider: Arc<dyn NavigationProvider>,
        resolver: Arc<NnsResolver>,
        blossom: Arc<BlossomFetcher>,
    ) -> Self {
        Self {
            inner: BlitzApplication::new(proxy),
            handle: Handle::current(),
            net_provider,
            resolver,
            blossom,
            navigation_provider,
            keyboard_modifiers: Default::default(),
            current_input: initial_input,
            current_document: None,
            current_js_runtime: None,
            prepared_document: None,
            pending_script_summary: None,
            pending_document_reset: false,
            chrome_handles: None,
            last_overlay_markup: None,
            selection_overlay: None,
            url_history: Vec::new(),
        }
    }

    pub fn add_window(&mut self, window_config: WindowConfig<WindowRenderer>) {
        self.inner.add_window(window_config);
    }

    pub fn prepare_initial_state(
        &mut self,
        document: FetchedDocument,
        prompt: Option<SelectionPrompt>,
    ) -> String {
        self.set_document(document);
        self.set_selection_prompt(prompt);
        self.compose_html()
    }

    fn set_document(&mut self, document: FetchedDocument) {
        self.current_js_runtime = None;
        self.prepared_document = None;
        self.pending_script_summary = None;
        self.pending_document_reset = true;
        self.chrome_handles = None;
        self.last_overlay_markup = None;
        self.selection_overlay = None;

        self.current_input = document.display_url.clone();

        if !document.scripts.is_empty() {
            match JsPageRuntime::new(&document.contents, &document.scripts) {
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

        let mut prepared_doc = self.build_document_with_chrome(&contents, &base_url, None);

        if let Some(runtime) = self.current_js_runtime.as_mut() {
            runtime.attach_document(&mut *prepared_doc);
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
                self.apply_overlay_markup_to_doc(&mut prepared_doc);
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

    fn set_selection_prompt(&mut self, prompt: Option<SelectionPrompt>) {
        self.selection_overlay = prompt.map(|prompt| SelectionOverlayState {
            highlighted: prompt
                .default_index
                .min(prompt.options.len().saturating_sub(1)),
            prompt,
        });

        if self.prepared_document.is_some() {
            let mut doc = self
                .prepared_document
                .take()
                .expect("prepared document present");
            self.apply_overlay_markup_to_doc(&mut doc);
            self.prepared_document = Some(doc);
        }
    }

    fn compose_html(&self) -> String {
        if let Some(runtime) = &self.current_js_runtime {
            match runtime.environment().document_html() {
                Ok(html) => return html,
                Err(err) => {
                    if let Some(document) = &self.current_document {
                        error!(
                            target = "quickjs",
                            url = %document.base_url,
                            error = %err,
                            "failed to serialize prepared document"
                        );
                    } else {
                        error!(target = "quickjs", error = %err, "failed to serialize prepared document");
                    }
                }
            }
        }

        if let Some(document) = &self.current_document {
            let overlay_html = self
                .selection_overlay
                .as_ref()
                .map(render_selection_overlay);
            crate::wrap_with_url_bar(
                &document.contents,
                &self.current_input,
                overlay_html.as_deref(),
            )
        } else {
            crate::wrap_with_url_bar("<p>Loading…</p>", &self.current_input, None)
        }
    }

    fn window_mut(&mut self) -> &mut View<WindowRenderer> {
        self.inner
            .windows
            .values_mut()
            .next()
            .expect("window available")
    }

    fn build_document_with_chrome(
        &self,
        contents: &str,
        base_url: &str,
        overlay_html: Option<&str>,
    ) -> HtmlDocument {
        let html = crate::wrap_with_url_bar(contents, &self.current_input, overlay_html);
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

    fn apply_overlay_markup_to_doc(&mut self, document: &mut HtmlDocument) {
        let Some(handles) = self.chrome_handles else {
            return;
        };

        let overlay_markup = self
            .selection_overlay
            .as_ref()
            .map(render_selection_overlay);

        if let Err(err) =
            write_overlay_markup(document, handles.overlay_host, overlay_markup.as_deref())
        {
            if let Some(current) = &self.current_document {
                error!(
                    target = "quickjs",
                    url = %current.base_url,
                    error = %err,
                    "failed to apply overlay markup"
                );
            } else {
                error!(target = "quickjs", error = %err, "failed to apply overlay markup");
            }
        } else {
            self.last_overlay_markup = overlay_markup;
        }
    }

    fn render_current_document(&mut self, retain_scroll: bool) {
        if self.current_document.is_none() {
            return;
        }

        let overlay_markup = self
            .selection_overlay
            .as_ref()
            .map(render_selection_overlay);
        let overlay_ref = overlay_markup.as_deref();

        if self.pending_document_reset {
            let (base_url, contents) = {
                let current = self
                    .current_document
                    .as_ref()
                    .expect("current_document must be set");
                (current.base_url.clone(), current.contents.clone())
            };

            let mut doc = self.prepared_document.take().unwrap_or_else(|| {
                self.build_document_with_chrome(&contents, &base_url, overlay_ref)
            });

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

            self.apply_overlay_markup_to_doc(&mut doc);

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

            self.last_overlay_markup = overlay_markup.clone();
            self.pending_document_reset = false;
            return;
        }

        let base_url = self
            .current_document
            .as_ref()
            .map(|doc| doc.base_url.clone())
            .unwrap_or_default();
        let overlay_changed = self.last_overlay_markup.as_deref() != overlay_ref;

        let mut updated_handles: Option<DocumentChromeHandles> = None;
        let overlay_update = if overlay_changed {
            let existing_handles = self.chrome_handles;
            if self.current_js_runtime.is_some() {
                {
                    let view = self.window_mut();
                    let doc = view.downcast_doc_mut::<RuntimeDocument>();
                    let html_doc = doc.html_mut();
                    match ensure_handles(existing_handles, html_doc) {
                        Ok((handles, should_store)) => {
                            if should_store {
                                updated_handles = Some(handles);
                            }
                            write_overlay_markup(html_doc, handles.overlay_host, overlay_ref)
                        }
                        Err(err) => Err(err),
                    }
                }
            } else {
                {
                    let view = self.window_mut();
                    let doc = view.downcast_doc_mut::<HtmlDocument>();
                    match ensure_handles(existing_handles, doc) {
                        Ok((handles, should_store)) => {
                            if should_store {
                                updated_handles = Some(handles);
                            }
                            write_overlay_markup(doc, handles.overlay_host, overlay_ref)
                        }
                        Err(err) => Err(err),
                    }
                }
            }
        } else {
            Ok(())
        };

        if let Some(handles) = updated_handles {
            self.chrome_handles = Some(handles);
        }

        match overlay_update {
            Ok(()) => {
                if overlay_changed {
                    self.last_overlay_markup = overlay_markup.clone();
                }
            }
            Err(err) => {
                error!(
                    target = "quickjs",
                    url = %base_url,
                    error = %err,
                    "failed to update overlay markup"
                );
            }
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
        let resolver = Arc::clone(&self.resolver);
        let net_provider = Arc::clone(&self.net_provider);
        let blossom = Arc::clone(&self.blossom);
        let proxy = self.inner.proxy.clone();

        self.handle.spawn(async move {
            match prepare_navigation(&input, resolver).await {
                Ok(NavigationPlan::Fetch(request)) => {
                    let proxy_clone = proxy.clone();
                    run_fetch_task(request, net_provider, blossom, proxy_clone, retain_scroll)
                        .await;
                }
                Ok(NavigationPlan::RequiresSelection(prompt)) => {
                    let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Prompt {
                        prompt: Box::new(prompt),
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
        });
    }

    fn accept_selection(&mut self) {
        let state = match self.selection_overlay.clone() {
            Some(state) => state,
            None => return,
        };
        let Some(claim) = state.prompt.options.get(state.highlighted).cloned() else {
            return;
        };

        let resolver = Arc::clone(&self.resolver);
        let net_provider = Arc::clone(&self.net_provider);
        let blossom = Arc::clone(&self.blossom);
        let proxy = self.inner.proxy.clone();
        let name = state.prompt.name.clone();
        let display_url = state.prompt.display_url.clone();

        self.selection_overlay = None;
        self.render_current_document(false);

        self.handle.spawn(async move {
            if let Err(err) = resolver.record_selection(&name, &claim.pubkey_hex).await {
                let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Failed {
                    message: err.to_string(),
                }));
                let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                return;
            }

            let fetch_request = match crate::navigation::claim_fetch_request_with_path(
                &claim,
                display_url.clone(),
                &name,
                state.prompt.preferred_path.as_deref(),
            ) {
                Ok(request) => request,
                Err(err) => {
                    let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Failed {
                        message: err.to_string(),
                    }));
                    let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                    return;
                }
            };

            run_fetch_task(fetch_request, net_provider, blossom, proxy, false).await;
        });
    }

    fn move_selection(&mut self, direction: i32) {
        if let Some(state) = &mut self.selection_overlay {
            let len = state.prompt.options.len();
            if len == 0 {
                return;
            }
            let idx = state.highlighted as i32 + direction;
            let wrapped = if idx < 0 {
                len as i32 - 1
            } else {
                idx % len as i32
            };
            state.highlighted = wrapped as usize;
            self.render_current_document(false);
        }
    }

    fn dismiss_selection(&mut self) {
        if self.selection_overlay.take().is_some() {
            self.render_current_document(false);
        }
    }

    fn try_navigate_blossom(&mut self, url: &Url, retain_scroll: bool) -> bool {
        if url.scheme() != "blossom" {
            return false;
        }

        let Some(document) = &self.current_document else {
            return false;
        };
        let Some(context) = &document.blossom else {
            return false;
        };

        if let Some(host) = url.host_str() {
            if !host.is_empty() && host != context.pubkey_hex {
                return false;
            }
        }

        let mut path = url.path().to_string();
        if path.is_empty() {
            path = "/".to_string();
        }

        let fetch_request = FetchRequest {
            source: FetchSource::Blossom(BlossomFetchRequest {
                name: context.name.clone(),
                pubkey_hex: context.pubkey_hex.clone(),
                root_hash: context.root_hash.clone(),
                servers: context.servers.clone(),
                relays: context.relays.clone(),
                path: path.clone(),
                tls_key: context.tls_key.clone(),
                endpoints: context.endpoints.clone(),
            }),
            display_url: blossom_display_label(&context.name, &path),
        };

        let previous = self.current_input.clone();
        if previous != fetch_request.display_url {
            self.url_history.push(previous);
        }
        self.current_input = fetch_request.display_url.clone();

        let net_provider = Arc::clone(&self.net_provider);
        let blossom = Arc::clone(&self.blossom);
        let proxy = self.inner.proxy.clone();

        self.handle.spawn(run_fetch_task(
            fetch_request,
            net_provider,
            blossom,
            proxy,
            retain_scroll,
        ));

        true
    }

    fn handle_navigation_message(&mut self, message: NavigationMessage) {
        match message {
            NavigationMessage::Completed {
                document,
                prompt,
                retain_scroll,
            } => {
                self.set_document(*document);
                self.set_selection_prompt(prompt);
                self.render_current_document(retain_scroll);
            }
            NavigationMessage::Prompt {
                prompt,
                retain_scroll,
            } => {
                self.current_input = prompt.display_url.clone();
                self.set_selection_prompt(Some(*prompt));
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
            blossom: None,
            scripts: Vec::new(),
        };
        self.set_document(document);
        self.selection_overlay = None;
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

        if self.try_navigate_blossom(&url, false) {
            return;
        }

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
            if self.selection_overlay.is_some() {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowUp) | PhysicalKey::Code(KeyCode::KeyK) => {
                        if event.state.is_pressed() {
                            self.move_selection(-1);
                        }
                        return;
                    }
                    PhysicalKey::Code(KeyCode::ArrowDown) | PhysicalKey::Code(KeyCode::KeyJ) => {
                        if event.state.is_pressed() {
                            self.move_selection(1);
                        }
                        return;
                    }
                    PhysicalKey::Code(KeyCode::Enter) => {
                        if event.state.is_pressed() {
                            self.accept_selection();
                        }
                        return;
                    }
                    PhysicalKey::Code(KeyCode::Escape) => {
                        if event.state.is_pressed() {
                            self.dismiss_selection();
                        }
                        return;
                    }
                    _ => {}
                }
            }

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
    overlay_host: usize,
    content_root: usize,
    url_input: usize,
}

impl DocumentChromeHandles {
    fn compute(document: &mut HtmlDocument) -> anyhow::Result<Self> {
        let overlay_host =
            find_node_by_id(document, "overlay-host").context("overlay host element missing")?;
        let content_root =
            find_node_by_id(document, "content").context("content container missing")?;
        let url_input = find_node_by_id(document, "url-input").context("url input missing")?;

        Ok(Self {
            overlay_host,
            content_root,
            url_input,
        })
    }
}

fn ensure_handles(
    existing: Option<DocumentChromeHandles>,
    document: &mut HtmlDocument,
) -> anyhow::Result<(DocumentChromeHandles, bool)> {
    if let Some(handles) = existing {
        let overlay_ok = document.get_node(handles.overlay_host).is_some();
        let content_ok = document.get_node(handles.content_root).is_some();
        let input_ok = document.get_node(handles.url_input).is_some();
        if overlay_ok && content_ok && input_ok {
            return Ok((handles, false));
        }
    }

    let handles = DocumentChromeHandles::compute(document)?;
    Ok((handles, true))
}

fn write_overlay_markup(
    document: &mut HtmlDocument,
    overlay_host: usize,
    overlay_html: Option<&str>,
) -> anyhow::Result<()> {
    if document.get_node(overlay_host).is_none() {
        anyhow::bail!("overlay host node {overlay_host} missing");
    }

    {
        let mut mutator = document.mutate();
        mutator.set_inner_html(overlay_host, overlay_html.unwrap_or(""));
    }

    Ok(())
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

fn render_selection_overlay(state: &SelectionOverlayState) -> String {
    let mut rows = String::new();
    let now = unix_timestamp();

    for (idx, claim) in state.prompt.options.iter().enumerate() {
        let classes = if idx == state.highlighted {
            "overlay-option selected"
        } else {
            "overlay-option"
        };
        let aria_selected = if idx == state.highlighted {
            "true"
        } else {
            "false"
        };
        let published = human_time(now - claim.created_at.as_u64() as i64);
        let relay_count = claim.relays.len();
        let pubkey = abbreviate(&claim.pubkey_npub);
        let note = claim
            .note
            .as_ref()
            .map(|note| format!("<span class=\"overlay-note\">{}</span>", encode_text(note)))
            .unwrap_or_default();
        let location_raw = describe_claim_location(claim);
        let location = encode_text(&location_raw);
        rows.push_str(&format!(
            "<li class=\"{classes}\" role=\"option\" aria-selected=\"{aria_selected}\" tabindex=\"0\">\n                <div class=\"overlay-line\">\n                    <span class=\"overlay-ip\">{location}</span>\n                    <span class=\"overlay-pubkey\">{pubkey}</span>\n                </div>\n                <div class=\"overlay-meta\">Published {published} · {relay_count} relay(s)</div>\n                {note}\n            </li>"
        ));
    }

    let status = if state.prompt.from_cache {
        "Results loaded from cache"
    } else {
        "Fetched live from relays"
    };

    format!(
        "<aside id=\"nns-overlay\" role=\"dialog\" aria-label=\"NNS selection\">\n            <header><h2>Select site for {name}</h2><p>{status}. Use arrows to choose, Enter to confirm.</p></header>\n            <ul role=\"listbox\">{rows}</ul>\n        </aside>",
        name = encode_text(&state.prompt.name),
        status = encode_text(status)
    )
}

fn describe_claim_location(claim: &NnsClaim) -> String {
    match &claim.location {
        ClaimLocation::DirectIp(addr) => addr.to_string(),
        ClaimLocation::Blossom { root_hash, .. } => {
            if root_hash.len() > 12 {
                let prefix = &root_hash[..12];
                format!("blossom:{prefix}…")
            } else {
                format!("blossom:{root_hash}")
            }
        }
        ClaimLocation::LegacyUrl(url) => url.to_string(),
    }
}

fn human_time(delta: i64) -> String {
    if delta <= 0 {
        return "just now".into();
    }
    let minutes = delta / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}d ago")
    } else if hours > 0 {
        format!("{hours}h ago")
    } else if minutes > 0 {
        format!("{minutes}m ago")
    } else {
        format!("{delta}s ago")
    }
}

async fn run_fetch_task(
    request: FetchRequest,
    net_provider: Arc<Provider<Resource>>,
    blossom: Arc<BlossomFetcher>,
    proxy: EventLoopProxy<BlitzShellEvent>,
    retain_scroll: bool,
) {
    match execute_fetch(&request, net_provider, blossom).await {
        Ok(document) => {
            let event = ReadmeEvent::Navigation(Box::new(NavigationMessage::Completed {
                document: Box::new(document),
                prompt: None,
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

fn blossom_display_label(name: &str, path: &str) -> String {
    if path.is_empty() || path == "/" {
        return name.to_string();
    }

    if path.starts_with('/') {
        format!("{name}{path}")
    } else {
        format!("{name}/{path}")
    }
}

fn abbreviate(value: &str) -> String {
    if value.len() <= 12 {
        value.to_string()
    } else {
        format!("{}…{}", &value[..6], &value[value.len() - 4..])
    }
}
