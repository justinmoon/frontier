# QuickJS Integration Plan (Rust Host)

## Objective
Embed QuickJS inside the Frontier Rust host so that inline `<script>` blocks from fetched HTML can execute against a small, predictable DOM surface. Start with plain JavaScript execution and DOM mutations that update the rendered document, then layer TypeScript transpilation in a follow-up milestone.

## Current Context
- Documents are fetched in `src/navigation.rs` and handed to the UI loop as `FetchedDocument { base_url, contents, … }`.
- `ReadmeApplication::render_current_document` (in `src/readme_application.rs`) wraps the raw HTML with the URL bar shell, builds a `blitz_html::HtmlDocument`, and hands it to Blitz for rendering.
- There is no JavaScript runtime today; HTML is treated as static content.

The QuickJS work slots naturally between the fetch stage and the render stage: mutate `FetchedDocument.contents` (or layer a hydrated DOM tree) before we hand the HTML off to Blitz.

## Milestone 1 — QuickJS Runtime Bootstrap
1. Add `rquickjs` (or another maintained QuickJS binding) to `Cargo.toml`. Prefer a crate that vendors QuickJS so we do not manage C build steps manually.
2. Create `src/js/runtime.rs` that exposes a `QuickJsEngine` with:
   - `QuickJsEngine::new()` constructing runtime + context.
   - `fn eval(&self, source: &str, filename: &str) -> anyhow::Result<()>` for script execution.
   - Hooked `console.log` that forwards to `tracing::info!`.
3. Add a small smoke test under `tests/quickjs_runtime.rs` that instantiates the engine and logs "hello" to ensure the runtime actually executes scripts inside our tree (no mocks).
4. Thread ownership carefully (runtime/context must outlive JS values). Hide raw pointers behind safe Rust wrappers using the binding’s API.

## Milestone 2 — Minimal DOM Surface & Event Channel
1. When we receive HTML, build a `DomSnapshot`:
   - Parse the HTML body with `scraper` (or `html5ever` directly) to index nodes by `id` and expose a simple tree we can mutate in Rust.
   - Store both the parsed tree and the original HTML string so we can reserialize after mutations.
2. Expose the snapshot to QuickJS by constructing JS globals:
   - `document.getElementById(id)` returning a proxy that holds a Rust `NodeHandle`.
   - Element prototype supports `textContent`, `innerHTML`, `setAttribute`, and `addEventListener` stubs (no-ops for now except logging unimplemented calls).
3. Provide a `FrontierEmitter` JS object (`frontier.emitDomPatch(patch)`) implemented in Rust.
   - Collect patches in a `Vec<DomPatch>` (e.g. `SetText { id, value }`, `SetAttribute { id, name, value }`).
   - Immediately apply the patch to the `DomSnapshot` so that subsequent JS reads see the updated DOM.
4. Decide scope: the initial milestone only needs enough surface to support `document.getElementById(...).textContent = '...'` plus `console.log`. Keep the API deterministic and well documented.

## Milestone 3 — Pipeline Integration
1. Introduce an evaluation step in `ReadmeApplication` (likely inside `set_document` or a dedicated `apply_inline_scripts(&mut FetchedDocument)` helper):
   - Scan the HTML for `<script>` tags with no `src` and type `text/javascript`/unset.
   - For each block, feed the contents to `QuickJsEngine::eval`.
   - On errors, capture exception details and surface them in the console/logs while leaving the rest of the page intact.
2. After executing all scripts, reserialize the mutated `DomSnapshot` back into a string and replace `FetchedDocument.contents` before calling `render_current_document`.
3. Keep the engine scoped per document navigation for now (no persistent global state). Recreate a fresh runtime whenever we process a new document so we avoid cross-page leakage.
4. Ensure the QuickJS work runs on a background thread or synchronously before we enter the UI event loop; avoid holding the Tokio runtime hostage. Initially we can run synchronously on the fetch handler thread and measure later.
5. Update logging so that console output appears in our existing tracing pipeline with clear page URL context.

## Milestone 4 — Demo Asset & Tests
1. Add `assets/quickjs-demo.html` containing:
   ```html
   <h1 id="message">Loading…</h1>
   <script>
     document.getElementById('message').textContent = 'Hello from QuickJS!';
     console.log('JavaScript executed successfully');
   </script>
   ```
2. Extend `tests/layout_test.rs` (or add a new UI test) that loads the demo HTML through the same helper we use for other layout tests and asserts the resulting DOM contains "Hello from QuickJS!". Use the real QuickJS runtime—no mocks.
3. Provide manual instructions (`README` snippet or dev note) to run `just run file://$PWD/assets/quickjs-demo.html` and observe the DOM + console output.
4. Ensure `just ci` (nix-driven) passes locally after the new dependency and tests are in place.

## Milestone 5 — TypeScript (Follow-Up)
1. Bundle `typescriptServices.js` (2.8 MB) under `assets/` and ship it with the browser.
2. Extend the runtime wrapper with a `transpile_typescript(ts_source: &str) -> anyhow::Result<String>` using the in-process compiler.
3. For `<script type="text/typescript">`, call the transpiler, then evaluate the emitted JavaScript within the same runtime.
4. Add a demo asset and regression test proving TypeScript transpilation works (e.g., a counter component with type annotations).

## Risks & Open Questions
- HTML parsing fidelity: `scraper` handles well-formed documents but may mangle malformed HTML—monitor for regressions.
- Performance: Creating a new QuickJS runtime per navigation is simplest but may be slow for large pages. Measure before optimizing.
- Security: Executing arbitrary scripts from fetched pages opens the door to network observers altering content. We should gate script execution behind trust decisions later.
- Future DOM features: we only expose a sliver of the DOM; document what is supported so content authors know the limits.

## Definition of Done (Milestone 1–4)
- QuickJS runtime runs inside Frontier with a safe Rust wrapper.
- Inline scripts from demo HTML mutate the DOM and logs appear in STDOUT.
- A regression test covers the end-to-end behavior using the real runtime.
- Documentation/demo instructions updated, and `just ci` reports success.
