# Evidence bundle - #937 `sqlite-es` row, matrix section 14

Collected 2026-09-18 15:21 from run 2 (HFS on :18085, Elasticsearch on :19200).

## T3 elapsed time

Taken from the submission detail page, not from a stopwatch:

- submission `00551711-0025-4912-8fc8-4693840cf05f` ("T3 run2 mmap 30GB")
- created  `2026-09-17T10:38:50.287Z`
- finished `2026-09-17T15:15:37.892Z`
- **elapsed 4h 36m 47s (16608 s, 4.61 h)**
- 24 output files, 0 error files, status Completed

## Screenshots

All nine were captured on 2026-09-18 at 15:20 local, after the full T0-T9 pass and
after the T9 rehydration restart. `01-t3-submission-completed.png` is a historical
detail page, so it shows the import itself; the rest show end-of-pass state.

- [x] `01-t3-submission-completed.png` - T3 submission detail, Completed - status card + full submission log
- [x] `02-sql-exports-list.png` - SQL Exports list with completed cards and their `finished in` times
- [x] `03-ui-dashboard.png` - `/ui` **at the end of the pass**, not immediately
  after T3: the counts include the resources created in T2, T4, T5 and T8
  (Patient reads 11,705, not the corpus's 11,704)
- [x] `04-subscriptions.png` - `/ui/subscriptions` after T9
- [x] `05-bulk-exports-list.png` - Bulk Data exports list (T5)
- [x] `06-view-definitions.png` - ViewDefinitions (T6)
- [x] `07-sql-queries.png` - SQL Queries (T7)
- [x] `08-bulk-import-list.png` - Submissions list
- [x] `09-status.png` - `/ui/status`

## Server logs

- `hfs-sqlite-es-T3-IMPORT.log` - 170 lines - **the T3 import itself.** Covers
  2026-09-17T10:36:59Z onward and holds the line the timing rests on:
  `15:14:53.136Z bulk-submit indexed every resource during ingest; no deferred
  reindex`. Zero `search index queue is full` and zero `reported unindexed`.
- `t3-import-monitor.log` - 17 lines - the monitor that sampled the import.
- `hfs-sqlite-es-post-T3-session.log` - 2452 lines - the *later* session, which ran
  T4 through T9. It does **not** contain the import; an earlier revision of this
  bundle attached it as if it did.
- `hfs-sqlite-es-run2-after-restart.log` - 48 lines - the restart that exercised
  subscription rehydration (T9).

## Export samples

`$sql-export` output as written by the server. The full pass produced 9 output
files (two duplicate multi-MB NDJSON pairs among them); only the smallest sample
of each format actually produced is committed here, to keep this bundle small:

- `samples/sqlexport-1db0356c-shard-0.parquet` - 268560 bytes (smallest of 3 parquet outputs; `sqlexport-36e6fb30-shard-0.parquet` is byte-identical and `sqlexport-89a13fb6-shard-0.parquet`, 560419 bytes, was dropped)
- `samples/sqlexport-91bff4fa-shard-0.ndjson` - 127 bytes (smallest of 5 NDJSON outputs; two multi-MB pairs, ~1.5 MB each, were dropped)
- `samples/sqlexport-a4be799c-shard-0.csv` - 862697 bytes (the only CSV output)

**No JSON-format sample is included.** The only JSON export in the pass,
`all-three-json`, failed on the 1,000,000-row source limit
(`HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`) — see finding 9 in the new matrix
section. There is no successful JSON output from this run to sample.

## Resource counts at the end of the pass

| Type | Count |
|---|---:|
| Patient | 11705 |
| Encounter | 827983 |
| Condition | 476456 |
| Observation | 7699987 |
| Procedure | 2177375 |
| Organization | 1136 |
| Practitioner | 1136 |
| PractitionerRole | 1140 |
| Location | 1137 |

## Not captured

- `build.log` - the build output was never redirected to a file and is
  not reconstructible after the fact. The binary's configuration is
  already recorded in the first comment on #1126.
- T2 Per-Action Outcomes screenshot - taken manually during the T2 pass;
  it lives outside this bundle.
