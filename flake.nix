{
  description = "Frontier Browser - A modern web browser with Nostr integration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # Use rust-overlay for latest stable Rust
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Source filtering - include Rust sources, Cargo files, and assets
        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (pkgs.lib.hasSuffix "\.rs" path) ||
            (pkgs.lib.hasSuffix "\.toml" path) ||
            (pkgs.lib.hasSuffix "\.lock" path) ||
            (pkgs.lib.hasInfix "/assets" path) ||
            (craneLib.filterCargoSources path type);
        };

        # Common arguments for all crane builds
        commonArgs = {
          inherit src;
          pname = "frontier";
          version = "0.1.0";

          nativeBuildInputs = with pkgs; [
            pkg-config
            python3  # Required for stylo build.rs
          ];

          buildInputs = with pkgs; [
            dav1d
            openssl
          ] ++ pkgs.lib.optionals stdenv.isDarwin [
            apple-sdk
            libiconv
          ] ++ pkgs.lib.optionals stdenv.isLinux [
            libxkbcommon
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
            vulkan-loader
            xdotool
          ];

          # Disable incremental builds for Nix
          CARGO_BUILD_INCREMENTAL = "false";
        };

        # Build dependencies only (for caching)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the main package
        frontier = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false;  # Run tests separately
        });

        # Run offline tests
        frontier-tests = craneLib.cargoTest (commonArgs // {
          inherit cargoArtifacts;
          cargoTestArgs = "--test offline_test";
        });

        ciScript = pkgs.writeShellApplication {
          name = "frontier-ci";
          runtimeInputs = [
            pkgs.nix
            pkgs.bash
          ];
          text = ''
            exec nix develop .#default --command ${pkgs.bash}/bin/bash ${./scripts/ci.sh}
          '';
        };

      in
      {
        # Packages
        packages = {
          default = frontier;
          frontier = frontier;
          ci = ciScript;
        };

        # Checks run by `nix flake check`
        checks = {
          inherit frontier frontier-tests;

          # Formatting check
          frontier-fmt = craneLib.cargoFmt {
            inherit src;
          };

          # Clippy linting
          frontier-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });
        };

        # Development shell using craneLib.devShell pattern
        devShells.default = craneLib.devShell {
          inputsFrom = [ frontier ];

          packages = with pkgs; [
            just
            cargo-watch
            cargo-nextest
            git
          ];

          shellHook = ''
            export RUST_BACKTRACE=1
            if command -v just >/dev/null 2>&1; then
              echo "Available just recipes:"
              just --list
            else
              echo "Install 'just' to list available recipes."
            fi
          '';
        };

        # App for `nix run`
        apps.default = flake-utils.lib.mkApp {
          drv = frontier;
          name = "frontier";
        };

        apps.ci = flake-utils.lib.mkApp {
          drv = ciScript;
          name = "frontier-ci";
        };
      }
    );
}
