# React Demo WebDriver Plan

## Goals

- Ship a reusable WebDriver harness that can drive Frontier headlessly for small demo apps (counter, timer, future CRUD examples).
- Cover deterministic end-to-end flows in `cargo test` / `just ci` so regressions are caught without manual GUI interaction.
- Keep demo assets simple (plain HTML + vendored React bundles) and runnable without extra JS tooling.

## Core Requirements

1. **Headless WebDriver service**
   - `start_webdriver` must spin up entirely in tests (no winit window, no GUI) and expose the endpoints the harness relies on (`/session`, `/element/...`, `/frontier/pump`, `/status`).
   - Provide a graceful shutdown handle so tests do not leak tasks.

2. **Selector + interaction support**
   - Support reliable element lookup beyond `#id` selectors, or document the limit and fail fast when unsupported selectors are used.
   - Translate WebDriver clicks into real DOM events via the runtime so React state updates are exercised.
   - Add helpers for value reads (`/text`, future `/property`) and timer pumping so async UIs can be verified deterministically.

3. **Asset workflow**
   - Keep `assets/react-demos/` self-contained by vendoring the React/ReactDOM UMD bundles (no `bun install` step).
   - Document how to add a new demo (HTML stub + test) and ensure each demo ships with an automated WebDriver test case.

4. **CI integration**
   - Ensure at least one WebDriver test runs by default (no `#[ignore]`).
   - Make the tests tolerant to slower CI boxes (use timeouts + polling instead of fixed sleeps where possible).

## Near-Term Tasks

- Extend `HeadlessSession` networking so WebDriver sessions can navigate to `http(s)` URLs when needed, or explicitly reject them with a clear error.
- Flesh out the HTTP API with `/status` and capability negotiation so third-party clients (Selenium/Playwright) can be layered on later.
- Add coverage for the timer demo (`webdriver_timer_start_stop`) once the timer can be driven without real-time sleeps (use pump endpoint).
- Write docs in `notes/` for running the harness and debugging failures (log locations, enabling tracing).
- Expand abort-signal regression coverage to real DOM elements so React demos exercise listener cleanup paths (build on new EventTarget fixes before shipping more interactive flows).
- Implement HTML timer clamping (WPT `html/webappapis/timers/type-long-settimeout.any.js`) to stabilise timer-driven demos and keep the pump loop deterministic.

## Stretch Work

- Broaden interaction surface (keyboard input, form submission, pointer move/drag).
- Share the harness with other demo suites (nostr integrations, SQLite sample) once the basic flows are stable.
- Mirror a subset of WebDriver conformance tests to guard protocol compatibility as the API grows.
