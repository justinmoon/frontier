# DOM Mutation Cycle: Deep Dive

## Part 1: Why Can't JavaScript Hold Rust Pointers?

### The Core Problem: Memory Safety

JavaScript is a **garbage-collected** language. Rust uses **ownership + borrowing**. They're fundamentally incompatible:

```rust
// Rust's rules:
let mut node = Node::new();
let reference = &mut node;
drop(node);  // node is freed
// reference is now INVALID - Rust prevents this at compile time!
```

```javascript
// JavaScript's world:
let obj = { data: 42 };
let ref1 = obj;
let ref2 = obj;  // Multiple references OK
obj = null;      // Original reference gone
// ref1 and ref2 still work! GC keeps object alive
```

**What if we gave JavaScript a raw Rust pointer?**

```rust
// DANGEROUS - DON'T DO THIS
let node = Box::new(Node::new());
let ptr = Box::into_raw(node);  // Get raw pointer: *mut Node

// Pass to JavaScript somehow...
// JavaScript stores it as a number: 0x7f8a3c00e010

// Later in Rust:
drop(node_arena);  // Arena freed, pointer now dangling!

// JavaScript still has 0x7f8a3c00e010
// If JS calls back to Rust with this pointer... 💥 SEGFAULT
```

Rust can't track what JavaScript does with pointers, so **we can't give JS direct memory addresses**.

### How Chrome/Firefox Do It

**Chrome (V8 + Blink):**

Chrome's DOM is implemented in **C++**, not Rust, but the problem is similar. They use **wrapper objects**:

```cpp
// C++ side (Blink):
class Element : public Node {
    int internal_id_;
    // ... lots of C++ data
};

// V8 (JavaScript engine) side:
class V8Element : public ScriptWrappable {
    Element* impl_;  // Pointer to C++ object
    // When JS references this, V8 keeps the wrapper alive
    // Wrapper keeps the C++ object alive
};
```

**Key technique:** V8's garbage collector can trace into C++ objects via "wrapper tracing". When a JavaScript `Element` is still reachable, V8 tells Blink "keep this C++ Element alive".

This requires:
1. Custom GC integration between V8 and Blink
2. Wrapper objects that live in both worlds
3. Complex lifetime management (visit `https://source.chromium.org/chromium/chromium/src/+/main:third_party/blink/renderer/bindings/core/v8/to_v8_traits.h`)

**Firefox (SpiderMonkey + Gecko):**

Similar approach but uses **reflector objects**:

```cpp
// Gecko DOM (C++)
class Element : public nsINode {
    JS::Heap<JSObject*> mReflector;  // Back-pointer to JS wrapper
};

// SpiderMonkey traces both directions
// JS -> C++ and C++ -> JS
```

Both browsers essentially create a **bidirectional link** between JS objects and native objects, with GC integration.

### Why Frontier Uses String Handles Instead

**Frontier's approach is simpler and safer:**

```
JavaScript World          Rust World
───────────────          ──────────
  { [HANDLE]: "n42" } ──┬─> Parse "n42" -> node_id: 42
                        │
                        └─> Look up in BaseDocument.nodes[42]
                            └─> Returns &Node
```

**Benefits:**
1. ✅ **No GC integration needed** - QuickJS doesn't need to know about Rust objects
2. ✅ **Safe** - Invalid handle = lookup fails, no segfault
3. ✅ **Simple** - String is a primitive type, crosses FFI boundary easily
4. ✅ **Debuggable** - You can print "n42", can't print `0x7f8a3c00e010`

**Tradeoffs:**
1. ❌ Extra lookup on every operation (hash map: `O(1)` average)
2. ❌ String allocation overhead (mitigated by NODE_CACHE in JS)

### The Handle Format

```rust
// src/js/dom.rs
fn format_handle(node_id: usize) -> String {
    format!("n{}", node_id)  // 42 -> "n42"
}

fn parse_handle(handle: &str) -> Result<usize> {
    handle
        .strip_prefix('n')
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| anyhow!("invalid node handle: {}", handle))
}
```

**Why not just pass the number?**

