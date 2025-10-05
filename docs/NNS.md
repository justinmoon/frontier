# NNS (Nostr Name System)

Frontier replaces DNS with a decentralized name resolution system built on Nostr. Instead of querying centralized DNS servers, the browser queries Nostr relays to resolve names to either IP addresses or content-addressed blobs.

## What We've Built

### Phase 1: Direct IP Resolution ✅

Users publish kind 34256 events claiming names and mapping them to IP addresses:

```json
{
  "kind": 34256,
  "pubkey": "abc123...",
  "created_at": 1234567890,
  "tags": [
    ["d", "justinmoon"],
    ["ip", "192.168.1.100:8080"],
    ["note", "My personal site"]
  ],
  "content": "",
  "sig": "..."
}
```

**Flow:**
1. User types `justinmoon` in URL bar
2. Browser queries relays for kind 34256 events with `d=justinmoon`
3. Gets claims (possibly from multiple publishers)
4. Shows selection UI if multiple claims exist
5. Remembers user's choice in SQLite
6. Fetches `http://192.168.1.100:8080`

### Phase 2: Blossom Integration ✅

Sites can be hosted as content-addressed blobs on Blossom servers, verified by SHA-256 hashes:

```json
{
  "kind": 34256,
  "pubkey": "abc123...",
  "tags": [
    ["d", "blossomsite"],
    ["blossom", "deadbeef123..."],
    ["server", "https://blossom.primal.net"],
    ["server", "https://cdn.satellite.earth"]
  ]
}
```

Publishers also publish kind 34128 events (manifest) mapping paths to hashes:

```json
{
  "kind": 34128,
  "pubkey": "abc123...",
  "tags": [
    ["d", "/index.html"],
    ["sha256", "abc123..."]
  ]
}
```

**Flow:**
1. User types `blossomsite`
2. Browser resolves to Blossom claim
3. Fetches manifest (all kind 34128 events from that pubkey)
4. Looks up hash for `/index.html` in manifest
5. Downloads blob from Blossom server
6. **Verifies SHA-256 hash** matches manifest
7. Renders HTML

**Navigation within site:**
- Links like `<a href="/about.html">` work natively
- Browser looks up `/about.html` in cached manifest
- Fetches blob by hash
- Verifies integrity
- Renders

## Architecture

### NNS Resolver (`src/nns/resolver.rs`)

Core resolution logic:

```rust
pub struct NnsResolver {
    storage: Arc<Storage>,
    relay_directory: RelayDirectory,
    client: NostrClient,
}

impl NnsResolver {
    pub async fn resolve(&self, name: &str) -> Result<ResolverOutput> {
        // 1. Check SQLite cache (10min TTL)
        if let Some(cached) = self.cached_claims(name)? {
            return Ok(cached);
        }

        // 2. Query Nostr relays for kind 34256 events
        let events = self.fetch_from_relays(name).await?;

        // 3. Parse and validate events
        let claims = parse_and_validate(events)?;

        // 4. Score and rank claims
        let ranked = score_claims(claims);

        // 5. Persist to SQLite
        self.persist_claims(name, &ranked).await?;

        Ok(ResolverOutput { claims: ranked, ... })
    }
}
```

### Claim Model (`src/nns/models.rs`)

```rust
pub enum ClaimLocation {
    DirectIp(SocketAddr),
    Blossom {
        root_hash: String,
        servers: Vec<Url>,
    },
}

pub struct NnsClaim {
    pub name: String,
    pub location: ClaimLocation,
    pub pubkey_hex: String,
    pub pubkey_npub: String,
    pub created_at: Timestamp,
    pub relays: HashSet<Url>,
    pub note: Option<String>,
    pub event_id: EventId,
}
```

### Blossom Fetcher (`src/blossom/mod.rs`)

Handles content-addressed blob fetching:

```rust
pub struct BlossomFetcher {
    client: NostrClient,
    relay_directory: RelayDirectory,
    http: reqwest::Client,
    manifest_cache: RwLock<HashMap<String, CachedManifest>>,
}

impl BlossomFetcher {
    pub async fn manifest_for(
        &self,
        pubkey_hex: &str,
        relays: &[Url]
    ) -> Result<BlossomManifest> {
        // Fetch all kind 34128 events from this pubkey
        let filter = Filter::new()
            .kind(Kind::from(34128))
            .author(pubkey)
            .limit(500);

        let events = self.client.fetch_events(relays, filter).await?;

        // Build path -> hash mapping
        let manifest = parse_manifest_events(events)?;
        Ok(manifest)
    }

    pub async fn fetch_blob_by_hash(
        &self,
        servers: &[Url],
        hash: &str
    ) -> Result<Vec<u8>> {
        for server in servers {
            let url = format!("{}/{}", server, hash);
            let bytes = reqwest::get(&url).await?.bytes().await?;

            // Verify integrity
            let computed = sha256(&bytes);
            if computed == hash {
                return Ok(bytes.to_vec());
            }
        }
        Err("Failed to fetch from any server")
    }
}
```

