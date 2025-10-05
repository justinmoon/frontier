# Chrome Architecture

Frontier uses a dual-document architecture to render browser UI (Dioxus) and web content (HTML) in a single window.

## How It Works

### Two Documents, One Window

```
┌─────────────────────────────────────┐
│ Chrome (Dioxus - Full Screen)       │
│ ┌─────────────────────────────────┐ │
│ │ [Address Bar] [Go] [Alert]      │ │ ← Opaque (pointer-events: auto)
│ └─────────────────────────────────┘ │
│                                     │
│     (Transparent overlay area)      │ ← Transparent (pointer-events: none)
│                                     │
│  ┌───────────────────────────────┐ │
│  │ Alert Dialog (when shown)     │ │ ← Opaque modal (pointer-events: auto)
│  └───────────────────────────────┘ │
└─────────────────────────────────────┘

┌─────────────────────────────────────┐
│ Content (HTML - Offset by 50px)     │
│                                     │
│  Rendered HTML from web pages       │
│                                     │
└─────────────────────────────────────┘
```

**Chrome document (Dioxus):**

- Full window height
- Transparent below address bar
- Captures events only on opaque elements (address bar, dialogs)

**Content document (HTML):**

- Positioned 50px down from top
- Height = window height - 50px
- Only receives events when chrome has no active modal

### Separate Vello Scenes

We render each document to its own `vello::Scene`, then composite them:

```rust
// src/dual_view.rs render()
let mut content_scene = vello::Scene::new();
let mut chrome_scene = vello::Scene::new();

// Paint content at offset
paint_scene(&mut content_painter, content_doc, ...);
paint_scene(&mut chrome_painter, chrome_doc, ...);

// Composite with transforms
scene_painter.inner.append(&content_scene, Some(Affine::translate((0, chrome_offset))));
scene_painter.inner.append(&chrome_scene, Some(Affine::IDENTITY));
```

**Why separate scenes?**

- Vello's `push_layer` has clipping issues with transforms
- Each scene has independent coordinate space
- Clean separation of concerns

## Event Routing

Mouse/keyboard events route based on state:

```rust
let route_to_chrome = mouse_y < CHROME_HEIGHT || has_chrome_overlay();

if route_to_chrome {
    chrome_doc.handle_ui_event(event);
} else {
    content_doc.handle_ui_event(event);  // Adjust Y coordinate
}
```

When alert is showing, all events go to chrome to prevent click-through.
