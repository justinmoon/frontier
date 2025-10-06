# React + QuickJS Compatibility Research Task

## Background

We're building a custom web browser (Frontier) using QuickJS as the JavaScript engine. We're attempting to run React UMD bundles but encountering failures. We need to determine: **Is this a fundamental limitation of QuickJS, or can it be fixed?**

## UPDATE: Polyfill Progress

**BREAKTHROUGH**: Adding `var self = globalThis;` fixes the "self is not defined" error!

After adding this polyfill:
```javascript
if (typeof self === 'undefined') { var self = globalThis; }
```

Production builds now load successfully! But we hit a NEW error:

```
❌ Script 2 execution failed:
   File: inline-2.js
   Error: Error: invalid 'instanceof' right operand
Stack:     at xg (eval_script:53:489)
           at Qj (eval_script:171:405)
           at unstable_runWithPriority (eval_script:24:26)
```

**Status**: React libraries load, but `ReactDOM.render()` fails with `instanceof` issue.

## What We've Discovered

### Test Setup
- JavaScript Engine: QuickJS (via rquickjs Rust bindings)
- React Version: React 17.x UMD bundles
- Environment: Custom browser with DOM bridge to Rust/Blitz

### Exact Failures

#### 1. React Production Builds (`react.production.min.js`)
```
❌ Script 0 execution failed:
   File: react.production.min.js
   Error: Error: self is not defined
Stack:     at <anonymous> (eval_script:9:146)
           at <anonymous> (eval_script:9:196)
           at <eval> (eval_script:31:1)

❌ Script 1 execution failed:
   File: react-dom.production.min.js
   Error: Error: self is not defined
Stack:     at <anonymous> (eval_script:12:153)
           at <anonymous> (eval_script:12:224)
           at <eval> (eval_script:245:1)
```

**Result**: React libraries completely fail to load. 0 scripts execute successfully.

#### 2. React Development Builds (`react.development.js`)
```
✓ Script 0 (react.development.js) - LOADS SUCCESSFULLY
✓ Script 1 (react-dom.development.js) - LOADS SUCCESSFULLY

❌ Script 2 execution failed:
   File: inline-2.js (calling ReactDOM.render())
   Error: Error: not a function
Stack:     at apply (native)
           at call (native)
           at printWarning (eval_script:73:37)
           at error (eval_script:47:9)
           at render (eval_script:29726:193)
           at <eval> (eval_script:11:54)
```

The inline script that fails:
```javascript
var h = React.createElement;

function SimpleComponent() {
    return h('div', null,
        h('h1', null, 'Hello from React'),
        h('button', { id: 'test-btn' }, 'Click Me')
    );
}

ReactDOM.render(h(SimpleComponent), document.getElementById('root'));
```

**Result**: React libraries load, but `ReactDOM.render()` fails deep inside React's code with "not a function" error at native `apply`/`call`.

### What Works

Simple inline JavaScript works perfectly:
```javascript
document.getElementById('root').setAttribute('data-test', 'success');
```
✓ 100% success rate, all DOM APIs functional

## Specific Questions

1. **✅ SOLVED: `self` missing from QuickJS**
   - Fixed with: `var self = globalThis;`
   - React production builds now load!

2. **NEW: Why does `instanceof` fail?**
   - Error: `invalid 'instanceof' right operand`
   - Occurs when calling `ReactDOM.render()`
   - Is this a QuickJS bug or intentional limitation?
   - What constitutes a "valid" instanceof operand in QuickJS?
   - Can we polyfill or work around this?

3. **Development Builds: Why does `apply (native)` fail?**
   - Error occurs at `apply (native)` and `call (native)`
   - Are `Function.prototype.apply/call` implemented in QuickJS?
   - Could this be a `this` binding issue?
   - Different error than production builds - why?

4. **ES2015+ Feature Support**
   - React UMD is transpiled for browsers (ES5 compatible)
   - Does QuickJS fully implement ES5 `Function.prototype` methods?
   - Does it fully implement `instanceof`?
   - Are there known incompatibilities with popular transpiled libraries?

5. **Has anyone successfully run React in QuickJS?**
   - Search for: "QuickJS React", "QuickJS React UMD", "QuickJS React compatibility"
   - Search for: "QuickJS instanceof error"
   - Are there known workarounds or patches?
   - Would Preact or other lighter alternatives work better?

## What We Need

**Primary Goal**: Determine if React + QuickJS is fundamentally broken or fixable with polyfills/patches.

**Deliverable**: Clear answer to:
- "Can React run in QuickJS?" (Yes/No/Maybe)
- If YES: What specific changes/polyfills are needed?
- If NO: Why not? (Missing APIs, incompatible implementations, etc.)
- If MAYBE: What would need to be investigated further?

## Resources

- QuickJS repo: `~/code/quickjs`
- QuickJS documentation: `~/code/quickjs/doc/quickjs.texi`
- Test262 failures: `~/code/quickjs/test262_errors.txt`
- Our test: `/Users/justin/code/frontier/worktrees/dom-api-milestones-claude/tests/react_gui_integration_test.rs`
- React UMD bundles: `/Users/justin/code/frontier/worktrees/dom-api-milestones-claude/assets/react*.js`

## Investigation Approach

1. Check QuickJS documentation for global object model (`self`, `window`, `globalThis`)
2. Verify `Function.prototype.apply/call` implementation in QuickJS
3. Search for existing React + QuickJS projects or discussions
4. Test simple polyfills (e.g., `var self = globalThis;`)
5. If needed: examine React UMD bundle code to see what exactly it's checking for

## Context

We chose QuickJS for:
- Lightweight (vs V8/JSC)
- Good Rust bindings (rquickjs)
- ES2020 support claimed

But we're open to switching engines if React compatibility is fundamentally broken.

**Time constraint**: Need answer soon - this blocks React integration work.
