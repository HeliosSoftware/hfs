#!/usr/bin/env bash
# PostgreSQL grouped fresh-create benchmark for issue #1455.
#
# The default campaign is intentionally large: three source arms, five batch
# sizes, three fresh-database trials, plus a separate oversized correctness/RSS
# matrix. Set DRY_RUN=1, PILOT=1, or the subset variables documented below
# before spending that time.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
CONTROLLER="$ROOT/crates/hfs/tests/bulk_submit/measure_memory.py"
ISSUE_SOURCE_REF="${ISSUE_SOURCE_REF:-8b127592b30083946feb54cf530a26aebf1f3940}"
CURRENT_MAIN_REF="${CURRENT_MAIN_REF:-3c09d6a87a80a16d9cb4198fd9cafef3faea4e2e}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/target/issue-1455/$RUN_ID}"
WORKTREE_ROOT="$EVIDENCE_ROOT/worktrees"
BINARY_ROOT="$EVIDENCE_ROOT/binaries"
RAW_ROOT="$EVIDENCE_ROOT/raw"
PG_IMAGE="${PG_IMAGE:-postgres:16-alpine}"
PG_PORT_START="${PG_PORT:-19455}"
PROVIDER_PORT="${PROVIDER_PORT:-19457}"
NORMAL_RESOURCES="${NORMAL_RESOURCES:-10000}"
OVERSIZED_RESOURCES="${OVERSIZED_RESOURCES:-203}"
ARMS="${ARMS:-issue-source,current-main,candidate}"
BATCH_SIZES="${BATCH_SIZES:-100,101,128,500,1000}"
TRIALS="${TRIALS:-3}"
FIXTURE_MODES="${FIXTURE_MODES:-normal,oversized}"
DRY_RUN="${DRY_RUN:-0}"
PILOT="${PILOT:-0}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
PREBUILT_BINARY_ROOT="${PREBUILT_BINARY_ROOT:-}"

if [ "$PILOT" = "1" ]; then
  ARMS="${PILOT_ARMS:-current-main,candidate}"
  BATCH_SIZES="${PILOT_BATCH_SIZES:-101}"
  TRIALS="${PILOT_TRIALS:-1}"
  FIXTURE_MODES="${PILOT_FIXTURE_MODES:-normal}"
  NORMAL_RESOURCES="${PILOT_RESOURCES:-1000}"
fi

arm_ref() {
  case "$1" in
    issue-source) printf '%s\n' "$ISSUE_SOURCE_REF" ;;
    current-main) printf '%s\n' "$CURRENT_MAIN_REF" ;;
    candidate) printf '%s\n' "issue-worktree" ;;
    *) echo "unknown arm: $1" >&2; return 2 ;;
  esac
}

expected_counts() {
  local arm="$1" batch="$2" total="$3" fixture="$4"
  if [ "$fixture" = "oversized" ]; then
    if [ "$total" != "203" ] || [ "$batch" != "500" ]; then
      echo "oversized oracle requires 203 resources and batch 500" >&2
      return 2
    fi
    if [ "$arm" = "candidate" ]; then
      printf '4 0\n'
    else
      printf '203 203\n'
    fi
    return
  fi

  local full=$((total / batch)) remainder=$((total % batch)) inserts savepoints
  if [ "$arm" = "candidate" ]; then
    inserts=$((full * ((batch + 99) / 100)))
    if [ "$remainder" -gt 0 ]; then
      inserts=$((inserts + ((remainder + 99) / 100)))
    fi
    savepoints=0
  else
    if [ "$batch" -le 100 ]; then
      inserts=$full
      savepoints=0
    else
      inserts=$((full * batch))
      savepoints=$((full * batch))
    fi
    if [ "$remainder" -gt 0 ]; then
      if [ "$remainder" -le 100 ]; then
        inserts=$((inserts + 1))
      else
        inserts=$((inserts + remainder))
        savepoints=$((savepoints + remainder))
      fi
    fi
  fi
  printf '%s %s\n' "$inserts" "$savepoints"
}

port_is_free() {
  python3 - "$1" <<'PY'
import socket, sys
port = int(sys.argv[1])
probe = socket.socket()
probe.settimeout(0.25)
try:
    probe.connect(("127.0.0.1", port))
except OSError:
    pass
else:
    raise SystemExit(1)
finally:
    probe.close()
sock = socket.socket()
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    sock.bind(("127.0.0.1", port))
except OSError:
    raise SystemExit(1)
finally:
    sock.close()
PY
}

