# Ants Demo Sprint Plan

Target: Deliver the Ants Nostr search client (`~/code/ants`) running inside Frontier with live relay results. This sprint stitches together the JavaScript runtime work with the networking feature set from the prior Nostr plan.

## Track A – QuickJS Runtime Readiness
1. **External classic scripts** – Implement real `<script src>` execution so Ants’ Next/React bundle boots without manual loaders (`src/js/processor.rs`, `src/main.rs`).
2. **React sentinel fidelity** – Fix `BlitzJsBridge::create_comment_node` serialization and add regression coverage (`tests/quickjs_dom_test.rs`).
3. **Event handle normalization** – Align JS-visible node handles with Blitz expectations before listener dispatch to avoid React event glitches (`src/js/environment.rs`).
4. **Input surface tests** – Extend integration tests to cover keyboard + pointer events driven via `RuntimeDocument`, ensuring Ants’ search input works as expected.

Deliverable: React demo (`just run -- --react-demo`) works end-to-end using the same script pipeline Ants will rely on.

## Track B – Web Platform Bridges for Ants
1. **Fetch/HTTP bridge** – Expose `window.fetch`, `Request`, and `Response` using `blitz_net::Provider`. Support JSON/text bodies, relative URLs, and error propagation; add integration tests.
2. **WebSocket bridge** – Land the Rust `WebSocketManager`, QuickJS bindings, and DOM-facing `WebSocket` API. Reuse `tls::connect_websocket`, handle text/binary frames, and cover with CI using the existing mock relay.
3. **Minimal storage shim** – Provide an in-memory `localStorage` so Ants’ cache helpers and NIP-07 gating logic don’t crash (log persistence TODO).
4. **Utility shims** – Confirm existing polyfills (`queueMicrotask`, timers) cover Ants; add UTF-8 `TextEncoder/TextDecoder` now to unblock future clients.

Deliverable: Simple QuickJS harness exercising fetch + WebSocket + localStorage passes in CI.

## Track C – Ants Integration
1. **Launcher** – Add `just demo-ants` (or similar) to build/serve the Ants bundle and open it in Frontier.
2. **Smoke test** – Navigate to Ants, run a NIP-50 search, validate relay status updates, NIP-05 lookups, and ensure no runtime exceptions. Document necessary environment variables or manual steps.
3. **Regression coverage** – Add an automated test (likely end-to-end using the existing harness) that loads a trimmed Ants build or test page exercising fetch + WebSocket + React rendering.
4. **Notes & follow-ups** – Record any missing APIs (MutationObserver, WASM cache fallback, etc.) in `notes/` for future sprints.

## Out of Scope After Merge
- Signing flows (NIP-07), persistent storage, or key management UI.
- Full WASM cache support for NDK (accept graceful fallback to live relays).
- Broader Nostr client compatibility until Ants is stable.

## Success Criteria
1. Ants runs inside Frontier, connects to live relays, and displays search results without console errors.
2. `just ci` covers new fetch/WebSocket bridges and critical QuickJS fixes.
3. Documentation + notes clearly outline remaining gaps for nostrudel/Damus.

Once Track C is green, revisit the backlog below for deeper WPT work and expanded Nostr support.

---

# JavaScript Runtime Backlog

## Mission
Deliver a production-ready QuickJS-powered DOM runtime that can execute real-world React 18 apps, interoperate seamlessly with Blitz, and form the base for a sustainable Web Platform Test (WPT) pipeline.

## Where We Stand (master)
- QuickJS engine + DOM bridge live per navigation and stay attached to a persistent `HtmlDocument`.
- `RuntimeDocument` now pumps timers during `poll`, registers wakers, and keeps Blitz aligned with JS-driven redraws.
- Timer queue floors zero-delay intervals (`setInterval(..., 0)`) and is covered by `intervals_floor_zero_delay` in `tests/quickjs_dom_test.rs`.
- React counter demo lives under `assets/react-counter/` with a smoke test that exercises the runtime end-to-end (manual external script loader for now).
- UI events flow through `JsEventHandler` → `DispatchOutcome`, respecting preventDefault/stopPropagation from JS listeners.

## Next Critical Work (pre-WPT)
1. **External classic scripts** – Teach `JsPageRuntime::run_blocking_scripts` to fetch/execute `<script src="...">` in document order so we can drop the custom loader in `src/main.rs`. Validate by running the React counter without manual loading.
2. **Comment fidelity** – Ensure comment nodes serialize as `<!-- text -->`; cover with `tests/quickjs_dom_test.rs`.
3. **Handle normalization** – Align JS-visible handles with Blitz’ internal mapping before listener dispatch (`src/js/dom.rs`, `src/js/environment.rs`).
4. **Input surface coverage** – Drive keyboard/input/IME events through `RuntimeDocument` in tests; no direct JS eval shortcuts.
5. **React demo parity** – Wire `cargo run -- --react-demo` to reuse the new external-script pipeline and validate timer/listener cleanup across rerenders.
6. **Source layout health** – Audit oversized modules (e.g., `src/js/environment.rs` now >2k LOC). Plan targeted splits (timers, DOM bindings, websocket bridge) once Track A lands to keep diffs manageable for agents.

## Supplemental Improvements
- Broaden DOM API coverage: `createElementNS`, attribute removal, cloning/deep tree mutations used by React reconciler.
- Measure timer + microtask behavior under load; consider a shared microtask queue instead of ad-hoc `Promise.resolve().then` once more async APIs arrive.
- Track open issues in `notes/` (requestAnimationFrame stub, MutationObserver limitations) and retire legacy helpers when Rust alternatives ship.

## WPT Timeline
Begin importing a curated WPT subset **after** items 1–4 above are complete and the React demo no longer relies on manual script loaders.
1. Pull targeted suites from `~/code/wpt` (start with `domparsing/` and `html/webappapis/timers/`).
2. Reuse the lightweight harness in `~/code/blitz` (`wpt_harness.rs`) to execute tests inside QuickJS.
3. Gate `just ci` on ~10–15 curated tests; expand only when new APIs land and document unsupported features inline.

## Research Pointers (~/code/)
- `blitz` – EventDriver usage and prior WPT harness spikes.
- `dioxus` – Handle normalization + virtual DOM mutation patterns.
- `nsite` – External resource streaming/cache strategies for script loading.

## Working Agreement
- NEVER use mocks in integration tests; exercise the same paths the GUI hits.
- Keep the directory structure relatively flat until the APIs settle.
- Log tech-debt finds in `notes/` while they’re fresh.
- Ensure `just ci` passes before claiming a milestone is done.

Next agent: follow Track A/B/C to land the Ants demo, then resume backlog items with an eye toward splitting oversized modules like `src/js/environment.rs` to keep future edits tractable.
