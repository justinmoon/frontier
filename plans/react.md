# React DOM Integration Plan

## Objective
Build a production-quality React runtime inside Frontier so real-world React 18 apps render, update, and handle input without reparsing HTML. The browser should host a long-lived QuickJS runtime per document, mirror React-driven DOM mutations into Blitz via `DocumentMutator`, surface the DOM APIs React depends on, and drive events/timers using the same pathways the Blitz UI already uses.

## Current Reality
- `FetchedDocument` now carries a `Vec<ScriptDescriptor>` populated by `collect_document_scripts` in `src/navigation.rs:584`, and `ReadmeApplication::set_document` spins up a `JsPageRuntime` before first paint. Blocking inline scripts still run ahead of rendering, but DOM changes are applied by reserializing HTML rather than mutating Blitz’s live tree.
- `JsDomEnvironment` in `src/js/environment.rs:10` wraps a Kuchiki `DomSnapshot` (`src/js/dom.rs:9`), so QuickJS mutations diverge from the Blitz `BaseDocument`. Only `textContent`, `innerHTML`, and `setAttribute` are supported.
- `ReadmeApplication::render_current_document` in `src/readme_application.rs:144` rebuilds an `HtmlDocument` from a string every time the UI refreshes, so there is still no single source of truth shared with Blitz after initial serialization.
- Events, timers, network fetches, script tags with `src`, and module/defer semantics are all unimplemented. React cannot bootstrap in this environment.

## Milestone 0 — Branch Baseline & References
1. Rebase `plans-doc-component-codex` (and the historical `react-dom-rendering-codex` branch) onto the current `master` after confirming QuickJS landed. Resolve Cargo git dependencies so `blitz`, `dioxus`, and `blitz-shell` revisions match `~/code/blitz` `main`.
2. Run `just ci` to capture the baseline and record any failing suites in `notes/quickjs_followups.md` so we can track regressions introduced during rebase.
3. Skim the reference implementations: `~/code/dioxus/packages/native-dom/src/mutation_writer.rs` for mutation application patterns, `~/code/blitz/packages/blitz-dom/src/mutator.rs` for available operations, and `~/code/react/packages/react-dom` for the host config React expects.

## Milestone 1 — Persistent Runtime Per Document
1. Introduce `JsPageRuntime` in `src/js/session.rs` that owns a `QuickJsEngine`, task handles, and a queue of DOM operations. It should expose lifecycle hooks: `init(document_id, scripts)`, `suspend()`, and `teardown()`.
2. Extend `FetchedDocument` in `src/navigation.rs:78` to carry a `Vec<ScriptDescriptor>` (inline source, external URL, execution mode). Replace `process_document_inline_scripts` with a `collect_document_scripts` helper that records the manifest without mutating HTML.
3. Change `ReadmeApplication` to hold `current_js_runtime: Option<JsPageRuntime>` alongside `current_document`. When a navigation completes, build the `HtmlDocument`, attach the runtime, and execute scripts against the live document before first paint.
4. Use the existing `tokio::runtime::Handle` stored on `ReadmeApplication` (`src/readme_application.rs:54`) to spawn timer futures and a periodic microtask pump that posts back into the winit loop (e.g., via `BlitzShellEvent::Embedder`).
5. On navigation away or window close, call `JsPageRuntime::teardown()` so QuickJS contexts are dropped and outstanding timers are cancelled.

## Milestone 2 — Blitz DOM Bridge (Single Source of Truth)
1. Replace the Kuchiki snapshot layer. Refactor `JsDomEnvironment` so it receives a mutable borrow of the active `BaseDocument` (obtained via `HtmlDocument::deref_mut()`) and hands it to a new `BlitzJsBridge`.
2. `BlitzJsBridge` maintains bidirectional maps between JS node handles (`String` IDs derived from `QuickJsAtom`s) and Blitz node IDs (`usize`). During initialization, walk the document once using `BaseDocument::nodes` to seed the mapping.
3. Translate JS DOM operations into `DocumentMutator` calls: `createElement`, `createTextNode`, `appendChild`, `insertBefore`, `removeChild`, `cloneNode`, `setAttribute`, `removeAttribute`, dataset/class/style helpers, and text updates. Flush mutations in batches using `BaseDocument::mutate()` just before Blitz paints (hook into `ReadmeApplication::window_event` before delegating to `self.inner`).
4. For DOM reads (`childNodes`, traversal APIs, `getAttribute`, layout queries), read directly from the live `BaseDocument` so React’s reconciliation sees the true tree.
5. Drop `DomSnapshot`/`DomPatch` from `src/js/dom.rs` once the bridge is authoritative; update the QuickJS bootstrapping code to call into the bridge instead of emitting JSON patches.

## Milestone 3 — Web Platform Surface for React
1. Expose the minimal globals React requires: `window`, `document`, `Node`, `Element`, `Text`, `Comment`, `Document`, `EventTarget`, and collections (`NodeList`, `DOMTokenList`). Use QuickJS class bindings to wire prototype chains.
2. Implement DOM construction APIs (`document.createElement`, `document.createElementNS`, `createTextNode`, `createComment`, `createDocumentFragment`) so they allocate through `BlitzJsBridge` and return proxy objects that forward operations.
3. Add property shorthands React relies on: `node.textContent`, `node.innerHTML`, `element.className`, `element.style`, `value/checked/defaultValue`, dataset proxies, `dangerouslySetInnerHTML`, and boolean attribute semantics.
4. Provide timing primitives backed by tokio: `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`, `queueMicrotask`, and `requestAnimationFrame`. Ensure the microtask queue is drained before each batch of DOM mutations is flushed.
5. Ship a no-op but standards-compliant `MutationObserver` implementation that queues mutation records from the bridge so React’s hydration checks pass, even if we only support attribute/text records initially.

