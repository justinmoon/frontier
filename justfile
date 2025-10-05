# Run the browser
run *ARGS:
    cargo run -- {{ARGS}}

# Run CI checks
ci:
    nix run .#ci

# Run all tests
test:
    cargo test
    cargo test --test online_test -- --ignored

# Run NNS E2E test (includes fixtures)
[group('test')]
nns-e2e:
    cargo test --test nns_e2e_test -- --nocapture

# Run NNS E2E test with manual browser (starts fixtures and launches browser)
[group('test')]
nns-manual:
    ./scripts/test_nns_full_e2e.sh

# Run offline tests (fast, no network required)
[group('test')]
offline:
    cargo test --test offline_test

# Run online tests (requires network)
[group('test')]
online:
    cargo test --test online_test -- --ignored

# Build the browser
build:
    cargo build

# Build release version
build-release:
    cargo build --release

# Clean build artifacts
clean:
    cargo clean
