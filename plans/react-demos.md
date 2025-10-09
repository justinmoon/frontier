# React Micro-Demos Roadmap

## Why
- We want confidence that Frontier’s QuickJS runtime and DOM bridge stay stable before layering complex stacks like Next.js/Ants.
- React already runs a counter demo on master; expanding to a small battery of stateful components (timers, CRUD, filters) gives broad coverage of user interactions, timers, and keyed updates.
- These fixtures become living regression tests, catching DOM/event regressions early and giving us a safe sandbox before we attempt nostr networking flows.

## Plan
1. **Asset Scaffolding**
   - Add `assets/react-demos/` containing standalone HTML bundles for each demo.
   - Reuse the existing counter harness pattern so `quickjs_dom_test.rs` can spin up a runtime against each asset.

2. **Timer Demo**
   - React component with Start/Stop buttons driving a `setInterval` clock state.
   - Test pumps timers, toggles buttons, and asserts the displayed time advances only when running.

3. **CRUD List Demo**
   - Text input + add button producing list items with remove buttons.
   - Test sends keyboard events and clicks via `RuntimeDocument`, asserting list length and contents mutate as expected.

4. **Filterable Table Demo**
   - Render a static dataset and a filter input (7GUIs filtering task style) plus reset button.
   - Test programmatically updates filter text and verifies DOM row counts refresh accordingly.

5. **Harness & Helpers**
   - Extend `tests/quickjs_dom_test.rs` with one test per demo, factoring helpers for repeated event dispatch (keydown, click, timer pump).
   - Ensure each test serializes DOM at end and checks key selectors/handles.

6. **Documentation**
   - Capture setup + intended coverage in `notes/` so future devs know how/when to use or extend the demos.
