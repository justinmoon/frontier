# Dioxus Chrome + Blitz Content Architecture Spec

## Overview
Chrome (URL bar, UI controls) rendered by Dioxus. Content (web pages) rendered by Blitz. Two completely separate rendering contexts that cannot interfere.

## Architecture Diagram

```
┌─────────────────────────────────────────┐
│  Winit Window                           │
│  ┌───────────────────────────────────┐  │
│  │ Dioxus Chrome (Top 50px)          │  │
│  │ ┌──────────────────┬────────┐    │  │
│  │ │ [URL Input Box]  │ [Go]   │    │  │
│  │ └──────────────────┴────────┘    │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │ Blitz Content (Remaining space)   │  │
│  │                                   │  │
│  │  <rendered web page here>         │  │
│  │                                   │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

## Component Breakdown

### 1. Window Management (src/main.rs changes)

Currently creates one Blitz document. Need to create:
- One Dioxus VirtualDom for chrome
- One Blitz HtmlDocument for content
- Both render to same winit window

### 2. Rendering Strategy

**Option A: Two separate render passes**
```rust
fn render_frame() {
    // 1. Render Dioxus chrome to top 50px
    dioxus_vdom.render_to_viewport(Rect { x: 0, y: 0, width: window_width, height: 50 });

    // 2. Render Blitz content to remaining space
    blitz_doc.render_to_viewport(Rect { x: 0, y: 50, width: window_width, height: window_height - 50 });
}
```

**Option B: Dioxus owns window, embeds Blitz as "custom element"**
```rust
// Dioxus component
fn Chrome(cx: Scope) -> Element {
    cx.render(rsx! {
        div {
            class: "chrome",
            input { id: "url-input", value: "{url}" }
            button { onclick: |_| navigate(), "Go" }
        }
        BlitzViewport { document: blitz_doc }  // Custom component that renders Blitz
    })
}
```

### 3. File Structure

```
src/
  main.rs              - Setup both Dioxus and Blitz
  chrome.rs            - Dioxus chrome component (delete current impl)
  content.rs           - Blitz document management
  bridge.rs            - Communication between Dioxus <-> Blitz
  readme_application.rs - Updated to manage both renderers
```

### 4. Chrome Component (src/chrome.rs - NEW)

```rust
use dioxus::prelude::*;

pub struct ChromeState {
    pub current_url: String,
    pub is_loading: bool,
}

#[component]
pub fn Chrome(cx: Scope, on_navigate: EventHandler<String>) -> Element {
    let url_input = use_state(cx, || "".to_string());

    cx.render(rsx! {
        nav {
            id: "chrome-bar",
            style: "height: 50px; background: #f6f8fa; border-bottom: 1px solid #d0d7de; display: flex; padding: 8px 12px; gap: 8px;",

            input {
                r#type: "url",
                id: "url-input",
                value: "{url_input}",
                oninput: move |evt| url_input.set(evt.value.clone()),
                style: "flex: 1; height: 34px; padding: 0 12px; border: 1px solid #d0d7de; border-radius: 6px;",
                placeholder: "Enter URL..."
            }

            button {
                id: "go-button",
                onclick: move |_| on_navigate.call(url_input.get().clone()),
                style: "height: 34px; padding: 0 16px; background: #2da44e; color: white; border-radius: 6px;",
                "Go"
            }
        }
    })
}
```

### 5. Content Management (src/content.rs - NEW)

```rust
use blitz_html::HtmlDocument;
use blitz_dom::DocumentConfig;

pub struct ContentManager {
    doc: HtmlDocument,
}

impl ContentManager {
    pub fn new(initial_url: &str, initial_html: &str) -> Self {
        let doc = HtmlDocument::from_html(
            initial_html,
            DocumentConfig {
                base_url: Some(initial_url.to_string()),
                ..Default::default()
            },
        );
        Self { doc }
    }

    pub fn load_html(&mut self, url: &str, html: &str) {
        self.doc = HtmlDocument::from_html(
            html,
            DocumentConfig {
                base_url: Some(url.to_string()),
                ..Default::default()
            },
        );
    }

    pub fn get_document(&self) -> &HtmlDocument {
        &self.doc
    }

    pub fn get_document_mut(&mut self) -> &mut HtmlDocument {
        &mut self.doc
    }
}
```

### 6. Bridge/Communication (src/bridge.rs - NEW)

```rust
use std::sync::mpsc::{channel, Sender, Receiver};

pub enum ChromeEvent {
    Navigate(String),
    Reload,
    Back,
}

pub enum ContentEvent {
    LoadComplete(String),
    LoadFailed(String),
    TitleChanged(String),
}

pub struct EventBridge {
    chrome_tx: Sender<ChromeEvent>,
    chrome_rx: Receiver<ChromeEvent>,
    content_tx: Sender<ContentEvent>,
    content_rx: Receiver<ContentEvent>,
}

