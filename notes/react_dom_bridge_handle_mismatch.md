# React DOM Bridge Handle Mismatch — Recommended Path Forward

## What We Know
- QuickJS-side proxies are keyed by the string handle returned from `createElement`. React stores internals (e.g. `__reactFiber`) directly on those proxies.
- `DomState::allocate_node_id` generates temporary handles (`alloc_*`) that map to real Blitz DOM node ids once the `create_element` patch is applied.
- Any Rust→JS surface that later looks up the same node (e.g. `getElementById`, bubbling in `dispatchEvent`) currently returns the canonical Blitz DOM id ("33"), not the temporary handle.
- Because the proxy cache only knows about `alloc_*`, the new handle string creates a fresh proxy without the React metadata, so React's synthetic event system never finds the registered listener.

## Architecture Direction
1. **Make handles stable from the JS perspective.** Introduce a single place in `DomState` that translates between internal node ids (`usize`) and the public handle string that JS sees. Keep returning `alloc_*` for nodes created from JS so existing proxies stay valid, but ensure every Rust→JS call path converts node ids back into those public handles before QuickJS sees them.
2. **Track both directions explicitly.** Extend `DomState` with a reverse map (`usize -> String`) that is populated whenever we register `alloc_* -> node_id` in `CreateElement` / `CreateTextNode`, and cleaned up when nodes are removed or replaced. This avoids O(n) scans when translating ids.
3. **Centralise translation helpers.** Add methods such as `fn public_handle_for(&self, node_id: usize) -> String` and `fn maybe_public_handle(&self, handle: &str) -> Option<String>` that encapsulate the alias logic. These helpers should be the only way any binding returns a handle string to JS.
4. **Audit every binding that returns handles.** Update `handle_from_element_id`, `get_children`, `get_parent`, event targets/bubbling, and any other exported functions (`__frontier_dom_get_*`) to call the helper before handing results to JS. This keeps proxies consistent regardless of whether the node originated from HTML parsing or runtime creation.
5. **Normalise handles inside JS once.** In `createNodeProxy`, detect when we are handed a canonical Blitz id (after the Rust changes this should be rare, but events fired from Rust may still do it) by calling a new `__frontier_dom_public_handle(handle)` binding. If the handle maps to an existing proxy, reuse it; otherwise cache both the alias and the canonical id so subsequent lookups converge.
6. **Tighten removal bookkeeping.** When nodes are dropped (`remove_child`, `replace_child`, etc.), remove their entries from both maps so stale handles cannot be resurrected and memory stays bounded.
7. **Verify with real React flows.** After implementing the above, rerun `cargo test --test react_gui_integration_test -- --nocapture` and `just ci` to ensure React's click handler now fires and no regressions exist in other DOM interactions (especially child/parent traversal).

## Open Questions / Follow-ups
- Do we want to expose canonical handles to JS for debugging? If so, surface them via a read-only property (e.g. `node.__frontierInternalHandle`) so React continues to see the stable alias.
- Should future features (e.g. server-rendered hydration) skip temporary handles entirely by reserving real ids upfront? That remains an option, but stabilising the translation layer keeps the existing patch pipeline intact while we evaluate longer-term direction.
- Event ingestion from Blitz relays needs to use the same translation helper before calling into `__frontier_dispatch_event`; audit those call sites once we wire actual relay-driven interaction.

## Next Steps Checklist
- [ ] Add reverse-handle bookkeeping and translation helpers in `DomState`.
- [ ] Update every exported DOM binding to return the public handle.
- [ ] Teach `createNodeProxy` to collapse canonical handles back onto existing proxies.
- [ ] Cover the regression with an assertion in `tests/react_gui_integration_test.rs` that the counter increments when clicked.
- [ ] Run `just ci` and keep iterating until it passes.
