# Phase 3: Eliminate DNS

## Goal

Remove DNS dependency. All services publish IP addresses and TLS public keys via Nostr. Everything (websites, Blossom servers, relays) uses the same discovery mechanism.

## Current State

**Phase 1 & 2 done:**
- Direct IP sites work (no DNS)
- Blossom sites work (but still use DNS for server discovery: `https://blossom.primal.net`)

**Remaining DNS usage:**
- Blossom server URLs: `["server", "https://blossom.primal.net"]`
- Relay URLs: `wss://relay.damus.io`

## Solution

Embed IP address and TLS verification info directly in kind 34256 events.

## Event Format

**One event kind for everything: 34256**

```json
{
  "kind": 34256,
  "pubkey": "service-identity-key",
  "tags": [
    ["d", "identifier"],
    ["ip", "1.2.3.4:8443"],
    ["tls-pubkey", "deadbeef..."],
    ["tls-alg", "ed25519"],
    ["blossom", "root-hash"]  // Optional: for content-addressed sites
  ]
}
```

**The pubkey IS the service identity.** Reputation attaches to this key.

## Examples

### Direct IP Site (Rails/Django)

```json
{
  "kind": 34256,
  "pubkey": "abc123...",
  "tags": [
    ["d", "myrailsapp"],
    ["ip", "1.2.3.4:8443"],
    ["tls-pubkey", "deadbeef..."],
    ["tls-alg", "ed25519"]
  ]
}
```

User types `myrailsapp` → browser connects to `https://1.2.3.4:8443` with TLS verification.

### Blossom Site

```json
{
  "kind": 34256,
  "pubkey": "publisher...",
  "tags": [
    ["d", "mysite"],
    ["blossom", "root-hash..."],
    ["ip", "5.6.7.8:8443"],
    ["tls-pubkey", "cafe..."],
    ["tls-alg", "ed25519"]
  ]
}
```

Plus kind 34128 manifest events (existing nsite standard):

```json
{
  "kind": 34128,
  "pubkey": "publisher...",
  "tags": [
    ["d", "/index.html"],
    ["sha256", "file-hash..."]
  ]
}
```

### Relay

```json
{
  "kind": 34256,
  "pubkey": "relay-operator...",
  "tags": [
    ["d", "damus-relay"],
    ["ip", "9.10.11.12:7777"],
    ["tls-pubkey", "1234..."],
    ["tls-alg", "ed25519"]
  ]
}
```

Relay config becomes pubkey-based:

```yaml
relays:
  - npub1abc...  # Damus
  - npub1def...  # nos.lol
```

## TLS Implementation

### Server Setup

```bash
# 1. Generate keypair
openssl genpkey -algorithm ED25519 -out server.key

# 2. Create self-signed cert
openssl req -new -x509 -key server.key -out server.crt -days 3650 \
  -subj "/CN=myserver"

# 3. Extract pubkey hex
openssl pkey -in server.key -pubout -outform DER | tail -c 32 | xxd -p -c 32

# 4. Publish to Nostr
nak event --kind 34256 \
  -d mysite \
  --tag "ip=1.2.3.4:8443" \
  --tag "tls-pubkey=HEX_FROM_STEP_3" \
  --tag "tls-alg=ed25519" \
  --sec YOUR_NSEC \
  wss://relay.damus.io

# 5. Configure server to use cert
# nginx/caddy/etc with server.crt + server.key
```

### Browser Implementation (rustls)

```rust
use rustls::{ClientConfig, ServerCertVerifier};

struct NostrCertVerifier {
    expected_pubkey: Vec<u8>,
}

impl ServerCertVerifier for NostrCertVerifier {
    fn verify_server_cert(&self, cert: &Certificate, ...) -> Result<...> {
        let cert_pubkey = extract_pubkey_from_cert(cert)?;
        if cert_pubkey == self.expected_pubkey {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TLSError::General("Pubkey mismatch"))
        }
    }
}

// Connect
let config = ClientConfig::builder()
    .with_custom_certificate_verifier(Arc::new(NostrCertVerifier {
        expected_pubkey: hex::decode(pubkey_from_nostr)?
    }))
    .with_no_client_auth();

let client = reqwest::Client::builder()
    .use_preconfigured_tls(config)
    .build()?;

client.get(format!("https://{}", ip)).send().await?
```

