# window.nostr + NostrdB Integration Plan

## Objectives
- Expose a first-class `window.nostr` namespace that wraps NostrdB timelines, note access, relay orchestration, and profile metadata so web apps can treat it as their primary data API.
- Add the latest `nostrdb-rs` (and any dependent crates) as Cargo dependencies while keeping the Nix build working across platforms.
- Provide a minimal vanilla-JS demo page that renders a nostr timeline for a user-specified npub using only `window.nostr`; the demo should fetch the npub’s contact list (kind 3), backfill followed authors, and render profile info while remote subscriptions hydrate the cache.
- Keep `just ci` and `nix flake check` green; add coverage/tests for the new API surface.

## Phase 0 – Discovery & Preconditions
- [ ] Audit current JS bridge (`src/js`) to map how globals are injected; note how `window` is materialized inside QuickJS (`runtime_document.rs`, `bridge.rs`).
- [ ] Review existing nostr client pieces (e.g. `src/net`, `src/blossom`, any nostr-related modules) to understand available relay connections and signing flows.
- [ ] Read `~/code/nostrdb-rs` docs + `shell.nix`; note required environment variables, native dependencies, target features.
- [ ] Inspect `~/code/nostr-zig` flake examples to see how nostrdb/ldmb is compiled there; capture any linker flags or pkg-config expectations we must mirror.

## Phase 1 – Bring in Latest NostrdB
- [ ] Add `nostrdb = { git = "https://github.com/..." , rev = "..." }` (matching `~/code/nostrdb-rs` HEAD) to `Cargo.toml`; no vendoring.
- [ ] Update dependent crates in `Cargo.toml` if upstream NostrdB requires additional workspace members.
- [ ] Run `cargo update -p nostrdb` to ensure lockfile coherence.
- [ ] Extend `flake.nix` build inputs with any libraries NostrdB needs (e.g. `lmdb`, `secp256k1`, `openssl`, `libiconv`) and native tooling (`rustPlatform.bindgenHook`, `cmake`, `gnumake`, `pkg-config`, `LIBCLANG_PATH`) mirroring `~/code/nostrdb-rs/shell.nix` and the `nostr-zig` flakes that already compile nostrdb/ldmb.
- [ ] Regenerate `flake.lock` if new inputs (e.g. `lmdb`) require overriding.
- [ ] Build via `nix build` and `cargo build` to validate linking across macOS/linux; capture any necessary `RUSTFLAGS` or environment toggles.

## Phase 2 – Bind NostrdB Inside Frontier Core
- [ ] Add a Rust module (e.g. `src/net/nostrdb_bridge.rs`) that wraps core NostrdB operations: opening the DB, managing transactions, building filters, streaming results, and resolving contact lists into author filters.
- [ ] Introduce an async task runner / channel that can service JS requests without blocking the reactor; leverage existing executor architecture (check `src/js/runtime.rs`).
- [ ] Define a typed `NostrDbService` struct accessible via the global application state (likely stored in `AppContext` or equivalent) responsible for:
  - managing the DB directory & map size configuration
  - connecting to configured relays and subscribing via NostrdB
  - providing query APIs for timelines, profiles, contact lists, and single-note lookups
- [ ] Ensure we reuse or supersede any current nostr timeline logic so there’s a single ingestion path (avoid double-fetching).

## Phase 3 – Design the window.nostr Surface
- [ ] Draft an API contract doc (Rust doc comment + JS stub) for:
  - `window.nostr.open(opts)` → returns a promise once DB + relays ready (idempotent)
  - `window.nostr.timeline({ authors, kinds, since, limit })` → async iterator / event emitter delivering hydrated notes; automatically expand `authors` by fetching the account’s latest contact list when requested
  - `window.nostr.getNote(id)` and `window.nostr.getProfile(pubkey)` → promise-based lookups
  - `window.nostr.getContactList(pubkey)` → promise returning the latest kind-3 info and derived follow set
  - `window.nostr.subscribe(filter)` → returns handle with `.on('event', cb)` + `.close()`
  - `window.nostr.setRelays([...])` + `.status()` for diagnostics
  - A `version` string and feature detection hooks
- [ ] Map each JS surface call to a Rust command: extend the QuickJS bridge to marshal arguments/results (see `src/js/bridge.rs` for pattern).
- [ ] Implement serialization helpers (e.g. convert NostrdB note/profile/contact structs into plain JS objects with consistent shape).
- [ ] Handle resource lifetimes: store subscriptions in a Rust side registry keyed by JS handles so we can tear them down when GC’d or explicitly closed.
- [ ] Add error mapping so Rust errors become JS exceptions with actionable messages.

## Phase 4 – Prototype Vanilla JS Timeline Demo
- [ ] Create a demo HTML/JS asset (e.g. `assets/demos/nostr-timeline/index.html`) that:
  - renders an `<input>` for npub, a “Load timeline” button, and a feed container
  - calls `window.nostr.setRelays(defaultRelays)` on load
  - on submit, fetches the npub’s contact list, derives the follow set, then calls `window.nostr.timeline({ authors: follows, kinds: [1], limit: 100 })`
  - displays profile metadata (name, picture) for each note author and appends notes in reverse chronological order
- [ ] Ensure the demo only touches `window.nostr`; all networking/storage must happen in Rust.
- [ ] Wire demo into existing router/launcher so it can be opened from Frontier (maybe under a `just demo-nostr` command).

## Phase 5 – Testing & Tooling
- [ ] Add Rust integration tests for the new service layer (e.g. spin up an in-memory NostrdB with fixture events; assert timeline filters return expected notes).
- [ ] Add JS-side smoke tests if we have headless automation (maybe reuse `blitz` UI harness); otherwise add manual QA checklist in `tests/README`.
- [ ] Extend CI (`just ci`) to run any new tests/linters; ensure `nix flake check` still passes.
- [ ] Provide logging hooks so developers can inspect relay traffic and NostrdB queries when debugging (`RUST_LOG=frontier::nostr=debug`).

## Phase 6 – Documentation & Developer Guidance
- [ ] Document the `window.nostr` API in `docs/window-nostr.md` with examples, lifecycle notes, and limitations (no signing yet).
- [ ] Update `README.md` (or a dedicated NostrdB section) with setup instructions (DB directory, relay defaults).
- [ ] Add inline Rust docs and TypeScript declaration file (`assets/types/window-nostr.d.ts`) to help app authors.
- [ ] Note follow-up work: signing, write operations, permission model.

> Deliverable: fully working `window.nostr` runtime backed by NostrdB, plus a minimal timeline demo proving the API end-to-end.
