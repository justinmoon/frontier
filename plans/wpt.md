# WPT Enablement Plan (`wpt` branch)

## Goals
- Execute a curated slice of Web Platform Tests against the QuickJS + DOM runtime.
- Integrate the WPT run into `just ci` so it blocks regressions.
- Keep feedback fast (<5s) and manifest manageable while we expand coverage intentionally.

## Guiding Principles
- Reuse real runtime components (`JsPageRuntime`, `RuntimeDocument`) – no mocks, no alternate harnesses.
- Prefer `.any.js` tests and APIs we actively support; add new tests before implementing new features.
- Vendor only what we need: canonical `testharness.js/report.js`, curated tests, and a simple manifest.
- Document unsupported/flake cases in `notes/` for future follow-up.

## Deliverables
1. Minimal WPT runner wired into Rust tests (`src/wpt/runner.rs`, invoked from `tests/wpt_smoke.rs`).
2. Curated manifest (`tests/wpt/manifest.txt`) plus vendored resources under `third_party/wpt/`.
3. `just wpt` command and `just ci` integration.
4. Developer docs & notes updates (readme snippet, `notes/wpt-unsupported.md`).

## Milestones

### M1 – Harness Skeleton
- Branch off `wpt`.
- Study reference runner in `~/code/blitz/wpt/runner/` for parsing/report patterns.
- Implement minimal QuickJS harness that can load a JS file, expose `test`, `async_test`, assertions, and collect results.
- Add smoke test running a trivial inline WPT snippet to prove plumbing.

### M2 – First Real WPT Test
- Vendor `testharness.js` + `testharnessreport.js` and a single timer test (`html/webappapis/timers/zero-timeout.any.js`).
- Add manifest & loader so runner can map manifest entries → local files.
- Ensure harness parses report output into Rust assertions (pass/fail with names + diagnostics).
- Wire `just wpt` and include it in `just ci` (fail fast on harness regressions).

### M3 – Timers & DOM Core Slice
- Expand manifest to ~10 tests: timers (delay flooring, clearTimeout) and basic DOM tree ops (`dom/nodes/` helpers) that match current feature set.
- Fix runtime gaps revealed by tests (e.g., handle normalization, attribute removal).
- Ensure run completes <5s locally; parallelize tests within harness if needed.
- Capture unsupported tests/features in `notes/wpt-unsupported.md`.

### M4 – Event & Regression Coverage
- Add 3–5 event-focused tests (`dom/events/`) covering listener lifecycle and bubbling.
- Record corresponding UI regression via Blitz accessibility harness when functionality overlaps user flows.
- Update docs with run instructions, workflow for adding new tests, and sync steps for vendored files.

## Tooling & Maintenance
- Create `scripts/sync_wpt.sh` for pulling upstream files into `third_party/wpt/` with manual review checkpoints.
- Track flaky or slow tests separately; never skip silently.
- Revisit manifest quarterly to expand coverage (DOM Parsing, Fetch once API lands) guided by project priorities.
- Log future work/tech debt in `notes/` as gaps appear during test runs.

## Exit Criteria
- `just ci` (including WPT slice) passes reliably on clean checkout and in CI.
- At least 15 curated WPT tests green, spanning timers, DOM manipulation, and events.
- Documentation enables another contributor to update/add tests without assistance.
