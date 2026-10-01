# Evidence bundle — #1178 `s3` (MinIO) row with Keycloak auth ON

Collected 2026-10-01 01:40Z. `main` `0eb99c238`, the #1171 release binary reused (`--all-features`, sha256 `cf65bec2…`). Keycloak 26.1.5 native (repo `realm.json`), MinIO `RELEASE.2025-09-07T16-13-09Z` native on a fresh data directory (buckets `hfs`, `hfs-export`, `hfs-sql-export`), full 18,955,865-resource Synthea corpus served by the Node HTTP/1.1 static server.

## T3

- submission `a503bf77-27dd-4ac8-8069-8e9631b8bfbe` (`synthea-s3`), recipient `2b860e2a-0db3-42e4-81e5-bd408e87641f`, created `2026-09-29T14:52:43Z`, ~220 resources/s.
- 2026-09-30 14:14Z: the tester's debug log filled the host disk → MinIO `507 XMinioStorageFull` for ~2 min → lease reclaim → the worker replayed the manifest from file 1 (#1610). At 2026-10-01 01:25Z, with `23,46x,xxx Resources written` (> the corpus) and files 1–8 replayed, the job was cancelled with `DELETE /bulk-submit-status/{id}` → 202 (the Import page offered no Abort once the expired-token poll had marked it failed, #1561). **Not Completed.** Every type up to `Procedure` was in the bucket before the replay; `Provenance` and `SupplyDelivery` never were.
- `outputs/t3-monitor-api.log` — one sample per minute (`x-progress`), including the 401 lines of the tester's own stale bearers (noted in `outputs/deviations.txt`).

## Logs

- `hfs-s3-keycloak.log` — **INFO/WARN/ERROR only**, stitched from: the first 50 MB and the last 2 GB of the original debug log before it was truncated at 902 GB apparent (14:16Z, 2026-09-30), the rolling non-debug extracts taken every 20 min after that (`outputs/log-trim.log`), and the final log after the last restarts. 417 lines. The raw debug log is not kept (it is what filled the disk).
- `outputs/webhook.log` — every rest-hook delivery of T8/T9.

## Results files

`outputs/a1-a5.out`, `outputs/t2-*.out` (Batch page: baseline and session), `outputs/t3-kickoff.out`, `outputs/t7-results.txt` (ViewDefinition / SQL View subjects), `outputs/t7-query-csv.txt` + `outputs/t7-query-csv-poll.log` (SQL Query subject: 812 × `202` over 6.9 h of in-process scanning, then lost to the restart — `not found or was cancelled`), `outputs/sql-run-csv-head.txt`, `outputs/deviations.txt`, `outputs/outbound-token.txt`.

## Screenshots (`shots/`)

`t1-ui-dashboard`, `t2-61-negative` (no session), `t2-61-transaction-session`, `t2-62-hospital-session`, `t2-62-practitioner-session`, `t3-submission-created`, `t3-detail-after-token-expiry`, `t3-compartments-expired-token`, `t3-search-parameters-expired-token`, `t4-resources-session`, `t5-bulk-exports-list` / `-session`, `t6-view-definitions`, `t7-sql-exports-list`, `t9-subscriptions-before`, `t9-subscriptions-after-step3`, `t9-failure-path`, `t9-subscriptions-after-reactivation`, `t9-subscriptions-after-restart`, `t9-subscriptions-disabled`.

## Samples

`samples/sqlexport-vd-csv-patient_demographics.csv` (header + 11,705), `samples/sqlexport-vd-parquet-patient_demographics.parquet`, `samples/sqlexport-view-ndjson-female_patients.ndjson` (5,813), `samples/sqlexport-one-patient.ndjson`, `samples/sqlrun-json-patient_demographics.json` (`$sql-run`). No Bulk Data sample: `$export` is 501 on this row. No JSON `$sql-export` sample: the only JSON export carries the SQL Query subject.

## Issues filed from this pass

#1590, #1610.
