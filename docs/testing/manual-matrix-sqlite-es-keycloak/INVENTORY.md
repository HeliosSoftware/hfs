# Evidence bundle — #1171 `sqlite-es` row with Keycloak auth ON

Collected 2026-09-29 03:00Z on the test host (Linux, 8 cores, 23 GB). `main` `0eb99c238`, release build (`cargo build --workspace --all-features --release`, `CARGO_BUILD_JOBS=1`). Keycloak 26.1.5 (native, repo `realm.json`), Elasticsearch 8.15.0 (native, 4 GB heap, index prefix `hfs_20260928_1171`), full 18,955,865-resource Synthea corpus served locally by a Node HTTP/1.1 static server.

## T3 timing (from the store, not a stopwatch)

- submission `3ad87fb2-e899-422a-9574-41fc2063488a` (`synthea-sqlite-es`), recipient id `7fb981dd-ecb7-4b66-873d-82af27ab7e10`
- created `2026-09-28T21:33:56.340Z`; final manifest `transactionTime` `2026-09-29T02:34:42.749Z`
- **elapsed 5 h 00 m 46 s (18,046 s)**; searchable at completion (index-during-ingest, queue 64 / concurrency 8 / coalesce 4, `bulk-submit indexed every resource during ingest; no deferred reindex needed` at 02:34:18.716Z)
- 24 output files, 0 error files; `hfs.db` 19 GB; ES indices 49.9 GB (56 shards)

## Logs

- `hfs-sqlite-es-keycloak.log` — every restart of the pass appended, **filtered to INFO/WARN/ERROR** (the raw `HFS_LOG_LEVEL=debug` file is 349 MB; 1,206 non-debug lines). 0 `ERROR`. The `Token expired` WARNs on `/bulk-submit-status` between 22:43Z and 22:57Z are the tester's monitor sending a stale `.curlrc` bearer (see `outputs/deviations.txt`), not HFS.
- `outputs/expiry-log-excerpt.txt` — the debug lines of the outbound-token expiry (Import poll 401, conformance self-calls 401).
- `outputs/t1-attempt-index-during-ingest.log` — HFS refusing the issue's literal `HFS_BULK_SUBMIT_INDEX_DURING_INGEST=true` (removed flag).
- `outputs/build.log.tail`, `outputs/build-attempt1-oom.log` — the successful single-job build and the OOM-killed 6-job attempt.
- `outputs/t3-monitor-api.log` — one sample per minute of the ingest (`x-progress`, ES docs, DB size); `outputs/t3-final-status.json` — the recipient's completion manifest.
- `outputs/webhook.log` — every rest-hook delivery of T8/T9 (handshakes, event notifications, retries).

## Results files

- `outputs/a1-a5.out` — A1–A5 with the full OperationOutcomes.
- `outputs/t2-*.out` — Batch page runs (6.1 without session, 6.1–6.4 with session).
- `outputs/t3-kickoff.out` — Import page kick-off.
- `outputs/t4-results.txt`, `outputs/t4-results-lpid.txt` — the 84 + 19 T4 searches with totals versus expectations.
- `outputs/t4-t9.out`, `outputs/t7-results.txt`, `outputs/t7-results-2.txt` — T5–T9 API runs and the `$sql-export` matrix.
- `outputs/corpus-served-bytes.txt` (24 ok / 0 TRUNC), `outputs/corpus-counts.tsv`, `outputs/rail-counts.tsv` (API `_summary=count`), `outputs/es-counts.tsv` (ES `_count` with HFS's filter).
- `outputs/deviations.txt`, `outputs/outbound-token.txt` (mint times and expiries of every outbound bearer).

## Screenshots (`shots/`)

`t1-ui-dashboard`, `t2-61-negative` (no session: raw 401), `t2-61-negative-session`, `t2-62-hospital-session`, `t2-62-practitioner-session`, `t2-63-transaction-session` (Per-Action Outcomes, 662 created), `t2-76-duplicate-session`, `t3-submission-created`, `t3-compartments-valid-token` / `-expired-token`, `t3-search-parameters-valid-token` / `-expired-token`, `t3-detail-after-token-expiry`, `t3-import-list-expired-token`, `t3-submission-detail-final`, `t3-import-list-final`, `t3-ui-dashboard-after-import`, `t4-resources-session`, `t5-bulk-exports-list` / `-session`, `t6-view-definitions`, `t7-sql-exports-list` / `-session`, `t9-subscriptions-before`, `t9-subscriptions-after-step3`, `t9-failure-path`, `t9-subscriptions-after-reactivation`, `t9-subscriptions-after-restart`, `t9-subscriptions-disabled`.

## Samples

- `samples/bulk-export-5.1-Organization.ndjson` — T5 5.1 (1,140 lines, both chunks concatenated).
- `samples/sqlexport-vd-csv-patient_demographics.csv`, `samples/sqlexport-vd-parquet-patient_demographics.parquet`, `samples/sqlexport-view-ndjson-female_patients.ndjson` — one `$sql-export` sample per format produced; JSON: `samples/sqlrun-json-patient_demographics.json` (`$sql-run`, since every JSON `$sql-export` in the pass carried the SQL Query subject and failed on the row limit, #1473/#1570). Note the CSV/JSON/NDJSON ViewDefinition samples show the two-column output of #1569.

## Issues filed from this pass

#1560, #1561, #1569, #1570, #1571.
