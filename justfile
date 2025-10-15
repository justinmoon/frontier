# List available recipes when running `just` with no arguments
default:
    @just --list

# Run the browser
run *ARGS:
    cargo run -- {{ARGS}}

# Run CI checks
ci:
    nix run .#ci

# Launch the React micro-demos landing page inside Frontier.
# The browser runs interactively but exits automatically after 30s so CI doesn't hang.
react-demos:
    bash -lc 'timeout 30 cargo run --bin frontier -- "file://$(pwd)/assets/react-demos/index.html"; code=$?; if [ $code -eq 124 ]; then echo "frontier exited after 30s timeout"; exit 0; else exit $code; fi'

# Run all tests (or specific test with args)
test *ARGS:
    @if [ -z "{{ARGS}}" ]; then \
        cargo test && cargo test --test online_test -- --ignored; \
    else \
        cargo test {{ARGS}}; \
    fi

# Run offline tests (fast, no network required)
[group('test')]
offline:
    cargo test --test offline_test

# Run online tests (requires network)
[group('test')]
online:
    cargo test --test online_test -- --ignored

# Run curated WPT slice
[group('test')]
wpt:
    cargo test --test wpt_smoke -- --nocapture

# Run full WPT timer suite and report coverage
[group('test')]
wpt-full:
    cargo test --test wpt_full -- --ignored --nocapture

# Build the browser
build:
    cargo build

# Build release version
build-release:
    cargo build --release

# Clean build artifacts
clean:
    cargo clean
