# T5 results: Bulk Data $export from the Export page (matrix section 9)

Environment: HFS pid 1378446 (release, R4, sqlite, Okta session), health 200 before/after every heavy step and at the end,
pid unchanged. Window 2026-10-08 03:09 - 03:36 UTC. Method: Playwright (real UI, scripts in `~/tmp/pw/t5/`) plus API
verification (`/export-status/{job}` manifest, line counts of the files in `~/tmp/okta-pass/work/bulk-exports/default/<job>/`,
curl on pill hrefs). Timings are NOT benchmark values (T4/T6/T7 ran concurrently).
Window instants used: `<T3 start>` = 2026-10-07T14:07:57Z, `<T3 end>` = 2026-10-08T00:55:56Z (just after "Processing finished at ...55.104Z").

## Per-step table

| Step | Method | Expected (matrix, adjusted) | Actual | Result | Duration (kick-off to Complete) |
|---|---|---|---|---|---|
| 9.1 Group fixture | Playwright editor (Edit raw, fill, Edit raw, Save Changes) + curl | Group saved, pill gone | `Group/manual-group` created, curl 200 (lastUpdated 03:09:26.035Z) | PASS | 2.4 s UI |
| 5.1 everything-small | Playwright + API | 6 files; Organization 1000+140 | 6 files: Location 1000+142 (1137+5 T2), Organization 1000+140, Practitioner 1000+140; Organization-0/-1 fetched via pill href: 1140 valid JSON lines | PASS | server ~1.3 s (files written 03:09:56); card said "finished in 4m 33s", see anomaly 1 |
| 5.2 one-patient | Playwright (combobox pick Cari853 Esperanza675 Parker433) + API | Patient 1, Condition 15, Observation 165 (+1 Condition from T8) | 3 files: Patient 1 (id 7d24f7a0..), Condition 16, Observation 165 | PASS | 91 s (card 1m 31s) |
| 5.3 group-active-conditions | Playwright + API | 2 files; Patient 1; Condition active, PID, < 15 | 2 files: Patient 1, Condition 1 (active, subject PID); API `Condition?patient=PID&clinical-status=active` total 1 | PASS | 10 s |
| 5.4 elements-subset | Playwright + API | "1 file"; id, gender, meta, SUBSETTED | **12 files** (Patient-0..11: 11 x 1000 + 705 = 11,705 lines = 11,704 + Larkin); every line keys = resourceType,id,gender,meta; tag SUBSETTED on all | PASS (file count differs: matrix text stale, chunking) | 5 s |
| 5.5 cancel-me | Playwright | In progress -> Cancel -> Cancelled | In progress at once (t+99 ms, "Waiting for the first status report..."), Cancel click -> chip Cancelled in 0.5 s; no output directory; HFS 200 and same pid, RSS 3.9 GB, 0 % CPU afterwards; then Delete -> warning text exact -> Delete export -> card gone | PASS | n/a |
| 5.6 negative | Playwright JS and no-JS | "Enter a name for this export." | JS: inline error, URL stays /new; no-JS: server re-rendered page shows the same text (POST /ui/bulk-export) | PASS | |
| 5.7 since-import | Playwright + API | Since line; Patient 11,704 | window "Since 2026-10-07T14:07:57Z"; **12 files**, 11,704 lines (Larkin absent); request `_since=...` | PASS (files: matrix "1 file" stale) | 5 s |
| 5.8 until-import | Playwright + API | Until line; 1 line Larkin917 | "Until 2026-10-07T14:07:57Z"; 1 file, 1 line Nicky270 Ann985 Larkin917 | PASS | 5 s |
| 5.9 since-until | Playwright + API | since -> until; 11,704 | "2026-10-07T14:07:57Z -> 2026-10-08T00:55:56Z"; 12 files, 11,704 lines | PASS | 5 s |
| 5.10 since-fixtures | Playwright + API | RiskAssessment, ValueSet, Group 1 line each, no Patient | 3 files Group, RiskAssessment (manual-risk), ValueSet (manual-test-vs), 1 line each; no Patient pill | PASS | 5 s |
| 5.11 last-day | Playwright + API | 1,140 (T2/T3 within last day) | window "Since 2026-10-07T03:25:04Z" (24 h before kick-off); Organization 1000+140 = 1,140. Extra: API kick-off with a window after T3 end returned `"output":[]` well-formed | PASS | 5 s |
| 5.12 negative | Playwright | "Enter a valid FHIR instant..." under field; All time unblocks | `yesterday` rejected with the exact text; switching to All time: field disabled, error hidden, submit proceeded (created `neg-since`, Organization 1140, deleted) | PASS | |
| 5.13 bad-group | Playwright | Failed at once, names missing Group; Retry fails identically; Delete | card Failed after 137 ms: "kick-off answered 404: Could not find the resource 'Group/does-not-exist'."; Retry: same card, same text; Delete -> warning -> Delete export: card gone | PASS | 0.14 s |
| 9.2 Clear / no-JS / validation (preamble) | Playwright, separate `javaScriptEnabled:false` context | Clear unchecks All Resources and all types, enables types, other values and heading unchanged; with errors | Verified: Clear with All Resources -> all_types off, 0 checked, 147/147 enabled, name/scope/group/elements/filter/preset/custom/until and heading unchanged; with 3 types selected -> 0 checked; selecting a type afterwards works (1 checked). No-JS: Clear hidden; impossible Custom date 2026-02-31T00:00:00Z rejected server side with the error under the field, all fields and 2 checked types kept; rejected HTML loaded with JS on (route-fulfilled copy of the response): error stays after Clear | PASS | |
| 9.2 empty selection = no filter | Playwright no-JS, Patients scope textarea PID, All Resources unchecked, no type | no resource-type filter | request `Patient/$export` without `_type`; 19 types exported (Condition 16, Encounter 36, Observation 165, ...); card Complete | PASS | 5 m 20 s |
| Download All Resources ZIP | curl on `/ui/bulk-export/active/<id>/download` with session cookie (+ click event) | ZIP of six NDJSON files | 200 `application/zip`, `bulk-export.zip`, 3,198,174 bytes, 0.3 s, six files Location-0001/0002, Organization-0001/0002, Practitioner-0001/0002 with 1000/142/1000/140/1000/140 lines | PASS via curl, browser click = STOP-AND-ASK | |
| 9.5 S3 output backend | n/a | | MinIO out of scope by owner decision | N/A | |

