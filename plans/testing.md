We now have a runnable automation host that launches the full Frontier chrome and exposes a thin HTTP/WebDriver façade. The remaining work focuses on turning that foundation into a production-grade harness capable of comprehensive end-to-end coverage.

### Remaining Work

1. **Client API & Guardrails**
   - Publish a sanctioned Rust client (and prepare for other languages) that wraps the HTTP endpoints; make it the only path tests can use.
   - Add linting/CI checks to block direct imports of `src/automation` internals or QuickJS hooks from tests.

2. **Input Fidelity**
   - Expand the host with full pointer sequences (move/down/up, hover, drag, wheel) and richer keyboard shortcuts.
   - Provide focus management helpers, scrolling, and accessibility-first queries (role/name) for resilient selectors.

3. **Observation & Debugging**
   - Capture DOM snapshots, console output, and relay/network traces around each command; emit artifacts in CI.
   - Offer optional screenshots/video and surface QuickJS exceptions prominently in test output.

4. **Test Suite Migration & Coverage**
   - Port all existing integration/e2e tests to the automation host and remove legacy headless helpers.
   - Identify missing coverage across chrome controls, rendered document behaviors, navigation/history, Nostril/relay flows, etc., and add end-to-end tests for each gap.

5. **Wait & Synchronisation Primitives**
   - Replace raw `pump` usage with higher-level waits (`wait_for_text`, `wait_for_network_idle`, etc.) driven by observable UI state.

6. **CI & Platform Experience**
   - Ensure the harness runs in CI across supported platforms and, over time, exercise multiple relay/network topologies.
   - Plan for GPU-accelerated builds so the same tests can validate both CPU and GPU renderers.

7. **Future Enhancements**
   - Explore property-based/randomized user flows once deterministic coverage is stable.
   - Schedule nightly runs with varied relay sets to catch federation regressions early.
