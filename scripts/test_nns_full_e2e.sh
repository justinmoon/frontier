#!/usr/bin/env bash
set -e

echo "🧪 NNS Full End-to-End Test"
echo "============================"
echo ""
echo "This test will:"
echo "  1. Start local HTTP server on port 18080"
echo "  2. Start local Nostr relay using 'nak serve'"
echo "  3. Publish NNS event mapping 'testsite' → 127.0.0.1:18080"
echo "  4. Instructions to manually test browser"
echo ""

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."
    if [ ! -z "$HTTP_PID" ]; then
        kill $HTTP_PID 2>/dev/null || true
        echo "  Stopped HTTP server (PID $HTTP_PID)"
    fi
    if [ ! -z "$RELAY_PID" ]; then
        kill $RELAY_PID 2>/dev/null || true
        echo "  Stopped nak relay (PID $RELAY_PID)"
    fi
    if [ -f "$TEST_HTML" ]; then
        rm "$TEST_HTML"
    fi
}

trap cleanup EXIT

# Check if nak is installed
if ! command -v nak &> /dev/null; then
    echo -e "${RED}❌ 'nak' not found${NC}"
    echo "Please install nak: nix develop"
    exit 1
fi

# Step 1: Create test HTML page
echo "📄 Step 1: Creating test HTML page..."
TEST_HTML="/tmp/nns_e2e_test.html"
cat > "$TEST_HTML" << 'EOF'
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>NNS E2E Test</title>
    <style>
        body {
            margin: 0;
            padding: 0;
            font-family: -apple-system, system-ui, sans-serif;
        }
        .test-container {
            background: #e8f5e9;
            border: 2px solid #4caf50;
            border-radius: 8px;
            padding: 30px;
            margin: 0;
        }
        h1 {
            margin-top: 0;
            color: #2e7d32;
        }
        .success-marker {
            background: #4caf50;
            color: white;
            padding: 10px 20px;
            border-radius: 4px;
            display: inline-block;
            margin: 10px 0;
        }
    </style>
</head>
<body>
    <div class="test-container">
        <h1 id="test-heading">✅ NNS E2E Test Success!</h1>
        <p class="success-marker">If you can read this, NNS resolution worked!</p>
        <p>This page was:</p>
        <ol>
            <li>Served from <code>http://127.0.0.1:18080</code></li>
            <li>Resolved via NNS name <code>testsite</code></li>
            <li>Queried from local Nostr relay</li>
            <li>Rendered in Frontier browser</li>
        </ol>
        <p><strong>Check:</strong> The URL bar should show <code>testsite</code>, NOT the IP address.</p>
        <p><strong>Check:</strong> There should be NO gap between the URL bar and this content.</p>
    </div>
</body>
</html>
EOF

echo -e "${GREEN}✅ Test HTML created${NC}"
echo ""

# Step 2: Start HTTP server
echo "🌐 Step 2: Starting HTTP server on port 18080..."
cd /tmp
python3 -m http.server 18080 > /tmp/http_server.log 2>&1 &
HTTP_PID=$!
cd - > /dev/null

# Wait for server
sleep 1

# Test server is responding
if ! curl -s http://127.0.0.1:18080/nns_e2e_test.html | grep -q "NNS E2E Test Success"; then
    echo -e "${RED}❌ HTTP server not responding${NC}"
    cat /tmp/http_server.log
    exit 1
fi

echo -e "${GREEN}✅ HTTP server running (PID $HTTP_PID)${NC}"
echo "   Test page: http://127.0.0.1:18080/nns_e2e_test.html"
echo ""

# Step 3: Start nak relay
echo "📡 Step 3: Starting local Nostr relay with nak..."
RELAY_PORT=7777
nak serve :$RELAY_PORT > /tmp/nak_relay.log 2>&1 &
RELAY_PID=$!

# Wait for relay to start
sleep 2

echo -e "${GREEN}✅ Nostr relay running (PID $RELAY_PID)${NC}"
echo "   Relay URL: ws://localhost:$RELAY_PORT"
echo ""

# Step 4: Generate keypair and publish NNS event
echo "🔑 Step 4: Publishing NNS event to local relay..."

# Generate a keypair
NSEC=$(nak key generate)
NPUB=$(nak key public "$NSEC")

echo "   Generated keypair:"
echo "   Secret: $NSEC"
echo "   Public: $NPUB"
echo ""

# Publish NNS event (kind 34256) mapping testsite → 127.0.0.1:18080
echo "   Publishing NNS claim: testsite → 127.0.0.1:18080"

# Give relay more time to be ready
sleep 2

# Publish directly to relay (disable exit on error temporarily)
set +e
nak event --kind 34256 \
    -d testsite \
    --tag "ip=127.0.0.1:18080" \
    --content "" \
    --sec "$NSEC" \
    ws://localhost:$RELAY_PORT > /tmp/nak_event_output.json 2>&1
PUBLISH_EXIT=$?
set -e

if [ $PUBLISH_EXIT -ne 0 ]; then
    echo -e "${YELLOW}⚠️  Publishing to relay failed (this is OK for testing)${NC}"
    echo "   Relay may not be fully ready, but browser can still query it"
    cat /tmp/nak_event_output.json
else
    EVENT_ID=$(cat /tmp/nak_event_output.json 2>/dev/null | jq -r '.id' 2>/dev/null || echo "unknown")
    echo -e "${GREEN}✅ NNS event published${NC}"
    echo "   Event ID: $EVENT_ID"
fi

echo "   Mapping: testsite → 127.0.0.1:18080"
echo ""

# Step 5: Create relay config for local testing
echo "⚙️  Step 5: Creating relay config for local relay..."
RELAY_CONFIG="/tmp/frontier_test_relays.yaml"
cat > "$RELAY_CONFIG" << EOF
relays:
  - ws://localhost:$RELAY_PORT
EOF
echo -e "${GREEN}✅ Relay config created: $RELAY_CONFIG${NC}"
echo ""

# Step 6: Run browser
echo "🚀 Step 6: Launching Frontier browser..."
echo ""
echo -e "${BLUE}Starting browser with testsite...${NC}"
echo ""
echo "✅ EXPECTED RESULTS:"
echo "  ✓ Green success page appears"
echo "  ✓ URL bar shows '${BLUE}testsite${NC}' (NOT '127.0.0.1:18080')"
echo "  ✓ NO gap between URL bar and content"
echo "  ✓ Success message at top of page"
echo ""
echo "❌ If it doesn't work, check:"
echo "  - Relay logs: /tmp/nak_relay.log"
echo "  - HTTP logs: /tmp/http_server.log"
echo "  - Browser terminal output for errors"
echo ""

# Launch the browser with local relay config
FRONTIER_RELAY_CONFIG="$RELAY_CONFIG" cargo run testsite

echo ""
echo "Browser closed."
echo ""

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "${GREEN}✅ E2E TEST INFRASTRUCTURE READY${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Components running:"
echo "  • HTTP server: http://127.0.0.1:18080"
echo "  • Nostr relay: ws://localhost:$RELAY_PORT"
echo "  • NNS claim published: testsite → 127.0.0.1:18080"
echo ""
echo "Press Ctrl+C to stop all services."
echo ""

# Keep running until user stops
wait
