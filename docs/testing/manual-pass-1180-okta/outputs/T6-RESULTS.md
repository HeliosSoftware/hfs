# T6 results - Create a ViewDefinition and examine its output (matrix section 10)

Window: 2026-10-08 ~03:05Z-03:25Z, server http://localhost:8080 (health 200 before/after, pid 1378446 unchanged), Playwright via `~/tmp/pw/t6/*.mjs`. Timings are not benchmark values (shared server). Results card ran in 0-4 ms (50 rows) for every definition.

IDs: VD (patient_demographics) = 01a11980-7f7b-7431-88f3-27be80c40910; VD2 (observation_flat) = 01a11981-3416-74b0-9b06-2e3d5503c252; SQL View left behind by me: t6_pd_ids = 01a11982-619f-7f10-9898-363bfd111698 (reads `pd`, `SELECT id FROM pd`). Marker `~/tmp/okta-pass/work/t6-vds-saved` touched after both VDs verified through the API (versionId 1, never touched again; confirmed unchanged at end).

| Step | Method | Expected | Actual | Result |
|---|---|---|---|---|
| 10.1.1-3 | UI | Create New, starter, paste, 50 rows, columns, No issues. | Title "New View Definition"; "50 rows · 3 ms"; headers ID,GENDER,BIRTH_DATE,FAMILY,CITY; chip "No issues." | PASS |
| 10.1.4 lint | UI | squiggle + gutter, hover `Unknown key "colum"`, Ctrl+Shift+M panel, Ctrl+. fix, chip No issues. | Squiggle, 2 gutter markers, hover shows `Unknown key "colum"` with Rename/Remove actions; Ctrl+Shift+M opens panel; Ctrl+. (panel closed) opens the panel (2 fixes at cursor, so it does not apply directly - see deviations); clicking Rename to "column" in panel -> "No issues." | PASS (with deviation) |
| 10.1.5 completion | UI | `getRes`+Ctrl+Space offers getResourceKey(); key completion with required tags | List shows `getResourceKey / getResourceKey()`, Enter accepts. Empty object position lists structural keys.| string · required`); at top level (status deleted) `status` is listed as plain `string` | PASS (deviation noted) |
| 10.1.6 save | UI+API | `?vd=<id>&saved=1`, Saved., rail + Recently used | URL ok, "Saved.", rail shows patient_demographics/Patient in both lists; API: 1 hit, content as pasted | PASS |
| 10.1.7 cross-check | UI+API | PID row `female · 2015-12-29 · Parker433 · Everett` | PID row present among the 50: female, 2015-12-29, Parker433, Everett; `GET /Patient?_id=` for PID, 01a1168e-... (Larkin917, female, 1996-04-19, Millis) and 44219cdc-... (Mante251, 1987-06-07, Arlington) all match | PASS |
| 10.2 | UI+API | 50 Observation rows, codes like 8302-2, saved | 50 rows; first rows codes 4548-4, 8302-2, 72514-3...; chip No issues.; saved, API 1 hit | PASS |
| 10.3 filter | UI | `patient` leaves only patient_demographics | Typing alone does nothing: the filter is a GET form (`?filter=`) applied on Enter. With Enter the rail shows patient_demographics (+ the two copies, which already existed when this was checked; the before-copies check was typed without Enter and showed all rows) | PASS (deviation) |
| 10.3 duplicate | UI+API | `_copy` selected with own canonical; 2nd duplicate `_copy_2`; original unchanged | `patient_demographics_copy` url `.../patient_demographics_copy`; `_copy_2` url `.../patient_demographics_copy_2`; original url unchanged. Duplicating the url-less SQL View `t6_pd_ids` gave `t6_pd_ids_copy` with no url | PASS |
| 10.3 SQL View dependency | UI | Reads from names+links the original; copy where=false -> 0 rows; SQL View still original rows | Reads from: `pd` ViewDefinition `patient_demographics` -> `/ui/sql/view-definitions?vd=<VD>`; copy saved with where `false` -> "0 rows · 0 ms"; SQL View still returns the 50 rows (Patient ids) | PASS |
| 10.3 SQL Query / Library duplicate with canonical | - | SQL Query keeps reading original SQL View after copy edit; Library duplicate gets own canonical | Not exercised (my SQL View has no canonical url and creating a SQL Query was out of the effort budget) | NOT VERIFIED |
| 10.3 delete | UI | confirm text, copy disappears | In-page `<dialog>` (no native dialog events): `Delete view definition "patient_demographics_copy"? This cannot be undone.` Cancel/Confirm; both copies removed from rail. SQL View copy `t6_pd_ids_copy` also deleted (`Delete "t6_pd_ids_copy"? ...`) | PASS |
| 10.3 negative | UI | lint flags Nope, Results "Could not run the view. ...", last successful run, Save prompt, Cancel | chip "1 issue"; "Could not run the view. $sql-run returned 422 Unprocessable Entity: unknown resource type "Nope""; "last successful run" label; prompt "This view definition still has 1 error. Save it anyway?" - Cancel; no save | PASS (wording differs) |

Originals checked through the API before and after 10.3: both exist, versionId 1, unchanged.

## Deviations from the matrix text
- 10.1.4: the typo yields 2 issues (extra `A select must have at least one of column, select, or unionAll` / select-without-output), not one. Ctrl+. does not apply a fix directly when two fixes (Rename, Remove) are available - per `vd-editor.js` it opens the lint panel; the Rename was applied by clicking it there. The matrix wording "press Ctrl+. and apply the fix" is consistent only in this reading. Keyboard shortcuts all worked with Playwright (no STOP-AND-ASK).
- 10.3: filter needs Enter. Prompt text reads "1 error." not "1 error(s)".
- Results card timings are 0-4 ms, so the "within a second" text appeared immediately.

## Errors/WARN
No browser page errors, no native dialogs/popups/downloads (events empty). No relevant WARN/ERROR in the HFS log for the window.

## Left behind
SQL View `t6_pd_ids` (id above, depends on the original VD) remains; its copy was deleted. It was not deleted because the brief allows deleting copies only. Other agent resources (female_patients, tall_female_patients) are not mine.

## Findings
None blocking. One minor observation above (2 issues instead of 1 for the typo).

## Screenshots (screenshots/T6-*)
T6-10.1-01..15 (create, results, squiggle/hover, lint panel, ctrl-dot, after-fix, completion, saved, saved results, required tags, column keys), T6-10.2-01..03, T6-10.3-01..16 (filter, duplicates, sqlview before/after save, copy where false, SQL view still original, duplicate sqlview, delete confirms, negative), plus T6-probe-* (exploration). Scripts: `~/tmp/pw/t6/`.
