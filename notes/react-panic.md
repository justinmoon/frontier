# React Timer Panic Status (2025-10-11)

## Update (2025-10-12)
- Added a bridge-side fix that seeds Stylo’s `primary` styles for every element created through the QuickJS DOM bridge so `document.resolve()` no longer panics on the timer subtree.
- Captured the exact React patch stream and codified it as a unit test (`react_timer_patch_sequence_resolves`) which now passes, confirming the style seed keeps Stylo happy in a controlled environment.
- Running the full demo now advances past the original panic but aborts later with `BaseDocument::get_node` misaligned pointer errors while normalising the event chain (`BlitzJsBridge::node_type`). That crash is new and needs separate investigation.
- TODO: dig into why event normalisation is requesting stale node handles (or why the slab bookkeeping is corrupt) after the timer DOM mounts.

## Reproduction & Symptoms
- Launching `just react-demos` (or running `cargo run --bin frontier file://…/assets/react-demos/timer.html`) still crashes Frontier, but now after the timer subtree mounts instead of during style resolution.
- Console shows a panic in `blitz_dom::document::BaseDocument::get_node` complaining about a misaligned pointer while `BlitzJsBridge::node_type` normalises an event propagation chain.
- Prior root cause (`ElementStyles::primary` unwrap) is no longer observed once the style seeding fix is in place.

## Observations
- Bridged DOM mutations now reach resolve without tripping Stylo; the failing path happens during subsequent event dispatch when the runtime asks for node types along the propagation chain (handles `1`, `2`, `14`, `15`, …).
- The misaligned pointer suggests either slab metadata corruption or lookups on nodes that were removed/replaced; worth verifying whether the event chain contains handles for nodes that React dropped when rebuilding the timer UI.
- The React patch stream (captured via updated logging) is deterministic and encoded in the new regression test, giving us a repeatable baseline without JS.

## What We Tried
1. **Chrome Injection Rewrite** – Replaced string-based wrapping with DOM mutation so rendered DOM and runtime HTML stay in sync.
2. **Logging** – Added verbose logging around DOM bridge attachment and `DomState::apply_patch` to verify the sequence of operations.
3. **Style Seeding in Blitz** – Ensured nodes created via `DocumentMutator::create_element` receive the root’s initial style (`ComputedValues::initial_values…`). No effect.
4. **Stylo Fallback Hack** – Patched local `style::ElementStyles::primary()` to supply a default style instead of panicking. Crash persists, suggesting other parts expect real computed values before layout.
5. **Resetting Stylo State in Blitz** – Injected style seeding before `BaseDocument::resolve()` to populate missing data. Still hits the same panic.

## Current Hypothesis
- The initial Stylo panic is fixed by seeding `primary` styles when JS creates or inserts DOM nodes; the remaining abort appears to stem from event normalisation walking handles that no longer map to live nodes (or whose Stylo data was cleared during React’s wholesale subtree swap).
- Because the slab panics point at `node_id` values like `1` (the root `<html>`), it’s likely we are either clearing and recreating structural nodes during `set_text_content`/`append_child` or the event chain provided by QuickJS contains stale handles captured before the swap.

## Suggested Next Step
- Trace why the QuickJS event path ends up asking for invalid handles after the timer DOM is mounted (potentially by instrumenting `JsDomEnvironment::normalize_chain` and cross-checking against the DOM tree right before the crash).
- Audit slab/node lifetimes during `set_text_content` + `append_child` (React clears `#root` before appending the rebuilt subtree) to ensure we are not accidentally invalidating document-level nodes like `<html>` (handle `1`).
- With the pure DOM test in place, add another harness that simulates the event chain using the same handles to see if we can reproduce the slab failure in isolation.

## Files & Patches Touched
- **Frontier**
  - `src/js/bridge.rs` now seeds Stylo metadata whenever JS creates, inserts, or clones nodes to keep computed styles present ahead of layout/traversal.
  - `tests/react_timer_patch_test.rs` exercises the captured React patch stream against the DOM bridge to guard against regressions in style initialisation.
- **Notes**
  - This file tracks both the historical context and the new misaligned-pointer failure to unblock whoever chases the next bug.

## Requested Follow-up
- Instrument the QuickJS event chain to log which handles show up after React rerenders, then compare with the live DOM just before the misaligned pointer abort.
- Double-check whether `set_text_content` on the `#root` container is blowing away DOM nodes that `normalize_chain` still references, and if so update the chain before dispatching events.
- Consider replaying the captured event chain in a unit test (similar to the DOM patch replay) once we understand the failing handles.
