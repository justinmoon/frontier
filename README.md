# Frontier Browser

The Web is Dead

Long Live the Web

## QuickJS Demo

The QuickJS integration can be exercised locally without network access:

1. Build the demo asset: `assets/quickjs-demo.html` (already tracked in the repo).
2. Run the browser against the file: `just run file://$PWD/assets/quickjs-demo.html`.
3. The heading updates to "Hello from QuickJS!" and the console prints `JavaScript executed successfully`.

The same asset is exercised in `tests/quickjs_dom_test.rs`, so CI will fail if script execution regresses.

## React Demo

The React integration with GUI event handling can be tested locally:

```bash
./target/debug/frontier "file://$(pwd)/assets/react-sync-counter.html"
```

This demo shows:
- React 17 rendering a counter component
- Event listeners registered via `addEventListener`
- GUI clicks dispatching to JavaScript event handlers
- DOM updates from JavaScript reflected in the rendered UI

Click the "Increment" button and watch the counter update! The same mechanism works with vanilla JavaScript (try `simple-counter.html`).
