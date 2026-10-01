# Evidence bundle — #1173 `pg-es` row with Keycloak auth ON

Layout follows the #1171 bundle (#1588). `main` `e063ef8ab`, release build R4-only (see `outputs/deviations.txt`, D2). Host: Windows 11 x86_64, 20 cores, 64 GB; Docker Desktop (VM 32 GB) runs PostgreSQL 16, Elasticsearch 8.15.0 (8 GB heap) and Keycloak 26.1 with the repo's `docker/keycloak/realm.json`; the full 18,955,865-resource Synthea corpus is served locally by a Node HTTP/1.1 static server. `RESULTS.md` holds the matrix row, every step's outcome and the findings list.

Evidence branch only; not proposed for merge.

## T3 timing (from the store, not a stopwatch)

| | |
|---|---|
| Submission | `synthea-pg-es-keycloak`, id `1a5b8df6-4dd2-4510-a2c6-68eb1cfcc7bc`, started from the Import page by the signed-in `demo` user (plan B) |
| `<T3 start>` | `2026-10-01T09:43:55.568Z` (`bulk_submissions.created_at`) |
| `<T3 end>` | `2026-10-01T15:34:16.777Z` (detail page *Processing finished at*); manifest `transactionTime` 15:33:10.173Z |
| Elapsed | 5 h 49 min 15 s (to the manifest) |
| Outputs / errors | 24 output files, 18,955,733 resources; 1 `outcome` file with 132 `error` entries (Provenance stored but not indexed after a 30 s Elasticsearch write timeout) |
| Fully searchable | 16:13:46Z, after the automatic deferred Provenance reindex (`errorCount` 0) |
| PostgreSQL / Elasticsearch | 92 GB / 44.1 GB, 64 shards; every type equal to the corpus (plus T2) after the reindex |
| HFS peak memory | 834 MB working set |

## Logs

- `hfs-pg-es-keycloak.log` — every start of the pass appended, each with a banner listing the environment (the outbound token redacted). `HFS_LOG_LEVEL=info`.
- `outputs/hfs-events.txt` — every stop/start of HFS and detached helper with its reason and Windows pid.
- `outputs/build.log` — T0.
- `outputs/outbound-token.txt` — mint time, expiry and lifespan of every `HFS_OUTBOUND_BEARER_TOKEN`, one line per HFS start.
- `outputs/keycloak-runtime-changes.txt` — every change made to the running realm through the admin API (plan B, D5; the repo `realm.json` is never edited).
- `outputs/t3-monitor-api.log` — one sample per minute of the ingest (`x-progress`, Elasticsearch docs, HFS memory); `outputs/t3-final-status.json` — the completion manifest; `outputs/t3-outcome-132.ndjson` — its 132-entry outcome file.
- `outputs/t3-monitor.log` — the coarser 5-minute host monitor (PostgreSQL size, container stats).
- `outputs/webhook.log` — every rest-hook delivery of T8/T9.
- `outputs/corpus-server.log`, `outputs/corpus-server-headers.txt` — the static server's request log and its HTTP/1.1 keep-alive headers.

## Results files

- `outputs/a1-a5.out`, `outputs/smart-configuration.json` — A1–A5.
- `outputs/t1-smoke.txt` — T1 smoke and startup-log lines.
- `outputs/api-checks.log` — every API cross-check, with a fresh `hfs-backend-client` bearer (D8).
- `outputs/corpus-served-bytes.txt`, `outputs/corpus-counts.tsv`, `outputs/es-counts.tsv`, `outputs/pg-counts.tsv`, `outputs/t3-counts.txt` — corpus vs PostgreSQL vs Elasticsearch per type.
- `outputs/t4-api-sweep.tsv` — the 55 T4 searches run through the API (total, included, rows, first values).
- `outputs/t5-api/<name>/` — each T5 `$export` run through the API: kick-off headers/body, status URL, manifest, `log.txt` with lines per file and per type.
- `outputs/t7-api/<name>/` — each T7 `$sql-export` run through the API: kick-off body, result manifest, `log.txt` with lines/rows/schema per output.
- The full export files themselves (about 90 MB of synthetic NDJSON/CSV/Parquet) are not committed; their counts are in the `log.txt` files and they can be regenerated with the same kick-offs.
- `outputs/deviations.txt` — D1–D8.

## Screenshots (`shots/`, 84 files)

Named `<step>-<what>.png`:

- Auth and environment: `expiry-*` (expired-token observation), `planb-*` (Keycloak login, signed-in UI), `harness-logout-invalid-redirect-port-18080.png`.
- T1: `t1-ui-dashboard.png`. T2: `t2-6x-*` (without and with a session).
- T3: `t3-import-page-no-session-401.png`, `t3-import-list-empty.png`, `t3-dialog-filled.png`, `t3-submission-created.png`, `t3-import-list-failed-during-reindex.png`, `t3-import-detail-failed-1-error-file.png`, `t3-dashboard-during-provenance-reindex.png`.
- T4: `t4-81-*` (fixtures), `t4-41-*`, `t4-42-*`, `t4-46c-*`, `t4-412-*`, `t4-414-*`, `t4-425-*`.
- T5: `t5-clear-all-resources.png`, `t5-51-*`, `t5-52-one-patient-failed-500.png`, `t5-56-*`, `t5-512-*`, `t5-513-*`, `t5-delete-one-patient-*`.
- T6: `t6-101-*`, `t6-102-*`, `t6-103-*`.
- T7: `t7-111-*`, `t7-112-*`, `t7-subjects-table.png`, `t7-7f-*`, `t7-7l-*`, `t7-115-*`, `t7-page-*`, `t7-restart-cancelled.png`.
- T8: `t8-*`. T9: `t9-*` (steps 1–6).

## Samples (`samples/`)

- `t5-51-Organization-1.ndjson` — the 140-line second Organization file of 5.1.
- `t5-53-group-active-condition.ndjson` — the single active Condition of 5.3.
- `t7-7c-female_patients.parquet` — 7.c, 5,814 rows (`id, birth_date, city`).
- `t7-7g-one-patient.ndjson`, `t7-7h-one-group.json` — 7.g and 7.h (the anchor patient).
- `t7-7m-vd-csv-first-20-rows.csv` — header and first 20 rows of 7.m.

## Issues filed from this pass

_None yet: findings are listed in `RESULTS.md` and filed only after the tester approves them._
