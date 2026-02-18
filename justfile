#!/usr/bin/env just --justfile

# Centralized build script (GitHub release)
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

# Build for Apple Silicon (aarch64)
build-arm: (_run "build-arm")

# Build for Intel macOS (x86_64)
build-intel: (_run "build-intel")

# Build for Linux x86_64 (static musl)
build-linux-x64: (_run "build-linux-x64")

# Build for Linux ARM64 (static musl)
build-linux-arm: (_run "build-linux-arm")

# Bump version, build all targets, and publish to GitHub
release: _bump (_run "build-arm") (_run "build-intel") (_run "build-linux-x64") (_run "build-linux-arm") _publish

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
    # Ensure github remote exists and push branch + tag
    git remote get-url github &>/dev/null || git remote add github https://github.com/jensbech/scrn.git
    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    git push github "$BRANCH" --force
    git push github "$TAG" --force
    # Delete any existing release (may be a stale draft) so we always create fresh
    if gh release view "$TAG" --repo jensbech/scrn &>/dev/null; then
        echo "Deleting existing release ${TAG}..."
        gh release delete "$TAG" --repo jensbech/scrn --yes --cleanup-tag=false
    fi
    gh release create "$TAG" "${ASSETS[@]}" \
        --repo jensbech/scrn \
        --title "$TAG" \
        --notes "Release ${VERSION}" \
        --latest
    echo "Published ${TAG}"
    echo "Done: https://github.com/jensbech/scrn/releases/tag/${TAG}"

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
