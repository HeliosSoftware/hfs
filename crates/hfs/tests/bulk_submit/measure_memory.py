#!/usr/bin/env python3
"""Bulk-submit memory benchmark controller for issue #995 (current HEAD).

Drives the release ``hfs`` binary natively on macOS against a dedicated
PostgreSQL container the caller owns, and records **external** observations
only: HFS RSS from ``ps`` every 0.5 s, host memory pressure/swap/compressor
vitals every 5 s, PostgreSQL state through ``docker exec <container> psql``,
and HTTP phase markers with the raw responses behind them.  The server is
never instrumented and no internal phase is claimed; see
``MEMORY_MEASUREMENT.md`` next to this file for the protocol.

A successful attempt requires the deferred reindex to be positively verified:
the ``deferred-index rebuild started job_id=...`` log line, ``$reindex-status``
reporting ``completed`` with ``errorCount == 0`` and ``processed == total``,
and a SQL coverage probe of zero unindexed Patients.  Anything less marks the
attempt unverified, keeps its raw timings labelled incomplete, and stops the
run instead of continuing to the next job.

Safety: starts one HFS process in its own process group plus an in-process
loopback provider, and stops exactly those.  The PostgreSQL container is
inspected and queried, never stopped or reconfigured.  Credentials are never
logged or written to the output directory.
"""

from __future__ import annotations

import argparse
import csv
import functools
import hashlib
import http.server
import json
import os
import re
import signal
import socket
import subprocess
import sys
import threading
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Optional

SCHEMA_VERSION = 1
TENANT = "default"
FAMILY = "Pilot995"
MRN_SYSTEM = "http://helios.example/mrn"
SUBMITTER_SYSTEM = "http://helios.example/bench"
SUBMITTER_VALUE = "mem995"
FIXTURE_FILE_COUNT = 4

EXIT_OK, EXIT_ABORTED, EXIT_CONFIG, EXIT_VALIDATION, EXIT_PREFLIGHT = 0, 2, 3, 4, 5

# Resource-constrained by design: this is not a default-server measurement.
HFS_ENV_BASE = {
    # hfs logs its database URL at info; persistence info still supplies the
    # registry and deferred-reindex markers used by this controller.
    "RUST_LOG": "info,hfs=warn",
    "HFS_STORAGE_BACKEND": "postgres",
    "HFS_DEFAULT_TENANT": TENANT,
    "HFS_DEFAULT_FHIR_VERSION": "R4",
    "HFS_BULK_SUBMIT_ENABLED": "true",
    "HFS_BULK_SUBMIT_WORKER_CONCURRENCY": "1",
    "HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT": "1",
    # Poll rate limit defaults to 10 hits / 60 s, which would quantise the
    # terminal marker to 6 s; it throttles nothing on the ingest path.
    "HFS_BULK_SUBMIT_POLL_RATE_LIMIT": "1000000",
    "HFS_PG_MAX_CONNECTIONS": "4",
    "HFS_MAX_PAGE_SIZE": "1000",
    "HFS_AUTH_ENABLED": "false",
    "HFS_AUDIT_BACKEND": "none",
}


# --------------------------------------------------------------------------
# utilities
# --------------------------------------------------------------------------


def iso_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def mono() -> float:
    return time.monotonic()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def redact_db_url(url: str) -> str:
    try:
        parts = urllib.parse.urlsplit(url)
    except Exception:
        return "<unparsable database url>"
    if not parts.hostname:
        return "<redacted database url>"
    user = parts.username or ""
    auth = f"{urllib.parse.quote(user)}:REDACTED@" if user else ""
    port = f":{parts.port}" if parts.port else ""
    return f"{parts.scheme}://{auth}{parts.hostname}{port}{parts.path}"


def db_url_parts(url: str) -> dict[str, Optional[str]]:
    parts = urllib.parse.urlsplit(url)
    user = urllib.parse.unquote(parts.username) if parts.username else "postgres"
    return {
        "user": user,
        "db": (parts.path or "").lstrip("/") or user,
        "host": parts.hostname,
        "port": parts.port,
    }


def run_capture(cmd: Iterable[str], timeout: float = 60.0) -> subprocess.CompletedProcess:
    return subprocess.run(list(cmd), capture_output=True, text=True, timeout=timeout, check=False)


def pgrep_count(name: str) -> int:
    try:
        proc = run_capture(["pgrep", "-x", name], timeout=10)
    except Exception:
        return 0
    return len([line for line in proc.stdout.splitlines() if line.strip().isdigit()])


def port_is_free(host: str, port: int) -> bool:
    """connect() first: a bind-only probe misreports an active listener."""
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
    try:
        holder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        holder.bind((host, port))
    except OSError:
        return False
    finally:
        holder.close()
    return True


def pick_free_port(preferred: int, host: str = "127.0.0.1", span: int = 400) -> int:
    if preferred and port_is_free(host, preferred):
        return preferred
    start = preferred or 19000
    for candidate in range(start, start + span):
        if port_is_free(host, candidate):
            return candidate
    raise RuntimeError(f"no free loopback port in {start}..{start + span}")


def ps_sample(pid: int) -> Optional[dict[str, float]]:
    try:
        proc = run_capture(["ps", "-o", "rss=,vsz=,pcpu=", "-p", str(pid)], timeout=15)
    except Exception:
        return None
    if proc.returncode != 0:
        return None
    fields = proc.stdout.split()
    if len(fields) < 3:
        return None
    try:
        return {
            "rss_kib": float(fields[0]),
            "vsz_kib": float(fields[1]),
            "cpu_percent": float(fields[2]),
        }
    except ValueError:
        return None


def parse_vm_stat(text: str) -> dict[str, Any]:
    page_size = 16384
    match = re.search(r"page size of (\d+) bytes", text)
    if match:
        page_size = int(match.group(1))
    pages: dict[str, int] = {}
    for line in text.splitlines():
        if ":" not in line:
            continue
        key, _, rest = line.partition(":")
        value = rest.strip().rstrip(".")
        if value.isdigit():
            pages[key.strip().strip('"')] = int(value)

    def mib(name: str) -> Optional[float]:
        value = pages.get(name)
        return None if value is None else value * page_size / 1024 / 1024

    free_spec = pages.get("Pages free", 0) + pages.get("Pages speculative", 0)
    swapouts = pages.get("Swapouts")
    return {
        "page_size": page_size,
        "free_pages": pages.get("Pages free"),
        "speculative_pages": pages.get("Pages speculative"),
        "free_spec_mib": free_spec * page_size / 1024 / 1024,
        "active_mib": mib("Pages active"),
        "inactive_mib": mib("Pages inactive"),
        "wired_mib": mib("Pages wired down"),
        "compressor_mib": mib("Pages occupied by compressor"),
        "compressed_pages": pages.get("Pages stored in compressor"),
        "swapouts_pages": swapouts,
        "swapout_bytes": None if swapouts is None else swapouts * page_size,
        "swapins_pages": pages.get("Swapins"),
    }


def parse_swapusage(text: str) -> dict[str, Optional[float]]:
    def mib(label: str) -> Optional[float]:
        match = re.search(rf"{label}\s*=\s*([0-9.]+)([MG])", text)
        if not match:
            return None
        value = float(match.group(1))
        return value * 1024 if match.group(2) == "G" else value

    return {
        "swap_total_mib": mib("total"),
        "swap_used_mib": mib("used"),
        "swap_free_mib": mib("free"),
    }


def host_vitals() -> dict[str, Any]:
    sample: dict[str, Any] = {"wall": iso_now(), "mono": mono()}
    sample.update(zip(("load1", "load5", "load15"), os.getloadavg()))
    try:
        sample.update(parse_vm_stat(run_capture(["vm_stat"], timeout=20).stdout))
    except Exception as exc:
        sample["vm_stat_error"] = str(exc)
    try:
        sample.update(parse_swapusage(run_capture(["sysctl", "-n", "vm.swapusage"], timeout=20).stdout))
    except Exception as exc:
        sample["swapusage_error"] = str(exc)
    try:
        level = run_capture(["sysctl", "-n", "kern.memorystatus_vm_pressure_level"], timeout=10).stdout.strip()
        sample["pressure_level"] = int(level) if level.isdigit() else None
    except Exception:
        sample["pressure_level"] = None
    return sample


def _round(value: Any, digits: int = 1) -> Optional[float]:
    return None if value is None else round(float(value), digits)


def _mib(value: Any) -> Optional[float]:
    return None if value in (None, 0) else round(int(value) / 1024 / 1024, 1)


def _median(values: list[float]) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return _round(ordered[middle])
    return _round((ordered[middle - 1] + ordered[middle]) / 2)


def _json_array_length(path: Path) -> Optional[int]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None
    if isinstance(payload, list):
        return len(payload)
    if isinstance(payload, dict) and isinstance(payload.get("entry"), list):
        return len(payload["entry"])
    return None


# --------------------------------------------------------------------------
# writers
# --------------------------------------------------------------------------


class CsvWriter:
    def __init__(self, path: Path, columns: list[str]):
        self.columns = columns
        self._lock = threading.Lock()
        self._handle = open(path, "w", encoding="utf-8", newline="")
        self._writer = csv.DictWriter(self._handle, fieldnames=columns, extrasaction="ignore")
        self._writer.writeheader()
        self._handle.flush()

    def write(self, row: dict[str, Any]) -> None:
        with self._lock:
            self._writer.writerow({key: ("" if row.get(key) is None else row.get(key)) for key in self.columns})
            self._handle.flush()

    def close(self) -> None:
        with self._lock:
            self._handle.close()


class JsonlWriter:
    def __init__(self, path: Path):
        self._lock = threading.Lock()
        self._handle = open(path, "w", encoding="utf-8", buffering=1)

    def write(self, record: dict[str, Any]) -> None:
        with self._lock:
            self._handle.write(json.dumps(record, default=str) + "\n")

    def close(self) -> None:
        with self._lock:
            self._handle.close()


def write_json_atomic(path: Path, payload: Any) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")
    os.replace(tmp, path)


class RunLog:
    def __init__(self, path: Path, echo: bool = True):
        self._lock = threading.Lock()
        self._handle = open(path, "a", encoding="utf-8", buffering=1)
        self.echo = echo

    def line(self, event: str, **fields: Any) -> None:
        record = {"wall": iso_now(), "mono": round(mono(), 3), "event": event}
        record.update(fields)
        with self._lock:
            self._handle.write(json.dumps(record, default=str) + "\n")
            self._handle.flush()
            if self.echo:
                detail = " ".join(f"{key}={value}" for key, value in fields.items())
                print(f"[{record['wall']}] {event}{' ' + detail if detail else ''}", flush=True)

    def close(self) -> None:
        with self._lock:
            self._handle.close()


class Aborted(Exception):
    """The measurement cannot produce a verified result; stop the run."""

    def __init__(self, reason: str, detail: Optional[dict[str, Any]] = None):
        super().__init__(reason)
        self.reason = reason
        self.detail = detail or {}


class ConfigError(Exception):
    pass


class PgError(Exception):
    pass


# --------------------------------------------------------------------------
# HFS log follower
# --------------------------------------------------------------------------


class LogFollower:
    """Incremental reader over the HFS log file written by our own process."""

    def __init__(self, path: Path):
        self.path = path
        self.offset = 0
        self._pending = b""

    def seek_end(self) -> int:
        try:
            self.offset = self.path.stat().st_size
        except OSError:
            self.offset = 0
        return self.offset

    def read_new(self) -> list[str]:
        try:
            with open(self.path, "rb") as handle:
                handle.seek(self.offset)
                chunk = handle.read()
                self.offset = handle.tell()
        except OSError:
            return []
        if not chunk:
            return []
        data = self._pending + chunk
        lines = data.split(b"\n")
        self._pending = lines.pop()
        return [line.decode("utf-8", errors="replace") for line in lines]

    def tail(self, max_bytes: int = 6000) -> str:
        try:
            size = self.path.stat().st_size
            with open(self.path, "rb") as handle:
                handle.seek(max(0, size - max_bytes))
                return handle.read().decode("utf-8", errors="replace")
        except OSError:
            return ""


