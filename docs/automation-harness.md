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
- Responses are delivered over a oneshot channel after the app has handled the event, and every
  command now snapshots the DOM + metadata under `target/automation-artifacts/<session>/<step>` so
  CI failures have breadcrumbs.

Automation client
------------------
Tests should rely on the Rust helper in `src/automation_client/` instead of crafting raw HTTP.
`AutomationHost::spawn` launches the binary, negotiates the bind address, and exposes an
`AutomationSession` with ergonomics for clicks, rich pointer sequences, keyboard actions,
`wait_for_text`, and `wait_for_element`. The helper also exposes the artifact directory so tests
can attach additional diagnostics when needed.

Endpoints (stable for now)
--------------------------
- Selectors are structured records: `{"selector": {"kind": "css", "selector": "#status"}}` or
  `{"selector": {"kind": "role", "role": "button", "name": "Submit"}}`. Query parameters use
  `?kind=css&selector=#status` or `?kind=role&role=button&name=Submit`.
- `POST /session` creates the single active session (optionally navigating immediately).
- `POST /session/frontier/click` dispatches a real pointer click.
- `POST /session/frontier/pointer` executes WebDriver-style pointer sequences (move/down/up/scroll).
- `POST /session/frontier/type` focuses the element and commits text through IME events.
- `POST /session/frontier/keyboard` synthesises keyboard text and shortcut actions.
- `POST /session/frontier/focus` / `scroll` ensure targets are ready before interacting.
- `POST /session/frontier/pump` still exists as the low-level escape hatch while higher-level
  waits are built out.
- `GET  /session/frontier/text?...` and `GET /session/frontier/exists?...` expose rendered text and
  role/name presence for assertions.

Example (Rust integration test)
-------------------------------
```rust
use std::path::PathBuf;

use frontier::automation_client::{
    AutomationHost, AutomationHostConfig, ElementSelector, WaitOptions,
};

let asset_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
let host = AutomationHost::spawn(
    AutomationHostConfig::default().with_asset_root(asset_root)
)?;

let session = host.session_from_asset("index.html")?;
let timer_url = "file:///…/timer.html";

session.type_text(&ElementSelector::css("#url-input"), timer_url)?;
session.click(&ElementSelector::css("#go-button"))?;
session.navigate_url(timer_url)?;

let heading = session.wait_for_text(
    &ElementSelector::css("#timer-heading"),
    WaitOptions::default_text_wait(),
)?;
assert!(heading.contains("Timer"));
```

Why a separate process?
-----------------------
macOS requires the winit event loop to be created on the process’ main thread. Cargo tests run on
worker threads, so embedding the app in-process deadlocks. The host sidesteps this by launching a
real browser process and talking to it over HTTP, which also matches the project goal of “tests
speak WebDriver APIs and never reach into QuickJS.”

Artifacts
---------
`target/automation-artifacts/<session>/<step_label>` contains:
- `command.txt` – debug dump of the command that ran.
- `reply.json` – serialised `AutomationResponse` plus any artifacts collected for that command.
- `dom.html` – DOM snapshot (when QuickJS can serialise it).
- `error.txt` – present whenever the command returned `Err`, mirroring the failure surfaced to the
  client helper.

Next steps
----------
- Continue fleshing out WebDriver compatibility (screenshots, network captures, richer waits).
- Capture console output and QuickJS exception summaries alongside DOM snapshots.
- Drive more suites through `automation_client` so raw HTTP usage can eventually be removed.