## Demo Checklist

### Local Testing

- [ ] Generate test TLS cert + key
- [ ] Start local HTTP server with TLS (port 8443)
- [ ] Publish kind 34256 event with localhost:8443
- [ ] Browser connects and verifies cert pubkey
- [ ] Start local Blossom server with TLS
- [ ] Publish kind 34256 + kind 34128 events
- [ ] Browser fetches blobs with TLS verification
- [ ] Start `nak serve` with TLS (or simple relay)
- [ ] Browser connects to relay via WebSocket with TLS

### Hetzner Deployment

Deploy to ~/configs/hetzner:

- [ ] HTTP server with TLS (Rails/static site)
- [ ] Blossom server with TLS
- [ ] Nostr relay with TLS

All publish kind 34256 events with their real IPs.

## Browser Changes

### Navigation Flow

```
User enters "mysite"
    ↓
Query kind 34256 with d="mysite"
    ↓
Extract: ip, tls-pubkey, blossom (if present)
    ↓
If blossom tag:
    Query kind 34128 from same pubkey
    Build path→hash manifest
    Look up /index.html
    ↓
Connect to https://<ip>
Verify TLS cert pubkey matches event
    ↓
Fetch content
```

### Code Changes

```rust
// src/nns/models.rs
pub struct NnsClaim {
    pub name: String,
    pub ip: SocketAddr,
    pub tls_pubkey: Option<String>,
    pub blossom_hash: Option<String>,
    pub pubkey: String,
    // ...
}

// src/navigation.rs
async fn connect_with_tls_verification(
    ip: SocketAddr,
    expected_pubkey: &str
) -> Result<Response> {
    let verifier = NostrCertVerifier {
        expected_pubkey: hex::decode(expected_pubkey)?
    };
    let config = build_tls_config(verifier);
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(config)
        .build()?;

    client.get(format!("https://{}", ip)).send().await
}
```

## Relay Config Migration

**Old (DNS-based):**
```yaml
relays:
  - wss://relay.damus.io
  - wss://nos.lol
```

**New (pubkey-based):**
```yaml
relays:
  - npub: npub1abc...
    name: "Damus Relay"
  - npub: npub1def...
    name: "nos.lol"
```

Browser queries kind 34256 from each npub to get IP + TLS info.

## Legacy Fallback (Optional)

For servers that want to support both Frontier and legacy browsers:

```json
{
  "tags": [
    ["d", "mysite"],
    ["ip", "1.2.3.4:8443"],
    ["tls-pubkey", "..."],
    ["url", "https://mysite.com"]  // Legacy DNS fallback
  ]
}
```

Browser tries `ip` first, falls back to `url` if connection fails.

## Security

**Eliminated:**
- ❌ DNS spoofing
- ❌ CA compromise
- ❌ Domain hijacking

**Protected:**
- ✅ MITM (TLS cert verified via Nostr)
- ✅ Content tampering (SHA-256 for Blossom)
- ✅ Encrypted connections (TLS 1.3)

**Remaining trust:**
- Nostr relays (for event distribution)
- User's choice (when multiple claims exist)

## Implementation Plan

1. **Week 1:** TLS verification with rustls
   - Custom `ServerCertVerifier`
   - Extract pubkey from certs
   - Match against Nostr-published key

2. **Week 2:** Update event parsing
   - Parse `ip` and `tls-pubkey` tags from kind 34256
   - Update navigation to use IP instead of URL
   - Test with self-signed certs locally

3. **Week 3:** Relay support
   - Update relay config format
   - Query kind 34256 for relay connection info
   - WebSocket with TLS verification

4. **Week 4:** Deploy demo
   - Set up Hetzner services
   - Publish events
   - Test end-to-end

## Success Criteria

- [ ] Browser extracts pubkey from TLS certificates
- [ ] Browser verifies cert pubkey matches Nostr event
- [ ] Direct IP sites work with TLS
- [ ] Blossom sites work with TLS
- [ ] Relays work with TLS
- [ ] MITM attacks detected and rejected
- [ ] No DNS lookups anywhere in the stack

## The Win

**Zero DNS. Zero CAs. Fully decentralized web.**

Services publish their own IPs and TLS keys via Nostr. Browser verifies cryptographically. The network effect of Nostr replaces centralized infrastructure.
