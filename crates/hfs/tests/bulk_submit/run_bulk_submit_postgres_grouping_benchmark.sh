#!/usr/bin/env bash
# Runs the issue #1456 mixed fresh/existing PostgreSQL benchmark.
#
# The controller requires explicit baseline/candidate binaries and source
# checkouts. It owns and removes only the PostgreSQL container, HFS processes,
# and loopback provider that it starts.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTROLLER="$SCRIPT_DIR/measure_postgres_grouping.py"

if [ "$#" -eq 0 ]; then
  cat >&2 <<'EOF'
Usage:
  run_bulk_submit_postgres_grouping_benchmark.sh \
    --baseline-binary /absolute/path/to/baseline/hfs \
    --baseline-repo /absolute/path/to/baseline/repo \
    --candidate-binary /absolute/path/to/candidate/hfs \
    --candidate-repo /absolute/path/to/candidate/repo \
    [--output-dir /tmp/hfs-1456-pgi02-r1/benchmark]

Add --dry-run to print the default 42-trial matrix without starting services.
Use --resources 100 --trials 1 for a bounded end-to-end harness check.
The baseline repo must be unchanged at the documented base commit.
EOF
  exit 2
fi

exec python3 "$CONTROLLER" "$@"
