# NNS Removal (2025-03-17)

- Migrated all NNS, Blossom, storage, and TLS code into the standalone `~/code/nns-claude` repo (includes CI + flake).
- Frontier now only handles direct URL/IP navigation; no resolver selection UI remains.
- CI passes without nostr/sqlite deps; `Cargo.toml` trimmed accordingly.
- Follow-ups:
  - Reintroduce decentralized name resolution once the demo stabilises in the external repo.
  - Audit remaining docs/tests for lingering references to NNS concepts.
