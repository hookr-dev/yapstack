#!/usr/bin/env bash
set -euo pipefail

# Build the yapstack-sidecar binary for one or more target triples.
#
# Usage:
#   ./scripts/build-sidecar.sh                  # release build for current host
#   ./scripts/build-sidecar.sh --dev             # debug build (faster, for development)
#   ./scripts/build-sidecar.sh aarch64-apple-darwin x86_64-apple-darwin
#
# The script copies the resulting binary into
#   apps/desktop/src-tauri/binaries/yapstack-sidecar-<triple>
# which is the naming convention Tauri expects for bundled sidecars.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARIES_DIR="$PROJECT_ROOT/apps/desktop/src-tauri/binaries"

RELEASE=true

# Default: detect host triple
detect_host_triple() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64) echo "aarch64-apple-darwin" ;;
                x86_64) echo "x86_64-apple-darwin" ;;
                *) echo "unknown-apple-darwin" ;;
            esac
            ;;
        Linux)
            echo "x86_64-unknown-linux-gnu"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            echo "x86_64-pc-windows-msvc"
            ;;
        *)
            echo "unknown-unknown-unknown"
            ;;
    esac
}

# Parse flags
POSITIONAL=()
for arg in "$@"; do
    case "$arg" in
        --dev) RELEASE=false ;;
        *) POSITIONAL+=("$arg") ;;
    esac
done

if [ ${#POSITIONAL[@]} -eq 0 ]; then
    TARGETS=("$(detect_host_triple)")
else
    TARGETS=("${POSITIONAL[@]}")
fi

mkdir -p "$BINARIES_DIR"

if $RELEASE; then
    PROFILE_LABEL="release"
    CARGO_FLAGS="--release"
    PROFILE_DIR="release"
else
    PROFILE_LABEL="debug (dev)"
    CARGO_FLAGS=""
    PROFILE_DIR="debug"
fi

echo "Building yapstack-sidecar with whisper feature ($PROFILE_LABEL)..."
echo "Targets: ${TARGETS[*]}"
echo ""

for target in "${TARGETS[@]}"; do
    echo "=== Building for $target ==="

    # Enable GPU acceleration per platform
    FEATURES="whisper"
    if [[ "$target" == *"apple"* ]]; then
        FEATURES="whisper,metal"
    elif [[ "$target" == *"windows"* ]]; then
        FEATURES="whisper,cuda"
    fi

    # shellcheck disable=SC2086
    cargo build \
        $CARGO_FLAGS \
        --target "$target" \
        -p yapstack-sidecar \
        --features "$FEATURES"

    # Determine source binary path
    if [[ "$target" == *"windows"* ]]; then
        src="$PROJECT_ROOT/target/$target/$PROFILE_DIR/yapstack-sidecar.exe"
        dest="$BINARIES_DIR/yapstack-sidecar-${target}.exe"
    else
        src="$PROJECT_ROOT/target/$target/$PROFILE_DIR/yapstack-sidecar"
        dest="$BINARIES_DIR/yapstack-sidecar-${target}"
    fi

    if [ ! -f "$src" ]; then
        echo "ERROR: binary not found at $src"
        exit 1
    fi

    cp "$src" "$dest"
    chmod +x "$dest"
    # Strip is also set in Cargo profile.release, but belt-and-suspenders
    if command -v strip &>/dev/null && [[ "$target" != *"windows"* ]]; then
        strip "$dest" 2>/dev/null || true
    fi
    echo "Copied to $dest"
    echo ""
done

echo "Done. Sidecar binaries are in $BINARIES_DIR"
ls -lh "$BINARIES_DIR"
