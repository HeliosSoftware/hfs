# T4 results - one manual search per FHIR search type (matrix 8, 8.1-8.4)

Server http://localhost:8080 (pid 1378446, release build, sqlite, Okta auth), user token. Run 2026-10-08:
fixtures 03:08 UTC, API sweep 03:09-~04:10, then the session was **interrupted for ~5 h and resumed ~08:50 UTC**
(re-checks, 4.11 retry, UI evidence 09:10-09:45). Other tracks (T5-T7) ran concurrently during the first part, and
the abandoned 4.11 query kept the server at ~100% CPU for ~25 min, so ALL timings are not benchmark values.
Expected counts adjusted for extras: Encounter(PID) 24+12 = 36, Condition(PID) 15+1 = 16 (the 12 T8 Encounters have class AMB and no period, the extra Condition has no code/status).

Method: API = `curl` with `_total=accurate` (`$EVID/outputs/t4-api-sweep.tsv`, bodies `T4-<row>.body`); UI = Playwright, first query of every row
typed into the QUERY box on /ui/resources, Enter, header read (`outputs/t4-ui-sweep.tsv`, columns: id, query, header, rows shown, Open-in-New-Tab links found, error, ms, HTTP).

## 8.1 Fixtures
RiskAssessment/manual-risk and ValueSet/manual-test-vs created in the Resource Editor (Edit raw, fill `#editor-source`, Edit raw, Save Changes, pill gone; ~2.4 s each); `GET` both = 200 with expected JSON. Screenshots T4-8.1-01..04. Marker `t4-fixtures-done` touched. (The "No issues." chip was not separately read.)

## Per-row table (API actual per query; UI header of first query)
| 4.1 | PASS | 4.1a=30; 4.1b=30; 4.1c=30; 4.1d=3; 4.1e=3; 4.1f=3; 4.1g=3 | UI first-query: 30 results, 30 results |
| 4.2 | PASS | 4.2a=5814; 4.2b=5891; 4.2c=1; 4.2d=175355; 4.2e=175355 | UI first-query: 5,814 results, 5,891 results |
| 4.3 | PASS | 4.3a=1268; 4.3b=24; 4.3c=11705; 4.3d=0 | UI first-query: 1,268 results, 24 results |
| 4.4 | PASS | 4.4a=1; 4.4b=0; 4.4c=1 | UI first-query: 1 results, 0 results |
| 4.5 | PASS | 4.5a=146589; 4.5b=146589; 4.5c=200 | UI first-query: 146,589 results |
| 4.6 | PASS | 4.6a=165; 4.6b=16; 4.6c=36 | UI first-query: 165 results, 16 results |
| 4.7 | PASS | 4.7a=1; 4.7b=1 | UI first-query: 1 results, 1 results |
| 4.8 | PASS | 4.8=146589 | UI first-query: 146,589 results |
| 4.9 | PASS | 4.9a=1; 4.9b=2 | UI first-query: 1 results, 2 results |
| 4.10 | PASS | 4.10a=165; 4.10b=11939; | UI first-query: 165 results, 11,939 results |
| 4.11 | FAIL | 4.11=HTTP 000: no respons | UI first-query: API only |
| 4.12 | PASS | 4.12a=1; 4.12b=165; 4.12c=11705 | UI first-query: 1 results · 16 included, 165 results |
| 4.13 | PASS | 4.13=83 | UI first-query: 83 results |
| 4.14 | PASS | 4.14=5 | UI first-query: API only |
| 4.15 | PASS | 4.15a=1; 4.15b=19; 4.15c=13; 4.15d=1 | UI first-query: 1 results, 19 results |
| 4.16 | PASS | 4.16a=1; 4.16b=11704 | UI first-query: 1 results, 11,704 results |
| 4.17 | PASS | 4.17a=5; 4.17b=1; 4.17c=38; 4.17d=17 | UI first-query: 5 results |
| 4.18 | PASS | 4.18a=49; 4.18b=49 | UI first-query: 49 results · 4 included, 49 results · 4 included |
| 4.19 | PASS | 4.19a=38; 4.19b=38 | UI first-query: 38 results |
| 4.20 | PASS | 4.20a=2; 4.20b=2; 4.20c=2; 4.20d=2; 4.20e=2 | UI first-query: 2 results, 2 results |
| 4.21 | PASS | 4.21a=6; 4.21b=3; 4.21c=2; 4.21d=3; 4.21e=3; 4.21f=9; 4.21g=4 | UI first-query: 3 results |
| 4.22 | PASS | 4.22a=1; 4.22b=5294; 4.22c=4 | UI first-query: 1 results · 8 included, 5,294 results |
| 4.23 | PASS | 4.23a=1; 4.23b=1; 4.23c=2; 4.23d=38 | UI first-query: 1 results, 38 results |
| 4.24 | PASS | 4.24a=35; 4.24b=1; 4.24c=12 | UI first-query: 1 results |
| 4.25 | PASS | 4.25=36 | UI first-query: 36 results |
| 4.26 | PASS | 4.26a=1; 4.26b=2; 4.26c=25; 4.26d=7; 4.26e=18 | UI first-query: 1 results, 25 results |
| 4.27 | PASS | 4.27a=15; 4.27b=4; 4.27c=8 | UI first-query: 15 results, 4 results |
| 4.28 | PASS | 4.28a=1; 4.28b=1; 4.28c=8; 4.28d=1 | UI first-query: 1 results, 8 results |
| 4.29 | PASS | 4.29a=1; 4.29b=1; 4.29c=1 | UI first-query: 1 results, 1 results · 25 included |
All UI first-query headers equal the API totals (e.g. 4.1a "30 results", 4.2a "5,814 results", 4.6c "36 results · 1 included", 4.12a "1 results · 16 included", 4.27a "15 results"). HTTP 200 for every UI search. Row 4.11 was not run in the UI (API hangs, see Findings).

