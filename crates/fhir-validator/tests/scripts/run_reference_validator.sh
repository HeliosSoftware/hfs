#!/usr/bin/env bash
#
# Differential-testing spike driver (issue #427), phase 1.
#
# Provisions the HL7 reference validator (`validator_cli.jar` — the cheap path
# proven by hts-ig-conformance.yml, NOT the Inferno compose validator) and runs
# it over the SAME deterministic sample of the vendored FHIR example corpus that
# `tests/differential.rs` runs our engine over. Emits an intermediate JSON the
# Rust side consumes:
#
#   target/differential/<version>.reference.json
#     { "version": "R4",
#       "results": [ { "file": "...", "wallMs": 1234,
#                      "issues": [ {"severity","code","expression"} ... ] } ] }
#
# Per-file invocation is deliberate: it yields the honest "wall-clock per
# resource" number #427 asks for (JVM start included). Batch/server mode is the
# obvious optimization and is called out as phase 2 — we measure the worst case
# first, then decide whether it needs improving.
#
# Usage:  run_reference_validator.sh <R4|R4B|R5> [sample_size]
# Env:    DIFFERENTIAL_SAMPLE_SIZE (default 50; CLI arg overrides), VALIDATOR_JAR
#
# Requires: java (21+), jq, curl. In CI these come from setup-java@v5 + the
# self-hosted runner image (same as validator-conformance.yml / hts-ig).

set -euo pipefail

VERSION="${1:?usage: run_reference_validator.sh <R4|R4B|R5> [sample_size]}"
SAMPLE_SIZE="${2:-${DIFFERENTIAL_SAMPLE_SIZE:-50}}"

case "$VERSION" in
  R4)  FHIR_VERSION="4.0.1" ;;
  R4B) FHIR_VERSION="4.3.0" ;;
  R5)  FHIR_VERSION="5.0.0" ;;
  *) echo "::error::unsupported version '$VERSION' (expected R4|R4B|R5)"; exit 2 ;;
esac

# Repo-root-relative paths (this script lives at
# crates/fhir-validator/tests/scripts/).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
CORPUS_DIR="$REPO_ROOT/crates/fhir/tests/data/json/$VERSION"
OUT_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/differential"
OUT_FILE="$OUT_DIR/$(echo "$VERSION" | tr '[:upper:]' '[:lower:]').reference.json"

for tool in java jq curl; do
  command -v "$tool" >/dev/null 2>&1 || { echo "::error::missing required tool: $tool"; exit 3; }
done

if [ ! -d "$CORPUS_DIR" ]; then
  echo "::error::[CORPUS-MISSING] $CORPUS_DIR does not exist. It is vendored; a missing directory is a checkout problem."
  exit 4
fi

mkdir -p "$OUT_DIR"

# ── Provision validator_cli.jar (latest), unless one is supplied ────────────
JAR="${VALIDATOR_JAR:-$OUT_DIR/validator_cli.jar}"
if [ ! -f "$JAR" ]; then
  echo "Downloading validator_cli.jar (latest) -> $JAR"
  curl -fsSL --max-time 600 \
    https://github.com/hapifhir/org.hl7.fhir.core/releases/latest/download/validator_cli.jar \
    -o "$JAR"
fi
ls -lh "$JAR"

# ── Deterministic sample: MUST match tests/differential.rs::sample_files ─────
# Rust sorts PathBufs bytewise (== LC_ALL=C sort), filters to files that parse
# as a FHIR resource (top-level "resourceType"), takes the first N. Mirror that
# exactly, or the two halves diff different resources.
mapfile -t ALL < <(cd "$CORPUS_DIR" && ls -1 *.json 2>/dev/null | LC_ALL=C sort)
SAMPLE=()
for f in "${ALL[@]}"; do
  [ "${#SAMPLE[@]}" -ge "$SAMPLE_SIZE" ] && break
  # `has("resourceType")` — jq -e exits non-zero if false/parse-fails.
  if jq -e 'type == "object" and has("resourceType")' "$CORPUS_DIR/$f" >/dev/null 2>&1; then
    SAMPLE+=("$f")
  fi
done

if [ "${#SAMPLE[@]}" -eq 0 ]; then
  echo "::error::[NO-SAMPLE] no resources sampled from $CORPUS_DIR; refusing to emit an empty run."
  exit 5
fi
echo "Sampled ${#SAMPLE[@]} of requested $SAMPLE_SIZE resources from $VERSION"

# ── Run the validator per file, timing each, collecting normalized issues ───
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# jq filter: OperationOutcome issues -> {severity, code, expression}. The
# reference validator emits `expression` (FHIRPath); older builds use
# `location`. Prefer the first of either. A run that produced no OperationOutcome
# (parse failure) yields an empty issue list for that file.
ISSUE_FILTER='
  if type=="object" and .resourceType=="OperationOutcome" then
    [ .issue[]? | {
        severity: (.severity // ""),
        code: (.code // ""),
        expression: ((.expression // .location // [""])[0] // "")
      } ]
  else [] end'

RESULTS="$TMP/results.jsonl"
: > "$RESULTS"

i=0
for f in "${SAMPLE[@]}"; do
  i=$((i+1))
  oo="$TMP/oo.json"
  rm -f "$oo"
  start_ns=$(date +%s%N)
  # -output writes the OperationOutcome; failures still produce one. Never abort
  # the sweep on a single file's non-zero exit — capture whatever it emitted.
  java -jar "$JAR" "$CORPUS_DIR/$f" -version "$FHIR_VERSION" -output "$oo" \
    >/dev/null 2>"$TMP/stderr.log" || true
  end_ns=$(date +%s%N)
  wall_ms=$(( (end_ns - start_ns) / 1000000 ))

  if [ -f "$oo" ]; then
    issues=$(jq -c "$ISSUE_FILTER" "$oo" 2>/dev/null || echo '[]')
  else
    issues='[]'
  fi
  jq -cn --arg file "$f" --argjson wallMs "$wall_ms" --argjson issues "$issues" \
    '{file:$file, wallMs:$wallMs, issues:$issues}' >> "$RESULTS"
  echo "  [$i/${#SAMPLE[@]}] $f -> ${wall_ms}ms, $(echo "$issues" | jq 'length') issue(s)"
done

# Assemble the final document.
jq -s --arg version "$VERSION" '{version:$version, results:.}' "$RESULTS" > "$OUT_FILE"

total_ms=$(jq '[.results[].wallMs] | add // 0' "$OUT_FILE")
n=$(jq '.results | length' "$OUT_FILE")
mean=$(awk -v t="$total_ms" -v n="$n" 'BEGIN{ if (n>0) printf "%.1f", t/n; else print "0" }')
echo "Wrote $OUT_FILE"
echo "$VERSION: $n resources, ${total_ms}ms total, ${mean}ms/resource (reference validator, JVM start incl.)"
