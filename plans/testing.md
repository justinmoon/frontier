We want a Playwright-style, true end-to-end framework where tests only drive the UI exactly as a human would, so we can shrink manual QA while giving coding agents real-time visibility into the running app.

**Target Experience**
- Tests speak WebDriver-only APIs that model user intent (pointer, keyboard, navigation) and never reach inside our runtime guts.
- The harness captures rich artifacts (DOM snapshots, relay traffic, console output) so agents can see what the browser saw when failures happen.
- CI runs the full suite against the same nostr-integrated browser we ship, eliminating gaps between automation and production.

**Immediate Actions**
- Seal `HeadlessSession` and other automation internals behind the WebDriver server; publish a narrow `frontier_webdriver` client so tests cannot import low-level helpers even inside the repo.
- Implement user-realistic actions: pointer move/down/up sequences with hit testing, keyboard typing, focus management, scrolling, and form field reads—wired to the same event queue QuickJS uses.
- Flesh out the WebDriver surface to match spec (capabilities negotiation, element properties, pointer actions) and document which commands are allowed; deny merges that add custom RPCs or script execution.
- Replace raw CSS selectors with accessibility-first queries (role/name from Blitz tooling) to encourage resilient, user-visible interrogation of the UI.
- Keep the `pump` helper private; expose higher-level waits (`wait_for_text`, `wait_for_network_idle`) that poll via user-surface APIs rather than time travel.

**Guardrails for Authors**
- Add lint rules that fail when integration tests import modules under `src/automation` or call QuickJS internals.
- Ship a CLippy-style check (or cargo plugin) that verifies test files only use the sanctioned client helpers.
- Provide snippets/examples in `notes/testing` that demonstrate the happy path, so contributors have a clear template.

**Visibility & Debuggability**
- Record every significant user action and resulting DOM mutation to an artifact log so agents can reason about flaky behavior without rerunning locally.
- Capture periodic screenshots plus relay/network traces, wiring them into CI artifacts.
- Surface QuickJS exceptions prominently in the test output with actionable context (file, line, offending script) instead of burying them in logs.

**Longer-Term Bets**
- Run a nightly suite against multiple relay topologies and OS/browser variants, reusing the same user-action API to catch federation regressions early.
- Integrate hardware-accelerated builds once available so the same tests can optionally target the GPU-backed engine.
- Explore property-based user flows (randomized but user-valid interactions) once the deterministic action API is battle-tested.
