#!/usr/bin/env python3
"""Reproducible PostgreSQL mixed-batch grouping benchmark for issue #1456.

The controller owns one PostgreSQL container, a loopback fixture server, and
one HFS process at a time. It never stops a process or container it did not
start. Use ``--dry-run`` to inspect the complete default 42-trial matrix.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import http.server
import io
import json
import os
import re
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
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Optional


BASE_COMMIT = "3c09d6a87a80a16d9cb4198fd9cafef3faea4e2e"
TENANT = "default"
FAMILY = "GroupingAfter"
BEFORE_FAMILY = "GroupingBefore"
NARRATIVE_TERM = "HeliosGroupingNarrative"
SUBMITTER_SYSTEM = "https://helios.software/benchmarks/pgi02"
SUBMITTER_VALUE = "pgi02"
DEFAULT_OUTPUT = "/tmp/hfs-1456-pgi02-r1/benchmark"
DEFAULT_POSTGRES_IMAGE = "postgres:16-alpine"
DEFAULT_RESOURCES = 10_000
DEFAULT_TRIALS = 3
DEFAULT_PG_MAX_CONNECTIONS = 4
DEFAULT_POLL_INTERVAL = 0.25
DEFAULT_SAMPLE_INTERVAL = 0.5
BATCH_SIZE = 100


@dataclass(frozen=True)
class Fixture:
    name: str
    existing_offsets: tuple[int, ...]
    description: str

    def existing(self, index: int) -> bool:
        return index % BATCH_SIZE in self.existing_offsets

    def existing_count(self, resources: int) -> int:
        return sum(1 for index in range(resources) if self.existing(index))

    def fresh_runs_per_full_batch(self) -> int:
        existing = set(self.existing_offsets)
        runs = 0
        in_run = False
        for offset in range(BATCH_SIZE):
            fresh = offset not in existing
            if fresh and not in_run:
                runs += 1
            in_run = fresh
        return runs


FIXTURES = (
    Fixture("existing-000", (), "all fresh control"),
    Fixture("existing-001-edge", (0,), "one existing id at the batch edge"),
    Fixture("existing-001-center", (50,), "one existing id at the batch center"),
    Fixture(
        "existing-010-center-clustered",
        tuple(range(45, 55)),
        "ten contiguous existing ids centered in each batch",
    ),
    Fixture(
        "existing-010-spaced",
        tuple(range(0, 100, 10)),
        "ten evenly spaced existing ids in each batch",
    ),
    Fixture(
        "existing-050-alternating",
        tuple(range(0, 100, 2)),
        "alternating existing and fresh ids",
    ),
    Fixture("existing-100", tuple(range(100)), "all existing control"),
)


class BenchmarkError(RuntimeError):
    pass


def run(
    argv: Iterable[str],
    *,
    cwd: Optional[Path] = None,
    timeout: float = 120.0,
    input_bytes: Optional[bytes] = None,
    check: bool = True,
) -> subprocess.CompletedProcess:
    command = list(argv)
    result = subprocess.run(
        command,
        cwd=cwd,
        input=input_bytes,
        capture_output=True,
        timeout=timeout,
        check=False,
    )
    if check and result.returncode != 0:
        stderr = result.stderr.decode("utf-8", "replace")[-4000:]
        raise BenchmarkError(f"command failed ({result.returncode}): {' '.join(command)}\n{stderr}")
    return result


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True, default=str) + "\n")
    os.replace(temporary, path)


def source_fingerprint(repo: Path) -> dict[str, Any]:
    def git(*args: str) -> bytes:
        return run(["git", *args], cwd=repo, timeout=60).stdout

    head = git("rev-parse", "HEAD").decode().strip()
    branch = git("rev-parse", "--abbrev-ref", "HEAD").decode().strip()
    tracked = git("diff", "--binary", "HEAD", "--", ".")
    untracked_names = [
        item.decode("utf-8")
        for item in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0")
        if item
    ]
    tracked_digest = hashlib.sha256()
    tracked_digest.update(b"hfs-tracked-source-fingerprint-v1\0")
    tracked_digest.update(head.encode())
    tracked_digest.update(b"\0tracked-diff\0")
    tracked_digest.update(tracked)
    digest = tracked_digest.copy()
    digest.update(b"\0untracked-section\0")
    untracked: list[dict[str, Any]] = []
    for relative in sorted(untracked_names):
        path = repo / relative
        content = os.readlink(path).encode() if path.is_symlink() else path.read_bytes()
        encoded = relative.encode()
        digest.update(b"\0untracked\0")
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
        untracked.append(
            {"path": relative, "bytes": len(content), "sha256": sha256_bytes(content)}
        )
    return {
        "head": head,
        "branch": branch,
        "dirty": bool(tracked or untracked),
        "tracked_diff_bytes": len(tracked),
        "tracked_diff_sha256": sha256_bytes(tracked),
        "tracked_source_fingerprint_sha256": tracked_digest.hexdigest(),
        "untracked": untracked,
        "full_worktree_fingerprint_sha256": digest.hexdigest(),
    }


def arm_order(fixture_index: int, trial: int) -> tuple[str, str]:
    """Counterbalance second-position effects across paired acceptance fixtures."""
    baseline_starts_odd_trials = fixture_index % 2 == 0 and fixture_index != len(FIXTURES) - 1
    baseline_first = baseline_starts_odd_trials if trial % 2 else not baseline_starts_odd_trials
    return ("baseline", "candidate") if baseline_first else ("candidate", "baseline")


def port_is_free(port: int, host: str = "127.0.0.1") -> bool:
    probe = socket.socket()
    probe.settimeout(0.35)
    try:
        probe.connect((host, port))
    except OSError:
        pass
    else:
        return False
    finally:
        probe.close()
    holder = socket.socket()
    holder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        holder.bind((host, port))
    except OSError:
        return False
    finally:
        holder.close()
    return True


def pick_port(preferred: int) -> int:
    for port in range(preferred, preferred + 500):
        if port_is_free(port):
            return port
    raise BenchmarkError(f"no free loopback port in {preferred}..{preferred + 499}")


def patient_id(index: int) -> str:
    return f"pgi02-{index:05d}"


def patient(index: int, *, before: bool = False) -> dict[str, Any]:
    family = BEFORE_FAMILY if before else FAMILY
    phase = "Before" if before else "After"
    return {
        "resourceType": "Patient",
        "id": patient_id(index),
        "identifier": [
            {"system": "https://helios.software/benchmarks/pgi02/mrn", "value": f"MRN-{index:05d}"}
        ],
        "active": index % 2 == 0,
        "name": [{"family": family, "given": ["Mixed", f"P{index:05d}"]}],
        "birthDate": "1980-01-01",
        "text": {
            "status": "generated",
            "div": (
                '<div xmlns="http://www.w3.org/1999/xhtml">'
                f"{NARRATIVE_TERM} {phase} {index:05d}</div>"
            ),
        },
    }


def expected_statement_counts(fixture: Fixture, resources: int, arm: str) -> dict[str, int]:
    batches = resources // BATCH_SIZE
    existing = fixture.existing_count(resources)
    fresh = resources - existing
    baseline_fallback = arm == "baseline" and existing > 0
    savepoints = resources if baseline_fallback else existing
    if existing == 0:
        inserts = batches
    elif existing == resources:
        inserts = 0
    elif arm == "baseline":
        inserts = fresh
    else:
        inserts = fixture.fresh_runs_per_full_batch() * batches
    return {
        "classification": batches,
        "point_read": savepoints,
        "savepoint": savepoints,
        "release_savepoint": savepoints,
        "rollback_to_savepoint": 0,
        "resource_insert": inserts,
    }


def request(
    method: str,
    url: str,
    payload: Optional[dict[str, Any]] = None,
    timeout: float = 60.0,
) -> dict[str, Any]:
    body = None if payload is None else json.dumps(payload).encode()
    headers = {"Accept": "application/fhir+json"}
    if body is not None:
        headers["Content-Type"] = "application/fhir+json"
    req = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            raw = response.read()
            status = response.status
            response_headers = {key.lower(): value for key, value in response.headers.items()}
    except urllib.error.HTTPError as error:
        raw = error.read()
        status = error.code
        response_headers = {key.lower(): value for key, value in error.headers.items()}
    except Exception as error:
        return {"status": 0, "headers": {}, "body": b"", "json": None, "error": str(error)}
    try:
        decoded = json.loads(raw) if raw.strip() else None
    except json.JSONDecodeError:
        decoded = None
    return {
        "status": status,
        "headers": response_headers,
        "body": raw,
        "json": decoded,
        "error": None,
    }


class Provider:
    def __init__(self, directory: Path, port: int):
        handler = lambda *args, **kwargs: http.server.SimpleHTTPRequestHandler(  # noqa: E731
            *args, directory=str(directory), **kwargs
        )
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.url = f"http://127.0.0.1:{port}"

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=10)


class OwnedHfs:
    def __init__(self, binary: Path, repo: Path, env: dict[str, str], log: Path):
        self.log_handle = log.open("wb")
        self.process = subprocess.Popen(
            [str(binary), "--host", "127.0.0.1", "--port", env["HFS_SERVER_PORT"], "--log-level", "info"],
            cwd=repo,
            env=env,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.log = log

    @property
    def pid(self) -> int:
        return self.process.pid

    def stop(self) -> None:
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait(timeout=10)
        self.log_handle.close()


class Sampler:
    def __init__(self, hfs_pid: int, container: str, destination: Path, interval: float):
        self.pid = hfs_pid
        self.container = container
        self.destination = destination
        self.interval = interval
        self.stop_event = threading.Event()
        self.rows: list[dict[str, Any]] = []
        self.thread = threading.Thread(target=self._loop, daemon=True)

    def start(self) -> None:
        self.thread.start()

    def stop(self, measurement_started: float, measurement_ended: float) -> dict[str, Any]:
        self.stop_event.set()
        self.thread.join(timeout=max(45.0, self.interval * 3))
        if self.thread.is_alive():
            raise BenchmarkError("memory sampler did not stop after its bounded command timeout")
        # An explicit endpoint sample makes coverage independently verifiable
        # even when search readiness lands just after a periodic sample.
        self._sample_once()
        with self.destination.open("w", newline="") as handle:
            writer = csv.DictWriter(
                handle,
                fieldnames=(
                    "wall_time",
                    "sample_started_monotonic",
                    "sample_completed_monotonic",
                    "hfs_rss_mib",
                    "postgres_memory_mib",
                ),
            )
            writer.writeheader()
            writer.writerows(self.rows)
        hfs = [row["hfs_rss_mib"] for row in self.rows if row["hfs_rss_mib"] is not None]
        postgres = [
            row["postgres_memory_mib"]
            for row in self.rows
            if row["postgres_memory_mib"] is not None
        ]
        gaps = [
            self.rows[index]["sample_started_monotonic"]
            - self.rows[index - 1]["sample_started_monotonic"]
            for index in range(1, len(self.rows))
        ]
        maximum_gap = max(gaps) if gaps else None
        gap_limit = max(5.0, self.interval * 4)
        first_offset = (
            self.rows[0]["sample_started_monotonic"] - measurement_started
            if self.rows
            else None
        )
        final_offset = (
            self.rows[-1]["sample_started_monotonic"] - measurement_ended
            if self.rows
            else None
        )
        coverage_ok = (
            len(self.rows) >= 2
            and first_offset is not None
            and first_offset <= gap_limit
            and final_offset is not None
            and 0 <= final_offset <= gap_limit
            and maximum_gap is not None
            and maximum_gap <= gap_limit
        )
        return {
            "samples": len(self.rows),
            "hfs_peak_mib": max(hfs) if hfs else None,
            "postgres_peak_mib": max(postgres) if postgres else None,
            "maximum_gap_seconds": maximum_gap,
            "coverage": {
                "ok": coverage_ok,
                "thread_finished": not self.thread.is_alive(),
                "first_sample_offset_seconds": first_offset,
                "final_sample_offset_seconds": final_offset,
                "maximum_gap_limit_seconds": gap_limit,
                "measurement_duration_seconds": measurement_ended - measurement_started,
            },
        }

    @staticmethod
    def _memory_mib(value: str) -> Optional[float]:
        match = re.match(r"\s*([0-9.]+)\s*([KMG]i?B)", value, re.IGNORECASE)
        if not match:
            return None
        number = float(match.group(1))
        unit = match.group(2).lower()
        if unit.startswith("k"):
            return number / 1024
        if unit.startswith("g"):
            return number * 1024
        return number

    def _loop(self) -> None:
        while not self.stop_event.is_set():
            self._sample_once()
            self.stop_event.wait(self.interval)

    def _sample_once(self) -> None:
        started = time.monotonic()
        try:
            rss = run(
                ["ps", "-o", "rss=", "-p", str(self.pid)], check=False, timeout=10
            ).stdout.decode().strip()
        except Exception:
            rss = ""
        hfs_rss = float(rss) / 1024 if rss.isdigit() else None
        try:
            stats = run(
                ["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", self.container],
                check=False,
                timeout=20,
            ).stdout.decode().strip()
        except Exception:
            stats = ""
        pg_memory = self._memory_mib(stats.split("/")[0]) if stats else None
        self.rows.append(
            {
                "wall_time": time.time(),
                "sample_started_monotonic": started,
                "sample_completed_monotonic": time.monotonic(),
                "hfs_rss_mib": hfs_rss,
                "postgres_memory_mib": pg_memory,
            }
        )


class Controller:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.output = Path(args.output_dir).expanduser().resolve()
        self.baseline_repo = Path(args.baseline_repo).expanduser().resolve()
        self.candidate_repo = Path(args.candidate_repo).expanduser().resolve()
        self.baseline_binary = Path(args.baseline_binary).expanduser().resolve()
        self.candidate_binary = Path(args.candidate_binary).expanduser().resolve()
        self.controller_path = Path(__file__).resolve()
        self.container: Optional[str] = None
        self.hfs: Optional[OwnedHfs] = None
        self.provider: Optional[Provider] = None
        self.pg_port = pick_port(args.pg_port)
        self.provider_port = pick_port(args.provider_port)
        if self.provider_port == self.pg_port:
            self.provider_port = pick_port(self.provider_port + 1)
        self.run_token = uuid.uuid4().hex[:10]
        self.db_prefix = f"pgi02_{self.run_token}"
        self.password = uuid.uuid4().hex
        self.results: list[dict[str, Any]] = []
        self.expected_arms: Optional[dict[str, Any]] = None
        self.expected_controller_sha256: Optional[str] = None
        self.provenance_checks: list[dict[str, Any]] = []

    def acceptance_configuration(self) -> dict[str, Any]:
        fields = {
            "resources": (self.args.resources, DEFAULT_RESOURCES),
            "trials": (self.args.trials, DEFAULT_TRIALS),
            "postgres_image": (self.args.postgres_image, DEFAULT_POSTGRES_IMAGE),
            "hfs_pool_max_connections": (
                self.args.pg_max_connections,
                DEFAULT_PG_MAX_CONNECTIONS,
            ),
            "poll_interval_seconds": (self.args.poll_interval, DEFAULT_POLL_INTERVAL),
            "sample_interval_seconds": (self.args.sample_interval, DEFAULT_SAMPLE_INTERVAL),
        }
        actual = {name: values[0] for name, values in fields.items()}
        canonical = {name: values[1] for name, values in fields.items()}
        mismatches = {
            name: {"actual": values[0], "canonical": values[1]}
            for name, values in fields.items()
            if values[0] != values[1]
        }
        return {
            "enforced": not mismatches,
            "actual": actual,
            "canonical": canonical,
            "mismatches": mismatches,
        }

    def acceptance_run(self) -> bool:
        return bool(self.acceptance_configuration()["enforced"])

    def validate_inputs(self) -> None:
        if self.args.resources < BATCH_SIZE or self.args.resources % BATCH_SIZE != 0:
            raise BenchmarkError(f"--resources must be a multiple of {BATCH_SIZE} and at least {BATCH_SIZE}")
        if self.args.trials < 1:
            raise BenchmarkError("--trials must be at least 1")
        if self.args.poll_interval <= 0 or self.args.sample_interval <= 0:
            raise BenchmarkError("--poll-interval and --sample-interval must be greater than zero")
        if self.args.pg_max_connections < 1:
            raise BenchmarkError("--pg-max-connections must be at least 1")
        for path, label in (
            (self.baseline_repo, "baseline repo"),
            (self.candidate_repo, "candidate repo"),
            (self.baseline_binary, "baseline binary"),
            (self.candidate_binary, "candidate binary"),
        ):
            if not path.exists():
                raise BenchmarkError(f"{label} does not exist: {path}")
        for repo in (self.baseline_repo, self.candidate_repo):
            if self.output == repo or repo in self.output.parents:
                raise BenchmarkError("--output-dir must be outside both source repositories")
        if not os.access(self.baseline_binary, os.X_OK) or not os.access(self.candidate_binary, os.X_OK):
            raise BenchmarkError("both HFS binaries must be executable")
        registry_a = self.baseline_repo / "data/search-parameters-r4.json"
        registry_b = self.candidate_repo / "data/search-parameters-r4.json"
        if not registry_a.is_file() or not registry_b.is_file():
            raise BenchmarkError("both repos must contain data/search-parameters-r4.json")
        if sha256_file(registry_a) != sha256_file(registry_b):
            raise BenchmarkError("baseline and candidate R4 search registries differ")
        baseline_head = run(
            ["git", "rev-parse", "HEAD"], cwd=self.baseline_repo, timeout=60
        ).stdout.decode().strip()
        if baseline_head != BASE_COMMIT:
            raise BenchmarkError(
                f"baseline repo must be at {BASE_COMMIT}, found {baseline_head}"
            )
        baseline_source = source_fingerprint(self.baseline_repo)
        if baseline_source["tracked_diff_bytes"] != 0:
            raise BenchmarkError("baseline repo has tracked changes; the baseline must be unchanged")
        candidate_ancestor = run(
            ["git", "merge-base", "--is-ancestor", BASE_COMMIT, "HEAD"],
            cwd=self.candidate_repo,
            timeout=60,
            check=False,
        )
        if candidate_ancestor.returncode != 0:
            raise BenchmarkError(f"candidate HEAD does not descend from base commit {BASE_COMMIT}")

    def plan(self) -> dict[str, Any]:
        registry_a = self.baseline_repo / "data/search-parameters-r4.json"
        registry_b = self.candidate_repo / "data/search-parameters-r4.json"
        arms = {
            "baseline": self._arm_metadata(self.baseline_repo, self.baseline_binary, registry_a),
            "candidate": self._arm_metadata(self.candidate_repo, self.candidate_binary, registry_b),
        }
        fixtures = []
        trial_matrix = []
        for fixture_index, fixture in enumerate(FIXTURES):
            fixtures.append(
                {
                    "name": fixture.name,
                    "description": fixture.description,
                    "existing_per_batch": len(fixture.existing_offsets),
                    "existing_total": fixture.existing_count(self.args.resources),
                    "fresh_runs_per_batch": fixture.fresh_runs_per_full_batch(),
                    "expected_statements": {
                        arm: expected_statement_counts(fixture, self.args.resources, arm)
                        for arm in ("baseline", "candidate")
                    },
                }
            )
            for trial in range(1, self.args.trials + 1):
                order = arm_order(fixture_index, trial)
                for position, arm in enumerate(order, start=1):
                    trial_matrix.append(
                        {
                            "fixture": fixture.name,
                            "trial": trial,
                            "position": position,
                            "arm": arm,
                        }
                    )
        controller_sha256 = sha256_file(self.controller_path)
        self.expected_arms = arms
        self.expected_controller_sha256 = controller_sha256
        acceptance = self.acceptance_configuration()
        return {
            "schema_version": 1,
            "issue": 1456,
            "base_commit": BASE_COMMIT,
            "dry_run": bool(self.args.dry_run),
            "controller": {
                "path": str(self.controller_path),
                "sha256": controller_sha256,
            },
            "output_dir": str(self.output),
            "resources": self.args.resources,
            "batch_size": BATCH_SIZE,
            "trials_per_arm_fixture": self.args.trials,
            "matrix_trials": len(FIXTURES) * 2 * self.args.trials,
            "trial_matrix": trial_matrix,
            "arm_order_policy": (
                "odd-trial starting arm is counterbalanced by fixture; "
                "the 0/100 controls and 1-edge/1-center gate pairs use opposite majorities"
            ),
            "fixtures": fixtures,
            "arms": arms,
            "postgres": {
                "image": self.args.postgres_image,
                "port": self.pg_port,
                "shared_preload_libraries": "pg_stat_statements",
                "pg_stat_statements.track": "all",
                "pg_stat_statements.track_utility": "on",
                "hfs_pool_max_connections": self.args.pg_max_connections,
            },
            "runtime": self.hfs_settings("http://127.0.0.1:PORT", "DATABASE", Path("ARTIFACTS")),
            "gates": {
                **acceptance,
                "existing-001-edge": "candidate median terminal time <= 90% of baseline",
                "existing-001-center": "candidate median terminal time <= 90% of baseline",
                "existing-000": "candidate median terminal time and HFS peak RSS <= 105% of baseline",
                "existing-100": "candidate median terminal time and HFS peak RSS <= 105% of baseline",
            },
        }

    def verify_provenance(self, stage: str) -> None:
        if self.expected_arms is None or self.expected_controller_sha256 is None:
            raise BenchmarkError("provenance baseline was not initialized")
        current_controller = sha256_file(self.controller_path)
        failures = []
        current_arms: dict[str, Any] = {}
        for arm, repo, binary in (
            ("baseline", self.baseline_repo, self.baseline_binary),
            ("candidate", self.candidate_repo, self.candidate_binary),
        ):
            registry = repo / "data/search-parameters-r4.json"
            current = self._arm_metadata(repo, binary, registry)
            current_arms[arm] = current
            expected = self.expected_arms[arm]
            comparisons = (
                ("tracked source fingerprint", current["source"]["tracked_source_fingerprint_sha256"], expected["source"]["tracked_source_fingerprint_sha256"]),
                ("binary SHA-256", current["binary"]["sha256"], expected["binary"]["sha256"]),
                ("binary size", current["binary"]["bytes"], expected["binary"]["bytes"]),
                ("registry SHA-256", current["registry"]["sha256"], expected["registry"]["sha256"]),
            )
            for label, actual, wanted in comparisons:
                if actual != wanted:
                    failures.append(f"{arm} {label} changed")
        if current_controller != self.expected_controller_sha256:
            failures.append("controller SHA-256 changed")
        check = {
            "stage": stage,
            "controller_sha256": current_controller,
            "arms": current_arms,
            "untracked_policy": (
                "recorded in each full_worktree_fingerprint_sha256 but informational; "
                "HEAD plus tracked diff is the enforced source identity"
            ),
            "ok": not failures,
            "failures": failures,
        }
        self.provenance_checks.append(check)
        if self.output.exists():
            atomic_json(self.output / "provenance-checks.json", self.provenance_checks)
        if failures:
            raise BenchmarkError("provenance drift: " + "; ".join(failures))

    def _arm_metadata(self, repo: Path, binary: Path, registry: Path) -> dict[str, Any]:
        registry_data = json.loads(registry.read_text())
        registry_entries = (
            len(registry_data)
            if isinstance(registry_data, list)
            else len(registry_data.get("entry", []))
            if isinstance(registry_data, dict) and isinstance(registry_data.get("entry"), list)
            else None
        )
        return {
            "repo": str(repo),
            "source": source_fingerprint(repo),
            "binary": {
                "path": str(binary),
                "sha256": sha256_file(binary),
                "bytes": binary.stat().st_size,
            },
            "registry": {
                "path": str(registry),
                "sha256": sha256_file(registry),
                "entries": registry_entries,
            },
        }

    def hfs_settings(self, base_url: str, database: str, artifacts: Path) -> dict[str, str]:
        return {
            "HFS_STORAGE_BACKEND": "postgres",
            "HFS_DATABASE_URL": database,
            "HFS_BASE_URL": base_url,
            "HFS_SERVER_HOST": "127.0.0.1",
            "HFS_SERVER_PORT": base_url.rsplit(":", 1)[-1],
            "HFS_DEFAULT_TENANT": TENANT,
            "HFS_DEFAULT_FHIR_VERSION": "R4",
            "HFS_BULK_SUBMIT_ENABLED": "true",
            "HFS_BULK_SUBMIT_WORKER_CONCURRENCY": "1",
            "HFS_BULK_SUBMIT_FILE_CONCURRENCY": "1",
            "HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT": "1",
            "HFS_BULK_SUBMIT_BATCH_SIZE": str(BATCH_SIZE),
            "HFS_BULK_SUBMIT_DEFER_INDEXING": "true",
            "HFS_BULK_SUBMIT_SKIP_UNCHANGED": "false",
            "HFS_BULK_SUBMIT_POLL_RATE_LIMIT": "1000000",
            "HFS_BULK_SUBMIT_OUTPUT_BACKEND": "local-fs",
            "HFS_BULK_SUBMIT_OUTPUT_DIR": str(artifacts),
            "HFS_PG_MAX_CONNECTIONS": str(self.args.pg_max_connections),
            "HFS_AUTH_ENABLED": "false",
            "HFS_AUDIT_BACKEND": "none",
            "HFS_UI_ENABLED": "false",
            "HFS_LOG_LEVEL": "info",
            "RUST_LOG": "info,hfs=warn",
        }

    def start_container(self) -> None:
        name = f"hfs-1456-pgi02-{os.getpid()}-{self.run_token}"
        run(
            [
                "docker", "run", "-d", "--name", name,
                "-e", "POSTGRES_USER=helios",
                "-e", f"POSTGRES_PASSWORD={self.password}",
                "-e", "POSTGRES_DB=postgres",
                "-p", f"127.0.0.1:{self.pg_port}:5432",
                self.args.postgres_image,
                "-c", "shared_preload_libraries=pg_stat_statements",
                "-c", "pg_stat_statements.track=all",
                "-c", "pg_stat_statements.track_utility=on",
            ],
            timeout=120,
        )
        self.container = name
        for _ in range(90):
            result = run(
                ["docker", "exec", name, "pg_isready", "-U", "helios", "-d", "postgres"],
                check=False,
                timeout=10,
            )
            if result.returncode == 0:
                break
            time.sleep(1)
        else:
            raise BenchmarkError("owned PostgreSQL container did not become ready")
        settings = self.psql(
            "postgres",
            "SELECT current_setting('shared_preload_libraries'), "
            "current_setting('pg_stat_statements.track'), "
            "current_setting('pg_stat_statements.track_utility');",
        )
        if not settings or settings[0] != ["pg_stat_statements", "all", "on"]:
            raise BenchmarkError(f"unexpected pg_stat_statements settings: {settings}")
        inspected = run(["docker", "inspect", name], timeout=60)
        inspection = json.loads(inspected.stdout)[0]
        version = self.psql("postgres", "SELECT version();")
        atomic_json(
            self.output / "postgres.json",
            {
                "container_name": name,
                "container_id": inspection.get("Id"),
                "image": (inspection.get("Config") or {}).get("Image"),
                "image_id": inspection.get("Image"),
                "server_version": version[0][0] if version else None,
                "settings": {
                    "shared_preload_libraries": settings[0][0],
                    "pg_stat_statements.track": settings[0][1],
                    "pg_stat_statements.track_utility": settings[0][2],
                },
            },
        )

    def psql(self, database: str, sql: str, *, timeout: float = 180.0) -> list[list[str]]:
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        result = run(
            [
                "docker", "exec", "-e", "PGPASSWORD=" + self.password,
                self.container, "psql", "-X", "-q", "-A", "-t", "-F", "\x1f",
                "-v", "ON_ERROR_STOP=1", "-U", "helios", "-d", database, "-c", sql,
            ],
            timeout=timeout,
        )
        return [
            line.split("\x1f")
            for line in result.stdout.decode("utf-8", "replace").splitlines()
            if line.strip()
        ]

    def psql_stdin(self, database: str, sql: bytes, *, timeout: float = 300.0) -> None:
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        run(
            [
                "docker", "exec", "-i", "-e", "PGPASSWORD=" + self.password,
                self.container, "psql", "-X", "-q", "-v", "ON_ERROR_STOP=1",
                "-U", "helios", "-d", database,
            ],
            input_bytes=sql,
            timeout=timeout,
        )

    def create_database(self, database: str) -> None:
        self.psql("postgres", f'CREATE DATABASE "{database}";')

    def drop_database(self, database: str) -> None:
        self.psql(
            "postgres",
            f"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{database}' "
            "AND pid <> pg_backend_pid();",
        )
        self.psql("postgres", f'DROP DATABASE IF EXISTS "{database}";')

    def dump_database(self, database: str, destination: Path) -> None:
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        result = run(
            [
                "docker", "exec", "-e", "PGPASSWORD=" + self.password,
                self.container, "pg_dump", "-U", "helios", "-d", database,
                "--format=custom", "--no-owner", "--no-privileges",
            ],
            timeout=600,
        )
        destination.write_bytes(result.stdout)

    def restore_database(self, database: str, source: Path) -> None:
        self.create_database(database)
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        run(
            [
                "docker", "exec", "-i", "-e", "PGPASSWORD=" + self.password,
                self.container, "pg_restore", "-U", "helios", "-d", database,
                "--no-owner", "--no-privileges", "--exit-on-error",
            ],
            input_bytes=source.read_bytes(),
            timeout=900,
        )

    def database_url(self, database: str) -> str:
        password = urllib.parse.quote(self.password, safe="")
        return f"postgres://helios:{password}@127.0.0.1:{self.pg_port}/{database}"

    def clean_hfs_env(self, settings: dict[str, str]) -> dict[str, str]:
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("HFS_") and key != "HELIOS_OBS_MODE"
        }
        env.update(settings)
        return env

    def start_hfs(self, arm: str, database: str, directory: Path, port: int) -> tuple[OwnedHfs, str]:
        repo = self.baseline_repo if arm == "baseline" else self.candidate_repo
        binary = self.baseline_binary if arm == "baseline" else self.candidate_binary
        url = f"http://127.0.0.1:{port}"
        settings = self.hfs_settings(url, self.database_url(database), directory / "artifacts")
        atomic_json(directory / "hfs-settings.json", {**settings, "HFS_DATABASE_URL": "<redacted>"})
        hfs = OwnedHfs(binary, repo, self.clean_hfs_env(settings), directory / "hfs.log")
        self.hfs = hfs
        for _ in range(self.args.startup_timeout):
            if hfs.process.poll() is not None:
                raise BenchmarkError(f"HFS exited during startup; see {hfs.log}")
            if request("GET", url + "/health", timeout=5)["status"] == 200:
                return hfs, url
            time.sleep(1)
        raise BenchmarkError(f"HFS startup timed out; see {hfs.log}")

    def stop_hfs(self) -> None:
        if self.hfs:
            self.hfs.stop()
            self.hfs = None

    def prepare_schema_dump(self) -> Path:
        directory = self.output / "seeds" / "schema-init"
        directory.mkdir(parents=True, exist_ok=True)
        database = f"{self.db_prefix}_schema"
        self.create_database(database)
        port = pick_port(self.args.hfs_port)
        try:
            self.start_hfs("baseline", database, directory, port)
            self.stop_hfs()
            self.psql(database, "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;")
            destination = self.output / "seeds" / "schema.dump"
            self.dump_database(database, destination)
            return destination
        finally:
            self.stop_hfs()
            self.drop_database(database)

    def seed_fixture_dump(self, fixture: Fixture, schema_dump: Path) -> Path:
        destination = self.output / "seeds" / f"{fixture.name}.dump"
        database = f"{self.db_prefix}_seed_{fixture.name.replace('-', '_')}"
        self.restore_database(database, schema_dump)
        try:
            rows = [
                f"{patient_id(index)}\t{index}"
                for index in range(self.args.resources)
                if fixture.existing(index)
            ]
            if rows:
                sql = (
                    "CREATE TEMP TABLE pgi_seed_ids(id text PRIMARY KEY, ordinal integer NOT NULL);\n"
                    "COPY pgi_seed_ids(id, ordinal) FROM STDIN;\n"
                    + "\n".join(rows)
                    + "\n\\.\n"
                    "WITH seeded AS (\n"
                    "  SELECT id, ordinal, jsonb_build_object(\n"
                    "    'resourceType', 'Patient', 'id', id,\n"
                    "    'identifier', jsonb_build_array(jsonb_build_object(\n"
                    "      'system', 'https://helios.software/benchmarks/pgi02/mrn',\n"
                    "      'value', 'MRN-' || lpad(ordinal::text, 5, '0'))),\n"
                    "    'active', (ordinal % 2 = 0),\n"
                    "    'name', jsonb_build_array(jsonb_build_object(\n"
                    f"      'family', '{BEFORE_FAMILY}', 'given', jsonb_build_array('Mixed', 'P' || lpad(ordinal::text, 5, '0')))),\n"
                    "    'birthDate', '1980-01-01',\n"
                    "    'text', jsonb_build_object(\n"
                    "      'status', 'generated',\n"
                    f"      'div', '<div xmlns=\"http://www.w3.org/1999/xhtml\">{NARRATIVE_TERM} Before ' || lpad(ordinal::text, 5, '0') || '</div>')) AS data\n"
                    "  FROM pgi_seed_ids\n"
                    "), inserted AS (\n"
                    "  INSERT INTO resources\n"
                    "    (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)\n"
                    "  SELECT 'default', 'Patient', id, '1', data, '2024-01-01T00:00:00Z', FALSE, '4.0'\n"
                    "  FROM seeded RETURNING *\n"
                    ")\n"
                    "INSERT INTO resource_history\n"
                    "  (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)\n"
                    "SELECT tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version\n"
                    "FROM inserted;\n"
                ).encode()
                self.psql_stdin(database, sql, timeout=600)
            self.dump_database(database, destination)
            atomic_json(
                destination.with_suffix(".json"),
                {
                    "fixture": fixture.name,
                    "description": fixture.description,
                    "existing_offsets": fixture.existing_offsets,
                    "existing_total": fixture.existing_count(self.args.resources),
                    "sha256": sha256_file(destination),
                },
            )
            return destination
        finally:
            self.drop_database(database)

    def create_corpus(self) -> Path:
        fixtures = self.output / "fixtures"
        fixtures.mkdir(parents=True, exist_ok=True)
        corpus = fixtures / "patients.ndjson"
        with corpus.open("w") as handle:
            for index in range(self.args.resources):
                handle.write(json.dumps(patient(index), separators=(",", ":")) + "\n")
        atomic_json(
            fixtures / "corpus.json",
            {
                "resources": self.args.resources,
                "file": corpus.name,
                "bytes": corpus.stat().st_size,
                "sha256": sha256_file(corpus),
                "family": FAMILY,
                "narrative_term": NARRATIVE_TERM,
            },
        )
        return corpus

    def write_manifest(self, trial_dir: Path) -> str:
        if not self.provider:
            raise BenchmarkError("fixture provider is not running")
        manifest = {
            "transactionTime": "2024-01-01T00:00:00Z",
            "request": self.provider.url + "/manifest.json",
            "requiresAccessToken": False,
            "output": [
                {
                    "type": "Patient",
                    "url": self.provider.url + "/patients.ndjson",
                    "count": self.args.resources,
                }
            ],
            "error": [],
            "deleted": [],
        }
        atomic_json(self.output / "fixtures" / "manifest.json", manifest)
        atomic_json(trial_dir / "input-manifest.json", manifest)
        return self.provider.url + "/manifest.json"

    def stats_snapshot(self, database: str) -> dict[str, float]:
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        result = run(
            [
                "docker", "exec", "-e", "PGPASSWORD=" + self.password,
                self.container, "psql", "-X", "-q", "--csv", "-U", "helios", "-d", database,
                "-c", "SELECT query, calls::double precision AS calls FROM pg_stat_statements "
                "WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())",
            ],
            timeout=120,
        )
        rows = csv.DictReader(io.StringIO(result.stdout.decode("utf-8", "replace")))
        snapshot: dict[str, float] = defaultdict(float)
        for row in rows:
            snapshot[row["query"]] += float(row["calls"])
        return dict(snapshot)

    @staticmethod
    def statement_metrics(before: dict[str, float], after: dict[str, float]) -> dict[str, Any]:
        deltas: dict[str, float] = {}
        for query, calls in after.items():
            delta = calls - before.get(query, 0.0)
            if delta > 0:
                deltas[query] = delta

        def total(predicate) -> int:
            return int(round(sum(calls for query, calls in deltas.items() if predicate(" ".join(query.lower().split())))))

        metrics = {
            "classification": total(
                lambda q: "from resources as resource" in q
                and "candidate(resource_type, id)" in q
                and "join unnest" in q
            ),
            "point_read": total(
                lambda q: q.startswith("select version_id, data, last_updated, is_deleted, fhir_version")
                and "where tenant_id = $1 and resource_type = $2 and id = $3" in q
            ),
            "savepoint": total(lambda q: q == "savepoint bulk_entry"),
            "release_savepoint": total(lambda q: q == "release savepoint bulk_entry"),
            "rollback_to_savepoint": total(lambda q: q == "rollback to savepoint bulk_entry"),
            "resource_insert": total(
                lambda q: "with input (resource_type, id, version_id, data, last_updated, fhir_version)" in q
                and "insert into resources" in q
                and "select resource_type, id from ins" in q
            ),
        }
        return {"available": True, "metrics": metrics, "positive_deltas": deltas}

    def kickoff(self, base_url: str, submission_id: str, manifest_url: str) -> str:
        payload = {
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "submitter",
                    "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": SUBMITTER_VALUE},
                },
                {"name": "submissionId", "valueString": submission_id},
                {"name": "manifestUrl", "valueUrl": manifest_url},
                {"name": "fhirBaseUrl", "valueUrl": self.provider.url + "/fhir"},
                {
                    "name": "submissionStatus",
                    "valueCoding": {
                        "system": "http://hl7.org/fhir/event-status",
                        "code": "completed",
                    },
                },
            ],
        }
        started = request("POST", base_url + "/$bulk-submit", payload, self.args.request_timeout)
        if started["status"] != 200:
            raise BenchmarkError(f"bulk-submit kickoff failed: HTTP {started['status']}")
        status_payload = {
            "resourceType": "Parameters",
            "parameter": [
                payload["parameter"][0],
                payload["parameter"][1],
            ],
        }
        opened = request("POST", base_url + "/$bulk-submit-status", status_payload)
        location = opened["headers"].get("content-location")
        if opened["status"] != 202 or not location:
            raise BenchmarkError(f"bulk-submit status kickoff failed: HTTP {opened['status']}")
        return location

    def poll_terminal(self, location: str, started: float) -> dict[str, Any]:
        deadline = time.monotonic() + self.args.terminal_timeout
        polls = []
        while time.monotonic() < deadline:
            response = request("GET", location, timeout=self.args.request_timeout)
            polls.append(
                {
                    "elapsed": time.monotonic() - started,
                    "status": response["status"],
                    "progress": response["headers"].get("x-progress"),
                }
            )
            if response["status"] == 200 and isinstance(response["json"], dict):
                pages = [response["json"]]
                current = response["json"]
                while True:
                    links = [
                        link for link in current.get("link", [])
                        if link.get("relation") == "next" and link.get("url")
                    ]
                    if not links:
                        break
                    page = request("GET", links[0]["url"], timeout=self.args.request_timeout)
                    if page["status"] != 200 or not isinstance(page["json"], dict):
                        raise BenchmarkError("failed to fetch a terminal manifest page")
                    current = page["json"]
                    pages.append(current)
                return {
                    "seconds": time.monotonic() - started,
                    "polls": polls,
                    "manifest": pages[0],
                    "output": [item for page in pages for item in page.get("output", [])],
                    "outcome": [item for page in pages for item in page.get("outcome", [])],
                    "deleted": [item for page in pages for item in page.get("deleted", [])],
                }
            if response["status"] == 429:
                time.sleep(float(response["headers"].get("retry-after", "1")))
            else:
                time.sleep(self.args.poll_interval)
        raise BenchmarkError("terminal manifest timeout")

    @staticmethod
    def search_count(base_url: str, query: str) -> Optional[int]:
        response = request("GET", base_url + "/Patient?" + query + "&_summary=count&_count=1")
        if response["status"] != 200 or not isinstance(response["json"], dict):
            return None
        total = response["json"].get("total")
        return int(total) if isinstance(total, (int, float)) else None

    def wait_search_ready(self, base_url: str, database: str, started: float) -> dict[str, Any]:
        deadline = time.monotonic() + self.args.search_timeout
        observations = []
        while time.monotonic() < deadline:
            family = self.search_count(base_url, "family=" + urllib.parse.quote(FAMILY))
            narrative = self.search_count(base_url, "_text=" + urllib.parse.quote(NARRATIVE_TERM))
            rows = self.psql(
                database,
                "SELECT "
                "(SELECT COUNT(DISTINCT resource_id) FROM search_index "
                " WHERE tenant_id = 'default' AND resource_type = 'Patient'), "
                "(SELECT COUNT(*) FROM resource_fts "
                " WHERE tenant_id = 'default' AND resource_type = 'Patient');",
            )
            indexed = int(rows[0][0]) if rows else -1
            fts = int(rows[0][1]) if rows else -1
            observations.append(
                {"elapsed": time.monotonic() - started, "family": family, "text": narrative, "indexed": indexed, "fts": fts}
            )
            if family == self.args.resources and narrative == self.args.resources and indexed == self.args.resources and fts == self.args.resources:
                return {"seconds": time.monotonic() - started, "observations": observations}
            time.sleep(self.args.poll_interval)
        raise BenchmarkError("search-ready timeout")

    def fetch_receipts(self, terminal: dict[str, Any]) -> list[str]:
        references = []
        for item in terminal["output"]:
            response = request("GET", item.get("url", ""), timeout=300)
            if response["status"] != 200:
                raise BenchmarkError("receipt artifact download failed")
            for line in response["body"].decode("utf-8", "replace").splitlines():
                if line.strip():
                    parsed = json.loads(line)
                    references.append(parsed.get("reference"))
        return references

    def csv_query(self, database: str, sql: str, *, timeout: float = 600.0) -> list[dict[str, str]]:
        if not self.container:
            raise BenchmarkError("PostgreSQL container is not running")
        result = run(
            [
                "docker", "exec", "-e", "PGPASSWORD=" + self.password,
                self.container, "psql", "-X", "-q", "--csv", "-U", "helios", "-d", database,
                "-c", sql,
            ],
            timeout=timeout,
        )
        return list(csv.DictReader(io.StringIO(result.stdout.decode("utf-8", "replace"))))

    def validate_trial(
        self,
        fixture: Fixture,
        database: str,
        submission_id: str,
        terminal: dict[str, Any],
        base_url: str,
    ) -> dict[str, Any]:
        failures: list[str] = []
        if terminal["outcome"] or terminal["deleted"]:
            failures.append("terminal manifest contains outcome/deleted entries")
        expected_refs = [f"Patient/{patient_id(index)}" for index in range(self.args.resources)]
        references = self.fetch_receipts(terminal)
        if references != expected_refs:
            failures.append("receipt references are not the exact input order")

        resource_rows = self.csv_query(
            database,
            "SELECT id, version_id, is_deleted, fhir_version, data::text AS data FROM resources "
            "WHERE tenant_id = 'default' AND resource_type = 'Patient' ORDER BY id",
        )
        if len(resource_rows) != self.args.resources:
            failures.append(f"resources count is {len(resource_rows)}, expected {self.args.resources}")
        for index, row in enumerate(resource_rows):
            if index >= self.args.resources:
                break
            expected_id = patient_id(index)
            expected_version = "2" if fixture.existing(index) else "1"
            if (
                row["id"] != expected_id
                or row["version_id"] != expected_version
                or row["is_deleted"] != "f"
                or row["fhir_version"] != "4.0"
            ):
                failures.append(f"resource identity/version mismatch at {expected_id}")
                break
            if json.loads(row["data"]) != patient(index):
                failures.append(f"resource content mismatch at {expected_id}")
                break

        history_rows = self.csv_query(
            database,
            "SELECT id, version_id, is_deleted, fhir_version, data::text AS data "
            "FROM resource_history "
            "WHERE tenant_id = 'default' AND resource_type = 'Patient' ORDER BY id, version_id",
        )
        history: dict[str, dict[str, dict[str, str]]] = defaultdict(dict)
        for row in history_rows:
            history[row["id"]][row["version_id"]] = row
        for index in range(self.args.resources):
            identifier = patient_id(index)
            expected_versions = ["1", "2"] if fixture.existing(index) else ["1"]
            actual = history.get(identifier, {})
            if sorted(actual) != expected_versions:
                failures.append(f"history mismatch at {patient_id(index)}")
                break
            expected_content = {
                "1": patient(index, before=fixture.existing(index)),
                **({"2": patient(index)} if fixture.existing(index) else {}),
            }
            if any(
                actual[version]["is_deleted"] != "f"
                or actual[version]["fhir_version"] != "4.0"
                or json.loads(actual[version]["data"]) != expected_content[version]
                for version in expected_versions
            ):
                failures.append(f"history content mismatch at {identifier}")
                break

        entry_rows = self.csv_query(
            database,
            "SELECT line_number, resource_id, created, outcome FROM bulk_entry_results "
            f"WHERE tenant_id = 'default' AND submission_id = '{submission_id}' "
            "ORDER BY file_url, line_number",
        )
        if len(entry_rows) != self.args.resources:
            failures.append("bulk_entry_results count mismatch")
        else:
            for index, row in enumerate(entry_rows):
                created = "f" if fixture.existing(index) else "t"
                if (
                    row["line_number"] != str(index + 1)
                    or row["resource_id"] != patient_id(index)
                    or row["created"] != created
                    or row["outcome"] != "success"
                ):
                    failures.append(f"entry result mismatch at line {index + 1}")
                    break

        manifests = self.csv_query(
            database,
            "SELECT manifest_id, status, total_entries, processed_entries, failed_entries, skipped_entries "
            "FROM bulk_manifests "
            f"WHERE tenant_id = 'default' AND submission_id = '{submission_id}'",
        )
        expected_manifest = {
            "status": "completed",
            "total_entries": str(self.args.resources),
            "processed_entries": str(self.args.resources),
            "failed_entries": "0",
            "skipped_entries": "0",
        }
        manifest_id = manifests[0]["manifest_id"] if len(manifests) == 1 else None
        if len(manifests) != 1 or any(manifests[0].get(key) != value for key, value in expected_manifest.items()):
            failures.append("manifest counters/status mismatch")

        change_rows = self.csv_query(
            database,
            "SELECT manifest_id, resource_type, resource_id, change_type, previous_version, "
            "new_version, previous_content::text AS previous_content "
            "FROM bulk_submission_changes "
            f"WHERE tenant_id = 'default' AND submission_id = '{submission_id}' ORDER BY resource_id",
        )
        if len(change_rows) != self.args.resources:
            failures.append("bulk_submission_changes count mismatch")
        else:
            for index, row in enumerate(change_rows):
                existing = fixture.existing(index)
                if (
                    row["manifest_id"] != manifest_id
                    or row["resource_type"] != "Patient"
                    or row["resource_id"] != patient_id(index)
                ):
                    failures.append(f"change identity mismatch at {patient_id(index)}")
                    break
                if existing:
                    if row["change_type"] != "update" or row["previous_version"] != "1" or row["new_version"] != "2":
                        failures.append(f"update change mismatch at {patient_id(index)}")
                        break
                    if json.loads(row["previous_content"]) != patient(index, before=True):
                        failures.append(f"rollback content mismatch at {patient_id(index)}")
                        break
                elif (
                    row["change_type"] != "create"
                    or row["previous_version"]
                    or row["new_version"] != "1"
                    or row["previous_content"]
                ):
                    failures.append(f"create change mismatch at {patient_id(index)}")
                    break

        family = self.search_count(base_url, "family=" + urllib.parse.quote(FAMILY))
        narrative = self.search_count(base_url, "_text=" + urllib.parse.quote(NARRATIVE_TERM))
        if family != self.args.resources or narrative != self.args.resources:
            failures.append("family or _text search count mismatch")
        return {
            "ok": not failures,
            "failures": failures,
            "receipts": len(references),
            "resources": len(resource_rows),
            "history_rows": len(history_rows),
            "changes": len(change_rows),
            "family_count": family,
            "text_count": narrative,
        }

    def run_trial(self, fixture: Fixture, dump: Path, arm: str, trial: int) -> dict[str, Any]:
        trial_dir = self.output / "trials" / fixture.name / f"trial-{trial:02d}-{arm}"
        trial_dir.mkdir(parents=True, exist_ok=False)
        database = f"{self.db_prefix}_{fixture.name.replace('-', '_')}_{trial}_{arm[0]}"
        self.verify_provenance(f"before {fixture.name} trial {trial} {arm}")
        self.restore_database(database, dump)
        port = pick_port(self.args.hfs_port)
        submission_id = f"{fixture.name}-t{trial:02d}-{arm}-{self.run_token}"
        sampler: Optional[Sampler] = None
        measurement_started: Optional[float] = None
        try:
            hfs, base_url = self.start_hfs(arm, database, trial_dir, port)
            manifest_url = self.write_manifest(trial_dir)
            stats_available = True
            stats_error = None
            try:
                before = self.stats_snapshot(database)
            except Exception as error:
                stats_available = False
                stats_error = str(error)
                before = {}
            started = time.monotonic()
            measurement_started = started
            sampler = Sampler(hfs.pid, self.container or "", trial_dir / "memory.csv", self.args.sample_interval)
            sampler.start()
            location = self.kickoff(base_url, submission_id, manifest_url)
            terminal = self.poll_terminal(location, started)
            try:
                after = self.stats_snapshot(database)
                statements = self.statement_metrics(before, after) if stats_available else {
                    "available": False, "error": stats_error, "metrics": {}
                }
            except Exception as error:
                statements = {"available": False, "error": str(error), "metrics": {}}
            search_ready = self.wait_search_ready(base_url, database, started)
            measurement_ended = time.monotonic()
            memory = sampler.stop(started, measurement_ended)
            sampler = None
            validation = self.validate_trial(fixture, database, submission_id, terminal, base_url)
            expected = expected_statement_counts(fixture, self.args.resources, arm)
            memory_available = (
                memory.get("samples", 0) > 0
                and memory.get("hfs_peak_mib") is not None
                and memory.get("postgres_peak_mib") is not None
                and memory.get("coverage", {}).get("ok") is True
                and memory.get("coverage", {}).get("thread_finished") is True
            )
            comparable = (
                bool(statements.get("available"))
                and statements.get("metrics") == expected
                and memory_available
            )
            comparison_reasons = []
            if not statements.get("available"):
                comparison_reasons.append("pg_stat_statements unavailable")
            elif statements.get("metrics") != expected:
                comparison_reasons.append("statement counts differ from the fixture invariant")
            if not memory_available:
                comparison_reasons.append("memory samples are unavailable or do not cover the measurement window")
            if not validation["ok"]:
                raise BenchmarkError("; ".join(validation["failures"]))
            result = {
                "fixture": fixture.name,
                "arm": arm,
                "trial": trial,
                "database": database,
                "submission_id": submission_id,
                "terminal_seconds": terminal["seconds"],
                "search_ready_seconds": search_ready["seconds"],
                "memory": memory,
                "statement_counts": statements,
                "expected_statement_counts": expected,
                "validation": validation,
                "comparable": comparable,
                "comparison_reasons": comparison_reasons,
                "terminal": terminal,
            }
            atomic_json(trial_dir / "result.json", result)
            print(
                f"{fixture.name} trial={trial} arm={arm} "
                f"terminal={terminal['seconds']:.3f}s ready={search_ready['seconds']:.3f}s "
                f"rss={memory['hfs_peak_mib']} comparable={comparable}",
                flush=True,
            )
            return result
        except Exception as error:
            atomic_json(
                trial_dir / "failure.json",
                {
                    "fixture": fixture.name,
                    "arm": arm,
                    "trial": trial,
                    "database": database,
                    "submission_id": submission_id,
                    "error": str(error),
                },
            )
            raise
        finally:
            if sampler:
                ended = time.monotonic()
                try:
                    sampler.stop(measurement_started or ended, ended)
                except Exception:
                    pass
            self.stop_hfs()
            self.drop_database(database)

    @staticmethod
    def aggregate(values: list[float]) -> dict[str, Optional[float]]:
        if not values:
            return {"median": None, "minimum": None, "maximum": None, "spread": None}
        return {
            "median": statistics.median(values),
            "minimum": min(values),
            "maximum": max(values),
            "spread": max(values) - min(values),
        }

    def summarize_fixture(self, fixture: Fixture) -> dict[str, Any]:
        selected = [result for result in self.results if result["fixture"] == fixture.name]
        arms = {}
        for arm in ("baseline", "candidate"):
            arm_results = [result for result in selected if result["arm"] == arm]
            arms[arm] = {
                "trials": len(arm_results),
                "comparable_trials": sum(1 for result in arm_results if result["comparable"]),
                "terminal_seconds": self.aggregate([result["terminal_seconds"] for result in arm_results]),
                "search_ready_seconds": self.aggregate([result["search_ready_seconds"] for result in arm_results]),
                "hfs_peak_mib": self.aggregate(
                    [result["memory"]["hfs_peak_mib"] for result in arm_results if result["memory"]["hfs_peak_mib"] is not None]
                ),
                "postgres_peak_mib": self.aggregate(
                    [result["memory"]["postgres_peak_mib"] for result in arm_results if result["memory"]["postgres_peak_mib"] is not None]
                ),
            }
        gate_failures = []
        baseline = arms["baseline"]
        candidate = arms["candidate"]
        if baseline["comparable_trials"] != self.args.trials or candidate["comparable_trials"] != self.args.trials:
            gate_failures.append("one or more trials are not comparable")
        if self.acceptance_run() and fixture.name in ("existing-001-edge", "existing-001-center"):
            ratio = candidate["terminal_seconds"]["median"] / baseline["terminal_seconds"]["median"]
            if ratio > 0.90:
                gate_failures.append(f"candidate terminal median ratio {ratio:.4f} exceeds 0.90")
        if self.acceptance_run() and fixture.name in ("existing-000", "existing-100"):
            time_ratio = candidate["terminal_seconds"]["median"] / baseline["terminal_seconds"]["median"]
            baseline_rss = baseline["hfs_peak_mib"]["median"]
            candidate_rss = candidate["hfs_peak_mib"]["median"]
            rss_ratio = (
                candidate_rss / baseline_rss
                if baseline_rss is not None and candidate_rss is not None and baseline_rss > 0
                else None
            )
            if time_ratio > 1.05:
                gate_failures.append(f"candidate terminal median ratio {time_ratio:.4f} exceeds 1.05")
            if rss_ratio is None:
                gate_failures.append("HFS peak RSS median is unavailable")
            elif rss_ratio > 1.05:
                gate_failures.append(f"candidate HFS peak RSS median ratio {rss_ratio:.4f} exceeds 1.05")
        return {"fixture": fixture.name, "arms": arms, "gate_failures": gate_failures}

    def execute(self) -> int:
        self.validate_inputs()
        plan = self.plan()
        if self.args.dry_run:
            print(json.dumps(plan, indent=2, sort_keys=True))
            return 0
        if self.output.exists():
            raise BenchmarkError(f"output directory already exists: {self.output}")
        self.output.mkdir(parents=True)
        atomic_json(self.output / "plan.json", plan)
        self.create_corpus()
        plan["corpus"] = json.loads((self.output / "fixtures" / "corpus.json").read_text())
        atomic_json(self.output / "plan.json", plan)
        self.provider = Provider(self.output / "fixtures", self.provider_port)
        self.provider.start()
        try:
            self.start_container()
            schema_dump = self.prepare_schema_dump()
            summaries = []
            for fixture_index, fixture in enumerate(FIXTURES):
                dump = self.seed_fixture_dump(fixture, schema_dump)
                for trial in range(1, self.args.trials + 1):
                    order = arm_order(fixture_index, trial)
                    for arm in order:
                        result = self.run_trial(fixture, dump, arm, trial)
                        self.results.append(result)
                        atomic_json(self.output / "results.json", self.results)
                summary = self.summarize_fixture(fixture)
                summaries.append(summary)
                atomic_json(self.output / "summary.json", {"fixtures": summaries})
                self.write_markdown(summaries)
                if summary["gate_failures"]:
                    raise BenchmarkError(
                        f"stop gate failed for {fixture.name}: " + "; ".join(summary["gate_failures"])
                    )
            self.verify_provenance("final")
            return 0
        finally:
            self.stop_hfs()
            if self.provider:
                self.provider.stop()
                self.provider = None
            if self.container and not self.args.keep_postgres:
                run(["docker", "rm", "-f", self.container], check=False, timeout=120)
                self.container = None

    def write_markdown(self, summaries: list[dict[str, Any]]) -> None:
        lines = [
            "# PostgreSQL mixed-grouping benchmark summary",
            "",
            f"Base commit: `{BASE_COMMIT}`",
            "",
            "| Fixture | Arm | Terminal median/min/max/spread (s) | Search-ready median (s) | HFS RSS median (MiB) | Comparable |",
            "|---|---|---:|---:|---:|---:|",
        ]
        for summary in summaries:
            for arm in ("baseline", "candidate"):
                data = summary["arms"][arm]
                timing = data["terminal_seconds"]
                lines.append(
                    f"| {summary['fixture']} | {arm} | "
                    f"{timing['median']:.3f}/{timing['minimum']:.3f}/{timing['maximum']:.3f}/{timing['spread']:.3f} | "
                    f"{data['search_ready_seconds']['median']:.3f} | "
                    f"{self._format_number(data['hfs_peak_mib']['median'])} | "
                    f"{data['comparable_trials']}/{data['trials']} |"
                )
            if summary["gate_failures"]:
                lines.extend(["", "Stop gate: " + "; ".join(summary["gate_failures"]), ""])
        (self.output / "summary.md").write_text("\n".join(lines) + "\n")

    @staticmethod
    def _format_number(value: Optional[float]) -> str:
        return "unavailable" if value is None else f"{value:.3f}"


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description="Benchmark PostgreSQL bulk-submit grouping across mixed fresh/existing batches",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    result.add_argument("--baseline-binary", required=True)
    result.add_argument("--baseline-repo", required=True)
    result.add_argument("--candidate-binary", required=True)
    result.add_argument("--candidate-repo", required=True)
    result.add_argument("--output-dir", default=DEFAULT_OUTPUT)
    result.add_argument("--resources", type=int, default=DEFAULT_RESOURCES, help="small validation override; must remain a multiple of 100")
    result.add_argument("--trials", type=int, default=DEFAULT_TRIALS, help="small validation override; default produces 42 trials")
    result.add_argument("--postgres-image", default=DEFAULT_POSTGRES_IMAGE)
    result.add_argument("--pg-port", type=int, default=18456)
    result.add_argument("--provider-port", type=int, default=19456)
    result.add_argument("--hfs-port", type=int, default=18856)
    result.add_argument("--pg-max-connections", type=int, default=DEFAULT_PG_MAX_CONNECTIONS)
    result.add_argument("--startup-timeout", type=int, default=180)
    result.add_argument("--terminal-timeout", type=float, default=3600)
    result.add_argument("--search-timeout", type=float, default=1800)
    result.add_argument("--request-timeout", type=float, default=60)
    result.add_argument("--poll-interval", type=float, default=DEFAULT_POLL_INTERVAL)
    result.add_argument("--sample-interval", type=float, default=DEFAULT_SAMPLE_INTERVAL)
    result.add_argument("--keep-postgres", action="store_true", help="leave only this controller's owned container running")
    result.add_argument("--dry-run", action="store_true")
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        return Controller(args).execute()
    except BenchmarkError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("ERROR: interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