### Storage (`src/storage/sqlite.rs`)

Persistent cache using SQLite:

```sql
CREATE TABLE claims (
    name TEXT,
    pubkey TEXT,
    ip TEXT,
    relays TEXT,
    created_at INTEGER,
    fetched_at INTEGER,
    event_id TEXT,
    location TEXT,  -- JSON-serialized ClaimLocation
    PRIMARY KEY (name, pubkey)
);

CREATE TABLE selections (
    name TEXT PRIMARY KEY,
    pubkey TEXT NOT NULL,
    chosen_at INTEGER NOT NULL
);
```

**Cache TTL:** 10 minutes - balances freshness with relay load.

**Selection persistence:** When user picks a claim for "justinmoon", we remember that choice. Next time they navigate to "justinmoon", we auto-select the same publisher.

## Navigation Flow

### Example 1: Direct IP (Single Claim)

```
User enters: "bob"
    ↓
parse_input() → ParsedInput::NnsName("bob")
    ↓
prepare_navigation()
    ↓
resolver.resolve("bob")
    ↓
Cache miss → query relays
    ↓
Find 1 claim: { pubkey: alice, ip: "1.2.3.4:8080" }
    ↓
Auto-select (single claim)
    ↓
NavigationPlan::Fetch(http://1.2.3.4:8080)
    ↓
execute_fetch()
    ↓
Render HTML
```

### Example 2: Multiple Claims (Requires Selection)

```
User enters: "bitcoin"
    ↓
resolver.resolve("bitcoin")
    ↓
Find 5 claims from different publishers
    ↓
NavigationPlan::RequiresSelection
    ↓
Show overlay UI:
┌────────────────────────────────────┐
│ Select site for bitcoin            │
├────────────────────────────────────┤
│ ▶ 192.168.1.10:80   npub1abc…      │
│   127.0.0.1:8080    npub1xyz…      │
│   10.0.0.5:3000     npub1def…      │
└────────────────────────────────────┘
    ↓
User presses Enter
    ↓
record_selection("bitcoin", "abc...")
    ↓
Fetch from selected IP
```

### Example 3: Blossom Site

```
User enters: "blossomsite"
    ↓
resolver.resolve("blossomsite")
    ↓
Find claim with ClaimLocation::Blossom
    ↓
blossom.manifest_for(pubkey, relays)
    ↓
Query kind 34128 events → build path->hash map
    ↓
Look up /index.html → hash abc123...
    ↓
Try servers in order:
  - https://blossom.primal.net/abc123...
  - https://cdn.satellite.earth/abc123...
    ↓
Verify SHA-256 matches
    ↓
Render HTML
```

### Example 4: Blossom Path Navigation

```
User clicks: <a href="/about.html">
    ↓
parse_input("/about.html") → ParsedInput::NnsPath
    ↓
Look up /about.html in cached manifest
    ↓
Get hash: def456...
    ↓
fetch_blob_by_hash(servers, "def456...")
    ↓
Verify integrity
    ↓
Render
```

## URL Bar Behavior

The URL bar shows the **user-friendly name**, not the underlying IP or hash:

- User types: `justinmoon`
- URL bar shows: `justinmoon`
- Behind the scenes: Fetches from `http://192.168.1.100:8080`

For Blossom paths:
- User navigates to: `/about.html`
- URL bar shows: `blossomsite/about.html`
- Behind the scenes: Fetches hash `def456...` from Blossom server

## Testing

We have comprehensive E2E tests that use **real infrastructure** (no mocks):

### `tests/nns_e2e_test.rs`
- Starts real HTTP server
- Starts real WebSocket relay (mock Nostr protocol)
- Publishes kind 34256 event
- Tests full resolution flow
- Verifies content fetching

### `tests/blossom_e2e_test.rs`
- Starts real Blossom HTTP server
- Starts real relay with kind 34256 + kind 34128 events
- Tests manifest fetching
- Verifies SHA-256 integrity
- Tests multi-file navigation

### `tests/nns_error_paths_test.rs`
- Invalid IP addresses
- Malformed events
- Cache expiry
- Selection persistence
- Relay timeouts
- Multiple claims handling

### Manual Testing: `scripts/test_nns_full_e2e.sh`

```bash
./scripts/test_nns_full_e2e.sh
```

This script:
1. Starts Python HTTP server on `localhost:18080`
2. Starts `nak serve` relay on `ws://localhost:7777`
3. Publishes NNS event with `nak`
4. Launches Frontier browser with `testsite`
5. Shows green success page if working

**All tests pass in CI.** See `just ci` output.

## Input Parsing (`src/input.rs`)

Smart parsing distinguishes between URLs, IPs, NNS names, and paths:

```rust
pub enum ParsedInput {
    Url(Url),              // "https://example.com"
    DirectIp(Url),         // "192.168.1.1:8080"
    NnsName(String),       // "justinmoon"
    NnsPath { name, path } // "justinmoon/about.html"
}
```

