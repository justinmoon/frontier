# React GUI Integration - Follow-up Work

This document outlines the proper architecture for JavaScript execution in the Frontier browser, building on the quick-win implementation.

## Current State (Updated 2025-10-06)

**What we have:**
- ✅ React UMD bundles load and execute
- ✅ QuickJS promise/job queue processing implemented
- ✅ Runtime created synchronously on GUI thread and stored
- ✅ React 17 legacy sync mode works (ReactDOM.render)

**What doesn't work yet:**
- ❌ React 18 concurrent rendering (`createRoot()`)
- ❌ GUI event handlers → JS runtime integration
- ❌ Dynamic re-rendering after user interactions

**Critical Discovery - React 18 Concurrent Rendering:**

React 18's `createRoot().render()` uses **concurrent rendering** which requires:
1. An event loop (MessageChannel or setTimeout)
2. Multiple render passes scheduled asynchronously
3. Scheduler that processes work in chunks

Our current implementation:
- ✅ Processes QuickJS promise queue (`runtime.execute_pending_job()`)
- ❌ No MessageChannel API
- ❌ No event loop running to process setTimeout(0) callbacks
- ❌ Render completes but DOM updates are scheduled, not executed

**Solution:** Use React 17's legacy sync mode (`ReactDOM.render()`) which renders synchronously and works perfectly in our environment. React 18 concurrent mode requires Phase 4 (MessageChannel + event loop integration).

**Previous State (Option 2 - Abandoned):**
- Background JS runtime loading (non-blocking GUI)
- HTML renders immediately, scripts load asynchronously
- Runtime was created in background thread and discarded (not Send-safe)
- Moved back to synchronous runtime creation on GUI thread

## Phase 1: Proper Navigation Integration (Option 3)

**Goal:** Pre-fetch external scripts during navigation, have runtime ready when document loads.

### Changes Needed

1. **Extend `FetchedDocument` structure**
   ```rust
   pub struct FetchedDocument {
       pub contents: String,
       pub scripts: Vec<ScriptDescriptor>,
       pub fetched_scripts: HashMap<usize, String>, // ← NEW: Pre-fetched external scripts
       // ... existing fields
   }
   ```

2. **Move script fetching into `prepare_navigation()`**
   - Extract `ScriptFetcher` logic from `JsPageRuntime`
   - Fetch all external scripts during navigation
   - Store fetched content in `FetchedDocument`

3. **Make `JsPageRuntime::new()` synchronous again**
   ```rust
   pub fn new(
       html: &str,
       scripts: &[ScriptDescriptor],
       fetched_scripts: HashMap<usize, String>, // ← Pre-fetched
       config: DocumentConfig,
   ) -> Result<Option<Self>>
   ```

4. **Update `set_document()` to create runtime synchronously**
   - No spawning needed
   - Runtime ready immediately
   - Execute scripts inline

### Benefits
- ✅ Clean architecture - navigation handles I/O, GUI handles display
- ✅ Fast - scripts already loaded when rendering
- ✅ Single-phase rendering
- ✅ Synchronous `set_document()` again

### Estimated Effort
~4-6 hours of refactoring

---

## Phase 2: Runtime-DOM Integration

**Goal:** Connect JS runtime mutations back to rendered DOM.

### Problem
Currently the runtime modifies its own `HtmlDocument` copy, but the GUI renders a separate document. Changes don't propagate.

### Solution

#### Option A: Shared Document (Complex)
```rust
pub struct ReadmeApplication {
    current_document: Arc<RwLock<HtmlDocument>>, // ← Shared
    current_runtime: Option<JsPageRuntime>,
}

// Runtime mutates shared document
runtime.execute_scripts(); // Modifies Arc<RwLock<HtmlDocument>>

// GUI re-renders from shared state
self.compose_html() // Reads Arc<RwLock<HtmlDocument>>
```

**Challenges:**
- Lock contention between JS and rendering
- Threading complexity
- Blitz may not support shared documents

#### Option B: Patch-Based Updates (Clean)
```rust
// Runtime emits patches
let patches = runtime.execute_scripts();

// Apply patches to GUI's document
for patch in patches {
    self.current_document.apply(patch);
}

// Re-render
self.compose_html()
```

**Implementation:**
1. Runtime collects all DOM mutations as patches
2. Return patches from `run_blocking_scripts()`
3. GUI applies patches to its document
4. Trigger re-render via event

**Recommended:** Option B - cleaner separation of concerns

### Estimated Effort
~8-10 hours

---

## Phase 3: Event Handling Integration

**Goal:** User interactions trigger JS event handlers, JS can schedule DOM updates.

### Components

1. **GUI → Runtime: Event Dispatch**
   ```rust
   fn handle_click(&mut self, element_id: String) {
       if let Some(runtime) = &mut self.current_runtime {
           let patches = runtime.dispatch_event(&element_id, "click");
           self.apply_patches(patches);
       }
   }
   ```

2. **Runtime → GUI: Async Updates**
   ```rust
   // setTimeout, requestAnimationFrame, etc.
   runtime.process_timers() -> Vec<DomPatch>
   ```

