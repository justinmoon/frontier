#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${FRONTIER_CI_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)}"

if [[ ! -f "$ROOT_DIR/Cargo.toml" ]]; then
  printf 'Unable to locate project root (missing Cargo.toml in %s)\n' "$ROOT_DIR" >&2
  exit 1
fi

cd "$ROOT_DIR"

run_step() {
  local description="$1"
  shift
  printf '\n=== %s ===\n' "$description"
  "$@"
}

run_step "Checking formatting" cargo fmt --all -- --check
run_step "Checking build" cargo check --workspace --all-targets
run_step "Running clippy" cargo clippy --all-targets --workspace -- -D warnings
run_step "Running test suite" cargo test
run_step "Running curated WPT slice" just wpt
run_step "Running online tests" cargo test --test online_test -- --ignored

printf '\nCI pipeline completed successfully.\n'
