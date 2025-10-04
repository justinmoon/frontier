# NNS Quick Start Guide

## Running the Browser Manually

The simplest way to test NNS:

```bash
# Just run the browser with an NNS name
cargo run testsite
```

By default, it will:
- Query public relays (relay.damus.io, nos.lol)
- Look for NNS claims for "testsite"
- Show claim selection UI if multiple publishers claim the name
- Navigate to the chosen IP address

## Full E2E Test with Local Infrastructure

For complete end-to-end testing with local relay + HTTP server:

```bash
./scripts/test_nns_full_e2e.sh
```

This will:
1. Start local HTTP server on `localhost:18080`
2. Start local Nostr relay on `ws://localhost:7777`
3. Publish NNS event: `testsite` → `127.0.0.1:18080`
4. Create relay config pointing to local relay
5. **Automatically launch browser with `testsite`**
6. Clean up when you close the browser

### Expected Results

✅ Browser should show:
- URL bar displays `testsite` (NOT the IP address)
- Green success page with "NNS E2E Test Success!"
- No gap between URL bar and content

## Publishing Your Own NNS Claim

To make a name resolve to your local dev server:

```bash
# Start your HTTP server
python3 -m http.server 8080

# Publish NNS claim (requires nak in nix shell)
nak event --kind 34256 \
    -d mysite \
    --tag "ip=127.0.0.1:8080" \
    --content "" \
    --sec <your-nsec> \
    wss://relay.damus.io \
    wss://nos.lol

# Test it
cargo run mysite
```

## Using Custom Relays

Create a relay config file:

```yaml
# relays.yaml
relays:
  - wss://relay.damus.io
  - wss://nos.lol
  - ws://localhost:7777  # your local relay
```

Run with custom config:

```bash
FRONTIER_RELAY_CONFIG=./relays.yaml cargo run testsite
```

## Troubleshooting

**"No claims found"**
- Check the NNS event was published: `nak req -k 34256 -d testsite wss://relay.damus.io`
- Try different relays in config
- Check relay is reachable

**Browser shows IP instead of name**
- This is the current behavior (display_url not yet wired up)
- The navigation still works correctly

**Connection errors**
- Verify HTTP server is running: `curl http://127.0.0.1:8080`
- Check relay logs: `/tmp/nak_relay.log`
- Check HTTP logs: `/tmp/http_server.log`

## Running Tests

```bash
# Full CI suite
just ci

# Unit tests only
cargo test

# With fixtures (requires nak)
./scripts/run_tests_with_fixtures.sh
```

## Next Steps

See full documentation:
- `E2E_TESTING.md` - Comprehensive testing guide
- `NNS_TESTING.md` - NNS-specific details
- `IMPLEMENTATION_SUMMARY.md` - Architecture overview