impl EventBridge {
    pub fn new() -> Self {
        let (chrome_tx, chrome_rx) = channel();
        let (content_tx, content_rx) = channel();
        Self { chrome_tx, chrome_rx, content_tx, content_rx }
    }

    pub fn send_chrome_event(&self, event: ChromeEvent) {
        self.chrome_tx.send(event).unwrap();
    }

    pub fn poll_chrome_events(&self) -> Vec<ChromeEvent> {
        self.chrome_rx.try_iter().collect()
    }

    // Similar for content events...
}
```

### 7. Main Application (src/main.rs changes)

```rust
// Replace current approach with:

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    let event_loop = create_default_event_loop();
    let proxy = event_loop.create_proxy();

    // Fetch initial content
    let (base_url, contents, _) = rt.block_on(fetch(&raw_url, net_provider.clone()));

    // Create Dioxus chrome
    let mut chrome_vdom = VirtualDom::new(Chrome);

    // Create Blitz content manager
    let mut content_manager = ContentManager::new(&base_url, &contents);

    // Create event bridge
    let bridge = EventBridge::new();

    // Create dual-renderer application
    let mut application = DualRendererApplication::new(
        chrome_vdom,
        content_manager,
        bridge,
        net_provider,
    );

    event_loop.run_app(&mut application).unwrap()
}
```

### 8. Updated Application Handler (src/readme_application.rs changes)

```rust
pub struct DualRendererApplication {
    chrome_vdom: VirtualDom,
    content_manager: ContentManager,
    bridge: EventBridge,
    net_provider: Arc<Provider<Resource>>,
    // ... other fields
}

impl ApplicationHandler<BlitzShellEvent> for DualRendererApplication {
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // Route events to appropriate renderer

        // Events in top 50px go to Dioxus
        if let WindowEvent::CursorMoved { position, .. } = &event {
            if position.y < 50.0 {
                // Send to Dioxus chrome
                self.chrome_vdom.handle_event(event);
            } else {
                // Send to Blitz content
                self.content_manager.get_document_mut().handle_event(event);
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: BlitzShellEvent) {
        // Poll bridge for chrome events (navigate, etc.)
        for chrome_event in self.bridge.poll_chrome_events() {
            match chrome_event {
                ChromeEvent::Navigate(url) => {
                    // Fetch new content and update Blitz document
                    self.fetch_and_load(&url);
                }
                // ... handle other events
            }
        }
    }
}
```

## Key Questions to Resolve

### Q1: How does Dioxus render to a winit window?

Need to investigate:
- Does Dioxus have a winit renderer? (I know it has web, desktop via webview, TUI)
- May need `dioxus-desktop` or custom renderer
- Alternative: Use `dioxus-tui` for chrome? (text-based UI in terminal style)

**Most likely answer**: Use **dioxus-native** or write a custom Dioxus renderer that outputs to Vello (same as Blitz uses)

### Q2: How to composite two renderers?

Options:
1. **Scissor/viewport approach**: Render Dioxus to top 50px, Blitz to rest
2. **Texture approach**: Dioxus renders to texture, Blitz renders to texture, composite both
3. **Layered approach**: Render Blitz full-screen, then Dioxus chrome on top

**Recommendation**: Viewport approach - set viewport/scissor rect for each render pass

### Q3: Event routing?

Need to route mouse/keyboard events based on position:
- Y < 50px → Dioxus chrome
- Y >= 50px → Blitz content

Can use `WindowEvent::CursorMoved` to track cursor position.

## Dependencies to Add

```toml
[dependencies]
dioxus = "0.6"  # Or latest version
dioxus-core = "0.6"
# May need dioxus renderer crate depending on approach
```

## Implementation Steps

1. **Research Phase**: Determine how to render Dioxus to winit window alongside Blitz
2. **Prototype**: Get basic Dioxus chrome rendering in top 50px
3. **Event Routing**: Split events between chrome (Dioxus) and content (Blitz)
4. **Bridge**: Implement communication (navigate events from Dioxus → fetch → update Blitz)
5. **Migration**: Move all chrome logic from current chrome.rs to Dioxus component
6. **Testing**: Update tests to work with dual-renderer architecture

## Potential Blockers

1. **Dioxus winit integration** - May not exist or be well-documented
2. **Rendering complexity** - Compositing two renderers could be tricky
3. **Event handling** - Routing events correctly between renderers
4. **Performance** - Two render passes per frame might impact performance

## Alternative: Keep It Simpler?

If Dioxus integration proves too complex, consider:
- **Native UI chrome** (Cocoa on Mac, Win32 on Windows) + Blitz content
- **egui** (immediate mode GUI) for chrome + Blitz content (easier winit integration)
- Accept current architecture and rely on style isolation best practices

Would you like me to research the Dioxus winit rendering approach first, or should I spec out an egui-based alternative?
