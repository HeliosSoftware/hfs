# HFS Manual Testing Matrix

This document is the manual, end-to-end acceptance pass for the `hfs` binary. It is
organised **backend-first**: every storage backend gets the same sequence of test
procedures, and the results are recorded in the matrix at the top.

Everything below is executed against a build produced exactly the way `ci.yml`
builds it: all FHIR versions, all databases, all features (`--all-features`).

From T2 onward every step is performed **by hand in the web UI** (`/ui`). The tester
does not call the FHIR API with `curl` or any other client; the only command-line
work is downloading and unpacking the test data, serving it over HTTP, and running
the tiny webhook receiver that the subscription test needs.

Legend for result cells: `☐` not run · `✅` pass · `❌` fail (link the issue) ·
`N/A` not supported on this backend (expected, see [Expected support](#expected-support-by-backend)).

---

## 1. The matrix

Fill one row per backend per release candidate. Copy this table into the release
issue and replace the `☐` cells.

| Backend (`HFS_STORAGE_BACKEND`) | T0 Build | T1 Start | T2 Batch / Transaction | T3 Bulk import | T4 Search types | T5 Bulk export | T6 ViewDefinition | T7 SQL export (VD / query / view) | T8 Subscription | T9 Activity dashboard |
|---|---|---|---|---|---|---|---|---|---|---|
| `sqlite` | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `sqlite-es` (SQLite + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `postgres` | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `pg-es` (PostgreSQL + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `mongodb` | ☐ | ☐ | ☐ | ☐ | ☐ (4.10, 4.11 N/A) | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `mongo-es` (MongoDB + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `s3` (MinIO) | ☐ | ☐ | ☐ (batch only) | ☐ | N/A (no search) | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `s3-es` (MinIO + Elasticsearch) | ☐ | ☐ | ☐ (batch only) | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |

Tester: ______  Commit: ______  Date: ______  OS/arch: ______

### Expected support by backend

Derived from the project skills and the backend capability traits. A cell marked
`N/A` above is a *documented* gap. If a backend that should work returns an error,
that is a failure.

| Capability | sqlite | sqlite-es | postgres | pg-es | mongodb | mongo-es | s3 | s3-es |
|---|---|---|---|---|---|---|---|---|
| CRUD, history | yes | yes | yes | yes | yes | yes | yes | yes |
| Search | yes | yes (ES) | yes | yes (ES) | yes | yes (ES) | **no** | yes (ES) |
| Chained and `_has` search | yes | yes | yes | yes | **no** | yes | no | yes |
| Transaction Bundles | yes | yes | yes | yes | yes | yes | **no** (batch only) | **no** (batch only) |
| Bulk Data `$export` (job store) | yes | yes | yes | yes | no (501) | no (501) | no (501) | no (501) |
| `$bulk-submit` ingestion (Import page) | yes | yes | yes | yes | yes | yes | yes¹ | yes¹ |
| `$sql-run` / `$sql-export` runner | in-DB | in-DB (primary) | in-DB | in-DB (primary) | in-DB (aggregation) | in-DB (primary) | in-process scan | in-process scan |
| Subscriptions engine | yes | yes | yes | yes | yes | yes | yes | yes |
| `$reindex` | yes | yes | yes | yes | yes | yes | no (501) | yes |
| Per-user UI settings (saved queries, export job lists) | yes | yes | yes | yes | yes | yes | yes¹ | yes¹ |

¹ S3 in prefix-per-tenant mode (the default, `HFS_S3_BUCKET`). Bucket-per-tenant
mode with no system bucket returns `501` for `$bulk-submit` and user settings.

The `near` (geo) search parameter is not implemented on any backend, so it is not
part of T4.

---

## 2. Prerequisites

| Tool | Why |
|---|---|
| Rust 1.90+ (edition 2024), `cargo` | build |
| Python 3 with dev headers, `maturin` not required | `--workspace` includes `pysof` (PyO3 cdylib); the build needs a Python interpreter on `PATH` |
| Docker | Postgres, Elasticsearch, MongoDB, MinIO |
| `curl`, `jq` | T1 smoke check only; `jq` optionally trims the import manifest in T3 |
| `tar`, `python3` | unpack the corpora; `python3 -m http.server` serves the corpus to the Import page and runs the webhook receiver in T8 |
| ~45 GB free disk | corpus (3.6 GB tar.gz, 35 GB extracted) plus SQLite/Postgres data |
| A modern browser with JavaScript on | every step from T2 on runs in `/ui`; the Batch / Transaction page needs JavaScript |

Shell conventions used below:

```bash
export HFS=http://localhost:8080          # HFS base URL
export WORK=$PWD/manual-test               # scratch dir for corpora, fixtures, logs
mkdir -p "$WORK/fixtures"
```

All requests go to the default tenant (`HFS_DEFAULT_TENANT=default`); the sidebar
tenant selector stays on `default`. Authentication stays disabled for this pass.

Test data used from T2 on:

| Archive | Contents | Used in |
|---|---|---|
| <https://hfs-manual-test.s3.us-east-1.amazonaws.com/fhir2.tar.gz> (3.6 GB) | Synthea R4 corpus as NDJSON: 24 files, 18,955,865 resources for 11,704 Massachusetts patients, plus a Bulk Data `manifest.json` | T3 |
| <https://hfs-manual-test.s3.us-east-1.amazonaws.com/fhir-batch-import.tar.gz> (187 KB) | Three Synthea Bundles: `hospitalInformation…json` (batch, 9 entries), `practitionerInformation…json` (batch, 8 entries), `Nicky270_Ann985_Larkin917_…json` (transaction, 662 entries) | T2 |

One patient from the corpus is used as the anchor for T4–T8. Its id is stable
because the bulk import preserves resource ids:

| | |
|---|---|
| `PID` | `7d24f7a0-6f2e-ce3b-5568-db7b14695583` |
| Name | Cari853 Esperanza675 **Parker433**, female, born 2015-12-29 |
| Address | Everett, MA 02149 |
| SSN identifier | `http://hl7.org/fhir/sid/us-ssn` \| `999-33-3920` |
| Corpus rows | 24 Encounters, 165 Observations (15 body-height), 15 Conditions, 18 Procedures |

---

## 3. T0 — Build (the full CI build)

`ci.yml` tests with `cargo test --workspace --all-features` and releases with
`cargo build --workspace --all-features --release`. Use the release form so the
import and export timings are representative.

```bash
cd /path/to/hfs
git status --short | grep -v 'crates/fhir/tests/data' # working tree should be clean apart from R6 fixture churn
cargo build --workspace --all-features --release 2>&1 | tee "$WORK/build.log"
./target/release/hfs --help | head -5
```

`--all-features` on `helios-hfs` enables: `R4,R4B,R5,R6`, `sqlite,postgres,mongodb,
elasticsearch,s3`, `ui`, `subscriptions`, `cloudwatch`, `otel`. The R6 spec files are
downloaded on first build; the build also rewrites the checked-in R6 fixture files
under `crates/fhir/tests/data` — do not commit those.

If Python is unavailable on the machine, build the default members instead and note
the deviation in the results: `cargo build --all-features --release` (skips `pysof`).

Pass criteria: build exits 0; `hfs --help` prints usage.

---

## 4. Backend infrastructure

Start only what the row under test needs. Ports below are the ones the start
commands in section 5 assume. These match the images CI uses.

```bash
# PostgreSQL 16 (postgres, pg-es)
docker run -d --name hfs-pg -p 5432:5432 \
  -e POSTGRES_USER=helios -e POSTGRES_PASSWORD=helios -e POSTGRES_DB=helios postgres:16

# Elasticsearch 8.15.0 (any *-es composite)
docker run -d --name hfs-es -p 9200:9200 \
  -e discovery.type=single-node -e xpack.security.enabled=false \
  -e "ES_JAVA_OPTS=-Xms1g -Xmx1g" elasticsearch:8.15.0

# MongoDB 7.0 (mongodb, mongo-es)
docker run -d --name hfs-mongo -p 27017:27017 mongo:7.0

# MinIO (s3, s3-es, and the S3 output-backend variants of T5/T7)
docker run -d --name hfs-minio -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=hfs-minio -e MINIO_ROOT_PASSWORD=hfs-minio-secret \
  minio/minio:latest server /data --console-address ":9001"
# create the buckets once MinIO is up (console at http://localhost:9001)
docker run --rm --network host -e MC_HOST_local=http://hfs-minio:hfs-minio-secret@localhost:9000 \
  minio/mc mb --ignore-existing local/hfs local/hfs-export local/hfs-sql-export
```

Readiness checks:

```bash
docker exec hfs-pg pg_isready -U helios
curl -s localhost:9200/_cluster/health | jq .status
docker exec hfs-mongo mongosh --quiet --eval 'db.runCommand({ping:1}).ok'
curl -sf localhost:9000/minio/health/live && echo minio ok
```

Reset between backend rows: `docker rm -fv hfs-pg hfs-es hfs-mongo hfs-minio` and
recreate. For SQLite delete `data/hfs.db*` and `data/bulk_export.db*`; on every
backend also delete `data/submit` (bulk-import status artifacts).

---

## 5. T1 — Start HFS

### Common environment (every backend)

```bash
export HFS_SERVER_HOST=127.0.0.1 HFS_SERVER_PORT=8080 HFS_BASE_URL=http://localhost:8080
export HFS_LOG_LEVEL=info
export HFS_DEFAULT_FHIR_VERSION=R4
export HFS_MAX_BODY_SIZE=104857600        # headroom for the T3 transaction bundle (2.5 MB) and fixture bundles
export HFS_REQUEST_TIMEOUT=600            # large bundles on composite backends
export HFS_SUBSCRIPTIONS_ENABLED=true
export HFS_BULK_EXPORT_OUTPUT_DIR=$WORK/bulk-exports  # T5 local-fs output
export HFS_EXPORT_DIR=$WORK/sql-exports               # T7 fs sink
# composites: make searches read-your-write so T3 counts and T4 are deterministic
export HFS_COMPOSITE_SYNC_MODE=synchronous HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for
```

`HFS_BASE_URL` matters more than usual in this pass: the Import page makes HFS
submit `$bulk-submit` *to itself* at that URL, so it must be reachable from the HFS
process.

### Per-backend environment

| Backend | Additional environment |
|---|---|
| `sqlite` | `HFS_STORAGE_BACKEND=sqlite` (DB at `./data/hfs.db`; `HFS_DATA_DIR` stays the repo `./data` so the search-parameter files load) |
| `sqlite-es` | `HFS_STORAGE_BACKEND=sqlite-es HFS_ELASTICSEARCH_NODES=http://localhost:9200` |
| `postgres` | `HFS_STORAGE_BACKEND=postgres HFS_DATABASE_URL=postgresql://helios:helios@localhost:5432/helios` |
| `pg-es` | as `postgres` plus `HFS_STORAGE_BACKEND=pg-es HFS_ELASTICSEARCH_NODES=http://localhost:9200` |
| `mongodb` | `HFS_STORAGE_BACKEND=mongodb HFS_MONGODB_URI=mongodb://localhost:27017 HFS_MONGODB_DATABASE=helios` |
| `mongo-es` | as `mongodb` plus `HFS_STORAGE_BACKEND=mongo-es HFS_ELASTICSEARCH_NODES=http://localhost:9200` |
| `s3` | `HFS_STORAGE_BACKEND=s3 HFS_S3_BUCKET=hfs HFS_S3_ENDPOINT=http://localhost:9000 HFS_S3_FORCE_PATH_STYLE=true HFS_S3_REGION=us-east-1 AWS_ACCESS_KEY_ID=hfs-minio AWS_SECRET_ACCESS_KEY=hfs-minio-secret` |
| `s3-es` | as `s3` plus `HFS_STORAGE_BACKEND=s3-es HFS_ELASTICSEARCH_NODES=http://localhost:9200` |

Note on `s3`/`s3-es`: one process has one AWS credential chain, so MinIO as the
primary store means the T5/T7 S3 *output* variants must also target MinIO.

### Start and smoke

```bash
./target/release/hfs 2>&1 | tee "$WORK/hfs-$HFS_STORAGE_BACKEND.log" &
sleep 3
curl -sf $HFS/health | jq .
curl -sf $HFS/metadata | jq '{fhirVersion, software: .software.name, rest: (.rest[0].resource | length)}'
curl -sf "$HFS/metadata" | jq -r '.rest[0].operation[].name' | sort | tr '\n' ' '   # expect export, sql-run, sql-export, bulk-submit, ...
open $HFS/ui   # dashboard renders; sidebar shows the backend and FHIR version
```

Pass criteria: `/health` is 200; CapabilityStatement `fhirVersion` is `4.0.1`;
the startup log names the expected backend (and Elasticsearch index prefix for
composites); `/ui` loads with zero resources.

Also check version switching works on a multi-version build: `curl -sf
"$HFS/metadata?_format=json" -H 'Accept: application/fhir+json; fhirVersion=5.0'
| jq .fhirVersion` should report `5.0.0`. Then make sure the sidebar FHIR-version
selector is back on **R4** before continuing — the SQL pages refuse to run when the
sidebar version differs from the server default.

---

## 6. T2 — Batch / Transaction page

This step runs on the **empty server**, straight after T1 and before the corpus
import, so that its negative case is deterministic. It uses the second archive. The
patient file is a 662-entry `transaction` Bundle whose Encounters reference Synthea's
hospital organisations and practitioners by **conditional reference**
(`Organization?identifier=…`, `Practitioner?identifier=…`). Those resources are
created by the two `…Information…` files, which are `batch` Bundles. Uploading the
patient first therefore has to fail; the tester examines that failure, then loads
the reference data and repeats the patient upload.

```bash
mkdir -p "$WORK/batch" && cd "$WORK/batch"
curl -L -o fhir-batch-import.tar.gz https://hfs-manual-test.s3.us-east-1.amazonaws.com/fhir-batch-import.tar.gz
tar -xzf fhir-batch-import.tar.gz && ls      # hospitalInformation….json  practitionerInformation….json  Nicky270_Ann985_Larkin917_….json
```

### 6.1 Negative: the patient transaction without its reference data

1. Sidebar → **Batch & Data** → **Batch / Transaction** (`/ui/batch`).
2. Drag `Nicky270_Ann985_Larkin917_….json` onto **Drop a bundle JSON file here**, or
   click it and pick the file. The page moves to the **Execution Plan** stage.
3. Verify the request strip reads `POST [base] · Bundle · transaction · 662 entries`
   and the notice says *"Transaction: all or nothing — if any entry fails, the server
   rolls the whole bundle back."* There is no batch/transaction selector; the mode
   comes from the Bundle.
4. On the **Actions** tab, entry 1 is `POST Patient`; expand it to see the JSON body
   (Nicky270 Ann985 Larkin917, born 1996-04-19, Millis). Expand entry 2
   (`POST Encounter`): `serviceProvider.reference` is a conditional reference such as
   `Organization?identifier=https://github.com/synthetichealth/synthea|756ed90d-…`
   and `participant[0].individual.reference` is
   `Practitioner?identifier=http://hl7.org/fhir/sid/us-npi|9999…`. Nothing on the
   server matches them yet.
5. Click **Execute**. While it runs both buttons are disabled and *Executing…* shows.
6. **Expected failure.** The page stays on the Execution Plan and the error above
   the plan reads *"The request failed. — Conditional reference
   'Organization?identifier=…' matches no existing resource"* (the reference named
   may be one of the `Practitioner?identifier=…` ones instead; either is correct).
   Record the exact text. It is the server's `OperationOutcome` diagnostics, so it
   must name the reference, not just say "bad request".
7. Confirm the rollback: open **Resources**; the rail counts for **Patient**,
   **Encounter**, and **Observation** are still 0 and the dashboard is still empty.
8. Back on **Batch / Transaction** click **Cancel**; the page returns to the Upload
   stage with the error cleared.

On the `s3` and `s3-es` rows the transaction is refused for a different reason
(no multi-object atomicity) — record that message and mark 6.1 and 6.3 N/A.

### 6.2 Happy path: the reference data batches

1. Upload `hospitalInformation….json`. The strip reads
   `POST [base] · Bundle · batch · 9 entries` and the notice says *"Batch: entries
   run independently — a failed entry does not stop or undo the others."* The
   Actions list alternates `POST Organization` / `POST Location`.
2. **Execute** → **Per-Action Outcomes**: the badge shows HTTP `200`, the head
   reads **9 created**, and every row carries `201 Created`. Click **Done**.
3. Upload `practitionerInformation….json`: `batch · 8 entries`
   (`POST Practitioner` / `POST PractitionerRole`) → **Execute** → **8 created**.
   **Done**.
4. On **Resources** the rail now shows **Organization 4**, **Location 5**,
   **Practitioner 4**, **PractitionerRole 4**. Run
   `GET /Organization?identifier=https://github.com/synthetichealth/synthea|756ed90d-15f4-377d-b99f-ca1de5633481`
   → **1 result**, MEDWAY COUNTRY MANOR SKILLED NURSING & REHABILITAT.

### 6.3 Happy path: the patient transaction

1. Upload `Nicky270_Ann985_Larkin917_….json` again and **Execute**.
2. The **Per-Action Outcomes** stage appears: HTTP `200`, **662 created**, every row
   `201 Created`. Click **Done**.
3. Verify in **Resources**:
   - `GET /Patient?given=Nicky270&family=Larkin917&birthdate=1996-04-19` → **1 result**.
     Click the id: the modal subject line shows `Patient/<new id>`; note it as `LPID`.
   - `GET /Encounter?subject=Patient/<LPID>` → **49 results**;
     `GET /Observation?subject=Patient/<LPID>` → **106 results**;
     `GET /Condition?subject=Patient/<LPID>` → **33 results**.
   - Open one of the Encounters: `serviceProvider.reference` is now a literal
     `Organization/<id>` and `participant[0].individual.reference` a literal
     `Practitioner/<id>`, and those ids are among the resources created in 6.2 (the
     `Organization` rail entry lists exactly four).
   - The `urn:uuid:` references inside the bundle were rewritten too: the Encounter's
     `subject.reference` is `Patient/<LPID>`.

### 6.4 Negative: files the page must refuse

Save these two fixtures, then upload each; the page stays on Upload and shows the
message:

| File | Contents | Message |
|---|---|---|
| `$WORK/fixtures/not-a-bundle.json` | `{"resourceType":"Patient","id":"x"}` | *That JSON is not a FHIR Bundle.* |
| `$WORK/fixtures/collection.json` | `{"resourceType":"Bundle","type":"collection","entry":[]}` | *Only Bundles of type batch or transaction can be executed here.* |
| `$WORK/build.log` (any non-JSON file) | — | *That file is not valid JSON.* |

Pass criteria: 6.1 is rejected with diagnostics naming the
unresolvable conditional reference and creates nothing; 6.2 creates 9 + 8 resources
with per-entry `201` statuses; 6.3 creates 662 resources atomically with resolved
references; 6.4 gives the exact messages. On `s3`/`s3-es` run 6.2 and 6.4 only.

---

## 7. T3 — Import the Synthea corpus from the Import page

The corpus is a Bulk Data export of 11,704 Synthea patients (18,955,865 resources in
24 NDJSON files) plus a `manifest.json` that references those files at
`http://localhost:8000/…`. HFS ingests it with the Bulk Data `$bulk-submit`
operation, driven from the **Import** page, which makes HFS fetch the manifest and
every file from a small HTTP server you run on port 8000.

### 7.1 Download, unpack, and serve the corpus

```bash
mkdir -p "$WORK/corpus" && cd "$WORK/corpus"
[ -f fhir2.tar.gz ] || curl -L -o fhir2.tar.gz https://hfs-manual-test.s3.us-east-1.amazonaws.com/fhir2.tar.gz   # 3.6 GB
tar -xzf fhir2.tar.gz                    # extracts a directory containing the 24 *.ndjson files and manifest.json
cd fhir2 2>/dev/null || cd "$(dirname "$(find . -name manifest.json | head -1)")"
ls | wc -l                               # 26 files (24 NDJSON, manifest.json, parameters.json)
python3 -m http.server 8000 --bind 127.0.0.1 > "$WORK/corpus-http.log" 2>&1 &
curl -sf http://localhost:8000/manifest.json | head -c 400   # the manifest is being served
```

Leave the HTTP server running until the import has finished.

**Optional reduced import.** The full corpus is ~35 GB of NDJSON; on a slow machine
or a composite backend it can take hours. The later steps only need the file types
below, so the tester may serve a trimmed manifest instead and record the deviation
in the matrix cell:

```bash
jq '.output |= map(select(.type | IN("Patient","Encounter","Condition","Observation","Procedure",
                                     "Organization","Practitioner","PractitionerRole","Location")))' \
   manifest.json > manifest-core.json          # 11,197,644 resources; Observation (7.5 GB) is the bulk of it
```

Whichever manifest is used, the counts in T4 for `Patient`, `Encounter`,
`Condition`, and `Observation` are unchanged.

### 7.2 Create the submission in the UI

1. Open `$HFS/ui`. In the sidebar under **Batch & Data**, click **Import**
   (`/ui/bulk-import`). The **Submissions** table is empty:
   *"No submissions yet. Create one to get started."*
2. Click **New Submission**. The **Create Bulk Submission** dialog opens with focus in
   **Submission name**.
3. Fill in:
   - **Submission name**: `synthea-<backend>` (e.g. `synthea-sqlite`).
   - **Manifest URL**: `http://localhost:8000/manifest.json`
     (or `http://localhost:8000/manifest-core.json` for the reduced import).
   - **Authentication**: leave **None** selected.
   - Leave **Advanced options** collapsed (defaults: submitter
     `urn:helios:hfs:bulk-submit`, format `application/fhir+ndjson`).
4. Click **Submit**. The page redirects to the submission's detail page
   (`/ui/bulk-import/{id}`). Note the wall-clock time as a UTC instant (for example
   `2026-09-04T14:00:00Z`); 4.16 uses it as `<T3 start>`.

### 7.3 Watch the submission

On the detail page verify:

- The summary card shows **Manifest URL** = the URL you typed, **Data Recipient** =
  `http://localhost:8080`, a **Submission ID**, **Submitter**
  `urn:helios:hfs:bulk-submit | <submission id>`, **Status** = **In Progress**,
  **Authentication** = `none`.
- The **Submission Log** (newest first) contains
  `Submitting manifest "http://localhost:8000/manifest.json"...`,
  `Manifest accepted by the recipient (200).`, and `Bulk status kick-off request`.
- The status card shows **Processing** with a progress bar, and refreshes on its own
  every 5 s. Its text is the recipient's progress report (or *"Waiting for the
  recipient's first status report…"* right after kick-off).
- In the HTTP server log (`$WORK/corpus-http.log`) the NDJSON files are being
  requested one after another.

Wait for the status card to change to **Result** → *"Processing finished at …"*,
**Output files** = 24 (or 9 for the reduced manifest) and **Error files** = 0, the
summary **Status** = **Completed**, and the log to end with
`Status: got 200 OK — processing finished cleanly (24 outputs); submission completed.`
Record the elapsed time in the matrix.

If the status becomes **Failed**, the **Error files** count is non-zero, or the log
shows `POST http://localhost:8080/$bulk-submit → …` with an error, record the log text
and file an issue.

### 7.4 Verify the data landed and is searchable

1. Open `$HFS/ui` (the dashboard). The stat cards and the resources-over-time chart
   must reflect the import; the **Patient** card reads 11,705 (the corpus plus the
   patient from T2).
2. Click **Resources** in the sidebar. The **Resource Types** rail shows a live count
   next to every type; compare against the manifest plus what T2 created:

   | Type | Expected count |
   |---|---|
   | Patient | 11,705 |
   | Encounter | 827,968 |
   | Condition | 476,455 |
   | Observation | 7,699,987 |
   | Procedure | 2,177,375 |
   | Organization / Practitioner / PractitionerRole | 1,140 each |
   | Location | 1,142 |

3. In the **QUERY** box type `GET /Patient?_id=7d24f7a0-6f2e-ce3b-5568-db7b14695583`
   and press **Run**. One row: Cari853 Esperanza675 Parker433, female, 2015-12-29.
   Click the id link; the **Edit Resource** modal opens with the JSON. Close it.
4. Type `GET /Observation?_summary=count` and **Run**: the results header reads
   **7,699,987 results** and the table says *No results.* (a count-only Bundle has no
   entries; that is correct).
5. On composites, confirm the Elasticsearch document counts match
   (`curl localhost:9200/_cat/indices/hfs*` — an infrastructure check, not an HFS API
   call).

### 7.5 Optional: back to Batch / Transaction

The corpus contains its own copy of every organisation and practitioner that T2
created, each with the same identifier. Upload `Nicky270_Ann985_Larkin917_….json`
once more on **Batch / Transaction** and **Execute**: it must now be rejected with
*"The request failed. — Conditional reference 'Organization?identifier=…' matches
more than one resource"*, and `GET /Patient?given=Nicky270&family=Larkin917` on
**Resources** is still **1 result**. Click **Cancel**.

Pass criteria: the submission finishes **Completed** with 0 error files; the rail
counts match the table; the anchor patient is found by id; the dashboard reflects
the import; the optional duplicate-reference upload is rejected without side effects.

---

## 8. T4 — One manual search per FHIR search type

All searches are typed into the **QUERY** box on **Resources** (`/ui/resources`).
The box accepts a raw FHIR search (`GET /Patient?name=Parker433&_count=5`) and
**Run** (or Enter) executes it and renders the Bundle in the **Results** card. The
results header shows **N results** taken from `Bundle.total` and, when the Bundle
carries `_include`/`_revinclude` entries, **· M included**. The **Open in New Tab**
link is the exact path that ran — hover it to confirm the URL the UI built, or click
it to see the raw Bundle.

Expected counts assume the full corpus plus the T2 transaction (11,705 patients).
`PID` is the anchor patient `7d24f7a0-6f2e-ce3b-5568-db7b14695583`.

### 8.1 Fixtures (created in the Resource Editor)

Two of the rows need resources the corpus does not contain. Create them with the
standalone editor's raw-JSON pane. Because each carries an `id`, **Save Changes**
issues a `PUT` and the ids are known in advance.

1. Open `$HFS/ui/editor?type=RiskAssessment` (type the URL; the editor is not in the
   sidebar). In the **JSON** card click **Edit raw**, replace the text with:

   ```json
   {"resourceType":"RiskAssessment","id":"manual-risk","status":"final",
    "subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"},
    "prediction":[{"probabilityDecimal":0.8}]}
   ```

   Click **Edit raw** again (the guided form re-renders and the chip reads **No
   issues.**), then **Save Changes** → status line **Saved.**
2. Open `$HFS/ui/editor?type=ValueSet`, **Edit raw**, paste, save:

   ```json
   {"resourceType":"ValueSet","id":"manual-test-vs","status":"active",
    "url":"http://example.org/fhir/ValueSet/manual-test","name":"ManualTest"}
   ```

### 8.2 The searches

| # | Search type | Type into the QUERY box | Expected in the Results card |
|---|---|---|---|
| 4.1 | **string** | `GET /Patient?family=Parker433` then `GET /Patient?family:exact=Parker433` then `GET /Patient?family:contains=arker43` then `GET /Patient?name=cari853` | **30 results** for the first two; the `:contains` form ≥ 30; the lower-case `name=cari853` form finds the anchor patient (≥ 8 results — `name` also matches given names, case-insensitively) |
| 4.2 | **token** | `GET /Patient?gender=female` · `GET /Patient?gender:not=female` · `GET /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999-33-3920` · `GET /Observation?code=http://loinc.org\|8302-2` · `GET /Observation?code=8302-2` | **5,814** · **5,891** (the two add up to 11,705) · **1 result** = the anchor patient · > 175,000 results, identical for the `system\|code` and code-only forms |
| 4.3 | **date** | `GET /Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01` · `GET /Encounter?patient=PID&date=ge2016` · `GET /Patient?_lastUpdated=ge<today, YYYY-MM-DD>` | **1,268 results** · between 1 and 24 results, every `period.start` in 2016 or later · **11,705** |
| 4.4 | **number** | `GET /RiskAssessment?probability=gt0.5` · `GET /RiskAssessment?probability=lt0.5` · `GET /RiskAssessment?probability=ap0.8` | **1 result** (`manual-risk`) · **0 results** · **1 result** |
| 4.5 | **quantity** | `GET /Observation?code=8302-2&value-quantity=gt150` · `GET /Observation?code=8302-2&value-quantity=gt150\|\|cm` · `GET /Observation?code=8302-2&value-quantity=lt50\|http://unitsofmeasure.org\|cm` | first two > 0 and equal (every corpus height is in cm); open a row and check `valueQuantity.value` > 150; the third is a strict subset (infant heights) |
| 4.6 | **reference** | `GET /Observation?subject=Patient/PID` · `GET /Condition?patient=PID` · `GET /Encounter?subject=PID&_include=Encounter:subject` | **165** · **15** · **24 results · 1 included** (the included Patient is not shown as a row; the raw Bundle via **Open in New Tab** has one entry with `search.mode = include`) |
| 4.7 | **uri** | `GET /ValueSet?url=http://example.org/fhir/ValueSet/manual-test` · `GET /ValueSet?url:below=http://example.org/fhir` | **1 result** · ≥ 1 |
| 4.8 | **composite** | `GET /Observation?code-value-quantity=http://loinc.org\|8302-2$gt150` | > 0; equals the first count in 4.5; every row is a Body Height with value > 150 |
| 4.9 | **special** (`_id`) | `GET /Patient?_id=PID` · `GET /Patient?_id=PID,<LPID from T2>` | **1** · **2** |
| 4.10 | **chained** | `GET /Observation?subject.identifier=http://hl7.org/fhir/sid/us-ssn\|999-33-3920` · `GET /Observation?subject:Patient.family=Parker433&_count=5` | **165 results** (same as 4.6) · > 165, every row's `subject.display` ends in Parker433. **N/A on `mongodb`** (forward chains unsupported; expect a clear error, not a 500) |
| 4.11 | **reverse chained** | `GET /Patient?_has:Observation:patient:code=http://loinc.org\|8302-2&_count=5` | > 0; pick a row, then `GET /Observation?patient=<that id>&code=8302-2` is > 0. **N/A on `mongodb`** |
| 4.12 | **_revinclude / _sort / paging** | `GET /Patient?_id=PID&_revinclude=Condition:patient` · `GET /Observation?patient=PID&_sort=-date&_count=5` · `GET /Patient?_count=20&_total=accurate` | **1 result · 15 included** · **165 results**, 5 rows, `effective` dates descending (also try the **Sort** dropdown: *Most recent*/*Oldest* re-run with `_sort` swapped) · **11,705 results**, 20 rows, **Next** appears; click it — the total stays 11,705 and **Previous** appears |
| 4.13 | **_content** (full text) | `GET /Patient?_content=Everett` | ≥ 83 results (83 patients live in Everett); on composites check the log to confirm Elasticsearch served it |
| 4.14 | **visual builder + saved query** | On **Saved Queries** (`/ui/queries`, type the URL) click **Patient** in the rail, then **+ Add condition**: parameter `family`, modifier **is**, value `Parker433`; **+ Add condition**: parameter `birthdate`, comparator **ge**, value `2010-01-01`; **+ _count** → key `_sort`, value `birthdate`. | The QUERY box reads `GET /Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate`; **Run** shows the Parker433 children (≥ 1, birth dates ascending). Enter **Name** `Parker kids`, click **Save**; it appears under **Patient** in the saved list; **Run** there re-runs it and its meta shows `1×`; the **Recent** dropdown lists it under **Saved**. |

### 8.3 Searches over the data loaded by Batch / Transaction (T2)

These target the Larkin patient (`LPID`, noted in 6.3) and the organisations and
practitioners the batch bundles created. Counts are as of after T3: the corpus
carries its own copy of each organisation and practitioner, so the reference data
shows up twice, while the patient and everything under it exist only once.

| # | Search type | Type into the QUERY box | Expected in the Results card |
|---|---|---|---|
| 4.15 | **token / string** on the patient | `GET /Patient?identifier=http://hl7.org/fhir/sid/us-ssn\|999-19-2626` · `GET /Patient?address-city=Millis` · `GET /Patient?family=Larkin917&given=Nicky270&gender=female` | **1 result** = `LPID` (this SSN exists only in the batch archive) · **13 results** (12 corpus + `LPID`) · ≥ 1, `LPID` among them |
| 4.16 | **date** (`_lastUpdated`) separates the two import paths | `GET /Patient?_lastUpdated=lt<T3 start>` · `GET /Patient?_lastUpdated=ge<T3 start>` where `<T3 start>` is the instant noted in 7.2 in UTC, e.g. `2026-09-04T14:00:00Z` | **1 result** = `LPID` (created in T2, before the import) · **11,704** |
| 4.17 | **token + date** on Encounters | `GET /Encounter?patient=LPID&class=EMER` · `GET /Encounter?patient=LPID&class=IMP` · `GET /Encounter?patient=LPID&date=ge2020` · `GET /Encounter?patient=LPID&type=http://snomed.info/sct\|424619006` | **5** · **1** · **38** · **17** (prenatal visits) |
| 4.18 | **reference + `_include`** through references the transaction resolved | `GET /Encounter?patient=LPID&_include=Encounter:service-provider` · `GET /Encounter?patient=LPID&_include=Encounter:participant` | **49 results · 4 included** (the four batch Organizations) · **49 results · 4 included** (the four batch Practitioners). In the raw Bundle (**Open in New Tab**) every `serviceProvider.reference` is a literal `Organization/<id>` |
| 4.19 | **chained** through the batch reference data | `GET /Encounter?patient=LPID&service-provider.name=ENCOMPASS` · `GET /Encounter?patient=LPID&participant.identifier=http://hl7.org/fhir/sid/us-npi\|9999989798` | **38** · **38** (38 of the 49 encounters are at ENCOMPASS HEALTH BRAINTREE with Dr. Nickolas58 Schumm995). **N/A on `mongodb`** |
| 4.20 | **batch reference data**, duplicated by the corpus | `GET /Organization?name=TIMOTHY DANIELS HOUSE` · `GET /Organization?address-city=HOLLISTON` · `GET /Practitioner?identifier=http://hl7.org/fhir/sid/us-npi\|9999888693` · `GET /Practitioner?family=Torphy630&given=Laine739&gender=female` · `GET /Location?name=A&A HEALTHCARE LLC` | **2 results** each (one created by the T2 batch with a server-assigned id, one imported by T3 with the Synthea id) |
| 4.21 | **clinical data** under the patient | `GET /Condition?patient=LPID&clinical-status=active` · `GET /Condition?patient=LPID&code=http://snomed.info/sct\|72892002` · `GET /Observation?patient=LPID&code=29463-7&value-quantity=gt60` · `GET /Observation?patient=LPID&code-value-quantity=http://loinc.org\|8302-2$gt160` · `GET /Immunization?patient=LPID&vaccine-code=http://hl7.org/fhir/sid/cvx\|140` · `GET /MedicationRequest?patient=LPID&status=stopped` · `GET /MedicationRequest?patient=LPID&code=http://www.nlm.nih.gov/research/umls/rxnorm\|757594` | **6** · **3** (Normal pregnancy) · **2** (60.2 kg and 64.5 kg) · **3** (all 164.1 cm) · **3** (seasonal influenza) · **9** · **4** (Jolivette 28 Day Pack) |
| 4.22 | **`_revinclude` / `_has` / `_sort`** | `GET /Patient?_id=LPID&_revinclude=Immunization:patient` · `GET /Patient?_has:Condition:patient:code=http://snomed.info/sct\|706893006&_count=50` · `GET /Observation?patient=LPID&code=29463-7&_sort=date` | **1 result · 8 included** · `LPID` is among the rows · **4 results** whose values read 55.4, 58.5, 60.2, 64.5 from top to bottom (open each row). **`_has` is N/A on `mongodb`** |

### 8.4 Searches over the data loaded by the bulk import (T3)

These target the anchor patient (`PID`) and corpus-only reference data, beyond what
8.2 already covers.

| # | Search type | Type into the QUERY box | Expected in the Results card |
|---|---|---|---|
| 4.23 | **token / string / date** on the patient | `GET /Patient?identifier=https://github.com/synthetichealth/synthea\|7d24f7a0-6f2e-ce3b-5568-db7b14695583` · `GET /Patient?phone=555-613-6236` · `GET /Patient?birthdate=2015-12-29` · `GET /Patient?address-city=Everett&gender=female` | **1** = `PID` · **1** = `PID` · **2**, `PID` among them · **38** |
| 4.24 | **token + date** on Encounters | `GET /Encounter?patient=PID&class=AMB` · `GET /Encounter?patient=PID&class=EMER` · `GET /Encounter?patient=PID&date=ge2020` | **23** · **1** · **12** |
| 4.25 | **references the bulk import left unresolved** | `GET /Encounter?patient=PID&_include=Encounter:service-provider`, then open one row | **24 results** with **no** *included* count. In the JSON, `serviceProvider.reference` is still the string `Organization?identifier=https://github.com/synthetichealth/synthea\|…`: the bulk import stores resources verbatim and does not rewrite conditional references, unlike the transaction in 4.18. Expected — record it, not a failure |
| 4.26 | **clinical data** under the patient | `GET /Condition?patient=PID&clinical-status=active` · `GET /Condition?patient=PID&code=http://snomed.info/sct\|65363002` · `GET /Immunization?patient=PID` · `GET /MedicationRequest?patient=PID` · `GET /Procedure?patient=PID` | **1** · **2** (Otitis media) · **25** · **7** · **18** |
| 4.27 | **quantity + `_sort`** (growth chart) | `GET /Observation?patient=PID&code=8302-2&_sort=date&_count=20` · `GET /Observation?patient=PID&code=8302-2&value-quantity=gt120` · `GET /Observation?patient=PID&code=8302-2&value-quantity=gt100\|\|cm` | **15 results**, oldest first; opening the first and last rows shows 72 cm (2016-09-07) and 145.3 cm (2026-01-06) · **4** · **8** |
| 4.28 | **corpus-only reference data** | `GET /Organization?identifier=https://github.com/synthetichealth/synthea\|e2a8b444-9b8f-36ff-84c4-05ee98589482` · `GET /Organization?name=WHITLEY WELLNESS` · `GET /Location?address-city=Fitchburg` · `GET /Location?name=Fitchburg Outpatient Clinic` | **1** each (WHITLEY WELLNESS LLC, Charlestown, is the anchor's usual provider and is not in the batch archive) |
| 4.29 | **`_has` / `_revinclude`** across the corpus | `GET /Patient?_has:Condition:patient:code=http://snomed.info/sct\|65363002&_id=PID` · `GET /Patient?_id=PID&_revinclude=Immunization:patient` · `GET /Patient?_id=PID&_revinclude=Encounter:patient&_revinclude=Procedure:patient` | **1** · **1 result · 25 included** · **1 result · 42 included** (24 + 18). **`_has` is N/A on `mongodb`** |

Pass criteria: every row in 8.2–8.4 produces the expected count or shape; no row
reports an error except the documented N/A rows on `mongodb`; the **Open in New Tab** URL matches
what was typed. On `s3` (standalone) the whole step is N/A: the backend has no
search, and the Resources page reports an error for every query — record the message.

---

## 9. T5 — Bulk Data `$export` from the Export page

The **Export** page (`/ui/bulk-export`, sidebar **Batch & Data → Export**) kicks off
`$export` jobs. Output is always NDJSON (the page has no output-format selector). The
axes worth covering are the three scopes, type restriction, `_typeFilter`,
`_elements`, the `_since`/`_until` window, cancel, retry, delete, and the local-fs vs
S3 output backend.

### 9.1 Group fixture

Open `$HFS/ui/editor?type=Group`, **Edit raw**, paste, **Save Changes** (**Saved.**):

```json
{"resourceType":"Group","id":"manual-group","type":"person","actual":true,
 "member":[{"entity":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"}}]}
```

### 9.2 Exports

For each export: **Export** → **New Export** (`/ui/bulk-export/new`), fill the form,
**Start Export**, and watch the card on the **Exports** list. In-progress cards
refresh every 5 s and show the server's progress text; complete cards show **N
files**, *finished in …*, and one download pill per resource type.

| # | Name | Form | Expect on the card |
|---|---|---|---|
| 5.1 | `everything-small` | scope **Everything**; untick **All Resources** and tick only `Organization`, `Practitioner`, `Location` | **Complete · 3 files**; pills `Organization`, `Practitioner`, `Location`. Download `Organization`: 1,140 lines (1,136 corpus + 4 from T2); each line is one JSON object |
| 5.2 | `one-patient` | scope **Patients**; in **Patients** search `Parker433` and pick Cari853 Esperanza675 Parker433 (or paste `PID`); types `Patient`, `Condition`, `Observation` | **Complete · 3 files**; `Patient` file has 1 line, `Condition` 15, `Observation` 165 |
| 5.3 | `group-active-conditions` | scope **Group**, **Group ID** `manual-group`; types `Patient`, `Condition`; **Type filter** `Condition?clinical-status=active`; **FHIR elements** empty; **Since** *All time* | **Complete · 2 files**; `Patient` has 1 line; every line of `Condition` has `clinicalStatus` = `active` and belongs to `PID` (fewer than the 15 of 5.2) |
| 5.4 | `elements-subset` | scope **Everything**; type `Patient` only; **FHIR elements** `id,gender` | **Complete · 1 file**; each Patient line has only `id`, `gender`, `meta` and the `meta.tag` `SUBSETTED` |
| 5.5 | `cancel-me` | scope **Everything**, **All Resources** ticked | while **In progress**, click **Cancel** → chip **Cancelled** |
| 5.6 | negative | leave **Name** empty and **Start Export** | the form re-renders with *"Enter a name for this export."* |

Then on `everything-small` click **Download All Resources**: the browser saves a ZIP
holding the three NDJSON files. On `cancel-me` click **Delete** → the warning
*"Delete cancel-me and its output files from the server? This cannot be undone."* →
**Delete export**; the card disappears.

### 9.3 Time window (`_since` / `_until`)

The **Since** control has the presets *All time*, *Last day*, *Last 7 days*, *Last 4
weeks*, and *Custom* (which enables **Custom instant**); **Until** is an HFS
extension. Both filter on `meta.lastUpdated`, and the three loads so far happened at
distinct times: the T2 transaction (before `<T3 start>`), the T3 corpus (between
`<T3 start>` and the *Processing finished at* instant on the submission's detail
page, call it `<T3 end>`), and the T4/T5 fixtures (after `<T3 end>`). All instants
are entered in UTC, e.g. `2026-09-04T14:00:00Z`.

| # | Name | Form | Expect on the card |
|---|---|---|---|
| 5.7 | `since-import` | scope **Everything**; type `Patient`; **Since** *Custom*, **Custom instant** `<T3 start>` | window line **Since <instant>**; **Complete · 1 file**; `Patient` has **11,704** lines — the Larkin patient from T2 is older than the window and absent |
| 5.8 | `until-import` | type `Patient`; **Since** *All time*; **Until** `<T3 start>` | window line **Until <instant>**; `Patient` has **1** line: Nicky270 Ann985 Larkin917 |
| 5.9 | `since-until` | type `Patient`; **Since** *Custom* `<T3 start>`, **Until** `<T3 end>` | window line `since → until`; `Patient` has **11,704** lines |
| 5.10 | `since-fixtures` | types `Patient`, `RiskAssessment`, `ValueSet`, `Group`; **Since** *Custom* `<T3 end>` | `RiskAssessment`, `ValueSet`, `Group` pills with **1** line each (`manual-risk`, `manual-test-vs`, `manual-group`); no `Patient` pill, or an empty `Patient` file |
| 5.11 | `last-day` | type `Organization`; **Since** *Last day* | window line shows an instant about 24 h ago; **1,140** lines when T2 and T3 ran within the last day (otherwise only the T2 copies, 4 lines) |
| 5.12 | negative | **Since** *Custom*, **Custom instant** `yesterday` | the form re-renders with *"Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z."* under the field; switch the preset back to *All time* and the same text no longer blocks the submit (the field is disabled) |

### 9.4 Failure and Retry

| # | Name | Form | Expect on the card |
|---|---|---|---|
| 5.13 | `bad-group` | scope **Group**, **Group ID** `does-not-exist`; type `Patient` | the card appears at once as **Failed** with the kick-off diagnostics naming the missing Group; click **Retry** — the same card resets, runs with the same parameters, and fails identically; **Delete** → *Delete export* removes it |

### 9.5 S3 output backend (`sqlite` and `postgres` rows only)

Restart HFS with `HFS_BULK_EXPORT_OUTPUT_BACKEND=s3 HFS_BULK_EXPORT_S3_BUCKET=hfs-export
HFS_BULK_EXPORT_S3_ENDPOINT=http://localhost:9000 HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE=true
HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN=false` plus the MinIO credentials from section
5, and repeat 5.1. The download pills must be pre-signed MinIO URLs that download.

Pass criteria: 5.1–5.4 and 5.7–5.11 complete with the stated files and line counts;
5.5 cancels; 5.6 and 5.12 are rejected; 5.13 fails, retries, and deletes as
described; the ZIP download works. On `mongodb`, `mongo-es`, `s3`, `s3-es` the
card appears immediately as **Failed** with
`kick-off answered 501: bulk export not supported by this backend` — record N/A, and
check that **Delete** removes the failed card.

---

## 10. T6 — Create a ViewDefinition and examine its output

Everything happens on **SQL on FHIR → View Definitions** (`/ui/sql/view-definitions`).
The page has no Run button: the **Results** card runs the current editor text on
load and again 500 ms after every edit, saved or not, capped at 50 rows.

### 10.1 `patient_demographics`

1. Click **Create New**. The **Definition (JSON)** editor holds a starter
   `new_view` document; the title reads **New View Definition**.
2. Select all in the editor and replace it with:

   ```json
   { "resourceType": "ViewDefinition", "url": "http://example.org/ViewDefinition/patient_demographics",
     "name": "patient_demographics", "status": "active", "resource": "Patient",
     "select": [ { "column": [
         { "name": "id",        "path": "getResourceKey()", "type": "id" },
         { "name": "gender",    "path": "gender" },
         { "name": "birth_date","path": "birthDate", "type": "date" },
         { "name": "family",    "path": "name.first().family" },
         { "name": "city",      "path": "address.first().city" } ] } ],
     "where": [ { "path": "active.exists().not() or active = true" } ] }
   ```

3. Within a second the **Results** card shows **50 rows · N ms** with the columns
   `id, gender, birth_date, family, city`; the **Guided form** chip reads **No issues.**
4. **Lint**: change `"column"` to `"colum"`. A squiggle and gutter marker appear;
   hover shows `Unknown key "colum"`. Press **Ctrl+Shift+M** to open the lint panel,
   then **Ctrl+.** on the line and apply the fix **Rename to "column"**. The chip
   returns to **No issues.**
5. **Completion**: inside the `id` column's `path` string delete `getResourceKey()`,
   type `getRes` and press **Ctrl+Space**; the list offers `getResourceKey()`. Accept
   it. Also delete a key name and press **Ctrl+Space** at the object position: the
   structural keys are offered with required ones tagged **required**.
6. Click **Save**. The page redirects to `?vd=<id>&saved=1`, shows **Saved.**, and the
   rail lists `patient_demographics · Patient` (also under **Recently used**). Note
   the id as `VD`.
7. Cross-check a row: copy an `id` from the results table, then on **Resources** run
   `GET /Patient?_id=<that id>`; `gender`, `birthDate`, and the family name match.
   For `PID` itself the row must read `female · 2015-12-29 · Parker433 · Everett`.

### 10.2 `observation_flat`

Click **Create New** again and paste:

```json
{ "resourceType": "ViewDefinition", "url": "http://example.org/ViewDefinition/observation_flat",
  "name": "observation_flat", "status": "active", "resource": "Observation",
  "select": [ { "column": [
      { "name": "id",         "path": "getResourceKey()", "type": "id" },
      { "name": "patient_id", "path": "subject.getReferenceKey(Patient)", "type": "id" },
      { "name": "code",       "path": "code.coding.first().code" },
      { "name": "value",      "path": "value.ofType(Quantity).value", "type": "decimal" },
      { "name": "effective",  "path": "effective.ofType(dateTime)", "type": "dateTime" } ] } ] }
```

Results show 50 Observation rows with `code` values such as `8302-2`. **Save**; note
the id as `VD2`.

### 10.3 Rail, duplicate, delete

- Type `patient` into **Filter views**: only `patient_demographics` remains.
- With `patient_demographics` selected click **Duplicate**: a `patient_demographics_copy`
  is created and selected. Click **Delete** → confirm
  *Delete view definition "patient_demographics_copy"? This cannot be undone.* → it
  disappears from the rail.
- Negative: in a new definition set `"resource": "Nope"` — the lint panel flags it and
  the Results card shows *"Could not run the view. …"* while the previous table stays
  labelled *last successful run*. Click **Save** anyway: the prompt *"This view
  definition still has 1 error(s). Save it anyway?"* appears; choose Cancel.

Pass criteria: both definitions save and run; lint, fix, and completion behave as
described; the cross-check row matches the stored Patient; duplicate/delete work.
This step is expected to pass on all eight backends.

---

## 11. T7 — SQL export with a ViewDefinition, a SQL query, and a SQL view

`$sql-export` is driven from **SQL on FHIR → SQL Export**. A subject may be a
**ViewDefinition**, a **SQL Query** (Library), or a **SQL View** (Library). Cover each
kind and every output format (**NDJSON**, **CSV**, **JSON**, **Parquet**) — 11.4 has the
full kind × format grid.

### 11.1 Create the SQL View on `/ui/sql/views`

1. **SQL on FHIR → SQL Views** → **Create New**. The **Library (JSON)** card holds a
   starter Library; the **View definition (SQL)** card holds `SELECT * FROM v`.
2. Replace the Library JSON with:

   ```json
   {"resourceType":"Library","name":"female_patients","status":"active",
    "url":"http://example.org/Library/female_patients",
    "type":{"coding":[{"system":"http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes","code":"sql-view"}]},
    "relatedArtifact":[{"type":"depends-on","resource":"http://example.org/ViewDefinition/patient_demographics","label":"pd"}]}
   ```

3. Replace the SQL with `SELECT id, birth_date, city FROM pd WHERE gender = 'female'`.
4. The **Preview** card refreshes to 50 rows with `id, birth_date, city`. **Save** →
   **Saved.**; the rail shows `female_patients · active`. Note the id as `QV`.

### 11.2 Create the SQL Query on `/ui/sql/queries`

1. **SQL on FHIR → SQL Queries** → **Create New**.
2. Library JSON:

   ```json
   {"resourceType":"Library","name":"tall_female_patients","status":"active",
    "url":"http://example.org/Library/tall_female_patients",
    "type":{"coding":[{"system":"http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes","code":"sql-query"}]},
    "relatedArtifact":[{"type":"depends-on","resource":"http://example.org/ViewDefinition/observation_flat","label":"obs"},
                       {"type":"depends-on","resource":"http://example.org/Library/female_patients","label":"fp"}],
    "parameter":[{"name":"min_height","use":"in","type":"decimal"}]}
   ```

3. SQL:

   ```sql
   SELECT fp.id, fp.city, MAX(obs.value) AS height
   FROM fp JOIN obs ON obs.patient_id = fp.id
   WHERE obs.code = '8302-2' AND obs.value > :min_height
   GROUP BY fp.id, fp.city
   ```

4. The live **Results** preview cannot supply parameter values, so it reports an error
   naming the unbound `min_height` parameter — record the exact message; this is
   expected. To see the query run here, temporarily replace `:min_height` with `150`:
   rows with `height > 150` appear. Put `:min_height` back and **Save**; note the id
   as `QQ`. The type chip reads **SQL Query** and the status chip **active**.
5. Negative: change the `code` to `sql-view` and **Save** — rejected with
   *The Library's SQL on FHIR type must be "sql-query" to save it here.* Restore it.

### 11.3 Kick off one export per subject kind, cycling the formats

Open **SQL on FHIR → SQL Export** → **New SQL Export** (`/ui/sql/export/new`). The
**Subjects** table lists `patient_demographics`, `observation_flat` (kind
**ViewDefinition**), `tall_female_patients` (**SQL Query**, with a **1 parameter**
chip) and `female_patients` (**SQL View**). Ticking the query reveals its
`:min_height · decimal` field.

| # | Name | Subjects | Format | Expect on **SQL Exports** |
|---|---|---|---|---|
| 7.a | `vd-ndjson` | `patient_demographics` | **NDJSON** | **Complete**, 1 file; the file has 11,705 lines (one per patient — all corpus patients pass the `where`) |
| 7.b | `query-csv` | `tall_female_patients`, `min_height` = `150` | **CSV**, **Include a header row** ticked (visible only for CSV) | 1 file; header `id,city,height`; every `height` > 150; two ids spot-checked in **Resources** are female (`GET /Patient?_id=<id>`) |
| 7.c | `view-parquet` | `female_patients` | **Parquet** | 1 file; opens with `pyarrow`/`duckdb`; schema `id, birth_date, city`; row count = 5,814 |
| 7.d | `all-three-json` | all three | **JSON** | **3 files**, named after the subjects; each is one JSON array |
| 7.e | `cancel-me` | `observation_flat` | NDJSON | click **Cancel** while **In progress** → **Cancelled** |
| 7.f | negative | nothing ticked | — | *"Select at least one subject."*; tick the query, clear `min_height` → *"This value is required."* |

In-progress cards poll every 5 s and show `N subjects (… ) · FORMAT · started …`.
On a complete card click **View files**: the detail page lists **Job**, **Format**
(`CSV · with header row` for 7.b), **Subjects** with the `:min_height = 150` chip, and
**Output files** with download pills. Download and inspect each file as described.
Record the wall-clock *finished in* time of 7.a per backend.

Then: on `vd-ndjson` open **⋮ → Run again** — a brand-new card appears (the old one
stays); on the finished copy use **⋮ → Remove from list**. Restart HFS while one job
is **In progress**: after the restart its card resolves to **Cancelled · the server no
longer knows this job** (not an error), while complete cards keep their downloads.

### 11.4 Complete the subject-kind × format matrix

The View Definitions, SQL Queries, and SQL Views pages only preview JSON (50 rows,
no download), so the four output formats are reachable only through SQL Export.
7.a–7.d cover each format once; these five jobs finish the grid so every subject
kind is exported in every format.

| # | Name | Subjects | Format | Expect |
|---|---|---|---|---|
| 7.m | `vd-csv` | `patient_demographics` | **CSV**, header on | header `id,gender,birth_date,family,city` plus **11,705** data lines; `PID`'s line reads `…,female,2015-12-29,Parker433,Everett` |
| 7.n | `vd-parquet` | `patient_demographics` | **Parquet** | schema `id, gender, birth_date, family, city`; **11,705** rows; `birth_date` is a date column, not a string |
| 7.o | `query-ndjson` | `tall_female_patients`, `min_height` = `150` | **NDJSON** | one JSON object per line with `id`, `city`, `height`; line count equals the data-line count of 7.b |
| 7.p | `query-parquet` | `tall_female_patients`, `min_height` = `150` | **Parquet** | schema `id, city, height` with `height` numeric; row count equals 7.o |
| 7.q | `view-ndjson` | `female_patients` | **NDJSON** | **5,814** lines, each with `id`, `birth_date`, `city` and no `gender` key |

Coverage after 7.a–7.q:

| Subject kind | NDJSON | CSV | JSON | Parquet |
|---|---|---|---|---|
| ViewDefinition | 7.a | 7.m | 7.d | 7.n |
| SQL Query | 7.o | 7.b | 7.d | 7.p |
| SQL View | 7.q | 7.k (no header) | 7.d | 7.c |

Bulk Data `$export` (T5) has a single output format, NDJSON; the Export page offers
no format selector, so nothing more is needed there.

### 11.5 Filters, tracking id, and the subjects table

`<T3 start>` and `<T3 end>` are the instants defined in 9.3.

| # | Name | Form | Expect |
|---|---|---|---|
| 7.g | `one-patient` | `patient_demographics`; **Patients**: type `Parker433` and pick Cari853 Esperanza675 Parker433 (or paste `PID`); **NDJSON** | 1 file with **1** line: `female`, `2015-12-29`, `Parker433`, `Everett`; the detail page's **Job** card lists **Patients** |
| 7.h | `one-group` | `patient_demographics`; **Groups**: `manual-group`; **JSON** | 1 file holding a one-element array for `PID`; the **Job** card lists **Groups** |
| 7.i | `since-import` | `patient_demographics`; **Since** *Custom* `<T3 start>`; **NDJSON** | **11,704** lines (the Larkin patient is older than the window); the **Job** card lists **Since** |
| 7.j | `since-nothing` | `patient_demographics`; **Since** *Custom* `<T3 end>` | **Complete**; the output has **0** rows |
| 7.k | `tracked-csv-noheader` | `female_patients`; **CSV**; open **Advanced**, **Tracking id** `release-check-01`, untick **Include a header row** | the file has **no** header and **5,814** lines; the **Job** card shows **Format** `CSV · no header row` and **Tracking id** `release-check-01` |
| 7.l | negative | **Since** *Custom* `yesterday` · **Tracking id** of 201 characters · **Patients** (no-JS textarea, or paste) `not a valid id!` | *"Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z."* · *"Tracking id must be 200 characters or fewer."* · *"Enter only valid logical Patient IDs, separated by commas or new lines."* — each re-render keeps everything else you typed |

Subjects table controls (on `/ui/sql/export/new`):

- Click **Queries** in the segmented switch: only `tall_female_patients` stays
  visible; **All** brings the rest back. Type `female` in **Filter subjects**: only
  `female_patients` and `tall_female_patients` remain; the **Select all** header box
  ticks just those two and the hint reads **2 of 4 selected**.
- Clear the filter, tick `patient_demographics`, then filter to `obs`: the hint still
  says **2 of 4 selected** — hiding a row never unchecks it — and **Start Export**
  submits both.
- On **SQL Queries**, select `tall_female_patients` and temporarily replace
  `:min_height` with `150` so the preview succeeds: an **Export as files** button
  appears in the results card head and opens `/ui/sql/export/new?subject=Library/<QQ>`
  with that query pre-checked. Do not save the change.

### 11.6 Failure and Retry

1. On **SQL Queries** → **Create New**, set the Library name to `broken_query`, keep
   the starter `relatedArtifact` but point it at
   `http://example.org/ViewDefinition/patient_demographics` (label `pd`), and set the
   SQL to `SELECT * FROM table_that_does_not_exist`. The preview shows *"Could not run
   the query. …"*; **Save** anyway.
2. Export it (`broken`, NDJSON). The card reaches **Failed**; its detail page shows
   *"The export stopped on subject broken_query: …"* with the SQL error.
3. Click **Retry**: a **new** card is created with the same parameters and fails the
   same way; the original card is untouched. Use **⋮ → Copy job id** on one of them
   (the button shows **Copied**), then **⋮ → Remove from list** on both.
4. Delete `broken_query` on **SQL Queries** (**Delete** → confirm).

### 11.7 Optional S3 sink (`sqlite`/`postgres` rows)

Restart with `HFS_EXPORT_SINK=s3 HFS_EXPORT_S3_BUCKET=hfs-sql-export
HFS_EXPORT_S3_REGION=us-east-1` (MinIO credentials and
`AWS_ENDPOINT_URL=http://localhost:9000`) and repeat 7.a; the pills must be
pre-signed URLs that download.

Pass criteria: every kick-off in 7.a–7.d, 7.g–7.k, and 7.m–7.q produces a card that
reaches **Complete**; the files parse in their declared format with the stated contents and
row counts; 7.e cancels; 7.f and 7.l are rejected; the subjects table controls,
Run again / Retry / Remove / Copy job id, and the restart behave as described. This step is expected to
pass on **all eight backends** (in-DB on SQLite/Postgres/Mongo, in-process on S3).

---

## 12. T8 — Add a subscription and deliver a notification

Uses the R4 backport (the default version is R4): the topic is a `Basic` resource and
the Subscription uses `criteria` + `channel`. Both are created in the Resource Editor;
notifications are triggered by uploading a small batch Bundle on the Batch /
Transaction page.

### 12.1 Start a rest-hook receiver

This is the endpoint HFS delivers to; it is not an HFS API call.

```bash
python3 - <<'PY' > "$WORK/webhook.log" 2>&1 &
import http.server, json, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        print(json.dumps({"path": self.path, "auth": self.headers.get("Authorization"),
                          "body": json.loads(body) if body else None}), flush=True)
        self.send_response(200); self.end_headers()
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", 9999), H).serve_forever()
PY
```

### 12.2 Create the topic and the subscription in the Resource Editor

1. Open `$HFS/ui/editor?type=Basic`, click **Edit raw**, paste, click **Edit raw**
   again (the extension rows render; unknown extension URLs are not errors), then
   **Save Changes** → **Saved.**

   ```json
   {"resourceType":"Basic","id":"manual-topic",
    "code":{"coding":[{"system":"http://hl7.org/fhir/fhir-types","code":"SubscriptionTopic"}]},
    "extension":[
     {"url":"http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url","valueUri":"http://example.org/topics/encounter-start"},
     {"url":"http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.title","valueString":"Encounter created"},
     {"url":"http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger","extension":[
        {"url":"resource","valueUri":"http://hl7.org/fhir/StructureDefinition/Encounter"},
        {"url":"supportedInteraction","valueCode":"create"}]}]}
   ```

2. Open `$HFS/ui/editor?type=Subscription`, **Edit raw**, paste, **Save Changes**:

   ```json
   {"resourceType":"Subscription","id":"manual-sub","status":"requested","reason":"manual matrix",
    "meta":{"profile":["http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-subscription"]},
    "criteria":"http://example.org/topics/encounter-start",
    "channel":{"type":"rest-hook","endpoint":"http://127.0.0.1:9999/webhook","payload":"application/fhir+json",
      "header":["Authorization: Bearer manual-token"],
      "_payload":{"extension":[{"url":"http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content","valueCode":"id-only"}]}}}
   ```

3. Within a couple of seconds `$WORK/webhook.log` gains one line: the **handshake**
   notification, with `"auth": "Bearer manual-token"`.
4. Verify the engine persisted the activation: on **Resources** run
   `GET /Subscription?_id=manual-sub&_elements=status` — the `status` column reads
   **active**. Click the id, open the **History** tab: two versions, and the diff of
   v1 → v2 shows `status: requested → active`.

### 12.3 Trigger and verify delivery

Save this batch Bundle as `$WORK/fixtures/encounters.json`:

```json
{"resourceType":"Bundle","type":"batch","entry":[
 {"request":{"method":"POST","url":"Encounter"},"resource":{"resourceType":"Encounter","status":"in-progress",
  "class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"},"subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"}}},
 {"request":{"method":"POST","url":"Encounter"},"resource":{"resourceType":"Encounter","status":"in-progress",
  "class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"},"subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"}}},
 {"request":{"method":"POST","url":"Encounter"},"resource":{"resourceType":"Encounter","status":"in-progress",
  "class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"},"subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"}}}]}
```

1. **Batch / Transaction** → upload `encounters.json` → `batch · 3 entries` →
   **Execute** → **3 created**. Within a few seconds `$WORK/webhook.log` has **4 lines**
   (1 handshake + 3 event notifications). The last line carries
   `"auth": "Bearer manual-token"` and a `SubscriptionStatus`-style `Parameters`
   whose `events-since-subscription-start` counter reads 3.
2. Non-matching resource: open `$HFS/ui/editor?type=Condition`, **Edit raw**, paste
   `{"resourceType":"Condition","subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"}}`,
   **Save Changes**. The log stays at 4 lines — a Condition does not match the topic.
3. Failure path (used again in T9): kill the receiver (`kill %2` or its pid), upload
   `encounters.json` again, wait ~30 s. Then restart the receiver with the script
   from 12.1 (append to the same log); the queued notifications arrive with backoff.

Pass criteria: the handshake arrives; the stored Subscription flips
`requested → active` with a new version; each created Encounter yields exactly one
notification carrying the configured `Authorization` header; a non-matching resource
does not notify; retries deliver after the receiver returns.

---

## 13. T9 — Subscription activity dashboard

Open **Tools → Subscriptions** (`$HFS/ui/subscriptions`) while `manual-sub` is
active. The page is read-only and does not auto-refresh: reload it to see new figures.

Verify:

1. The four status cards: **Active** = 1 (*delivering*), **Failing** = 0
   (*Needs attention*), **Idle** = 0 (*No clients*), **Delivered in 24 h** = the number
   of notifications delivered in T8 (3, or 6 after the failure path recovered) with a
   *…% first try* sublabel.
2. The table row shows: **Subscription** `manual-sub` with topic short name
   `encounter-start` (hover shows the canonical URL), **Channel** `rest-hook` with
   endpoint `http://127.0.0.1:9999/webhook`, **Status** chip **Active**, **Last 24 hrs**
   a sparkline whose tooltip is the 24-hour count, **Sent** equal to the number of
   Encounters created since the subscription started, **Fail streak** `0`.
3. Upload `encounters.json` once more on **Batch / Transaction**, reload the page:
   **Delivered in 24 h** and **Sent** advance by 3; the sparkline gains a point in the
   current half-hour bucket.
4. Failure path: kill the receiver, upload `encounters.json`, wait ~30 s, reload. The
   chip becomes **Error** (after 3 consecutive failures), the row is highlighted, the
   **Failing** card reads 1, and **Fail streak** counts the failures. Try the **Sort**
   menu (**Status** / **Most sent** / **Fail streak**). Restart the receiver; after the
   retries land, reload: the chip is **Active** again and the streak is `0`.
5. Restart HFS: the engine rehydrates (`HFS_SUBSCRIPTION_REHYDRATE=true`) and the row
   returns as **Active** without re-creating anything. Check the log for
   `Failed to persist subscription status transition` — it must not appear.
6. Negative: start HFS with `HFS_SUBSCRIPTIONS_ENABLED=false` and open the page; it
   renders only the notice *"The subscriptions engine is not enabled on this
   server."* naming `HFS_SUBSCRIPTIONS_ENABLED=true`, and the sidebar entry is still
   present.

Also glance at `$HFS/ui` (the main dashboard) and `/ui/status` after T2–T8: the stat
cards reflect the imported counts plus the resources created in T2, T4, T5, and T8.

Pass criteria: all six checks hold; no browser console errors; the page is usable
without JavaScript (plain reload shows the same figures).

---

## 14. Recording results

For each backend row, attach to the release issue:

- `$WORK/build.log` tail and `hfs-<backend>.log`.
- Screenshots of: the T3 submission detail page in its **Completed** state (status
  card + log), the T2 **Per-Action Outcomes** stage for the transaction, the **SQL
  Exports** list with a completed card, `/ui` after import, and `/ui/subscriptions`
  after T9 step 3.
- The T3 elapsed time and the T7 7.a *finished in* time.
- One downloaded sample from T5 (5.1) and each format from T7.
- For any `❌`: the page, what was entered, the exact on-screen message, and the log
  excerpt, filed as an issue and linked from the matrix cell.

## 15. Known expectations and gotchas

- **Bulk export on MongoDB/S3** returns `501`: the Export page shows a **Failed** card
  reading `kick-off answered 501: bulk export not supported by this backend`. Expected.
- **S3 standalone has no search**: the Resources page cannot run queries on the `s3`
  row (T4 is N/A); `s3-es` searches through Elasticsearch.
- **Transaction Bundles on S3** are refused by design; batch Bundles work.
- **MongoDB** does not support chained or `_has` searches (T4 rows 4.10, 4.11).
- **`near`** is not implemented on any backend and is deliberately absent from T4.
- **`$reindex` on `s3` standalone** returns `501` (no search index). Expected.
- **Elasticsearch composites** are eventually consistent unless
  `HFS_COMPOSITE_SYNC_MODE=synchronous` *and* `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`
  are set, as they are in section 5. Without them T3 counts and T4 may lag.
- **Import page = HFS submitting to itself**: the Data Recipient is `HFS_BASE_URL`,
  and the manifest and files are fetched by the HFS process, so `localhost:8000` must
  be reachable from it. Re-submitting the same manifest URL for the same submission
  is refused with `409 … already submitted`.
- **Batch / Transaction page needs JavaScript** and is file-upload only (no paste);
  the body limit is `HFS_MAX_BODY_SIZE` (10 MiB by default).
- **T2 order matters**: the patient transaction fails until the two reference-data
  batches have run, and after the T3 import its conditional references match two
  Organizations, so it is rejected again (7.5).
- **SQL pages have no Run button**: results follow the editor text with a 500 ms
  delay, capped at 50 rows; the SQL Query preview cannot bind `:parameters` — values
  are supplied on the SQL Export page only.
- **Sidebar FHIR version** must equal the server default (R4) or every `$sql-run`
  preview fails with an explicit message.
- **SQL export job list is per user and per tenant** and lives in the settings
  document; a restart turns in-progress cards into *Cancelled · the server no longer
  knows this job*. Expected.
- **Rest-hook to loopback** works for `id-only` payloads without extra flags;
  `full-resource` payloads require an `https://` endpoint.
  `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS` only affects the messaging channel.
- **One AWS credential chain per process**: with MinIO as the primary store, the S3
  export/sink buckets must also live in MinIO.
- **R6 fixtures**: any cargo build rewrites files under `crates/fhir/tests/data`;
  never `git commit -a` after building.
- Auth stays off for this pass; when auth is on, `$export`, `$bulk-submit`,
  `$sql-export`, `$purge`, and `$reindex` need their `system/*` scopes.
