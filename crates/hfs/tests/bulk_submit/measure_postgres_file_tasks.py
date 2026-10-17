#!/usr/bin/env python3
"""Reproducible PostgreSQL file-task benchmark for issue #1457.

The harness owns every HFS process, fixture server, and database it creates.
It never prints or persists the PostgreSQL admin URL. Timed runs and profiling
runs are separate so profiler overhead cannot satisfy the acceptance budget.

Fixture directory contract:

* ``manifest.json`` is a Bulk Data manifest with at least eight output files.
* Output URLs may be absolute; their basename must name a file in this folder.
* At least one output has type ``Provenance`` and is at least 1 MiB by default.
* ``benchmark-probes.json`` contains ``structured`` and ``full_text`` arrays.
  Each item is ``{"path": "Patient?...", "minimum": 1}``.

The two release binaries should be built from the baseline and changed source
trees respectively. The harness records binary hashes and discovers the source
revision by walking upward from ``target/release/hfs`` when possible.
"""

from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import json
import os
import re
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SUBMITTER_SYSTEM = "https://helios.software/bench/postgres-file-tasks"
SAFE_DATABASE = re.compile(r"^hfs_1457_[a-z0-9_]+$")


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def http_json(
    method: str,
    url: str,
    tenant: str,
    payload: dict[str, Any] | None = None,
    timeout: float = 60,
) -> tuple[int, dict[str, str], Any]:
    body = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(
        url,
        data=body,
        method=method,
        headers={
            "Accept": "application/fhir+json",
            "Content-Type": "application/fhir+json",
            "X-Tenant-ID": tenant,
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
            return (
                response.status,
                {key.lower(): value for key, value in response.headers.items()},
                json.loads(raw) if raw else None,
            )
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            parsed: Any = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = raw.decode("utf-8", "replace")
        return error.code, {k.lower(): v for k, v in error.headers.items()}, parsed


@dataclass(frozen=True)
class PgConnection:
    host: str
    port: str
    user: str
    password: str
    database: str
    query: str

    @classmethod
    def parse(cls, value: str) -> "PgConnection":
        parsed = urllib.parse.urlsplit(value)
        if parsed.scheme not in {"postgres", "postgresql"} or not parsed.hostname:
            raise ValueError("admin URL must be a postgres:// or postgresql:// URL")
        return cls(
            parsed.hostname,
            str(parsed.port or 5432),
            urllib.parse.unquote(parsed.username or "postgres"),
            urllib.parse.unquote(parsed.password or ""),
            parsed.path.lstrip("/") or "postgres",
            parsed.query,
        )

    def env(self, database: str | None = None) -> dict[str, str]:
        result = os.environ.copy()
        result.update(
            {
                "PGHOST": self.host,
                "PGPORT": self.port,
                "PGUSER": self.user,
                "PGDATABASE": database or self.database,
                "PGPASSWORD": self.password,
            }
        )
        return result

    def database_url(self, database: str) -> str:
        auth = urllib.parse.quote(self.user, safe="")
        if self.password:
            auth += ":" + urllib.parse.quote(self.password, safe="")
        query = f"?{self.query}" if self.query else ""
        return f"postgresql://{auth}@{self.host}:{self.port}/{database}{query}"


class DatabaseOwner:
    def __init__(self, connection: PgConnection) -> None:
        self.connection = connection
        self.created: set[str] = set()

    def sql(self, statement: str, database: str | None = None) -> str:
        completed = subprocess.run(
            ["psql", "-X", "-v", "ON_ERROR_STOP=1", "-Atqc", statement],
            env=self.connection.env(database),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode:
            raise RuntimeError(f"psql failed: {completed.stderr.strip()}")
        return completed.stdout.strip()

    def create(self, name: str) -> None:
        if not SAFE_DATABASE.fullmatch(name):
            raise RuntimeError(f"refusing unsafe database name: {name}")
        self.sql(f'CREATE DATABASE "{name}"')
        self.created.add(name)

    def drop(self, name: str) -> None:
        if name not in self.created or not SAFE_DATABASE.fullmatch(name):
            raise RuntimeError(f"refusing to drop unowned database: {name}")
        self.sql(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)')
        self.created.remove(name)

    def cleanup(self) -> None:
        for name in sorted(self.created):
            try:
                self.drop(name)
            except Exception as error:  # cleanup must attempt every owned DB
                print(f"warning: could not drop owned database {name}: {error}", file=sys.stderr)


class GlobalThrottle:
    def __init__(self, bytes_per_second: int) -> None:
        self.rate = bytes_per_second
        self.lock = threading.Lock()
        self.started = time.monotonic()
        self.sent = 0

    def wait(self, size: int) -> None:
        if self.rate <= 0:
            return
        with self.lock:
            self.sent += size
            delay = self.sent / self.rate - (time.monotonic() - self.started)
        if delay > 0:
            time.sleep(delay)


class FixtureServer:
    def __init__(self, root: Path, manifests: dict[str, bytes], throttle: int) -> None:
        self.manifests = manifests
        limiter = GlobalThrottle(throttle)
        root = root.resolve()

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
                key = urllib.parse.urlsplit(self.path).path.lstrip("/")
                if key in manifests:
                    body = manifests[key]
                    limiter.wait(len(body))
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                return super().do_GET()

            def copyfile(self, source: Any, outputfile: Any) -> None:
                while True:
                    chunk = source.read(64 * 1024)
                    if not chunk:
                        return
                    limiter.wait(len(chunk))
                    outputfile.write(chunk)

        handler = functools.partial(Handler, directory=str(root))
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.server.server_port}"

    def __enter__(self) -> "FixtureServer":
        self.thread.start()
        return self

    def set_manifest(self, name: str, body: bytes) -> None:
        self.manifests[name] = body

    def __exit__(self, *_args: object) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


class ProcessSampler:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self.process = process
        self.samples: list[dict[str, Any]] = []
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self.stop_event.wait(0.2):
            try:
                status = Path(f"/proc/{self.process.pid}/status").read_text()
                rss_match = re.search(r"^VmRSS:\s+(\d+)", status, re.MULTILINE)
                threads: dict[str, int] = {}
                for stat in Path(f"/proc/{self.process.pid}/task").glob("*/stat"):
                    fields = stat.read_text().split()
                    threads[stat.parent.name] = int(fields[13]) + int(fields[14])
                self.samples.append(
                    {
                        "t": time.monotonic(),
                        "rss_kib": int(rss_match.group(1)) if rss_match else 0,
                        "thread_ticks": threads,
                    }
                )
            except (FileNotFoundError, ProcessLookupError, IndexError, ValueError):
                return

    def __enter__(self) -> "ProcessSampler":
        self.thread.start()
        return self

    def __exit__(self, *_args: object) -> None:
        self.stop_event.set()
        self.thread.join(timeout=2)

    def summary(self) -> dict[str, Any]:
        peak = max((sample["rss_kib"] for sample in self.samples), default=0)
        first = self.samples[0]["thread_ticks"] if self.samples else {}
        last = self.samples[-1]["thread_ticks"] if self.samples else {}
        cpu = {tid: last[tid] - first.get(tid, last[tid]) for tid in last}
        return {"peak_rss_kib": peak, "thread_cpu_ticks": cpu}


class HfsProcess:
    def __init__(
        self,
        binary: Path,
        database_url: str,
        concurrency: int,
        log_path: Path,
        profile_path: Path | None = None,
    ) -> None:
        self.port = free_port()
        self.base_url = f"http://127.0.0.1:{self.port}"
        self.output_dir = log_path.parent / f"{log_path.stem}-artifacts"
        env = os.environ.copy()
        env.update(
            {
                "HFS_BASE_URL": self.base_url,
                "HFS_STORAGE_BACKEND": "postgres",
                "HFS_DATABASE_URL": database_url,
                "HFS_BULK_SUBMIT_ENABLED": "true",
                "HFS_BULK_SUBMIT_WORKER_CONCURRENCY": "1",
                "HFS_BULK_SUBMIT_FILE_CONCURRENCY": str(concurrency),
                "HFS_BULK_SUBMIT_BATCH_SIZE": "100",
                "HFS_BULK_SUBMIT_DEFER_INDEXING": "true",
                "HFS_BULK_SUBMIT_POLL_RATE_LIMIT": "0",
                "HFS_BULK_SUBMIT_LEASE_DURATION": "9",
                "HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL": "3",
                "HFS_BULK_SUBMIT_OUTPUT_BACKEND": "local-fs",
                "HFS_BULK_SUBMIT_OUTPUT_DIR": str(self.output_dir),
                "HFS_PG_MAX_CONNECTIONS": "16",
                "HFS_LOG_LEVEL": "info",
                "HFS_PERF_PHASES": "1",
                "RUST_LOG": "info,hfs_perf=info",
                "TOKIO_WORKER_THREADS": "8",
            }
        )
        command = [str(binary), "--host", "127.0.0.1", "--port", str(self.port)]
        if profile_path is not None:
            command = ["perf", "record", "-F", "99", "-g", "-o", str(profile_path), "--", *command]
        self.log_handle = log_path.open("wb")
        self.process = subprocess.Popen(
            command,
            env=env,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )

    def wait_ready(self, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"HFS exited before readiness with {self.process.returncode}")
            try:
                status, _, _ = http_json("GET", f"{self.base_url}/metadata", "bench", timeout=2)
                if status == 200:
                    return
            except OSError:
                pass
            time.sleep(0.1)
        raise RuntimeError("HFS did not become ready")

    def stop(self) -> None:
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        self.log_handle.close()


def git_provenance(binary: Path) -> dict[str, Any]:
    resolved = binary.resolve()
    result: dict[str, Any] = {"path": str(resolved), "sha256": sha256(resolved)}
    for parent in resolved.parents:
        if (parent / ".git").exists() or (parent / ".git").is_file():
            completed = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=parent,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            if completed.returncode == 0:
                status = subprocess.run(
                    ["git", "status", "--porcelain=v1"],
                    cwd=parent,
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    check=False,
                ).stdout
                diff = subprocess.run(
                    ["git", "diff", "--binary", "HEAD"],
                    cwd=parent,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    check=False,
                ).stdout
                state = hashlib.sha256(status.encode() + b"\0" + diff).hexdigest()
                result.update(
                    {
                        "source_root": str(parent),
                        "revision": completed.stdout.strip(),
                        "source_state_sha256": state,
                        "dirty": bool(status),
                    }
                )
            break
    return result


def load_fixture(root: Path, minimum_provenance_bytes: int) -> dict[str, Any]:
    manifest_path = root / "manifest.json"
    probes_path = root / "benchmark-probes.json"
    if not manifest_path.is_file() or not probes_path.is_file():
        raise RuntimeError("fixture-dir must contain manifest.json and benchmark-probes.json")
    manifest = json.loads(manifest_path.read_text())
    output = manifest.get("output", [])
    if len(output) < 8:
        raise RuntimeError("fixture manifest must contain at least eight output files")
    files: list[dict[str, Any]] = []
    expected_by_type: dict[str, set[str]] = {}
    lines = 0
    provenance_large = False
    for entry in output:
        name = Path(urllib.parse.urlsplit(entry["url"]).path).name
        path = root / name
        if not path.is_file():
            raise RuntimeError(f"fixture output is missing: {name}")
        resource_type = str(entry.get("type") or "Resource")
        identifiers = expected_by_type.setdefault(resource_type, set())
        file_identifiers: set[str] = set()
        with path.open(encoding="utf-8") as handle:
            for line_number, line in enumerate(handle, 1):
                if not line.strip():
                    continue
                resource = json.loads(line)
                if resource.get("resourceType") != resource_type or not resource.get("id"):
                    raise RuntimeError(f"{name}:{line_number} lacks matching resourceType/id")
                identifier = str(resource["id"])
                if identifier in identifiers:
                    raise RuntimeError(
                        f"duplicate {resource_type}/{identifier}; acceptance fixture IDs must be unique"
                    )
                identifiers.add(identifier)
                file_identifiers.add(identifier)
                lines += 1
        if resource_type == "Provenance" and path.stat().st_size >= minimum_provenance_bytes:
            provenance_large = True
        files.append(
            {
                "path": path,
                "name": name,
                "type": resource_type,
                "sha256": sha256(path),
                "ids": sorted(file_identifiers),
            }
        )
    if not provenance_large:
        raise RuntimeError(
            f"fixture needs a Provenance output of at least {minimum_provenance_bytes} bytes"
        )
    probes = json.loads(probes_path.read_text())
    for group in ("structured", "full_text"):
        if not probes.get(group):
            raise RuntimeError(f"benchmark-probes.json needs a non-empty {group} array")
    return {
        "manifest": manifest,
        "files": files,
        "probes": probes,
        "line_count": lines,
        "unique_count": sum(len(values) for values in expected_by_type.values()),
        "by_type": {key: len(value) for key, value in expected_by_type.items()},
        "hashes": {"manifest.json": sha256(manifest_path), **{item["name"]: item["sha256"] for item in files}},
    }


def warmed_fixture(fixture: dict[str, Any]) -> None:
    for item in fixture["files"]:
        if sha256(item["path"]) != item["sha256"]:
            raise RuntimeError(f"fixture changed during run: {item['path']}")


def fixture_for_scenario(fixture: dict[str, Any], scenario: str) -> dict[str, Any]:
    if scenario != "single-file":
        return fixture
    selected = next(item for item in fixture["files"] if item["type"] == "Provenance")
    return {
        **fixture,
        "files": [selected],
        "line_count": len(selected["ids"]),
        "unique_count": len(selected["ids"]),
        "by_type": {"Provenance": len(selected["ids"])},
    }


def rewrite_manifest(fixture: dict[str, Any], provider_url: str, single_file: bool) -> bytes:
    manifest = json.loads(json.dumps(fixture["manifest"]))
    output = manifest["output"]
    if single_file:
        output = [next(item for item in output if item.get("type") == "Provenance")]
    for entry in output:
        entry["url"] = f"{provider_url}/{Path(urllib.parse.urlsplit(entry['url']).path).name}"
    manifest["output"] = output
    manifest["requiresAccessToken"] = False
    return (json.dumps(manifest, separators=(",", ":")) + "\n").encode()


def kickoff(base_url: str, tenant: str, run_id: str, manifest_url: str) -> tuple[str, float]:
    submission_id = f"pgi03-{run_id}"
    payload = {
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": run_id}},
            {"name": "submissionId", "valueString": submission_id},
            {"name": "manifestUrl", "valueUrl": manifest_url},
            {"name": "fhirBaseUrl", "valueUrl": base_url},
            {"name": "submissionStatus", "valueCoding": {"system": "http://hl7.org/fhir/event-status", "code": "completed"}},
        ],
    }
    started = time.monotonic()
    status, _, response = http_json("POST", f"{base_url}/$bulk-submit", tenant, payload)
    if status != 200:
        raise RuntimeError(f"bulk submit kickoff returned HTTP {status}: {response}")
    return submission_id, started


def poll_terminal(base_url: str, tenant: str, run_id: str, submission_id: str, timeout: float) -> float:
    payload = {
        "resourceType": "Parameters",
        "parameter": [
            {"name": "submitter", "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": run_id}},
            {"name": "submissionId", "valueString": submission_id},
        ],
    }
    status, headers, response = http_json("POST", f"{base_url}/$bulk-submit-status", tenant, payload)
    if status != 202 or not headers.get("content-location"):
        raise RuntimeError(f"status kickoff returned HTTP {status}: {response}")
    poll_url = urllib.parse.urljoin(base_url + "/", headers["content-location"])
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        status, _, response = http_json("GET", poll_url, tenant)
        if status == 200:
            if isinstance(response, dict) and response.get("error"):
                raise RuntimeError(f"terminal manifest contains errors: {response['error'][:3]}")
            return time.monotonic()
        if status not in (202, 429):
            raise RuntimeError(f"status poll returned HTTP {status}: {response}")
        time.sleep(0.2)
    raise RuntimeError("bulk submit did not reach terminal status")


