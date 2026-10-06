# Run 5 — #1178 `s3` (MinIO) + Keycloak, T3 at scale, `main` `5f01d3fd0`

Collected 2026-10-05 20:33Z → 2026-10-06 04:36Z. Same binary and configuration as run 4 (`hfs 0.2.4`, `HFS_LOG_LEVEL=info`, auth on, login `hfs-web`, realm lifespan 300 s). MinIO on `data-1178c`, which already held the run-3 half corpus and the run-4 Observation subset. No build ran during the import.

## Guards (`outputs/guard.sh`, `outputs/guard.log`, 894 samples every 30 s)
- **Disk floor:** 135 GiB, the larger of 15 % of the 900 GiB subvolume quota and 60 GiB. `zpool` is not available inside the container, so the quota reported by `df` stands in for the pool size.
- **Memory:** cancel if MemAvailable drops below 2 GiB.
- **Swap:** cancel on sustained growth. At first this was "baseline + 1 GiB for 5 min". It was redefined at 02:18Z, after a slow idle-page trickle with no memory pressure, as "swap growing in each of 10 consecutive samples by > 256 MB in total, or baseline + 3 GiB" (`outputs/deviations.txt`).
- **Action:** a guard cancels the job with `DELETE /bulk-submit-status/{id}` and keeps the data.

## 1. `synthea-s3-rest` — cancelled by the disk guard
`manifest-rest.json` = Observation + ExplanationOfBenefit, 9,305,396 resources. Created on the Import page at 20:36:02Z. At 21:06Z the measured slope was 22.9 KB per resource, so the projected end was **98 GiB free (89.1 % used)**, below the 135 GiB floor. Observation alone would have ended at 123 GiB (86.3 %), also below the floor. The job was cancelled at 21:06:18Z (`DELETE` → 202, then 404; `outputs/cancel-1.out`) with 429,300 written. The worker had stored **432,565 ExplanationOfBenefit** by the time it stopped; they stay in the bucket.

## 2. `synthea-s3-obs-head` — ✅
The first **6,500,000** lines of `Observation.ndjson` (`outputs/manifest-obs-head.json`), served by `outputs/prefix-server.js` as an exact byte prefix (6,354,508,616 bytes, HTTP/1.1, no copy on disk; byte and line check in `outputs/corpus-served-bytes-obs-head.txt`). Projected end ~148 GiB free.

| | UTC |
|---|---|
| created on the Import page | 2026-10-05 21:07:21.9 |
| last resource / result / change object; `success_count 6500000` | 2026-10-06 04:29:27.24 |
| manifest `completed` | 04:33:30.81 |
| recipient 200 (`transactionTime`) | 04:34:00.52 |
| **write phase** | **7 h 22 m 05 s** (245 res/s) |
| **receipt step** | **4 m 03.6 s** |

0 errors. The receipt file has **6,500,000 lines, all unique, 0 missing and 0 extra** against the ids of the served prefix (`outputs/t3-receipts-check.txt`). 148,812 of the run-4 subset Observations fall inside the prefix and were rewritten. The bucket now holds **6,526,690 Observation**, exactly the expected 175,502 + 6,500,000 − 148,812 (`outputs/observation-count.txt`, `outputs/expected-observation-count.txt`). Detail page: **Completed · Output files 1 · Error files 0** (`shots/t3-submission-completed.png`).

**Bucket total after the run: 16,609,778 resources**, made up of:
- 9,650,510 from runs 3 and 4 before this run;
- 13 resources from the run-4 steps;
- 6,526,690 Observation;
- 432,565 ExplanationOfBenefit.

## Resources (`outputs/resources-summary.txt`)
| | |
|---|---|
| subvolume (pool stand-in) | 900 GiB |
| final occupancy | **743 GiB used, 157 GiB free, 82.6 %** (minimum free 156 GiB) |
| MemAvailable minimum | **21,752 MB** |
| swap used max | 1,481 MB (baseline 900 MB) |
| `hfs` RSS max | 164 MB (30 s samples), 172 MB (1-min samples) |
| `minio` RSS max | 1,757 MB |
| guards triggered | disk guard once, before the run (the projection on `synthea-s3-rest`); **none during `synthea-s3-obs-head`** |