3. **Event Loop Integration**
   ```rust
   // In main event loop
   loop {
       // Handle window events
       event_loop.poll();

       // Process pending JS timers/microtasks
       if let Some(patches) = runtime.process_pending() {
           apply_patches(patches);
       }

       // Render frame
       render();
   }
   ```

### Challenges
- Winit event loop is synchronous
- Need to poll JS timers/microtasks each frame
- React's concurrent rendering needs multiple update cycles

### Estimated Effort
~12-16 hours

---

## Phase 4: Concurrent Rendering Support

**Goal:** React 18's `createRoot()` works fully with state updates.

### Problem
React 18 uses concurrent rendering:
- Schedules updates via `MessageChannel`
- Requires event loop to process async tasks
- Multiple render passes for single state update

### Solution Path

1. **Implement microtask queue polling**
   ```rust
   // Each frame
   runtime.process_microtasks() // Run queued promises
   ```

2. **Message channel support**
   ```rust
   global.MessageChannel = function() {
       // Queue messages for next frame
   }
   ```

3. **Scheduler integration**
   - React uses `scheduler` package
   - Needs `requestIdleCallback` or `MessageChannel`
   - Poll message queue each frame

### Alternative: React 17 Sync Mode
For now, provide example that uses sync rendering:
```javascript
// Instead of:
const root = createRoot(document.getElementById('root'));

// Use (legacy):
ReactDOM.render(<App />, document.getElementById('root'));
```

This works immediately without concurrent rendering support.

### Estimated Effort
- Full concurrent support: ~20-30 hours
- React 17 examples: ~2 hours

---

## Phase 5: Performance & Optimization

### Issues to Address

1. **Script Caching**
   - Cache compiled scripts (not just source)
   - Avoid re-parsing React bundles on every page load

2. **Lazy Loading**
   - Don't load React bundles until needed
   - Code splitting support

3. **Worker Threads**
   - Move JS execution to separate thread
   - Communicate via channels
   - Prevent long-running scripts from blocking GUI

4. **Memory Management**
   - Limit runtime memory usage
   - GC tuning for QuickJS
   - Clean up old runtimes on navigation

### Estimated Effort
~20-30 hours total

---

## Timeline Recommendation

### Sprint 1 (Current)
- [x] Option 2 implementation (quick win)
- [ ] Document architecture decisions
- [ ] Create React 17 sync rendering examples

### Sprint 2 (Next)
- [ ] Phase 1: Navigation-integrated script fetching
- [ ] Phase 2: Runtime-DOM patch integration
- [ ] Test: Basic React app with click handlers

### Sprint 3
- [ ] Phase 3: Event handling integration
- [ ] Test: Interactive React counter works in GUI
- [ ] Performance profiling

### Sprint 4+
- [ ] Phase 4: Concurrent rendering (if needed)
- [ ] Phase 5: Performance optimization
- [ ] Advanced features (workers, modules, etc.)

---

## Testing Strategy

### Unit Tests
- ✅ Already have: `tests/react_umd_e2e_test.rs`
- Add: Patch application tests
- Add: Event dispatch tests

### Integration Tests
- React Counter (simple state)
- React TodoMVC (complex state, lists)
- React Router (navigation)

### Performance Tests
- Script load time benchmarks
- DOM update benchmarks
- Memory usage monitoring

---

## Alternative Approaches to Consider

### A. Use WebView
**Pros:** Everything works out of the box
**Cons:** Defeats purpose of custom browser, huge dependency

### B. Use Servo
**Pros:** Real browser engine, full web platform
**Cons:** Massive dependency, Rust but not Blitz

### C. QuickJS → JSC/V8
**Pros:** Better performance, more compatibility
**Cons:** Larger binaries, more complex FFI

**Recommendation:** Stick with QuickJS for now. It's working well and keeps the project lean.

---

## Success Metrics

### Phase 1 Complete
- [ ] React Counter works in GUI
- [ ] No GUI blocking during script load
- [ ] Scripts execute within 100ms of HTML load

### Phase 2 Complete
- [ ] Click handlers work
- [ ] DOM updates visible in GUI
- [ ] No memory leaks after 100 navigations

### Phase 3 Complete
- [ ] setTimeout/setInterval work
- [ ] React state updates propagate
- [ ] 60fps rendering maintained

### Phase 4 Complete
- [ ] React 18 createRoot works
- [ ] Concurrent features work
- [ ] Complex SPAs functional

---

## Open Questions

1. **How to handle infinite loops / long-running scripts?**
   - Timeout mechanism?
   - Separate process?
   - User confirmation?

2. **Cross-origin script security?**
   - CSP implementation?
   - Sandbox per origin?

3. **Module support (ESM)?**
   - Import maps?
   - Dynamic imports?

4. **Developer tools?**
   - Console integration?
   - Debugger support?
   - React DevTools?

---

## Related Work

- **Blitz DOM**: Already has event system, needs JS integration
- **Blitz Shell**: Event loop, needs async task support
- **QuickJS Bridge**: Working well, minimal changes needed
- **Script Fetcher**: Can be extracted to navigation layer

---

## Non-Goals (For Now)

- ❌ WebAssembly support
- ❌ Service Workers
- ❌ Web Workers (maybe later)
- ❌ Full Web Platform compatibility
- ❌ Chrome extension APIs

Focus: **Core React/SPA functionality first**
