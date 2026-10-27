# FHIR benchmark scripts

Scripts for `.github/workflows/fhir-benchmark.yml`. The workflow runs one job per storage leg; these files start the backends, run the suites and build the summary.

## Legs

Six legs: `sqlite`, `postgres`, `mongodb`, `sqlite-elasticsearch`, `postgres-elasticsearch`, `mongodb-elasticsearch` (`resolve-matrix.sh` maps the `backend` input to legs).

## Suites, in run order

1. `prewarm`
2. `import`
3. `crud`
4. `search`
5. `sql-run` (HFS's own read-only SQL-on-FHIR suite, `k6/sql-run.js`)
6. `insert`
7. `bulk-import` (`bulk-import.sh` and `bulk_import.py`; not a k6 suite; `$bulk-submit` into a fresh database; always last)

The `indexing` (`$reindex`) suite was removed. Recover it from git history if needed.

## Indexing mode per leg (`bulk-import`)

| Leg | Mode |
|---|---|
| `sqlite`, `postgres`, `mongodb` | deferred (`HFS_BULK_SUBMIT_DEFER_INDEXING=true`): ingest, then one index rebuild |
| the three `*-elasticsearch` legs | inline (`HFS_BULK_SUBMIT_DEFER_INDEXING=false`) with file concurrency 4, batch size 1000, Elasticsearch bulk concurrency 4 |

`bulk-import.sh` sets these; a variable already set in the environment wins. Reasons and measurements: [`$bulk-submit` indexing mode per storage backend](../../../crates/rest/README.md#bulk-submit-indexing-mode-per-storage-backend).

## Report rows

On the Elasticsearch legs the rows `↳ search indexing (seconds)` read `(= ingest)`: the index was built while ingesting, so the value is the ingest time, not a separate rebuild.

## Report outputs

Each run produces, in this order:

- **Job summary text.** The comparison tables (unchanged), then a collapsed `### Charts` section: one for the compare job (`md_charts.py compare --in comparison.md`) and one per leg (`md_charts.py leg --box ... --leg ... --run-dir ...`). Both are display only and never fail the job.
- **Run history block** (`history.py`): the last 5 runs of the branch plus the latest `main` run, as tables and Mermaid charts. Old and new row labels are joined (see `LABEL_ALIASES`), except the old `index rebuild` row, which is a different quantity from `search indexing` and stays apart.
- **Standalone HTML report**, artifact `fhir-bench-report-<run id>`: one dark file (`report_html.py`, template `report_template.html`), uploaded with `archive: false` so it is not zipped. It opens in the browser through the `Full report (HTML)` link on the last line of the summary; that link works for signed-in viewers of the repository only.
  Sections: run summary, 1 Is this run valid?, 2 Results, 3 Detail (timeline and host load, Bulk Import and Batch/Transaction Import by phase, import comparison, `$sql-run` by view, trend), 4 Full data. CI passes `--no-shapes`, so there is no "Search by query shape" section.
- **Report data**, artifact `fhir-bench-report-data-<run id>` (`report-data.json`). Later runs read it (`history.py --data-dir`) for the trend in the HTML report.

All four artifacts and the summary are best effort (`continue-on-error`); a failure here never fails the benchmark.

## Run time

`run_timing.py` splits the wall-clock of a run per leg (jobs API plus `runner-info.txt`):

- **Waiting for resources**: runner queue, the Docker host capacity gate (`capacity_wait_s`) and, if a leg records `host_wait_s`, a wait for a quiet host.
- **Behind earlier legs**: queue time spent because `max_parallel` sequences the legs.
- **Working**: the rest.

Outputs: `run-time.md` and `run-timing.json`. `compare_legs.py --timing run-time.md` inserts the Markdown under the header; `report_html.py --timing-json run-timing.json` draws the HTML tile and timeline. A failure leaves the report unchanged.

## Build job

The workflow compiles **once**: the `build` job runs the only `cargo build` of the workflow and uploads `target/release/hfs`; every leg downloads that binary and picks its backend at runtime through `HFS_STORAGE_BACKEND`. The binary is always built with the full feature set `R4,sqlite,postgres,mongodb,elasticsearch` (FHIR R4 only). Building only the backends of the selected legs saved just about 1-3.5 min on a cold build (measured with 2 jobs) and made the binary differ between dispatches: a build without `sqlite` has no bulk-export job store for mongodb-based servers (the benchmark itself does not call `$export`).

The `build` job sets `CARGO_BUILD_JOBS: 8`; every other job keeps the workflow default of 2. The runners have 8 CPUs and the compile is CPU-bound. Measured with `backend=sqlite tests=prewarm` dispatches on throwaway branches, full feature set, wall time of `cargo build --release`:

| `CARGO_BUILD_JOBS` | `target/` | Wall time | CPU used | Drop in available memory | Largest process | Run |
|---|---|---:|---:|---:|---:|---|
| 2 (before) | cold | 13 min 22 s, 13 min 30 s, 14 min 22 s | - | - | - | runs 40322445009, 40335047310, 40385370213 (cargo's own timing, all 485 crates) |
| 4 | cold | 7 min 03 s | 388 % | 7.4 GiB | 4.5 GiB | run 40521626800 (github-agent6) |
| 8 | cold | 4 min 40 s | 650 % | 7.2 GiB | 4.5 GiB | run 40521631125 (github-agent1) |
| cargo default (8) | cold | 4 min 33 s | - | - | - | run 40507218293 (github-agent1, cache-warming self-check) |
| 4 | warm | 1 min 56 s | 288 % | 2.7 GiB | 2.9 GiB | run 40523191815 (github-agent7) |
| 8 | warm | 37 s | 455 % | 1.7 GiB | 2.5 GiB | run 40523196753 (github-agent6) |

The two warm rows are not cold numbers and not comparable with each other: the 4-job run started from an older build on its runner, the 8-job run reused the `target/` of the 4-job cold run. A runner keeps its `target/` until another workflow checks out into the same workspace with the default `clean: true` (about 30 self-hosted jobs in 17 workflow files do, counted by their checkout step), which is why a full-set build on `github-agent3` can still start cold.

There is no build cache. An `actions/cache` of the release build was tried and dropped: on an exact-key hit from a different runner the restore took 167 s for 1,827 MB and cargo still rebuilt eight workspace crates in 9 min 09 s (12 min 08 s in total, run 40510258406), because the fresh checkout's file times are newer than the restored outputs; a miss paid 126-292 s to save; and every entry used 0.8-1.8 GB of the repository's 10 GB cache quota. The cache was only judged with 2 jobs; a hit with 8 jobs was not measured. The 49.8 s hit quoted earlier (run 40417473144) ran on the runner that had saved that cache minutes before and is not typical.

## Docker host

Every leg uses the remote Docker daemon from the `DOCKER_HOST` secret, shared with the rest of CI (18 workflows use it). The capacity gate (`capacity-gate.sh`) prints the host `MemTotal` (20000 MB in run 40385370213) and waits for enough `MemAvailable` only at the start of a leg; it does not give exclusive use. Overlapping CI jobs can halve crud/search results (run 40353889556), so check running workflows before dispatching.

## Run workflow form

`Use workflow from` selects the branch whose workflow and scripts run. Fields: `backend`, `tests`, `vus`, `duration`, `run_id`, `log_level`; Postgres: `pg_shared_buffers`, `pg_effective_cache_size`, `pg_work_mem`, `pg_max_wal_size`, `pg_min_wal_size`, `pg_wal_buffers`, `pg_checkpoint_timeout`, `hfs_pg_max_connections`; `max_parallel` (1..2); `es_heap`, `es_sync_mode`; `mongo_wt_cache_gb`, `hfs_mongo_max_connections`. The indexing recipes are fixed per leg type in `bulk-import.sh` (see above) and are deliberately not form fields.

## Tests

`.github/workflows/fhir-bench-tests.yml` runs on pull requests touching `.github/scripts/fhir-bench/**`, `fhir-benchmark.yml` or itself (and on manual dispatch): the unit tests, `bash -n` / `py_compile` on every script, and a YAML check of `fhir-benchmark.yml`. Locally:

```bash
cd .github/scripts/fhir-bench && python3 -m unittest discover -s . -p 'test_*.py'
```

## Run the generators locally

```bash
gh run download <run id> -D DIR          # DIR/<artifact name>/...
cd .github/scripts/fhir-bench
python3 md_charts.py compare --in DIR/fhir-bench-comparison-<run id>/comparison.md > charts.md
python3 report_html.py --run-dir DIR --run-id <run id> --no-shapes \
  --out report.html --dump-json report-data.json
```

Drop `--no-shapes` to include the search-by-shape section when `search-points.json` is present. `md_charts.py leg` also needs the leg's box Markdown (`--box`), which CI captures inside the leg job.

## Test GitHub rendering cheaply

Push a throwaway `tmp/` branch whose workflow is triggered by `push`, with `permissions: actions: read`, and have it download the artifacts of an earlier run (`actions/download-artifact` with `run-id` and `github-token`). It then runs the generators above and appends to `$GITHUB_STEP_SUMMARY`, with no servers and no benchmark. Delete the branch afterwards.

## Known limits

- The job summary cannot be previewed locally; check it on GitHub.
- Artifact links, including the HTML report, need a signed-in session with access to the repository.
