# React DOM Integration Plan

## 0. Baseline
- Rebase `react-dom-rendering-codex` onto current `master` now that the QuickJS runtime is merged, resolve git deps to the latest Blitz/Dioxus revisions, and note any conflicts in `notes/`.

## 1. Persistent JS Runtime per Document
- Replace the one-shot inline script executor with a long-lived `JsDomEnvironment` tied to each active `HtmlDocument` so scripts and timers survive beyond initial load.
- Pump QuickJS microtasks and timer callbacks on the main tokio loop; expose hooks for navigation lifecycle (init, suspend, teardown) to cleanly destroy runtimes on page exit.

## 2. Host DOM Bridge (Single Source of Truth)
- Follow the `dioxus_native_dom::MutationWriter` pattern: treat Blitz’s `BaseDocument` as the authoritative DOM, and translate JS operations directly into `DocumentMutator` calls (create/insert/remove/reorder/clone nodes, text updates, attributes, styles, classList, dataset, namespaces).
- Build a `BlitzJsBridge` that maintains the JS ↔ Blitz node mapping (string IDs for JS proxies, `usize` node IDs for Blitz), indexes the initial document, queues DOM operations emitted from JS, and applies them in batches via `DocumentMutator` between render ticks.
- For DOM reads (e.g., `childNodes`, traversal, `getAttribute`), consult the live `BaseDocument` through the bridge—drop the Kuchiki snapshot entirely so the tree can’t diverge.

## 3. Web Platform Surface Required by React
- Provide the minimal `window`/`document` surface React expects: element factories (`createElement`, `createTextNode`, fragments, namespaces, head/body accessors), globals (`location`, `history`, `navigator.userAgent`, `performance.now`, `MutationObserver` stub), and correct prototypes (`Node`, `Element`, `Text`, `Comment`, `EventTarget`, `Document`).
- Implement timers (`setTimeout`, `clearTimeout`, `setInterval`, `requestAnimationFrame`, `queueMicrotask`) backed by tokio tasks, ensuring the QuickJS job queue is drained ahead of each paint.
- Handle attribute/property shorthands (value/checked/defaultValue, innerHTML/textContent, style/classList/dataset setters) inside the bridge so React’s host config behaves as on browsers.

## 4. Script Loading Pipeline
- Scan `<script>` elements after HTML parse, resolve URLs against the document’s base (legacy HTTP and Blossom contexts), and fetch sources through `Provider`/`BlossomFetcher`.
- Execute classic scripts sequentially in load order (including inline) within the persistent runtime; structure the loader so defer/async/module support can be added later without rework.
- Cache fetched bundles per origin/root hash to prevent redundant network hits across navigations/back-forward.

## 5. Event Bridge
- Wire `addEventListener`/`removeEventListener` from JS into Blitz’s `EventDriver`, tracking capture/bubble phases and listener options inside the bridge.
- Translate Blitz `UiEvent`s into QuickJS `Event`/`MouseEvent`/`KeyboardEvent` objects (with timestamps, target/currentTarget, bubbling), honouring `stopPropagation`/`stopImmediatePropagation`/`preventDefault`.
- Support `dispatchEvent` from JS so synthetic events flow back through Blitz’s driver, enabling React’s synthetic event system.

## 6. Runtime Asset Fetches
- Allow JS bundles to request additional resources (lazy chunks, images, CSS) via the existing providers; ensure URL resolution works for both HTTP and Blossom-served assets, and pass through response bodies to JS.
- Provide minimal blob/data URL handling so builds that inline assets still function.

## 7. Testing & CI
- Add Rust integration tests that load sample React bundles (e.g., UMD counter/todo apps under `assets/`), run them through the loader, and assert resulting Blitz DOM structure via `DocumentMutator` queries.
- Add a Blitz accessibility/UI test that interacts with the sample React app (click button, see count update) to prove event wiring and state updates end-to-end.
- Ensure `just ci` stays green—iterate locally until the new tests are reliable.

## 8. Documentation, Risks & Notes
- Document the JS host APIs, supported features, and known limitations (no fetch/WebRTC yet, classic scripts only) in `docs/`.
- Track open risks (QuickJS performance, event memory management, timer accuracy) and follow-up work (module scripts, hydration, broader Web APIs, profiling) in `notes/`.
- Success criteria: run the React 18 UMD counter app, handle user clicks updating state, keep the DOM in sync without reparsing, and pass all CI checks.
