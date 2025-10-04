# Run the browser
run *ARGS:
    cargo run -- {{ARGS}}

# Run all tests
test:
    cargo test
    cargo test --test online_test -- --ignored

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