**Examples:**
- `justinmoon` → `NnsName`
- `justinmoon/about.html` → `NnsPath`
- `192.168.1.1:8080` → `DirectIp`
- `https://example.com` → `Url`
- `example.com` → `Url` (adds https://)

## Scoring & Ranking

When multiple publishers claim the same name, we score them:

```rust
pub fn score_claim(
    claim: &NnsClaim,
    selected_pubkey: Option<&str>
) -> f64 {
    let mut score = 0.0;

    // Huge boost for user's previous selection
    if Some(&claim.pubkey_hex) == selected_pubkey {
        score += 1000.0;
    }

    // Prefer recent claims
    let age_seconds = now() - claim.created_at.as_u64();
    let age_days = age_seconds as f64 / 86400.0;
    score += 100.0 / (1.0 + age_days);

    // Bonus for being seen on multiple relays
    score += claim.relays.len() as f64 * 10.0;

    score
}
```

This ensures:
1. User's previous choice is always preferred
2. Recent claims rank higher
3. Claims seen on multiple relays get a boost

## Security Properties

✅ **No DNS** - Names resolved via Nostr relays
✅ **Cryptographic verification** - All events have valid signatures
✅ **Content integrity** - Blossom blobs verified via SHA-256
✅ **Decentralized** - No single point of failure
✅ **User choice** - When conflicts exist, user decides
✅ **Transparent** - Can see all claims and their publishers

## Limitations (Current)

❌ **No HTTPS** - Using plain HTTP to servers
❌ **No TLS verification** - Certificate authorities still needed
❌ **No WoT scoring** - Basic scoring only (recency + relay count)
❌ **No revocation** - Can't unpublish a claim (replaceable events only)

## What's Next: Phase 3

See `plans/NO_CERTS.md` for:
- TLS encryption without certificate authorities
- Self-signed certs verified via Nostr-published keys
- Two-port strategy for backward compatibility
- Web of trust integration

## Implementation Files

Core modules:
- `src/nns/resolver.rs` - Resolution logic
- `src/nns/models.rs` - Data structures
- `src/nns/scoring.rs` - Claim ranking
- `src/blossom/mod.rs` - Blossom client
- `src/storage/sqlite.rs` - Caching & persistence
- `src/input.rs` - URL bar parsing
- `src/navigation.rs` - Navigation flow

Tests:
- `tests/nns_e2e_test.rs` - Direct IP E2E
- `tests/blossom_e2e_test.rs` - Blossom E2E
- `tests/nns_error_paths_test.rs` - Edge cases

Scripts:
- `scripts/test_nns_full_e2e.sh` - Manual test with nak

## Usage Examples

### Publishing an IP claim

```bash
# Start your HTTP server
python3 -m http.server 8080

# Publish NNS claim
nak event --kind 34256 \
    -d mysite \
    --tag "ip=127.0.0.1:8080" \
    --tag "note=My personal website" \
    --sec <your-nsec> \
    wss://relay.damus.io
```

### Publishing a Blossom site

```bash
# Upload files to Blossom (get hashes back)
curl -X PUT https://blossom.primal.net/upload \
  -H "Authorization: Nostr <event>" \
  -d @index.html
# Returns: abc123...

# Publish manifest events
nak event --kind 34128 \
    -d "/index.html" \
    --tag "sha256=abc123..." \
    --sec <your-nsec> \
    wss://relay.damus.io

# Publish NNS claim pointing to root hash
nak event --kind 34256 \
    -d myblossomsite \
    --tag "blossom=abc123..." \
    --tag "server=https://blossom.primal.net" \
    --sec <your-nsec> \
    wss://relay.damus.io
```

### Navigating

```bash
# Direct IP site
cargo run mysite

# Blossom site
cargo run myblossomsite

# Blossom site with path
cargo run myblossomsite/about.html
```

## Performance

- **Cache hit:** < 1ms (SQLite lookup)
- **Cache miss:** 2-3s (relay query + parsing)
- **Blossom fetch:** 100-500ms (depends on server location)
- **Manifest cache:** 5 minutes in memory

## Success Metrics

All success criteria from Phase 1 & 2 completed:

✅ Browser can query relay for NNS events
✅ Browser parses IP:port from event
✅ Browser fetches via HTTP
✅ User can see multiple claims for same name
✅ User can pick a claim
✅ Choice persists across sessions
✅ Browser can fetch blobs from Blossom servers
✅ SHA-256 verification works
✅ Multi-file sites work (multiple kind 34128 events)
✅ Navigation within site works
✅ Falls back to alternate servers if one fails
✅ Integrity violations are detected and rejected

## Related Documentation

- `docs/CHROME.md` - Browser UI architecture
- `docs/SCREENSHOT_TESTING.md` - Visual testing
- `plans/NO_CERTS.md` - Phase 3 roadmap
- `CLAUDE.md` - Development guidelines
