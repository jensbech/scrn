#!/usr/bin/env just --justfile

GITHUB_REPO := "jensbech/scrn"
TARGETS := "arm intel linux-x64 linux-arm"
BUILD_SCRIPT := "https://github.com/jensbech/rust-build-tools/releases/latest/download/rust-build"

# Default recipe
default:
    @just --list

# Run centralized build script (local sibling or remote fallback)
[private]
_run *ARGS:
    #!/usr/bin/env bash
    set -e
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    if [ -x "../rust-build-tools/rust-build" ]; then
        ../rust-build-tools/rust-build {{ARGS}}
    else
        SCRIPT=$(mktemp)
        trap 'rm -f "$SCRIPT"' EXIT
        curl -fsSL "{{BUILD_SCRIPT}}" -o "$SCRIPT"
        bash "$SCRIPT" {{ARGS}}
    fi

# Install cross-compilation toolchain and targets
setup: (_run "setup")

# Build release binary for current architecture
build: (_run "build")

# Build debug version (faster for development)
build-dev: (_run "build-dev")

# Run tests
test: (_run "test")

# Format and lint
lint: (_run "lint")

# Clean build artifacts
clean: (_run "clean")

# Print version from Cargo.toml
version: (_run "version")

# Bump version, build targets, and publish to GitHub
release: (_run "release" GITHUB_REPO TARGETS)