def bundle_total(base_url: str, tenant: str, path: str) -> int:
    separator = "&" if "?" in path else "?"
    status, _, body = http_json("GET", f"{base_url}/{path}{separator}_summary=count&_count=1", tenant)
    if status != 200 or not isinstance(body, dict) or not isinstance(body.get("total"), int):
        raise RuntimeError(f"search probe failed for {path}: HTTP {status} {body}")
    return int(body["total"])


def wait_search_ready(
    base_url: str,
    tenant: str,
    expected: dict[str, int],
    probes: dict[str, Any] | None,
    timeout: float,
) -> float:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        counts_ready = all(
            bundle_total(base_url, tenant, resource_type) == count
            for resource_type, count in expected.items()
        )
        probes_ready = probes is None or all(
            bundle_total(base_url, tenant, str(probe["path"]))
            >= int(probe.get("minimum", 1))
            for group in ("structured", "full_text")
            for probe in probes[group]
        )
        if counts_ready and probes_ready:
            return time.monotonic()
        time.sleep(0.25)
    raise RuntimeError("search counts did not become exact")


def verify_probes(base_url: str, tenant: str, probes: dict[str, Any]) -> dict[str, int]:
    results: dict[str, int] = {}
    for group in ("structured", "full_text"):
        for probe in probes[group]:
            total = bundle_total(base_url, tenant, str(probe["path"]))
            if total < int(probe.get("minimum", 1)):
                raise RuntimeError(f"{group} probe returned {total}: {probe['path']}")
            results[f"{group}:{probe['path']}"] = total
    return results


