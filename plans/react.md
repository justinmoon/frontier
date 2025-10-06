# React DOM Integration Plan

## Objective

Build a production-quality React runtime inside Frontier so real-world React 18 apps render, update, and handle input without reparsing HTML. The browser should host a long-lived QuickJS runtime per document, mirror React-driven DOM mutations into Blitz via `DocumentMutator`, surface the DOM APIs React depends on, and drive events/timers using the same pathways the Blitz UI already uses.

## Progress Snapshot

- `FetchedDocument` collects a full script manifest and `ReadmeApplication` instantiates `JsPageRuntime` so blocking inline scripts execute before first paint.
- `JsDomEnvironment` now applies text/innerHTML/attribute writes directly to the live Blitz `BaseDocument` through `BlitzJsBridge`, eliminating Kuchiki snapshots for those paths.
- Inline-script execution has parity in processor and runtime flows, with integration tests (`tests/quickjs_dom_test.rs`) covering QuickJS execution against real DOMs.
- **Milestones 1-5 COMPLETE:**
  1. ✅ Persistent runtime with document ownership
  2. ✅ Full DOM mutation/read API coverage (createElement, appendChild, textContent, attributes, etc.)
  3. ✅ Event loop with timers (`setTimeout`/`setInterval`), microtasks (`queueMicrotask`), and event listeners
  4. ✅ Script loading pipeline with external script fetching and caching
  5. ✅ React counter validation with state persistence and event handling
- `requestAnimationFrame` deferred due to QuickJS evaluation issues (see `notes/requestAnimationFrame-issue.md`).
- `innerHTML` setter not working correctly (see `notes/innerHTML-issue.md` for investigation notes).
- Notes in `notes/` capture follow-ups identified while wiring the bridge so we can track tech debt as we go.

## Gaps To Close Before WPT

- **Runtime lifecycle:** we still rebuild an `HtmlDocument` from HTML strings each render; establish a single `HtmlDocument`/`BaseDocument` per navigation, keep it alive, and reuse one QuickJS runtime so timers/microtasks persist.
- **DOM mutation coverage:** extend `BlitzJsBridge`/`DomState` to support create/insert/remove/reorder nodes, attribute removal, class/style/dataset helpers, fragments, cloning, and batched flush semantics instead of serializing `inner_html`.
- **DOM read surface:** expose `document.getElementById`, traversal utilities, attribute getters, and node inspection methods so React can reconcile without falling back to HTML strings.
- **Event piping:** wire add/remove listener bookkeeping to Blitz events, funnel dispatch from Blitz → QuickJS, and respect `stopPropagation`/`preventDefault` decisions. Ensure listeners survive document reflows.
- **Timers & microtasks:** bridge `setTimeout`, `setInterval`, `queueMicrotask`, and `requestAnimationFrame` onto the existing Tokio runtime so async React features work predictably.
- **Script loading:** execute external scripts in document order (including Blossom URLs) and cache per origin/root hash rather than skipping them.
- **State persistence:** after QuickJS mutates the DOM, sync the authoritative `BaseDocument` back into `current_document` without reserializing for future renders.

## Near-Term Milestones

### 1. Persistent Runtime & Document Ownership

- Keep a single `HtmlDocument`/`BaseDocument` instance per navigation; hand mutable borrows to QuickJS instead of converting back to strings.
- Ensure `JsPageRuntime` attaches once, survives re-renders, and tears down cleanly on navigation or window close.

### 2. Mutation & Read API Coverage

- Expand `BlitzJsBridge` to generate node IDs, apply `createElement`/`createTextNode`/`appendChild`/`insertBefore`/`removeChild`/`replaceChild`/`cloneNode` via `DocumentMutator`.
- Add DOM read helpers (element lookups, child traversal, attributes, text) that pull straight from Blitz so React can diff against real state.

### 3. Event Loop & Timers

- Introduce a bidirectional event bridge: register listeners with Blitz, translate Blitz UI events into QuickJS event objects, and bubble/capture correctly.
- Implement timers/microtasks in Rust using the existing Tokio handle; make sure the queue drains before each paint.

### 4. Script Loading Pipeline

- Fetch classic scripts (HTTP + Blossom) in document order, execute inline and external code synchronously as required, and defer async/module work for later milestones.
- Cache fetched scripts by origin/root hash to avoid repeated downloads.

### 5. Definition of Done for Pre-WPT

- React counter sample (UMD) can bootstrap, attach to an existing root element, and respond to click events.
- Navigations retain JS state until teardown, and reloading a document does not duplicate DOM mutations or leak timers.
- All inline/external classic scripts execute with the same semantics as browsers for blocking vs async/defer.

## WPT Validation Strategy

1. **Harness:** reuse the existing `~/code/wpt` checkout. Evaluate the lightweight TypeScript harness in `~/code/deno` as a template; port the minimal pieces to Rust (test discovery, HTTP server, result aggregation) so we can integrate with `just ci`.
2. **Initial Suite:** start with a curated set (10–20 tests total) covering DOM Core tree mutations, `document.getElementById`, event dispatch, and timer basics. Track which tests pass/fail to measure progress.
3. **Execution:** run the WPT subset inside CI nightly and on PRs touching the JS runtime. Persist results (e.g., JSON summary) so regressions are obvious.
4. **Iteration:** expand coverage as features land (e.g., add HTML parsing, pointer events, timers). Record unsupported APIs directly in the plan to keep expectations aligned.

## Stretch Goals & Visibility

- Document supported/unsupported Web APIs in `docs/` as we implement them so downstream teams know capability limits.
- Add a Blitz accessibility/UI test that clicks through the React counter once the DOM/events/timers stack is stable.
- Track remaining gaps to full spec compliance in `notes/` (e.g., MutationObserver fidelity, layout queries) to inform future milestones.

Success remains: run the React 18 UMD counter app, handle user input, keep the DOM in sync without reparsing, pass the curated WPT subset, and keep `just ci` green.
