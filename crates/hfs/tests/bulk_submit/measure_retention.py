#!/usr/bin/env python3
"""#995 diagnostic campaign: instrumented manager inventory and sparse malloc snapshots.

Uses the same CLI as measure_memory.py. Requires its temporary Rust probe patch
and HFS_995_RETENTION_PROBE=1. Does not enable cleanup or change ingestion.
"""
import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path

import measure_memory as m


class RetentionController(m.Controller):
    def snapshot(self, job, label):
        out = self.out / "retention"
        out.mkdir(exist_ok=True)
        driver = Path(__file__).resolve()
        self.write_json("retention/driver.json", {
            "path": str(driver), "sha256": hashlib.sha256(driver.read_bytes()).hexdigest(),
            "checkpoints": "startup job 1; postidle jobs 1, every 5, and final",
            "timings_comparable_to_uninstrumented": False,
        })
        pid = self.hfs.pid
        for name, command in [
            ("footprint", ["/usr/bin/footprint", "-p", str(pid), "--swapped", "--noCategories"]),
            ("vmmap", ["/usr/bin/vmmap", "-summary", str(pid)]),
            ("heap", ["/usr/bin/heap", "-s", "--noContent", str(pid)]),
        ]:
            self.raise_if_aborted()
            started = time.monotonic()
            record = {"job": job, "label": label, "pid": pid, "command": command,
                      "wall_iso": m.iso_now(), "mono_s": started}
            raw = out / f"job{job:02d}-{label}-{name}.txt"
            try:
                with raw.open("w") as output:
                    result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=15)
                record["returncode"] = result.returncode
            except subprocess.TimeoutExpired:
                record["timeout"] = True
            except OSError as exc:
                record["error"] = str(exc)
            record.update(duration_s=time.monotonic() - started, raw=str(raw))
            with (out / "snapshots.jsonl").open("a") as output:
                output.write(json.dumps(record) + "\n")
            if record.get("returncode") != 0:
                raise m.Aborted("retention_snapshot_failed", record)

    def inventory(self, job):
        job_id = self.current_attempt["reindex"]["job_id"]
        log_path = self.hfs.log_path
        offset = log_path.stat().st_size
        response = m.http_json("GET", f"{self.base_url}/$reindex-status/{job_id}")
        params = m.parameters_map(response["json"]) if response["status"] == 200 else {}
        if params.get("status") != "completed":
            raise m.Aborted("retention_job_not_quiescent", {"status": response["status"], "parameters": params})
        deadline = m.mono() + 5
        matches = []
        while m.mono() < deadline:
            self.raise_if_aborted()
            with log_path.open("rb") as log:
                log.seek(offset)
                lines = log.read().decode(errors="replace").splitlines()
            matches = [line for line in lines if "[DEBUG-995-retention]" in line and job_id in line]
            if matches:
                break
            time.sleep(0.05)
        if len(matches) != 1:
            raise m.Aborted("retention_inventory_missing_or_ambiguous", {"job": job, "matches": matches})
        fields = {key: int(value) for key, value in re.findall(r"\b(\w+)=(\d+)\b", matches[0])}
        required = {"jobs_len", "jobs_capacity", "finished_jobs", "channels_len", "channels_capacity",
                    "closed_channels", "occupied_entry_bytes", "owned_buffer_capacity_bytes"}
        if not required.issubset(fields):
            raise m.Aborted("retention_inventory_parse_failed", {"raw": matches[0], "fields": fields})
        record = {"job": job, "pid": self.hfs.pid, "mode": self.args.mode,
                  "wall_iso": m.iso_now(), "mono_s": m.mono(), "job_id": job_id,
                  "fields": fields, "raw": matches[0], "sample_point": "after idle before heap and validation"}
        with (self.out / "retention-inventory.jsonl").open("a") as output:
            output.write(json.dumps(record) + "\n")
        self.log.line("retention_inventory", job=job, **fields)

    def phase(self, name, job, extra=None):
        result = super().phase(name, job, extra)
        if name == "prekickoff_baseline" and job == 1:
            self.snapshot(job, "startup")
        return result

    def idle(self, job):
        result = super().idle(job)
        self.inventory(job)
        if job == 1 or job % 5 == 0 or job == self.args.jobs:
            self.snapshot(job, "postidle")
        return result


if __name__ == "__main__":
    m.Controller = RetentionController
    sys.exit(m.main())