def database_checks(owner: DatabaseOwner, database: str, tenant: str, fixture: dict[str, Any]) -> dict[str, int]:
    def count(table: str) -> int:
        literal = tenant.replace("'", "''")
        return int(owner.sql(f"SELECT count(*) FROM {table} WHERE tenant_id = '{literal}'", database) or 0)

    checks = {table: count(table) for table in (
        "resources", "resource_history", "bulk_entry_results", "bulk_submission_changes"
    )}
    expected_unique = fixture["unique_count"]
    expected_lines = fixture["line_count"]
    expected = {
        "resources": expected_unique,
        "resource_history": expected_unique,
        "bulk_entry_results": expected_lines,
        "bulk_submission_changes": expected_unique,
    }
    if checks != expected:
        raise RuntimeError(f"database exactness failed: got {checks}, expected {expected}")
    return checks


def parse_internal_metrics(log_path: Path) -> dict[str, Any]:
    text = log_path.read_text(errors="replace")
    pool = re.findall(r"postgres_pool_checkout[^\n]*", text)
    heartbeat = re.findall(r"submit_heartbeat_(?:schedule_delay|rpc)[^\n]*", text)
    return {
        "pool_wait": pool or None,
        "heartbeat_delay": heartbeat or None,
        "available": bool(pool and heartbeat),
    }


