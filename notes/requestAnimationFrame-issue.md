# requestAnimationFrame Implementation Issue

## Summary
Attempted to add requestAnimationFrame support to Milestone 3 but encountered JavaScript evaluation errors in QuickJS when trying to pass timestamps to callbacks.

## What Works
- setTimeout/setInterval/clearTimeout/clearInterval (fully working)
- queueMicrotask (fully working, committed in 9b1d589)
- performance.now() binding in Rust (functional)

## What Doesn't Work
- requestAnimationFrame with timestamp parameter
- The JavaScript bootstrap code fails to evaluate when trying to:
  1. Check `callback.__isAnimationFrame` flag in timer fire handler
  2. Call `global.__frontier_performance_now()` to get timestamp
  3. Pass timestamp as argument to rAF callbacks

## Technical Details
- Added `start_time: Rc<Instant>` to `JsDomEnvironment`
- Created `__frontier_performance_now()` binding that returns elapsed milliseconds
- Split TIMER_BOOTSTRAP into PART1 and PART2 to extend timer handling
- QuickJS throws generic "Exception" error during eval without specific details

## Attempted Solutions
1. Split bootstrap into two parts to isolate rAF code
2. Simplified to just wrap setTimeout with 16ms delay
3. Tried exposing timerCallbacks Map to global scope
4. All approaches failed with same generic exception

## Next Steps
- Simplest working approach: Make rAF a thin wrapper around setTimeout(callback, 16)
- For full spec compliance, need to debug why JavaScript evaluation fails
- Consider using a different approach to pass timestamp (maybe via global variable?)
- React might work without timestamp parameter since it's optional in the spec

## Priority
- Low: React primarily needs timers and microtasks (both working)
- rAF is used for animations and scheduling but not critical for basic functionality
- Can be addressed in follow-up PR after event listeners are implemented
