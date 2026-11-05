#!/usr/bin/env bash
# Smoke-test one platform's release build on a machine of that platform.
#
#   release-smoke.sh <dir> <wheel-dir>
#
# <dir> is laid out like a release archive: the shipped binaries (with `.exe`
# on Windows) plus `data/`. <wheel-dir> holds that platform's pysof wheels.
#
# Checks that every shipped binary starts and answers `--version`, that `hfs`
# serves a write, a read and a search over SQLite, and that a pysof wheel
# installs and runs a ViewDefinition. It does not test behavior; CI's test
# jobs do that on the native builds. It catches a build that links but does
# not run: a missing DLL, a wrong-glibc binary, a wheel tagged for the wrong
# platform.
set -euo pipefail

bin_dir=$(cd "$1" && pwd)
wheel_dir=$(cd "$2" && pwd)

ext=""
windows=false
case "${RUNNER_OS:-$(uname -s)}" in
  Windows | MINGW* | MSYS* | CYGWIN*)
    ext=".exe"
    windows=true
    ;;
esac

fail() {
  echo "::error::$*"
  exit 1
}

for b in hfs hts fhirpath-server fhirpath-cli sof-server sof-cli config-advisor; do
  exe="$bin_dir/$b$ext"
  [ -f "$exe" ] || fail "missing $b$ext in $bin_dir"
  chmod +x "$exe" 2>/dev/null || true
  if ! out=$("$exe" --version 2>&1); then
    echo "$out" | tail -20
    fail "$b$ext --version exited non-zero"
  fi
  echo "ok: $(echo "$out" | head -1)"
done

work=$(mktemp -d)
if [ -d "$bin_dir/data" ]; then
  cp -r "$bin_dir/data" "$work/"
fi

port=18080
base="http://127.0.0.1:$port"
(cd "$work" && exec env HFS_SERVER_PORT=$port "$bin_dir/hfs$ext" >"$work/hfs.log" 2>&1) &
hfs_pid=$!

stop_hfs() {
  kill "$hfs_pid" 2>/dev/null || true
  if $windows; then
    taskkill //F //IM "hfs.exe" >/dev/null 2>&1 || true
  fi
}
trap stop_hfs EXIT

ready=false
for _ in $(seq 1 180); do
  if curl -sf -o /dev/null "$base/metadata"; then
    ready=true
    break
  fi
  if ! kill -0 "$hfs_pid" 2>/dev/null; then
    tail -50 "$work/hfs.log"
    fail "hfs exited before serving /metadata"
  fi
  sleep 1
done
if ! $ready; then
  tail -50 "$work/hfs.log"
  fail "hfs did not serve /metadata within 180 s"
fi
echo "ok: hfs serves /metadata"

curl -sf -X PUT -H 'Content-Type: application/fhir+json' \
  --data '{"resourceType":"Patient","id":"smoke-1","name":[{"family":"Smoketest"}]}' \
  "$base/Patient/smoke-1" >/dev/null || fail "PUT Patient/smoke-1 failed"
curl -sf "$base/Patient/smoke-1" | grep -q '"Smoketest"' || fail "GET Patient/smoke-1 did not return the patient"
curl -sf "$base/Patient?family=Smoketest" | grep -q '"smoke-1"' || fail "search Patient?family=Smoketest did not find the patient"
echo "ok: hfs write, read and search"
stop_hfs
trap - EXIT

# `python` first: on Windows, `python3` can be the Microsoft Store stub.
py=$(command -v python || command -v python3) || fail "no python on PATH"
"$py" -m venv "$work/venv"
vpy="$work/venv/bin/python"
[ -x "$vpy" ] || vpy="$work/venv/Scripts/python.exe"
"$vpy" -m pip install -q --no-index --find-links "$wheel_dir" pysof ||
  fail "no pysof wheel in $wheel_dir installs on $("$py" --version 2>&1)"
"$vpy" - <<'EOF'
import pysof

view = {
    "resourceType": "ViewDefinition",
    "status": "active",
    "resource": "Patient",
    "select": [{"column": [
        {"name": "id", "path": "id"},
        {"name": "family", "path": "name.family"},
    ]}],
}
bundle = {
    "resourceType": "Bundle",
    "type": "collection",
    "entry": [{"resource": {"resourceType": "Patient", "id": "p1", "name": [{"family": "Doe"}]}}],
}
out = pysof.run_view_definition(view, bundle, "csv").decode()
assert "p1" in out and "Doe" in out, out
print("ok: pysof", getattr(pysof, "__version__", "?"), "runs a ViewDefinition")
EOF
