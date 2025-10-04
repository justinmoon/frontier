# NNS Testing Guide

This guide walks you through testing the Nostr Name System (NNS) implementation.

## Quick Start

### 1. Start the Test HTTP Server

In one terminal, run:

```bash
python3 scripts/test_server.py
```

This starts a simple HTTP server on `localhost:8080` that serves a test page.

### 2. Publish an NNS Claim

You have two options:

#### Option A: Using the Rust script (recommended)

```bash
# If you have rust-script installed:
rust-script scripts/publish_nns_claim.rs testsite 127.0.0.1:8080

# Or compile and run:
rustc scripts/publish_nns_claim.rs -o /tmp/publish_nns && /tmp/publish_nns testsite 127.0.0.1:8080
```

#### Option B: Using the Python script

```bash
# Install nostr-sdk for Python first:
pip install nostr-sdk

# Then run:
python3 scripts/publish_nns_claim.py testsite --ip 127.0.0.1:8080
```

**Important:** Save the private key that gets printed! You'll need it to update your claim later.

### 3. Test in Frontier Browser

Build and run Frontier:

```bash
cargo run
```

In the URL bar, enter:
```
testsite
```

The browser should:
1. Query Nostr relays for claims to "testsite"
2. Find your published claim
3. Fetch the content from `http://127.0.0.1:8080`
4. Display the test page

## Testing Multiple Claims

To test the claim selection UI, publish multiple claims to the same name:

```bash
# First claim (from your key)
rust-script scripts/publish_nns_claim.rs mysite 127.0.0.1:8080 YOUR_PRIVATE_KEY

# Second claim (generates a new key)
rust-script scripts/publish_nns_claim.rs mysite 192.168.1.100:8080
```

When you navigate to "mysite" in Frontier, you should see a selection UI showing both claims.

## Testing Saved Choices

1. Navigate to a name with multiple claims
2. Select one of the claims
3. Close and restart Frontier
4. Navigate to the same name again
5. It should automatically use your saved choice

Saved choices are stored in: `~/.local/share/frontier/nns_choices.json` (on macOS: `~/Library/Application Support/frontier/nns_choices.json`)

## Troubleshooting

### "No claims found for name"

- Make sure your claim was successfully published to the relays
- Wait a few seconds and try again (relay propagation can take time)
- Check that you're using the same relays in both the browser and publish script

### "Failed to resolve NNS name"

- Ensure your Nostr relays are accessible
- Check your internet connection
- Try using different relays in `src/nns.rs`

### Connection timeout

- Make sure the test server is running on the correct port
- If using a remote IP, ensure it's accessible from your machine
- Check firewall settings

## Advanced Testing

### Test with Remote Server

1. Deploy the test page to a public server
2. Publish an NNS claim with the public IP:
   ```bash
   rust-script scripts/publish_nns_claim.rs myremotesite YOUR_PUBLIC_IP:8080 YOUR_PRIVATE_KEY
   ```
3. Test accessing it from Frontier

### Test Direct IP Access

You can also bypass NNS and directly access IP:port combinations:

```
127.0.0.1:8080
192.168.1.100:8080
```

These should load without querying Nostr relays.

## Next Steps

Once basic NNS resolution is working, see `plans/BLOSSOM.md` for adding content-addressed storage.
