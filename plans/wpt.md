# WPT Strategy

## Targets
- `just wpt` stays the fast gatekeeper: execute the curated manifest of tests we expect to stay green on every PR and fail CI on any regression.
- `just wpt-full` becomes the exhaustive runner: walk the entire WPT manifest that Frontier can load (all `.any.js`, `.html`, etc.) so we surface gaps outside the curated slice.

## Automation
- Nightly CI job runs `just wpt-full`, stores JSON/CSV summary artefacts, and publishes a history graph (pass/fail counts) so we can measure coverage over time.
- When `wpt-full` finds tests that newly pass, open an automated PR that promotes them into the curated manifest and removes the unexpected-pass warning.

## Near-Term Work
- Build a manifest generator that reads WPT’s `MANIFEST.json` and emits both curated and full lists (avoids hand-maintaining large files).
- Update the runner so `wpt-full` can stream results (progress + opt-in filtering) without exploding runtime.

