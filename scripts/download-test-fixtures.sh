#!/usr/bin/env bash
set -euo pipefail

# Placeholder script for downloading test WAV fixtures.
# Will be populated when audio processing tests are added.

FIXTURES_DIR="$(dirname "$0")/../tests/fixtures"
mkdir -p "$FIXTURES_DIR"

echo "Test fixtures directory ready at: $FIXTURES_DIR"
echo "No fixtures to download yet."