# --------------------------------------------------------------------------
# PostgreSQL through the dedicated container (read-only probes)
# --------------------------------------------------------------------------


class PgClient:
    TABLES = (
        "resources",
        "resource_history",
        "search_index",
        "resource_fts",
        "bulk_submissions",
        "bulk_manifests",
        "bulk_entry_results",
        "bulk_submission_changes",
        "bulk_submit_files",
    )
    UNIT = "\x1f"

    def __init__(self, container: str, user: str, database: str):
        self.container = container
        self.user = user
        self.database = database

    def psql(self, sql: str, timeout: float = 120.0) -> dict[str, Any]:
        cmd = [
            "docker", "exec", "-e", f"PGOPTIONS=-c statement_timeout={max(1, int((timeout - 5) * 1000))}", self.container, "psql",
            "-U", self.user, "-d", self.database,
            "-X", "-q", "-A", "-t", "-F", self.UNIT, "-v", "ON_ERROR_STOP=1", "-c", sql,
        ]
        try:
            proc = run_capture(cmd, timeout=timeout)
        except Exception as exc:
            return {"ok": False, "error": str(exc), "rows": [], "stderr": ""}
        return {
            "ok": proc.returncode == 0,
            "error": None if proc.returncode == 0 else (proc.stderr.strip() or "psql failed"),
            "rows": [line.split(self.UNIT) for line in proc.stdout.splitlines() if line.strip()],
            "stderr": proc.stderr.strip(),
        }

    def require(self, sql: str) -> list[list[str]]:
        result = self.psql(sql)
        if not result["ok"]:
            raise PgError(result["error"] or "psql probe failed")
        return result["rows"]

    def wait_ready(self, attempts: int = 15, interval: float = 2.0) -> bool:
        for _ in range(attempts):
            if self.psql("SELECT 1", timeout=30)["ok"]:
                return True
            time.sleep(interval)
        return False

    def stream_rows(self, sql: str, timeout: float = 1800.0):
        """Stream rows instead of buffering the whole result set in one blob.

        JSONB text output never contains a newline, so a line is a row.
        """
        cmd = [
            "docker", "exec", "-e", f"PGOPTIONS=-c statement_timeout={max(1, int((timeout - 5) * 1000))}", self.container, "psql",
            "-U", self.user, "-d", self.database,
            "-X", "-q", "-A", "-t", "-F", self.UNIT, "-v", "ON_ERROR_STOP=1", "-c", sql,
        ]
        # Spool to disk so timeout covers the query and pipe consumption, while
        # the controller never holds the entire result set in memory.
        with tempfile.TemporaryFile(mode="w+t", encoding="utf-8") as output:
            process = subprocess.run(cmd, stdout=output, stderr=subprocess.PIPE,
                                     text=True, timeout=timeout, check=False)
            if process.returncode != 0:
                raise PgError(process.stderr.strip() or "psql stream failed")
            output.seek(0)
            for line in output:
                line = line.rstrip("\n")
                if line:
                    yield line.split(self.UNIT)

    def existing_tables(self) -> set[str]:
        listing = ", ".join(f"'{name}'" for name in self.TABLES)
        sql = (
            "SELECT table_name FROM information_schema.tables "
            f"WHERE table_schema = 'public' AND table_name IN ({listing});"
        )
        return {row[0] for row in self.require(sql) if row}

    def counts(self) -> dict[str, Any]:
        """Counts and coverage.  Only tables that exist are named in SQL."""
        tables = self.existing_tables()
        parts: list[str] = []
        for table in self.TABLES:
            if table in tables:
                parts.append(f"SELECT '{table}', count(*)::text FROM {table} WHERE tenant_id = '{TENANT}'")
        if "resources" in tables:
            parts.append(
                f"SELECT 'resources_patient', count(*)::text FROM resources "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient'"
            )
            parts.append(
                f"SELECT 'version:' || version_id, count(*)::text FROM resources "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' GROUP BY 1"
            )
        if "resource_history" in tables:
            parts.append(
                f"SELECT 'history:' || version_id, count(*)::text FROM resource_history "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' GROUP BY 1"
            )
        if "resources" in tables and "search_index" in tables:
            # The #903 verdict: a Patient with no index rows is unsearchable.
            parts.append(
                "SELECT 'unindexed_patients', count(*)::text FROM ("
                f"SELECT id FROM resources WHERE tenant_id = '{TENANT}' "
                "AND resource_type = 'Patient' AND is_deleted = FALSE EXCEPT "
                f"SELECT resource_id FROM search_index WHERE tenant_id = '{TENANT}' "
                "AND resource_type = 'Patient') missing"
            )
            parts.append(
                f"SELECT 'search_index_patient', count(*)::text FROM search_index "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient'"
            )
        rows = self.require("\nUNION ALL\n".join(parts) + ";") if parts else []
        flat: dict[str, Any] = {}
        version_spread: dict[str, int] = {}
        history_spread: dict[str, int] = {}
        for row in rows:
            if len(row) < 2:
                continue
            key, value = row[0], row[1]
            try:
                number = int(value)
            except ValueError:
                continue
            if key.startswith("version:"):
                version_spread[key.split(":", 1)[1]] = number
            elif key.startswith("history:"):
                history_spread[key.split(":", 1)[1]] = number
            else:
                flat[key] = number
        return {"counts": flat, "version_spread": version_spread, "history_spread": history_spread}

    def detail(self, resource_ids: list[str], submission_id: str) -> dict[str, Any]:
        """Versions, history and receipt bookkeeping.  Never SELECT * on changes."""
        tables = self.existing_tables()
        ids = ", ".join("'" + rid.replace("'", "") + "'" for rid in resource_ids)
        sid = submission_id.replace("'", "")
        queries: dict[str, str] = {
            "resources": (
                f"SELECT id, version_id, is_deleted, last_updated, fhir_version, data::text FROM resources "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' AND id IN ({ids}) ORDER BY id;"
            ),
            "history": (
                f"SELECT id, version_id, last_updated FROM resource_history "
                f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' AND id IN ({ids}) "
                "ORDER BY id, last_updated, version_id;"
            ),
        }
        if "bulk_submissions" in tables:
            queries["submission"] = (
                f"SELECT submission_id, status, created_at, updated_at, completed_at FROM bulk_submissions "
                f"WHERE tenant_id = '{TENANT}' AND submission_id = '{sid}';"
            )
        if "bulk_manifests" in tables:
            queries["manifests"] = (
                f"SELECT manifest_id, status, total_entries, processed_entries, failed_entries FROM bulk_manifests "
                f"WHERE tenant_id = '{TENANT}' AND submission_id = '{sid}' ORDER BY added_at;"
            )
        if "bulk_entry_results" in tables:
            queries["entry_results"] = (
                f"SELECT outcome, coalesce(created::text, 'null'), count(*) FROM bulk_entry_results "
                f"WHERE tenant_id = '{TENANT}' AND submission_id = '{sid}' GROUP BY 1, 2 ORDER BY 1, 2;"
            )
        if "bulk_submission_changes" in tables:
            queries["changes"] = (
                f"SELECT change_type, coalesce(previous_version, 'null'), new_version, count(*) "
                f"FROM bulk_submission_changes WHERE tenant_id = '{TENANT}' AND submission_id = '{sid}' "
                "GROUP BY 1, 2, 3 ORDER BY 1, 2, 3;"
            )
        if "bulk_submit_files" in tables:
            queries["files"] = (
                f"SELECT file_type, count(*) FROM bulk_submit_files "
                f"WHERE tenant_id = '{TENANT}' AND submission_id = '{sid}' GROUP BY 1 ORDER BY 1;"
            )
        data: dict[str, list[list[str]]] = {}
        for name, sql in queries.items():
            data[name] = self.require(sql)
        return {"tables": sorted(tables), "data": data}


# --------------------------------------------------------------------------
# loopback fixture provider
# --------------------------------------------------------------------------


class FixtureProvider:
    def __init__(self, root: Path, preferred_port: int, log: RunLog, log_path: Path):
        self.root = root
        self.preferred_port = preferred_port
        self.log = log
        self.httpd: Optional[http.server.ThreadingHTTPServer] = None
        self.port = 0
        self._lock = threading.Lock()
        self._handle = open(log_path, "a", encoding="utf-8", buffering=1)

    def _factory(self) -> Callable[..., http.server.SimpleHTTPRequestHandler]:
        provider = self

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, fmt: str, *args: Any) -> None:
                provider._record(f"{self.address_string()} {fmt % args}")

            def log_error(self, fmt: str, *args: Any) -> None:
                provider._record(f"error {self.address_string()} {fmt % args}")

        return functools.partial(Handler, directory=str(self.root))

    def _record(self, message: str) -> None:
        with self._lock:
            self._handle.write(json.dumps({"wall": iso_now(), "message": message}) + "\n")

    def start(self) -> str:
        base = self.preferred_port or 19200
        for candidate in range(base, base + 60):
            if not port_is_free("127.0.0.1", candidate):
                continue
            try:
                self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", candidate), self._factory())
            except OSError:
                continue
            self.port = candidate
            break
        if self.httpd is None:
            raise ConfigError("could not bind a loopback provider port")
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, name="provider", daemon=True).start()
        url = f"http://127.0.0.1:{self.port}"
        self.log.line("provider_started", base=url, root=str(self.root))
        return url

    def stop(self) -> None:
        if self.httpd is not None:
            try:
                self.httpd.shutdown()
                self.httpd.server_close()
            except Exception:
                pass
        with self._lock:
            self._handle.close()


# --------------------------------------------------------------------------
# HFS process
# --------------------------------------------------------------------------


