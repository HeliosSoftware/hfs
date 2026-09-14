#!/usr/bin/env python3
"""Measure deferred-reindex coordination for issue #1087.

The controller talks to a dedicated, already running HFS instance and serves
only the fixtures it creates. It never starts, stops, or cleans up HFS or its
database. Evidence is written to a new caller-selected directory.

The HFS log must include INFO events from ``helios_persistence``. The script
correlates physical reindex jobs from those events and polls each job through
``$reindex-status``. It does not use the process-global ``hfs_perf`` counters,
which cannot attribute work when jobs overlap.
"""

from __future__ import annotations

import argparse
import functools
import http.server
import json
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SUBMITTER_SYSTEM = "https://helios.software/bench/reindex-coordination"
START_MARKERS = (
    "deferred reindex generation started",
    "deferred-index rebuild started",
)
JOB_ID_RE = re.compile(r'(?:job_id|"job_id")\s*[=:]\s*"?([0-9a-fA-F-]{36})')
TERMINAL = {"completed", "failed", "cancelled"}


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def http_json(
    method: str,
    url: str,
    tenant: str,
    payload: dict[str, Any] | None = None,
    timeout: float = 60.0,
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
            parsed = json.loads(raw) if raw else None
            return response.status, {k.lower(): v for k, v in response.headers.items()}, parsed
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            parsed = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = raw.decode("utf-8", "replace")
        return error.code, {k.lower(): v for k, v in error.headers.items()}, parsed


def parameters_map(resource: Any) -> dict[str, Any]:
    values: dict[str, Any] = {}
    if not isinstance(resource, dict):
        return values
    for parameter in resource.get("parameter", []):
        name = parameter.get("name")
        if not name:
            continue
        value_key = next((key for key in parameter if key.startswith("value")), None)
        if value_key:
            values[name] = parameter[value_key]
    return values


class FixtureServer:
    def __init__(self, root: Path, host: str, port: int) -> None:
        handler = functools.partial(
            http.server.SimpleHTTPRequestHandler, directory=str(root)
        )
        self.server = http.server.ThreadingHTTPServer((host, port), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        host, port = self.server.server_address[:2]
        return f"http://{host}:{port}"

    def __enter__(self) -> "FixtureServer":
        self.thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


class Controller:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.base_url = args.base_url.rstrip("/")
        self.output = args.output_dir.resolve()
        self.fixtures = self.output / "fixtures"
        self.run_id = uuid.uuid4().hex[:12]
        self.tenant_prefix = args.tenant or f"hfs-1087-{self.run_id}"
        self.tenant = self.tenant_prefix
        self.provider_url = ""

    def prepare(self) -> None:
        if self.output.exists():
            raise RuntimeError(f"output directory already exists: {self.output}")
        if not self.args.hfs_log.is_file():
            raise RuntimeError(f"HFS log does not exist: {self.args.hfs_log}")
        self.fixtures.mkdir(parents=True)
        status, _, metadata = http_json(
            "GET", f"{self.base_url}/metadata", self.tenant, timeout=10
        )
        if status != 200:
            raise RuntimeError(f"HFS metadata returned HTTP {status}: {metadata}")
        self.write_json(
            "run.json",
            {
                "started_at": now_iso(),
                "run_id": self.run_id,
                "tenant_prefix": self.tenant_prefix,
                "base_url": self.base_url,
                "hfs_log": str(self.args.hfs_log.resolve()),
                "resources_per_manifest": self.args.resources_per_manifest,
                "preexisting_per_scenario": self.args.preexisting,
                "burst_manifests": self.args.burst_manifests,
                "poll_interval_seconds": self.args.poll_interval,
                "metadata_software": metadata.get("software")
                if isinstance(metadata, dict)
                else None,
            },
        )

    def write_json(self, relative: str, value: Any) -> None:
        path = self.output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")

    def patient(self, identifier: str, family: str) -> dict[str, Any]:
        return {
            "resourceType": "Patient",
            "id": identifier,
            "active": True,
            "name": [{"family": family, "given": ["Coordination"]}],
        }

    def seed_preexisting(self, identifiers: list[str], old_family: str) -> None:
        for identifier in identifiers[: self.args.preexisting]:
            status, _, result = http_json(
                "PUT",
                f"{self.base_url}/Patient/{urllib.parse.quote(identifier)}",
                self.tenant,
                self.patient(identifier, old_family),
            )
            if status not in (200, 201):
                raise RuntimeError(
                    f"preexisting Patient/{identifier} returned HTTP {status}: {result}"
                )

    def make_manifest(
        self,
        scenario: str,
        manifest_index: int,
        identifiers: list[str],
        family: str,
        split_files: int = 1,
    ) -> str:
        output = []
        for part in range(split_files):
            selected = identifiers[part::split_files]
            filename = f"{scenario}-{manifest_index}-part-{part}.ndjson"
            path = self.fixtures / filename
            with path.open("w", encoding="utf-8") as handle:
                for identifier in selected:
                    handle.write(json.dumps(self.patient(identifier, family)) + "\n")
            output.append(
                {
                    "type": "Patient",
                    "url": f"{self.provider_url}/{filename}",
                    "count": len(selected),
                }
            )
        manifest_name = f"{scenario}-{manifest_index}.json"
        manifest = {
            "transactionTime": now_iso(),
            "request": f"{self.provider_url}/{manifest_name}",
            "requiresAccessToken": False,
            "output": output,
            "error": [],
            "deleted": [],
        }
        (self.fixtures / manifest_name).write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        return f"{self.provider_url}/{manifest_name}"

    def kickoff(self, scenario: str, index: int, manifest_url: str) -> dict[str, Any]:
        submission_id = f"{self.run_id}-{scenario}-{index}"
        payload = {
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "submitter",
                    "valueIdentifier": {
                        "system": SUBMITTER_SYSTEM,
                        "value": self.run_id,
                    },
                },
                {"name": "submissionId", "valueString": submission_id},
                {"name": "manifestUrl", "valueUrl": manifest_url},
                {"name": "fhirBaseUrl", "valueUrl": f"{self.provider_url}/fhir"},
                {
                    "name": "submissionStatus",
                    "valueCoding": {
                        "system": "http://hl7.org/fhir/event-status",
                        "code": "completed",
                    },
                },
            ],
        }
        started = time.monotonic()
        status, _, response = http_json(
            "POST", f"{self.base_url}/$bulk-submit", self.tenant, payload
        )
        if status != 200:
            raise RuntimeError(
                f"$bulk-submit {submission_id} returned HTTP {status}: {response}"
            )
        return {
            "submission_id": submission_id,
            "kickoff_monotonic": started,
            "kickoff_at": now_iso(),
        }

    def poll_submission(self, submission: dict[str, Any]) -> dict[str, Any]:
        payload = {
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "submitter",
                    "valueIdentifier": {
                        "system": SUBMITTER_SYSTEM,
                        "value": self.run_id,
                    },
                },
                {"name": "submissionId", "valueString": submission["submission_id"]},
            ],
        }
        status, headers, response = http_json(
            "POST", f"{self.base_url}/$bulk-submit-status", self.tenant, payload
        )
        poll_url = headers.get("content-location")
        if status != 202 or not poll_url:
            raise RuntimeError(
                f"status kickoff returned HTTP {status} without Content-Location: {response}"
            )
        poll_url = urllib.parse.urljoin(self.base_url + "/", poll_url)
        deadline = time.monotonic() + self.args.timeout
        polls = []
        while time.monotonic() < deadline:
            status, headers, response = http_json("GET", poll_url, self.tenant)
            polls.append(
                {
                    "elapsed_seconds": round(
                        time.monotonic() - submission["kickoff_monotonic"], 3
                    ),
                    "http_status": status,
                    "progress": headers.get("x-progress"),
                }
            )
            if status == 200:
                return {
                    **submission,
                    "terminal_at": now_iso(),
                    "terminal_seconds": polls[-1]["elapsed_seconds"],
                    "polls": polls,
                    "manifest": response,
                }
            if status not in (202, 429):
                raise RuntimeError(f"status poll returned HTTP {status}: {response}")
            time.sleep(self.args.poll_interval)
        raise RuntimeError(f"submission {submission['submission_id']} did not finish")

    def search_count(self, family: str) -> int | None:
        query = urllib.parse.urlencode(
            {"family:exact": family, "_summary": "count", "_count": "1"}
        )
        status, _, bundle = http_json(
            "GET", f"{self.base_url}/Patient?{query}", self.tenant
        )
        if status != 200 or not isinstance(bundle, dict):
            return None
        total = bundle.get("total")
        return int(total) if isinstance(total, (int, float)) else None

    def log_segment(self, offset: int) -> str:
        return self.args.hfs_log.read_bytes()[offset:].decode("utf-8", "replace")

    def job_ids(self, log_text: str) -> list[str]:
        found = []
        for line in log_text.splitlines():
            if not any(marker in line for marker in START_MARKERS):
                continue
            match = JOB_ID_RE.search(line)
            if match and match.group(1) not in found:
                found.append(match.group(1))
        return found

    def reindex_status(self, job_id: str) -> dict[str, Any]:
        status, _, resource = http_json(
            "GET", f"{self.base_url}/$reindex-status/{job_id}", self.tenant
        )
        values = parameters_map(resource)
        return {"job_id": job_id, "http_status": status, **values}

    def wait_for_reindex(
        self,
        offset: int,
        family: str,
        expected: int,
        first_kickoff: float,
        submissions_done: threading.Event | None = None,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + self.args.timeout
        known: list[str] = []
        last_new_job = time.monotonic()
        readiness = None
        samples = []
        max_active = 0
        submissions_done_seen = submissions_done is None
        while time.monotonic() < deadline:
            if (
                not submissions_done_seen
                and submissions_done is not None
                and submissions_done.is_set()
            ):
                submissions_done_seen = True
                # A hook can start just after its submission becomes terminal.
                # Require a fresh quiet window from that terminal barrier.
                last_new_job = time.monotonic()
            log_text = self.log_segment(offset)
            discovered = self.job_ids(log_text)
            if discovered != known:
                known = discovered
                last_new_job = time.monotonic()
            statuses = [self.reindex_status(job_id) for job_id in known]
            active = sum(job.get("status") not in TERMINAL for job in statuses)
            max_active = max(max_active, active)
            count = self.search_count(family)
            if count == expected and readiness is None:
                readiness = round(time.monotonic() - first_kickoff, 3)
            samples.append(
                {
                    "at_seconds": round(time.monotonic() - first_kickoff, 3),
                    "active_jobs": active,
                    "job_count": len(known),
                    "search_count": count,
                }
            )
            all_terminal = bool(statuses) and all(
                job.get("status") in TERMINAL for job in statuses
            )
            quiet = time.monotonic() - last_new_job >= self.args.quiet_seconds
            if readiness is not None and all_terminal and quiet and submissions_done_seen:
                lines = [
                    line
                    for line in log_text.splitlines()
                    if any(marker in line for marker in START_MARKERS)
                    or "deferred reindex generation" in line
                ]
                return {
                    "physical_jobs": len(known),
                    "max_observed_active_jobs": max_active,
                    "kickoff_to_search_ready_seconds": readiness,
                    "jobs": statuses,
                    "processed_resources": sum(
                        int(job.get("processed", 0)) for job in statuses
                    ),
                    "reported_resource_totals": sum(
                        int(job.get("total", 0)) for job in statuses
                    ),
                    "entries_created": sum(
                        int(job.get("entriesCreated", 0)) for job in statuses
                    ),
                    "samples": samples,
                    "log_lines": lines,
                }
            time.sleep(self.args.poll_interval)
        raise RuntimeError(
            f"reindex did not become quiet and searchable; jobs={known}, "
            f"count={self.search_count(family)}, expected={expected}"
        )

    def run_scenario(self, name: str) -> dict[str, Any]:
        # A full-type reindex is tenant-scoped. Give each case an equivalent
        # empty tenant so earlier cases do not inflate later scan totals.
        self.tenant = f"{self.tenant_prefix}-{name}"
        manifest_count = {"combined": 1, "consecutive": 2, "overlapping": 2}.get(
            name, self.args.burst_manifests
        )
        family = f"Coordination-{self.run_id}-{name}"
        old_family = f"Obsolete-{self.run_id}-{name}"
        groups: list[list[str]] = []
        for manifest_index in range(manifest_count):
            if name == "consecutive":
                prefix = f"{self.run_id}-{name}"
            else:
                prefix = f"{self.run_id}-{name}-{manifest_index}"
            groups.append(
                [
                    f"{prefix}-{resource_index}"
                    for resource_index in range(self.args.resources_per_manifest)
                ]
            )
        unique_ids = list(dict.fromkeys(identifier for group in groups for identifier in group))
        self.seed_preexisting(unique_ids, old_family)
        manifest_families = [
            f"{family}-{index}" if name == "consecutive" else family
            for index in range(manifest_count)
        ]
        manifests = [
            self.make_manifest(
                name,
                index,
                identifiers,
                manifest_families[index],
                split_files=2 if name == "combined" else 1,
            )
            for index, identifiers in enumerate(groups)
        ]
        log_offset = self.args.hfs_log.stat().st_size
        started_at = now_iso()

        if name == "consecutive":
            submissions = []
            terminals = []
            first_kickoff = time.monotonic()
            reindex = None
            for index, manifest in enumerate(manifests):
                submission = self.kickoff(name, index, manifest)
                if not submissions:
                    first_kickoff = submission["kickoff_monotonic"]
                submissions.append(submission)
                terminals.append(self.poll_submission(submission))
                reindex = self.wait_for_reindex(
                    log_offset,
                    manifest_families[index],
                    len(unique_ids),
                    first_kickoff,
                )
            assert reindex is not None
        else:
            # Reindex can finish before terminal-manifest polling sees it. Start
            # the sampler before kickoff so short overlapping jobs are visible.
            first_kickoff = time.monotonic()
            submissions_done = threading.Event()
            with ThreadPoolExecutor(max_workers=manifest_count + 1) as pool:
                monitor = pool.submit(
                    self.wait_for_reindex,
                    log_offset,
                    family,
                    len(unique_ids),
                    first_kickoff,
                    submissions_done,
                )
                try:
                    futures = [
                        pool.submit(self.kickoff, name, index, manifest)
                        for index, manifest in enumerate(manifests)
                    ]
                    submissions = [future.result() for future in futures]
                    terminals = [self.poll_submission(item) for item in submissions]
                finally:
                    submissions_done.set()
                reindex = monitor.result()
        final_family = manifest_families[-1]
        expected_job_range = {
            "combined": (1, 1),
            "consecutive": (2, 2),
            "overlapping": (1, 2),
            "burst": (1, manifest_count),
        }[name]
        result = {
            "scenario": name,
            "tenant": self.tenant,
            "started_at": started_at,
            "family": final_family,
            "manifest_families": manifest_families,
            "manifest_count": manifest_count,
            "unique_resources": len(unique_ids),
            "preexisting_resources": min(self.args.preexisting, len(unique_ids)),
            "submissions": terminals,
            "reindex": reindex,
            "checks": {
                "search_ready": self.search_count(final_family) == len(unique_ids),
                "obsolete_search_absent": self.search_count(old_family) == 0
                and all(
                    self.search_count(previous_family) == 0
                    for previous_family in manifest_families[:-1]
                    if previous_family != final_family
                ),
                "all_jobs_completed_cleanly": all(
                    job.get("status") == "completed"
                    and int(job.get("errorCount", -1)) == 0
                    for job in reindex["jobs"]
                ),
                "all_submission_manifests_clean": all(
                    not terminal["manifest"].get("outcome")
                    and not terminal["manifest"].get("deleted")
                    and not terminal["manifest"].get("error")
                    for terminal in terminals
                ),
                "same_tenant_jobs_never_observed_overlapping": reindex[
                    "max_observed_active_jobs"
                ]
                <= 1,
                "physical_job_count_in_expected_range": expected_job_range[0]
                <= reindex["physical_jobs"]
                <= expected_job_range[1],
            },
        }
        self.write_json(f"scenarios/{name}.json", result)
        return result

    def run(self) -> int:
        self.prepare()
        with FixtureServer(
            self.fixtures, self.args.provider_host, self.args.provider_port
        ) as server:
            self.provider_url = server.base_url
            results = [
                self.run_scenario(name)
                for name in ("combined", "consecutive", "overlapping", "burst")
            ]
        passed = all(all(item["checks"].values()) for item in results)
        summary = {
            "finished_at": now_iso(),
            "passed": passed,
            "scenarios": [
                {
                    "scenario": item["scenario"],
                    **item["reindex"],
                    "checks": item["checks"],
                }
                for item in results
            ],
        }
        self.write_json("summary.json", summary)
        print(json.dumps(summary, indent=2))
        return 0 if passed else 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True, help="Dedicated HFS base URL")
    parser.add_argument("--hfs-log", required=True, type=Path, help="HFS INFO log file")
    parser.add_argument("--output-dir", required=True, type=Path, help="New evidence directory")
    parser.add_argument("--tenant", help="Tenant prefix; default is unique per run")
    parser.add_argument("--resources-per-manifest", type=int, default=200)
    parser.add_argument("--preexisting", type=int, default=20)
    parser.add_argument("--burst-manifests", type=int, default=4)
    parser.add_argument("--provider-host", default="127.0.0.1")
    parser.add_argument("--provider-port", type=int, default=0)
    parser.add_argument("--poll-interval", type=float, default=0.1)
    parser.add_argument("--quiet-seconds", type=float, default=1.0)
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args()
    if args.resources_per_manifest < 1:
        parser.error("--resources-per-manifest must be positive")
    if args.preexisting < 1:
        parser.error("--preexisting must be positive")
    if args.burst_manifests < 3:
        parser.error("--burst-manifests must be at least 3")
    if args.poll_interval <= 0 or args.quiet_seconds <= 0 or args.timeout <= 0:
        parser.error("poll, quiet, and timeout values must be positive")
    return args


def main() -> int:
    try:
        return Controller(parse_args()).run()
    except (OSError, RuntimeError, ValueError) as error:
        print(f"measurement failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
