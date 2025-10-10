# WPT Coverage Gaps

Curated WPT slice is intentionally small. Record failing candidates here so future work can expand coverage.

- `dom/events/EventTarget-*` – requires richer `EventTarget` + `window` listener plumbing (current polyfill only handles `load`).
- `html/webappapis/timers/type-long-settimeout.any.js` – expects timer clamping semantics for very large delays; current QuickJS integration triggers the guard timer.
- Additional DOM APIs (querySelector, mutation observers, etc.) are unimplemented; add entries as new gaps are discovered.

When addressing any item above, update the `third_party/wpt` submodule to the desired revision, add the test path to `tests/wpt/manifest.txt`, and extend `wpt_smoke.rs` so CI covers the new behavior.