pick_free_port() {
  local port="$1"
  while ! port_is_free "$port"; do
    port=$((port + 1))
  done
  printf '%s\n' "$port"
}

arm_repo() {
  if [ "$1" = "candidate" ]; then
    printf '%s\n' "$ROOT"
  else
    printf '%s/%s\n' "$WORKTREE_ROOT" "$1"
  fi
}

arm_binary() {
  printf '%s/hfs-%s\n' "$BINARY_ROOT" "$1"
}

arm_manifest() {
  printf '%s/hfs-%s.build.json\n' "$BINARY_ROOT" "$1"
}

prebuilt_arm_binary() {
  printf '%s/hfs-%s\n' "$PREBUILT_BINARY_ROOT" "$1"
}

prebuilt_arm_manifest() {
  printf '%s/hfs-%s.build.json\n' "$PREBUILT_BINARY_ROOT" "$1"
}

build_manifest() {
  local action="$1" arm="$2" ref="$3" repo="$4" binary="$5" manifest="$6"
  python3 - "$action" "$arm" "$ref" "$repo" "$binary" "$manifest" "$CARGO_BUILD_JOBS" <<'PY'
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

action, arm, ref, repo_arg, binary_arg, manifest_arg, build_jobs = sys.argv[1:]
repo = Path(repo_arg) if repo_arg else None
binary = Path(binary_arg)
manifest = Path(manifest_arg)
expected_command = [
    "cargo", "build", "--manifest-path", "$REPO/Cargo.toml", "--locked",
    "--release", "-p", "helios-hfs", "--bin", "hfs", "--no-default-features",
    "--features", "R4,postgres",
]

def file_sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def compiled_inputs(root):
    selected = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        parts = relative.parts
        include = relative.as_posix() in {
            "Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml",
            ".cargo/config", ".cargo/config.toml",
        }
        include |= len(parts) >= 3 and parts[0] == "crates" and (
            path.name in {"Cargo.toml", "build.rs"}
            or any(part in {
                "src", "migrations", "assets", "templates", "packs", "grammar",
                "resources", "terminology-data", "vendor",
            } for part in parts[2:])
        )
        include |= bool(parts) and parts[0] == "data"
        if include:
            selected.append(relative)
    selected.sort(key=lambda value: value.as_posix())
    digest = hashlib.sha256(b"hfs-compiled-inputs-v1\0")
    for relative in selected:
        encoded = relative.as_posix().encode()
        content_hash = bytes.fromhex(file_sha256(root / relative))
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update(content_hash)
    return {
        "algorithm": "sha256:hfs-compiled-inputs-v1",
        "scope": [
            "Cargo.toml", "Cargo.lock", "rust-toolchain*", ".cargo/config*",
            "crates/**/Cargo.toml", "crates/**/build.rs", "crates/**/src/**",
            "crates/**/migrations/**", "crates/**/assets/**",
            "crates/**/templates/**", "crates/**/packs/**", "crates/**/grammar/**",
            "crates/**/resources/**", "crates/**/terminology-data/**",
            "crates/**/vendor/**", "data/**",
        ],
        "file_count": len(selected),
        "sha256": digest.hexdigest(),
    }

def load_and_check_envelope():
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"prebuilt binary must be an executable file: {binary}")
    if not manifest.is_file():
        raise SystemExit(f"prebuilt build manifest is required: {manifest}")
    value = json.loads(manifest.read_text())
    if value.get("schema_version") != 1:
        raise SystemExit(f"unsupported build manifest schema: {manifest}")
    if value.get("arm") != arm:
        raise SystemExit(f"build manifest arm mismatch for {arm}: {manifest}")
    if (value.get("source") or {}).get("ref") != ref:
        raise SystemExit(f"build manifest source ref mismatch for {arm}: {manifest}")
    build = value.get("build") or {}
    if build.get("command") != expected_command:
        raise SystemExit(f"build command mismatch for {arm}: {manifest}")
    if build.get("inputs") != {
        "package": "helios-hfs", "binary": "hfs", "profile": "release",
        "locked": True, "default_features": False, "features": ["R4", "postgres"],
    }:
        raise SystemExit(f"build inputs mismatch for {arm}: {manifest}")
    toolchain = build.get("toolchain") or {}
    if not toolchain.get("cargo_version") or not toolchain.get("rustc_version"):
        raise SystemExit(f"build toolchain is missing for {arm}: {manifest}")
    recorded_binary = value.get("binary") or {}
    if recorded_binary.get("filename") != binary.name:
        raise SystemExit(f"binary filename mismatch for {arm}: {manifest}")
    if recorded_binary.get("bytes") != binary.stat().st_size:
        raise SystemExit(f"binary size mismatch for {arm}: {manifest}")
    if recorded_binary.get("sha256") != file_sha256(binary):
        raise SystemExit(f"binary SHA-256 mismatch for {arm}: {manifest}")
    return value

