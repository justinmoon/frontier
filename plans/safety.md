# Safety Notes: QuickJS DOM Bridge Pointer Handling

## Background
Our QuickJS DOM bridge (`BlitzJsBridge`) caches a pointer to the active `BaseDocument` so that
JS-initiated DOM mutations can execute outside Rust’s borrow checker. Historically we stored
that pointer as `NonNull<BaseDocument>` and never refreshed it after the document mutated or
was replaced during React reconciliation. Once the underlying `HtmlDocument` was rebuilt, the
bridge dereferenced freed memory, leading to misaligned-pointer panics in `BaseDocument::get_node`.

## Risks of the Current Approach
- `NonNull<BaseDocument>` bypasses Rust’s lifetime checks; we must manually guarantee it stays
  valid. Any mismatch (e.g. reattaching a different document) turns into UB.
- Multiple mutation paths (set_text_content, append_child, clone_node, etc.) rely on the cached
  pointer, so a stale pointer cascades into user-visible crashes.

## Safer Alternatives
1. **Lifetime-bound bridge** – parameterise `BlitzJsBridge<'doc>` over `&'doc mut BaseDocument` and
   rebuild it whenever we attach/reattach. Rust then enforces that the bridge never outlives the
   document.
2. **Own the mutator** – keep a `DocumentMutator` (or wrapper) inside `DomState`/bridge and drop it
   when reattaching. All mutations go through the mutator while the borrow is active.
3. **Shared ownership** – wrap the document in `Rc<RefCell<BaseDocument>>` and have the bridge hold
   a `Weak`. When reattaching, upgrade the weak reference or swap the inner value. Borrow checks
   prevent use-after-free.

## Recommendation
Follow option 1 for minimal churn: reconstruct `BlitzJsBridge` on every attach/reattach and store
a borrowed reference instead of `NonNull`. That keeps FFI usage simple while letting the compiler
prove safety. If JS APIs require long-lived handles, wrap the document in `Rc<RefCell<_>>` and track
weak references instead of raw pointers.
