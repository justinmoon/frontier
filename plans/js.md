# Frontier JavaScript Integration Plan

## Mission

Build and ship a resilient QuickJS-backed DOM environment that can execute real sites (React 18 included), interop cleanly with Blitz, and provide a foundation for running Web Platform Tests (WPT).

## Current Baseline (codex branch)

- QuickJS runtime spins per-document, tied to a persistent `HtmlDocument`.
- DOM mutations flow through `BlitzJsBridge` with real handles (no HTML string diffs).
- Event propagation feeds Blitz events into JS via `JsEventHandler` and `DispatchOutcome`.
- Timer infrastructure exists, but needs polish before production.

## Critical Fixes (blockers before WPT)

1. **Timer busy loop** – add a minimum delay for zero/negative intervals in `TimerManager::register_timer` (`src/js/environment.rs`). Cover with a regression test that asserts `setInterval` does not peg a core.
2. **Comment fidelity** – ensure `BlitzJsBridge::create_comment_node` stores the payload and serializers emit `<!-- payload -->` (React relies on comment sentinels). Update `serialize_node` accordingly and add a DOM round-trip test.
3. **External blocking scripts** – extend `JsPageRuntime` so `<script src="...">` executes in document order. Reuse the manual loader currently in `main.rs` and delete the bespoke `load_external_scripts` helper once the runtime handles it.
4. **React counter validation** – port the Claude `tests/react_counter_test.rs` (trim exploratory prints) and make it drive the same APIs as production: fetch → attach document → dispatch via Blitz events.
5. **UI event bridge** – guarantee keyboard/input/IME events stay wired by exercising `RuntimeDocument` inside tests where possible. Follow the pattern in `tests/quickjs_dom_test.rs` and expand coverage instead of calling private helpers.

## Supplemental Improvements

- Normalize DOM handles the same way Blitz does before invoking JS listeners (review `src/js/dom.rs` + `src/js/environment.rs`).
- Add coverage for `document.createElementNS`, attribute removal, and tree mutation combos the React reconciler depends on.
- Write smoke tests that re-use the timer and event paths through `RuntimeDocument` instead of evaluating inline JS from Rust.

## WPT Timeline

Start curating a WPT subset **after** the five blockers above and the React counter test are green. At that point:

1. Pull an initial DOM Core + timers slice from `~/code/wpt` (look for `domparsing/` and `html/webappapis/timers/`).
2. Adapt the lightweight harness we used in `~/code/blitz` (search for `wpt_harness.rs`) so tests launch via the same QuickJS environment.
3. Add the curated suite to `just ci` once it passes locally; keep the scope tight (≈10–15 tests) until we stabilize more APIs.

## Research Pointers (~/code/)

- `~/code/blitz` – reference existing EventDriver examples and WPT harness scaffolding.
- `~/code/dioxus` – inspect how VDOM bridges manage handle normalization.

## Working Agreement

- NEVER use mocks in integration tests; exercise the same code paths the GUI hits.
- Keep the directory structure flat-ish; resist premature abstraction of the JS bridge.
- Log tech-debt discoveries in `notes/` as you encounter them.
- Ensure `just ci` stays green before declaring milestones complete.

Next agent: start with the timer + comment fixes, then wire external scripts and port the React counter test. Once those land, revisit this plan and queue the WPT harness work.
