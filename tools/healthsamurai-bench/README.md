# HealthSamurai benchmark harness (HFS next to WSO2)

Local harness used on 2026-11-18 to run HFS against the upstream
[HealthSamurai FHIR Server Performance Benchmark](https://github.com/HealthSamurai/fhir-server-performance-benchmark),
next to WSO2, with upstream's own Docker Compose stack, k6 suites and container
limits. It is separate from the CI workflow `.github/workflows/fhir-benchmark.yml`,
which runs HFS natively on a runner. Tracked in #2612.

Nothing here is pushed to or proposed to HealthSamurai.

## What is here

| Path | What |
|---|---|
| `hfs/Dockerfile`, `hfs/Dockerfile.dockerignore` | Release build of HFS (R4 only, clang + lld, 8 MB stack). `FEATURES` build arg picks the Cargo features. The own ignore file is needed because the build context's `.dockerignore` excludes `**/*.rs`. |
| `docker-compose.hfs.yaml` | HFS on Postgres: 8 CPU / 24 GB, database `hfs` on upstream's shared `postgres:18`, `HFS_PG_MAX_CONNECTIONS=32`. |
| `docker-compose.hfses.yaml` | HFS on Postgres + Elasticsearch, Elasticsearch **inside** the 8 CPU / 24 GB budget: HFS 6 CPU / 20 GB, Elasticsearch 2 CPU / 4 GB (2 GB heap). |
| `docker-compose.hfses-wide.yaml` | The same, with Elasticsearch on **its own** 8 CPU / 30 GB, like upstream's database containers (HFS keeps 8 CPU / 24 GB). |
| `upstream-e6a4941.patch` | The two upstream files that must change: `runner.sh` (adds the `hfs` and `hfses` servers) and `infra/postgres/init/create-database.sql` (`CREATE DATABASE hfs;`). Apply on upstream commit `e6a4941`. |
| `scripts/run-matrix.sh` | HFS (Postgres) and WSO2, 3 rounds, alternating which goes first, fresh database before each run. |
| `scripts/run-es.sh` | HFS Postgres + Elasticsearch, 3 rounds, fresh Elasticsearch volume and database per round, drain gate between import and search. |
| `scripts/report.py`, `scripts/analyze.py` | Medians of 3 rounds, spread and ratios. `analyze.py` also writes `bench-runs.json` (one row per server x suite x round). |
| `results/` | k6 logs per run (`R<round>-<server>-<suite>.log`) and the JSON metrics in `results/data/`. |

`results/es-shared-budget/failed-R2-import-9-errors/` is the first attempt of
round 2 (9 of 2005 import requests failed, cause unknown). It is kept as
evidence and is not part of any median.

## How to rerun

1. Clone upstream and apply the patch:
   ```bash
   git clone https://github.com/HealthSamurai/fhir-server-performance-benchmark
   cd fhir-server-performance-benchmark
   git checkout e6a4941
   git apply <hfs>/tools/healthsamurai-bench/upstream-e6a4941.patch
   cp -r <hfs>/tools/healthsamurai-bench/hfs .
   cp <hfs>/tools/healthsamurai-bench/docker-compose.hfs*.yaml .
   ```
2. Build the images from a clean `git archive` export of the HFS commit to
   measure (the working tree can hold build output and secrets):
   ```bash
   git -C <hfs> archive --format=tar --prefix=ctx/ <commit> | tar -x -C <ctx-parent>
   docker build -f hfs/Dockerfile -t hfs-bench:main-c52ff36 <ctx>
   docker build -f hfs/Dockerfile --build-arg FEATURES="R4,sqlite,postgres,elasticsearch" \
     -t hfs-bench:main-c52ff36-es <ctx>
   ```
   The tags are the ones the compose files use; change both if you build another commit.
3. Run, one server at a time on a quiet machine:
   ```bash
   export BENCH=$PWD
   bash <hfs>/tools/healthsamurai-bench/scripts/run-matrix.sh          # HFS Postgres + WSO2
   ES_COMPOSE=docker-compose.hfses.yaml      OUT=$PWD/results-local-es \
     bash <hfs>/tools/healthsamurai-bench/scripts/run-es.sh            # ES inside the budget
   ES_COMPOSE=docker-compose.hfses-wide.yaml OUT=$PWD/results-local-es-wide \
     bash <hfs>/tools/healthsamurai-bench/scripts/run-es.sh            # ES on its own budget
   ```
   Round counts come from `ROUNDS` (default `1 2 3`). The scripts start the
   `postgres`, `prometheus` and `tgz` containers they need, and abort a round
   if HFS does not log `Composite storage initialized`. On Windows run them
   from Git Bash; they set `MSYS2_ARG_CONV_EXCL="*"`.
4. Analyse: point `RESULTS_PG`, `RESULTS_ES_SHARED` and `RESULTS_ES_OWN` at the
   new result folders (by default the scripts read `results/` next to them) and
   run `python scripts/analyze.py`.

## Conditions of the recorded run

- HFS built from main commit `c52ff36`, FHIR R4, release profile.
- Host: Docker Desktop on WSL2, VM with 44 GiB of memory and 20 vCPU. Upstream's
  reference machine has 64 cores, 500 GB of memory and NVMe, so compare ratios,
  not absolute values.
- Upstream suites and order: prewarm (10 VUs, 30 s), CRUD (300 VUs, 4 min),
  import (20 VUs, 1000 Bundles), search (30 VUs, 2 min); no VACUUM.
- Postgres: upstream's `postgres:18` (8 CPU / 30 GB, `postgres.conf` unchanged),
  one database per server, recreated before each run.
- Elasticsearch: `elasticsearch:8.15.0`, single node, 2 GB heap, synchronous
  composite mode (`HFS_COMPOSITE_SYNC_MODE=synchronous`, write refresh `wait_for`,
  refresh interval 50 ms, repair interval 60 s), the setting the CI benchmark uses.
- WSO2: `ghcr.io/wso2/fhir-server:latest` (reports 2.2.0). WSO2 was measured once,
  not repeated for the Elasticsearch runs, because the conditions are identical.

## Results (medians of 3 rounds)

| Suite | HFS Postgres | HFS + ES, shared budget | HFS + ES, own budget | WSO2 |
|---|---:|---:|---:|---:|
| Prewarm, req/s | 2033 | 81 | 173 | 1826 |
| CRUD, req/s | 2567 | 267 | 964 | 2793 |
| Import, total seconds (lower is better) | 736 | 657 | 496 | 833 |
| Search, req/s | 2646 | 238 | 1094 | 2273 |

Run `scripts/analyze.py` for the full table with round ranges, p50/p95 and ratios.

## Caveats

- With Elasticsearch, writes are capped by the synchronous refresh. An
  asynchronous run was not done.
- Search with Elasticsearch is slower than HFS on Postgres alone in both
  budgets (p50 102 ms shared and 16.8 ms own, against 7.9 ms); the cause was not investigated.
- The k6 import check accepts any HTTP 200, and k6 reports p90 and p95 only.
- The elasticsearch document count after import (1,965,870) differs from the
  Postgres resource count (1,632,062); it was not reconciled and probably
  counts contained and nested documents.
- Which of the two Elasticsearch budgets HealthSamurai would accept is unknown.
