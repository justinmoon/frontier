# Frontier JavaScript Integration Plan

## Mission

Deliver a production-ready QuickJS-powered DOM runtime that can execute real-world React 18 apps, interoperate seamlessly with Blitz, and form the base for a sustainable Web Platform Test (WPT) pipeline.

## Where We Stand (master)

- QuickJS engine + DOM bridge live per navigation and stay attached to a persistent `HtmlDocument`.
- `RuntimeDocument` now pumps timers during `poll`, registers wakers, and keeps Blitz aligned with JS-driven redraws.
- Timer queue floors zero-delay intervals (`setInterval(..., 0)`) and is covered by `intervals_floor_zero_delay` in `tests/quickjs_dom_test.rs`.
- React counter demo lives under `assets/react-counter/` with a smoke test that exercises the runtime end-to-end (manual external script loader for now).
- UI events flow through `JsEventHandler` → `DispatchOutcome`, respecting preventDefault/stopPropagation from JS listeners.

## Next Critical Work (pre-WPT)

1. **External classic scripts** – Teach `JsPageRuntime::run_blocking_scripts` to fetch/execute `<script src="...">` in document order so we can drop the custom `load_external_scripts` helper in `src/main.rs` and the test harness. Validate by running the React counter without manual loading.
2. **Comment fidelity** – `BlitzJsBridge::create_comment_node` still discards payloads; fix serialization to emit `<!-- text -->` and add a regression test (`tests/quickjs_dom_test.rs`). React uses comment sentinels for roots.
3. **Handle normalization** – Align JS-visible handles with Blitz’ internal mapping before listener dispatch (`src/js/dom.rs`, `src/js/environment.rs`). This resolves bugs called out in `notes/react-runtime.md` and keeps future event work predictable.
4. **Input surface coverage** – Extend integration tests to drive keyboard/input/IME events through `RuntimeDocument` (no direct JS eval shortcuts). Look at `tests/quickjs_dom_test.rs` for patterns and add new cases.
5. **React demo parity** – Wire `cargo run -- --react-demo` to reuse the same external-script pipeline and ensure the runtime survives repeated renders (timers cleared, listeners intact).
6. **Source layout health** – Several JS/bridge modules are ballooning (>500 LOC). Split the larger files into focused modules (e.g. move comment payload handling, timer machinery, event dispatch) once the current work lands to keep review cycles manageable.

## Supplemental Improvements

- Broaden DOM API coverage: `createElementNS`, attribute removal, cloning/deep tree mutations used by React reconciler. Keep changes small and back each with tests.
- Measure timer + microtask behavior under load; consider a shared microtask queue instead of ad-hoc `Promise.resolve().then` once we have more async features.
- Track open issues in `notes/` (e.g., requestAnimationFrame stub) and prune legacy helpers once equivalent Rust hooks land.

## WPT Timeline

Begin importing a curated WPT subset **after** items 1–4 above are complete and the React demo no longer relies on manual script loaders.

1. Pull a DOM Core + timers slice from `~/code/wpt` (start with `domparsing/` and `html/webappapis/timers/`).
2. Reuse the lightweight harness pattern in `~/code/blitz` (search for `wpt_harness.rs`) to spin tests inside our QuickJS runtime. Keep everything under the existing CI runner.
3. Gate `just ci` on ~10–15 curated tests. Grow the list only when new APIs ship; document unsupported features inline.

## Research Pointers (~/code/)

- `~/code/blitz` – EventDriver usage and prior WPT harness spikes.
- `~/code/dioxus` – Handle normalization + virtual DOM mutation patterns.
- `~/code/nsite` – External resource streaming/cache strategies that could back script loading.

## Working Agreement

- NEVER use mocks in integration tests; exercise the same paths the GUI hits.
- Keep the directory structure relatively flat until the APIs settle.
- Log tech-debt finds in `notes/` while they’re fresh.
- Ensure `just ci` passes before claiming a milestone is done.

Next agent: focus on the external script pipeline, comment serialization, and strengthening input/event tests. Once those land, revisit this plan to green-light the initial WPT import.
