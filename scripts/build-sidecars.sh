#!/usr/bin/env bash
set -euo pipefail

# Build all sidecar workers for one or more target triples.
#
# Today this only fans out to the transcription sidecar; the embedding
# sidecar will be added here once it lands.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

"$SCRIPT_DIR/build-transcription-sidecar.sh" "$@"
