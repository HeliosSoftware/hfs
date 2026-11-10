# Inventory: manual pass #939 `pg-es`

Everything here was produced by the run described in `RESULTS.md`. Sizes are approximate.

| Path | What it is |
|---|---|
| `RESULTS.md` | Result, deviations, findings with their classification, every row |
| `shots/` | 324 screenshots, 1280x800, named `Tn-<row>-<what>-NN.png` (`-rerun` = repeated row) |
| `outputs/` | API responses, manifests, exports and counts saved by the scripts (165 entries); `t3-pg-counts.tsv` and `t3-es-counts.tsv` are the per-type counts of T3 |
| `samples/` | one small download per export kind (the 84 MB JSON exports of 7.big-json are not committed) |
| `logs/` | HFS server log (`hfs-pg-es.log`), per-step console output, run progress, build tail |
| `metrics/` | `steps.tsv` (every row with its time), `shots.tsv`, T3 progress and start/end, corpus byte check and expected counts |
| `scripts/` | Playwright and API scripts (`lib.mjs`, `steps/`), the server start and restart scripts |

## Screenshots by step

- **T1**: 3 files
- **T2**: 24 files
- **T3**: 8 files
- **T4**: 107 files
- **T5**: 55 files
- **T6**: 21 files
- **T7**: 76 files
- **T8**: 12 files
- **T9**: 18 files

## Checklist of the evidence the issue asked for

- [x] `/ui` after import: shots/T3-7.5-ui-dashboard-after-import-01.png
- [x] Completed submission detail: shots/T3-7.3-submission-detail-final-01.png
- [x] `/ui/sql/export` with a completed card: T7 `7.final-sql-exports-list` and the `7.a-*` shots
- [x] `/ui/subscriptions` after T8/T9: T9 shots (`13.*`)
- [x] T4 manifest and T6 result manifest: outputs/t5-manifest-*.json and the t7 manifests
- [x] build log tail, server log, T2/T3 timing: logs/build-tail.log, logs/hfs-pg-es.log, RESULTS.md