class HfsProcess:
    def __init__(self, argv: list[str], env: dict[str, str], cwd: Path, log_path: Path):
        self.argv = argv
        self.env = env
        self.cwd = cwd
        self.log_path = log_path
        self.popen: Optional[subprocess.Popen] = None
        self.pid: Optional[int] = None
        self._handle = None

    def start(self) -> int:
        self._handle = open(self.log_path, "wb", buffering=0)
        self.popen = subprocess.Popen(
            self.argv, cwd=str(self.cwd), env=self.env,
            stdout=self._handle, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
        self.pid = self.popen.pid
        return self.pid

    def alive(self) -> bool:
        return self.popen is not None and self.popen.poll() is None

    def stop(self, grace_seconds: float = 10.0) -> dict[str, Any]:
        """Terminate exactly this process group; never anything else."""
        result: dict[str, Any] = {"pid": self.pid, "sigkill": False, "exit_code": None}
        if self.popen is not None and self.popen.poll() is None and self.pid:
            for signum, wait in ((signal.SIGTERM, grace_seconds), (signal.SIGKILL, 5.0)):
                try:
                    os.killpg(os.getpgid(self.pid), signum)
                except (ProcessLookupError, PermissionError):
                    break
                result["sigkill"] = signum == signal.SIGKILL
                try:
                    self.popen.wait(timeout=wait)
                    break
                except subprocess.TimeoutExpired:
                    continue
        if self.popen is not None:
            result["exit_code"] = self.popen.poll()
        if self._handle is not None:
            self._handle.close()
            self._handle = None
        return result


def http_request(
    method: str, url: str, body: Optional[bytes] = None, timeout: float = 60.0
) -> dict[str, Any]:
    request = urllib.request.Request(url, data=body, method=method)
    request.add_header("Accept", "application/fhir+json, application/json")
    if body is not None:
        request.add_header("Content-Type", "application/fhir+json")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return {
                "status": response.status,
                "headers": {key.lower(): value for key, value in response.headers.items()},
                "body": response.read(),
                "error": None,
            }
    except urllib.error.HTTPError as exc:
        payload = b""
        try:
            payload = exc.read()
        except Exception:
            pass
        return {
            "status": exc.code,
            "headers": {key.lower(): value for key, value in (exc.headers or {}).items()},
            "body": payload,
            "error": f"HTTP {exc.code}",
        }
    except Exception as exc:
        return {"status": 0, "headers": {}, "body": b"", "error": str(exc)}


def http_json(method: str, url: str, payload: Any = None, timeout: float = 60.0) -> dict[str, Any]:
    body = json.dumps(payload).encode("utf-8") if payload is not None else None
    result = http_request(method, url, body=body, timeout=timeout)
    parsed = None
    if result["body"]:
        try:
            parsed = json.loads(result["body"].decode("utf-8", errors="replace"))
        except json.JSONDecodeError:
            parsed = None
    result["json"] = parsed
    return result


def parameters_map(payload: Any) -> dict[str, Any]:
    values: dict[str, Any] = {}
    for parameter in (payload or {}).get("parameter", []):
        for key, value in parameter.items():
            if key != "name":
                values[parameter.get("name")] = value
    return values


# --------------------------------------------------------------------------
# sampling thread
# --------------------------------------------------------------------------


class Sampler(threading.Thread):
    def __init__(self, ctl: "Controller"):
        super().__init__(name="sampler", daemon=True)
        self.ctl = ctl
        self._stop_event = threading.Event()

    def stop(self) -> None:
        self._stop_event.set()

    def run(self) -> None:
        next_host = 0.0
        docker_thread = None
        if self.ctl.args.docker_stats_interval > 0:
            docker_thread = threading.Thread(target=self.sample_database, daemon=True)
            docker_thread.start()
        while not self._stop_event.is_set():
            now = mono()
            try:
                self.ctl.sample_rss()
                if now >= next_host:
                    self.ctl.sample_host()
                    next_host = now + self.ctl.args.host_interval
                self.ctl.check_deadline()
            except Exception as exc:
                self.ctl.log.line("sampler_error", error=str(exc))
            self._stop_event.wait(self.ctl.args.sample_interval)
        if docker_thread is not None:
            docker_thread.join(timeout=7)

    def sample_database(self) -> None:
        # Docker's stats call can take seconds. It must not block HFS RSS or
        # the host-pressure watchdog.
        while not self._stop_event.is_set():
            try:
                self.ctl.sample_docker_stats()
            except Exception as exc:
                self.ctl.log.line("database_sampler_error", error=str(exc))
            self._stop_event.wait(self.ctl.args.docker_stats_interval)


# --------------------------------------------------------------------------
# controller
# --------------------------------------------------------------------------


RSS_COLUMNS = [
    "wall_iso", "mono_s", "rel_s", "phase", "job", "pid",
    "rss_kib", "rss_mib", "vsz_kib", "cpu_percent",
]
HOST_COLUMNS = [
    "wall_iso", "mono_s", "rel_s", "phase", "job", "page_size", "free_pages",
    "speculative_pages", "free_spec_mib", "active_mib", "inactive_mib", "wired_mib",
    "compressor_mib", "compressed_pages", "swap_total_mib", "swap_used_mib",
    "swap_free_mib", "pressure_level", "swapouts_pages_cum", "swapout_bytes_cum",
    "swapout_bytes_since_start", "swapins_pages_cum", "load1", "load5", "load15",
]
PHASE_COLUMNS = [
    "phase", "job", "wall_iso", "mono_s", "rel_s", "rss_mib", "host_free_spec_mib",
    "host_swap_free_mib", "host_compressor_mib", "pressure_level",
    "swapout_bytes_since_start", "uptime_seconds", "extra_json",
]
DOCKER_COLUMNS = [
    "wall_iso", "mono_s", "rel_s", "phase", "job", "container", "mem_usage_mib",
    "mem_limit_mib", "mem_percent", "cpu_percent", "pids", "block_io", "net_io",
]


class Controller:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.t0 = mono()
        self.run_id = os.urandom(3).hex()
        self.repo_root = (
            Path(args.repo_root).expanduser().resolve()
            if args.repo_root
            else Path(__file__).resolve().parents[4]
        )
        self.out = Path(args.output_dir).expanduser().resolve()
        self.binary = Path(args.binary).expanduser().resolve()
        parts = db_url_parts(args.database_url)
        self.pg = PgClient(
            args.pg_container,
            args.pg_user or str(parts["user"]),
            args.pg_db or str(parts["db"]),
        )

        self.log: Optional[RunLog] = None
        self.provider: Optional[FixtureProvider] = None
        self.hfs: Optional[HfsProcess] = None
        self.follower: Optional[LogFollower] = None
        self.sampler: Optional[Sampler] = None
        self.provider_base: Optional[str] = None
        self.csv_rss: Optional[CsvWriter] = None
        self.csv_host: Optional[CsvWriter] = None
        self.csv_phase: Optional[CsvWriter] = None
        self.csv_docker: Optional[CsvWriter] = None
        self.jsonl_phase: Optional[JsonlWriter] = None
        self.jsonl_pg: Optional[JsonlWriter] = None
        self.jsonl_stops: Optional[JsonlWriter] = None

        self.lock = threading.Lock()
        self.phases: list[dict[str, Any]] = []
        self.stops: list[dict[str, Any]] = []
        self.checks: list[dict[str, Any]] = []
        self.attempts: list[dict[str, Any]] = []
        self.rss_rows: list[dict[str, Any]] = []
        self.preflight_info: dict[str, Any] = {}
        self.fixture_info: dict[str, Any] = {}
        self.current_attempt: Optional[dict[str, Any]] = None
        self.current_phase = "preflight"
        self.current_job = 0
        self.last_host: Optional[dict[str, Any]] = None
        self.swapout_baseline_bytes: Optional[int] = None
        self.rss_baseline_mib: Optional[float] = None
        self.pressure_streak = 0
        self.swap_growth_streak = 0
        self.abort: Optional[dict[str, Any]] = None
        self.signal_number: Optional[int] = None
        self.deadline_mono = self.t0 + args.deadline

    # -- paths / fixtures -------------------------------------------------

    @property
    def base_url(self) -> str:
        return f"http://{self.args.host}:{self.args.hfs_port}"

    @property
    def hfs_dir(self) -> Path:
        return self.out / "hfs"

    @property
    def jobs_dir(self) -> Path:
        return self.out / "jobs"

    @property
    def fixtures_dir(self) -> Path:
        return self.out / "fixtures"

    def patient_id(self, file_index: int, offset: int) -> str:
        return f"p995-{file_index}-{offset}"

    def patient_resource(self, file_index: int, offset: int) -> dict[str, Any]:
        return {
            "resourceType": "Patient",
            "id": self.patient_id(file_index, offset),
            "identifier": [
                {
                    "system": MRN_SYSTEM,
                    "value": f"MRN-{file_index}-{offset}",
                    "use": "official",
                }
            ],
            "active": offset % 2 == 0,
            "name": [{"family": FAMILY, "given": [f"File{file_index}", f"Index{offset}"]}],
            "telecom": [
                {"system": "phone", "value": f"555-{file_index:03d}-{offset:04d}", "use": "home"}
            ],
            "gender": "female" if offset % 2 == 0 else "male",
            "birthDate": ["1970-01-01", "1975-05-05", "1980-09-09", "1985-12-12"][offset % 4],
            "address": [
                {
                    "use": "home",
                    "line": [f"{offset} Memory Way"],
                    "city": "Springfield",
                    "state": "IL",
                    "postalCode": "62701",
                }
            ],
        }

    def file_offsets(self) -> list[list[int]]:
        """Local offsets per file, so ids read `p995-<file>-<offset>`."""
        total = self.args.resources
        per_file = total // FIXTURE_FILE_COUNT
        layout = [list(range(per_file)) for _ in range(FIXTURE_FILE_COUNT)]
        remainder = total - per_file * FIXTURE_FILE_COUNT
        if remainder:
            layout[-1].extend(range(per_file, per_file + remainder))
        return layout

    def expected_ids(self, file_index: int) -> list[str]:
        return [self.patient_id(file_index, offset) for offset in self.file_offsets()[file_index]]

    def expected_total(self) -> int:
        return self.args.resources

    def expected_active(self) -> int:
        return sum(1 for offsets in self.file_offsets() for offset in offsets if offset % 2 == 0)

    def ensure_fixtures(self, job: int) -> str:
        if not self.fixture_info:
            files = []
            for file_index, offsets in enumerate(self.file_offsets()):
                path = self.fixtures_dir / f"patients-{file_index}.ndjson"
                with open(path, "w", encoding="utf-8") as handle:
                    for offset in offsets:
                        handle.write(json.dumps(self.patient_resource(file_index, offset)) + "\n")
                files.append(
                    {
                        "name": path.name,
                        "count": len(offsets),
                        "bytes": path.stat().st_size,
                        "sha256": sha256_file(path),
                        "ids": [self.patient_id(file_index, offset) for offset in offsets],
                    }
                )
            self.fixture_info = {
                "family": FAMILY,
                "mrn_system": MRN_SYSTEM,
                "files": files,
                "total": sum(entry["count"] for entry in files),
                "total_bytes": sum(entry["bytes"] for entry in files),
                "scheme": "identical resources re-submitted with stable ids (reimports)",
            }
        manifest_path = self.fixtures_dir / f"manifest-{job:02d}.json"
        manifest = {
            "transactionTime": "2024-01-01T00:00:00Z",
            "request": f"{self.provider_base}/{manifest_path.name}",
            "requiresAccessToken": False,
            "output": [
                {
                    "type": "Patient",
                    "url": f"{self.provider_base}/{entry['name']}",
                    "count": entry["count"],
                }
                for entry in self.fixture_info["files"]
            ],
            "error": [],
            "deleted": [],
        }
        manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        return f"{self.provider_base}/{manifest_path.name}"

    # -- outputs ----------------------------------------------------------

    def prepare_outputs(self) -> None:
        if self.out.exists():
            raise ConfigError(f"output dir already exists: {self.out} (must be a new path)")
        self.out.mkdir(parents=True)
        for directory in (self.hfs_dir, self.jobs_dir, self.fixtures_dir):
            directory.mkdir(parents=True, exist_ok=True)
        self.log = RunLog(self.out / "controller.log", echo=not self.args.quiet)
        self.csv_rss = CsvWriter(self.out / "rss.csv", RSS_COLUMNS)
        self.csv_host = CsvWriter(self.out / "host.csv", HOST_COLUMNS)
        self.csv_phase = CsvWriter(self.out / "phases.csv", PHASE_COLUMNS)
        self.csv_docker = CsvWriter(self.out / "docker_stats.csv", DOCKER_COLUMNS)
        self.jsonl_phase = JsonlWriter(self.out / "phases.jsonl")
        self.jsonl_pg = JsonlWriter(self.out / "pg_probes.jsonl")
        self.jsonl_stops = JsonlWriter(self.out / "stops.jsonl")
        self.log.line(
            "run_start",
            output_dir=str(self.out),
            database=redact_db_url(self.args.database_url),
            pg_container=self.args.pg_container,
        )

    def write_json(self, relative: str, payload: Any) -> None:
        path = self.out / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        write_json_atomic(path, payload)

    # -- phases / sampling ------------------------------------------------

    def metrics_uptime(self) -> Optional[float]:
        if not (self.hfs and self.hfs.alive()):
            return None
        result = http_request("GET", f"{self.base_url}/metrics", timeout=15)
        if result["status"] != 200:
            return None
        match = re.search(r"uptime_seconds\s+([0-9.]+)", result["body"].decode("utf-8", "replace"))
        return float(match.group(1)) if match else None

    def phase(self, name: str, job: int, extra: Optional[dict[str, Any]] = None) -> dict[str, Any]:
        pid = self.hfs.pid if self.hfs else None
        sample = ps_sample(pid) if pid else None
        host = self.last_host or {}
        swapout_growth = None
        if host.get("swapout_bytes") is not None and self.swapout_baseline_bytes is not None:
            swapout_growth = int(host["swapout_bytes"]) - self.swapout_baseline_bytes
        record = {
            "phase": name,
            "job": job,
            "wall_iso": iso_now(),
            "mono_s": round(mono(), 3),
            "rel_s": round(mono() - self.t0, 3),
            "rss_mib": round(sample["rss_kib"] / 1024, 1) if sample else None,
            "host_free_spec_mib": _round(host.get("free_spec_mib")),
            "host_swap_free_mib": _round(host.get("swap_free_mib")),
            "host_compressor_mib": _round(host.get("compressor_mib")),
            "pressure_level": host.get("pressure_level"),
            "swapout_bytes_since_start": swapout_growth,
            "uptime_seconds": self.metrics_uptime(),
            "extra": extra or {},
        }
        with self.lock:
            self.phases.append(record)
            self.current_phase = name
            self.current_job = job
        if self.csv_phase:
            self.csv_phase.write(
                {
                    **{key: record[key] for key in PHASE_COLUMNS if key != "extra_json"},
                    "extra_json": json.dumps(record["extra"], default=str),
                }
            )
        if self.jsonl_phase:
            self.jsonl_phase.write(record)
        if self.log:
            self.log.line(
                "phase", phase=name, job=job, rss_mib=record["rss_mib"],
                swap_free_mib=record["host_swap_free_mib"], pressure=record["pressure_level"],
            )
        return record

    def sample_rss(self) -> None:
        pid = self.hfs.pid if self.hfs else None
        if not pid:
            return
        sample = ps_sample(pid)
        if sample is None:
            return
        rss_mib = sample["rss_kib"] / 1024
        row = {
            "wall_iso": iso_now(),
            "mono_s": round(mono(), 3),
            "rel_s": round(mono() - self.t0, 3),
            "phase": self.current_phase,
            "job": self.current_job,
            "pid": pid,
            "rss_kib": int(sample["rss_kib"]),
            "rss_mib": round(rss_mib, 3),
            "vsz_kib": int(sample["vsz_kib"]),
            "cpu_percent": sample["cpu_percent"],
        }
        with self.lock:
            self.rss_rows.append(row)
        if self.csv_rss:
            self.csv_rss.write(row)
        if rss_mib > self.args.operational_max_rss_mib:
            self.request_stop(
                {
                    "rule": "rss_operational_max",
                    "observed_mib": round(rss_mib, 1),
                    "threshold_mib": self.args.operational_max_rss_mib,
                }
            )

    def sample_host(self) -> None:
        vitals = host_vitals()
        self.last_host = vitals
        if self.swapout_baseline_bytes is None and vitals.get("swapout_bytes") is not None:
            self.swapout_baseline_bytes = int(vitals["swapout_bytes"])
        growth = None
        if vitals.get("swapout_bytes") is not None and self.swapout_baseline_bytes is not None:
            growth = int(vitals["swapout_bytes"]) - self.swapout_baseline_bytes
        row = {
            "wall_iso": vitals["wall"],
            "mono_s": round(vitals["mono"], 3),
            "rel_s": round(vitals["mono"] - self.t0, 3),
            "phase": self.current_phase,
            "job": self.current_job,
            "page_size": vitals.get("page_size"),
            "free_pages": vitals.get("free_pages"),
            "speculative_pages": vitals.get("speculative_pages"),
            "free_spec_mib": _round(vitals.get("free_spec_mib")),
            "active_mib": _round(vitals.get("active_mib")),
            "inactive_mib": _round(vitals.get("inactive_mib")),
            "wired_mib": _round(vitals.get("wired_mib")),
            "compressor_mib": _round(vitals.get("compressor_mib")),
            "compressed_pages": vitals.get("compressed_pages"),
            "swap_total_mib": _round(vitals.get("swap_total_mib")),
            "swap_used_mib": _round(vitals.get("swap_used_mib")),
            "swap_free_mib": _round(vitals.get("swap_free_mib")),
            "pressure_level": vitals.get("pressure_level"),
            "swapouts_pages_cum": vitals.get("swapouts_pages"),
            "swapout_bytes_cum": vitals.get("swapout_bytes"),
            "swapout_bytes_since_start": growth,
            "swapins_pages_cum": vitals.get("swapins_pages"),
            "load1": vitals.get("load1"),
            "load5": vitals.get("load5"),
            "load15": vitals.get("load15"),
        }
        if self.csv_host:
            self.csv_host.write(row)
        pressure = vitals.get("pressure_level")
        if pressure is not None:
            self.pressure_streak = self.pressure_streak + 1 if pressure != 1 else 0
            if self.pressure_streak >= self.args.pressure_samples:
                self.request_stop(
                    {
                        "rule": "memory_pressure_sustained",
                        "observed": pressure,
                        "threshold_samples": self.args.pressure_samples,
                        "sample": row,
                    }
                )
                return
        if growth is not None:
            self.swap_growth_streak = (
                self.swap_growth_streak + 1
                if growth / 1024 / 1024 > self.args.swap_growth_mib
                else 0
            )
            if self.swap_growth_streak >= self.args.swap_growth_samples:
                self.request_stop(
                    {
                        "rule": "swapout_growth",
                        "observed_mib": round(growth / 1024 / 1024, 1),
                        "threshold_mib": self.args.swap_growth_mib,
                        "sample": row,
                    }
                )

    def sample_docker_stats(self) -> None:
        result = run_capture(
            ["docker", "stats", "--no-stream", "--format", "{{json .}}", self.args.pg_container],
            timeout=5,
        )
        row = {
            "wall_iso": iso_now(),
            "mono_s": round(mono(), 3),
            "rel_s": round(mono() - self.t0, 3),
            "phase": self.current_phase,
            "job": self.current_job,
            "container": self.args.pg_container,
        }
        for line in result.stdout.splitlines():
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            usage, _, limit = (payload.get("MemUsage") or "").partition("/")
            row.update(
                {
                    "mem_usage_mib": _usage_mib(usage),
                    "mem_limit_mib": _usage_mib(limit),
                    "mem_percent": payload.get("MemPerc"),
                    "cpu_percent": payload.get("CPUPerc"),
                    "pids": payload.get("PIDs"),
                    "block_io": payload.get("BlockIO"),
                    "net_io": payload.get("NetIO"),
                }
            )
        if self.csv_docker:
            self.csv_docker.write(row)

    def check_deadline(self) -> None:
        if self.args.deadline > 0 and mono() > self.deadline_mono:
            self.request_stop(
                {
                    "rule": "deadline",
                    "observed_s": round(mono() - self.t0, 1),
                    "threshold_s": self.args.deadline,
                }
            )

    def request_stop(self, event: dict[str, Any]) -> None:
        with self.lock:
            if self.abort is not None:
                return
            record = {
                **event,
                "wall_iso": iso_now(),
                "mono_s": round(mono(), 3),
                "rel_s": round(mono() - self.t0, 3),
                "last_60s": self.rss_rows[-120:],
            }
            self.stops.append(record)
            self.abort = {"reason": f"watchdog:{event['rule']}", "detail": record}
        if self.jsonl_stops:
            self.jsonl_stops.write(record)
        if self.log:
            self.log.line("stop_requested", rule=event["rule"], detail=json.dumps(event, default=str))
        if self.hfs is not None:
            stopped = self.hfs.stop(grace_seconds=5.0)
            if self.log:
                self.log.line("hfs_stopped_by_watchdog", **stopped)

    def raise_if_aborted(self) -> None:
        with self.lock:
            abort = self.abort
        if abort is not None:
            raise Aborted(abort["reason"], abort.get("detail", {}))
        if self.signal_number is not None:
            raise Aborted(f"signal:{self.signal_number}")

    # -- preflight --------------------------------------------------------

    def preflight(self) -> dict[str, Any]:
        if not self.binary.is_file():
            raise ConfigError(f"binary not found: {self.binary}")
        if not os.access(self.binary, os.X_OK):
            raise ConfigError(f"binary is not executable: {self.binary}")
        r4 = self.repo_root / "data" / "search-parameters-r4.json"
        if not r4.is_file():
            raise ConfigError(
                f"missing {r4}: HFS would fall back to the embedded parameter set "
                "(pass --repo-root if the checkout is elsewhere)"
            )
        info: dict[str, Any] = {
            "binary": {
                "path": str(self.binary),
                "sha256": sha256_file(self.binary),
                "size_bytes": self.binary.stat().st_size,
                "mtime": datetime.fromtimestamp(self.binary.stat().st_mtime, timezone.utc)
                .isoformat(timespec="seconds")
                .replace("+00:00", "Z"),
            },
            "controller": {
                "path": str(Path(__file__).resolve()),
                "sha256": sha256_file(Path(__file__).resolve()),
                "python": sys.version,
            },
            "git": {},
            "host": {},
            "search_parameters_file": {
                "path": str(r4),
                "sha256": sha256_file(r4),
                "bytes": r4.stat().st_size,
                "entries": _json_array_length(r4),
            },
            "foreign_processes": {
                "hfs": pgrep_count("hfs"),
                "rustc": pgrep_count("rustc"),
                "cargo": pgrep_count("cargo"),
            },
        }
        for key, cmd in (
            ("head", ["git", "rev-parse", "HEAD"]),
            ("branch", ["git", "rev-parse", "--abbrev-ref", "HEAD"]),
        ):
            try:
                info["git"][key] = run_capture(cmd, timeout=20).stdout.strip()
            except Exception as exc:
                info["git"][key] = f"error: {exc}"
        try:
            info["git"]["dirty"] = bool(run_capture(["git", "status", "--porcelain"], timeout=30).stdout.strip())
        except Exception:
            info["git"]["dirty"] = None
        try:
            info["host"] = {
                "macos": run_capture(["sw_vers", "-productVersion"], timeout=20).stdout.strip(),
                "build": run_capture(["sw_vers", "-buildVersion"], timeout=20).stdout.strip(),
                "cpu": run_capture(["sysctl", "-n", "machdep.cpu.brand_string"], timeout=20).stdout.strip(),
                "ncpu": run_capture(["sysctl", "-n", "hw.ncpu"], timeout=20).stdout.strip(),
                "memsize_bytes": run_capture(["sysctl", "-n", "hw.memsize"], timeout=20).stdout.strip(),
            }
        except Exception as exc:
            info["host"] = {"error": str(exc)}
        info["postgres"] = self._docker_inspect()
        info["postgres"]["psql_ready"] = self.pg.wait_ready()
        if not info["postgres"]["psql_ready"]:
            raise ConfigError(
                f"psql is not usable inside container {self.args.pg_container!r}; "
                "the deferred-reindex verdict requires SQL coverage probes"
            )
        info["db_before"] = self.pg.counts()

        samples = []
        for index in range(self.args.preflight_samples):
            vitals = host_vitals()
            samples.append(
                {
                    "wall_iso": vitals["wall"],
                    "free_spec_mib": _round(vitals.get("free_spec_mib")),
                    "swap_free_mib": _round(vitals.get("swap_free_mib")),
                    "compressor_mib": _round(vitals.get("compressor_mib")),
                    "pressure_level": vitals.get("pressure_level"),
                }
            )
            if self.swapout_baseline_bytes is None and vitals.get("swapout_bytes") is not None:
                self.swapout_baseline_bytes = int(vitals["swapout_bytes"])
            if index + 1 < self.args.preflight_samples:
                time.sleep(self.args.preflight_interval)
        info["preflight_samples"] = samples
        info["preflight_medians"] = {
            key: _median([s[key] for s in samples if s.get(key) is not None])
            for key in ("free_spec_mib", "swap_free_mib", "compressor_mib")
        }

        blocked: list[str] = []
        counts = info["db_before"]["counts"]
        stale = {
            key: counts[key]
            for key in ("resources", "resource_history", "bulk_submissions", "bulk_manifests")
            if isinstance(counts.get(key), int) and counts[key] > 0
        }
        if stale and not self.args.allow_nonempty_db:
            blocked.append("database is not fresh: " + ", ".join(f"{k}={v}" for k, v in stale.items()))
        if info["foreign_processes"]["hfs"]:
            blocked.append(f"another hfs process is running ({info['foreign_processes']['hfs']})")
        if info["foreign_processes"]["rustc"]:
            blocked.append(f"a rustc build is active ({info['foreign_processes']['rustc']} processes)")
        recent = [s["pressure_level"] for s in samples[-3:]]
        if recent and all(level is not None and level != 1 for level in recent):
            blocked.append(f"memory pressure is not normal: {recent}")
        info["blocked"] = bool(blocked)
        info["block_reasons"] = blocked
        self.preflight_info = info
        for reason in blocked:
            self.log.line("preflight_blocked", reason=reason)
        if not blocked:
            self.log.line(
                "preflight_ok",
                swap_free_mib=info["preflight_medians"]["swap_free_mib"],
                free_spec_mib=info["preflight_medians"]["free_spec_mib"],
                compressor_mib=info["preflight_medians"]["compressor_mib"],
            )
        return info

    def _docker_inspect(self) -> dict[str, Any]:
        result = run_capture(
            ["docker", "inspect", "--format", "{{json .}}", self.args.pg_container], timeout=60
        )
        if result.returncode != 0:
            raise ConfigError(
                f"docker inspect {self.args.pg_container!r} failed: {result.stderr.strip()[:200]}"
            )
        payload = json.loads(result.stdout.strip().splitlines()[0])
        host_config = payload.get("HostConfig") or {}
        state = payload.get("State") or {}
        return {
            "container": self.args.pg_container,
            "id": payload.get("Id"),
            "image": (payload.get("Config") or {}).get("Image"),
            "running": state.get("Running"),
            "started_at": state.get("StartedAt"),
            "memory_limit_mib": _mib(host_config.get("Memory")),
            "memory_swap_mib": _mib(host_config.get("MemorySwap")),
            "nano_cpus": host_config.get("NanoCpus"),
            "user": self.pg.user,
            "database": self.pg.database,
        }

    # -- HFS lifecycle ----------------------------------------------------

    def effective_env(self) -> dict[str, str]:
        env = dict(HFS_ENV_BASE)
        env["HFS_BASE_URL"] = self.base_url
        env["HFS_SERVER_HOST"] = self.args.host
        env["HFS_SERVER_PORT"] = str(self.args.hfs_port)
        env["HFS_LOG_LEVEL"] = self.args.hfs_log_level
        env["HFS_BULK_SUBMIT_FILE_CONCURRENCY"] = str(self.args.file_concurrency)
        env["HFS_BULK_SUBMIT_DEFER_INDEXING"] = "true" if self.args.defer_indexing else "false"
        env["HFS_BULK_SUBMIT_OUTPUT_BACKEND"] = "local-fs"
        env["HFS_BULK_SUBMIT_OUTPUT_DIR"] = str(self.out / "artifacts")
        for item in self.args.hfs_env:
            key, _, value = item.partition("=")
            env[key.strip()] = value
        return env

    def start_hfs(self, job: int) -> None:
        # An unrelated shell's HFS settings must not change this experiment.
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("HFS_") and key != "HELIOS_OBS_MODE"}
        overrides = self.effective_env()
        env.update(overrides)
        # Credentials travel in the environment, never in argv.
        env["HFS_DATABASE_URL"] = self.args.database_url
        argv = [
            str(self.binary),
            "--host", self.args.host,
            "--port", str(self.args.hfs_port),
            "--log-level", self.args.hfs_log_level,
        ]
        log_path = self.hfs_dir / f"job{job:02d}.log"
        self.hfs = HfsProcess(argv, env, self.repo_root, log_path)
        pid = self.hfs.start()
        self.follower = LogFollower(log_path)
        self.phase(
            "server_start",
            job,
            {
                "pid": pid,
                "argv": argv,
                "log": str(log_path),
                "env": {
                    key: (redact_db_url(value) if key == "HFS_DATABASE_URL" else value)
                    for key, value in overrides.items()
                },
            },
        )
        deadline = mono() + self.args.startup_timeout
        while True:
            self.raise_if_aborted()
            if not self.hfs.alive():
                raise Aborted("hfs_exited_during_startup", {"log_tail": self.follower.tail()})
            if http_request("GET", f"{self.base_url}/health", timeout=15)["status"] == 200:
                break
            if mono() > deadline:
                raise Aborted("hfs_startup_timeout", {"log_tail": self.follower.tail()})
            time.sleep(1.0)
        sample = ps_sample(pid)
        self.rss_baseline_mib = round(sample["rss_kib"] / 1024, 1) if sample else None
        counts = self.pg.counts()
        self.phase(
            "startup_complete",
            job,
            {
                "rss_baseline_mib": self.rss_baseline_mib,
                "db_counts": counts["counts"],
                "search_parameter_file_entries": self.preflight_info["search_parameters_file"]["entries"],
                "search_parameter_log_lines": [
                    line
                    for line in self.follower.read_new()
                    if "SearchParameter" in line
                ][:5],
            },
        )

    def stop_hfs(self, job: int, reason: str) -> None:
        if self.hfs is None:
            return
        sample = ps_sample(self.hfs.pid) if self.hfs.pid else None
        self.phase(
            "server_stop",
            job,
            {"reason": reason, "rss_mib": round(sample["rss_kib"] / 1024, 1) if sample else None},
        )
        stopped = self.hfs.stop()
        if self.log:
            self.log.line("hfs_stopped", job=job, reason=reason, **stopped)
        self.hfs = None
        self.follower = None

    # -- submit -----------------------------------------------------------

    def search_count(self, query: str) -> Optional[int]:
        """Parameterised searches go through the index; the bare one does not."""
        suffix = f"?{query}&_summary=count&_count=1" if query else "?_summary=count&_count=1"
        result = http_json("GET", f"{self.base_url}/Patient{suffix}")
        with (self.out / "search-checks.jsonl").open("a") as evidence:
            evidence.write(json.dumps({"wall_iso": iso_now(), "query": query,
                "status": result["status"], "error": result["error"],
                "json": result["json"]}) + "\n")
        if result["status"] != 200 or not isinstance(result["json"], dict):
            return None
        total = result["json"].get("total")
        return int(total) if isinstance(total, (int, float)) else None

    def kickoff(self, submission_id: str, manifest_url: str) -> dict[str, Any]:
        payload = {
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "submitter",
                    "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": SUBMITTER_VALUE},
                },
                {"name": "submissionId", "valueString": submission_id},
                {"name": "manifestUrl", "valueUrl": manifest_url},
                {"name": "fhirBaseUrl", "valueUrl": f"{self.provider_base}/fhir"},
                {
                    "name": "submissionStatus",
                    "valueCoding": {
                        "system": "http://hl7.org/fhir/event-status",
                        "code": "completed",
                    },
                },
            ],
        }
        return http_json(
            "POST", f"{self.base_url}/$bulk-submit", payload=payload, timeout=self.args.request_timeout
        )

    def poll_terminal(
        self, job: int, submission_id: str, kickoff_mono: float
    ) -> dict[str, Any]:
        status_payload = {
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "submitter",
                    "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": SUBMITTER_VALUE},
                },
                {"name": "submissionId", "valueString": submission_id},
            ],
        }
        opened = http_json(
            "POST", f"{self.base_url}/$bulk-submit-status", payload=status_payload, timeout=60
        )
        poll_url = opened["headers"].get("content-location")
        if opened["status"] != 202 or not poll_url:
            raise Aborted(
                "submit_status_kickoff_failed",
                {"status": opened["status"], "error": opened["error"]},
            )
        interval = max(0.25, self.args.endpoint_poll_interval)
        deadline = mono() + self.args.terminal_timeout
        polls: list[dict[str, Any]] = []
        while True:
            self.raise_if_aborted()
            if mono() > deadline:
                raise Aborted("submit_terminal_timeout", {"polls": polls[-20:]})
            result = http_json("GET", poll_url, timeout=60)
            polls.append(
                {
                    "elapsed_s": round(mono() - kickoff_mono, 3),
                    "status": result["status"],
                    "x_progress": result["headers"].get("x-progress"),
                    "retry_after": result["headers"].get("retry-after"),
                }
            )
            if result["status"] == 200:
                manifest = result["json"] if isinstance(result["json"], dict) else {}
                pages = [manifest]
                guard = 0
                while guard < 50:
                    guard += 1
                    nexts = [
                        link
                        for link in manifest.get("link", [])
                        if link.get("relation") == "next" and link.get("url")
                    ]
                    if not nexts:
                        break
                    page = http_json("GET", nexts[0]["url"], timeout=60)
                    if page["status"] != 200 or not isinstance(page["json"], dict):
                        raise Aborted("submit_manifest_page_failed", {"url": nexts[0]["url"]})
                    manifest = page["json"]
                    pages.append(manifest)
                return {
                    "poll_url": poll_url,
                    "polls": polls,
                    "pages": len(pages),
                    "manifest": pages[0],
                    "output": [entry for page in pages for entry in page.get("output", [])],
                    "outcome": [entry for page in pages for entry in page.get("outcome", [])],
                    "deleted": [entry for page in pages for entry in page.get("deleted", [])],
                    "terminal_s": round(mono() - kickoff_mono, 3),
                    "job": job,
                }
            if result["status"] == 404:
                raise Aborted("submit_poll_404", {"poll_url": poll_url, "polls": polls[-5:]})
            if result["status"] == 429:
                time.sleep(float(result["headers"].get("retry-after") or 6))
            else:
                time.sleep(interval)

    def receipt_check(self, job: int, output: list[dict[str, Any]]) -> dict[str, Any]:
        """Download every receipt artifact and require the exact aggregate id set.

        The worker writes one receipt per resource *type*, not per input file, so
        the four Patient files come back as a single combined artifact.  The
        verdict is therefore on the union of all output entries.
        """
        expected = {
            self.patient_id(file_index, offset)
            for file_index, offsets in enumerate(self.file_offsets())
            for offset in offsets
        }
        summaries = []
        ids: list[str] = []
        for entry in output:
            url = entry.get("url")
            fetched = http_request("GET", url, timeout=300) if url else {"status": 0, "body": b""}
            lines = [line for line in fetched["body"].decode("utf-8", "replace").splitlines() if line.strip()]
            unparsable = 0
            bad_reference = 0
            for line in lines:
                try:
                    reference = str(json.loads(line).get("reference", ""))
                except json.JSONDecodeError:
                    unparsable += 1
                    continue
                if reference.startswith("Patient/"):
                    ids.append(reference.split("/", 1)[1])
                else:
                    bad_reference += 1
            declared_count = entry.get("count")
            declared_size = entry.get("fileSize")
            summaries.append(
                {
                    "url": url,
                    "type": entry.get("type"),
                    "status": fetched["status"],
                    "lines": len(lines),
                    "bytes": len(fetched["body"]),
                    "sha256": hashlib.sha256(fetched["body"]).hexdigest(),
                    "declared_count": declared_count,
                    "declared_file_size": declared_size,
                    "declared_count_ok": declared_count is None or int(declared_count) == len(lines),
                    "declared_file_size_ok": declared_size is None or int(declared_size) == len(fetched["body"]),
                    "unparsable_lines": unparsable,
                    "bad_reference_lines": bad_reference,
                }
            )
        seen_ids: set[str] = set()
        duplicate_ids: set[str] = set()
        for identifier in ids:
            if identifier in seen_ids:
                duplicate_ids.add(identifier)
            seen_ids.add(identifier)
        duplicates = sorted(duplicate_ids)
        result = {
            "ok": (
                len(summaries) >= 1
                and all(item["status"] == 200 for item in summaries)
                and all(item["unparsable_lines"] == 0 for item in summaries)
                and all(item["bad_reference_lines"] == 0 for item in summaries)
                and all(item["declared_count_ok"] for item in summaries)
                and all(item["declared_file_size_ok"] for item in summaries)
                and not duplicates
                and len(ids) == self.expected_total()
                and set(ids) == expected
            ),
            "output_entries": len(summaries),
            "references": len(ids),
            "unique_references": len(set(ids)),
            "duplicates": duplicates[:5],
            "expected_references": len(expected),
            "artifacts": summaries,
        }
        self.write_json(f"jobs/job{job:02d}/receipts.json", result)
        if not result["ok"]:
            raise Aborted("receipt_mismatch", result)
        return result

    # -- reindex verification ---------------------------------------------

    def verify_reindex(
        self, job: int, submission_id: str, log_offset: int, kickoff_mono: float
    ) -> dict[str, Any]:
        """Positive verification or the attempt is unverified and the run stops."""
        total = self.expected_total()
        evidence: dict[str, Any] = {
            "defer_indexing": bool(self.args.defer_indexing),
            "log_offset_at_kickoff": log_offset,
        }
        if self.args.defer_indexing:
            deadline = mono() + self.args.reindex_timeout
            job_id = None
            lines: list[str] = []
            while job_id is None:
                self.raise_if_aborted()
                if mono() > deadline:
                    raise Aborted(
                        "reindex_job_id_not_found",
                        {"log_tail": self.follower.tail(), "evidence": evidence},
                    )
                lines.extend(self.follower.read_new())
                hooks = [
                    line
                    for line in lines
                    if "bulk fast-load: rebuilding deferred search indexes" in line
                    and submission_id in line
                ]
                started = [
                    line for line in lines if "deferred-index rebuild started" in line
                ]
                ids = {
                    match.group(1)
                    for line in started
                    for match in [re.search(r'job_id="?([0-9a-fA-F-]{36})"?', line)]
                    if match
                }
                if ids:
                    evidence["hook_lines"] = hooks[-3:]
                    evidence["start_lines"] = started[-3:]
                    if len(ids) > 1 or len(hooks) != 1:
                        raise Aborted(
                            "reindex_correlation_ambiguous",
                            {"job_ids": sorted(ids), "hook_lines": hooks[-3:]},
                        )
                    job_id = ids.pop()
                time.sleep(min(1.0, self.args.endpoint_poll_interval))
            evidence["job_id"] = job_id
            evidence["job_id_source"] = "hfs log line 'deferred-index rebuild started' after kickoff"
            status_deadline = mono() + self.args.reindex_timeout
            status_polls: list[dict[str, Any]] = []
            while True:
                self.raise_if_aborted()
                result = http_json("GET", f"{self.base_url}/$reindex-status/{job_id}", timeout=60)
                params = parameters_map(result["json"]) if result["status"] == 200 else {}
                status_polls.append(
                    {"status": result["status"], "reindex": params.get("status"), "at_s": round(mono() - kickoff_mono, 1)}
                )
                if params.get("status") in ("completed", "failed", "cancelled"):
                    break
                if mono() > status_deadline:
                    raise Aborted(
                        "reindex_timeout",
                        {"polls": status_polls[-10:], "evidence": evidence},
                    )
                time.sleep(max(0.5, self.args.endpoint_poll_interval))
            evidence["status_polls"] = status_polls[-10:]
            evidence["status"] = params.get("status")
            evidence["total"] = params.get("total")
            evidence["processed"] = params.get("processed")
            evidence["entries_created"] = params.get("entriesCreated")
            evidence["error_count"] = params.get("errorCount")
            if (
                params.get("status") != "completed"
                or params.get("errorCount") != 0
                or params.get("processed") != params.get("total")
                or params.get("total") != total
            ):
                raise Aborted("reindex_not_completed_cleanly", {"evidence": evidence})
        else:
            evidence["job_id_source"] = "not applicable: HFS_BULK_SUBMIT_DEFER_INDEXING=false"

        counts = self.pg.counts()
        evidence["db_counts"] = counts["counts"]
        unindexed = counts["counts"].get("unindexed_patients")
        indexed_total = self.search_count(f"family={FAMILY}")
        evidence["search_total_family"] = indexed_total
        evidence["unindexed_patients"] = unindexed
        evidence["reindex_s"] = round(mono() - kickoff_mono, 3)
        if unindexed != 0:
            raise Aborted("reindex_coverage_incomplete", {"evidence": evidence})
        if indexed_total != total:
            raise Aborted("search_index_count_mismatch", {"evidence": evidence})
        evidence["verified"] = True
        self.jsonl_pg.write({"phase": "reindex_verified", "job": job, **evidence})
        return evidence

    # -- idle / validation ------------------------------------------------

    def idle(self, job: int) -> float:
        self.phase("idle_start", job, {"idle_seconds": self.args.idle_seconds})
        end = mono() + self.args.idle_seconds
        while mono() < end:
            self.raise_if_aborted()
            time.sleep(min(0.5, max(0.05, end - mono())))
        return self.phase("idle_end", job)["mono_s"]

    def validate_job(self, job: int, submission_id: str, output: list[dict[str, Any]]) -> dict[str, Any]:
        """Exactness checks after the idle window; labelled as validation.

        The receipt artifact is written per resource *type*, so the four Patient
        files come back as one combined artifact and the verdict is on the union
        of all output entries.
        """
        self.phase("validation_start", job, {"label": "post_idle_validation"})
        started = mono()
        total = self.expected_total()
        layout = self.file_offsets()
        sample_points = [
            (0, layout[0][0]),
            (FIXTURE_FILE_COUNT // 2, layout[FIXTURE_FILE_COUNT // 2][0]),
            (FIXTURE_FILE_COUNT - 1, layout[-1][-1]),
        ]
        sampled = [self.patient_id(file_index, offset) for file_index, offset in sample_points]
        expected = {
            self.patient_id(file_index, offset): self.patient_resource(file_index, offset)
            for file_index, offsets in enumerate(layout)
            for offset in offsets
        }
        receipts = self.receipt_check(job, output)
        detail = self.pg.detail(sampled, submission_id)
        counts = self.pg.counts()
        checks: list[dict[str, Any]] = []

        def add(name: str, kind: str, ok: bool, expected: Any, actual: Any, note: str = "") -> None:
            check = {
                "job": job,
                "name": name,
                "kind": kind,
                "ok": bool(ok),
                "expected": expected,
                "actual": actual,
                "note": note,
            }
            checks.append(check)
            self.checks.append(check)

        # Exact current content, id set, version and history - streamed from SQL.
        content_mismatches: list[dict[str, Any]] = []
        seen_ids: set[str] = set()
        version_distribution: Counter = Counter()
        content_rows = 0
        for row in self.pg.stream_rows(
            "SELECT id, version_id, is_deleted, data::text FROM resources "
            f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' ORDER BY id"
        ):
            if len(row) < 4:
                continue
            content_rows += 1
            resource_id, version_id, is_deleted, body = row[0], row[1], row[2], row[3]
            seen_ids.add(resource_id)
            version_distribution[version_id] += 1
            wanted = expected.get(resource_id)
            if wanted is None:
                content_mismatches.append({"id": resource_id, "reason": "unexpected id"})
                continue
            if is_deleted == "t":
                content_mismatches.append({"id": resource_id, "reason": "is_deleted"})
                continue
            try:
                stored = json.loads(body)
            except json.JSONDecodeError:
                content_mismatches.append({"id": resource_id, "reason": "unparsable jsonb"})
                continue
            # `data` holds the submitted body only: server meta is merged on read.
            diffs = [key for key, value in wanted.items() if stored.get(key) != value]
            if set(stored) != set(wanted):
                diffs.append("keys:" + ",".join(sorted(set(stored) ^ set(wanted))[:5]))
            if diffs:
                content_mismatches.append({"id": resource_id, "reason": "content", "diff": diffs[:5]})

        add("pg_content_all", "hard", not content_mismatches and content_rows == total,
            f"{total} stored bodies identical to the fixture",
            {"rows": content_rows, "mismatches": len(content_mismatches),
             "examples": content_mismatches[:5]})
        add("pg_id_set", "hard", seen_ids == set(expected), f"{total} ids", {
            "rows": len(seen_ids),
            "missing": len(set(expected) - seen_ids),
            "unexpected": len(seen_ids - set(expected)),
        })
        add("pg_version_distribution", "hard", dict(version_distribution) == {str(job): total},
            {str(job): total}, dict(version_distribution))

        history_counts: Counter = Counter()
        history_keys: Counter = Counter()
        for row in self.pg.stream_rows(
            "SELECT id, version_id FROM resource_history "
            f"WHERE tenant_id = '{TENANT}' AND resource_type = 'Patient' ORDER BY id, version_id"
        ):
            if len(row) >= 2:
                history_keys[(row[0], row[1])] += 1
                history_counts[row[0]] += 1
        expected_versions = {str(version) for version in range(1, job + 1)}
        bad_history: dict[str, Any] = {}
        for (resource_id, version_id), count in history_keys.items():
            if version_id not in expected_versions or count != 1:
                bad_history[resource_id] = f"{version_id}x{count}"
        for resource_id in expected:
            if history_counts.get(resource_id) != job:
                bad_history.setdefault(resource_id, f"versions={history_counts.get(resource_id)}")
        add("pg_history_exact", "hard", not bad_history and len(history_counts) == total,
            {str(version): total for version in range(1, job + 1)},
            {"ids": len(history_counts), "bad_ids": len(bad_history),
             "examples": dict(list(bad_history.items())[:5])})

        add("pg_patient_resources", "hard", counts["counts"].get("resources_patient") == total,
            total, counts["counts"].get("resources_patient"))
        add("pg_unindexed_patients", "hard", counts["counts"].get("unindexed_patients") == 0,
            0, counts["counts"].get("unindexed_patients"))

        # HTTP exactness (the bare count is storage-only; the family search is indexed)
        add("http_storage_total", "hard", self.search_count("") == total, total, self.search_count(""))
        family_total = self.search_count(f"family={FAMILY}")
        add("http_family_search", "hard", family_total == total, total, family_total)
        active_total = self.search_count("active=true")
        add("http_active_search", "hard", active_total == self.expected_active(),
            self.expected_active(), active_total)
        middle_file, middle_offset = sample_points[1]
        identifier_total = self.search_count(
            "identifier="
            + urllib.parse.quote(f"{MRN_SYSTEM}|MRN-{middle_file}-{middle_offset}", safe="")
        )
        add("http_identifier_search", "hard", identifier_total == 1, 1, identifier_total)

        # Supplemental: three sampled resources over HTTP, including meta version.
        for resource_id in sampled:
            response = http_json("GET", f"{self.base_url}/Patient/{resource_id}")
            stored = response["json"] if isinstance(response["json"], dict) else {}
            wanted = self._expected_for_id(resource_id)
            matches = bool(wanted) and all(stored.get(key) == value for key, value in wanted.items())
            version = str((stored.get("meta") or {}).get("versionId", ""))
            add(f"http_sample_{resource_id}", "hard",
                response["status"] == 200 and matches and version == str(job),
                {"content": "matches fixture", "version": job},
                {"status": response["status"], "version": version})

        entry_results = detail["data"].get("entry_results", [])
        receipt_ok = (
            len(entry_results) == 1
            and entry_results[0][0] == "success"
            and int(entry_results[0][2]) == total
            and entry_results[0][1] == ("true" if job == 1 else "false")
        )
        add("pg_entry_results", "hard", receipt_ok,
            {"outcome": "success", "created": job == 1, "count": total}, entry_results)
        changes = detail["data"].get("changes", [])
        if job == 1:
            changes_ok = len(changes) == 1 and changes[0][0] == "create" and int(changes[0][3]) == total
            expected_changes = {"change_type": "create", "count": total}
        else:
            changes_ok = (
                len(changes) == 1
                and changes[0][0] == "update"
                and changes[0][1] == str(job - 1)
                and changes[0][2] == str(job)
                and int(changes[0][3]) == total
            )
            expected_changes = {
                "change_type": "update",
                "previous_version": job - 1,
                "new_version": job,
                "count": total,
            }
        add("pg_submission_changes", "hard", changes_ok, expected_changes, changes)
        manifests = detail["data"].get("manifests", [])
        add("pg_manifests_terminal", "hard",
            bool(manifests) and all(row[1] == "completed" for row in manifests),
            "every manifest completed", manifests)

        result = {
            "job": job,
            "submission_id": submission_id,
            "label": "post_idle_validation",
            "duration_s": round(mono() - started, 3),
            "checks": checks,
            "receipts": receipts,
            "pg_detail": detail,
            "pg_counts": counts,
            "sampled_ids": sampled,
            "version_distribution": dict(version_distribution),
            "history_ids": len(history_counts),
        }
        self.write_json(f"jobs/job{job:02d}/validation.json", result)
        self.phase(
            "validation_end",
            job,
            {
                "checks": len(checks),
                "failed_hard": len([c for c in checks if c["kind"] == "hard" and not c["ok"]]),
                "failed_soft": len([c for c in checks if c["kind"] == "soft" and not c["ok"]]),
                "duration_s": result["duration_s"],
            },
        )
        return result

    def _expected_for_id(self, resource_id: str) -> dict[str, Any]:
        match = re.fullmatch(r"p995-(\d+)-(\d+)", resource_id)
        if not match:
            return {}
        file_index, offset = int(match.group(1)), int(match.group(2))
        return self.patient_resource(file_index, offset)

    # -- job / run orchestration ------------------------------------------

    def run_job(self, job: int) -> None:
        submission_id = f"{SUBMITTER_VALUE}-{self.run_id}-j{job}"
        total = self.expected_total()
        attempt: dict[str, Any] = {
            "ordinal": job,
            "submission_id": submission_id,
            "mode": self.args.mode,
            "defer_indexing": bool(self.args.defer_indexing),
            "status": "running",
            "timings": {"label": "incomplete", "comparable": False},
            "throughput_resources_per_s": None,
        }
        self.current_attempt = attempt
        self.attempts.append(attempt)
        if self.args.mode == "restart" or self.hfs is None:
            self.start_hfs(job)
        manifest_url = self.ensure_fixtures(job)
        baseline = self.phase(
            "prekickoff_baseline",
            job,
            {
                "submission_id": submission_id,
                "validation_effects": job > 1,
                "manifest": str(Path(manifest_url).name),
            },
        )
        log_offset = self.follower.seek_end()
        self.phase("kickoff", job, {"submission_id": submission_id})
        kickoff_mono = mono()
        kickoff = self.kickoff(submission_id, manifest_url)
        self.write_json(
            f"jobs/job{job:02d}/kickoff.json",
            {"submission_id": submission_id, "status": kickoff["status"], "body": kickoff["json"]},
        )
        if kickoff["status"] != 200:
            raise Aborted(
                "kickoff_failed",
                {"status": kickoff["status"], "body": kickoff["json"], "error": kickoff["error"]},
            )
        self.phase("kickoff_accepted", job, {"submission_id": submission_id, "status": kickoff["status"]})
        terminal = self.poll_terminal(job, submission_id, kickoff_mono)
        attempt["timings"]["kickoff_to_terminal_s"] = terminal["terminal_s"]
        declared = sum(int(entry.get("count") or 0) for entry in terminal["output"])
        self.phase(
            "submit_terminal",
            job,
            {
                "submission_id": submission_id,
                "terminal_s": terminal["terminal_s"],
                "pages": terminal["pages"],
                "declared_count": declared,
                "outcome_entries": len(terminal["outcome"]),
                "deleted_entries": len(terminal["deleted"]),
            },
        )
        self.write_json(
            f"jobs/job{job:02d}/submit-terminal.json",
            {
                "submission_id": submission_id,
                "terminal_s": terminal["terminal_s"],
                "polls": terminal["polls"],
                "output": terminal["output"],
                "outcome": terminal["outcome"],
                "deleted": terminal["deleted"],
            },
        )
        if declared != self.expected_total():
            raise Aborted("manifest_count_mismatch", {"declared": declared, "expected": self.expected_total()})
        if terminal["outcome"]:
            raise Aborted("manifest_reports_outcome_entries", {"outcome": terminal["outcome"][:3]})
        if terminal["deleted"]:
            raise Aborted("manifest_reports_deleted_entries", {"deleted": terminal["deleted"][:3]})
        attempt["manifest"] = {
            "pages": terminal["pages"],
            "declared_count": declared,
            "output_entries": len(terminal["output"]),
            "outcome_entries": len(terminal["outcome"]),
            "deleted_entries": len(terminal["deleted"]),
            "transaction_time": (terminal["manifest"] or {}).get("transactionTime"),
            "requires_access_token": (terminal["manifest"] or {}).get("requiresAccessToken"),
        }
        reindex = self.verify_reindex(job, submission_id, log_offset, kickoff_mono)
        self.phase("reindex_terminal", job, {
            "job_id": reindex.get("job_id"), "verified": reindex["verified"],
            "deferred": bool(self.args.defer_indexing),
        })
        attempt["reindex"] = reindex
        attempt["timings"]["kickoff_to_reindex_s"] = reindex["reindex_s"]
        self.write_json(f"jobs/job{job:02d}/reindex.json", reindex)
        idle_end = self.idle(job)
        validation = self.validate_job(job, submission_id, terminal["output"])
        attempt["validation"] = {
            "duration_s": validation["duration_s"],
            "failed_hard": [c["name"] for c in validation["checks"] if c["kind"] == "hard" and not c["ok"]],
            "failed_soft": [c["name"] for c in validation["checks"] if c["kind"] == "soft" and not c["ok"]],
            "version_distribution": validation["version_distribution"],
            "history_ids": validation["history_ids"],
        }
        attempt["receipts"] = {
            "ok": validation["receipts"]["ok"],
            "output_entries": validation["receipts"]["output_entries"],
            "references": validation["receipts"]["references"],
            "checked_after": "idle_end (validation phase)",
        }
        attempt["rss"] = self.rss_stats(job, kickoff_mono, idle_end, baseline["rss_mib"])
        if self.args.mode == "restart":
            self.stop_hfs(job, "job_complete")
        self.write_summary()
        # A failed validation stops the run: the next job would build on an
        # unverified database state.
        if attempt["validation"]["failed_hard"]:
            attempt["status"] = "failed"
            raise Aborted(
                "hard_validation_failed",
                {"job": job, "checks": attempt["validation"]["failed_hard"]},
            )
        attempt["timings"].update({"label": "complete", "comparable": True, "verified": True})
        attempt["status"] = "verified"
        attempt["throughput_resources_per_s"] = round(total / max(terminal["terminal_s"], 0.001), 3)

    def rss_stats(self, job: int, start: float, end: float, baseline: Optional[float]) -> dict[str, Any]:
        """Measured window is kickoff..idle_end; startup and validation are separate."""
        rows = [row for row in self.rss_rows if row.get("job") == job]

        def block(values: list[float], base: Optional[float] = None) -> dict[str, Any]:
            if not values:
                return {"samples": 0}
            return {
                "samples": len(values),
                "first_mib": values[0],
                "peak_mib": max(values),
                "last_mib": values[-1],
                "delta_peak_mib": None if base is None else round(max(values) - base, 1),
            }

        measured = [row["rss_mib"] for row in rows if start <= row["mono_s"] <= end]
        return {
            "measured_window": {
                **block(measured, baseline),
                "baseline_mib": baseline,
                "window": "kickoff..idle_end",
            },
            "startup_window": block([row["rss_mib"] for row in rows if row["mono_s"] < start]),
            "validation_window": block([row["rss_mib"] for row in rows if row["mono_s"] > end]),
        }

    def mark_attempt_incomplete(self, reason: str) -> None:
        attempt = self.current_attempt
        if not attempt or attempt.get("status") not in ("running",):
            return
        attempt["status"] = "unverified" if "reindex" in reason else "failed"
        attempt["failure_reason"] = reason
        attempt["timings"].update({"label": "incomplete", "comparable": False, "verified": False})
        attempt["throughput_resources_per_s"] = None
        attempt["rss"] = self.rss_stats(attempt["ordinal"], 0.0, float("inf"), None)

    def hard_failures(self) -> list[dict[str, Any]]:
        return [check for check in self.checks if check["kind"] == "hard" and not check["ok"]]

    def write_summary(self) -> None:
        write_json_atomic(
            self.out / "summary.json",
            {
                "schema_version": SCHEMA_VERSION,
                "updated": iso_now(),
                "attempts": self.attempts,
                "phases": len(self.phases),
                "stops": self.stops,
                "hard_failures": [c["name"] for c in self.hard_failures()],
            },
        )

    def cleanup(self) -> None:
        if self.sampler is not None:
            self.sampler.stop()
            self.sampler.join(timeout=60)
            self.sampler = None
        if self.hfs is not None:
            stopped = self.hfs.stop()
            if self.log:
                self.log.line("hfs_stopped", reason="cleanup", **stopped)
            self.hfs = None
        if self.provider is not None:
            self.provider.stop()
            self.provider = None
        for writer in (
            self.csv_rss, self.csv_host, self.csv_phase, self.csv_docker,
            self.jsonl_phase, self.jsonl_pg, self.jsonl_stops,
        ):
            if writer is not None:
                writer.close()
        self.csv_rss = self.csv_host = self.csv_phase = self.csv_docker = None
        self.jsonl_phase = self.jsonl_pg = self.jsonl_stops = None

    def finish(self, status: str, exit_code: int, reason: Optional[str]) -> None:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "status": status,
            "exit_code": exit_code,
            "reason": reason,
            "controller": {"path": str(Path(__file__).resolve()), "python": sys.version},
            "config": {
                "binary": str(self.binary),
                "output_dir": str(self.out),
                "repo_root": str(self.repo_root),
                "resources": self.args.resources,
                "jobs": self.args.jobs,
                "mode": self.args.mode,
                "defer_indexing": bool(self.args.defer_indexing),
                "idle_seconds": self.args.idle_seconds,
                "file_concurrency": self.args.file_concurrency,
                "pg_container": self.args.pg_container,
                "database_url": redact_db_url(self.args.database_url),
                "hfs_env": {
                    key: (redact_db_url(value) if key == "HFS_DATABASE_URL" else value)
                    for key, value in self.effective_env().items()
                },
                "watchdog": {
                    "operational_max_rss_mib": self.args.operational_max_rss_mib,
                    "pressure_samples": self.args.pressure_samples,
                    "swap_growth_mib": self.args.swap_growth_mib,
                    "swap_growth_samples": self.args.swap_growth_samples,
                    "deadline_s": self.args.deadline,
                },
                "timers": {
                    "startup_timeout_s": self.args.startup_timeout,
                    "terminal_timeout_s": self.args.terminal_timeout,
                    "reindex_timeout_s": self.args.reindex_timeout,
                    "endpoint_poll_interval_s": self.args.endpoint_poll_interval,
                    "sample_interval_s": self.args.sample_interval,
                    "host_interval_s": self.args.host_interval,
                    "docker_stats_interval_s": self.args.docker_stats_interval,
                },
            },
            "preflight": self.preflight_info,
            "fixture": {
                key: value
                for key, value in self.fixture_info.items()
                if key != "files"
            }
            | {"files": [
                {key: value for key, value in entry.items() if key != "ids"}
                for entry in self.fixture_info.get("files", [])
            ]},
            "phases": self.phases,
            "attempts": self.attempts,
            "checks": self.checks,
            "stops": self.stops,
        }
        write_json_atomic(self.out / "run.json", payload)
        self.write_summary()
        if self.log:
            self.log.line("run_end", status=status, exit_code=exit_code, reason=reason)
            self.log.close()

    # -- top level --------------------------------------------------------

    def install_signal_handlers(self) -> None:
        def handler(signum: int, _frame: Any) -> None:
            self.signal_number = signum
            with self.lock:
                if self.abort is None:
                    self.abort = {"reason": f"signal:{signum}", "detail": {}}
            if self.hfs is not None:
                self.hfs.stop(grace_seconds=5.0)
            if self.log:
                self.log.line("signal_received", signal=signum)

        for signum in (signal.SIGINT, signal.SIGTERM):
            try:
                signal.signal(signum, handler)
            except (ValueError, OSError):
                pass

    def run(self) -> int:
        try:
            self.prepare_outputs()
        except ConfigError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            return EXIT_CONFIG
        self.install_signal_handlers()
        status, exit_code, reason = "ok", EXIT_OK, None
        try:
            info = self.preflight()
            if info["blocked"]:
                raise Aborted("preflight_blocked", {"reasons": info["block_reasons"]})
            fixture_plan = {
                "files": [
                    {"name": f"patients-{index}.ndjson", "count": len(offsets)}
                    for index, offsets in enumerate(self.file_offsets())
                ],
                "total": self.expected_total(),
            }
            self.log.line("fixture_plan", plan=json.dumps(fixture_plan))
            self.provider = FixtureProvider(
                self.fixtures_dir, self.args.provider_port, self.log, self.out / "provider.log"
            )
            self.provider_base = self.provider.start()
            self.sampler = Sampler(self)
            self.sampler.start()
            for job in range(1, self.args.jobs + 1):
                self.run_job(job)
        except Aborted as exc:
            reason = exc.reason
            self.mark_attempt_incomplete(exc.reason)
            if exc.reason == "preflight_blocked":
                status, exit_code = "preflight_blocked", EXIT_PREFLIGHT
            elif exc.reason == "hard_validation_failed":
                status, exit_code = "aborted", EXIT_VALIDATION
            elif self.signal_number is not None:
                status, exit_code = "interrupted", 128 + self.signal_number
            else:
                status, exit_code = "aborted", EXIT_ABORTED
            if self.log:
                self.log.line("run_aborted", reason=reason, detail=json.dumps(exc.detail, default=str))
        except ConfigError as exc:
            status, exit_code, reason = "config_error", EXIT_CONFIG, str(exc)
            print(f"ERROR: {exc}", file=sys.stderr)
        except KeyboardInterrupt:
            status, exit_code, reason = "interrupted", 130, "keyboard interrupt"
        except Exception as exc:
            status, exit_code, reason = "failed", EXIT_CONFIG, f"{type(exc).__name__}: {exc}"
            if self.log:
                self.log.line("run_failed", error=reason)
        finally:
            self.phase("series_end", self.current_job, {"status": status, "reason": reason})
            self.cleanup()
        if status == "ok" and self.args.strict_validation and self.hard_failures():
            failures = [check["name"] for check in self.hard_failures()]
            status, exit_code = "validation_failed", EXIT_VALIDATION
            reason = f"hard validation checks failed: {', '.join(failures)}"
        self.finish(status, exit_code, reason)
        verified = [attempt for attempt in self.attempts if attempt.get("status") == "verified"]
        print(
            f"\n{status}: {len(verified)}/{len(self.attempts)} attempt(s) verified; "
            f"artifacts in {self.out}"
        )
        return exit_code