## Milestone 4 — Script Loading Pipeline
1. During HTML parse (still in `src/js/processor.rs`), collect `<script>` nodes with `src`, `type`, `async`, `defer`, and inline source. Resolve `src` attributes against the document base URL using `::url::Url`.
2. Reuse the existing `Provider<Resource>` (`src/main.rs:83`) to fetch external scripts for HTTP/HTTPS and Blossom contexts. Store fetched code in the `ScriptDescriptor` manifest along with integrity metadata (hash, origin).
3. Execute classic scripts sequentially in insertion order inside `JsPageRuntime::init`, matching browser semantics. Structure the pipeline so `async` and `defer` simply enqueue work in separate queues that we can honor in a follow-up.
4. Cache fetched script bodies per origin/root hash (e.g., with `storage::Storage`) so back/forward or repeated navigations do not hit the network unnecessarily.
5. Surface network or evaluation errors to the user via the existing tracing logs and an overlay in the page chrome to aid debugging.

## Milestone 5 — Event Bridge
1. Teach the QuickJS bindings to record handlers via `addEventListener`/`removeEventListener`, tracking capture flags & passive options. Persist listeners inside `BlitzJsBridge` alongside node IDs.
2. Subscribe to Blitz events by integrating with `EventDriver` exposed by `blitz_shell` (see `~/code/blitz/packages/blitz-shell`). Translate incoming `UiEvent`s into QuickJS `Event` subclasses (`MouseEvent`, `KeyboardEvent`, `InputEvent`) and dispatch through `EventTarget`.
3. Implement `stopPropagation`, `stopImmediatePropagation`, and `preventDefault` so they interact with Blitz’s event phases and can cancel default actions.
4. Allow JS to call `dispatchEvent` on any node proxy. The bridge should fan this into Blitz’s driver so React synthetic events (which re-dispatch userland events) still reach the DOM.

## Milestone 6 — Runtime Resource Fetching
1. Provide a minimal `fetch` implementation in JS backed by the existing `Provider<Resource>` so React apps that lazy-load bundles or data can function. Keep it to `text`, `json`, and `arrayBuffer` for the first cut.
2. Add support for `Image` construction, `<img>` `onload`/`onerror`, and CSS injection (`<link rel="stylesheet">`) by reusing Blitz’s net handlers (`blitz_dom::net::ImageHandler`, `CssHandler`).
3. Handle blob/data URLs so bundlers that inline assets still render. Implement `URL.createObjectURL` for the simple blob-to-string case.

## Milestone 7 — Testing & CI
1. Add React demo assets under `assets/react-counter/` using the React 18 UMD builds from `~/code/react/packages/react-dom/umd`. Include a counter and todo list to exercise updates and list reconciliation.
2. Write an integration test (`tests/react_counter_test.rs`) that loads the counter HTML through the navigation pipeline, allows timers to tick, and asserts on the Blitz DOM via `DocumentMutator` queries.
3. Create a Blitz accessibility/UI test (similar to existing layout tests) that clicks the counter button and verifies the count updates, ensuring event wiring works end-to-end.
4. Extend `just ci` (nix target) to run the new tests. Iterate until the suite is deterministic; avoid mocks per the Agents Guide.
5. Document known flaky surfaces in `notes/TEST_FINDINGS.md` if any issues remain, but keep `just ci` green before merging.

## Milestone 8 — Documentation & Observability
1. Write `docs/REACT.md` explaining the supported DOM APIs, timer behavior, and limitations (e.g., no `fetch` streaming yet, classic scripts only). Cross-link from `README.md` and `docs/NNS.md` where we mention web platform support.
2. Capture follow-up work (module scripts, hydration optimizations, richer network stack) in `notes/quickjs_followups.md` so future agents have a backlog.
3. Improve logging: tag tracing spans with the document URL, script filename, and elapsed execution time so profiling React-heavy sites is easier.

## Definition of Done
- React 18 UMD counter app renders inside Frontier, responds to clicks, and updates the Blitz DOM without HTML reserialization.
- QuickJS runtimes live for the duration of a document, reuse timers/event loops across paints, and clean up on navigation.
- DOM mutations flow through `DocumentMutator`, keeping Blitz as the single source of truth.
- `just ci` passes, covering the new integration/unit/UI tests.
- Documentation and notes describe the new surface area and open risks.

## Risks & Mitigations
- **Runtime leaks:** Ensure every `JsPageRuntime` registers with `tokio::task::AbortHandle` so navigation teardown cancels timers. Add debug assertions in `drop` implementations.
- **Event ordering mismatches:** Mirror Chrome’s capture/bubble order using unit tests that compare sequences against expectations recorded from `~/code/blitz` demos.
- **Performance regressions:** Benchmark the React demo before/after bridging with tracing timers; if batches get large, introduce incremental flushes without compromising correctness.
- **Spec drift:** Track upstream React host config updates; add a quick script that imports `react-dom/server.browser.development.js` to verify required methods remain implemented.
