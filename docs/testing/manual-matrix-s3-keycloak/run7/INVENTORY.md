# Run 7 — #1178 `s3` (MinIO) + Keycloak, SQL on FHIR on the full corpus, `main` `933e4b926` — **closed: heavy exports not viable on standalone s3 (decision 2026-10-06)**

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
- **Not started:** 7.o and `observation_flat` NDJSON/Parquet. Angela's decision is that they are **not viable on standalone s3 with the full corpus**: the in-process runner materializes the whole view, reading all 7.7 M Observations, before any filter (#1473, #1705). They are not to be retried. This is not a failure of the #1715 fix.

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
#1821 and #1823 (new). Context, already open: #1473, #1705. Reproduced on the earlier binary: #1704 (fixed by #1788). Pending: when #1822 merges, a short pass to check that Compartments and Search Parameters load on s3.

## Orphan scans after the cancels (`outputs/orphan-scan-evidence.txt`)
The cancelled 7.b / 7.o jobs kept scanning inside hfs. The binary (`933e4b926`) predates #1788 (merged 14:00Z as `ba8bd71ed`), so this is the known #1704:
- hfs VmSwap went from 1.66 GB at 16:05Z to 7.1 GB at 18:17Z;
- hfs was still receiving ~0.8 MB/s from MinIO.

HFS was stopped to end the scans (`ba8bd71ed` = #1788).

## 7.e Cancel — ❌ → #1823 (`outputs/t7e.out`, `outputs/t7e-after.out`, `outputs/t7e-summary.txt`)
**Binary:** a new release build of `main` `04bb721d5`, which includes #1788 (26 m 28 s, HFS/MinIO stopped during it; `outputs/build-b-tail.txt`).

**Before the kick-off:** the guard was restarted with a new swap baseline (1,551 MB), and MinIO stayed under 10 % CPU for 5 min. hfs baseline: VmRSS 40 MB, VmSwap 0.

**What happened:**
1. `observation_flat` NDJSON was kicked off from the page at 18:51:27Z.
2. **Cancel** was pressed 60 s into *In progress*, at 18:52:44Z (303).
3. The card reached **Cancelled** within 10 s.
4. **But the scan did not stop.** In the first 5 min hfs received 214,564 KB more from MinIO and its RSS rose 110 → 275 MB. Over the next 1 h 25 m it received 3,529 MB more, with no idle period.
5. hfs RSS + swap reached 3,078 MB.
6. At 20:23:34Z, 1 h 31 m after the cancel, the 3 GB safety cap restarted HFS. The new process is back at 40 MB.

**Cause:** on S3, `scan_resources` lists every key of the type before the first row. #1788's cancel check only runs before the runner starts and then every 4,096 rows, so it never sees the cancel during the listing → **#1823**.

**Guard:** not triggered during 7.e. Swap peaked at 4,401 MB (+2,850 MB over the baseline, below the +3 GiB cap). MemAvailable minimum 21,081 MB; hfs RSS max 825 MB.

**Criterion** (Cancelled, and hfs RSS back to its earlier level within minutes): **not met.**
