#!/usr/bin/env bash
# Stages the Docker build context for the bulk-submit compose stack.
#
# The root Dockerfile is runtime-only: it expects the compiled binary (and, for
# the hfs image, the data/ directory of search parameters) to already be present
# at the root of the build context. CI does this in its "Prepare Docker build
# context" step; this script is the local equivalent.
#
# The compose stack runs the postgres primary store with S3 (MinIO) status
# artifacts, so the binary must carry those features — the default set
# (R4, sqlite, ui) makes the container exit with "The postgres backend requires
# the 'postgres' feature".
#
#     cargo build --release -p helios-hfs --features postgres,s3
#     ./docker/bulk-submit/stage-context.sh
#     docker compose -f docker/bulk-submit/docker-compose.yml up --build
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
binary="$repo_root/target/release/hfs"
context="$here/context"

if [ ! -x "$binary" ]; then
  echo "error: $binary not found — run" >&2
  echo "  cargo build --release -p helios-hfs --features postgres,s3" >&2
  exit 1
fi

rm -rf "$context"
mkdir -p "$context"
cp "$binary" "$context/hfs"
# include_data: true for the hfs image (search-parameter definitions).
cp -r "$repo_root/data" "$context/data"

echo "staged $context ($(du -sh "$context" | cut -f1))"
