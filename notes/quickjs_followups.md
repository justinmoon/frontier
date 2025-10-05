# QuickJS Follow-Ups

- Cache QuickJS runtime per navigation to avoid rebuilding the snapshot for every inline script evaluation (current implementation parses HTML twice).
- Extend DOM bridge with more operations (attribute removal, append/remove children) and expose event listener stubs.
- Support `<script type="module">` and `<script type="text/typescript">` once the TypeScript transpilation pathway is ready.
- Surface JavaScript exceptions to the UI so authors can debug failing scripts without checking terminal logs.
