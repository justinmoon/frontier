# React runtime polyfills

- Added lightweight polyfills for `MessageChannel`, `MessagePort`, `MutationObserver`, and `HTMLIFrameElement` to unblock React's scheduler. These should eventually be replaced with spec-compliant implementations.
- `MessagePort` currently forwards messages with `Promise.resolve().then(...)`. Consider moving to a shared event loop integration once the browser runtime has its own microtask queue.
