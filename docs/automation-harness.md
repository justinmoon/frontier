Frontier Automation Host
========================

Frontier’s UI automation now runs the full browser (ReadmeApplication + chrome) inside a
standalone host process. The host owns the winit event loop on its main thread and exposes a
minimal WebDriver-flavoured HTTP surface so tests can drive the app exactly like a user would.

How it works
------------
- `automation_host` binary bootstraps Frontier, loads an initial document, and starts an Axum
  server on `AUTOMATION_BIND` (use `AUTOMATION_BIND=127.0.0.1:0` for an ephemeral port; it prints
  `AUTOMATION_HOST_READY host:port` on stdout once listening).
- Each HTTP request is translated into an `AutomationCommand`. The host enqueues the command and
  pokes the winit event loop with `AutomationEvent`, so ReadmeApplication processes the action via
  the same pointer/keyboard paths users do.
- Responses are delivered over a oneshot channel after the app has handled the event, letting
  callers observe DOM state or await timers without peeking into QuickJS internals.

Endpoints (stable for now)
--------------------------
- `POST /session` → `{ "file"?: "index.html", "url"?: "https://..." }` creates the single active
  session and optionally navigates immediately.
- `POST /session/frontier/click` → `{ "selector": "#go-button" }` dispatches a real pointer click.
- `POST /session/frontier/type` → `{ "selector": "#url-input", "text": "https://example" }`
  focuses the element and commits text through IME events.
- `POST /session/frontier/pump` → `{ "milliseconds": 500 }` pumps timers/animation for the given
  duration (temporary escape hatch while we build higher-level waits).
- `GET  /session/frontier/text?selector=#content` returns `{ "value": "…" }` so tests can assert on
  rendered text.

Example (Rust integration test)
-------------------------------
```rust
let (mut host, addr) = launch_host(&asset_root)?; // spawn automation_host, read READY banner
let client = reqwest::blocking::Client::new();

create_session(&client, &addr, "index.html")?;
type_text(&client, &addr, "#url-input", "file:///…/timer.html")?;
click(&client, &addr, "#go-button")?;
pump(&client, &addr, Duration::from_millis(500))?;
let heading = get_text(&client, &addr, "#timer-heading")?;
assert!(heading.contains("Timer"));
```

Why a separate process?
-----------------------
macOS requires the winit event loop to be created on the process’ main thread. Cargo tests run on
worker threads, so embedding the app in-process deadlocks. The host sidesteps this by launching a
real browser process and talking to it over HTTP, which also matches the project goal of “tests
speak WebDriver APIs and never reach into QuickJS.”

Next steps
----------
- Flesh out the HTTP surface to align with the official WebDriver spec and add richer assertions
  (DOM queries by role/name, screenshots, network captures).
- Capture run artifacts (DOM snapshots, console output, relay traffic) so CI failures are easy to
  debug without reruns.
- Package a small client helper crate to hide the raw HTTP calls inside tests.
