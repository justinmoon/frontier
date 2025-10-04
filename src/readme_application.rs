use std::sync::Arc;

use crate::navigation::{
    execute_fetch, prepare_navigation, FetchRequest, FetchedDocument, NavigationPlan,
    SelectionPrompt,
};
use crate::nns::NnsResolver;
use crate::storage::unix_timestamp;
use crate::WindowRenderer;
use ::url::Url;
use blitz_dom::net::Resource;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_shell::{BlitzApplication, BlitzShellEvent, View, WindowConfig};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use html_escape::encode_text;
use tokio::runtime::Handle;
use winit::application::ApplicationHandler;
use winit::event::{Modifiers, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Theme, WindowId};

#[derive(Debug, Clone)]
pub enum ReadmeEvent {
    Refresh,
    Navigation(NavigationMessage),
}

#[derive(Debug, Clone)]
pub enum NavigationMessage {
    Completed {
        document: FetchedDocument,
        prompt: Option<SelectionPrompt>,
        retain_scroll: bool,
    },
    Prompt {
        prompt: SelectionPrompt,
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
    navigation_provider: Arc<dyn NavigationProvider>,
    keyboard_modifiers: Modifiers,
    current_input: String,
    current_document: Option<FetchedDocument>,
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
    ) -> Self {
        Self {
            inner: BlitzApplication::new(proxy),
            handle: Handle::current(),
            net_provider,
            resolver,
            navigation_provider,
            keyboard_modifiers: Default::default(),
            current_input: initial_input,
            current_document: None,
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
        self.current_input = document.display_url.clone();
        self.current_document = Some(document);
    }

    fn set_selection_prompt(&mut self, prompt: Option<SelectionPrompt>) {
        self.selection_overlay = prompt.map(|prompt| SelectionOverlayState {
            highlighted: prompt
                .default_index
                .min(prompt.options.len().saturating_sub(1)),
            prompt,
        });
    }

    fn compose_html(&self) -> String {
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

    fn render_current_document(&mut self, retain_scroll: bool) {
        if let Some(document) = &self.current_document {
            let html = self.compose_html();
            let doc = HtmlDocument::from_html(
                &html,
                DocumentConfig {
                    base_url: Some(document.base_url.clone()),
                    ua_stylesheets: None,
                    net_provider: Some(self.net_provider.clone()),
                    navigation_provider: Some(self.navigation_provider.clone()),
                    ..Default::default()
                },
            );
            self.window_mut()
                .replace_document(Box::new(doc) as _, retain_scroll);
        }
    }

    fn reload_document(&mut self, retain_scroll: bool) {
        let input = self.current_input.clone();
        self.spawn_navigation(input, retain_scroll);
    }

    fn spawn_navigation(&mut self, input: String, retain_scroll: bool) {
        let resolver = Arc::clone(&self.resolver);
        let net_provider = Arc::clone(&self.net_provider);
        let proxy = self.inner.proxy.clone();

        self.handle.spawn(async move {
            match prepare_navigation(&input, resolver).await {
                Ok(NavigationPlan::Fetch(request)) => {
                    match execute_fetch(&request, net_provider).await {
                        Ok(document) => {
                            let event = ReadmeEvent::Navigation(NavigationMessage::Completed {
                                document,
                                prompt: None,
                                retain_scroll,
                            });
                            let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                        }
                        Err(err) => {
                            let event = ReadmeEvent::Navigation(NavigationMessage::Failed {
                                message: err.to_string(),
                            });
                            let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                        }
                    }
                }
                Ok(NavigationPlan::RequiresSelection(prompt)) => {
                    let event = ReadmeEvent::Navigation(NavigationMessage::Prompt {
                        prompt,
                        retain_scroll,
                    });
                    let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                }
                Err(err) => {
                    let event = ReadmeEvent::Navigation(NavigationMessage::Failed {
                        message: err.to_string(),
                    });
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
        let proxy = self.inner.proxy.clone();
        let name = state.prompt.name.clone();
        let display_url = state.prompt.display_url.clone();

        self.selection_overlay = None;
        self.render_current_document(false);

        self.handle.spawn(async move {
            if let Err(err) = resolver.record_selection(&name, &claim.pubkey_hex).await {
                let event = ReadmeEvent::Navigation(NavigationMessage::Failed {
                    message: err.to_string(),
                });
                let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                return;
            }

            let fetch_request = FetchRequest {
                fetch_url: Url::parse(&format!("http://{}", claim.socket_addr))
                    .expect("valid socket"),
                display_url,
            };

            match execute_fetch(&fetch_request, net_provider).await {
                Ok(document) => {
                    let event = ReadmeEvent::Navigation(NavigationMessage::Completed {
                        document,
                        prompt: None,
                        retain_scroll: false,
                    });
                    let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                }
                Err(err) => {
                    let event = ReadmeEvent::Navigation(NavigationMessage::Failed {
                        message: err.to_string(),
                    });
                    let _ = proxy.send_event(BlitzShellEvent::Embedder(Arc::new(event)));
                }
            }
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

    fn handle_navigation_message(&mut self, message: NavigationMessage) {
        match message {
            NavigationMessage::Completed {
                document,
                prompt,
                retain_scroll,
            } => {
                self.set_document(document);
                self.set_selection_prompt(prompt);
                self.render_current_document(retain_scroll);
            }
            NavigationMessage::Prompt {
                prompt,
                retain_scroll,
            } => {
                self.current_input = prompt.display_url.clone();
                self.set_selection_prompt(Some(prompt));
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
        let url_str = options.url.to_string();
        let target = if url_str.contains("?url=") {
            if let Some(query) = options.url.query() {
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
                            self.handle_navigation_message(message.clone())
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
        rows.push_str(&format!(
            "<li class=\"{classes}\" role=\"option\" aria-selected=\"{aria_selected}\" tabindex=\"0\">\n                <div class=\"overlay-line\">\n                    <span class=\"overlay-ip\">{ip}</span>\n                    <span class=\"overlay-pubkey\">{pubkey}</span>\n                </div>\n                <div class=\"overlay-meta\">Published {published} · {relay_count} relay(s)</div>\n                {note}\n            </li>",
            ip = encode_text(&claim.socket_addr.to_string())
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

fn abbreviate(value: &str) -> String {
    if value.len() <= 12 {
        value.to_string()
    } else {
        format!("{}…{}", &value[..6], &value[value.len() - 4..])
    }
}