Could work, but strings are more explicit:
- `"n42"` is clearly a handle
- `42` could be confused with other numbers
- Prefix allows versioning: `"n42"`, `"e42"` (for events?), etc.

---

## Part 2: Complete DOM Mutation Cycle (Diagram)

Let's trace: **`element.textContent = "Hello"`**

### The Actors

```
┌─────────────────┐
│   JavaScript    │  QuickJS runtime
│   (DOM_BOOTSTRAP)
└────────┬────────┘
         │ FFI calls
         ▼
┌─────────────────┐
│  Bridge Layer   │  __frontier_dom_* functions
│  (environment.rs)
└────────┬────────┘
         │ Rust calls
         ▼
┌─────────────────┐
│    DomState     │  Mutation tracking
│    (dom.rs)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ BlitzJsBridge   │  Direct DOM manipulation
│  (bridge.rs)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  BaseDocument   │  Blitz's rendered DOM tree
│  (blitz_dom)
└─────────────────┘
```

### The Complete Flow

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 1: JavaScript Execution
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📍 User code:
   const element = document.getElementById('message');
   element.textContent = "Hello";

┌──────────────────────────────────────────────────────────────────┐
│ JavaScript (DOM_BOOTSTRAP)                                        │
│                                                                    │
│  1. element.textContent = "Hello" triggers setter                │
│     └─> ElementProto property setter (environment.rs:1258)       │
│                                                                    │
│  2. Collect descendants to invalidate cache                      │
│     const stale = collectDescendants(this[HANDLE]);              │
│     // Returns ["n43", "n44", ...] for child nodes              │
│                                                                    │
│  3. Call bridge function                                         │
│     global.__frontier_dom_set_text(                              │
│         this[HANDLE],    // "n42"                                │
│         "Hello"                                                   │
│     );                                                            │
│                                                                    │
│  4. Invalidate cache                                             │
│     for (const handle of stale) {                                │
│         NODE_CACHE.delete(handle);                               │
│     }                                                             │
└──────────┬───────────────────────────────────────────────────────┘
           │
           │ FFI boundary crossing
           │ rquickjs converts:
           │   - JS string "n42" -> Rust String
           │   - JS string "Hello" -> Rust String
           ▼

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 2: Bridge Function
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Rust: __frontier_dom_set_text (environment.rs:229-246)          │
│                                                                    │
│  fn(ctx: Ctx<'_>, handle: String, value: Option<String>)        │
│                                                                    │
│  1. Extract state from closure                                   │
│     let state_ref = Rc::clone(&state);                           │
│     let mut state = state_ref.borrow_mut();                      │
│                                                                    │
│  2. Prepare value                                                │
│     let text = value.unwrap_or_default();  // "Hello"            │
│                                                                    │
│  3. Call DomState method                                         │
│     match state.set_text_content_direct(&handle, &text) {        │
│         Ok(()) => Ok(()),                                         │
│         Err(err) => dom_error(&ctx, err)  // Throw JS exception │
│     }                                                             │
└──────────┬───────────────────────────────────────────────────────┘
           │
           ▼

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 3: DomState
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Rust: DomState::set_text_content_direct (dom.rs:150-156)        │
│                                                                    │
│  pub fn set_text_content_direct(&mut self,                       │
│                                 handle: &str,                    │
│                                 value: &str) -> Result<()>       │
│                                                                    │
│  1. Create patch object                                          │
│     let patch = DomPatch::TextContent {                          │
│         handle: handle.to_string(),  // "n42"                    │
│         value: value.to_string(),    // "Hello"                  │
│     };                                                            │
│                                                                    │
│  2. Apply patch (modifies live DOM)                              │
│     self.apply_patch(patch)?;                                    │
│                                                                    │
│  3. Return success                                               │
│     Ok(())                                                        │
└──────────┬───────────────────────────────────────────────────────┘
           │
           ▼

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 4: Apply Patch
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Rust: DomState::apply_patch (dom.rs:~350)                       │
│                                                                    │
│  1. Record mutation for debugging                                │
│     self.record_mutation(patch.clone());                         │
│     // Stores in self.mutations: Vec<DomPatch>                   │
│                                                                    │
│  2. Match on patch type                                          │
│     match patch {                                                │
│         DomPatch::TextContent { handle, value } => {             │
│             // Continue below...                                 │
│         }                                                         │
│         // ... other patch types                                 │
│     }                                                             │
│                                                                    │
│  3. Parse handle                                                 │
│     let node_id = parse_handle(&handle)?;  // "n42" -> 42       │
│                                                                    │
│  4. Get bridge (if attached)                                     │
│     let bridge = self.bridge_mut()?;                             │
│     // Returns &mut BlitzJsBridge or error if not attached       │
│                                                                    │
│  5. Delegate to bridge                                           │
│     bridge.set_text_content(node_id, &value)?;                   │
└──────────┬───────────────────────────────────────────────────────┘
           │
           ▼

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 5: BlitzJsBridge
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Rust: BlitzJsBridge::set_text_content (bridge.rs:~150)          │
│                                                                    │
│  pub fn set_text_content(&mut self,                              │
│                          node_id: usize,                         │
│                          text: &str) -> Result<()>               │
│                                                                    │
│  1. Get mutable access to BaseDocument                           │
│     self.with_document_mut(|document, _id_index| {               │
│         // Now we have: &mut BaseDocument                        │
│                                                                    │
│  2. Create a mutator                                             │
│     let mutator = DocumentMutator::new(document);                │
│                                                                    │
│  3. Set text content                                             │
│     mutator.set_text_content(node_id, text)?;                    │
│     // This is Blitz's API for modifying DOM                     │
│                                                                    │
│  4. Trigger relayout                                             │
│     document.mark_dirty_subtree(node_id);                        │
│     // Tells Blitz this node needs re-layout/re-render           │
│                                                                    │
│     Ok(())                                                        │
│     })                                                            │
└──────────┬───────────────────────────────────────────────────────┘
           │
           ▼

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 6: BaseDocument (Blitz DOM)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Rust: blitz_dom::BaseDocument                                    │
│                                                                    │
│  Before:                                                          │
│  ┌────────────────────────────┐                                  │
│  │ Node #42 (Element)         │                                  │
│  │   tag: <div id="message">  │                                  │
│  │   children:                │                                  │
│  │     ├─ Node #43 (Text)     │                                  │
│  │     │    data: "Loading..."│                                  │
│  │     └─ (layout box)        │                                  │
│  └────────────────────────────┘                                  │
│                                                                    │
│  mutator.set_text_content(42, "Hello"):                          │
│    1. Remove all children of node 42                             │
│    2. Create new Text node                                       │
│    3. Append as child                                            │
│                                                                    │
│  After:                                                           │
│  ┌────────────────────────────┐                                  │
│  │ Node #42 (Element)         │                                  │
│  │   tag: <div id="message">  │                                  │
│  │   children:                │                                  │
│  │     └─ Node #567 (Text)    │  ← New node!                    │
│  │          data: "Hello"     │                                  │
│  │                            │                                  │
│  │   dirty: true  ◄───────────┼─ Marked for re-render           │
│  └────────────────────────────┘                                  │
└──────────────────────────────────────────────────────────────────┘

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 7: Return to JavaScript
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

┌──────────────────────────────────────────────────────────────────┐
│ Stack unwinds:                                                    │
│                                                                    │
│  BlitzJsBridge::set_text_content  → Ok(())                       │
│           ↓                                                       │
│  DomState::apply_patch            → Ok(())                       │
│           ↓                                                       │
│  DomState::set_text_content_direct→ Ok(())                       │
│           ↓                                                       │
│  __frontier_dom_set_text          → Ok(())                       │
│           ↓                                                       │
│  (rquickjs converts Ok(()) to JS undefined)                      │
│           ↓                                                       │
│  ElementProto setter completes                                   │
│           ↓                                                       │
│  User's JavaScript continues...                                  │
└──────────────────────────────────────────────────────────────────┘

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STEP 8: Next Frame Render
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Later, in the event loop:

┌──────────────────────────────────────────────────────────────────┐
│ Blitz Rendering Pipeline                                         │
│                                                                    │
│  1. Check dirty flag                                             │
│     if document.has_dirty_nodes() {                              │
│                                                                    │
│  2. Style resolution                                             │
│     - Recompute styles for node #42 and descendants             │
│     - "Hello" inherits font, color, etc.                         │
│                                                                    │
│  3. Layout                                                        │
│     - Measure text "Hello" (5 characters)                        │
│     - Compute new bounding box                                   │
│     - Might be narrower than "Loading..." (11 chars)            │
│                                                                    │
│  4. Paint                                                         │
│     - Clear old text rendering                                   │
│     - Draw "Hello" with computed font/color/position             │
│                                                                    │
│  5. Compositor                                                    │
│     - Upload to GPU texture                                      │
│     - Send to window                                             │
│     }                                                             │
└──────────────────────────────────────────────────────────────────┘

User sees: "Hello" on screen! ✨
```

---

## Key Observations

### 1. **Layered Abstraction**

Each layer has a specific job:
- **JavaScript**: High-level DOM API
- **Bridge functions**: FFI boundary, error handling
- **DomState**: Mutation tracking, handle management
- **BlitzJsBridge**: Direct BaseDocument manipulation
- **BaseDocument**: Actual DOM tree + rendering state

### 2. **Error Handling**

Errors can occur at any layer:

```rust
// Parse error:
parse_handle("invalid") → Err("invalid node handle")
    ↓
dom_error(&ctx, err)  // Throws JavaScript exception
    ↓
JavaScript catch block (if any)
```

### 3. **The Handle Lookup**

```rust
// "n42" goes through multiple lookups:

Step 3: parse_handle("n42") → 42
Step 5: document.nodes[42] → &mut Node
        (array/vector lookup, O(1))

// Total overhead: string parse + bounds check
// Typically < 10 nanoseconds on modern hardware
```

### 4. **Why NODE_CACHE Matters**

```javascript
// Without cache:
const el1 = document.getElementById('msg');  // Creates wrapper
const el2 = document.getElementById('msg');  // Creates ANOTHER wrapper
console.log(el1 === el2);  // false! ❌

// With cache:
const el1 = document.getElementById('msg');  // Creates wrapper, stores in cache
const el2 = document.getElementById('msg');  // Returns cached wrapper
console.log(el1 === el2);  // true! ✅
```

The cache ensures **object identity**, critical for JavaScript equality checks and WeakMaps.

### 5. **Synchronous Mutation**

Unlike some frameworks (React, Vue), this is **immediate**:

```javascript
element.textContent = "Hello";
console.log(element.textContent);  // "Hello" immediately!

// The DOM is updated synchronously
// (though rendering happens later)
```

This matches browser behavior exactly.

---

## Alternative: How it Works in Chrome (for comparison)

Chrome's V8 + Blink does something similar but more complex:

```
JavaScript                 V8                      Blink (C++)
──────────────────────────────────────────────────────────────────
element.textContent = "Hello"
    │
    ├─> V8 calls setter
    │       │
    │       ├─> Unwrap V8Element
    │       │       │
    │       │       └─> Get impl_ pointer
    │       │               │
    │       │               └──────────────────> Element::setTextContent("Hello")
    │       │                                         │
    │       │                                         ├─> Modify C++ DOM tree
    │       │                                         ├─> Queue style recalc
    │       │                                         └─> Return
    │       │               ┌──────────────────────────┘
    │       └───────────────┘
    │
    └─> JavaScript continues
```

**Key difference:** Chrome uses **C++ pointers** in wrappers, but has elaborate GC integration to keep them valid. Frontier uses **string handles** for safety and simplicity.

---

## Summary

1. **JavaScript can't hold Rust pointers** because:
   - GC vs. ownership mismatch
   - Safety: dangling pointers would cause crashes
   - Chrome/Firefox solve this with complex GC integration

2. **Frontier uses string handles** instead:
   - Safe: bad handle = error, not crash
   - Simple: no GC integration needed
   - Debuggable: "n42" is readable

3. **DOM mutations flow through layers**:
   - JS wrapper → Bridge function → DomState → BlitzJsBridge → BaseDocument
   - Each layer adds error handling, tracking, or translation
   - Final mutation happens in Blitz's real DOM tree
   - Rendering happens later, asynchronously

The whole cycle takes microseconds, fast enough for interactive UIs!
