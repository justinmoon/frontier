Frontier’s automation story now revolves around the full chrome process instead of QuickJS-only headless helpers. `automation_host` launches `ReadmeApplication` on the main thread, exposes a WebDriver-flavoured HTTP surface, and the new `automation_client` crate gives tests ergonomic, sanctioned access to that surface. Every command captures DOM snapshots and metadata so CI failures come with artifacts by default.

### What’s Already Landed

1. **Client API & Guardrails**
   - Shipped `AutomationHost`/`AutomationSession` in `src/automation_client/` for Rust tests; includes spawn helpers, typed selectors, waits, and process lifetime management.
   - Introduced a Clippy denylist to block direct use of `AutomationCommand`/`AutomationState` outside the host and the application.

2. **Input Fidelity**
   - Implemented structured pointer and keyboard sequences (including focus helpers, scrolling, accessibility role/name selectors, IME text input, and modifier-aware shortcuts).
   - Click typing helpers layer on top of the richer sequence API so simple tests stay concise.

3. **Observation & Debugging**
   - Each automation command snapshots the DOM and writes structured artifacts under `target/automation-artifacts/<session>/<step>`.
   - Host wiring surfaces errors back to the client with serialized payloads ready for CI attachment.

4. **Test Coverage & Migration**
   - Added `tests/automation_interaction.rs` plus a regression harness for the chrome back button bug that reproduce issues against the full browser.
   - Retired the legacy WebDriver integration suite in favour of `automation_client`, so CI exercises the browser exclusively through the new host.

5. **Wait & Synchronisation Primitives**
   - Client exposes `wait_for_text`, `wait_for_element`, timed pump helpers, and selector-based existence checks.

6. **CI & Platform Experience**
   - CI drives the new host via `cargo test`; the binary honours `AUTOMATION_BIND`, `AUTOMATION_ASSET_ROOT`, and `AUTOMATION_ARTIFACT_ROOT` so runners can manage ports and artifacts.

### Still To Do

1. **Guardrails**
   - Extend linting so all UI/E2E tests consume `automation_client` (or documented exceptions) and remove the remaining `HeadlessSession` exports that let tests bypass the host.
   - Add similar checks for other languages once non-Rust harnesses appear.

2. **Richer Observability**
   - Capture console streams, QuickJS exceptions, network/relay traces, and optional screenshots/video alongside DOM dumps.
   - Convert artifacts into structured bundles that CI can upload automatically.

3. **Suite Migration**
   - Audit the remaining QuickJS-only regression tests and decide which need host-backed coverage versus documented exceptions.
   - Backfill end-to-end tests for navigation history, chrome controls, and relay/Nostr flows uncovered during triage.

4. **Higher-Level Waits**
   - Layer user-surface waits such as `wait_for_network_idle`, accessibility queries, and form-field assertions so tests rarely call `pump` directly.
