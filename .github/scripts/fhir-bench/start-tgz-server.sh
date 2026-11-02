#!/usr/bin/env bash
#
# Start the tgz bundle server for one benchmark leg, serving the Synthea
# `bulk_1k` import corpus from a persistent cache on the Docker host.
#
# Called from: the `benchmark` job's "Build & start tgz bundle server" step.
#
# Why a cache: the server (the benchmark repo's infra/tgz Go program) used to
# download the 418 MB corpus from GCS into its own anonymous volume on every
# leg, and the workflow allowed 120 s for that. Legs run concurrently on the
# same Docker host, so on run 41506467762 the sqlite and postgres legs
# timed out while their downloads were still in progress and mongodb's
# finished in 44 s.
#
# The server already skips its download when `.current` in its working
# directory holds the requested URL, and then only reads from that directory.
# So:
#
#   1. The corpus is extracted once into the named volume $CACHE_VOLUME, in the
#      directory /cache/<key>. <key> comes from the URL plus the object's ETag,
#      so a corpus that changes upstream gets a new directory and is downloaded
#      again.
#   2. On a cache miss a "seed" container runs the server with its working
#      directory set to a per-leg staging directory. Once `.current` appears,
#      the staging directory is renamed into place with `mv -T`. That rename is
#      atomic and fails when the target already exists (another leg won), so a
#      complete cache directory is never replaced or removed while a leg could
#      be serving from it. Concurrent misses (e.g. the first run after a corpus
#      change) each download; only one copy is kept.
#   3. The leg's server mounts the volume READ-ONLY with its working directory
#      set to /cache/<key>. It finds `.current`, skips the download, and is
#      ready within seconds.
#
# The cache volume is deliberately NOT labelled hfs-bench=1: the start-of-job
# volume reaper removes hfs-bench=1 volumes after LEG_TIMEOUT_MIN + 30 minutes,
# and the end-of-job leak check counts volumes by hfs-bench-run/hfs-bench-leg.
# It carries hfs-bench-cache=1 instead. docker-host-gc only sweeps anonymous
# (64-hex) volumes, so it never touches it either.
#
# Every container this script starts has an `hfs-bench-` name and the usual
# hfs-bench / hfs-ci labels, so the existing reapers cover a killed runner.
#
# Required environment:
#   BACKEND          matrix.backend
#   RUN_ID           github.run_id
#   DOCKER_HOST_IP   set in $GITHUB_ENV by "Resolve Docker host and runner IPs"
#   LEG_TIMEOUT_MIN  setup's leg_timeout_min (age guard for pruning old cache dirs)
#   GITHUB_ENV
#
# Optional environment:
#   CORPUS_URL             default: the upstream bulk_1k.tar.gz
#   TGZ_CACHE_VOLUME       default: hfs-bench-corpus-cache
#   TGZ_SEED_TIMEOUT_S     how long a cache-miss download may take (default 900)
#   TGZ_READY_TIMEOUT_S    how long the server may take to answer (default 120)
#
# Writes to $GITHUB_ENV: TGZ_CONTAINER, TGZ_SEED_CONTAINER, BUNDLE_URL.

# The cache_sh snippets are single-quoted on purpose: they expand inside the
# helper container, where the values arrive via -e, not in this shell.
# shellcheck disable=SC2016

set -euo pipefail

: "${BACKEND:?}" "${RUN_ID:?}" "${DOCKER_HOST_IP:?}" "${GITHUB_ENV:?}"
case "${LEG_TIMEOUT_MIN:-}" in
  ''|*[!0-9]*) echo "::error::LEG_TIMEOUT_MIN='${LEG_TIMEOUT_MIN:-}' is not an integer"; exit 1 ;;
esac

