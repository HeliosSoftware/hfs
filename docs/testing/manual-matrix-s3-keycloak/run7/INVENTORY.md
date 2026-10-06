# Run 7 — #1178 `s3` (MinIO) + Keycloak, SQL on FHIR on the full corpus, `main` `933e4b926` — **partial: memory guard tripped**

Collected 2026-10-06 13:20Z → 16:06Z. `hfs 0.2.4 (git 933e4b926)` includes #1782 (for #1671). The release build (`CARGO_BUILD_JOBS=1`, 1 h 57 m 58 s, peak RSS 16.3 GB; `outputs/build-tail.txt`) ran with HFS and MinIO stopped. MinIO runs on `data-1178c`, which holds the full 18,955,865-resource corpus; nothing was imported. Auth is on, `HFS_LOG_LEVEL=info`, and the **realm lifespan is 300 s** (default).

## Guards (`outputs/guard.sh`, `outputs/guard.log`, 85 samples every 30 s)
- MemAvailable below 2 GiB.
- Swap growing in each of 10 consecutive samples by more than 256 MB in total, or above baseline + 3 GiB (baseline 1,656 MB).
- Disk floor 135 GiB, checked before every export.

A guard trip sends `DELETE /export/{id}/status` for the running job.

**Triggered 3 times:**
1. **15:58:25Z:** +937 MB in 5 min → cancelled 7.b `query-csv` (`44482b78`, 202).
2. **15:59:55Z:** no job running.
3. **16:01:25Z:** no job running.

After the first trip:
- **Stray job:** the batch had already started 7.o `query-ndjson` (`ea776c96`, 15:58:29Z). It was stopped and the job cancelled by hand at 16:00Z (202, then 404).
- **Not started:** 7.o, `observation_flat` NDJSON/Parquet and 7.e (which needs an `observation_flat` scan) were held back, waiting for Angela's decision.

## T6 ✅ (`outputs/t6-api.out`)
`$sql-run` of `patient_demographics` returns 11,705 rows in every format, with the five columns and the `PID` row:

| format | time |
|---|---|
| json | 4.59 s |
| csv | 2.30 s |
| ndjson | 2.25 s |
| parquet | 2.27 s |

## #1671 with a session ✅ / #1821 (`outputs/t1671-check.out`, `shots/t1671-*`)
The outbound token expired at 15:24:49Z, and the six pages were reloaded at 15:25:21Z under the `demo` session.
- **Load with the session:** View Definitions, SQL Queries, SQL Views and New SQL Export (5 subjects), with no notice and no HTTP ≥ 400.
- **No 401 in the log** (0 *Token expired* / *Authentication failed* lines).
- **Still fail:** Compartments and Search Parameters (0 parameters) keep the degraded notice. The cause is not the token: their self-call is a search, and S3 answers it with **501** (`capability 'search' not supported by S3`). The notice blames the outbound token anyway → **#1821**.

## T7 (`outputs/t7-results.txt`, `outputs/t7-page-flows.out`)
- **SQL View `female_patients`:**
  - NDJSON: 5,813 lines, 10 s, hfs max RSS 546 MB.
  - Parquet: 5,813 rows, 10 s, hfs max RSS 565 MB.
- **7.b `query-csv`** (`tall_female_patients`, `min_height = 150`): kicked off 15:28:07Z, **not complete after 30 min**. In that time hfs RSS grew to 1,280 MB (VmHWM 1,324 MB). The host then swapped it out: VmSwap reached 1.47–1.66 GB at up to 64 MB/s while `/proc/meminfo` showed ~20 GB available and no container cgroup limit. The guard cancelled it. No row-limit message appeared before the cancel. The shape matches #1473 (the depends-on view is materialized in full before the `WHERE`) and #1705.
- **Independent count** from the corpus files: **4,944** female patients with an 8302-2 Observation > 150 (`outputs/independent-count.txt`). It could not be compared, because 7.b and 7.o did not complete.
- **Page flows** (no Observation scans): 7.f / 7.l exact texts; `patient_demographics` kick-off Complete; `broken_query` Failed with the SQLite error and Retry → a new card with the original untouched; Run again → a new card; Remove → 404.
- **After the cancels:** hfs CPU dropped to ~2 % with no reads. MinIO stayed at ~55 % CPU and its RSS climbed to 3.74 GB with no job running (its background scanner over 18.9 M objects); the swap levelled at ~3.3–3.7 GB.

## Resources (`outputs/resources-summary.txt`)
| | |
|---|---|
| MemAvailable minimum | 19,074 MB |
| swap used max | 3,683 MB (baseline 1,656 MB) |
| `hfs` RSS max | 1,280 MB (VmHWM 1,324 MB), plus up to 1.66 GB swapped |
| `minio` RSS max | 3,740 MB |
| disk | 306–307 GiB free throughout (66 % used) |

## Issues
#1821 (new). Context, already open: #1473, #1705.
