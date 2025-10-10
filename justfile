# Run the browser
run *ARGS:
    cargo run -- {{ARGS}}

# Run CI checks
ci:
    nix run .#ci

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

# Build the browser
build:
    cargo build

# Build release version
build-release:
    cargo build --release

# Clean build artifacts
clean:
    cargo clean
