# T7 results — SQL export with a ViewDefinition, a SQL query and a SQL view (matrix section 11)

Window: 2026-10-08 03:14 – 03:41 UTC. Server pid 1378446 (unchanged, `/health` 200 before/after every heavy step).
Backend: plain sqlite, Okta auth, R4 release build. Timings are NOT benchmark values (other tracks used the same server).
Methods: **UI** = Playwright (scripts in `~/tmp/pw/T7/`), **API** = Python/curl against `$sql-run` / `$sql-export` (`~/tmp/t7/`).
Output files were fetched through the pill `href` (`/export/<job>/shard-N.<ext>`), never through a browser download.
Pre-condition: T6 marker present; `ViewDefinition?name:exact=` gave exactly 1 each: `patient_demographics` (01a11980-7f7b-7431-88f3-27be80c40910), `observation_flat` (01a11981-3416-74b0-9b06-2e3d5503c252). (Plain `name=patient_demographics` is a prefix match and also returned T6's copies.)
Ids: QV `female_patients` = 01a11982-06f1-77e0-ba34-6d9886acf678; QQ `tall_female_patients` = 01a11982-a894-71a0-8a15-39d19832cabc; PID = 7d24f7a0-6f2e-ce3b-5568-db7b14695583.
Expected counts: Patient total is 11,705 (corpus 11,704 + Larkin), so the matrix numbers (11,705 / 5,814) apply unchanged. T3 start = 2026-10-07T14:07:57.580Z, T3 end = 2026-10-08T00:55:55.104Z (submission page).

## Per-step table

| Step | Method | Expected | Actual | Result | Duration |
|---|---|---|---|---|---|
| 11.1 SQL View | UI | saved, 50 preview rows `id, birth_date, city` | Saved., rail `female_patients · active`, preview `50 rows · 436 ms` | PASS | save 3.6 s |
| 11.2 SQL Query create | UI | unbound error; 150 shows rows; save; type chip SQL Query/active | saved (chips SQL Query / active). **Unbound `:min_height` shows "Waiting for a value for :min_height — the results below are from the last successful run."** not an error; page now has a per-run Parameters box. With 150 (box or literal) preview = **"Could not run the query. $sql-run returned 422 Unprocessable Entity: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit…"** after 20 s, so no rows | PASS (save) / DEVIATION (preview) | 20 s preview |
| 11.2 negative | UI | rejected | *The Library's SQL on FHIR type must be "sql-query" to save it here.* exact | PASS | |
| `$sql-run` | API | rows | view/VD 200 (Binary envelope, `_limit=5`); query `min_height=150` **422** OperationOutcome code `processing`, same row-cap text, 18.3 s | PASS (VD, view) / known failure (query) | 0.0 s / 0.7 s / 18.3 s |
| 7.a vd-ndjson | UI + API | 1 file, 11,705 lines | Complete, 1 file, 1,506,439 B, 11,705 lines (UI card "finished in 0m 05s", API ~3 s poll granularity) | PASS | 5 s UI |
| 7.b query-csv | UI + API | 1 file header `id,city,height` | **Failed**: HTTP **422** on `/export/<id>/result` (not 500): "dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit…"; detail: "The export stopped on subject tall_female_patients: …". Spot checks not possible | FAIL (known) | 18 s |
| 7.c view-parquet | UI + API | 5,814 rows, `id, birth_date, city` | 5,814 rows; id/birth_date/city all string; 268,741 B | PASS | 3 s |
| 7.d all-three-json | UI + API | 3 files, each an array | **Failed** same 422 (whole job fails, no partial files). Supplementary (API): inline non-persisted sql-query Library via `subjectResource` depending on `female_patients` only + the VD + the view, JSON → 3 files named after subjects, each an array (11,705 / 38 / 5,814) | FAIL (known) | 18 s |
| 7.e cancel-me | UI | In progress → Cancelled | card "In progress · Writing observation_flat", Cancel → "Cancelled · cancelled at 03:30 UTC" ~1.5 s | PASS | |
| 7.f negatives | UI | two messages | "Select at least one subject." ; query ticked + empty value: field turns red, hint "1 value missing", browser-native tooltip "Please fill out this field." blocks submit; "This value is required." is rendered (visible) only with JS disabled (and stored as `data-msg-param-required`) | PASS (see deviation) | |
| 7.g one-patient | UI + API | 1 line | PATIENTS `Patient/PID` on Job card; 1 line `female,2015-12-29,Parker433,Everett` | PASS | <1 s |
| 7.h one-group | UI + API | one-element array | GROUPS `Group/manual-group`; `[{PID…}]` | PASS | <1 s |
| 7.i since-import | UI + API | 11,704 lines | SINCE shown, 11,704 lines | PASS | 4 s |
| 7.j since-nothing | UI + API | Complete, 0 rows | Complete, "0 files" ("The job produced no output files."); manifest has no `output` | PASS | <1 s |
| 7.k tracked-csv-noheader | UI + API | no header, 5,814 lines, chips | Format "CSV · no header row", TRACKING ID release-check-01, 5,814 lines, first line is a data row | PASS | 4 s |
| 7.l negatives | UI (JS and no-JS) + API | three messages | With JS only the first failing field is reported when several are wrong (since) and the patients field is a typeahead (textarea disabled); 201-char tracking alone → "Tracking id must be 200 characters or fewer."; with JS disabled all three exact messages appear at once ("Enter only valid logical Patient IDs, separated by commas or new lines.") and all typed values are kept | PASS | |
| 7.m vd-csv | UI + API | header + 11,705 lines, PID line | 11,706 lines, header `id,gender,birth_date,family,city`, PID line `…,female,2015-12-29,Parker433,Everett`; Format "CSV · with header row" | PASS | 3 s |
| 7.n vd-parquet | UI + API | 11,705 rows, string birth_date | 11,705 rows; all five columns string | PASS | 3 s |
| 7.o query-ndjson | UI + API | = 7.b | Failed 422 row cap | FAIL (known) | 18 s |
| 7.p query-parquet | UI + API | = 7.o | Failed 422 row cap | FAIL (known) | 18 s |
| 7.q view-ndjson | UI + API | 5,814 lines, no gender | 5,814 lines, keys exactly id/birth_date/city | PASS | 3 s |
| Run again / Remove | UI | new card, old stays; remove copy | new card (Complete ~3 s), old stays, Remove on copy leaves 1 | PASS | |
| Copy job id | UI | button shows Copied | label became "Copied"; clipboard content **not verified** | STOP-AND-ASK (partial) | |
| Restart with job in progress | — | Cancelled · server no longer knows | needs HFS restart | **DEFERRED to lead** | |
| 11.5 subjects table | UI | Queries/All, filter, Select all, hint | all behave; hint reads "2 of **5** selected" (5th subject = T6's draft SQL view `t6_pd_ids`); hidden rows stay checked | PASS | |
| 11.5 both-vd (ticked pd + obs, Start Export submits both) | UI | submits both | Complete in **2 m 20 s** (manifest 135 s), 17 files: observation_flat 16 shards = **7,699,987 lines (= Observation total)**, ~1.30 GB total; pd 11,705 lines | PASS | 140 s |
| 11.5 "Export as files" button | UI | button appears, opens `?subject=Library/QQ` | **Not shown**: preview fails (422 row cap), so the button never appears. Direct URL `/ui/sql/export/new?subject=Library/QQ` pre-checks the query and shows its `:min_height` field. Unsaved edit later confirmed discarded (stored SQL still `:min_height`) | FAIL (known cause) / URL part PASS | |
| 11.6.1 broken_query | UI | "Could not run the query. …", save | saved; **preview text "Unknown table table_that_does_not_exist — line 1. Declare it under Reads from or fix the name. Your SQL is unchanged; …"** | PASS / DEVIATION | |
| 11.6.2 export broken | UI | Failed, "The export stopped on subject broken_query: …" | exact, with "SQLite error: no such table: table_that_does_not_exist." | PASS | 3 s |
| 11.6.3 Retry / Copy / Remove | UI | new card same failure, original untouched | yes; both removed (0 left) | PASS | |
| 11.6.4 delete broken_query | UI | Delete → confirm | in-page Confirm dialog (no native dialog), API search empty afterwards | PASS | |
| 11.7 S3 sink | — | — | MinIO out of scope by owner decision | **N/A** | |

## Deviations from the matrix text
1. 11.2: unbound parameter preview shows "Waiting for a value for :min_height…" instead of an error; the UI now has a Parameters box for preview values.
2. 11.2/11.5: the preview and the `Export as files` button cannot be shown with this corpus (observation_flat > 1,000,000 rows).
3. 11.6: preview message for a bad table is "Unknown table … Declare it under Reads from or fix the name." (not "Could not run the query.").
4. 11.5: subject count is 5 (T6's `t6_pd_ids`), hint "of 5".
5. 7.f: "This value is required." is visible only in the no-JS server render; with JS the browser's native tooltip appears.
6. 7.l: with JS only one message at a time when several fields are bad; the patient field is a typeahead (groups need the exact id `manual-group`).
7. Failed export result status is **422** (matrix/earlier note said 500); the card text reads "the result endpoint returned 422 Unprocessable Entity".
8. Beforeunload: leaving the SQL Queries page with an unsaved edit raises a native `beforeunload` dialog (Playwright `dialog` event) that aborted `goto`; not used further.
9. Playwright could not satisfy "Open the file download pill": pills not clicked (download = STOP-AND-ASK trigger); hrefs fetched with an authenticated GET instead (supplementary proof; bytes equal to the API/curl downloads).

## Errors / WARN in the HFS log (my window)
Only expected WARNs: `SQL query dependency failed to materialize … produced more than 1000000 rows` and `export job failed … exceeds 1000000-row limit` (for jobs 1baf54c8, 03edbac9, a3ede07e, 97c4aa91, 1b3f150b, 5036996d, 4fa5a561, 50e2afff), and for the intentional `broken_query` (`no such table: table_that_does_not_exist`, `status=422`). Nothing else; health always 200, pid unchanged.

## Findings
- **F1 (known): row cap makes the whole SQL Query path unusable on the full corpus.** `$sql-run` and `$sql-export` of `tall_female_patients` fail with **422** (OperationOutcome `processing`): "dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: … Narrow the dependency with a ViewDefinition 'where', or raise HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD." 12–20 s each. Consequence: 7.b/7.o/7.p/7.d and the preview/"Export as files" parts of 11.2/11.5 cannot pass. Cosmetic: the UI detail text ends with ".." (double period). With a non-persisted query that avoids `obs` the query path works in ndjson/csv/parquet/json (38 rows; schema `id,city,height(double)`).
- **F2: one failing subject fails the whole multi-subject job and no partial files are kept** (7.d: VD and view outputs are discarded). Not necessarily a defect, but the matrix expects 3 files.

## STOP-AND-ASK
- **Copy job id** (11.3, 11.6.3): the click shows the "Copied" label but the clipboard cannot be read. (a) Agent workaround: label + `data-copy-job-id` attribute equals the job id; does NOT prove the clipboard content. (b) Owner pastes it somewhere and sends a screenshot. (c) N/A. Effect: only the "Copied" label is evidenced.
- **Pill downloads**: not clicked; see deviation 9 (options: owner clicks one pill and confirms the file opens, or accept the href GET as proof).

## DEFERRED to the lead
- 11.3 "Restart HFS while one job is In progress": card must resolve to "Cancelled · the server no longer knows this job" and complete cards keep their downloads (jobs are held in memory: `helios_rest::export::in_memory`). Needs a restart; none was done. Existing complete cards (e.g. `vd-ndjson`, `view-parquet`, `vd-csv`) are available to check "downloads survive"; an In-progress job can be created with `observation_flat` NDJSON (about 2 min), or more slowly with the query.

## Files left in the system
- UI list still holds my cards (vd-ndjson, vd-csv, vd-parquet, view-*, tracked-csv-noheader, one-*, since-*, query-* (Failed), all-three-json (Failed), cancel-me). Small outputs remain in `~/tmp/okta-pass/work/sql-exports/` (about 11 MB). Resources I created: Library `female_patients` (QV), Library `tall_female_patients` (QQ); `broken_query` was deleted. Not deleted: QV and QQ (needed by other steps; delete in teardown).

## Screenshots (`$EVID/screenshots/`)
`T7-11.1-01..03`, `T7-11.2-01..08` (06/07 have two debug variants), `T7-11.3-01,05..08`, `T7-11.5-01..04`, `T7-11.5b-01..03`, `T7-11.6-01..10`, `T7-7a..7q-*` (form-filled, card, detail per job), `T7-7e-01..03`, `T7-7f-01..06`, `T7-7g/7h typeahead`, `T7-7l-03..05`.
## Output files
`$EVID/outputs/T7-files/` (job summaries, manifests, small outputs, `$sql-run` bodies, heavy-run log); big files were counted and deleted; working copies in `~/tmp/okta-pass/work/t7/`. Step rows are in `metrics/steps.tsv` (track T7).
