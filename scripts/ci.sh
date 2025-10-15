#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${FRONTIER_CI_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)}"

if [[ ! -f "$ROOT_DIR/Cargo.toml" ]]; then
  printf 'Unable to locate project root (missing Cargo.toml in %s)\n' "$ROOT_DIR" >&2
  exit 1
fi

cd "$ROOT_DIR"

# Ensure a display server is available for winit-based automation tests.
# Lazily spin up a virtual display on headless Linux CI runners.
if [[ "$(uname -s)" == "Linux" && -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
  cleanup_display() {
    if [[ -n "${FRONTIER_XVFB_PID:-}" ]]; then
      kill "$FRONTIER_XVFB_PID" >/dev/null 2>&1 || true
    fi
  }
  trap cleanup_display EXIT

  if command -v Xvfb >/dev/null 2>&1; then
    export DISPLAY=":99"
    Xvfb "$DISPLAY" -screen 0 1280x720x24 >/dev/null 2>&1 &
    FRONTIER_XVFB_PID=$!
    # Give Xvfb a moment to start
    sleep 1
    printf 'Started Xvfb on DISPLAY=%s\n' "$DISPLAY"
  else
    printf 'Xvfb not found and no display available; automation tests require a display\n' >&2
    exit 1
  fi
fi

run_step() {
  local description="$1"
  shift
  printf '\n=== %s ===\n' "$description"
  "$@"
}

run_step "Updating submodules" git submodule update --init --recursive
run_step "Checking formatting" cargo fmt --all -- --check
run_step "Checking build" cargo check --workspace --all-targets
run_step "Running clippy" cargo clippy --all-targets --workspace -- -D warnings
run_step "Running test suite" cargo test
run_step "Running online tests" cargo test --test online_test -- --ignored

printf '\nCI pipeline completed successfully.\n'
