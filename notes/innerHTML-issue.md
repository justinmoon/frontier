# innerHTML Setter Not Working

## Issue

The `innerHTML` setter successfully emits the correct DOM patch and calls `bridge.set_inner_html()`, but the changes are not reflected when reading `innerHTML` back or when serializing the document.

## Investigation

- The JavaScript proxy correctly intercepts `innerHTML` assignments
- The `emitPatch` function correctly creates an `InnerHtml` patch with the right data
- The `apply_patch` function in `dom.rs` correctly routes to `bridge.set_inner_html()`
- The `BlitzJsBridge::set_inner_html()` function calls `DocumentMutator::set_inner_html()`

## Likely Cause

The `DocumentMutator::set_inner_html()` implementation in blitz-dom may not be correctly parsing and inserting the HTML content, or the serialization path may not be reading the updated DOM structure.

## Test Case

```javascript
const root = document.getElementById('root');
root.innerHTML = '<span>Test Value</span>';
const after = root.innerHTML; // Returns empty or original content, not '<span>Test Value</span>'
```

## Workaround

Use `textContent` for simple text updates instead of `innerHTML` for now. The React counter demo has been updated to use `textContent` and static DOM elements.

## TODO

- Investigate `DocumentMutator::set_inner_html()` implementation in blitz-dom
- Check if HTML parsing is working correctly
- Verify that serialization reads from the correct DOM nodes after mutation
- Add comprehensive innerHTML tests once fixed
