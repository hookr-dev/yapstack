#!/usr/bin/env bash
set -euo pipefail

# Build all sidecar workers for one or more target triples.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

"$SCRIPT_DIR/build-transcription-sidecar.sh" "$@"
"$SCRIPT_DIR/build-embedding-sidecar.sh" "$@"
