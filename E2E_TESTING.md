# End-to-End Testing Guide

## Quick Test (Component Level)

Run the comprehensive test suite:

```bash
cargo test --test nns_e2e_full_test
```

This tests:
- ✅ URL bar shows NNS name (not resolved IP)
- ✅ No padding gaps between URL bar and content
- ✅ URL bar accessibility and structure
- ✅ Navigation flow simulation

## Full E2E Test (Manual with Real Browser)

This test runs the ACTUAL browser with a local relay and HTTP server:

```bash
./scripts/test_nns_full_e2e.sh
```

### What it does:

1. **Starts HTTP server** on `localhost:18080`
   - Serves a test HTML page with green success styling
   - Has specific markers to verify rendering

2. **Starts local Nostr relay** using `nak serve`
   - Runs on `ws://localhost:7777`
   - No external relay dependencies

3. **Publishes NNS event**
   - Creates kind 34256 event
   - Maps `testsite` → `127.0.0.1:18080`
   - Stores in local relay

4. **Provides test instructions**
   - Tells you to update relay URL in code
   - Guides you through manual browser test

### Manual Test Steps:

1. Run the script:
   ```bash
   ./scripts/test_nns_full_e2e.sh
   ```

2. When prompted, update `src/nns.rs` lines 155-156:
   ```rust
   client.add_relay("ws://localhost:7777").await?;
   // Comment out the default relays temporarily
   ```

3. Rebuild:
   ```bash
   cargo build
   ```

4. Run Frontier:
   ```bash
   cargo run
   ```

5. In the browser URL bar, type: `testsite`

6. Press Enter or click "Go"

### Expected Results:

✅ **SUCCESS INDICATORS:**
- Green page with "NNS E2E Test Success!" heading appears
- URL bar shows `testsite` (NOT `127.0.0.1:18080`)
- NO gap between URL bar and page content
- Content starts immediately below the URL bar

❌ **FAILURE INDICATORS:**
- Shows example.com or error page
- URL bar shows IP address instead of name
- Visible gap/padding between URL bar and content
- Browser errors in terminal

### Troubleshooting:

Check the logs if it fails:
```bash
cat /tmp/http_server.log  # HTTP server logs
cat /tmp/nak_relay.log    # Nostr relay logs
```

Common issues:
- **"No claims found"**: Relay connection failed or event not published
- **"Connection refused"**: HTTP server not running
- **Shows IP in URL bar**: Code not updated to use display_url
- **Gap at top**: CSS not applied or nested body margin override missing

## Test Infrastructure Files

- `tests/nns_e2e_full_test.rs` - Rust component tests (automated)
- `scripts/test_nns_full_e2e.sh` - Full e2e script (manual)
- `tests/layout_test.rs` - CSS layout verification
- `tests/nns_integration_test.rs` - URL parsing flow tests

## CI Integration

The automated tests run in CI:
```bash
just ci  # Runs all tests including e2e component tests
```

The manual e2e test with `nak` is for local development only.
