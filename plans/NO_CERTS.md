# Phase 3: Encryption Without Certificate Authorities

## Goal
Add TLS encryption to connections without depending on certificate authorities. Verify server identity via Nostr-published keys instead of CA-signed certificates.

## Prerequisite
Phase 2 (BLOSSOM.md) working - we have content-addressed sites served from blossom.

## The Problem with Traditional TLS
1. Browser connects to server
2. Server presents certificate signed by CA
3. Browser verifies CA signature via system trust store
4. **Central authority** (CA) can be compromised, coerced, or gate-keep

## Our Solution: Self-Signed Certs + Nostr Verification

### Server Side
Server generates a keypair and publishes the public key via Nostr:

```json
{
  "kind": 10063,  // Reuse blossom server list kind, or define new kind
  "pubkey": "<server operator's nostr key>",
  "tags": [
    ["server", "https://blossom.example.com"],
    ["tls-pubkey", "<hex-encoded-public-key>"],    // The server's TLS key
    ["tls-alg", "ed25519"]                         // Or "secp256k1" if we want that
  ],
  "content": "",
  "sig": "..."
}
```

Server creates **self-signed certificate** using this keypair:
```bash
# Generate ed25519 key
openssl genpkey -algorithm ED25519 -out server.key

# Create self-signed cert with that key
openssl req -new -x509 -key server.key -out server.crt -days 3650 \
  -subj "/CN=blossom.example.com"

# Serve HTTPS with this cert
```

### Browser Side

When connecting to a server discovered via NNS:

```rust
async fn fetch_with_verification(
    url: &str,
    expected_pubkey: &str  // From Nostr event
) -> Result<Response> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)  // Skip CA verification
        .build()?;

    let response = client.get(url).send().await?;

    // Get the cert from the connection
    let cert = response.connection_info().peer_certificate()?;

    // Extract public key from cert
    let cert_pubkey = extract_pubkey_from_cert(&cert)?;

    // Verify it matches what was published on Nostr
    if cert_pubkey != expected_pubkey {
        return Err("Certificate public key doesn't match Nostr announcement");
    }

    Ok(response)
}
```

## Updated NNS Event Format

NNS claims reference server by pubkey:
```json
// NNS claim (what users publish)
{
  "kind": 34256,
  "tags": [
    ["d", "justinmoon"],
    ["blossom", "abc123..."],
    ["server-pubkey", "xyz789..."]  // Points to server operator's pubkey
  ]
}

// Server operator publishes separately (kind 10070)
{
  "kind": 10070,
  "pubkey": "xyz789...",
  "tags": [
    ["url", "https://blossom.example.com"],              // Legacy endpoint
    ["url-frontier", "https://blossom.example.com:8443"], // Frontier endpoint
    ["tls-pubkey", "deadbeef..."],                        // For port 8443
    ["tls-alg", "ed25519"]
  ]
}
```

This is cleaner - server operators control their own identity event and can update endpoints without users re-publishing NNS claims.

## Key Generation: Pragmatic Approach

**For servers: Generate a fresh keypair**
- Don't reuse Nostr keys for TLS
- Use standard TLS key generation tools
- Publish public key via Nostr
- Rotate periodically (publish new event)

```bash
# Server operator workflow
openssl genpkey -algorithm ED25519 -out tls.key
openssl pkey -in tls.key -pubout -out tls.pub

# Extract hex pubkey
hex_pubkey=$(openssl pkey -pubin -in tls.pub -text | grep 'pub:' -A3 | tail -3 | tr -d ' :\n')

# Publish to nostr
nostr publish --kind 10070 \
  --tag url="https://my-blossom-server.com" \
  --tag tls-pubkey="$hex_pubkey" \
  --tag tls-alg="ed25519"
```

**For browser clients: No keys needed**
- Browser is just a client, doesn't need to authenticate itself
- One-way TLS (server auth only) is sufficient
- Users already have Nostr keys, but don't use them for TLS

## Backward Compatibility: The Two-Port Strategy

**Problem**: Self-signed certs break legacy browsers (Chrome, Firefox, Safari).

**Solution**: Run two endpoints - one for legacy, one for Frontier.

### Server Configuration (nginx example)

```nginx
# Port 443 - CA-signed cert for legacy browsers
server {
    listen 443 ssl;
    server_name blossom.example.com;
    ssl_certificate /path/to/letsencrypt-cert.crt;
    ssl_certificate_key /path/to/letsencrypt-key.key;

    location / {
        # Serve blobs as usual
        root /var/blossom/blobs;
    }
}

# Port 8443 - Self-signed cert for Frontier browser
server {
    listen 8443 ssl;
    server_name blossom.example.com;
    ssl_certificate /path/to/self-signed.crt;
    ssl_certificate_key /path/to/self-signed.key;

    location / {
        # Same blob serving logic
        root /var/blossom/blobs;
    }
}
```

### Server Identity Event

```json
{
  "kind": 10070,
  "pubkey": "<server operator's nostr key>",
  "tags": [
    ["url", "https://blossom.example.com"],              // Legacy (CA-verified)
    ["url-frontier", "https://blossom.example.com:8443"], // Frontier (self-signed)
    ["tls-pubkey", "deadbeef..."],                        // Pubkey for port 8443
    ["tls-alg", "ed25519"]
  ]
}
```

### Browser Behavior

```rust
// Frontier browser looks for url-frontier tag first
let url = if let Some(frontier_url) = event.get_tag("url-frontier") {
    // Use self-signed cert endpoint with Nostr verification
    frontier_url
} else if let Some(legacy_url) = event.get_tag("url") {
    // Fallback to CA-verified endpoint
    legacy_url
} else {
    return Err("No URL found");
};
```

### Why This Works