## Re-classified sweep rows (the original 5 FAIL)
- 4.3d `_lastUpdated=ge2026-10-08` = 0: wrong expectation date; all data was written 2026-10-07 UTC. 4.3c with `ge2026-10-07` = 11,705 PASS.
- 4.20e: script error (double URL-encoding of `%26`); retried properly = 2 PASS (UI also 2).
- 4.22b: 5,294 results, LPID is not on page 1 (50 rows); `Patient?_id=LPID&_has:Condition:patient:code=...706893006` = 1, so LPID has the condition. Matrix expectation "LPID among the rows" is unreachable on a 5,294-row result; wrong expectation.
- 4.10b: 11,939 (> 165 PASS) but rows' `subject` has only `reference`, no `display`; the five rows are one Parker433 patient (886d1faf...). Matrix text about `subject.display` does not match the corpus data.
- 4.11: see Findings (real).

## Deviations / matrix drift
- **No "Open in New Tab" link exists** in the Results card (0 matches in the live DOM for every query, nor in templates/locales). Paging/Sort exist. STOP-AND-ASK below.
- Visual builder (4.14): clicking Patient in the rail pre-adds `_summary=true`; the QUERY box read `GET /Patient?family=Parker433&birthdate=ge2010-01-01&_summary=true&_sort=birthdate` (matrix: no `_summary`). Run showed 5 Parker433 children, birthdates ascending (2013-02-03, ... ). Saved "Parker kids": listed under Patient, Run -> meta `1×`, Recent dropdown lists it under SAVED. Screens T4-8.2-4.14-01..05. Note: this left a saved query "Parker kids" on the server (created by the matrix step).
- 4.12c: Next works (first row changes, `_cursor` request, total stays 11,705); "Previous" appears as the **button** `#query-results-prev`, the small "Previous" tag stays hidden. Sort dropdown Most recent/Oldest re-runs with `_sort=-_lastUpdated`/`_lastUpdated` (30 results each).
- 4.3b/4.24c: the 12 T8 encounters lack `period`, so date results unchanged by them (24, 12).
- 4.2d/4.2e: 175,355 (> 175,000), identical.
- 4.13 on sqlite: 83 results (full-text works without Elasticsearch); "composite log check" N/A.
- 4.27 / 4.21 / 4.22 row-opening (valueQuantity of first/last rows) was verified from the JSON bodies, not by opening modals in the UI (not verified in UI). 4.12b descending dates checked on the JSON. 4.8/4.5 "open a row, value > 150" not opened in the UI.
- 4.5c: lt50 cm = 200 (strict subset of 146,589).
- 4.25: 36 results, 0 included (literal conditional references not rewritten; as documented).
- 4.29c: 54 included (36 Encounters + 18 Procedures; matrix 42 = 24+18 corpus only).

