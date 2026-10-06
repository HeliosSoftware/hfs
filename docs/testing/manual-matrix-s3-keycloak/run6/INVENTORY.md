# Run 6 — #1178 `s3` (MinIO) + Keycloak, completing the corpus, `main` `5f01d3fd0`

Collected 2026-10-06 05:25Z → 10:41Z. Same binary and configuration as runs 4–5 (`hfs 0.2.4`, `HFS_LOG_LEVEL=info`, auth on, login `hfs-web`, realm lifespan 300 s), MinIO on `data-1178c`. No build ran.

## 1. Space freed (`outputs/free-space.txt`)
Angela authorized deleting `data-1178b` (run 2). Before deleting, two checks: MinIO served `data-1178c`, and no process had a file descriptor or working directory under `data-1178b`.
- `du -s`: **228,534,612,480 B (212.8 GiB)**.
- Free space went from **157.5 GiB (82.5 % used)** to **356.5 GiB (60.4 % used)**.
- `data-1178c` was untouched.

## 2. `manifest-final.json` (`outputs/build-final.py`, `outputs/build-final.out`)
| | after the run-5 prefix / in the corpus | already in the bucket (excluded) | in the manifest |
|---|---|---|---|
| Observation | 1,199,881 | 26,690 (run-4 subset) | **1,173,191** |
| ExplanationOfBenefit | 1,605,515 | 432,565 (run-5 cancelled job) | **1,172,950** |
| total | | | **2,346,141** |

Nothing was re-imported as version 2. The 7.1 byte and line check passes on all three files (`outputs/corpus-served-bytes.txt`).

## 3. Disk projection (`outputs/disk-projection.txt`)
At 22.9 KB per resource the run needs 50.0 GiB, so the projected end is 306.4 GiB free (66.0 % used). The floor is 135 GiB (15 % of the 900 GiB subvolume quota, standing in for `zpool`, which the container cannot see) → OK.

## 4. Guards (`outputs/guard.sh`, `outputs/guard.log`, 332 samples every 30 s)
- Disk floor 135 GiB.
- MemAvailable below 2 GiB.
- Swap: growth in each of 10 consecutive samples by more than 256 MB, or baseline + 3 GiB.

**None triggered.**

## 5. T3 `synthea-s3-final` — ✅
| | UTC |
|---|---|
| created on the Import page | 07:54:34.4 |
| last resource / result / change object; `success_count 2346141` | 10:37:59.96 |
| manifest `completed` | 10:39:27.17 |
| recipient 200 (`transactionTime`) | 10:39:44.55 |
| **write phase** | **2 h 43 m 25.5 s** (239 res/s) |
| **receipt step** | **1 m 27.2 s** |

0 errors. The receipts are **2,346,141 = the manifest total**, with no wrong type, no missing and no extra ids:
- ExplanationOfBenefit: 1,172,950 receipts.
- Observation: 1,173,191 receipts.

Details in `outputs/t3-receipts-check.txt`. Detail page: **Completed** (`shots/t3-submission-completed.png`).

## 6. Bucket vs the full corpus `manifest.json` (`outputs/bucket-counts.txt`)
Every corpus type matches its `manifest.json` count exactly, including **Observation 7,699,881** and **ExplanationOfBenefit 1,605,515**, and no id is without `current.json`.

**Total: 18,955,919 = 18,955,865 corpus + 54 created by the pass's own steps:**
- Encounter 27 (T8/T9 uploads);
- T2: Organization 4, Location 5, Practitioner 4, PractitionerRole 4;
- Patient 1 (A2) and Condition 1 (T8);
- Basic 1, Subscription 1, Library 4, ViewDefinition 2.

## Resources (`outputs/resources-summary.txt`)
| | |
|---|---|
| final occupancy | **591.3 GiB used, 308.7 GiB free, 65.7 %** |
| MemAvailable minimum | **21,876 MB** |
| swap used max | **1,687 MB** (at the start of the run; flat to slightly down afterwards) |
| `hfs` RSS max | 139 MB |
| `minio` RSS max | 1,186 MB |
| guards triggered | none |