✅ **Zero disruption**: Legacy browsers keep working on port 443
✅ **Simple**: Just add one server block to nginx config
✅ **Gradual adoption**: Server can add port 8443 whenever ready
✅ **No DNS changes**: Same domain, different port
✅ **Clear separation**: CA cert on 443, self-signed on 8443

### Server Operator Steps

1. **Keep existing setup** (port 443 with CA cert)
2. **Generate new keypair** for self-signed cert:
   ```bash
   openssl genpkey -algorithm ED25519 -out frontier.key
   openssl req -new -x509 -key frontier.key -out frontier.crt -days 3650 \
     -subj "/CN=blossom.example.com"
   ```
3. **Add port 8443** to nginx/caddy/etc config
4. **Publish Nostr event** with both URLs + pubkey
5. **Done** - both browsers work

### Adoption Path

**Phase 3a: Browser supports both modes**
- Frontier browser can use CA-verified OR Nostr-verified
- Prefers Nostr-verified if available
- Shows which mode in URL bar

**Phase 3b: Early adopter servers add port 8443**
- 2-3 blossom servers add the alternative endpoint
- Publish kind 10070 events
- Test with Frontier browser

**Phase 3c: Gradual rollout**
- More servers add support as Frontier gains users
- No breaking changes for anyone
- Eventually port 443 could be deprecated (far future)

## Web of Trust Enhancement

When verifying server identity:
1. Check Nostr event is signed by expected pubkey ✓
2. Check event has good WoT score (optional):
   - Server operator followed by people you trust?
   - Server operator has good reputation?
   - Other users vouch for this server?

Could show trust indicator:
```
🔒 Encrypted (verified via Nostr)
   Server operated by @operator_name
   Trusted by 42 people in your network
```

## Alternative: Skip TLS, Use HTTP/3 + Noise

If TLS cert manipulation is too complex, could build simpler authenticated channel:
- HTTP over QUIC
- Custom handshake with Noise protocol
- Verify server's Nostr key directly

But this requires custom protocol implementation. TLS + self-signed certs is more pragmatic.

## Test Plan

1. Set up test blossom server with self-signed cert
2. Publish kind 10070 event with server's TLS pubkey
3. Browser fetches event, extracts pubkey
4. Browser connects via HTTPS, verifies cert pubkey matches
5. Accept if match, reject if mismatch
6. Try MITM attack (different cert) - should fail
7. Try serving from IP in event but with wrong cert - should fail

## Success Criteria

- [ ] Browser can extract public key from self-signed TLS cert
- [ ] Browser can verify cert pubkey matches Nostr event
- [ ] Connections are encrypted (TLS 1.3)
- [ ] MITM attacks are detected (mismatched pubkey)
- [ ] Server operators can easily publish their TLS pubkey
- [ ] Browser still works with traditional CA-verified sites (fallback)
- [ ] User can see verification status in UI

## Security Properties Achieved

✅ **No DNS**: Names resolved via Nostr
✅ **No CAs**: Identity verified via Nostr web of trust
✅ **Encrypted**: TLS 1.3 encryption
✅ **Verifiable**: Public keys gossiped via Nostr relays
✅ **Decentralized**: Any server can participate
✅ **Resilient**: Multiple relays, multiple blossom servers

## Implementation Notes

### Rust TLS Libraries

**Option A: rustls**
- Pure Rust TLS implementation
- Easy to customize cert verification
- Used by reqwest with `rustls-tls` feature

```rust
use rustls::{ClientConfig, ServerCertVerifier};

struct NostrCertVerifier {
    expected_pubkey: Vec<u8>,
}

impl ServerCertVerifier for NostrCertVerifier {
    fn verify_server_cert(&self, cert: &Certificate, ...) -> Result<...> {
        let pubkey = extract_pubkey(cert)?;
        if pubkey == self.expected_pubkey {
            Ok(())
        } else {
            Err("Pubkey mismatch")
        }
    }
}
```

**Option B: openssl**
- More complex but more features
- Can extract pubkey with openssl crate
- Custom verification callback

Choose rustls for simplicity.

### Key Rotation

Server operator can publish new event with new key:
```json
{
  "kind": 10070,
  "tags": [
    ["url", "https://my-server.com"],
    ["tls-pubkey", "new-key..."],
    ["tls-alg", "ed25519"],
    ["replaces", "<old-event-id>"]  // Optional: mark old key as replaced
  ]
}
```

Browser accepts most recent event for a given URL.

## Out of Scope

- Client authentication (mutual TLS)
- Revocation lists
- OCSP stapling
- Complex key hierarchies
- Automatic key rotation

## Future Enhancements

- Use secp256k1 for TLS keys (match Nostr)
- Mutual TLS with user's Nostr key
- HTTP/3 + QUIC with custom handshake
- Peer-to-peer direct connections (no servers)

## Deployment Strategy

**Week 1**: Implement browser-side cert verification with two-port support
**Week 2**: Set up test blossom server on localhost with both ports
**Week 3**: Deploy test server publicly (both 443 and 8443)
**Week 4**: Document server setup, reach out to blossom operators
**Week 5+**: Get 2-3 existing servers to add port 8443, iterate

## The Pragmatic Win

We don't need to invent a new protocol. We use:
- Standard TLS 1.3 (proven crypto)
- Standard HTTPS (proven protocol)
- Self-signed certs (already supported)
- Just change verification logic in browser

Existing servers need minimal changes:
1. Generate keypair for port 8443
2. Add one nginx server block
3. Publish one Nostr event

**Zero disruption**: Legacy browsers keep working on port 443. Frontier browser uses port 8443. That's it. No custom protocols, no fancy crypto, just replacing the CA trust root with Nostr social trust.
