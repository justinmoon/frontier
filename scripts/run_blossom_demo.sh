#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
cd "$ROOT_DIR"

INITIAL_URL=${1:-https://example.com}

echo "Building Frontier demo binaries..."
cargo build --bin frontier --bin blossom_demo > /dev/null

echo
cargo run --bin blossom_demo -- "$INITIAL_URL"
