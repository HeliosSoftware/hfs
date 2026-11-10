# Manual test pass #939: PostgreSQL + Elasticsearch (`pg-es`), anonymous access

Tester: Edson Terceros (run driven by Claude Code). Date: 2026-11-09 and 2026-11-10 (UTC). Commit: `main` at `f39f48276` (Release version 0.2.8). Host: Linux (Proxmox LXC), 8 cores, 23 GB RAM. PostgreSQL 16 and Elasticsearch 8.15.0 (8 GB heap) in Docker, corpus served by nginx:alpine over HTTP/1.1. Evidence for issue #939; the procedure is `MANUAL_TESTING_MATRIX.md` sections 2 to 13.

## Result in one paragraph

T0 to T9 were run end to end on the `pg-es` row. The full 18,955,865-resource Synthea corpus imported through the Import page in **1 h 12 min 52 s** and was fully searchable (after the deferred reindex repaired 4,049 resources the index had rejected) **1 h 15 min 50 s** after the start; PostgreSQL and Elasticsearch counts equal the corpus counts for all 24 types. The rows that failed on the first run were each investigated against an independent ground truth (PostgreSQL SQL, the NDJSON corpus, the source code): **one application defect was found** (the submission page's "Processing finished at" time is taken at the first status poll, not at completion); everything else is documented behavior, a limit the matrix itself already describes, a setting of this run that is outside the recommended recipe, or a mistake of our scripts or of the matrix text, all listed below with their fix. Nothing was found that would block a release on `pg-es`.

## Deviations from the matrix

- **R4-only build.** `cargo build --release -p helios-hfs --features postgres,elasticsearch,subscriptions,xml` instead of `cargo build --workspace --all-features --release` (T0). The T1 check of the R5 version switch is therefore `n/a`.
- **Bulk-submit indexing settings** were `HFS_BULK_SUBMIT_INDEX_QUEUE=64`, `INDEX_CONCURRENCY=8`, `INDEX_COALESCE=4`, `FILE_CONCURRENCY=4`, `BATCH_SIZE=1000` (the previous campaign's settings from the #939 comment), not the recipe in `crates/rest/README.md` (finding 2).
- **Ports and URLs.** PostgreSQL 18932, Elasticsearch 18921, HFS 18080 (`HFS_BASE_URL=http://localhost:18080`), corpus server 18801 with a copy of `manifest.json` whose URLs point at it.
- **T7 at full scale.** Rows 7.b, 7.d, 7.o, 7.p, 11.5 "all three subjects" and "Export as files" use `tall_female_patients`, whose dependency `observation_flat` exceeds the 1,000,000-row source limit on 7.7 M Observations. They were run a second time with `observation_flat` narrowed to the one Observation code the query uses (ids `*-narrow`), then the original view was restored.
- **Server not empty at the first T1.** A first attempt was discarded and the whole stack was emptied (database dropped, Elasticsearch indices deleted, new index prefix) after a test script wrote 9 resources to the real server by mistake (a Basic topic, a Subscription, 6 Encounters and 1 Condition). T1 to T9 in this document are all from the second, clean start.

## Matrix row (section 1) for `pg-es`

| T0 | T1 | T2 | T3 | T4 | T5 | T6 | T7 | T8 | T9 |
|---|---|---|---|---|---|---|---|---|---|
| ✅ (R4 only) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (full scale rows narrowed, see deviations) | ✅ | ✅ |

## Per-step result and counts

| Step | Script rows | Notes |
|---|---|---|
| T1 | 7 pass, 1 fail, 1 n/a | /health, /metadata (4.0.1), operations, startup line, dashboard 1,377 stored resources, clinical rail at 0. |
| T2 | 8 pass, 2 fail, 0 n/a | Transaction rolls back, batches create 9 and 8, transaction creates 662; 49 Encounter, 106 Observation, 33 Condition for the T2 patient; three rejected files. |
| T3 | 7 pass, 1 fail, 0 n/a | Details below. |
| T4 | 33 pass, 1 fail, 0 n/a | 29 search rows plus the `_total=accurate` criterion (93 UI requests, 0 without it). |
| T5 | 36 pass, 3 fail, 0 n/a | System, patient and group exports; formats; cancel; windows. Rows 5.1 and 5.2 were repeated with a corrected script (`*-rerun`). |
| T6 | 15 pass, 0 fail, 0 n/a | patient_demographics and observation_flat, UI and `$sql-run` in json, csv, ndjson and parquet (PAR1 checked). |
| T7 | 31 pass, 7 fail, 0 n/a | Grid of subjects by format, filters, cancel and retry, restart; narrowed re-run of the rows that hit the 1,000,000-row limit. |
| T8 | 11 pass, 0 fail, 0 n/a | Basic SubscriptionTopic, rest-hook Subscription, 4 notifications per upload, `$status`, `$events`, retry after the receiver returns. |
| T9 | 15 pass, 0 fail, 0 n/a | Cards, row, failure path timed against the matrix (streak 1 at 5 min 16 s, streak 2 at 10 min 17 s, Error at 15 min 19 s), reactivation, restart with rehydration, engine disabled. |

The tally counts the raw rows, including the first-run rows that were later superseded (see "Rows that failed").

## T3: import of the full corpus

| Moment (UTC) | Time | Since start |
|---|---|---|
| Submission created (`7.2`) | 2026-11-09 14:00:46 | 0 |
| Last ingest log line (`resources ingested but not indexed ... deferred reindex`) | 15:13:38 | **1 h 12 min 52 s** |
| Deferred reindex finished (`deferred reindex generation completed`) | 15:16:36 | **1 h 15 min 50 s** |
| "Processing finished at" shown by the UI | 15:51:02 | 1 h 50 min 16 s (see finding 3) |

- Status card: **Completed**, 24 output files, 0 error files, "4,049 resources flagged with warnings (e.g. being re-indexed for search)".
- Throughput: about 4,330 resources per second to the end of ingest, about 4,170 per second to fully searchable. The #939 comment of 2026-09-27 reported 1 h 25 min 58 s (about 3,675 per second) on commit `13649338f`.
- Counts: `outputs/t3-pg-counts.tsv` (PostgreSQL through `_summary=count`) and `outputs/t3-es-counts.tsv` (Elasticsearch `_count` with the filter of 7.5) equal `corpus-counts.tsv` plus the resources created by the T2 fixtures for **all 24 types, 0 differences**. Patient 11,705 and Observation 7,699,987.
- HFS memory peaked near 12.6 GB during the Observation file.
- The corpus byte check of 7.1 (`corpus-served-bytes.txt`): 24 of 24 files `ok`, no `TRUNC`.

## Findings

Classification used: **APP DEFECT** (proved, our setup ruled out), **BY DESIGN / DOCUMENTED** (code or docs say so), **SETTINGS** (outside the recommended recipe), **SCRIPT / MATRIX** (our script or the matrix text was wrong).

| # | Where | Classification | What happened and the evidence |
|---|---|---|---|
| 1 | T2 6.2c, T2 6.1 | BY DESIGN / DOCUMENTED + MATRIX | The Resource Types rail read Practitioner ≈5 while the API and a search return 4. The rail is fed by in-memory counters reconciled every `HFS_DASHBOARD_RECONCILE_SECS` (30 s); a write that commits during a reconcile can be counted twice until the next one (`crates/observability/src/dashboard_counters.rs`, module comment "The begin/finish subtraction"). The rail marks it with `≈`. The exact count by API was 4/5/4/4. The cause of the +1 is inferred from the code and the batch timing, not reproduced. The error of the rolled-back transaction (6.1) named a `Location?identifier=…`; the matrix lists only Organization or Practitioner. |
| 2 | T3 7.4 | SETTINGS (documented self-healing) | 8 pages with 4,049 resources in total (ExplanationOfBenefit 880, Observation 986, Procedure 1,151, Provenance 1,032) were rejected by the search index (`writing to the search index timed out after 30s`) at the memory peak with queue 64 and 8 index workers; the README warns that 8 index workers were unstable on postgres + Elasticsearch. The mechanism worked as designed (`ingest_index_sink.rs`): the batches were marked unindexed and the deferred reindex repaired them (4049 of 4049, `failed=0`, 38 s). Final counts matched in PostgreSQL and Elasticsearch. Not proved to be a product defect; to prove it, repeat with the README recipe (queue 16, concurrency 4) and see if the warnings disappear. |
| 3 | T3 7.3 / §14 | **APP DEFECT** (misleading timestamp) | The submission page prints "Processing finished at 2026-11-09T15:51:02Z", the instant of the first status poll after completion; the server log shows ingest done at 15:13:38Z and searchable at 15:16:36Z. `crates/ui/src/bulk_import.rs` (the `completed_at` match, falls back to `now_stamp()` = `Utc::now()` at poll time); there is no completion time recorded by the worker for bulk-submit. If nobody polls the status fragment, the page keeps showing "Queued - starting shortly" and In Progress. Our run did not keep the page polling (matrix 7.3 says to keep it open), which is why we noticed; the timestamp is still wrong in any case where the first poll comes late. |
| 4 | T4 4.22 | SCRIPT / MATRIX | `LPID is among the rows` cannot hold: the `_has` query matches 5,294 patients (5,293 from the corpus plus the T2 patient) and `_count=50` returns the first page. PostgreSQL (8,756 Conditions with the code, 5,294 distinct patients), the corpus NDJSON (8,755, 5,293) and the API (5,294) agree. Rerun row `4.22-rerun`. |
| 5 | T5 5.1 | SCRIPT / MATRIX | The export is right (6 files; Location 1,142, Organization 1,140, Practitioner 1,140 = stored counts). The expected numbers ignored the T2 fixtures; the matrix says 137 in one place and 1,142 in another. Rerun row `5.1-rerun`: pass. |
| 6 | T5 5.2 | SCRIPT, then ORDER | First run: the "Cari853" picker lists 3 patients and the script clicked the first (Tanna708 Cari853 Hegmann834: 35 Condition, 242 Observation, exactly what the export returned). Rerun with the patient picked by id: the anchor patient exported 1 Patient, 16 Condition, 165 Observation, equal to what the API stores; the matrix says 15 Condition, and the 16th is the Condition created by T8 step 12.2.2 (14:12:04Z), because the rerun ran after T8. Row `5.2-rerun` is recorded as fail only for that reason. |
| 7 | T7 7.b, 7.d, 7.o, 7.p, 11.5, exp-as-files | DOCUMENTED LIMIT | `tall_female_patients` depends on `observation_flat` (7,699,987 Observations); the default `HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD=1000000` stops it with the message that names the dependency and the variable (`crates/rest/src/handlers/sof/sqlquery.rs`, matrix note 2, finding 9 of the sqlite-es pass). With `observation_flat` narrowed to code 8302-2 the same rows ran: see the `*-narrow` rows (4,945 rows for `min_height=150` in csv, ndjson, parquet and json, 11,705 and 5,814 for the other two subjects). The original view was restored (`7n.restore`). The first narrowed run of `7.tbl-controls-narrow` ended Cancelled because T9 restarted the server (13.5) while it was running; it was repeated alone as `7.tbl-controls-narrow-rerun`: pass. |
| 8 | T1 5.4, T2 6.3c, T3 7.3a | SCRIPT | 5.4 expected the literal `pg-es` and the prefix `hfs`; the log says `storage_backend=postgres-elasticsearch` and `index_prefix=hfs_20261109_1357` (row `5.4-rerun`: pass). 6.3c could not find a string in the CodeMirror text of the modal (only the visible lines exist in the DOM); the API check of the same fact passed. 7.3a searched the label in lowercase and the page renders it in uppercase. |
| 9 | T7 harness | SCRIPT | Not an HFS issue: after printing its last line the Node process of the T7 script stayed alive with an open handle and had to be stopped by hand so the chain could continue. |

Matrix text corrections that came out of this run are appended to `MANUAL_TESTING_MATRIX.md` in the section "PostgreSQL + Elasticsearch — findings from the pg-es pass".

## Rows that failed or were superseded

| Step | Row | Result | Note |
|---|---|---|---|
| T1 | 5.4 | fail | startup line storage_backend=postgres-elasticsearch (expected pg-es) MISMATCH \| ES index prefix "hfs" NOT named in log (1 prefix lines) \| Seeded SearchParameters created=1372 (expected 1372 on an empty serve… |
| T2 | 6.2c | fail | FAIL: rail Organization ≈4, Location ≈5, Practitioner ≈5, PractitionerRole ≈4 (expected Organization 4, Location 5, Practitioner 4, PractitionerRole 4; the ≈ form also passes) \| ok: API counts {"Organization"… |
| T2 | 6.3c | fail | ok: 49 Encounters read via API; 0 with non-literal/unknown serviceProvider, participant[0].individual or subject != Patient/01a1c09b-0d9e-7d81-8ab6-06881cea69e7; sample: serviceProvider Organization/01a1c09a-e… |
| T3 | 7.3a | fail | summary: All Submissions synthea-pg-es The manifests, status, and run log of this submission. MANIFEST URL http://localhost:18801/manifest.json DATA RECIPIENT http://localhost:18080 SUBMISSION ID 43fde6d2-4b9c… |
| T4 | 4.22 | fail | ok: /Patient?_id=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&_revinclude=Immunization:patient => ui 1 +8inc, api 1 +8inc \| FAIL: /Patient?_has:Condition:patient:code=http://snomed.info/sct\|706893006&_count=50 => ui… |
| T5 | 5.1 | fail | status=complete; card="6 files"; files=6 (server-count expects 6); lines {"Location":1142,"Organization":1140,"Practitioner":1140} vs stored {"Organization":1140,"Practitioner":1140,"Location":1142}; wellForme… |
| T5 | 5.2 | fail | status=complete; files=3 (expect 3); lines {"Condition":35,"Observation":242,"Patient":1} vs stored-by-API {"Patient":1,"Condition":15,"Observation":165}; Patient id==PID false; matrix(1/15/165): false |
| T7 | 7.b | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries materialize each dependency in full bef… |
| T7 | 7.d | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries materialize each dependency in full bef… |
| T7 | 7.o | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries materialize each dependency in full bef… |
| T7 | 7.p | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries materialize each dependency in full bef… |
| T7 | 7.tbl-controls | fail | Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_patients"] hint="2 of 4 selected · 1 value missing"; after tick+filter obs hint="3 of 4 selected · 1 value missing"; s… |
| T7 | 7.exp-as-files | fail | exception: locator.waitFor: Timeout 15000ms exceeded. |
| T7 | 7.tbl-controls-narrow | fail | Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_patients"] hint="2 of 4 selected · 1 value missing"; after tick+filter obs hint="3 of 4 selected · 1 value missing"; s… |
| T5 | 5.2-rerun | fail | status=complete; files=3 (expect 3); lines {"Condition":16,"Observation":165,"Patient":1} vs stored-by-API {"Patient":1,"Condition":16,"Observation":165}; Patient id==PID true; matrix(1/15/165): false |

Rerun rows (`*-rerun`, `*-narrow`) are in the full table below; the T7 and T5 reruns exist because the first result was a limit or a script error as classified above.

## Every row

<details><summary>T1: 7 pass, 1 fail, 1 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 5.1 | pass | GET /health -> 200 {"status":"ok","service":"hfs","version":"0.2.8","uptime_seconds":54.180939756,"started_at":"2026-11-09T13:57:46.9655329 (expected 200) |
| 5.2 | pass | GET /metadata -> 200, fhirVersion 4.0.1 (expected 4.0.1), software Helios FHIR Server, 147 resource types |
| 5.3 | pass | operations: bulk-submit bulk-submit-status export group-export patient-export reindex sql-export sql-run validate versions (expected at least export, sql-run, sql-export… |
| 5.4 | fail | startup line storage_backend=postgres-elasticsearch (expected pg-es) MISMATCH \| ES index prefix "hfs" NOT named in log (1 prefix lines) \| Seeded SearchParameters creat… |
| 5.5 | pass | dashboard Stored resources "1.4k" (expected ~1.4k), chart total "1,377" (expected 1,377); API SearchParameter 1372 (expected 1372) + CompartmentDefinition 5 (expected 5)… |
| 5.6 | pass | clinical rail {"Patient":"0","Encounter":"0","Observation":"0","Condition":"0","Organization":"0","Practitioner":"0","Location":"0"} (expected all 0); SearchParameter ra… |
| 5.7 | pass | sidebar FHIR selector reads "FHIR R4" (expected FHIR R4) |
| 5.8 | na | R4-only build: no R5 available, version switch not applicable |
| 5.4-rerun | pass | Corrected check (5.4 failed because the script expected the literal pg-es and the default prefix hfs): startup line storage_backend=postgres-elasticsearch (pg-es = postg… |

</details>

<details><summary>T2: 8 pass, 2 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 6.1 | pass | ok: upload of Nicky270_Ann985_Larkin917_b9e958ef-8c96-12b1-f629-d9536dcbdfb1.json moved to Execution Plan \| ok: request strip "POST [base] · Bundle · transaction · 662 … |
| 6.2a | pass | ok: strip "POST [base] · Bundle · batch · 9 entries" (expected "POST [base] · Bundle · batch · 9 entries") \| ok: notice "Batch: entries run independently — a failed ent… |
| 6.2b | pass | ok: strip "POST [base] · Bundle · batch · 8 entries" (expected "POST [base] · Bundle · batch · 8 entries") \| ok: notice "Batch: entries run independently — a failed ent… |
| 6.2c | fail | FAIL: rail Organization ≈4, Location ≈5, Practitioner ≈5, PractitionerRole ≈4 (expected Organization 4, Location 5, Practitioner 4, PractitionerRole 4; the ≈ form also p… |
| 6.3a | pass | ok: Per-Action Outcomes: HTTP 200, head "662 created" (expected "662 created"), 662 rows, statuses {201 Created} (expected all "201 Created"), summary "" \| ok: Done ret… |
| 6.3b | pass | ok: GET /Patient?given=Nicky270&family=Larkin917&birthdate=1996-04-19 -> API 1 result, UI "1 results" (expected 1); LPID=01a1c09b-0d9e-7d81-8ab6-06881cea69e7 \| ok: moda… |
| 6.3c | fail | ok: 49 Encounters read via API; 0 with non-literal/unknown serviceProvider, participant[0].individual or subject != Patient/01a1c09b-0d9e-7d81-8ab6-06881cea69e7; sample:… |
| 6.4a | pass | not-a-bundle.json -> "That JSON is not a FHIR Bundle." (expected "That JSON is not a FHIR Bundle."); page stays on Upload=true |
| 6.4b | pass | collection.json -> "Only Bundles of type batch or transaction can be executed here." (expected "Only Bundles of type batch or transaction can be executed here."); page s… |
| 6.4c | pass | not-json.log -> "That file is not valid JSON. (Unexpected token 'b', "build star"... is not valid JSON)" (expected "That file is not valid JSON. (<parser detail>)"); pag… |

</details>

<details><summary>T3: 7 pass, 1 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 7.2 | pass | empty list="true", auth none=true, created id 43fde6d2-4b9c-483b-ae60-ed8567cc0a0a at 2026-11-09T14:00:46.469Z UTC (T3 start) |
| 7.3a | fail | summary: All Submissions synthea-pg-es The manifests, status, and run log of this submission. MANIFEST URL http://localhost:18801/manifest.json DATA RECIPIENT http://loc… |
| 7.3b | pass | status Completed; ingest wall-clock 6631s (2026-11-09T14:00:46.469Z -> 2026-11-09T15:51:17.731Z); output files 24; error files 0; text: All Submissions synthea-pg-es The… |
| 7.3b | pass | status Completed=true; Output files 24; Error files 0; warnings: 4,049 resources flagged (re-index); UI "Processing finished at" 2026-11-09T15:51:02.205Z = 1h 50m 16s af… |
| 7.4 | pass | mode DEFER_INDEXING=false (index during ingest). Index rejected 4049 resources in 9 pages (reason: writing to the search index timed out after 30s) at the memory peak; t… |
| 7.5-counts-api | pass | 24 corpus types in HFS (API _summary=count) vs corpus-counts.tsv + the resources created by the T2 fixtures (all types): 0 mismatches . Detail in outputs/t3-pg-counts.tsv |
| 7.5-ui | pass | dashboard after import: Home RESOURCE TYPES 28 used for R4 STORED RESOURCES 19.0M across active tenant EXPORT JOBS 0 running (0 queued) IMPORT JOBS 0 active UPTIME 1h 54… |
| 7.5-es-counts | pass | Elasticsearch _count per type (filter is_deleted=false, must_not is_contained, index hfs_20261109_1357_default_<type>) equals corpus + T2 for all 24 types: 0 mismatches,… |

</details>

<details><summary>T4: 33 pass, 1 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 8.1a | pass | ok: chip "No issues." (expected "No issues.") \| ok: Unsaved changes pill count 0 (expected 0) \| ok: status line "" \| ok: GET /RiskAssessment/manual-risk -> 200 |
| 8.1b | pass | ok: chip "No issues." (expected "No issues.") \| ok: Unsaved changes pill count 0 (expected 0) \| ok: status line "" \| ok: GET /ValueSet/manual-test-vs -> 200 |
| 4.1 | pass | ok: /Patient?family=Parker433 => ui 30, api 30 \| ok: /Patient?family:exact=Parker433 => ui 30, api 30 \| ok: /Patient?family:contains=arker43 => ui 30, api 30 \| ok: /P… |
| 4.1-post | pass | POST /Patient/_search family=Parker433 -> 200 total 30 vs GET 30 |
| 4.2 | pass | ok: /Patient?gender=female => ui 5,814, api 5,814 \| ok: /Patient?gender:not=female => ui 5,891, api 5,891 \| ok: /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999… |
| 4.3 | pass | ok: /Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01 => ui 1,268, api 1,268 \| ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&date=ge2016 => ui 24, ap… |
| 4.4 | pass | ok: /RiskAssessment?probability=gt0.5 => ui 1, api 1 \| ok: /RiskAssessment?probability=lt0.5 => ui 0, api 0 \| ok: /RiskAssessment?probability=ap0.8 => ui 1, api 1 |
| 4.5 | pass | ok: /Observation?code=8302-2&value-quantity=gt150 => ui 146,589, api 146,589 \| ok: /Observation?code=8302-2&value-quantity=gt150\|\|cm => ui 146,589, api 146,589 \| ok:… |
| 4.6 | pass | ok: /Observation?subject=Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 165, api 165 \| ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 15, api 15 … |
| 4.7 | pass | ok: /ValueSet?url=http://example.org/fhir/ValueSet/manual-test => ui 1, api 1 \| ok: /ValueSet?url:below=http://example.org/fhir => ui 1, api 1 |
| 4.8 | pass | ok: /Observation?code-value-quantity=http://loinc.org\|8302-2$gt150 => ui 146,589, api 146,589 \| ok: composite total 146589 equals 4.5 first count 146589 \| ok: 100 row… |
| 4.9 | pass | ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583,01a1c09b-0d9e-7d81-8ab6-06881cea69e7 => ui … |
| 4.10 | pass | ok: /Observation?subject.identifier=http://hl7.org/fhir/sid/us-ssn\|999-33-3920 => ui 165, api 165 \| ok: /Observation?subject:Patient.family=Parker433&_count=5 => ui 11… |
| 4.11 | pass | ok: /Patient?_has:Observation:patient:code=http://loinc.org\|8302-2&_count=5 => ui 11,705, api 11,705 \| ok: /Observation?patient=3dd31fcb-fd81-3a87-678e-b4c1128b4a23&co… |
| 4.12 | pass | ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_revinclude=Condition:patient => ui 1 +15inc, api 1 +15inc \| ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b… |
| 4.13 | pass | ok: /Patient?_content=Everett => ui 163, api 163 \| ok: ES served it? log elasticsearch lines: none; ES search query_total delta 2 |
| 4.14 | pass | ok: QUERY box reads "GET /Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate" \| ok: Run: ui 5, api 5 Parker433 children, birth dates ascending: 2013-02-03,… |
| 4.15 | pass | ok: /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999-19-2626 => ui 1, api 1 \| ok: /Patient?address-city=Millis => ui 19, api 19 \| ok: /Patient?address-city:exac… |
| 4.16 | pass | ok: /Patient?_lastUpdated=lt2026-11-09T14:00:46.469Z => ui 1, api 1 \| ok: /Patient?_lastUpdated=ge2026-11-09T14:00:46.469Z => ui 11,704, api 11,704 |
| 4.17 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&class=EMER => ui 5, api 5 \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&class=IMP => ui 1, a… |
| 4.18 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&_include=Encounter:service-provider => ui 49 +4inc, api 49 +4inc \| ok: /Encounter?patient=01a1c09b-0d9e-7d81… |
| 4.19 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&service-provider.name=ENCOMPASS => ui 38, api 38 \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69… |
| 4.20 | pass | ok: /Organization?name=TIMOTHY DANIELS HOUSE => ui 2, api 2 \| ok: /Organization?address-city=HOLLISTON => ui 2, api 2 \| ok: /Practitioner?identifier=http://hl7.org/fhi… |
| 4.21 | pass | ok: /Condition?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&clinical-status=active => ui 6, api 6 \| ok: /Condition?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&code=htt… |
| 4.22 | fail | ok: /Patient?_id=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&_revinclude=Immunization:patient => ui 1 +8inc, api 1 +8inc \| FAIL: /Patient?_has:Condition:patient:code=http://sn… |
| 4.23 | pass | ok: /Patient?identifier=https://github.com/synthetichealth/synthea\|7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?phone=555-613-6236 => ui 1, api 1… |
| 4.24 | pass | ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&class=AMB => ui 23, api 23 \| ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&class=EMER => ui 1,… |
| 4.25 | pass | ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_include=Encounter:service-provider => ui 24, api 24 \| ok: serviceProvider.reference left as conditional str… |
| 4.26 | pass | ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&clinical-status=active => ui 1, api 1 \| ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&code=htt… |
| 4.27 | pass | ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&code=8302-2&_sort=date&_count=20 => ui 15, api 15 \| ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b1… |
| 4.28 | pass | ok: /Organization?identifier=https://github.com/synthetichealth/synthea\|e2a8b444-9b8f-36ff-84c4-05ee98589482 => ui 1, api 1 \| ok: /Organization?name=WHITLEY WELLNESS =… |
| 4.29 | pass | ok: /Patient?_has:Condition:patient:code=http://snomed.info/sct\|65363002&_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?_id=7d24f7a0-6f2e-ce3b-… |
| 4.pc | pass | 93 UI queries sent; 0 without _total=accurate; last: http://127.0.0.1:18080/Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_revinclude=Encounter:patient&_revinclude=Pr… |
| 4.22-rerun | pass | Corrected expectation (the 4.22 failure was a script/matrix error, see RESULTS.md): GET /Patient?_has:Condition:patient:code=http://snomed.info/sct\|706893006 total=5294… |

</details>

<details><summary>T5: 36 pass, 3 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 5.0-group-fixture | pass | GET Group/manual-group -> 200, member=Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583, Unsaved pill visible=false |
| 5.6 | pass | message visible=true text="Enter a name for this export." url=http://127.0.0.1:18080/ui/bulk-export/new (expected "Enter a name for this export." on /new) |
| 5.6-nojs | pass | no-JS POST -> 400, message present=true (expected 400 + message) |
| 5.0-clear | pass | after Clear: allResources=false anyType=false anyDisabled=false name="clear-test" elements="id,gender" typeUsable=true |
| 5.0-clear-after-reject | pass | rejection visible before Clear=true, after Clear=true; allResources=false anyType=false name="reject-then-clear" customInstant="2026-02-30T00:00:00Z" typeUsable=true |
| 5.0-nojs-clear-hidden | pass | Clear hidden without JS=true, still hidden on the rejected page=true; POST 400 message present=true; name kept=true; All Resources unchecked manually=true |
| 5.2-picker | pass | Parker433: first page 8 options, footer "38 matches · scroll for more", after scroll 16 options, footer "38 matches · scroll for more" (API name total 38); Cari853: 3 op… |
| 5.1 | fail | status=complete; card="6 files"; files=6 (server-count expects 6); lines {"Location":1142,"Organization":1140,"Practitioner":1140} vs stored {"Organization":1140,"Practi… |
| 5.1-zip | pass | suggested=bulk-export.zip bytes=3198174 magic=PK entries=6 (expected 6 NDJSON files) |
| 5.2 | fail | status=complete; files=3 (expect 3); lines {"Condition":35,"Observation":242,"Patient":1} vs stored-by-API {"Patient":1,"Condition":15,"Observation":165}; Patient id==PI… |
| 5.3 | pass | status=complete; files=2 (expect 2); Patient lines=1; Condition lines=1 (API active=1, all conditions=15); allActive=true allBelongToPID=true |
| 5.4 | pass | status=complete; files=12 (stored 11705 Patients -> expect 12); lines=11705; only id/gender/meta keys=true; meta.tag SUBSETTED on all=true; matrix 12 files: true |
| 5.5 | pass | after Cancel chip status=cancelled text="cancel-me Cancelled Run again Delete Delete cancel-me and its output files from the server? This cannot be undone. Delet" (expec… |
| 5.5-delete | pass | card removed=true; warning="Delete cancel-me and its output files from the server? This cannot be undone." |
| 5.7 | pass | status=complete; Patient lines=11704 vs API _lastUpdated>=T3_START 11704; files=12; window line has "Since 2026-11-09T14:00:46.469Z"=true; matrix 11704 lines/12 files: t… |
| 5.8 | pass | status=complete; Patient lines=1 vs API _lastUpdated<=T3_START 1; window "Until 2026-11-09T14:00:46.469Z"=true; families=["Larkin917"]; matrix 1 Larkin917: true |
| 5.9 | pass | status=complete; Patient lines=11704 vs API window 11704; window line has both instants=true; text="since-until Complete View files Run again Delete Delete since-until a… |
| 5.10 | pass | status=complete; lines {"Group":1,"RiskAssessment":1,"ValueSet":1} vs API {"RiskAssessment":1,"ValueSet":1,"Group":1}; ids {"Group":["manual-group"],"RiskAssessment":["m… |
| 5.11 | pass | status=complete; window instant=2026-11-09T13:51:44Z (0.0 min from now-24h); Organization lines=1140 vs API last-24h 1140; files=2; card="2 files" |
| 5.12 | pass | error visible=true text="Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z." stayedOnForm=true; after preset->All time field disabled=true, submit started an expo… |
| 5.12-nojs | pass | no-JS POST -> 400, message present=true |
| 5.13 | pass | first card status=failed text="bad-group Failed Retry Delete Delete bad-group and its output files from the server? This cannot be undone. Delete export Keep export kick… |
| 5.13-delete | pass | both cards deleted: first=true retry=true |
| 5.A1-system | pass | system $export _type=Organization: manifest 2 file(s) [Organization:1000, Organization:140] requiresAccessToken=false; downloaded 1140 lines vs stored 1140; content-type… |
| 5.A2-patient | pass | patient $export _type=Patient,Condition: manifest 489 file(s) [Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:… |
| 5.A3-group | pass | group $export manual-group _type=Patient,Condition: manifest 2 file(s) [Condition:15, Patient:1] requiresAccessToken=false; Patient 1 (expect 1), Condition 15 vs stored … |
| 5.A4-format-application/fhir+ndjson | pass | _outputFormat=application/fhir+ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 20… |
| 5.A4-format-application/ndjson | pass | _outputFormat=application/ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 202 and… |
| 5.A4-format-ndjson | pass | _outputFormat=ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 202 and a complete … |
| 5.A5-bad-format-application/fhir+json | pass | _outputFormat=application/fhir+json -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat … |
| 5.A5-bad-format-text/csv | pass | _outputFormat=text/csv -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat 'text/csv'"}}… |
| 5.A5-bad-format-parquet | pass | _outputFormat=parquet -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat 'parquet'"}}]}… |
| 5.A5-no-prefer | pass | no Prefer: respond-async -> 400  |
| 5.A5-until-before-since | pass | _until < _since -> 400 (expected 400) |
| 5.A6-bad-group | pass | kick-off 404 (the UI row 5.13 shows Failed; the API accepted-or-rejected outcome is recorded here) |
| 5.A7-since-window | pass | window 2026-11-09T14:00:46.469Z..2026-11-09T15:16:36Z: 11704 Patient lines vs API 11704 |
| 5.A8-cancel | pass | DELETE status -> 202 (expect 202); GET after -> 404 (expect 404) |
| 5.1-rerun | pass | status=complete; card="6 files"; files=6 (server-count expects 6); lines {"Location":1142,"Organization":1140,"Practitioner":1140} vs stored {"Organization":1140,"Practi… |
| 5.2-rerun | fail | status=complete; files=3 (expect 3); lines {"Condition":16,"Observation":165,"Patient":1} vs stored-by-API {"Patient":1,"Condition":16,"Observation":165}; Patient id==PI… |

</details>

<details><summary>T6: 15 pass, 0 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 6.1.1-create-new | pass | title="New View Definition" starter name="new_view" (expected title "New View Definition" and a starter document) |
| 6.1.2-3-paste-and-results | pass | meta="50 rows · 9 ms" (expect "50 rows · N ms", stored Patients 11705, capped at 50); columns=id,gender,birth_date,family,city; table rows=50; chip="No issues." (expect … |
| 6.1.4-lint | pass | chip="2 issues"; tooltip="Unknown key "colum"Rename to "column"Remove "colum"unknown-key"; panel has Unknown key=true / select-needs-column=true; quick-fix menu="Unknown… |
| 6.1.5-completion | pass | getRes options [getResourceKeygetResourceKey()] has getResourceKey=true; accepted -> inserted "getResourceKey()"=true; object-position keys [resourcestring · required \|… |
| 6.1.6-save | pass | VD=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e url=http://127.0.0.1:18080/ui/sql/view-definitions?vd=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e&saved=1&return_to=%2Fui%2Fsql%2Fview-d… |
| 6.1.7-crosscheck | pass | row 01a1c09b-0d9e-7d81-8ab6-06881cea69e7 \| female \| 1996-04-19 \| Larkin917 \| Millis vs GET /Patient?_id=01a1c09b-0d9e-7d81-8ab6-06881cea69e7: gender=female birthDate… |
| 6.2-observation_flat | pass | meta="50 rows · 9 ms" (expect 50 rows); columns=id,patient_id,code,value,effective; sample codes=4548-4,8302-2,72514-3,29463-7,39156-5 (matrix: codes such as 8302-2; pre… |
| 6.A1-sql-run-patient_demographics | pass | patient_demographics: ndjson 200 50 rows ct=application/x-ndjson; json 200 50 rows; csv 200 header="id,gender,birth_date,family,city" 50 rows; parquet 200 5519 bytes hea… |
| 6.A2-sql-run-observation_flat | pass | observation_flat: ndjson 200 50 rows ct=application/x-ndjson; json 200 50 rows; csv 200 header="id,patient_id,code,value,effective" 50 rows; parquet 200 3906 bytes head=… |
| 6.A3-api-create-roundtrip | pass | ViewDefinition?name=patient_demographics total=1, name=observation_flat total=1 (expect 1 each) |
| 6.3.1-filter | pass | rail after filter "patient": [patient_demographics] (expected only patient_demographics) |
| 6.3.2-duplicate | pass | copy1 patient_demographics_copy http://example.org/ViewDefinition/patient_demographics_copy; copy2 patient_demographics_copy_2 http://example.org/ViewDefinition/patient_… |
| 6.3.3-dependent-sql-view | pass | Reads-from link to original=/ui/sql/view-definitions?vd=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e&return_to=%2Fui%2Fsql%2Fviews%3Flib%3D01a1c5bc-498c-7b01-8e91-3da114573749; … |
| 6.3.4-delete | pass | copy1: confirm text shown=true deleted=true; copy2: confirm shown=true deleted=true; rail now [nourl_vd, nourl_vd_copy, observation_flat, patient_demographics] |
| 6.3.5-negative-resource-nope | pass | chip="1 issue"; notice="Could not run the view. $sql-run returned 422 Unprocessable Entity: unknown resource type "Nope""; meta="last successful run"; save prompt shown=… |

</details>

<details><summary>T7: 31 pass, 7 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 7.setup-vd | pass | patient_demographics exists (01a1c5bc-0cf9-7342-8acf-e9ce61ee321e); observation_flat exists (01a1c5bc-1e76-7df1-8b14-415f4bd27af4) |
| 7.setup-view | pass | editor=codemirror/codemirror; saved=true notice="Saved." rail="female_patients active" QV=01a1c5bc-b451-7b51-a6ef-3742b71136bf; preview=" Preview 50 rows · 237 ms ID BIR… |
| 7.setup-query | pass | editor=codemirror/codemirror; waitingMsg=yes; paramField=true; chips=["SQL Query","active"]; saved=true QQ=01a1c5bc-d863-7451-af80-188e65d77d07; preview150=" Running que… |
| 7.setup-query-neg | pass | on-screen="The Library's SQL on FHIR type must be "sql-query" to save it here."; stored type code after attempt=sql-query |
| 7.a | pass | files=1 lines=11705 (expected 11705) keys=id,gender,birth_date,family,city finished-in="0m 00s" PID line={"id":"7d24f7a0-6f2e-ce3b-5568-db7b14695583","gender":"female","… |
| 7.a-viewfiles | pass | url=http://127.0.0.1:18080/ui/sql/export/c4fd354a-0928-4058-a61f-3d560cac49f6 cards=5 |
| 7.b | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries… |
| 7.c | pass | files=1 schema=[["id","string"],["birth_date","string"],["city","string"]] rows=5814 (expected 5814) |
| 7.d | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries… |
| 7.e | pass | before: In progress "Writing observation_flat · 0 of 1 subject (1 ViewDefinition) · NDJSON · started 13:55 UTC"; after: Cancelled "1 subject (1 ViewDefinition) · NDJSON … |
| 7.f | pass | nothing ticked -> "Could not start the export. Select at least one subject."; query w/o min_height -> "This value is required." |
| 7.m | pass | header="id,gender,birth_date,family,city" dataLines=11705 (expected 11705) PID line="7d24f7a0-6f2e-ce3b-5568-db7b14695583,female,2015-12-29,Parker433,Everett" |
| 7.n | pass | schema=[["id","string"],["gender","string"],["birth_date","string"],["family","string"],["city","string"]] rows=11705 (expected 11705) birth_date type=string |
| 7.o | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries… |
| 7.p | fail | card=failed/Failed warning="The export stopped on subject tall_female_patients: dependency 'obs' (ViewDefinition observation_flat) exceeds 1000000-row limit: SQL queries… |
| 7.q | pass | lines=5814 (expected 5814) keys=id,birth_date,city noGenderKey=true |
| 7.g | pass | lines=1 line={"id":"7d24f7a0-6f2e-ce3b-5568-db7b14695583","gender":"female","birth_date":"2015-12-29","family":"Parker433","city":"Everett"} equals Patient resource=true… |
| 7.h | pass | files=1 array=true len=1 firstId=7d24f7a0-6f2e-ce3b-5568-db7b14695583 JobCardGroups=["Group/manual-group"] |
| 7.i | pass | lines=11704 API _lastUpdated>=T3_START count=11704 (matrix 11,704 on full corpus) Job card Since="2026-11-09T14:00:46.469Z" |
| 7.j | pass | status=Complete files=0 outputsCount=0 Since="2026-11-09T15:16:36Z" |
| 7.k | pass | lines=5814 (expected 5814) firstLine="00114953-32b2-24e7-b63c-4df40d54c4e0,1954-06-14,Webster" Format="CSV · no header row" Tracking id="release-check-01" manifest-has-c… |
| 7.l | pass | all-three: submitted=false shown="Could not start the export. Enter only valid logical Patient IDs, separated by commas or new lines." hidden-in-DOM="Enter a valid FHIR … |
| 7.tbl-controls | fail | Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_patients"] hint="2 of 4 selected · 1 value missing"; after tick+filter obs hin… |
| 7.exp-as-files | fail | exception: locator.waitFor: Timeout 15000ms exceeded. |
| 7.life | pass | copy button text="Copied" clipboard=job id; run again -> 2 cards (new 454e306e-fb74-4109-a959-14e25b91d320 Complete), old c4fd354a-0928-4058-a61f-3d560cac49f6 still Comp… |
| 7.fail | pass | lint=Unknown table table_that_does_not_exist — line 1 saved=true; status=Failed; warning="The export stopped on subject broken_query: SQLite error: no such table: table_… |
| 7.big-json | pass | card=complete/Complete \| files=big-json_1.json,big-json_2.json,big-json_3.json,big-json_4.json,big-json_5.json,big-json_6.json,big-json_7.json,big-json_8.json,big-json_… |
| 7.restart | pass | card="Cancelled 1 subject (1 ViewDefinition) · JSON · cancelled at 14:00 UTC · the server no longer knows this job" detail lede="Cancelled 2026-11-10 14:00 UTC · JSON · … |
| 7n.setup-before | pass | un-narrowed COUNT over observation_flat (Observations on server=7699987) -> HTTP 422: {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"processing"… |
| 7n.setup | pass | PUT narrowed observation_flat (id 01a1c5bc-1e76-7df1-8b14-415f4bd27af4) -> HTTP 200; where "code.coding.first().code = '8302-2'"; codes ["8302-2"] parsed from the tall_f… |
| 7.b-narrow | pass | header="id,city,height" dataRows=4945 (ground truth ($sql-run of tall_female_patients, min_height=150, narrowed view) = 4945 rows) allHeight>150=true spot=00114953-32b2-… |
| 7.d-narrow | pass | files=3 all-three-json-narrow_patient_demographics.json:11705 all-three-json-narrow_tall_female_patients.json:4945 all-three-json-narrow_female_patients.json:5814; expec… |
| 7.o-narrow | pass | lines=4945 keys=id,city,height equals 7.b-narrow data lines (4945); ground truth ($sql-run of tall_female_patients, min_height=150, narrowed view) = 4945 rows |
| 7.p-narrow | pass | schema=[["id","string"],["city","string"],["height","string"]] rows=4945 equals 7.o-narrow lines (4945); ground truth ($sql-run of tall_female_patients, min_height=150, … |
| 7.tbl-controls-narrow | fail | Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_patients"] hint="2 of 4 selected · 1 value missing"; after tick+filter obs hin… |
| 7.exp-as-files-narrow | pass | confirm="You have unsaved changes. Discard them and leave this page?" url=http://127.0.0.1:18080/ui/sql/export/new?subject=Library/01a1c5bc-d863-7451-af80-188e65d77d07 q… |
| 7n.restore | pass | PUT original observation_flat (id 01a1c5bc-1e76-7df1-8b14-415f4bd27af4) -> HTTP 200; stored definition equals the saved original=true (where=null); versions in history=3… |
| 7.tbl-controls-narrow-rerun | pass | Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_patients"] hint="2 of 4 selected · 1 value missing"; after tick+filter obs hin… |

</details>

<details><summary>T8: 11 pass, 0 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 12.1 | pass | receiver listening on 127.0.0.1:9999 pid=2081555 log=/home/edson/tmp/939/work/webhook.log (expected: listening, empty log); fixture /home/edson/tmp/939/work/fixtures/enc… |
| 12.2.1 | pass | Unsaved pill before save=1 cleared=true; editor status=""; GET Basic/manual-topic -> 200 (expected 200, code SubscriptionTopic) |
| 12.2.2 | pass | Unsaved pill cleared=true; editor status=""; GET Subscription/manual-sub -> 200, endpoint=http://127.0.0.1:9999/webhook (expected http://127.0.0.1:9999/webhook) |
| 12.2.3 | pass | after 2899 ms: 1 log line(s), handshake lines=1, auth="Bearer manual-token" (expected 1 line, handshake, "Bearer manual-token") |
| 12.2.4 | pass | UI result row="manual-sub active 02:11 PM" (expected status column active); API ?_elements=status -> "active"; _history entries=2 (expected 2), statuses v1->v2=requested… |
| 12.3.1 | pass | request line="POST [base] · Bundle · batch · 3 entries"; created badge="3 created"; overall="200"; log lines=4 (expected 4); last auth="Bearer manual-token"; events-sinc… |
| 12.3.1-status | pass | GET Subscription/manual-sub/$status -> 200, status active=true, eventsSinceSubscriptionStart=3 (expected 3) |
| 12.3.1-events | pass | GET $events?eventsSinceNumber=1&eventsUntilNumber=3 -> 200, Bundle.type=history, entries=1 [SubscriptionStatus]; the matrix does not state an expected count, expected st… |
| 12.3.2 | pass | Condition saved (pill cleared=true, status=""); log lines before=4 after=4 (expected 4 and unchanged) |
| 12.3.3 | pass | receiver stopped=true; batch created="3 created"; log lines before=4, after 30000 ms with receiver down=4 (expected unchanged), after receiver restart +31624 ms=7 (expec… |
| 12.console | pass | no browser console errors during T8 (ignoring 2 editor id-probe 404/410 lines) |

</details>

<details><summary>T9: 15 pass, 0 fail, 0 n/a</summary>

| Row | Result | Observed |
|---|---|---|
| 13.1 | pass | cards: Active=1 (delivering); Failing=0 (Needs attention); Idle=0 (No clients); Delivered in 24 h=6 (83.3% first try); expected Active 1 (delivering), Failing 0 (Needs a… |
| 13.2 | pass | row: id=manual-sub topic="encounter-start" (title attr "http://example.org/topics/encounter-start") channel="rest-hook" endpoint="http://127.0.0.1:9999/webhook" status="… |
| 13.3 | pass | Delivered 6->9 (expected +3); Sent 6->9 (expected +3); sparkline points 48->48, tooltip "6"->"9" (a new point appears only when the current half-hour bucket is a new one… |
| 13.4a | pass | 30 s after upload, receiver down: status="Active" sent=12 streak=0 last24="9" spark=9/48pts alert=false; Failing card=0; "Connection failed" lines in HFS log=10; expecte… |
| 13.4b | pass | streak 1 first seen 5 min 16 s after upload (expected about 5 min, each right after a "Max retries exhausted" line; 15 s polling) |
| 13.4c | pass | streak 2 first seen 10 min 17 s after upload (expected about 10 min) |
| 13.4d | pass | Error chip after 15 min 19 s after upload (expected 15-17 min; window accepted 13-18 min); Failing card=1 (expected 1); status="Error" sent=12 streak=3 last24="9" spark=… |
| 13.4e | pass | status: label="Status" rows=1; sent: label="Most sent" rows=1; fails: label="Fail streak" rows=1 |
| 13.4f | pass | after the third window with the receiver back for 70 s: stored status=error (expected error), UI row "Error"; PUT status=requested -> HTTP 200, stored status active=true… |
| 13.5 | pass | restart cmd ok=true, server up=true; before: status="Active" sent=0 streak=0 last24="9" spark=9/48pts alert=false; after: status="Active" sent=0 streak=0 last24="0" spar… |
| 13.6 | pass | restart ok=true, up=true; notice="The subscriptions engine is not enabled on this server. Turn it on by starting HFS with: HFS_SUBSCRIPTIONS_ENABLED=true · --features su… |
| 13.6-restore | pass | engine restored with HFS_RESTART_CMD: up=true, page available=true |
| 13.7 | pass | no browser console errors on /ui/subscriptions and the Batch page (resource-load errors from the intentionally stopped server excluded) |
| 13.8 | pass | JS disabled: card values=[1,0,0,0] vs JS enabled=[1,0,0,0]; first row="manual-sub encounter-start rest-hook http://127.0.0.1:9999/webhook Active 0 0" |
| 13.9 | pass | /ui rendered (356 chars); server Encounter total=827980; stat cards text: Home RESOURCE TYPES — used for R4 STORED RESOURCES — across active tenant EXPORT JOBS — unavail… |

</details>

## Logs, outputs and how to reproduce

See `INVENTORY.md` for the file list. The Playwright and API scripts used for every step are under `scripts/` (`lib.mjs` plus `steps/t*.mjs`); the server start script with all `HFS_*` settings is `scripts/start-hfs.sh`.
