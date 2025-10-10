# Frontier Browser

The Web is Dead

Long Live the Web

## QuickJS Demo

The QuickJS integration can be exercised locally without network access:

1. Build the demo asset: `assets/quickjs-demo.html` (already tracked in the repo).
2. Run the browser against the file: `just run file://$PWD/assets/quickjs-demo.html`.
3. The heading updates to “Hello from QuickJS!” and the console prints `JavaScript executed successfully`.

The same asset is exercised in `tests/quickjs_dom_test.rs`, so CI will fail if script execution regresses.

## Web Platform Tests

Frontier ships a curated Web Platform Test (WPT) slice that exercises the QuickJS runtime end-to-end.

- Run the suite locally with `just wpt`. This executes the manifest-backed tests under `tests/wpt/manifest.txt`.
- Web Platform Tests are available through the `third_party/wpt` submodule. Run `git submodule update --init --recursive` after cloning.
- Add new coverage by updating the submodule to the desired revision and appending relative paths to the manifest.
- `cargo test` (and therefore `just ci`) executes the same slice, so regressions will block CI.
- Today’s baseline focuses on timer semantics. DOM/Event coverage is tracked in `notes/wpt-unsupported.md` once the relevant APIs land.
