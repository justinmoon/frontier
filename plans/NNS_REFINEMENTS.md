# NNS Refinements & Future Work

Post-NO_DNS merge improvements for the Nostr Name System.

## Current State (What's Done)

✅ **Phase 3 Complete: DNS Eliminated**
- Direct IP sites with TLS verification
- Blossom sites with TLS verification
- Custom TLS certificate verifier (`NostrCertVerifier`)
- Comprehensive E2E test covering all service types
- Events use kind 34256 with `ip`, `tls-pubkey`, `tls-alg` tags
- MITM protection via cryptographic verification

## Priority 0: Align With Codex Architecture Spec

Codex produced a richer blueprint we want to fold into the merged branch. Treat these as blocking tasks before new feature work.

### Roadmap & Documentation
- Replace `plans/NO_DNS.md` with the Codex-style structure (objectives, requirements, architecture, rollout, risks, tests, success criteria) while keeping content consistent with the merged implementation.

### Event & Data Model
- Extend `NnsClaim` with `ServiceKind`, rich `ClaimLocation` variants, and `ServiceEndpoint { transport, socket_addr, priority }`.
- Parse additional tags: `svc`, multiple `endpoint` entries, blossom root, `note`, and TLS material (`tls-pubkey`, `tls-alg`).
- Introduce `PublishedTlsKey` (algorithm + raw SPKI bytes) and validate during parse.
- Ensure navigation/blossom code consumes the richer structures and preserves TLS data.

### Storage Layer
- Expand `ClaimRecord` schema to store `location`, JSON `endpoints`, `svc_kind`, `tls_pubkey`, `tls_alg`, `blossom_root`, `note`.
- Update migrations (`ensure_column`) and serializers so cache reads restore full `NnsClaim` instances including TLS info.

### Resolver
- Adopt Codex’s cache TTL logic, selection persistence, and claim scoring helper (`score_claim`).
- Rank claims using selection + freshness and propagate full `ClaimLocation`/TLS info into fetch planning.
- Keep logging on parse failures but continue processing other events.

### TLS Module
- Replace ad-hoc verifier with Codex-style `src/tls/mod.rs` that:
  - Exposes `SecureHttpClient` and websocket connector builders using rustls 0.23 APIs.
  - Implements a proper `ServerCertVerifier` (Debug, TLS 1.2/1.3 hooks, SPKI comparison across algorithms).
  - Updates `Cargo.toml` to disable reqwest native-tls, enable `rustls-tls`, and add `rustls`, `tokio-rustls`, `x509-parser`, `hex`.

### Navigation & Blossom Flow
- Move to `FetchSource::{LegacyUrl, SecureHttp, Blossom}` using `SecureHttpClient` for HTTPS endpoints with iterative failover.
- Keep `BlossomDocumentContext` carrying TLS key/endpoints so in-document navigation reuses verification.
- Normalize HTTP/Blossom paths and selection prompts per Codex implementation.

### Blossom Fetcher
- Integrate TLS-aware blob fetching (reuse `SecureHttpClient`), verify blob hashes, and cache successful responses.
- Propagate TLS errors while continuing to retry other endpoints.

### Tests
- Port Codex scoring/storage unit tests and reconcile existing suites with the richer data model.
- Ensure `cargo test --test no_dns_e2e_test -- --nocapture` passes after TLS adjustments.
- Keep `just ci` green; add deterministic fixtures only.

### Cleanup
- Remove placeholder comments (e.g., “Tests removed”).
- Export new modules via `src/lib.rs` and ensure no dead code remains.

## Priority 1: Production Readiness

### WebSocket TLS for Relay Connections

**Status**: TLS info parsed, but WebSocket connector still plain.

**Implementation**:
```rust
use tokio_tungstenite::Connector;

let verifier = build_published_tls_verifier(relay_tls_key);
let tls_config = build_tls_config(verifier);
let connector = Connector::Rustls(Arc::new(tls_config));

let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(
    format!("wss://{}", relay_addr),
    None,
    false,
    Some(connector),
).await?;
```

**Testing**:
- Extend `tests/no_dns_e2e_test.rs` to establish the TLS WebSocket, exchange a message, and assert mismatched keys reject.

### Relay Configuration Migration

**Current**
```yaml
relays:
  - wss://relay.damus.io
  - wss://nos.lol
```

**Target**
```yaml
relays:
  - npub: npub1abc...
    name: "Damus Relay"
    bootstrap_url: wss://relay.damus.io  # optional fallback
  - npub: npub1def...
    name: "nos.lol"
```

**Bootstrap Strategy**
1. Use `bootstrap_url` for the initial connection.
2. Query relay npub for kind 34256 endpoints + TLS key.
3. Cache IP/TLS locally and prefer those for subsequent sessions.
4. Periodically refresh via relays; drop stale endpoints after TTL.

### Production Deployment Guide
- Create `docs/DEPLOY_NO_DNS.md` covering key generation, self-signed cert creation, SPKI extraction, event publishing, reverse-proxy config, rotation/monitoring, and automation examples.

## Priority 2: Multi-Endpoint Support

Aligned with Codex’s endpoint architecture.

### Event Format Enhancement
```json
["endpoint", "https", "1.2.3.4:8443", "0"]
["endpoint", "https", "5.6.7.8:8443", "10"]
["endpoint", "wss", "9.10.11.12:7777", "0"]
```

### Model & Strategy
- `TransportKind::{Https, Wss}`.
- `ServiceEndpoint { transport, socket_addr, priority }` stored in each claim.
- Connection policy: sort by priority, attempt sequentially with ~2s timeout, mark failures for 5 minutes, cache successes per session.

## Priority 3: Enhanced Security

### Certificate Pinning & Rotation
- Support `tls-pubkey`, `tls-pubkey-next`, `tls-pubkey-prev`; accept all, warn when using `prev`, cache rotation schedule.

### TLS Algorithm Expansion
- Add `secp256k1`, `rsa-pss`, `ecdsa-p256` (and keep `ed25519`).
- Verifier chooses extraction/verification logic based on algorithm tag.

### Service Kind Tag
```json
["svc", "site"], ["svc", "blossom-site"], ["svc", "blossom-server"], ["svc", "relay"], ["svc", "api"]
```
- Tailor UX/error handling based on service type.

## Priority 4: Performance & UX

### Endpoint Health Monitoring
- Track endpoint success/fail counts; expose diagnostics.
- UI surfacing for active endpoint, TLS key fingerprint, relay selection history.

### Offline & Cache Strategy
- Respect TTLs set in resolver; allow manual refresh; log when cached TLS data is stale.

---

This merged plan keeps the existing roadmap while surfacing the Codex spec as “Priority 0” work so we converge on the richer architecture before layering new features.
