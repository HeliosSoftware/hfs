#!/bin/bash
cd /home/angela/src/hfs; export CARGO_BUILD_JOBS=1
echo "=== build start $(date -u +%FT%TZ) $(git rev-parse --short HEAD)"
/usr/bin/time -v cargo build --workspace --all-features --release 2>&1 | tail -40
echo "=== build end $(date -u +%FT%TZ) exit ${PIPESTATUS[0]}"