## Deviations, anomalies

1. **"finished in ..." can differ from the server time.** On 5.1 the card said "finished in 4m 33s" although the files were written ~1.3 s after kick-off (manifest transactionTime 03:09:55.03, files 03:09:56.2). The UI card only learns the end when it polls (every 5 s while the list page is open); my first script was not on the list page, so the timer ran until the first poll. For all later rows (list page open) the card showed 0m 05s for sub-second exports, i.e. the figure is quantised by the 5 s poll. Not a defect of the export, noted for the documentation.
2. **Matrix file counts are stale for Patient:** 5.4, 5.7 and 5.9 say "1 file" but the output is chunked at 1,000 resources per file (12 files, 11,704/11,705 lines). Counts match.
3. 5.2 expected values: Condition 16 (matrix 15 + the T8/T9 Condition), Observation 165. First two attempts used the wrong patient because of my own operator mistake (the first option in the Parker433 list was picked, a different patient: 15 Conditions / 156 Observations; verified by comparing ids with the API). Both exports were deleted. Not a product problem.
4. 5.3 also ran with Since "All time" explicitly (preset empty); matrix text matches.
5. Kick-off POST redirect to the list took ~70-90 ms every time (kick-off is asynchronous).
6. Observed duration for Patient-compartment exports: ~90 s for 165 Observations (one Observation query dominates; the whole 19-type compartment export 5m20s). Not a benchmark.
7. The `Cancel` of cancel-me: the job was cancelled before any file was written (no output directory), so the "heavy export" never loaded the server.
8. 5.5 note: HFS list showed "1 running" for a few seconds after neg-since was started, until the card polled (same 5 s quantisation).

## Errors / WARN in HFS log during my window
None related to bulk export. The only WARN lines in the window (03:21 - 03:24) are T7 SQL-export failures (`observation_flat` dependency exceeds 1,000,000 rows). No ERROR lines from my steps.

## Findings
None that look like product defects. Documentation points: stale "1 file" statements in 5.4/5.7/5.9 (and 5.1 chunk count is right), "finished in" quantised by polling (anomaly 1), matrix `Complete . N files` wording.

## STOP-AND-ASK
- Step: 9.2 "Download All Resources" (matrix lines ~880-881) and the download pills. Element: `a.btn[aria-label="Download all resources from everything-small"]` and `.job-card__files a` (pills, `download` attribute). Event seen: `{"kind":"download","name":"bulk-export.zip"}` and `{"kind":"download","name":"Location-0"}` (browser context has acceptDownloads=false, so the file was not saved by the browser).
  Options: (a) done: fetched the same hrefs with curl as supplementary proof (ZIP: six NDJSON files with exact line counts; pills: Organization-0/-1 1000+140 lines of valid JSON); it does NOT prove the browser's save dialog / file name handling, only that the endpoint serves the file and Content-Disposition `attachment; filename="bulk-export.zip"`. (b) owner clicks it manually and sends a screenshot of the saved file. (c) mark N/A.
  Effect on pass criterion "the ZIP download works": satisfied by curl (option a); browser-side save not evidenced.

## Clean-up
All exports I created were deleted through the card Delete action (final list: "0 exports"); output directory empty. Sizes before delete: everything-small 558 KB, one-patient 27 KB, group-active 4.5 KB, elements-subset 561 KB, since-import / since-until 5.0 MB each, until-import 3 KB, since-fixtures few KB, last-day / neg-since ~1.3 MB each, empty-types-nojs (19 types) small. Total ~12 MB. The API job from the empty-window check was deleted via `DELETE /export-status/<id>` (202).

## Output files
- Evidence: `outputs/T5-5.1-dl-Organization-*.hdr`, `outputs/T5-5.2-api-*.{hdr,body}`, `outputs/T5-5.3-api-active-cond.*`, `outputs/T5-5.11-api-empty-window-*.{hdr,body}`; timings in `metrics/timings.tsv`; steps in `metrics/steps.tsv`.
- Scripts: `~/tmp/pw/t5/` (`run.mjs`, `exp.mjs`, `clear.mjs`, `neg.mjs`, `cancel.mjs`, `nojs2.mjs`, `del*.mjs`, `verify.sh`).
- Screenshots (66, `screenshots/T5-*`): 9.1 group before/after save; per-row `T5-9.2-5.x-02-form-filled`, `-03-card-after-start`, `-04-card-final` (5.1 to 5.4, 9.3 rows 5.7 to 5.11, 9.4 5.13 incl. before/after retry and delete), `T5-9.2-5.2-01-patient-search`, `T5-9.2-5.5-02-in-progress / -03-cancelled / delete-04..05`, `T5-9.2-5.6-01-empty-name`, `T5-9.3-5.12-01..03`, `T5-9.2-clear-01..02`, `T5-9.2-nojs-01..05`, `T5-9.2-nojs-empty-types-01..03`, `T5-final-01/02`, plus `T5-9.2-5.2-wrongpatient*` (wrong-patient attempts, deleted).
