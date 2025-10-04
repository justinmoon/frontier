# Nix Flake Setup

This project includes a Nix flake for reproducible development environments and pure builds using the Crane library.

## Quick Start

### Development Shell

Enter the development shell:
```bash
nix develop
```

From within the shell, you can use all standard commands:
```bash
just build        # Build the project
just test         # Run tests
just run          # Run the browser
cargo build       # Direct cargo commands also work
```

### Pure Nix Build

Build the project in a pure, reproducible environment:
```bash
nix build
```

The binary will be available at `./result/bin/frontier`.

Run directly without building:
```bash
nix run
```

### Running Tests and Checks

Run all checks (build, tests, clippy, formatting):
```bash
nix flake check
```

## What's Included

The Nix flake provides:

**Development Shell:**
- Rust toolchain (stable, latest)
- `cargo`, `rustc`, `rust-analyzer`
- `just` command runner
- `cargo-watch` for file watching
- `cargo-nextest` for improved test running
- All necessary system dependencies (OpenSSL, Python3, macOS frameworks, etc.)

**Pure Builds:**
- Reproducible builds using Crane
- Cached dependency builds for faster iteration
- Automatic formatting checks
- Clippy linting
- Offline test execution

## Dependencies

This project uses git dependencies for the Blitz browser engine packages:
- All blitz packages are fetched from `https://github.com/DioxusLabs/blitz.git`
- Pinned to a specific revision for reproducibility
- No local path dependencies required

## Files Added

- `flake.nix` - Nix flake configuration using the Crane library for Rust
- `flake.lock` - Lock file for reproducible flake inputs
- `Cargo.lock` - Cargo lock file (required by Nix/Crane, removed from `.gitignore`)
- `Cargo.toml` - Updated to use git dependencies instead of local paths