if action == "envelope":
    load_and_check_envelope()
elif action == "write":
    head = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
    ).strip()
    value = {
        "schema_version": 1,
        "arm": arm,
        "source": {"ref": ref, "head": head},
        "compiled_inputs": compiled_inputs(repo),
        "build": {
            "command": expected_command,
            "inputs": {
                "package": "helios-hfs", "binary": "hfs", "profile": "release",
                "locked": True, "default_features": False,
                "features": ["R4", "postgres"],
            },
            "environment": {"CARGO_BUILD_JOBS": build_jobs},
            "toolchain": {
                "cargo_version": subprocess.check_output(["cargo", "--version"], text=True).strip(),
                "rustc_version": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            },
        },
        "binary": {
            "filename": binary.name,
            "bytes": binary.stat().st_size,
            "sha256": file_sha256(binary),
        },
    }
    manifest.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
elif action == "validate":
    value = load_and_check_envelope()
    head = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
    ).strip()
    if (value.get("source") or {}).get("head") != head:
        raise SystemExit(f"source commit mismatch for {arm}: {manifest}")
    recorded_inputs = value.get("compiled_inputs") or {}
    actual_inputs = compiled_inputs(repo)
    if recorded_inputs != actual_inputs:
        raise SystemExit(
            f"compiled-input fingerprint mismatch for {arm}: {manifest}\n"
            f"recorded={recorded_inputs.get('sha256')} actual={actual_inputs['sha256']}"
        )
else:
    raise SystemExit(f"unknown build-manifest action: {action}")
PY
}

IFS=',' read -r -a ARM_LIST <<< "$ARMS"
IFS=',' read -r -a BATCH_LIST <<< "$BATCH_SIZES"
IFS=',' read -r -a FIXTURE_LIST <<< "$FIXTURE_MODES"

if [ -n "$PREBUILT_BINARY_ROOT" ]; then
  if [ ! -d "$PREBUILT_BINARY_ROOT" ]; then
    echo "prebuilt binary root is not a directory: $PREBUILT_BINARY_ROOT" >&2
    exit 2
  fi
  for arm in "${ARM_LIST[@]}"; do
    prebuilt="$(prebuilt_arm_binary "$arm")"
    manifest="$(prebuilt_arm_manifest "$arm")"
    build_manifest envelope "$arm" "$(arm_ref "$arm")" "" "$prebuilt" "$manifest"
  done
fi

echo "issue #1455 benchmark"
echo "  evidence: $EVIDENCE_ROOT"
echo "  controller: $CONTROLLER"
echo "  arms: ${ARM_LIST[*]}"
echo "  normal batches: ${BATCH_LIST[*]}"
echo "  trials: $TRIALS"
echo "  fixtures: ${FIXTURE_LIST[*]}"
echo "  PostgreSQL: $PG_IMAGE, preferred host port $PG_PORT_START"
echo "  provider preferred port: $PROVIDER_PORT; HFS port: 0 (auto)"

for arm in "${ARM_LIST[@]}"; do
  ref="$(arm_ref "$arm")"
  if [ -n "$PREBUILT_BINARY_ROOT" ]; then
    if [ "$arm" = "candidate" ]; then
      echo "  reuse $(prebuilt_arm_binary "$arm") for candidate; source fingerprint remains the issue worktree $ROOT"
    else
      echo "  reuse $(prebuilt_arm_binary "$arm") for $arm; create detached source worktree $WORKTREE_ROOT/$arm at $ref"
    fi
  elif [ "$arm" = "candidate" ]; then
    echo "  build candidate directly from issue worktree $ROOT (including uncommitted changes; isolated CARGO_TARGET_DIR)"
  else
    echo "  build $arm from $ref in detached worktree $WORKTREE_ROOT/$arm"
  fi
done