def _usage_mib(value: str) -> Optional[float]:
    match = re.match(r"\s*([0-9.]+)\s*([kKmMgG]?i?B)", value or "")
    if not match:
        return None
    number = float(match.group(1))
    unit = match.group(2).lower()
    if unit.startswith("b"):
        return round(number / 1024 / 1024, 1)
    if unit.startswith("k"):
        return round(number / 1024, 1)
    if unit.startswith("g"):
        return round(number * 1024, 1)
    return round(number, 1)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def parse_bool(value: str) -> bool:
    normalised = str(value).strip().lower()
    if normalised in ("1", "true", "yes", "on"):
        return True
    if normalised in ("0", "false", "no", "off"):
        return False
    raise argparse.ArgumentTypeError(f"expected true or false, got {value!r}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Drive $bulk-submit imports of a deterministic Patient corpus against a "
            "dedicated PostgreSQL container and record external RSS/pressure/DB evidence."
        ),
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("--binary", required=True, help="release hfs binary (R4+postgres)")
    parser.add_argument("--output-dir", required=True, help="new, nonexistent output directory")
    parser.add_argument("--resources", type=int, default=1000, help="Patients per submission")
    parser.add_argument("--jobs", type=int, default=1, help="submissions in this run")
    parser.add_argument("--mode", choices=("consecutive", "restart"), default="consecutive")
    parser.add_argument(
        "--defer-indexing", type=parse_bool, default=True,
        help="HFS_BULK_SUBMIT_DEFER_INDEXING for the measured server",
    )
    parser.add_argument("--idle-seconds", type=float, default=60.0)
    parser.add_argument("--file-concurrency", type=int, default=1)
    parser.add_argument("--pg-container", required=True, help="dedicated PostgreSQL container")
    parser.add_argument("--database-url", required=True, help="host-reachable PostgreSQL URL")
    parser.add_argument("--pg-user", help="psql -U (default: from the URL)")
    parser.add_argument("--pg-db", help="psql -d (default: from the URL)")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--hfs-port", type=int, default=0, help="0 picks a free loopback port")
    parser.add_argument("--provider-port", type=int, default=0, help="0 starts at 19200")
    parser.add_argument("--hfs-log-level", default="info")
    parser.add_argument("--sample-interval", type=float, default=0.5, help="HFS RSS cadence, seconds")
    parser.add_argument("--host-interval", type=float, default=5.0, help="host vitals cadence, seconds")
    parser.add_argument("--docker-stats-interval", type=float, default=60.0, help="0 disables")
    parser.add_argument("--endpoint-poll-interval", type=float, default=1.0)
    parser.add_argument("--request-timeout", type=float, default=60.0)
    parser.add_argument("--startup-timeout", type=float, default=180.0)
    parser.add_argument("--terminal-timeout", type=float, default=3600.0)
    parser.add_argument("--reindex-timeout", type=float, default=900.0)
    parser.add_argument("--deadline", type=float, default=10800.0, help="whole-run watchdog, seconds")
    parser.add_argument("--preflight-samples", type=int, default=5)
    parser.add_argument("--preflight-interval", type=float, default=5.0, help="settle between preflight samples")
    parser.add_argument("--operational-max-rss-mib", type=float, default=1536.0)
    parser.add_argument("--pressure-samples", type=int, default=6, help="consecutive non-normal pressure samples")
    parser.add_argument("--swap-growth-mib", type=float, default=256.0)
    parser.add_argument("--swap-growth-samples", type=int, default=3)
    parser.add_argument("--allow-nonempty-db", action="store_true", help="skip the fresh-database gate")
    parser.add_argument("--strict-validation", dest="strict_validation", action="store_true", default=True)
    parser.add_argument("--no-strict-validation", dest="strict_validation", action="store_false")
    parser.add_argument("--repo-root", help="checkout root containing data/ (default: derived)")
    parser.add_argument("--hfs-env", action="append", default=[], metavar="KEY=VALUE")
    parser.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    parser.add_argument("--quiet", action="store_true", help="do not echo controller events to stdout")
    return parser


