# Phase 1: IP Address Demo

## Goal
Prove the NNS (Nostr Name System) concept with minimal complexity. No DNS, no certificate authorities, no blossom. Just name → IP resolution via Nostr events.

## NNS Event Format

```json
{
  "kind": 34256,
  "pubkey": "<user's nostr pubkey>",
  "created_at": 1234567890,
  "tags": [
    ["d", "justinmoon"],           // claimed name
    ["ip", "192.168.1.100:8080"]   // target IP:port
  ],
  "content": "",
  "sig": "..."
}
```

- Uses replaceable event (kind 34xxx)
- `d` tag is the identifier (the name being claimed)
- Multiple people can publish events claiming the same name
- Browser uses WoT to rank claims

## Server Setup

Simple HTTP server serving static HTML:

```bash
# Server side - any simple HTTP server
python3 -m http.server 8080
```

Or a basic Rust server:
```rust
// Just serve index.html on port 8080
```

No HTTPS, no certs, just plain HTTP for this demo.

## Browser Changes

### 1. NNS Resolution
When user enters a name (no dots, no scheme):
1. Query hardcoded relay(s) for kind 34256 events with `d` tag matching the name
2. Collect all claims to this name
3. Rank by WoT (initially: simple heuristic or just show all)
4. Present to user if multiple claims
5. Remember user's choice locally

### 2. URL Bar Parsing
```rust
fn parse_input(input: &str) -> ParsedInput {
    if input.contains('.') || input.starts_with("http") {
        // Traditional URL
        ParsedInput::Url(input)
    } else if input.contains(':') {
        // Direct IP:port
        ParsedInput::Direct(input)
    } else {
        // NNS name
        ParsedInput::NnsName(input)
    }
}
```

### 3. Fetch Flow
```
User enters "justinmoon"
  ↓
Query relay for kind 34256 with d=justinmoon
  ↓
Get claims: [
  { pubkey: abc..., ip: "1.2.3.4:8080" },
  { pubkey: def..., ip: "5.6.7.8:8080" }
]
  ↓
Rank by WoT (v1: just show list)
  ↓
User picks one (or top-ranked auto-selected)
  ↓
HTTP GET http://1.2.3.4:8080/
  ↓
Render page
```

## Web of Trust (v1 - Simple)

For this demo, just show all claims with basic info:
- Pubkey (abbreviated)
- When claim was published
- Number of followers (if we have that data)

User manually picks which claim to trust. Store choice locally:
```json
{
  "name": "justinmoon",
  "chosen_pubkey": "abc123...",
  "timestamp": 1234567890
}
```

## Test Plan

1. Run HTTP server on localhost:8080 with simple index.html
2. Publish NNS event claiming "testsite" → "127.0.0.1:8080"
3. Enter "testsite" in browser URL bar
4. Browser queries relay, finds claim, fetches via HTTP
5. Page renders

## Success Criteria

- [ ] Browser can query relay for NNS events
- [ ] Browser parses IP:port from event
- [ ] Browser fetches via HTTP (no HTTPS)
- [ ] User can see multiple claims for same name
- [ ] User can pick a claim
- [ ] Choice persists across sessions
- [ ] Page renders correctly

## Out of Scope

- HTTPS/TLS
- Content addressing (sha256 verification)
- Blossom integration
- Sophisticated WoT scoring
- Certificate verification
- Dynamic port scanning

## Next Steps → BLOSSOM.md

Once this works, add content-addressed storage via Blossom.
