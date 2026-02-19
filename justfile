#!/usr/bin/env just --justfile

GITHUB_REPO := "jensbech/scrn"

# Default recipe
default:
    @just --list

# Run centralized build script (local sibling or remote fallback)
[private]
_run *ARGS:
    #!/usr/bin/env bash
    set -e
    export PATH="$HOME/.cargo/bin:$PATH"
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    if [ -x "../rust-build-tools/rust-build" ]; then
        ../rust-build-tools/rust-build {{ARGS}}
    else
        SCRIPT=$(mktemp)
        trap 'rm -f "$SCRIPT"' EXIT
        curl -fsSL "https://github.com/jensbech/rust-build-tools/releases/latest/download/rust-build" -o "$SCRIPT"
        bash "$SCRIPT" {{ARGS}}
    fi

# Install cross-compilation toolchain and targets
setup: (_run "setup")

# Build release binary for current architecture
build: (_run "build")

# Build for Apple Silicon (aarch64)
build-arm: (_run "build-arm")

# Build for Intel macOS (x86_64)
build-intel: (_run "build-intel")

# Build for Linux x86_64 (static musl)
build-linux-x64: (_run "build-linux-x64")

# Build for Linux ARM64 (static musl)
build-linux-arm: (_run "build-linux-arm")

# Bump version, build all targets, and publish to GitHub
release: _bump _build-all _publish

# Prompt for version bump type and update Cargo.toml
[private]
_bump:
    #!/usr/bin/env bash
    set -e
    CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"
    echo "Current version: ${CURRENT}"
    echo ""
    echo "  1) patch  → ${MAJOR}.${MINOR}.$((PATCH+1))"
    echo "  2) minor  → ${MAJOR}.$((MINOR+1)).0"
    echo "  3) major  → $((MAJOR+1)).0.0"
    echo ""
    read -rp "Bump type [1/2/3]: " CHOICE
    case "$CHOICE" in
        1|patch) NEW="${MAJOR}.${MINOR}.$((PATCH+1))" ;;
        2|minor) NEW="${MAJOR}.$((MINOR+1)).0" ;;
        3|major) NEW="$((MAJOR+1)).0.0" ;;
        *) echo "Invalid choice"; exit 1 ;;
    esac
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" Cargo.toml
    echo "Bumped ${CURRENT} → ${NEW}"

# Build all release targets sequentially and package assets
[private]
_build-all:
    #!/usr/bin/env bash
    set -e
    # Use absolute paths to rustup's toolchain to avoid Homebrew's standalone rust
    CARGO="$(rustup which cargo)"
    export RUSTC="$(rustup which rustc)"
    NAME=$(grep '^name' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    mkdir -p release
    echo "Building all targets for ${NAME} v${VERSION}..."
    echo ""
    echo "→ aarch64-apple-darwin"
    "$CARGO" build --release --target aarch64-apple-darwin
    cp "target/aarch64-apple-darwin/release/${NAME}" "release/${NAME}-${VERSION}-aarch64-apple-darwin"
    echo ""
    echo "→ x86_64-apple-darwin"
    "$CARGO" build --release --target x86_64-apple-darwin
    cp "target/x86_64-apple-darwin/release/${NAME}" "release/${NAME}-${VERSION}-x86_64-apple-darwin"
    echo ""
    echo "→ x86_64-unknown-linux-musl"
    "$CARGO" zigbuild --release --target x86_64-unknown-linux-musl
    cp "target/x86_64-unknown-linux-musl/release/${NAME}" "release/${NAME}-${VERSION}-x86_64-unknown-linux-musl"
    echo ""
    echo "→ aarch64-unknown-linux-musl"
    "$CARGO" zigbuild --release --target aarch64-unknown-linux-musl
    cp "target/aarch64-unknown-linux-musl/release/${NAME}" "release/${NAME}-${VERSION}-aarch64-unknown-linux-musl"
    echo ""
    echo "All targets built:"
    ls -lh release/${NAME}-${VERSION}-*

# Publish release assets to GitHub
[private]
_publish:
    #!/usr/bin/env bash
    set -e
    NAME=$(grep '^name' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    TAG="v${VERSION}"
    ASSETS=(release/${NAME}-${VERSION}-*)
    if [ ${#ASSETS[@]} -eq 0 ]; then
        echo "No release assets found for ${NAME}-${VERSION}"
        exit 1
    fi
    echo "Publishing ${TAG} to GitHub (${#ASSETS[@]} assets)..."
    git tag -f "$TAG"
    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    git push origin "$BRANCH" --force
    git push origin "$TAG" --force
    # Delete any existing release so we always create fresh
    if gh release view "$TAG" &>/dev/null; then
        echo "Deleting existing release ${TAG}..."
        gh release delete "$TAG" --yes
    fi
    gh release create "$TAG" "${ASSETS[@]}" \
        --title "$TAG" \
        --notes "Release ${VERSION}" \
        --latest
    echo "Published ${TAG}"
    echo "Done: https://github.com/{{GITHUB_REPO}}/releases/tag/${TAG}"

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
