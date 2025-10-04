#!/usr/bin/env bash
set -e

echo "🔧 Setting up test fixtures for NNS tests"
echo "=========================================="
echo ""

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up test fixtures..."
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

# Check dependencies
if ! command -v nak &> /dev/null; then
    echo -e "${RED}❌ 'nak' not found${NC}"
    echo "Run: nix develop"
    exit 1
fi

# Step 1: Create test HTML that exhibits the bug
echo "📄 Creating test HTML with problematic body margin..."
TEST_HTML="/tmp/nns_e2e_test.html"
cat > "$TEST_HTML" << 'EOF'
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>NNS Test</title>
    <style>
        /* THE BUG: This margin creates a gap */
        body {
            margin: 50px auto;
            padding: 20px;
            max-width: 800px;
            font-family: system-ui, sans-serif;
            background: #f6f8fa;
        }
        .test-box {
            background: #dafbe1;
            border: 2px solid #4caf50;
            padding: 20px;
            border-radius: 8px;
        }
    </style>
</head>
<body>
    <div class="test-box">
        <h1>NNS Test Page</h1>
        <p><strong>Bug test:</strong> This body has <code>margin: 50px auto;</code></p>
        <p>Our CSS <code>#content body { margin: 0 !important; }</code> should override it, but doesn't!</p>
        <p>Result: Visible gap between URL bar and content.</p>
    </div>
</body>
</html>
EOF

echo -e "${GREEN}✅ Test HTML created with body margin: 50px auto${NC}"
echo ""

# Step 2: Start HTTP server
echo "🌐 Starting HTTP server on localhost:18080..."
cd /tmp
python3 -m http.server 18080 > /tmp/http_server.log 2>&1 &
HTTP_PID=$!
cd - > /dev/null

sleep 1

# Verify server is running
if ! curl -s http://127.0.0.1:18080/nns_e2e_test.html | grep -q "NNS Test Page"; then
    echo -e "${RED}❌ HTTP server failed to start${NC}"
    cat /tmp/http_server.log
    exit 1
fi

echo -e "${GREEN}✅ HTTP server running (PID $HTTP_PID)${NC}"
echo ""

# Step 3: Start nak relay
echo "📡 Starting local Nostr relay..."
nak serve :7777 > /tmp/nak_relay.log 2>&1 &
RELAY_PID=$!

sleep 2

echo -e "${GREEN}✅ Nostr relay running on ws://localhost:7777 (PID $RELAY_PID)${NC}"
echo ""

# Step 4: Publish NNS event
echo "📝 Publishing NNS event to local relay..."
NSEC=$(nak key generate)
NPUB=$(nak key public "$NSEC")

echo "  Generated keypair for test"
echo "  Public key: $NPUB"
echo ""

# Publish event mapping testsite → 127.0.0.1:18080
EVENT_JSON=$(nak event --kind 34256 \
    --tag "d=testsite" \
    --tag "ip=127.0.0.1:18080" \
    --content "" \
    --sec "$NSEC")

echo "$EVENT_JSON" | nak relay publish ws://localhost:7777

sleep 1

EVENT_ID=$(echo "$EVENT_JSON" | jq -r '.id')
echo -e "${GREEN}✅ Published NNS event${NC}"
echo "  Event ID: $EVENT_ID"
echo "  Mapping: testsite → 127.0.0.1:18080"
echo ""

# Step 5: Run the tests
echo "🧪 Running test suite with fixtures..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Run only the ignored tests (those that need fixtures)
cargo test --test nns_with_fixtures_test -- --ignored --nocapture

TEST_EXIT_CODE=$?

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✅ ALL TESTS PASSED${NC}"
else
    echo -e "${RED}❌ TESTS FAILED${NC}"
    echo "Check logs:"
    echo "  HTTP server: /tmp/http_server.log"
    echo "  Nostr relay: /tmp/nak_relay.log"
fi

exit $TEST_EXIT_CODE
