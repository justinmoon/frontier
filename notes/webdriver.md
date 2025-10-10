# WebDriver harness

- Minimal WebDriver server now wraps the headless session worker. Supported commands: create session (file/url), navigate, `GET /session/:id/url`, `GET /session/:id/source`, find element (CSS `#id` selectors only), click, element text, delete session, and the `POST /session/:id/frontier/pump` helper to advance timers while headless.
- Element references are tracked per session with generated UUID handles, so clients can follow the WebDriver element protocol without leaking raw selectors.
- Known gaps:
  - Only synchronous interactions are implemented; no script execution or keyboard input yet.
  - CSS selectors are constrained to `#id`. Extending the headless harness to support richer queries would unlock more tests.
  - Returned JSON mimics the WebDriver spec shape but we should audit exact fields (especially capabilities negotiation) before public exposure.
- `tests/webdriver_test.rs` drives the counter demo end-to-end using the HTTP API; expand this with timer coverage once we add accessor commands for form fields (e.g., input value reads).