CORPUS_URL=${CORPUS_URL:-https://storage.googleapis.com/aidbox-public/synthea/performance/bulk_1k.tar.gz}
CACHE_VOLUME=${TGZ_CACHE_VOLUME:-hfs-bench-corpus-cache}
SEED_TIMEOUT_S=${TGZ_SEED_TIMEOUT_S:-900}
READY_TIMEOUT_S=${TGZ_READY_TIMEOUT_S:-120}

TGZ_IMAGE="hfs-bench-tgz:$RUN_ID"
TGZ_CONTAINER="hfs-bench-tgz-$BACKEND-$RUN_ID"
SEED_CONTAINER="hfs-bench-tgzseed-$BACKEND-$RUN_ID"
HELPER_CONTAINER="hfs-bench-tgzcache-$BACKEND-$RUN_ID"
STAGING_DIR="/cache/.staging-$RUN_ID-$BACKEND"
# Same age guard as the start-of-job reaper: anything older is from a dead run.
MAX_AGE_MIN=$(( LEG_TIMEOUT_MIN + 30 ))

echo "TGZ_CONTAINER=$TGZ_CONTAINER" >> "$GITHUB_ENV"
echo "TGZ_SEED_CONTAINER=$SEED_CONTAINER" >> "$GITHUB_ENV"

LABELS=(
  --label hfs-bench=1
  --label hfs-ci=true
  --label "hfs-bench-run=$RUN_ID"
  --label "hfs-bench-leg=$BACKEND"
)

# Run a shell snippet in a throwaway container that has the cache volume at
# /cache. The tgz image is golang:<ver> based, so sh + GNU coreutils/findutils
# are available. Values go in via -e, never spliced into the script text.
cache_sh() {
  local script=$1; shift
  docker rm -fv "$HELPER_CONTAINER" >/dev/null 2>&1 || true
  timeout 300 docker run --rm --name "$HELPER_CONTAINER" "${LABELS[@]}" \
    --log-driver local \
    -v "$CACHE_VOLUME:/cache" \
    -e CORPUS_URL="$CORPUS_URL" -e KEY="${KEY:-}" -e STAGING_DIR="$STAGING_DIR" \
    -e MAX_AGE_MIN="$MAX_AGE_MIN" "$@" \
    --entrypoint sh "$TGZ_IMAGE" -c "$script"
}

docker build -t "$TGZ_IMAGE" fhir-benchmark/infra/tgz

# ── Cache key ────────────────────────────────────────────────────────────────
# URL hash + ETag. A HEAD that fails or has no ETag only costs a re-download
# into a URL-only key; it never serves a mismatched corpus under the ETag key.
URL_HASH=$(printf '%s' "$CORPUS_URL" | sha256sum | cut -c1-16)
ETAG=$(curl -sfIL --max-time 30 "$CORPUS_URL" 2>/dev/null \
  | tr -d '\r' | awk 'tolower($1)=="etag:" {v=$2} END {print v}' \
  | tr -cd 'A-Za-z0-9' | cut -c1-40) || ETAG=""
if [ -n "$ETAG" ]; then
  KEY="$URL_HASH-$ETAG"
else
  echo "::warning::Could not read the corpus ETag; caching by URL only (a changed upstream corpus would not be noticed)"
  KEY="$URL_HASH-noetag"
fi
echo "Corpus: $CORPUS_URL"
echo "Cache:  volume $CACHE_VOLUME, dir /cache/$KEY"

docker volume create --label hfs-bench-cache=1 "$CACHE_VOLUME" >/dev/null

# Prune what dead runs left behind: superseded cache dirs and staging dirs.
# The age guard keeps a dir a live leg of another run might still be using.
cache_sh 'find /cache -mindepth 1 -maxdepth 1 ! -name "$KEY" -mmin "+$MAX_AGE_MIN" -print -exec rm -rf {} +' \
  | sed 's/^/  pruned /' || echo "::warning::Pruning old corpus cache dirs failed; continuing"

cache_is_complete() {
  cache_sh '[ -f "/cache/$KEY/.current" ] && [ "$(cat "/cache/$KEY/.current")" = "$CORPUS_URL" ]'
}

# ── Seed on a miss ───────────────────────────────────────────────────────────
if cache_is_complete; then
  echo "Corpus cache hit."
else
  echo "Corpus cache miss; downloading (up to ${SEED_TIMEOUT_S}s)..."
  cache_sh 'rm -rf "$STAGING_DIR"'
  docker rm -fv "$SEED_CONTAINER" >/dev/null 2>&1 || true
  # `-w` creates the staging directory; the server downloads into its cwd.
  docker run -d --name "$SEED_CONTAINER" "${LABELS[@]}" \
    --log-driver local \
    -v "$CACHE_VOLUME:/cache" -w "$STAGING_DIR" \
    "$TGZ_IMAGE" "$CORPUS_URL" >/dev/null

  SEED_START=$(date +%s)
  while :; do
    if timeout 30 docker exec "$SEED_CONTAINER" test -f "$STAGING_DIR/.current" 2>/dev/null; then
      echo "Corpus downloaded in $(( $(date +%s) - SEED_START ))s."
      break
    fi
    running=$(timeout 30 docker inspect -f '{{.State.Running}}' "$SEED_CONTAINER" 2>/dev/null || echo unknown)
    if [ "$running" = "false" ]; then
      echo "::error::Corpus download container exited before finishing"
      timeout 60 docker logs --tail 50 "$SEED_CONTAINER" 2>&1 || true
      exit 1
    fi
    if [ $(( $(date +%s) - SEED_START )) -ge "$SEED_TIMEOUT_S" ]; then
      echo "::error::Corpus download did not finish within ${SEED_TIMEOUT_S}s"
      timeout 60 docker logs --tail 50 "$SEED_CONTAINER" 2>&1 || true
      exit 1
    fi
    sleep 5
  done

  # Promote before removing the seed container (its cwd moving is harmless).
  # `mv -T` fails if another leg already promoted the same key; then ours is
  # a duplicate and goes.
  cache_sh 'if mv -T "$STAGING_DIR" "/cache/$KEY" 2>/dev/null; then echo "Corpus cached."; else echo "Another leg cached this corpus first; discarding ours."; rm -rf "$STAGING_DIR"; fi'
  docker rm -fv "$SEED_CONTAINER" >/dev/null 2>&1 || true

  if ! cache_is_complete; then
    echo "::error::Corpus cache /cache/$KEY is not complete after seeding"
    exit 1
  fi
fi

# ── Serve ────────────────────────────────────────────────────────────────────
docker rm -fv "$TGZ_CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$TGZ_CONTAINER" "${LABELS[@]}" \
  --log-driver local \
  -v "$CACHE_VOLUME:/cache:ro" -w "/cache/$KEY" \
  -p 0:8080 "$TGZ_IMAGE" "$CORPUS_URL" >/dev/null

echo "Waiting for tgz bundle server..."
READY_START=$(date +%s)
TGZ_PORT=""
while :; do
  TGZ_PORT=$(timeout 30 docker port "$TGZ_CONTAINER" 8080 2>/dev/null | head -1 | sed 's/.*://')
  if [ -n "$TGZ_PORT" ] && curl -sf --max-time 10 "http://$DOCKER_HOST_IP:$TGZ_PORT/hospitalInformation.json" -o /dev/null 2>/dev/null; then
    echo "tgz ready on $DOCKER_HOST_IP:$TGZ_PORT after $(( $(date +%s) - READY_START ))s"
    break
  fi
  running=$(timeout 30 docker inspect -f '{{.State.Running}}' "$TGZ_CONTAINER" 2>/dev/null || echo unknown)
  if [ "$running" = "false" ]; then
    echo "::error::tgz bundle server exited before becoming ready"
    timeout 60 docker logs --tail 50 "$TGZ_CONTAINER" 2>&1 || true
    exit 1
  fi
  if [ $(( $(date +%s) - READY_START )) -ge "$READY_TIMEOUT_S" ]; then
    echo "::error::tgz bundle server did not become ready within ${READY_TIMEOUT_S}s (serving from cache, so this is not a download)"
    timeout 60 docker logs --tail 50 "$TGZ_CONTAINER" 2>&1 || true
    exit 1
  fi
  sleep 2
done

echo "BUNDLE_URL=http://$DOCKER_HOST_IP:$TGZ_PORT" >> "$GITHUB_ENV"