def observed_scheduling(log_path: Path) -> str | None:
    text = log_path.read_text(errors="replace")
    match = re.search(r"scheduling=(?:\"|')?([a-z-]+)", text)
    return match.group(1) if match else None


def scrub_log(log_path: Path, connection: PgConnection, database_url: str) -> None:
    """Remove credentials before a server log becomes a retained artifact."""
    text = log_path.read_text(errors="replace")
    secrets = {
        database_url,
        connection.password,
        urllib.parse.quote(connection.password, safe="") if connection.password else "",
    }
    for secret in sorted((value for value in secrets if value), key=len, reverse=True):
        text = text.replace(secret, "[REDACTED]")
    log_path.write_text(text)


def run_trial(
    args: argparse.Namespace,
    owner: DatabaseOwner,
    fixture: dict[str, Any],
    output: Path,
    binary: Path,
    revision: str,
    concurrency: int,
    round_number: int,
    scenario: str,
    profile: bool = False,
) -> dict[str, Any]:
    token = uuid.uuid4().hex[:10]
    database = f"hfs_1457_{token}_{revision}_{concurrency}"
    database = database[:62]
    tenant = f"hfs-1457-{token}"
    trial_dir = output / "profiles" if profile else output / "trials"
    stem = f"{scenario}-r{round_number}-{revision}-n{concurrency}-{token}"
    result_path = trial_dir / f"{stem}.json"
    log_path = trial_dir / f"{stem}.log"
    profile_path = trial_dir / f"{stem}.perf.data" if profile else None
    profile_script = trial_dir / f"{stem}.perf.script" if profile else None
    owner.create(database)
    process: HfsProcess | None = None
    database_url = owner.connection.database_url(database)
    result: dict[str, Any] = {
        "scenario": scenario,
        "round": round_number,
        "revision": revision,
        "concurrency": concurrency,
        "database": database,
        "tenant": tenant,
        "started_at": now_iso(),
        "status": "running",
        "profile_artifact": str(profile_path) if profile_path else None,
        "profile_script": str(profile_script) if profile_script else None,
    }
    write_json(result_path, result)
    try:
        trial_fixture = fixture_for_scenario(fixture, scenario)
        warmed_fixture(trial_fixture)
        throttle = args.throttle_bytes_per_second if scenario == "throttled" else 0
        manifests: dict[str, bytes] = {}
        with FixtureServer(args.fixture_dir, manifests, throttle) as provider:
            provider.set_manifest(
                "manifest.json",
                rewrite_manifest(fixture, provider.base_url, scenario == "single-file"),
            )
            process = HfsProcess(
                binary,
                database_url,
                concurrency,
                log_path,
                profile_path,
            )
            process.wait_ready(args.timeout)
            with ProcessSampler(process.process) as sampler:
                submission, started = kickoff(
                    process.base_url, tenant, token, f"{provider.base_url}/manifest.json"
                )
                terminal = poll_terminal(process.base_url, tenant, token, submission, args.timeout)
                probe_config = fixture["probes"] if scenario != "single-file" else None
                search_ready = wait_search_ready(
                    process.base_url,
                    tenant,
                    trial_fixture["by_type"],
                    probe_config,
                    args.timeout,
                )
                probes = (
                    verify_probes(process.base_url, tenant, fixture["probes"])
                    if scenario != "single-file"
                    else {}
                )
            checks = database_checks(owner, database, tenant, trial_fixture)
            result.update(
                {
                    "status": "passed",
                    "ingest_seconds": terminal - started,
                    "search_ready_seconds": search_ready - started,
                    "resource_usage": sampler.summary(),
                    "database_checks": checks,
                    "search_probes": probes,
                }
            )
    except Exception as error:
        result.update({"status": "failed", "error": str(error)})
        raise
    finally:
        if process is not None:
            process.stop()
            scrub_log(log_path, owner.connection, database_url)
            result["internal_metrics"] = parse_internal_metrics(log_path)
            result["observed_scheduling"] = observed_scheduling(log_path)
            if revision == "changed":
                expected_scheduling = "independent-tasks" if concurrency > 1 else "inline"
                if result["observed_scheduling"] != expected_scheduling:
                    result.update(
                        {
                            "status": "failed",
                            "error": "changed binary did not log expected scheduling "
                            f"{expected_scheduling!r}",
                        }
                    )
            if profile_path is not None and profile_script is not None:
                with profile_script.open("wb") as handle:
                    completed = subprocess.run(
                        ["perf", "script", "-i", str(profile_path)],
                        stdout=handle,
                        stderr=subprocess.PIPE,
                        check=False,
                    )
                result["profile_script_returncode"] = completed.returncode
            if process.output_dir.exists():
                shutil.rmtree(process.output_dir)
        result["finished_at"] = now_iso()
        write_json(result_path, result)
        owner.drop(database)
    return result


