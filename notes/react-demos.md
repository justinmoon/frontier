# React demo fixtures

- React and ReactDOM UMD bundles are vendored under `assets/react-demos/vendor/`. When we upgrade React, copy the new minified files into that directory and update the HTML references.
- Shared static bundles live in `assets/react-demos/` (`counter.html`, `timer.html`, `index.html`). Each HTML file points to the vendored UMD scripts so the demos work without a JS package manager.
- `tests/quickjs_dom_test.rs` houses the coverage for each demo (counter + timer). Add new tests beside these helpers when introducing more fixtures.
- `just react-demos` shells to `cargo run --bin frontier -- "file://$(pwd)/assets/react-demos/index.html"`, keeping the workflow consistent with the standard Frontier launcher and avoiding special CLI flags.
- `assets/react-demos/test-index.html` is a giant click target that redirects to the timer demo; the ignored GUI regression test `tests/gui_automation_test.rs` launches Frontier, clicks the target via Enigo, and asserts that the current crash still reproduces.