## Errors / WARN in HFS log
No ERROR/WARN lines caused by T4 queries. WARNs in the window belong to other tracks (SOF 1,000,000-row limit, `$sql-run` missing table).

## Findings
1. **`GET /Patient?_has:Observation:patient:code=http://loinc.org|8302-2` (4.11) does not complete.** Client timeout 600 s (first try, HTTP 000) and again 900 s (08:49:45-09:04:45, HTTP 000, no body, no 504 despite HFS_REQUEST_TIMEOUT=600). Health stayed 200, pid unchanged, but the server stayed at ~100-120% CPU for ~25 more minutes (query keeps running after the client gives up, no cancel) and other requests (Patient read, /ui) were delayed or timed out during it. Expected per matrix: ">0" quickly. Not run in the UI.
2. Very slow reference/quantity searches over Observation (each ~1.5-8 min while other tracks were running): 4.6a 137 s, 4.10a 104 s, 4.21c 492 s (`Observation?patient=LPID&code=29463-7&value-quantity=gt60`, 2 results), 4.21d 183 s UI, 4.27b 211 s UI, 4.5a 165 s. Large-corpus performance, not a wrong result; includes contention with other tracks.
3. `Encounter?patient=PID&date=ge2016` took 31-38 s for 24 results.

## STOP-AND-ASK
- **Open in New Tab (all of 8.2-8.4, "URL matches what was typed")**. Matrix line 744/828. UI element: none present in the Results card (DOM search for "Open in New Tab"/"new tab" returned 0; `events` array empty: no popup/dialog). Options: (a) agent workaround: the executed request URL was captured from the browser network log (e.g. `http://localhost:8080/Patient?_count=20&_total=accurate`) and the same URLs are verified with curl in the sweep (does NOT prove a link exists or its href); (b) owner checks manually whether a link should exist and sends a screenshot; (c) mark the "Open in New Tab" parts N/A / matrix text outdated. Effect: pass criterion "Open in New Tab URL matches" not evidenced; all counts/shapes are.

## Output files
`outputs/t4-api-sweep.tsv`, `outputs/t4-ui-sweep.tsv`, `outputs/T4-<row>.body`, `outputs/T4-4.11-retry.*` (empty), `outputs/t4-rows.md`; steps appended to `metrics/steps.tsv`.
Screenshots (`screenshots/`): `T4-8.1-01..04-*`, `T4-8.x-<row>-01-query-results.png` for 4.1a-d, 4.2a-d, 4.3a-c, 4.4a-c, 4.5a, 4.6a-c, 4.7a-b, 4.8, 4.9a-b, 4.10a-b, 4.12a-c, 4.13, 4.15a-d, 4.16a-b, 4.17a, 4.18a-b, 4.19a, 4.20a/e, 4.21d, 4.22a-b, 4.23a/d, 4.24b, 4.25, 4.26a/c, 4.27a-b, 4.28a/c/d, 4.29a-b; `T4-8.2-4.12c-01/02-*`, `T4-8.2-4.12-sort-01/02-*`, `T4-8.2-4.14-01..05-*`. (Screens named with the `8.x` token cover 8.2, 8.3 and 8.4 rows by row number.)

## Resolution of "Open in New Tab" (2026-10-08)
Owner decision: **N/A**. The button was removed on purpose by #958 (closed 2026-09-08); the matrix lines that mention
it (744, 784, 806, 828) are outdated. The executed URLs were verified through the network log and curl, as described above.
