#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load notarization credentials from .env
if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  source "$PROJECT_ROOT/.env"
  set +a
  echo "=== Loaded .env ==="
else
  echo "WARNING: No .env file found — notarization will be skipped"
fi

echo "=== Building sidecars (release) ==="
"$SCRIPT_DIR/build-sidecars.sh"

echo ""
echo "=== Building DMG ==="
cd "$PROJECT_ROOT/apps/desktop"
pnpm tauri build --bundles dmg

echo ""
echo "=== Done ==="
DMG_PATH="$PROJECT_ROOT/target/release/bundle/dmg"
echo "DMG location:"
ls -lh "$DMG_PATH"/*.dmg 2>/dev/null || echo "Check target/release/bundle/dmg/ for the output"
