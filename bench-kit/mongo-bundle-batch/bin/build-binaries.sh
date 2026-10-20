#!/usr/bin/env bash
# Build one hfs release binary per labelled commit, each in its own detached git worktree under ./work/.
# Usage: bin/build-binaries.sh <repo-dir> <label>=<commit-ish> [<label>=<commit-ish> ...]
#   e.g. bin/build-binaries.sh ~/src/hfs main=678fc5061 ours=abc74288e
#        git -C ~/src/hfs fetch origin pull/2144/head:pr-2144 && bin/build-binaries.sh ~/src/hfs pr2144=pr-2144
# Output: ./bins/hfs-<label>   (the driver's --arm label=bins/hfs-<label>)
# Features default to the ones of the original runs (R4,sqlite,postgres,mongodb,elasticsearch); override with FEATURES=...
# ("R4,mongodb" builds faster and runs the same Mongo code path; keep it identical for every arm).
set -euo pipefail
[ $# -ge 2 ] || { sed -n '2,8p' "$0"; exit 2; }
REPO=$(readlink -f "$1"); shift
HERE=$(cd "$(dirname "$0")/.." && pwd); FEATURES=${FEATURES:-R4,sqlite,postgres,mongodb,elasticsearch}
mkdir -p "$HERE/work" "$HERE/bins"
for spec in "$@"; do
  L=${spec%%=*}; REF=${spec#*=}; [[ $L =~ ^[A-Za-z0-9._-]+$ ]] || { echo "bad label $L" >&2; exit 2; }
  SHA=$(git -C "$REPO" rev-parse --verify "$REF^{commit}")
  W="$HERE/work/$L"
  if [ -d "$W" ]; then git -C "$W" checkout -q --detach "$SHA"; else git -C "$REPO" worktree add --detach "$W" "$SHA" >/dev/null; fi
  echo "== building $L @ $SHA with features $FEATURES"
  # a shared target dir across arms is NOT used on purpose: each arm builds from its own tree (slower, but no cross-contamination)
  (cd "$W" && cargo build --release -p helios-hfs --features "$FEATURES")
  cp -f "$W/target/release/hfs" "$HERE/bins/hfs-$L"; echo "$L $SHA $FEATURES" >> "$HERE/bins/builds.txt"
  echo "== $HERE/bins/hfs-$L"
done