for fixture in "${FIXTURE_LIST[@]}"; do
  if [ "$fixture" = "normal" ]; then
    resources="$NORMAL_RESOURCES"
    sizes=("${BATCH_LIST[@]}")
  elif [ "$fixture" = "oversized" ]; then
    resources="$OVERSIZED_RESOURCES"
    sizes=(500)
  else
    echo "unknown fixture mode: $fixture" >&2
    exit 2
  fi
  for arm in "${ARM_LIST[@]}"; do
    for batch in "${sizes[@]}"; do
      read -r expected_inserts expected_savepoints < <(
        expected_counts "$arm" "$batch" "$resources" "$fixture"
      )
      for trial in $(seq 1 "$TRIALS"); do
        echo "  trial fixture=$fixture arm=$arm batch=$batch trial=$trial resources=$resources inserts=$expected_inserts savepoints=$expected_savepoints"
      done
    done
  done
done

if [ "$DRY_RUN" = "1" ]; then
  echo "DRY_RUN=1: no directories, worktrees, builds, containers, or databases were created"
  exit 0
fi

ACTIVE_CONTAINER=""
CREATED_WORKTREES=()

cleanup() {
  local index path
  if [ -n "$ACTIVE_CONTAINER" ]; then
    docker rm -f "$ACTIVE_CONTAINER" >/dev/null 2>&1 || true
    ACTIVE_CONTAINER=""
  fi
  index=$((${#CREATED_WORKTREES[@]} - 1))
  while [ "$index" -ge 0 ]; do
    path="${CREATED_WORKTREES[$index]}"
    case "$path" in
      "$WORKTREE_ROOT"/*)
        git -C "$ROOT" worktree remove --force "$path" >/dev/null 2>&1 || \
          echo "warning: could not remove benchmark worktree $path" >&2
        ;;
      *)
        echo "warning: refusing to remove untracked worktree path $path" >&2
        ;;
    esac
    index=$((index - 1))
  done
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$WORKTREE_ROOT" "$BINARY_ROOT" "$RAW_ROOT"

for arm in "${ARM_LIST[@]}"; do
  ref="$(arm_ref "$arm")"
  repo="$(arm_repo "$arm")"
  if [ "$arm" != "candidate" ]; then
    if [ -e "$repo" ]; then
      echo "refusing existing worktree path: $repo" >&2
      exit 2
    fi
    CREATED_WORKTREES+=("$repo")
    git -C "$ROOT" worktree add --detach "$repo" "$ref"
  fi
  binary="$(arm_binary "$arm")"
  manifest="$(arm_manifest "$arm")"
  if [ -n "$PREBUILT_BINARY_ROOT" ]; then
    prebuilt="$(prebuilt_arm_binary "$arm")"
    prebuilt_manifest="$(prebuilt_arm_manifest "$arm")"
    build_manifest validate "$arm" "$ref" "$repo" "$prebuilt" "$prebuilt_manifest"
    cp "$prebuilt" "$binary"
    cp "$prebuilt_manifest" "$manifest"
    chmod +x "$binary"
    echo "reused prebuilt $prebuilt as $binary"
  else
    build_dir="$EVIDENCE_ROOT/build/$arm"
    if [ -e "$build_dir" ]; then
      echo "refusing existing build target: $build_dir" >&2
      exit 2
    fi
    CARGO_TARGET_DIR="$build_dir" CARGO_BUILD_JOBS="$CARGO_BUILD_JOBS" \
      cargo build --manifest-path "$repo/Cargo.toml" --locked --release \
        -p helios-hfs --bin hfs --no-default-features --features R4,postgres
    cp "$build_dir/release/hfs" "$binary"
    chmod +x "$binary"
    build_manifest write "$arm" "$ref" "$repo" "$binary" "$manifest"
  fi
done

cleanup_container() {
  if [ -n "$ACTIVE_CONTAINER" ]; then
    docker rm -f "$ACTIVE_CONTAINER" >/dev/null 2>&1 || true
    ACTIVE_CONTAINER=""
  fi
}

run_trial() {
  local fixture="$1" arm="$2" batch="$3" trial="$4" resources="$5"
  local expected_inserts="$6" expected_savepoints="$7"
  local container="hfs-pgi01-r1-${RUN_ID//[^A-Za-z0-9_.-]/-}-${fixture}-${arm}-b${batch}-t${trial}"
  local output="$RAW_ROOT/$fixture/$arm/batch-$batch/trial-$trial"
  local port provider_port database_url fingerprint_plan fingerprint repo binary

  if docker inspect "$container" >/dev/null 2>&1; then
    echo "refusing existing container: $container" >&2
    return 2
  fi
  port="$(pick_free_port "$PG_PORT_START")"
  docker run -d --name "$container" \
    -e POSTGRES_USER=helios -e POSTGRES_PASSWORD=helios -e POSTGRES_DB=helios \
    -p "127.0.0.1:$port:5432" "$PG_IMAGE" \
    -c shared_preload_libraries=pg_stat_statements \
    -c pg_stat_statements.track_utility=on >/dev/null
  ACTIVE_CONTAINER="$container"
  for _ in $(seq 1 60); do
    docker exec "$container" pg_isready -U helios -d helios >/dev/null 2>&1 && break
    sleep 1
  done
  docker exec "$container" psql -U helios -d helios -v ON_ERROR_STOP=1 \
    -c 'CREATE EXTENSION IF NOT EXISTS pg_stat_statements' >/dev/null
  database_url="postgres://helios:helios@127.0.0.1:$port/helios"
  provider_port="$(pick_free_port "$PROVIDER_PORT")"
  repo="$(arm_repo "$arm")"
  binary="$(arm_binary "$arm")"

  fingerprint_plan="$(python3 "$CONTROLLER" --dry-run \
    --binary "$binary" --output-dir "$output" \
    --repo-root "$repo" --resources "$resources" --jobs 1 \
    --defer-indexing true --file-concurrency 1 --batch-size "$batch" \
    --fixture-mode "$fixture" --postgres-grouped-create-evidence \
    --expected-resource-inserts "$expected_inserts" \
    --expected-bulk-entry-savepoints "$expected_savepoints" \
    --pg-container "$container" --database-url "$database_url" \
    --hfs-port 0 --provider-port "$provider_port" --idle-seconds 0 \
    --request-timeout 300)"
  fingerprint="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["source"]["fingerprint_sha256"])' <<< "$fingerprint_plan")"

  python3 "$CONTROLLER" \
    --binary "$binary" --output-dir "$output" \
    --repo-root "$repo" --resources "$resources" --jobs 1 \
    --defer-indexing true --file-concurrency 1 --batch-size "$batch" \
    --fixture-mode "$fixture" --postgres-grouped-create-evidence \
    --expected-resource-inserts "$expected_inserts" \
    --expected-bulk-entry-savepoints "$expected_savepoints" \
    --expected-source-fingerprint "$fingerprint" \
    --pg-container "$container" --database-url "$database_url" \
    --hfs-port 0 --provider-port "$provider_port" --idle-seconds 0 \
    --request-timeout 300 \
    --sample-interval 0.25 --preflight-samples 3 --preflight-interval 1

  cleanup_container
}

for fixture in "${FIXTURE_LIST[@]}"; do
  if [ "$fixture" = "normal" ]; then
    resources="$NORMAL_RESOURCES"
    sizes=("${BATCH_LIST[@]}")
  else
    resources="$OVERSIZED_RESOURCES"
    sizes=(500)
  fi
  for batch in "${sizes[@]}"; do
    for trial in $(seq 1 "$TRIALS"); do
      # Rotate the starting arm by trial so cache warmth and host drift do not
      # always favor the same arm.
      arm_count="${#ARM_LIST[@]}"
      for step in $(seq 0 $((arm_count - 1))); do
        arm="${ARM_LIST[$(((trial - 1 + step) % arm_count))]}"
        read -r expected_inserts expected_savepoints < <(
          expected_counts "$arm" "$batch" "$resources" "$fixture"
        )
        run_trial "$fixture" "$arm" "$batch" "$trial" "$resources" \
          "$expected_inserts" "$expected_savepoints"
      done
    done
  done
done

python3 - "$RAW_ROOT" "$EVIDENCE_ROOT/aggregate.json" "$TRIALS" "$BATCH_SIZES" "$ARMS" "$FIXTURE_MODES" "$NORMAL_RESOURCES" "$OVERSIZED_RESOURCES" <<'PY'
import json, statistics, sys
from pathlib import Path

raw, destination = Path(sys.argv[1]), Path(sys.argv[2])
required_trials = int(sys.argv[3])
required_batches = {int(value) for value in sys.argv[4].split(",") if value}
required_arms = {value for value in sys.argv[5].split(",") if value}
required_fixtures = {value for value in sys.argv[6].split(",") if value}
normal_resources = int(sys.argv[7])
oversized_resources = int(sys.argv[8])
rows = []
for path in sorted(raw.glob("*/*/batch-*/trial-*/run.json")):
    relative = path.relative_to(raw).parts
    fixture, arm = relative[0], relative[1]
    batch = int(relative[2].split("-", 1)[1])
    trial = int(relative[3].split("-", 1)[1])
    run = json.loads(path.read_text())
    attempt = (run.get("attempts") or [{}])[0]
    sql = ((attempt.get("grouped_create_sql") or {}).get("interval_deltas") or {}).get(
        "kickoff_to_terminal", {}
    )
    wal = (((attempt.get("postgres_work") or {}).get("interval_deltas") or {}).get(
        "kickoff_to_terminal", {}
    )).get("wal_bytes")
    row = {
        "fixture": fixture,
        "arm": arm,
        "batch_size": batch,
        "trial": trial,
        "resource_count": (run.get("config") or {}).get("resources"),
        "status": run.get("status"),
        "comparable": bool((attempt.get("timings") or {}).get("comparable")),
        "comparison_reasons": (attempt.get("timings") or {}).get("comparison_reasons", []),
        "ingest_s": (attempt.get("timings") or {}).get("kickoff_to_terminal_s"),
        "search_ready_s": (attempt.get("timings") or {}).get("kickoff_to_verified_search_ready_s"),
        "hfs_peak_mib": (attempt.get("hfs_memory_kickoff_to_search_ready") or {}).get("peak_mib"),
        "wal_bytes_to_terminal": wal,
        "resource_insert_calls": (sql.get("resource_insert") or {}).get("calls"),
        "savepoint_calls": (sql.get("savepoint") or {}).get("calls"),
        "binary_sha256": ((run.get("preflight") or {}).get("binary") or {}).get("sha256"),
        "source_fingerprint": ((run.get("preflight") or {}).get("git") or {}).get("fingerprint_sha256"),
        "controller_sha256": ((run.get("preflight") or {}).get("controller") or {}).get("sha256"),
        "fixture_sha256": (run.get("fixture") or {}).get("corpus_sha256"),
        "hard_failures": [check.get("name") for check in run.get("checks", []) if check.get("kind") == "hard" and not check.get("ok")],
    }
    rows.append(row)

def summary(values):
    values = [float(value) for value in values if value is not None]
    return None if not values else {
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
        "spread": max(values) - min(values),
        "n": len(values),
    }

summaries = []
for key in sorted({(row["fixture"], row["arm"], row["batch_size"]) for row in rows}):
    selected = [row for row in rows if (row["fixture"], row["arm"], row["batch_size"]) == key]
    comparable = [row for row in selected if row["status"] == "ok" and row["comparable"] and not row["hard_failures"]]
    cell = {
        "fixture": key[0], "arm": key[1], "batch_size": key[2],
        "trials": len(selected), "comparable_trials": len(comparable),
        "hfs_peak_mib": summary([row["hfs_peak_mib"] for row in comparable]),
        "resource_insert_calls": sorted({row["resource_insert_calls"] for row in comparable}),
        "savepoint_calls": sorted({row["savepoint_calls"] for row in comparable}),
        "all_valid": len(comparable) == len(selected),
    }
    if key[0] == "normal":
        cell.update({
            "ingest_s": summary([row["ingest_s"] for row in comparable]),
            "search_ready_s": summary([row["search_ready_s"] for row in comparable]),
            "wal_bytes_to_terminal": summary([row["wal_bytes_to_terminal"] for row in comparable]),
        })
    summaries.append(cell)

lookup = {(row["fixture"], row["arm"], row["batch_size"]): row for row in summaries}
reasons = []
controller_hashes = {row["controller_sha256"] for row in rows if row["controller_sha256"]}
if len(controller_hashes) != 1:
    reasons.append(f"controller hash is not identical across trials: {sorted(controller_hashes)}")
for fixture in required_fixtures:
    hashes = {row["fixture_sha256"] for row in rows if row["fixture"] == fixture and row["fixture_sha256"]}
    if len(hashes) != 1:
        reasons.append(f"{fixture} fixture hash is not identical across trials: {sorted(hashes)}")
for arm in required_arms:
    arm_rows = [row for row in rows if row["arm"] == arm]
    binary_hashes = {row["binary_sha256"] for row in arm_rows if row["binary_sha256"]}
    source_hashes = {row["source_fingerprint"] for row in arm_rows if row["source_fingerprint"]}
    if len(binary_hashes) != 1:
        reasons.append(f"{arm} binary hash is not stable across trials: {sorted(binary_hashes)}")
    if len(source_hashes) != 1:
        reasons.append(f"{arm} source fingerprint is not stable across trials: {sorted(source_hashes)}")
full_matrix = (
    required_arms == {"issue-source", "current-main", "candidate"}
    and required_batches == {100, 101, 128, 500, 1000}
    and required_fixtures == {"normal", "oversized"}
    and required_trials >= 3
    and normal_resources == 10000
    and oversized_resources == 203
)
if not full_matrix:
    reasons.append("subset/pilot run: full gate requires three arms, both fixtures, five normal batch sizes, three trials, 10000 normal resources, and 203 oversized resources")
for fixture, expected_resources in (("normal", normal_resources), ("oversized", oversized_resources)):
    mismatches = [
        f"{row['arm']}/batch-{row['batch_size']}/trial-{row['trial']}={row['resource_count']}"
        for row in rows
        if row["fixture"] == fixture and row["resource_count"] != expected_resources
    ]
    if mismatches:
        reasons.append(
            f"{fixture} configured resource count must be {expected_resources}: {mismatches}"
        )
for arm in required_arms:
    for batch in required_batches:
        aggregate = lookup.get(("normal", arm, batch))
        if aggregate is None or aggregate["comparable_trials"] < required_trials:
            reasons.append(f"normal {arm} batch {batch} lacks {required_trials} comparable trials")
    oversized = lookup.get(("oversized", arm, 500))
    if oversized is None or oversized["comparable_trials"] < required_trials:
        reasons.append(f"oversized {arm} batch 500 lacks {required_trials} valid correctness/RSS trials")

def compare(baseline):
    details = {}
    improvements = []
    for batch in sorted(required_batches):
        base = lookup.get(("normal", baseline, batch))
        cand = lookup.get(("normal", "candidate", batch))
        if not base or not cand or not base["ingest_s"] or not cand["ingest_s"]:
            continue
        base_ingest = base["ingest_s"]["median"]
        cand_ingest = cand["ingest_s"]["median"]
        change = 100.0 * (cand_ingest - base_ingest) / base_ingest
        details[str(batch)] = {"ingest_change_percent": change}
        if batch > 100:
            improvements.append(-change)
        if batch == 100 and base["hfs_peak_mib"] and cand["hfs_peak_mib"]:
            details[str(batch)]["rss_change_percent"] = 100.0 * (
                cand["hfs_peak_mib"]["median"] - base["hfs_peak_mib"]["median"]
            ) / base["hfs_peak_mib"]["median"]
    batch100 = details.get("100", {})
    passed = bool(improvements) and max(improvements) >= 10.0 and batch100.get("ingest_change_percent", 999) <= 5.0 and batch100.get("rss_change_percent", 999) <= 5.0
    return {"baseline": baseline, "passed": passed, "details": details, "best_above_100_improvement_percent": max(improvements) if improvements else None}

causal = compare("current-main")
historical = compare("issue-source")
if causal["passed"] != historical["passed"]:
    reasons.append("historical and causal performance comparisons disagree")
gate = {
    "status": "incomplete" if not full_matrix else ("pass" if causal["passed"] and historical["passed"] and not reasons else "fail"),
    "causal": causal,
    "historical": historical,
    "reasons": reasons,
    "oversized_is_correctness_and_rss_only": True,
    "full_matrix": full_matrix,
    "configured_resource_counts": {
        "normal": normal_resources,
        "oversized": oversized_resources,
    },
}
payload = {
    "rows": rows,
    "normal_performance_aggregates": [row for row in summaries if row["fixture"] == "normal"],
    "oversized_correctness_rss": [row for row in summaries if row["fixture"] == "oversized"],
    "gate": gate,
}
destination.write_text(json.dumps(payload, indent=2) + "\n")
print(json.dumps(gate, indent=2))
if full_matrix and gate["status"] != "pass":
    raise SystemExit(1)
PY

echo "raw evidence: $RAW_ROOT"
echo "aggregate: $EVIDENCE_ROOT/aggregate.json"
