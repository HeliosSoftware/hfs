# Run 3 — #1178 `s3` (MinIO) + Keycloak, `main` `ff497ddbc` (with #1614)

Collected 2026-10-01 18:56Z → 2026-10-02 13:56Z. Release build of `ff497ddbc` (`--all-features`, rustc 1.99.0), MinIO `RELEASE.2025-09-07T16-13-09Z` on a fresh data directory, Keycloak 26.1.5 native. Corpus: `manifest-half.json`, the 24-file Synthea manifest minus `Observation` and `ExplanationOfBenefit` — **22 files, 9,650,469 resources**. Interactive login configured (`HFS_UI_LOGIN_CLIENT_ID=hfs-web`); `HFS_LOG_LEVEL=info`; realm lifespan 3600 s for the run. All deviations: `outputs/deviations.txt`.

## T3 — ❌ #1715

| Instant (UTC) | Event |
|---|---|
| 2026-10-01 19:00:09 | submission `synthea-s3-half-run3b` created on the Import page (session `demo`), recipient accepts the manifest |
| 2026-10-02 07:13:40 | recipient state: `total_entries 9650469`, `success_count 9650469`, `error_count 0`; manifest `phase downloaded`, `files_done 22/22` |
| 2026-10-02 07:14:02 | status `202 Processing 99% - 9,650,469 Resources written - Downloaded 22 of 22 files`, unchanged from then on |
| 2026-10-02 13:08 | receipt spool at 263,992 lines, one 1,000-entry page per ~80 s: the S3 page provider reloads every result object per page (#1715), ~209 h projected |
| 2026-10-02 13:32:40 | cancelled with `DELETE /bulk-submit-status/{id}` → 202, then 404; log: `stopping the manifest in flight`, `lease released by abort`, `run abandoned while writing result receipts` |
| 2026-10-02 13:53:34 | Import page: *Status poll answered 404; polling stopped and the submission is marked failed.* |

Ingest: all 9,650,469 resources written in **12 h 13 m 53 s** from creation (219/s average, 199–236/s per hour, `outputs/t3-run3-monitor.log`). Host during the ingest: CPU idle 78.4 %, iowait 9.8 % (`outputs/t3-run3-perf.log`, 721 samples). MinIO data directory after the run: 213 GB (`du -sh`). No lease reclaim and no replay this time (#1610 / #1614 not exercised: no stall occurred).

7.5 by id (`outputs/t3-75-byid.out`): `GET /Patient/7d24f7a0-…` → Cari853 Esperanza675 Parker433, female, 2015-12-29, v1; searches and `_summary=count` → 501 (no search on this row).

## Other steps on this binary

- **A1–A5** ✅ `outputs/a1-a5.out`.
- **T2** (session) ✅ `outputs/t2-*.out`: 6.1 refused with the backend-atomicity text (#1590 fixed), 6.2 9 + 8 created, 6.4 three refusals.
- **T6** ✅ `outputs/t6-create.out`, `outputs/t6-api.out`: `$sql-run` json / csv / ndjson / parquet 200, 11,705 rows, five columns, `PID` row `…,female,2015-12-29,Parker433,Everett`; `patient=PID` one row; no token → 401. Page: Results **50 rows · 1537 ms**. Rail filter does nothing on `s3` → **#1722**.
- **T7** ✅ `outputs/t7-results.txt`, `outputs/t7-page-flows*.out`: ViewDefinition and SQL View subjects in every format with the expected counts (11,705 / 5,813), `one-patient` 1 line, `release-check-01` echoed; SQL Query subject completes with **0 rows** (no `Observation` in the half corpus, so it cannot be checked for content); negatives 7.f / 7.l with the exact texts; page kick-off **Complete · 1 file · finished in 3m 01s**; 11.6 `broken_query` → **Failed** with *The export stopped on subject broken_query: SQLite error: no such table: table_that_does_not_exist*, Retry → a new card, original untouched; Run again → a new card; Remove → 404. Cancel was not exercised on this run (the cancel request in the harness had no job id); it passed on run 1.
- **T8** ✅ / **T9** ✅ `outputs/webhook.log`, `outputs/t9-6.out`: steps 1–4 as in the run-3 progress comment; step 5 restart → `Subscription engine rehydrated tenants=1 topics=1 subscriptions=1 dormant=0 failed=0`, 0 *Failed to persist*; step 6 `HFS_SUBSCRIPTIONS_ENABLED=false` → *The subscriptions engine is not enabled on this server. Turn it on by starting HFS with: HFS_SUBSCRIPTIONS_ENABLED=true*, sidebar entry present.

## Files

`hfs-s3-keycloak-run3.log` (INFO/WARN/ERROR from the run-3 start), `outputs/`, `shots/` (`*-run3-*` plus three UI-review screenshots), `samples/` (VD CSV, view Parquet, all-three JSON, one-patient NDJSON).

## Issues filed during run 3 and the parallel UI review

#1715 (receipt step), #1722 (SQL rail filter on s3), #1670, #1671, #1672, #1673, #1674, #1675, #1676.
