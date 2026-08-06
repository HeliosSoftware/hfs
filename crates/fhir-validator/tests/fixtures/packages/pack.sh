#!/usr/bin/env bash
# Rebuild sample.tgz from the expanded sample/ directory.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
SAMPLE="$ROOT/sample"
OUT="$ROOT/sample.tgz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/package"
cp "$SAMPLE"/* "$tmp/package/"

# Portable tar: GNU and BSD both accept -C + relative paths.
tar -czf "$OUT" -C "$tmp" package
echo "wrote $OUT ($(wc -c <"$OUT") bytes)"
