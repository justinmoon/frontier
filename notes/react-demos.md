# React demo fixtures

- React dependencies are installed with `bun install`; run this once (or after version bumps) from the repo root to refresh `node_modules` and `bun.lock`.
- Shared static bundles live in `assets/react-demos/` (`counter.html`, `timer.html`, `index.html`). Each HTML file pulls React/ReactDOM directly from `node_modules` so we avoid vendoring builds.
- `tests/quickjs_dom_test.rs` houses the coverage for each demo (counter + timer). Add new tests beside these helpers when introducing more fixtures.
- `just react-demos` shells to `cargo run --bin frontier -- "file://$(pwd)/assets/react-demos/index.html"`, keeping the workflow consistent with the standard Frontier launcher and avoiding special CLI flags.
- `assets/react-demos/test-index.html` is a giant click target that redirects to the timer demo; the ignored GUI regression test `tests/gui_automation_test.rs` launches Frontier, clicks the target via Enigo, and asserts that the current crash still reproduces.
