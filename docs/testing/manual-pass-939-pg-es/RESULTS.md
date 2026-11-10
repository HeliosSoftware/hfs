# Manual test pass #939: PostgreSQL + Elasticsearch (`pg-es`), anonymous access

Tester: Edson Terceros (run driven by Claude Code). Date: 2026-11-09 and 2026-11-10 (UTC). Commit: `main` at `f39f48276` (Release version 0.2.8), R4-only release build. Host: Linux (Proxmox LXC), 8 cores, 23 GB RAM. PostgreSQL 16 and Elasticsearch 8.15.0 (8 GB heap) in Docker, corpus served by nginx:alpine over HTTP/1.1. Evidence for issue #939; the procedure is `MANUAL_TESTING_MATRIX.md` sections 2 to 13.

## Result

All ten steps (T0 to T9) ran on the `pg-es` row. **167 checks: 165 pass, 2 n/a, 0 fail.** The two `n/a` are the R5 version switch of T1 (R4-only build) and the T9 main-dashboard check (captured right after a restart, with the cards still loading).

**Application findings:** one new issue, `$events` on R4 returns a body that is not an R4 resource, no events, and ignores its range parameters (#2547); and one already tracked issue showed up in the server log, the primary PostgreSQL is logged as unhealthy after reading a deleted resource (#2532, evidence added there). Nothing else needs to be reported.

The full 18,955,865-resource Synthea corpus imported through the Import page in **1 h 12 min 52 s** and was fully searchable **1 h 15 min 50 s** after the start; PostgreSQL and Elasticsearch counts equal the corpus for all 24 types.

## Deviations from the matrix

- **R4-only build.** `cargo build --release -p helios-hfs --features postgres,elasticsearch,subscriptions,xml` instead of `cargo build --workspace --all-features --release` (T0); the R5 version switch of T1 is `n/a`.
- **T7 rows on a narrowed view.** `tall_female_patients` depends on `observation_flat`, which has 7.7 M rows and exceeds the default 1,000,000-row source limit (`HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`, #1473, documented in matrix note 2). Rows 7.b, 7.d, 7.o, 7.p, 11.5 "all three subjects" and "Export as files" ran with `observation_flat` narrowed to the one Observation code the query uses (8302-2); the original definition was restored afterwards.
- **Bulk-submit indexing settings** of the previous campaign (`HFS_BULK_SUBMIT_INDEX_QUEUE=64`, `INDEX_CONCURRENCY=8`, `INDEX_COALESCE=4`, `FILE_CONCURRENCY=4`, `BATCH_SIZE=1000`), not the recipe in `crates/rest/README.md`.
- **Ports.** PostgreSQL 18932, Elasticsearch 18921, HFS 18080 (`HFS_BASE_URL=http://localhost:18080`), corpus server 18801.

## Matrix row (section 1 format)

| Backend | T0 | T1 | T2 | T3 | T4 | T5 | T6 | T7 | T8 | T9 |
|---|---|---|---|---|---|---|---|---|---|---|
| `pg-es` | ✅ (R4 only) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (6 rows on a narrowed view) | ✅ (`$events`, #2547) | ✅ |

## Results per step

| Step | Checks | Result | What was checked |
|---|---|---|---|
| T1 | 8 | 7 pass, 1 n/a | `/health` 200; `/metadata` `4.0.1` with `export`, `sql-run`, `sql-export`, `bulk-submit`; startup line `storage_backend=postgres-elasticsearch` and the index prefix; dashboard 1,377 stored resources (1,372 SearchParameter + 5 CompartmentDefinition); clinical rail at 0; sidebar on R4. |
| T2 | 10 | 10 pass | Whole step 0.8 min. The 662-entry transaction fails as a whole with a conditional-reference error and creates nothing; the batches create 9 and 8; the transaction then creates 662, giving the patient 49 Encounter, 106 Observation and 33 Condition with literal references that resolve; three rejected files with their exact messages. |
| T3 | 7 | 7 pass | 18,955,865 resources in 24 types, 24 output files, 0 error files. Ingest 1 h 12 min 52 s, fully searchable 1 h 15 min 50 s (about 4,330 and 4,170 resources/s). PostgreSQL and Elasticsearch counts equal the corpus for all 24 types. During the Observation file (HFS at 12.6 GB) the search index timed out after 30 s on 4,049 resources; the deferred reindex repaired all of them (4049 of 4049, `failed=0`). Database 94 GB, Elasticsearch 47.8 GB (35 indices) after the pass. |
| T4 | 33 | 33 pass | 29 search rows 4.1-4.29: string, token, date, number, quantity, reference, uri, composite, chained, `_has`, `_include`/`_revinclude`, `_sort`, paging, `_content`, POST search. 93 UI queries, all with `_total=accurate`, 0 page errors; every row compares the UI total with the API total. |
| T5 | 37 | 37 pass | System, patient and group `$export`; the three NDJSON spellings accepted and unsupported formats rejected with 400; cancel; `_since`/`_until` windows; a 12-file Patient export of 11,705 lines; a patient-level export of Patient+Condition in 489 files; the one-patient export holds exactly what the server stores for that patient. |
| T6 | 15 | 15 pass | `patient_demographics` and `observation_flat` in the UI and through `$sql-run` in json, csv, ndjson and parquet (`PAR1` magic checked); 50-row preview over `observation_flat` in 9 ms; duplicate gets a new canonical URL; delete confirmation. |
| T7 | 31 | 31 pass | Subjects ViewDefinition, SQLQuery and SQLView in ndjson, csv, json and parquet; `_since` filter (11,704 rows); cancel, retry, run again; a completed job is still downloadable after a restart; JSON export of `observation_flat` (7.7 M Observations) complete in 16 files (the first is 83.9 MB). Six rows on a narrowed view (see deviations): 4,945 rows for `min_height=150`. |
| T8 | 11 | 11 pass | Topic (`Basic`) and rest-hook Subscription; a 3-entry batch produces exactly 4 webhook log lines; `$status`; retry after the receiver returns. `$events` on R4: see #2547. |
| T9 | 15 | 14 pass, 1 n/a | Status cards, row and sparkline; failure path with the receiver down timed against the matrix: streak 1 at 5 min 16 s, streak 2 at 10 min 17 s, Error chip at 15 min 19 s (matrix: about 5, 10 and 15-17 min); sort menu; reactivation; restart with rehydration; engine-disabled notice. |

## Compared with the previous `pg-es` pass (comment on #939, 2026-10-02, build `e943ae3e7`)

| Item | Previous pass | This pass |
|---|---|---|
| Corpus ingest | 1 h 41 min stored, 1 h 45 min searchable, with `REFRESH_INTERVAL=30s` | 1 h 12 min 52 s stored, 1 h 15 min 50 s searchable, default refresh interval |
| T5 5.2 single-patient `$export` (#1663) | statement timeout | completes, content equals what the API stores |
| T6 preview over Observation (#1581), Duplicate (#1576) | failed on that build, not re-run | 50 rows in 9 ms; duplicates get a new canonical URL |
| T7 7.l text typed into Patients (#1575) | dropped, export ran unfiltered | rejected with "Enter only valid logical Patient IDs…" |
| T7 SQL Query over `observation_flat` (#1473) | row limit | same documented limit; passes on a narrowed view |

## Findings

| # | Where | Issue | Evidence |
|---|---|---|---|
| 1 | T8 12.3.1 | **#2547**: on R4, `GET /Subscription/<id>/$events?eventsSinceNumber=1&eventsUntilNumber=3` returns a `SubscriptionStatus` (not an R4 resource), no events, and ignores both range parameters; `$status` on the same subscription correctly returns `Parameters`. | `outputs/t8-12.3.1-events.json`, `crates/rest/src/handlers/subscriptions.rs` lines 149-197 |
| 2 | T6 6.3.4 | **#2532** (already open): `Backend marked unhealthy backend_id="postgres" failures=3` is logged when a deleted ViewDefinition is read (410 Gone). | `logs/hfs-pg-es.log` at 13:53:45Z and 13:53:47Z on 2026-11-10; comment added to #2532 |

Matrix text corrections that came out of this run are appended to `MANUAL_TESTING_MATRIX.md`, section "PostgreSQL + Elasticsearch — findings from the pg-es pass".

## Every check

<details><summary>T1: 8 checks, 7 pass, 1 n/a</summary>

| Check | Result | Observed |
|---|---|---|
| 5.1 | pass | GET /health -> 200 {"status":"ok","service":"hfs","version":"0.2.8","uptime_seconds":54.180939756,"started_at":"2026-11-09T13:57:46.9655329 (expected 200) |
| 5.2 | pass | GET /metadata -> 200, fhirVersion 4.0.1 (expected 4.0.1), software Helios FHIR Server, 147 resource types |
| 5.3 | pass | operations: bulk-submit bulk-submit-status export group-export patient-export reindex sql-export sql-run validate versions (expected at least export, sql-run, sql-export, bulk-submit; missing: none) |
| 5.5 | pass | dashboard Stored resources "1.4k" (expected ~1.4k), chart total "1,377" (expected 1,377); API SearchParameter 1372 (expected 1372) + CompartmentDefinition 5 (expected 5) = 1377 |
| 5.6 | pass | clinical rail {"Patient":"0","Encounter":"0","Observation":"0","Condition":"0","Organization":"0","Practitioner":"0","Location":"0"} (expected all 0); SearchParameter rail "1,372" (expected 1,372) |
| 5.7 | pass | sidebar FHIR selector reads "FHIR R4" (expected FHIR R4) |
| 5.8 | na | R4-only build: no R5 available, version switch not applicable |
| 5.4 | pass | startup line storage_backend=postgres-elasticsearch (pg-es = postgres-elasticsearch), index_prefix=hfs_20261109_1357 (matches the configured hfs_20261109_1357), Seeded SearchParameters created=1372 |

</details>

<details><summary>T2: 10 checks, 10 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 6.1 | pass | ok: upload of Nicky270_Ann985_Larkin917_b9e958ef-8c96-12b1-f629-d9536dcbdfb1.json moved to Execution Plan \| ok: request strip "POST [base] · Bundle · transaction · 662 entries" (expected "POST [base] · Bundle · transaction · 662… |
| 6.2a | pass | ok: strip "POST [base] · Bundle · batch · 9 entries" (expected "POST [base] · Bundle · batch · 9 entries") \| ok: notice "Batch: entries run independently — a failed entry does not stop or undo the others." \| ok: Actions follow … |
| 6.2b | pass | ok: strip "POST [base] · Bundle · batch · 8 entries" (expected "POST [base] · Bundle · batch · 8 entries") \| ok: notice "Batch: entries run independently — a failed entry does not stop or undo the others." \| ok: Actions follow … |
| 6.3a | pass | ok: Per-Action Outcomes: HTTP 200, head "662 created" (expected "662 created"), 662 rows, statuses {201 Created} (expected all "201 Created"), summary "" \| ok: Done returns to the Upload stage |
| 6.3b | pass | ok: GET /Patient?given=Nicky270&family=Larkin917&birthdate=1996-04-19 -> API 1 result, UI "1 results" (expected 1); LPID=01a1c09b-0d9e-7d81-8ab6-06881cea69e7 \| ok: modal subject line contains Patient/01a1c09b-0d9e-7d81-8ab6-0688… |
| 6.4a | pass | not-a-bundle.json -> "That JSON is not a FHIR Bundle." (expected "That JSON is not a FHIR Bundle."); page stays on Upload=true |
| 6.4b | pass | collection.json -> "Only Bundles of type batch or transaction can be executed here." (expected "Only Bundles of type batch or transaction can be executed here."); page stays on Upload=true |
| 6.4c | pass | not-json.log -> "That file is not valid JSON. (Unexpected token 'b', "build star"... is not valid JSON)" (expected "That file is not valid JSON. (<parser detail>)"); page stays on Upload=true |
| 6.2c | pass | the rail shows the same counts as the API (a leading ≈ is allowed by the matrix): {"Organization":"1140 (API 1140)","Location":"1142 (API 1142)","Practitioner":"1140 (API 1140)","PractitionerRole":"1140 (API 1140)"}. Right after … |
| 6.3c | pass | Encounter modal (editor scrolled end to end): serviceProvider Organization/01a1c09a-e164-76d1-99be-335081bf8826, participant.individual Practitioner/01a1c09b-047b-7332-a76e-4181af5d808c, subject Patient/01a1c09b-0d9e-7d81-8ab6-06… |

</details>

<details><summary>T3: 7 checks, 7 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 7.2 | pass | empty list="true", auth none=true, created id 43fde6d2-4b9c-483b-ae60-ed8567cc0a0a at 2026-11-09T14:00:46.469Z UTC (T3 start) |
| 7.3b | pass | status Completed=true; Output files 24; Error files 0; warnings: 4,049 resources flagged (re-index). Ingest done at 2026-11-09T15:13:38Z per the server log (1h 12m 52s after the 14:00:46Z start); deferred reindex finished 15:16:3… |
| 7.4 | pass | mode DEFER_INDEXING=false (index during ingest). Index rejected 4049 resources in 9 pages (reason: writing to the search index timed out after 30s) at the memory peak; the deferred reindex re-indexed them: outcome=completed, proc… |
| 7.5-counts-api | pass | 24 corpus types in HFS (API _summary=count) vs corpus-counts.tsv + the resources created by the T2 fixtures (all types): 0 mismatches . Detail in outputs/t3-pg-counts.tsv |
| 7.5-ui | pass | dashboard after import: Home RESOURCE TYPES 28 used for R4 STORED RESOURCES 19.0M across active tenant EXPORT JOBS 0 running (0 queued) IMPORT JOBS 0 active UPTIME 1h 54m since process start FHIR RESOURCES OVER TIME 11,534,3 \| r… |
| 7.5-es-counts | pass | Elasticsearch _count per type (filter is_deleted=false, must_not is_contained, index hfs_20261109_1357_default_<type>) equals corpus + T2 for all 24 types: 0 mismatches, including the 4049 resources the index rejected during inge… |
| 7.3a | pass | submission detail: Manifest URL, Data Recipient http://localhost:18080, Submission ID, Submitter, Status Completed, Authentication none, 24 output files, 0 error files, log lines (accepted 200, kick-off, finished cleanly, complet… |

</details>

<details><summary>T4: 33 checks, 33 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 8.1a | pass | ok: chip "No issues." (expected "No issues.") \| ok: Unsaved changes pill count 0 (expected 0) \| ok: status line "" \| ok: GET /RiskAssessment/manual-risk -> 200 |
| 8.1b | pass | ok: chip "No issues." (expected "No issues.") \| ok: Unsaved changes pill count 0 (expected 0) \| ok: status line "" \| ok: GET /ValueSet/manual-test-vs -> 200 |
| 4.1 | pass | ok: /Patient?family=Parker433 => ui 30, api 30 \| ok: /Patient?family:exact=Parker433 => ui 30, api 30 \| ok: /Patient?family:contains=arker43 => ui 30, api 30 \| ok: /Patient?name=cari853 => ui 3, api 3 \| ok: /Patient?name=Cari… |
| 4.1-post | pass | POST /Patient/_search family=Parker433 -> 200 total 30 vs GET 30 |
| 4.2 | pass | ok: /Patient?gender=female => ui 5,814, api 5,814 \| ok: /Patient?gender:not=female => ui 5,891, api 5,891 \| ok: /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999-33-3920 => ui 1, api 1 \| ok: /Observation?code=http://loin… |
| 4.3 | pass | ok: /Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01 => ui 1,268, api 1,268 \| ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&date=ge2016 => ui 24, api 24 \| ok: /Patient?_lastUpdated=ge2026-11-09 => ui 11,705,… |
| 4.4 | pass | ok: /RiskAssessment?probability=gt0.5 => ui 1, api 1 \| ok: /RiskAssessment?probability=lt0.5 => ui 0, api 0 \| ok: /RiskAssessment?probability=ap0.8 => ui 1, api 1 |
| 4.5 | pass | ok: /Observation?code=8302-2&value-quantity=gt150 => ui 146,589, api 146,589 \| ok: /Observation?code=8302-2&value-quantity=gt150\|\|cm => ui 146,589, api 146,589 \| ok: /Observation?code=8302-2&value-quantity=lt50\|http://unitso… |
| 4.6 | pass | ok: /Observation?subject=Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 165, api 165 \| ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 15, api 15 \| ok: /Encounter?subject=7d24f7a0-6f2e-ce3b-5568-db7b146955… |
| 4.7 | pass | ok: /ValueSet?url=http://example.org/fhir/ValueSet/manual-test => ui 1, api 1 \| ok: /ValueSet?url:below=http://example.org/fhir => ui 1, api 1 |
| 4.8 | pass | ok: /Observation?code-value-quantity=http://loinc.org\|8302-2$gt150 => ui 146,589, api 146,589 \| ok: composite total 146589 equals 4.5 first count 146589 \| ok: 100 rows all Body Height (8302-2) with value > 150 (violations 0) \… |
| 4.9 | pass | ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583,01a1c09b-0d9e-7d81-8ab6-06881cea69e7 => ui 2, api 2 |
| 4.10 | pass | ok: /Observation?subject.identifier=http://hl7.org/fhir/sid/us-ssn\|999-33-3920 => ui 165, api 165 \| ok: /Observation?subject:Patient.family=Parker433&_count=5 => ui 11,939, api 11,939 \| ok: 5 rows subject.reference in the 30 f… |
| 4.11 | pass | ok: /Patient?_has:Observation:patient:code=http://loinc.org\|8302-2&_count=5 => ui 11,705, api 11,705 \| ok: /Observation?patient=3dd31fcb-fd81-3a87-678e-b4c1128b4a23&code=8302-2 => ui 14, api 14 |
| 4.12 | pass | ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_revinclude=Condition:patient => ui 1 +15inc, api 1 +15inc \| ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_sort=-date&_count=5 => ui 165, api 165 \| ok: effe… |
| 4.13 | pass | ok: /Patient?_content=Everett => ui 163, api 163 \| ok: ES served it? log elasticsearch lines: none; ES search query_total delta 2 |
| 4.14 | pass | ok: QUERY box reads "GET /Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate" \| ok: Run: ui 5, api 5 Parker433 children, birth dates ascending: 2013-02-03,2015-11-17,2015-12-29,2019-05-06,2021-09-18 \| ok: Recent di… |
| 4.15 | pass | ok: /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999-19-2626 => ui 1, api 1 \| ok: /Patient?address-city=Millis => ui 19, api 19 \| ok: /Patient?address-city:exact=Millis => ui 13, api 13 \| ok: /Patient?family=Larkin917&g… |
| 4.16 | pass | ok: /Patient?_lastUpdated=lt2026-11-09T14:00:46.469Z => ui 1, api 1 \| ok: /Patient?_lastUpdated=ge2026-11-09T14:00:46.469Z => ui 11,704, api 11,704 |
| 4.17 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&class=EMER => ui 5, api 5 \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&class=IMP => ui 1, api 1 \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881… |
| 4.18 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&_include=Encounter:service-provider => ui 49 +4inc, api 49 +4inc \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&_include=Encounter:participant => ui 49 +… |
| 4.19 | pass | ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&service-provider.name=ENCOMPASS => ui 38, api 38 \| ok: /Encounter?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&participant.identifier=http://hl7.org/fhir/sid/us-npi\|99… |
| 4.20 | pass | ok: /Organization?name=TIMOTHY DANIELS HOUSE => ui 2, api 2 \| ok: /Organization?address-city=HOLLISTON => ui 2, api 2 \| ok: /Practitioner?identifier=http://hl7.org/fhir/sid/us-npi\|9999888693 => ui 2, api 2 \| ok: /Practitioner… |
| 4.21 | pass | ok: /Condition?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&clinical-status=active => ui 6, api 6 \| ok: /Condition?patient=01a1c09b-0d9e-7d81-8ab6-06881cea69e7&code=http://snomed.info/sct\|72892002 => ui 3, api 3 \| ok: /Observa… |
| 4.23 | pass | ok: /Patient?identifier=https://github.com/synthetichealth/synthea\|7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?phone=555-613-6236 => ui 1, api 1 \| ok: /Patient?birthdate=2015-12-29 => ui 2, api 2 \| ok: … |
| 4.24 | pass | ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&class=AMB => ui 23, api 23 \| ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&class=EMER => ui 1, api 1 \| ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7… |
| 4.25 | pass | ok: /Encounter?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_include=Encounter:service-provider => ui 24, api 24 \| ok: serviceProvider.reference left as conditional string (e.g. Organization?identifier=https://github.com/synthet… |
| 4.26 | pass | ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&clinical-status=active => ui 1, api 1 \| ok: /Condition?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&code=http://snomed.info/sct\|65363002 => ui 2, api 2 \| ok: /Immuniz… |
| 4.27 | pass | ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&code=8302-2&_sort=date&_count=20 => ui 15, api 15 \| ok: /Observation?patient=7d24f7a0-6f2e-ce3b-5568-db7b14695583&code=8302-2&value-quantity=gt120 => ui 4, api 4 \| o… |
| 4.28 | pass | ok: /Organization?identifier=https://github.com/synthetichealth/synthea\|e2a8b444-9b8f-36ff-84c4-05ee98589482 => ui 1, api 1 \| ok: /Organization?name=WHITLEY WELLNESS => ui 1, api 1 \| ok: /Location?address-city=Fitchburg => ui … |
| 4.29 | pass | ok: /Patient?_has:Condition:patient:code=http://snomed.info/sct\|65363002&_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583 => ui 1, api 1 \| ok: /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_revinclude=Immunization:patient => ui 1 +… |
| 4.pc | pass | 93 UI queries sent; 0 without _total=accurate; last: http://127.0.0.1:18080/Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583&_revinclude=Encounter:patient&_revinclude=Procedure:patient&_total=accurate; page errors: 0 |
| 4.22 | pass | GET /Patient?_has:Condition:patient:code=http://snomed.info/sct\|706893006 total=5294 (ground truth 5,294 = 5,293 corpus patients + the T2 patient; PostgreSQL SQL and Condition.ndjson agree); GET /Patient?_id=01a1c09b-0d9e-7d81-8… |

</details>

<details><summary>T5: 37 checks, 37 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 5.0-group-fixture | pass | GET Group/manual-group -> 200, member=Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583, Unsaved pill visible=false |
| 5.6 | pass | message visible=true text="Enter a name for this export." url=http://127.0.0.1:18080/ui/bulk-export/new (expected "Enter a name for this export." on /new) |
| 5.6-nojs | pass | no-JS POST -> 400, message present=true (expected 400 + message) |
| 5.0-clear | pass | after Clear: allResources=false anyType=false anyDisabled=false name="clear-test" elements="id,gender" typeUsable=true |
| 5.0-clear-after-reject | pass | rejection visible before Clear=true, after Clear=true; allResources=false anyType=false name="reject-then-clear" customInstant="2026-02-30T00:00:00Z" typeUsable=true |
| 5.0-nojs-clear-hidden | pass | Clear hidden without JS=true, still hidden on the rejected page=true; POST 400 message present=true; name kept=true; All Resources unchecked manually=true |
| 5.2-picker | pass | Parker433: first page 8 options, footer "38 matches · scroll for more", after scroll 16 options, footer "38 matches · scroll for more" (API name total 38); Cari853: 3 options, footer "" (API name total 3); matrix (38 matches, 8 t… |
| 5.1-zip | pass | suggested=bulk-export.zip bytes=3198174 magic=PK entries=6 (expected 6 NDJSON files) |
| 5.3 | pass | status=complete; files=2 (expect 2); Patient lines=1; Condition lines=1 (API active=1, all conditions=15); allActive=true allBelongToPID=true |
| 5.4 | pass | status=complete; files=12 (stored 11705 Patients -> expect 12); lines=11705; only id/gender/meta keys=true; meta.tag SUBSETTED on all=true; matrix 12 files: true |
| 5.5 | pass | after Cancel chip status=cancelled text="cancel-me Cancelled Run again Delete Delete cancel-me and its output files from the server? This cannot be undone. Delet" (expected cancelled) |
| 5.5-delete | pass | card removed=true; warning="Delete cancel-me and its output files from the server? This cannot be undone." |
| 5.7 | pass | status=complete; Patient lines=11704 vs API _lastUpdated>=T3_START 11704; files=12; window line has "Since 2026-11-09T14:00:46.469Z"=true; matrix 11704 lines/12 files: true |
| 5.8 | pass | status=complete; Patient lines=1 vs API _lastUpdated<=T3_START 1; window "Until 2026-11-09T14:00:46.469Z"=true; families=["Larkin917"]; matrix 1 Larkin917: true |
| 5.9 | pass | status=complete; Patient lines=11704 vs API window 11704; window line has both instants=true; text="since-until Complete View files Run again Delete Delete since-until and its output files from the server? This cannot be undone. … |
| 5.10 | pass | status=complete; lines {"Group":1,"RiskAssessment":1,"ValueSet":1} vs API {"RiskAssessment":1,"ValueSet":1,"Group":1}; ids {"Group":["manual-group"],"RiskAssessment":["manual-risk"],"ValueSet":["manual-test-vs"]}; Patient pill ab… |
| 5.11 | pass | status=complete; window instant=2026-11-09T13:51:44Z (0.0 min from now-24h); Organization lines=1140 vs API last-24h 1140; files=2; card="2 files" |
| 5.12 | pass | error visible=true text="Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z." stayedOnForm=true; after preset->All time field disabled=true, submit started an export=true |
| 5.12-nojs | pass | no-JS POST -> 400, message present=true |
| 5.13 | pass | first card status=failed text="bad-group Failed Retry Delete Delete bad-group and its output files from the server? This cannot be undone. Delete export Keep export kick-off answered 404: Could not find the resource &#39;Group/do… |
| 5.13-delete | pass | both cards deleted: first=true retry=true |
| 5.A1-system | pass | system $export _type=Organization: manifest 2 file(s) [Organization:1000, Organization:140] requiresAccessToken=false; downloaded 1140 lines vs stored 1140; content-type=application/fhir+ndjson |
| 5.A2-patient | pass | patient $export _type=Patient,Condition: manifest 489 file(s) [Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condition:1000, Condit… |
| 5.A3-group | pass | group $export manual-group _type=Patient,Condition: manifest 2 file(s) [Condition:15, Patient:1] requiresAccessToken=false; Patient 1 (expect 1), Condition 15 vs stored 15, all belong to PID=true |
| 5.A4-format-application/fhir+ndjson | pass | _outputFormat=application/fhir+ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 202 and a complete NDJSON manifest) |
| 5.A4-format-application/ndjson | pass | _outputFormat=application/ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 202 and a complete NDJSON manifest) |
| 5.A4-format-ndjson | pass | _outputFormat=ndjson: kick-off 202; manifest 200, first file Organization:1000; downloaded 1000 lines, content-type application/fhir+ndjson (expected 202 and a complete NDJSON manifest) |
| 5.A5-bad-format-application/fhir+json | pass | _outputFormat=application/fhir+json -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat 'application/fhir+json'"}}]} (expected 400 OperationOutcome … |
| 5.A5-bad-format-text/csv | pass | _outputFormat=text/csv -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat 'text/csv'"}}]} (expected 400 OperationOutcome "unsupported _outputFormat… |
| 5.A5-bad-format-parquet | pass | _outputFormat=parquet -> 400 {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid","details":{"text":"unsupported _outputFormat 'parquet'"}}]} (expected 400 OperationOutcome "unsupported _outputFormat '… |
| 5.A5-no-prefer | pass | no Prefer: respond-async -> 400  |
| 5.A5-until-before-since | pass | _until < _since -> 400 (expected 400) |
| 5.A6-bad-group | pass | kick-off 404 (the UI row 5.13 shows Failed; the API accepted-or-rejected outcome is recorded here) |
| 5.A7-since-window | pass | window 2026-11-09T14:00:46.469Z..2026-11-09T15:16:36Z: 11704 Patient lines vs API 11704 |
| 5.A8-cancel | pass | DELETE status -> 202 (expect 202); GET after -> 404 (expect 404) |
| 5.1 | pass | status=complete; card="6 files"; files=6 (server-count expects 6); lines {"Location":1142,"Organization":1140,"Practitioner":1140} vs stored {"Organization":1140,"Practitioner":1140,"Location":1142}; wellFormed=true; matrix(6 fil… |
| 5.2 | pass | one-patient export of the anchor patient (picked by id 7d24f7a0-6f2e-ce3b-5568-db7b14695583): Patient 1, Condition 16, Observation 165; stored for the patient: Condition 16, Observation 165; matrix 15 Condition + 165 Observation,… |

</details>

<details><summary>T6: 15 checks, 15 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 6.1.1-create-new | pass | title="New View Definition" starter name="new_view" (expected title "New View Definition" and a starter document) |
| 6.1.2-3-paste-and-results | pass | meta="50 rows · 9 ms" (expect "50 rows · N ms", stored Patients 11705, capped at 50); columns=id,gender,birth_date,family,city; table rows=50; chip="No issues." (expect "No issues.") |
| 6.1.4-lint | pass | chip="2 issues"; tooltip="Unknown key "colum"Rename to "column"Remove "colum"unknown-key"; panel has Unknown key=true / select-needs-column=true; quick-fix menu="Unknown key "colum"Rename to "column"Remove "colum""; after fix chi… |
| 6.1.5-completion | pass | getRes options [getResourceKeygetResourceKey()] has getResourceKey=true; accepted -> inserted "getResourceKey()"=true; object-position keys [resourcestring · required \| approvalDatestring \| authorother \| constantobject[] \| co… |
| 6.1.6-save | pass | VD=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e url=http://127.0.0.1:18080/ui/sql/view-definitions?vd=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e&saved=1&return_to=%2Fui%2Fsql%2Fview-definitions "Saved."=true; rail="patient_demographics Patient… |
| 6.1.7-crosscheck | pass | row 01a1c09b-0d9e-7d81-8ab6-06881cea69e7 \| female \| 1996-04-19 \| Larkin917 \| Millis vs GET /Patient?_id=01a1c09b-0d9e-7d81-8ab6-06881cea69e7: gender=female birthDate=1996-04-19 family=Larkin917 city=Millis |
| 6.2-observation_flat | pass | meta="50 rows · 9 ms" (expect 50 rows); columns=id,patient_id,code,value,effective; sample codes=4548-4,8302-2,72514-3,29463-7,39156-5 (matrix: codes such as 8302-2; present=true); VD2=01a1c5bc-1e76-7df1-8b14-415f4bd27af4; saved … |
| 6.A1-sql-run-patient_demographics | pass | patient_demographics: ndjson 200 50 rows ct=application/x-ndjson; json 200 50 rows; csv 200 header="id,gender,birth_date,family,city" 50 rows; parquet 200 5519 bytes head=PAR1 tail=PAR1 ct=application/vnd.apache.parquet; _format=… |
| 6.A2-sql-run-observation_flat | pass | observation_flat: ndjson 200 50 rows ct=application/x-ndjson; json 200 50 rows; csv 200 header="id,patient_id,code,value,effective" 50 rows; parquet 200 3906 bytes head=PAR1 tail=PAR1 ct=application/vnd.apache.parquet; _format=xm… |
| 6.A3-api-create-roundtrip | pass | ViewDefinition?name=patient_demographics total=1, name=observation_flat total=1 (expect 1 each) |
| 6.3.1-filter | pass | rail after filter "patient": [patient_demographics] (expected only patient_demographics) |
| 6.3.2-duplicate | pass | copy1 patient_demographics_copy http://example.org/ViewDefinition/patient_demographics_copy; copy2 patient_demographics_copy_2 http://example.org/ViewDefinition/patient_demographics_copy_2; original url http://example.org/ViewDef… |
| 6.3.3-dependent-sql-view | pass | Reads-from link to original=/ui/sql/view-definitions?vd=01a1c5bc-0cf9-7342-8acf-e9ce61ee321e&return_to=%2Fui%2Fsql%2Fviews%3Flib%3D01a1c5bc-498c-7b01-8e91-3da114573749; copy rows after where=false: 0 (UI meta "0 rows · 1 ms"); de… |
| 6.3.4-delete | pass | copy1: confirm text shown=true deleted=true; copy2: confirm shown=true deleted=true; rail now [nourl_vd, nourl_vd_copy, observation_flat, patient_demographics] |
| 6.3.5-negative-resource-nope | pass | chip="1 issue"; notice="Could not run the view. $sql-run returned 422 Unprocessable Entity: unknown resource type "Nope""; meta="last successful run"; save prompt shown=true; new_view count before=0 after cancel=0 |

</details>

<details><summary>T7: 31 checks, 31 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 7.setup-vd | pass | patient_demographics exists (01a1c5bc-0cf9-7342-8acf-e9ce61ee321e); observation_flat exists (01a1c5bc-1e76-7df1-8b14-415f4bd27af4) |
| 7.setup-view | pass | editor=codemirror/codemirror; saved=true notice="Saved." rail="female_patients active" QV=01a1c5bc-b451-7b51-a6ef-3742b71136bf; preview=" Preview 50 rows · 237 ms ID BIRTH_DATE CITY 01a1c09b-0d9e-7d81-8ab6-06881cea69e7 1996-04-19… |
| 7.setup-query | pass | editor=codemirror/codemirror; waitingMsg=yes; paramField=true; chips=["SQL Query","active"]; saved=true QQ=01a1c5bc-d863-7451-af80-188e65d77d07; preview150=" Running query…" |
| 7.setup-query-neg | pass | on-screen="The Library's SQL on FHIR type must be "sql-query" to save it here."; stored type code after attempt=sql-query |
| 7.a | pass | files=1 lines=11705 (expected 11705) keys=id,gender,birth_date,family,city finished-in="0m 00s" PID line={"id":"7d24f7a0-6f2e-ce3b-5568-db7b14695583","gender":"female","birth_date":"2015-12-29","family":"Parker433","city":"Everet… |
| 7.a-viewfiles | pass | url=http://127.0.0.1:18080/ui/sql/export/c4fd354a-0928-4058-a61f-3d560cac49f6 cards=5 |
| 7.c | pass | files=1 schema=[["id","string"],["birth_date","string"],["city","string"]] rows=5814 (expected 5814) |
| 7.e | pass | before: In progress "Writing observation_flat · 0 of 1 subject (1 ViewDefinition) · NDJSON · started 13:55 UTC"; after: Cancelled "1 subject (1 ViewDefinition) · NDJSON · cancelled at 13:55 UTC" |
| 7.f | pass | nothing ticked -> "Could not start the export. Select at least one subject."; query w/o min_height -> "This value is required." |
| 7.m | pass | header="id,gender,birth_date,family,city" dataLines=11705 (expected 11705) PID line="7d24f7a0-6f2e-ce3b-5568-db7b14695583,female,2015-12-29,Parker433,Everett" |
| 7.n | pass | schema=[["id","string"],["gender","string"],["birth_date","string"],["family","string"],["city","string"]] rows=11705 (expected 11705) birth_date type=string |
| 7.q | pass | lines=5814 (expected 5814) keys=id,birth_date,city noGenderKey=true |
| 7.g | pass | lines=1 line={"id":"7d24f7a0-6f2e-ce3b-5568-db7b14695583","gender":"female","birth_date":"2015-12-29","family":"Parker433","city":"Everett"} equals Patient resource=true matrixAnchorValues=true JobCardPatients=["Patient/7d24f7a0-… |
| 7.h | pass | files=1 array=true len=1 firstId=7d24f7a0-6f2e-ce3b-5568-db7b14695583 JobCardGroups=["Group/manual-group"] |
| 7.i | pass | lines=11704 API _lastUpdated>=T3_START count=11704 (matrix 11,704 on full corpus) Job card Since="2026-11-09T14:00:46.469Z" |
| 7.j | pass | status=Complete files=0 outputsCount=0 Since="2026-11-09T15:16:36Z" |
| 7.k | pass | lines=5814 (expected 5814) firstLine="00114953-32b2-24e7-b63c-4df40d54c4e0,1954-06-14,Webster" Format="CSV · no header row" Tracking id="release-check-01" manifest-has-clientTrackingId=true |
| 7.l | pass | all-three: submitted=false shown="Could not start the export. Enter only valid logical Patient IDs, separated by commas or new lines." hidden-in-DOM="Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z. \| Tracking id must b… |
| 7.life | pass | copy button text="Copied" clipboard=job id; run again -> 2 cards (new 454e306e-fb74-4109-a959-14e25b91d320 Complete), old c4fd354a-0928-4058-a61f-3d560cac49f6 still Complete; after remove -> 1 card(s) |
| 7.fail | pass | lint=Unknown table table_that_does_not_exist — line 1 saved=true; status=Failed; warning="The export stopped on subject broken_query: SQLite error: no such table: table_that_does_not_exist."; retry -> new 75a26e5b-a07a-462a-ae1d-… |
| 7.big-json | pass | card=complete/Complete \| files=big-json_1.json,big-json_2.json,big-json_3.json,big-json_4.json,big-json_5.json,big-json_6.json,big-json_7.json,big-json_8.json,big-json_9.json,big-json_10.json,big-json_11.json,big-json_12.json,bi… |
| 7.restart | pass | card="Cancelled 1 subject (1 ViewDefinition) · JSON · cancelled at 14:00 UTC · the server no longer knows this job" detail lede="Cancelled 2026-11-10 14:00 UTC · JSON · the server no longer knows this job" complete-card download … |
| 7n.setup-before | pass | un-narrowed COUNT over observation_flat (Observations on server=7699987) -> HTTP 422: {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"processing","details":{"text":"dependency 'obs' (ViewDefinition observa… |
| 7n.setup | pass | PUT narrowed observation_flat (id 01a1c5bc-1e76-7df1-8b14-415f4bd27af4) -> HTTP 200; where "code.coding.first().code = '8302-2'"; codes ["8302-2"] parsed from the tall_female_patients SQL (obs.code); narrowed dependency materiali… |
| 7.b | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] header="id,city,height" dataRows=4945 (ground truth ($sql-run of tall_female_patients, min_h… |
| 7.d | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] files=3 all-three-json-narrow_patient_demographics.json:11705 all-three-json-narrow_tall_fem… |
| 7.o | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] lines=4945 keys=id,city,height equals 7.b-narrow data lines (4945); ground truth ($sql-run o… |
| 7.p | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] schema=[["id","string"],["city","string"],["height","string"]] rows=4945 equals 7.o-narrow l… |
| 7.exp-as-files | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] confirm="You have unsaved changes. Discard them and leave this page?" url=http://127.0.0.1:1… |
| 7n.restore | pass | PUT original observation_flat (id 01a1c5bc-1e76-7df1-8b14-415f4bd27af4) -> HTTP 200; stored definition equals the saved original=true (where=null); versions in history=3; the un-narrowed view is back for later tests |
| 7.tbl-controls | pass | [run on observation_flat narrowed to code 8302-2, because the query on the full view stops at the documented 1,000,000-row limit, #1473] Queries->["tall_female_patients"] All->4/4; filter female->["tall_female_patients","female_p… |

</details>

<details><summary>T8: 11 checks, 11 pass</summary>

| Check | Result | Observed |
|---|---|---|
| 12.1 | pass | receiver listening on 127.0.0.1:9999 pid=2081555 log=/home/edson/tmp/939/work/webhook.log (expected: listening, empty log); fixture /home/edson/tmp/939/work/fixtures/encounters.json |
| 12.2.1 | pass | Unsaved pill before save=1 cleared=true; editor status=""; GET Basic/manual-topic -> 200 (expected 200, code SubscriptionTopic) |
| 12.2.2 | pass | Unsaved pill cleared=true; editor status=""; GET Subscription/manual-sub -> 200, endpoint=http://127.0.0.1:9999/webhook (expected http://127.0.0.1:9999/webhook) |
| 12.2.3 | pass | after 2899 ms: 1 log line(s), handshake lines=1, auth="Bearer manual-token" (expected 1 line, handshake, "Bearer manual-token") |
| 12.2.4 | pass | UI result row="manual-sub active 02:11 PM" (expected status column active); API ?_elements=status -> "active"; _history entries=2 (expected 2), statuses v1->v2=requested->active; UI History tab version rows=2 (expected 2); diff m… |
| 12.3.1 | pass | request line="POST [base] · Bundle · batch · 3 entries"; created badge="3 created"; overall="200"; log lines=4 (expected 4); last auth="Bearer manual-token"; events-since-subscription-start on last line=3 (expected 3); id-only pa… |
| 12.3.1-status | pass | GET Subscription/manual-sub/$status -> 200, status active=true, eventsSinceSubscriptionStart=3 (expected 3) |
| 12.3.1-events | pass | GET $events?eventsSinceNumber=1&eventsUntilNumber=3 -> 200, Bundle.type=history, entries=1 [SubscriptionStatus]; the matrix does not state an expected count, expected status 200 with a history Bundle whose first entry is the Subs… |
| 12.3.2 | pass | Condition saved (pill cleared=true, status=""); log lines before=4 after=4 (expected 4 and unchanged) |
| 12.3.3 | pass | receiver stopped=true; batch created="3 created"; log lines before=4, after 30000 ms with receiver down=4 (expected unchanged), after receiver restart +31624 ms=7 (expected 7); added events-since counters=[4,5,6] auth all Bearer … |
| 12.console | pass | no browser console errors during T8 (ignoring 2 editor id-probe 404/410 lines) |

</details>

<details><summary>T9: 15 checks, 14 pass, 1 n/a</summary>

| Check | Result | Observed |
|---|---|---|
| 13.1 | pass | cards: Active=1 (delivering); Failing=0 (Needs attention); Idle=0 (No clients); Delivered in 24 h=6 (83.3% first try); expected Active 1 (delivering), Failing 0 (Needs attention), Idle 0 (No clients), Delivered = 6 event lines in… |
| 13.2 | pass | row: id=manual-sub topic="encounter-start" (title attr "http://example.org/topics/encounter-start") channel="rest-hook" endpoint="http://127.0.0.1:9999/webhook" status="Active" sent=6 streak=0 last24="6" spark=6/48pts alert=false… |
| 13.3 | pass | Delivered 6->9 (expected +3); Sent 6->9 (expected +3); sparkline points 48->48, tooltip "6"->"9" (a new point appears only when the current half-hour bucket is a new one; same bucket just raises its value) |
| 13.4a | pass | 30 s after upload, receiver down: status="Active" sent=12 streak=0 last24="9" spark=9/48pts alert=false; Failing card=0; "Connection failed" lines in HFS log=10; expected Active, Failing 0, streak 0 |
| 13.4b | pass | streak 1 first seen 5 min 16 s after upload (expected about 5 min, each right after a "Max retries exhausted" line; 15 s polling) |
| 13.4c | pass | streak 2 first seen 10 min 17 s after upload (expected about 10 min) |
| 13.4d | pass | Error chip after 15 min 19 s after upload (expected 15-17 min; window accepted 13-18 min); Failing card=1 (expected 1); status="Error" sent=12 streak=3 last24="9" spark=9/48pts alert=true (expected streak 3 and highlighted row); … |
| 13.4e | pass | status: label="Status" rows=1; sent: label="Most sent" rows=1; fails: label="Fail streak" rows=1 |
| 13.4f | pass | after the third window with the receiver back for 70 s: stored status=error (expected error), UI row "Error"; PUT status=requested -> HTTP 200, stored status active=true; UI after: status="Active" sent=0 streak=0 last24="9" spark… |
| 13.5 | pass | restart cmd ok=true, server up=true; before: status="Active" sent=0 streak=0 last24="9" spark=9/48pts alert=false; after: status="Active" sent=0 streak=0 last24="0" spark=0/48pts alert=false; Delivered card=0 (expected 0); Active… |
| 13.6 | pass | restart ok=true, up=true; notice="The subscriptions engine is not enabled on this server. Turn it on by starting HFS with: HFS_SUBSCRIPTIONS_ENABLED=true · --features subscriptions"; sidebar entry present=true; table rows=0; expe… |
| 13.6-restore | pass | engine restored with HFS_RESTART_CMD: up=true, page available=true |
| 13.7 | pass | no browser console errors on /ui/subscriptions and the Batch page (resource-load errors from the intentionally stopped server excluded) |
| 13.8 | pass | JS disabled: card values=[1,0,0,0] vs JS enabled=[1,0,0,0]; first row="manual-sub encounter-start rest-hook http://127.0.0.1:9999/webhook Active 0 0" |
| 13.9 | na | dashboard captured 9 s after the restart of 13.6-restore, stat cards still loading ("—"): not a meaningful check; the cards were not re-read after T2-T8. Original note: /ui rendered (356 chars); server Encounter total=827980; sta… |

</details>

## Evidence

See `INVENTORY.md`. The Playwright and API scripts of every step are under `scripts/` (`lib.mjs`, `steps/t*.mjs`); the server start script with all `HFS_*` settings is `scripts/start-hfs.sh`.
