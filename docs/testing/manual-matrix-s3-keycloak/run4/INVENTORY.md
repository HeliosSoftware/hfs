# Run 4 — #1178 `s3` (MinIO) + Keycloak retest, `main` `5f01d3fd0`

Collected 2026-10-05 14:58Z → 19:48Z. `hfs 0.2.4 (git 5f01d3fd0)`, `cargo build --workspace --all-features --release` (rustc 1.99.0, `CARGO_BUILD_JOBS=1`, 3 h 02 m, peak RSS 16.5 GB; `outputs/build-tail.txt`). Includes #1725 (#1715) and #1729 (#1722). Same configuration as run 3: auth on, Keycloak 26.1.5 on :8180, interactive login (`hfs-web`), `HFS_LOG_LEVEL=info`. MinIO on the run-3 data directory `data-1178c`; **the corpus was not re-imported**.

## Deviations (`outputs/deviations.txt`)
- Run-3 data reused (9,650,469 half-corpus resources), plus an **Observation test subset**: every `8302-2` Observation plus every Observation of `PID`, 175,502 lines (`outputs/obs-subset-stats.txt`, `outputs/obs-subset-extract.sh`, `outputs/manifest-obs.json`), from the local corpus copy, served by the Node HTTP/1.1 server (7.1 byte check: `outputs/corpus-served-bytes.txt`).
- **T3 verified on the reduced manifest.** The run-3 receipts cannot be re-requested because that submission was cancelled.
- Realm `accessTokenLifespan` 3600 s for the run.
- The Import detail page was not kept open during T3. It polls only while open (by design), so its *Processing finished at* is the first view after completion. Times come from the recipient status (15 s samples) and the bucket.
- The outbound token expired at 19:01:31Z, and the New SQL Export page then answered *The library list could not be loaded … 401* under a valid session (#1671). HFS was restarted with a fresh token, and the T7 page flows were rerun (`outputs/t7-page-flows-expired-token.out` keeps the failed attempt).

## Integrity after the 2026-10-05 OOM (`outputs/bucket-counts.txt`, `outputs/t75-integrity.out`)
Every corpus type in the bucket equals its `manifest-half.json` count, and no id is without `current.json`. The 41 extra objects are the pass's own: A2 Patient 1; T2 Organization 4, Location 5, Practitioner 4, PractitionerRole 4; T8/T9 Encounter 15, Condition 1, Basic 1, Subscription 1; T6/T7 ViewDefinition 2, Library 3. `PID` reads back v1 with the expected values.

## T3 — ✅ (#1715 fixed) — `outputs/t3-timing.txt`
| | |
|---|---|
| created on the Import page | 18:02:51.406Z |
| last resource / result / change object | 18:13:04.47Z |
| manifest `completed` | 18:13:08.47Z |
| recipient 200 (`transactionTime`) | 18:13:11.913Z |
| **write phase** | **10 m 13 s** (~286 res/s) |
| **receipt step** | **~4 s** |

0 errors. The receipt file has 175,502 lines = N, 0 missing and 0 extra against the subset ids (`outputs/t3-receipts-check.txt`). Detail page: **Completed · Output files 1 · Error files 0** (`shots/t3-submission-completed.png`).

## Other steps
- **7.5** — `PID` v1; all 165 of `PID`'s Observations 200 by id; a height Observation reads `8302-2 72 cm`; searches 501.
- **T6** ✅ — `$sql-run` json / csv / ndjson / parquet 200, 11,705 rows, five columns, `PID` row; `patient=PID` one row; `observation_flat` 175,502 rows (174 s in-process scan), `PID` 165; no token 401.
- **T7** ✅ — every subject kind in every format (`outputs/t7-results.txt`, `outputs/t7-query-checks.txt`). **7.b / 7.o: 4,944 rows, all `height > 150` (min 150.1), the same ids in CSV and NDJSON**; Parquet 4,944 rows (`id string, city string, height double`), JSON 4,944; an independent count from the corpus files gives 4,944; two spot-checked ids are female. Page flows (`outputs/t7-page-flows.out`): 7.f / 7.l exact texts; kick-off Complete; page `query-csv` *CSV · with header row*, *:min_height = 150*, 4,945 lines; **7.e Cancel on `observation_flat`: In progress → Cancelled**; 11.6 `broken_query` Failed with the SQLite error, Retry a new card; Run again; Remove → 404; restart with a job in progress → *Cancelled · the server no longer knows this job*, the complete card keeps its download (`outputs/t7-restart-t9-5.out`).
- **#1722 on s3** — the rails filter: a matching filter narrows each rail (View Definitions, SQL Queries, SQL Views), and a non-matching one empties it with *No matches for “zzz”.* (`outputs/t1722-rails.out`). **But the main pane keeps the remembered definition, and "Clear the filter" never appears → #1780** (`outputs/t1722-main-pane.out`, `shots/t1722-vd-no-match.png`).
- **T9** ✅ (`outputs/t9.out`) — +3 → Delivered 3 · 100.0 % first try; receiver down → `Max retries exhausted` → **Error · Failing 1 · Sent 6 · Fail streak 3**, `status=error` v6; reactivation (PUT `status=requested`) → active v8 with the backport profile, +3 delivered; restart → `rehydrated subscriptions=1 failed=0`, 0 *Failed to persist*, +3 delivered; disabled engine → notice, sidebar entry kept; plain GET shows the same figures.
- **T4 / T5** N/A ✅ — 501 (`outputs/t4-t5.out`).

## Memory (`outputs/perf.log`, 292 one-minute samples; `outputs/rss-summary.txt`)
`hfs` RSS max 817 MB (18:49Z, during the in-process SQL Query exports; 66 MB during T3); `minio` max 1,219 MB; build max 16.2 GB with 6.7 GB still available. No OOM during the run.

## Issues
#1780 (new). Reproduced and still open: #1671.