def dry_run_plan(args: argparse.Namespace) -> dict[str, Any]:
    repo_root = (
        Path(args.repo_root).expanduser().resolve()
        if args.repo_root
        else Path(__file__).resolve().parents[4]
    )
    per_file = args.resources // FIXTURE_FILE_COUNT
    layout = [per_file] * FIXTURE_FILE_COUNT
    for _ in range(per_file * FIXTURE_FILE_COUNT, args.resources):
        layout[-1] += 1
    return {
        "schema_version": SCHEMA_VERSION,
        "dry_run": True,
        "binary": str(Path(args.binary).expanduser().resolve()),
        "output_dir": str(Path(args.output_dir).expanduser().resolve()),
        "repo_root": str(repo_root),
        "search_parameters_file": str(repo_root / "data" / "search-parameters-r4.json"),
        "fixture": {
            "files": [
                {"name": f"patients-{index}.ndjson", "count": count, "ids": f"p995-{index}-0..{count - 1}"}
                for index, count in enumerate(layout)
            ],
            "total": args.resources,
            "family": FAMILY,
            "mrn_system": MRN_SYSTEM,
        },
        "jobs": args.jobs,
        "mode": args.mode,
        "defer_indexing": bool(args.defer_indexing),
        "database": redact_db_url(args.database_url),
        "pg_container": args.pg_container,
        "watchdog": {
            "operational_max_rss_mib": args.operational_max_rss_mib,
            "pressure_samples": args.pressure_samples,
            "swap_growth_mib": args.swap_growth_mib,
            "swap_growth_samples": args.swap_growth_samples,
            "deadline_s": args.deadline,
        },
        "verification": [
            "submit terminal 200 for every manifest page, outcome/deleted empty, declared counts match",
            "receipt artifact download: exact aggregate Patient id set (one receipt per resource type)",
            "deferred reindex job id from the hfs log line, $reindex-status completed with errorCount=0 and processed=total",
            "SQL coverage: zero unindexed Patients, parameterised family search equals the expected total",
            "post-idle: version/history spread, content, entry_results receipts, submission changes",
        ],
    }


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.resources < FIXTURE_FILE_COUNT:
        parser.error(f"--resources must be >= {FIXTURE_FILE_COUNT}")
    if args.jobs < 1:
        parser.error("--jobs must be >= 1")
    if args.file_concurrency < 1:
        parser.error("--file-concurrency must be >= 1")
    if args.sample_interval <= 0 or args.host_interval <= 0:
        parser.error("--sample-interval and --host-interval must be > 0")
    if args.pressure_samples < 1 or args.swap_growth_samples < 1:
        parser.error("--pressure-samples and --swap-growth-samples must be >= 1")
    if args.dry_run:
        print(json.dumps(dry_run_plan(args), indent=2))
        return EXIT_OK
    try:
        args.hfs_port = args.hfs_port or pick_free_port(18810, args.host)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return EXIT_CONFIG
    return Controller(args).run()


if __name__ == "__main__":
    sys.exit(main())