def aggregate(results: list[dict[str, Any]]) -> dict[str, Any]:
    groups: dict[str, list[dict[str, Any]]] = {}
    for result in results:
        if result["scenario"] == "main" and result["status"] == "passed":
            groups.setdefault(f"{result['revision']}:n{result['concurrency']}", []).append(result)
    summary: dict[str, Any] = {}
    for key, values in groups.items():
        ingest = [float(value["ingest_seconds"]) for value in values]
        search = [float(value["search_ready_seconds"]) for value in values]
        rss = [int(value["resource_usage"]["peak_rss_kib"]) for value in values]
        summary[key] = {
            "trials": len(values),
            "ingest_median_seconds": statistics.median(ingest),
            "ingest_min_seconds": min(ingest),
            "ingest_max_seconds": max(ingest),
            "ingest_mad_seconds": statistics.median(
                [abs(value - statistics.median(ingest)) for value in ingest]
            ),
            "search_ready_median_seconds": statistics.median(search),
            "peak_rss_median_kib": statistics.median(rss),
        }
    metrics_available = all(
        result.get("internal_metrics", {}).get("available", False)
        for result in results
        if result["scenario"] == "main"
    )
    expected_main_groups = {
        f"{revision}:n{concurrency}"
        for revision in ("baseline", "changed")
        for concurrency in (1, 2, 4, 8)
    }
    complete_timed_matrix = set(summary) == expected_main_groups and all(
        summary[key]["trials"] == 3 for key in expected_main_groups
    )
    controls = [
        result
        for result in results
        if result["scenario"] in {"single-file", "throttled"}
    ]
    profiles = [result for result in results if result["scenario"] == "profile"]
    ingest_profile_markers = (
        "NdjsonEntry>::parse",
        "process_ndjson_stream",
        "BulkSubmitProvider>::process_entries",
    )
    preparation_overlaps = False
    for result in profiles:
        if (
            result["revision"] != "changed"
            or result["concurrency"] <= 1
            or result["status"] != "passed"
            or not result.get("profile_script")
        ):
            continue
        script = Path(result["profile_script"])
        if not script.is_file():
            continue
        preparation_threads: set[str] = set()
        for block in script.read_text(errors="replace").split("\n\n"):
            if not any(marker in block for marker in ingest_profile_markers):
                continue
            header = block.splitlines()[0] if block.splitlines() else ""
            match = re.match(r".+?\s+(\d+)\s+[0-9]+\.[0-9]+:", header)
            if match:
                preparation_threads.add(match.group(1))
        preparation_overlaps |= len(preparation_threads) >= 2
    gates: dict[str, Any] = {
        "complete_timed_matrix": complete_timed_matrix,
        "control_scenarios_passed": len(controls) == 4
        and all(result["status"] == "passed" for result in controls),
        "internal_pool_wait_and_heartbeat_available": metrics_available,
        "n1_time_regression_within_5_percent": None,
        "n1_rss_regression_within_5_percent": None,
        "changed_rss_below_n_times_n1": None,
        "changed_at_least_10_percent_faster_for_one_n_gt_1": None,
        "preparation_overlaps_multiple_runtime_threads": preparation_overlaps,
        "profiles_present_and_decoded": all(
            result["status"] == "passed"
            and result.get("profile_script_returncode") == 0
            and bool(result.get("profile_artifact"))
            and Path(result["profile_artifact"]).is_file()
            and Path(result["profile_artifact"]).stat().st_size > 0
            and bool(result.get("profile_script"))
            and Path(result["profile_script"]).is_file()
            and Path(result["profile_script"]).stat().st_size > 0
            for result in profiles
        )
        and len(profiles) == 8,
    }
    if "baseline:n1" in summary and "changed:n1" in summary:
        base = summary["baseline:n1"]
        changed = summary["changed:n1"]
        gates["n1_time_regression_within_5_percent"] = changed["ingest_median_seconds"] <= 1.05 * base["ingest_median_seconds"]
        gates["n1_rss_regression_within_5_percent"] = changed["peak_rss_median_kib"] <= 1.05 * base["peak_rss_median_kib"]
        gates["changed_rss_below_n_times_n1"] = all(
            summary[f"changed:n{n}"]["peak_rss_median_kib"] < n * changed["peak_rss_median_kib"]
            for n in (2, 4, 8)
            if f"changed:n{n}" in summary
        )
        gates["changed_at_least_10_percent_faster_for_one_n_gt_1"] = any(
            summary[f"changed:n{n}"]["ingest_median_seconds"] <= 0.9 * summary[f"baseline:n{n}"]["ingest_median_seconds"]
            for n in (2, 4, 8)
            if f"changed:n{n}" in summary and f"baseline:n{n}" in summary
        )
    eligible = metrics_available and all(value is True for value in gates.values())
    return {"groups": summary, "gates": gates, "acceptance_eligible": eligible}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-bin", type=Path, required=True)
    parser.add_argument("--changed-bin", type=Path, required=True)
    parser.add_argument("--fixture-dir", type=Path, required=True)
    parser.add_argument("--postgres-admin-url-env", required=True)
    parser.add_argument("--concurrency", default="1,2,4,8")
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=1800)
    parser.add_argument("--min-provenance-bytes", type=int, default=1024 * 1024)
    parser.add_argument("--throttle-bytes-per-second", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--skip-profiles", action="store_true")
    parser.add_argument("--baseline-revision")
    parser.add_argument("--changed-revision")
    args = parser.parse_args()
    args.fixture_dir = args.fixture_dir.resolve()
    return args


def main() -> int:
    args = parse_args()
    if args.output.exists():
        raise RuntimeError(f"output directory already exists: {args.output}")
    for command in ("psql",):
        if shutil.which(command) is None:
            raise RuntimeError(f"required command is missing: {command}")
    if not args.skip_profiles and shutil.which("perf") is None:
        raise RuntimeError("perf is required unless --skip-profiles is supplied")
    for binary in (args.baseline_bin, args.changed_bin):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError(f"release binary is not executable: {binary}")
    admin_value = os.environ.get(args.postgres_admin_url_env)
    if not admin_value:
        raise RuntimeError(f"environment variable is unset: {args.postgres_admin_url_env}")
    connection = PgConnection.parse(admin_value)
    concurrency = [int(value) for value in args.concurrency.split(",")]
    if concurrency != [1, 2, 4, 8]:
        raise RuntimeError("acceptance run requires --concurrency 1,2,4,8")
    if args.trials != 3:
        raise RuntimeError("acceptance run requires --trials 3")

    fixture = load_fixture(args.fixture_dir, args.min_provenance_bytes)
    args.output.mkdir(parents=True)
    provenance = {
        "baseline": git_provenance(args.baseline_bin),
        "changed": git_provenance(args.changed_bin),
    }
    if args.baseline_revision:
        provenance["baseline"]["revision"] = args.baseline_revision
    if args.changed_revision:
        provenance["changed"]["revision"] = args.changed_revision
    for label in ("baseline", "changed"):
        if not provenance[label].get("revision"):
            raise RuntimeError(
                f"could not discover {label} source revision; pass --{label}-revision"
            )
    if provenance["baseline"]["sha256"] == provenance["changed"]["sha256"]:
        raise RuntimeError("baseline and changed binaries have the same SHA-256")
    baseline_source = (
        provenance["baseline"]["revision"],
        provenance["baseline"].get("source_state_sha256", "clean"),
    )
    changed_source = (
        provenance["changed"]["revision"],
        provenance["changed"].get("source_state_sha256", "clean"),
    )
    if baseline_source == changed_source:
        raise RuntimeError("baseline and changed source provenance are identical")
    run: dict[str, Any] = {
        "started_at": now_iso(),
        "host": {"cpu_count": os.cpu_count(), "platform": sys.platform},
        "configuration": {
            "concurrency": concurrency,
            "trials": args.trials,
            "batch_size": 100,
            "worker_concurrency": 1,
            "postgres_pool_size": 16,
            "tokio_worker_threads": 8,
            "defer_indexing": True,
            "lease_duration_seconds": 9,
            "heartbeat_interval_seconds": 3,
            "fixture_warmup": "SHA-256/read every output before every trial",
            "cache_policy": "OS and PostgreSQL host caches are not reset",
            "admin_url_env": args.postgres_admin_url_env,
        },
        "fixture": {
            "hashes": fixture["hashes"],
            "line_count": fixture["line_count"],
            "unique_count": fixture["unique_count"],
            "by_type": fixture["by_type"],
        },
        "binaries": provenance,
        "results": [],
    }
    write_json(args.output / "run.json", run)
    owner = DatabaseOwner(connection)
    try:
        binaries = {"baseline": args.baseline_bin.resolve(), "changed": args.changed_bin.resolve()}
        for round_number in range(1, args.trials + 1):
            order = ["baseline", "changed"] if round_number % 2 else ["changed", "baseline"]
            for n in concurrency:
                for revision in order:
                    result = run_trial(
                        args, owner, fixture, args.output, binaries[revision], revision, n, round_number, "main"
                    )
                    run["results"].append(result)
                    write_json(args.output / "run.json", run)
        for scenario in ("single-file", "throttled"):
            for revision in ("baseline", "changed"):
                result = run_trial(
                    args, owner, fixture, args.output, binaries[revision], revision, 8, 1, scenario
                )
                run["results"].append(result)
                write_json(args.output / "run.json", run)
        if not args.skip_profiles:
            for n in concurrency:
                for revision in ("baseline", "changed"):
                    result = run_trial(
                        args,
                        owner,
                        fixture,
                        args.output,
                        binaries[revision],
                        revision,
                        n,
                        1,
                        "profile",
                        profile=True,
                    )
                    run["results"].append(result)
                    write_json(args.output / "run.json", run)
        run["summary"] = aggregate(run["results"])
        run["finished_at"] = now_iso()
        write_json(args.output / "run.json", run)
        write_json(args.output / "summary.json", run["summary"])
        return 0 if run["summary"]["acceptance_eligible"] else 2
    finally:
        owner.cleanup()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except Exception as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
