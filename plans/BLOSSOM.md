# Phase 2: Blossom Integration

## Goal
Replace direct IP addresses with content-addressed blobs stored on Blossom servers. Sites become immutable, verifiable, and can be hosted redundantly.

## Prerequisite
Phase 1 (IP.md) working - we have NNS resolution and basic fetching.

## NNS Event Format (Updated)

```json
{
  "kind": 34256,
  "pubkey": "<user's nostr pubkey>",
  "created_at": 1234567890,
  "tags": [
    ["d", "justinmoon"],                                    // claimed name
    ["blossom", "abc123...def"],                            // root hash (site manifest)
    ["server", "https://blossom.primal.net"],               // fallback server
    ["server", "https://cdn.satellite.earth"]               // another option
  ],
  "content": "",
  "sig": "..."
}
```

Alternatively, use the existing nsite format:
- kind 34128 events map paths to hashes
- Browser queries for all kind 34128 events from the pubkey
- Fetches each hash from blossom servers

## Site Publishing (nsite-style)

User uploads their static site:

```bash
# Upload files to blossom
curl -X PUT https://blossom.server/upload \
  -H "Authorization: Nostr <base64-event>" \
  -d @index.html

# Returns: { "sha256": "abc123..." }

# Publish path mapping
nostr publish --kind 34128 \
  --tag d="/index.html" \
  --tag sha256="abc123..."
```

Or use nsite's `uploadr.py` tool (~/code/nsite).

## Browser Changes

### 1. Blossom Resolution
```
User enters "justinmoon"
  ↓
Query relay for kind 34256 with d=justinmoon
  ↓
Get claim with blossom hash
  ↓
Query for kind 34128 events from that pubkey (path mappings)
  ↓
Find hash for /index.html
  ↓
Fetch hash from blossom server
  ↓
Verify sha256 matches
  ↓
Render HTML
```

### 2. Blossom Client
```rust
struct BlossomClient {
    servers: Vec<Url>,
}

impl BlossomClient {
    async fn fetch(&self, hash: &str) -> Result<Vec<u8>> {
        for server in &self.servers {
            let url = format!("{}/{}", server, hash);
            if let Ok(bytes) = reqwest::get(&url).await?.bytes().await {
                // Verify hash
                let computed = sha256(&bytes);
                if computed == hash {
                    return Ok(bytes.to_vec());
                }
            }
        }
        Err("Failed to fetch from any server")
    }
}
```

### 3. Blossom Server Discovery

Option A: Hardcoded fallback servers
```rust
const DEFAULT_BLOSSOM_SERVERS: &[&str] = &[
    "https://blossom.primal.net",
    "https://cdn.satellite.earth",
];
```

Option B: Query user's kind 10063 event (BUD-03)
```json
{
  "kind": 10063,
  "pubkey": "<site owner's pubkey>",
  "tags": [
    ["server", "https://blossom.server1.com"],
    ["server", "https://blossom.server2.com"]
  ]
}
```

Browser tries servers in order until one works.

## Content Integrity

Every blob fetch is verified:
1. Download bytes from blossom server
2. Compute SHA-256 hash
3. Compare to expected hash from kind 34128 event
4. Reject if mismatch

This is critical: even if blossom server is malicious, can't serve wrong content.

## Caching Strategy

Browser can cache blobs locally by hash:
```
~/.frontier/cache/
  abc123def.../  (blob content)
```

Since hashes are content-addressed:
- Cache never invalidates (hash changes = new content)
- Can cache aggressively
- Offline support for free

## Navigation Within Site

When user clicks link on the page:
1. Parse href: `/about.html`
2. Query kind 34128 events for `d="/about.html"`
3. Get hash
4. Fetch from blossom
5. Render

Links work naturally if site was uploaded with nsite structure.

## Test Plan

1. Upload simple static site to blossom using nsite tools
2. Publish kind 34256 event claiming name → blossom hash
3. Publish kind 34128 events for each file path
4. Enter name in browser
5. Browser fetches and verifies all hashes
6. Navigate between pages
7. Verify integrity (try to MITM/modify blob, should fail)

## Success Criteria

- [ ] Browser can fetch blobs from blossom servers
- [ ] SHA-256 verification works
- [ ] Multi-file sites work (multiple kind 34128 events)
- [ ] Navigation within site works
- [ ] Falls back to alternate servers if one fails
- [ ] Caching works (fast reload)
- [ ] Integrity violations are detected and rejected

## Still Out of Scope

- HTTPS/TLS (still using HTTP to blossom servers)
- Certificate verification
- Custom protocols
- Authentication

## Next Steps → NO_CERTS.md

Add encryption without certificate authorities.
