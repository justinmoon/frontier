# QuickJS Follow-Ups

- Cache QuickJS runtime per navigation to avoid rebuilding the snapshot for every inline script evaluation (current implementation parses HTML twice).
- Extend DOM bridge with more operations (attribute removal, append/remove children) and expose event listener stubs.
- Support `<script type="module">` and `<script type="text/typescript">` once the TypeScript transpilation pathway is ready.
- Surface JavaScript exceptions to the UI so authors can debug failing scripts without checking terminal logs.
- `BaseDocument::set_focus_to` still prints directly to stdout when focus changes. This surfaces in automated tests (see `runtime_document_handles_keyboard_and_ime_events`) and should be replaced with structured logging.
- IME dispatch currently exposes `event.value` but `event.imeState` is absent/empty in JS. Verify `insert_ime_event` wiring and ensure commit/preedit phases surface descriptive state for consumers.
