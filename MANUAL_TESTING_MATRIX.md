# HFS Manual Testing Matrix

This document is the manual, end-to-end acceptance pass for the `hfs` binary. It is
organised **backend-first**: every storage backend gets the same sequence of test
procedures, and the results are recorded in the matrix at the top.

Everything below is executed against a build produced exactly the way `ci.yml`
builds it: all FHIR versions, all databases, all features (`--all-features`).

Legend for result cells: `☐` not run · `✅` pass · `❌` fail (link the issue) ·
`N/A` not supported on this backend (expected, see [Expected support](#expected-support-by-backend)).

---

## 1. The matrix

Fill one row per backend per release candidate. Copy this table into the release
issue and replace the `☐` cells.

| Backend (`HFS_STORAGE_BACKEND`) | T0 Build | T1 Start | T2 Import 10k | T3 Search types | T4 Bulk `$export` | T5 ViewDefinition | T6 `$sql-export` (VD / query / view) | T7 Subscription | T8 Activity dashboard |
|---|---|---|---|---|---|---|---|---|---|
| `sqlite` | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `sqlite-es` (SQLite + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `postgres` | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `pg-es` (PostgreSQL + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| `mongodb` | ☐ | ☐ | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `mongo-es` (MongoDB + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `s3` (MinIO) | ☐ | ☐ | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |
| `s3-es` (MinIO + Elasticsearch) | ☐ | ☐ | ☐ | ☐ | N/A (501) | ☐ | ☐ | ☐ | ☐ |

Tester: ______  Commit: ______  Date: ______  OS/arch: ______

### Expected support by backend

Derived from the project skills and the backend capability traits. A cell marked
`N/A` above is a *documented* gap. If a backend that should work returns `501`,
that is a failure.

| Capability | sqlite | sqlite-es | postgres | pg-es | mongodb | mongo-es | s3 | s3-es |
|---|---|---|---|---|---|---|---|---|
| CRUD, history, search | yes | yes | yes | yes | yes | yes | yes | yes |
| Bulk Data `$export` (job store) | yes | yes | yes | yes | no (501) | no (501) | no (501) | no (501) |
| `$bulk-submit` ingestion | yes | yes | yes | yes | yes | yes | yes¹ | yes¹ |
| `$sql-run` / `$sql-export` runner | in-DB | in-DB (primary) | in-DB | in-DB (primary) | in-DB (aggregation) | in-DB (primary) | in-process scan | in-process scan |
| Subscriptions engine | yes | yes | yes | yes | yes | yes | yes | yes |
| `$reindex` | yes | yes | yes | yes | yes | yes | no (501) | yes |
| Per-user UI settings (`/_user/settings`) | yes | yes | yes | yes | yes | yes | yes¹ | yes¹ |

¹ S3 in prefix-per-tenant mode (the default, `HFS_S3_BUCKET`). Bucket-per-tenant
mode with no system bucket returns `501` for `$bulk-submit` and user settings.

---

## 2. Prerequisites

| Tool | Why |
|---|---|
| Rust 1.90+ (edition 2024), `cargo` | build |
| Python 3 with dev headers, `maturin` not required | `--workspace` includes `pysof` (PyO3 cdylib); the build needs a Python interpreter on `PATH` |
| Docker | Postgres, Elasticsearch, MongoDB, MinIO |
| `curl`, `jq`, `tar`, `python3` | import script, API calls, webhook receiver |
| `websocat` (optional) | websocket channel check in T7 |
| ~5 GB free disk | corpus (418 MB tar.gz, ~2.5 GB extracted) plus SQLite/Postgres data |
| A browser | `/ui` pages in T5, T6, T8 |

Shell conventions used below:

```bash
export HFS=http://localhost:8080          # HFS base URL
export FHIR_CT='Content-Type: application/fhir+json'
export WORK=$PWD/manual-test               # scratch dir for corpus, exports, logs
mkdir -p "$WORK"
```

All requests go to the default tenant (`HFS_DEFAULT_TENANT=default`); no
`X-Tenant-ID` header is needed. Authentication stays disabled for this pass.

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

# MinIO (s3, s3-es, and the S3 output-backend variants of T4/T6)
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
recreate. For SQLite delete `data/hfs.db*` and `data/bulk_export.db*`.

---

## 5. T1 — Start HFS

### Common environment (every backend)

```bash
export HFS_SERVER_HOST=127.0.0.1 HFS_SERVER_PORT=8080 HFS_BASE_URL=http://localhost:8080
export HFS_LOG_LEVEL=info
export HFS_DEFAULT_FHIR_VERSION=R4
export HFS_MAX_BODY_SIZE=104857600        # Synthea transaction bundles exceed the 10 MB default
export HFS_REQUEST_TIMEOUT=600            # large bundles on composite backends
export HFS_SUBSCRIPTIONS_ENABLED=true
export HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true   # rest-hook to 127.0.0.1 (T7)
export HFS_BULK_EXPORT_OUTPUT_DIR=$WORK/bulk-exports  # T4 local-fs output
export HFS_EXPORT_DIR=$WORK/sql-exports               # T6 fs sink
# composites: make searches read-your-write so T2 counts and T3 are deterministic
export HFS_COMPOSITE_SYNC_MODE=synchronous HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for
```

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
primary store means the T4/T6 S3 *output* variants must also target MinIO.

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
| jq .fhirVersion` should report `5.0.0`.

---

## 6. T2 — Import 10,000 FHIR resources from the Synthea corpus

The corpus is the same one the `fhir-benchmark.yml` workflow pulls: the public
Synthea `bulk_1k` archive of 1,000 patients as R4 transaction Bundles
(`hospitalInformation.json`, `practitionerInformation.json`, one bundle per patient).

> The archive lives in a public Google Cloud Storage bucket. If the team mirrors it
> to an S3 bucket, replace `CORPUS_URL` below (an `s3://` URL works with
> `aws s3 cp` in place of `curl`). The rest of the procedure is unchanged.

```bash
CORPUS_URL=https://storage.googleapis.com/aidbox-public/synthea/performance/bulk_1k.tar.gz
mkdir -p "$WORK/corpus" && cd "$WORK/corpus"
[ -f bulk_1k.tar.gz ] || curl -L -o bulk_1k.tar.gz "$CORPUS_URL"      # 418 MB
tar -xzf bulk_1k.tar.gz
find . -name '*.json' | wc -l                                          # ~1,002 bundles
```

Load bundles as FHIR transactions until at least 10,000 resources have been
accepted. Provider/organisation bundles go first so patient bundles can resolve
their references.

```bash
cat > "$WORK/import.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
HFS=${HFS:-http://localhost:8080}; TARGET=${TARGET:-10000}; total=0; files=0
cd "$(dirname "$0")/corpus"
ordered=$( { find . -name 'hospitalInformation*.json'; find . -name 'practitionerInformation*.json'; \
             find . -name '*.json' ! -name 'hospitalInformation*' ! -name 'practitionerInformation*' | sort; } )
for f in $ordered; do
  n=$(jq '.entry | length' "$f")
  code=$(curl -s -o "$WORK/last-response.json" -w '%{http_code}' -X POST "$HFS/" \
         -H 'Content-Type: application/fhir+json' -H 'Accept: application/fhir+json' --data-binary @"$f")
  if [ "$code" != "200" ]; then echo "FAIL $f -> HTTP $code"; jq -c '.issue[0]' "$WORK/last-response.json"; exit 1; fi
  ok=$(jq '[.entry[].response.status | select(startswith("201") or startswith("200"))] | length' "$WORK/last-response.json")
  total=$((total+ok)); files=$((files+1))
  printf '%5d files %7d resources (last: %s, %d entries)\n' "$files" "$total" "$(basename "$f")" "$n"
  [ "$total" -ge "$TARGET" ] && break
done
echo "IMPORTED $total resources from $files bundles"
SH
chmod +x "$WORK/import.sh"
time WORK=$WORK "$WORK/import.sh" | tee "$WORK/import-$HFS_STORAGE_BACKEND.log"
```

Verify the load landed and is searchable:

```bash
for t in Patient Encounter Observation Condition Procedure Claim Organization Practitioner; do
  printf '%-14s %s\n' $t "$(curl -s "$HFS/$t?_summary=count" | jq .total)"
done
curl -s "$HFS/Patient?_count=1" | jq '.entry[0].resource.id'   # keep one id handy:
export PID=$(curl -s "$HFS/Patient?_count=1" | jq -r '.entry[0].resource.id')
```

Record in the matrix: wall-clock time of the import, number of bundles, resource
total, and any bundle that failed. Then open `$HFS/ui` — the dashboard stat cards and
the resources-over-time chart must reflect the counts above.

Pass criteria: every bundle returns `200` with a `transaction-response`; summed
resource total ≥ 10,000; counts via `_summary=count` are non-zero for the types above;
on composites, the Elasticsearch indices (`curl localhost:9200/_cat/indices/hfs*`)
show matching document counts.

Optional alternative import path (exercises `$bulk-submit` instead of transactions):
serve an NDJSON manifest from a local `python3 -m http.server` and POST a
`$bulk-submit` `Parameters` with `manifestUrl`/`fhirBaseUrl`, then poll
`/bulk-submit-status/{token}`. See `/bulk-data-submit` for the exact parameters.
Not required for the matrix.

---

## 7. T3 — One manual search per FHIR search type

Run each against the loaded corpus. Confirm `total` is > 0 (or exactly the expected
count where stated) and spot-check that the returned resources actually satisfy the
criterion. `PID` is the patient id captured in T2.

| # | Search type | Request | What to verify |
|---|---|---|---|
| 3.1 | **string** | `GET /Patient?name=a` then `GET /Patient?family:exact=<a family name from 3.1>` | prefix match vs exact-modifier count differs sensibly; `Patient?name:contains=an` returns ≥ prefix count |
| 3.2 | **token** | `GET /Patient?gender=female` and `GET /Observation?code=http://loinc.org\|8302-2` | system\|code form works; `Observation?code=8302-2` (code only) returns the same total; `Patient?gender:not=female` is the complement |
| 3.3 | **date** | `GET /Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01` and `GET /Encounter?date=ge2015` | prefixes `ge/lt`; year-precision `date=ge2015` matches the whole year; `_lastUpdated=gt2020-01-01` returns everything imported today |
| 3.4 | **number** | first `POST /RiskAssessment` with the fixture below, then `GET /RiskAssessment?probability=gt0.5` and `?probability=lt0.5` | 1 hit and 0 hits respectively; `probability=ap0.8` matches |
| 3.5 | **quantity** | `GET /Observation?value-quantity=gt150` and `GET /Observation?value-quantity=gt150\|\|cm` | unit-qualified form is a subset of the unqualified one; returned `valueQuantity.value` > 150 |
| 3.6 | **reference** | `GET /Observation?subject=Patient/$PID`, `GET /Condition?patient=$PID`, `GET /Encounter?subject=$PID&_include=Encounter:subject` | all results reference the patient; `_include` adds exactly one `Patient` entry with `search.mode=include` |
| 3.7 | **uri** | `POST /ValueSet` fixture below, then `GET /ValueSet?url=http://example.org/fhir/ValueSet/manual-test` and `?url:below=http://example.org/fhir` | exact match = 1; `:below` ≥ 1 |
| 3.8 | **composite** | `GET /Observation?code-value-quantity=http://loinc.org\|8302-2$gt150` | every hit is a body-height observation with value > 150 |
| 3.9 | **special** | `GET /Location?near=42.36\|-71.06\|100\|km` (Synthea Locations carry `position`) | non-zero for Massachusetts corpora; `Location?near=0\|0\|1\|km` is 0 |
| 3.10 | **chained** | `GET /Observation?subject.name=<family from 3.1>&_count=5` | subjects match the name |
| 3.11 | **reverse chained** | `GET /Patient?_has:Observation:patient:code=http://loinc.org\|8302-2&_count=5` | every patient has ≥ 1 body-height observation |
| 3.12 | **_revinclude / _sort / paging** | `GET /Patient?_id=$PID&_revinclude=Condition:patient`; `GET /Observation?patient=$PID&_sort=-date&_count=5`; follow `link[rel=next]` on `GET /Patient?_count=20&_total=accurate` | conditions come back as `include`; dates descend; `next` link paginates and `total` is stable across pages |
| 3.13 | **_text / _content** (string full-text) | `GET /Patient?_content=<a city name>` | hits; on composites confirm in the log that Elasticsearch served the search |
| 3.14 | **POST search** | `POST /Patient/_search` with body `gender=male&_count=3` (`application/x-www-form-urlencoded`) | same shape as GET |

Fixtures for 3.4 and 3.7:

```bash
curl -s -X POST $HFS/RiskAssessment -H "$FHIR_CT" -d '{"resourceType":"RiskAssessment","status":"final",
  "subject":{"reference":"Patient/'$PID'"},"prediction":[{"probabilityDecimal":0.8}]}' | jq .id
curl -s -X POST $HFS/ValueSet -H "$FHIR_CT" -d '{"resourceType":"ValueSet","status":"active",
  "url":"http://example.org/fhir/ValueSet/manual-test","name":"ManualTest"}' | jq .id
```

Pass criteria: every row produces the expected shape and non-empty result; no `500`;
on composites, the ES query path is used (log at `HFS_LOG_LEVEL=debug` if in doubt).
Also run 3.1–3.3 from `/ui/queries` (visual search builder) and `/ui/resources` to
confirm the UI builds the same URLs.

---

## 8. T4 — Bulk Data `$export` in every output format

Bulk Data export accepts exactly one output format, `application/fhir+ndjson`, under
three spellings. All three must be accepted and any other value rejected with `400`.
The *output backend* (`local-fs` vs `s3`) is the other axis worth covering.

```bash
kick() { curl -s -D - -o /dev/null -H 'Prefer: respond-async' -H 'Accept: application/fhir+json' "$@" | grep -i '^content-location' | tr -d '\r' | cut -d' ' -f2; }

# 4.1 system-level, default format
STATUS=$(kick "$HFS/\$export")
# 4.2 patient-level, explicit MIME format, restricted types
STATUS_P=$(kick "$HFS/Patient/\$export?_outputFormat=application/fhir%2Bndjson&_type=Patient,Observation")
# 4.3 group-level, short alias, _typeFilter and _elements
GID=$(curl -s -X POST $HFS/Group -H "$FHIR_CT" -d '{"resourceType":"Group","type":"person","actual":true,
   "member":[{"entity":{"reference":"Patient/'$PID'"}}]}' | jq -r .id)
STATUS_G=$(kick "$HFS/Group/$GID/\$export?_outputFormat=ndjson&_type=Patient,Condition&_typeFilter=Condition%3Fclinical-status%3Dactive&_elements=id,code")
# 4.4 the third alias
kick "$HFS/\$export?_outputFormat=application/ndjson&_type=Organization"
# 4.5 negative: unsupported format -> 400 OperationOutcome
curl -s -o - -w '\n%{http_code}\n' -H 'Prefer: respond-async' "$HFS/\$export?_outputFormat=application/parquet"

# poll (202 + X-Progress while running, 200 + manifest when done)
until curl -s -o "$WORK/manifest.json" -w '%{http_code}' "$STATUS" | grep -q 200; do sleep 2; done
jq '{transactionTime, requiresAccessToken, outputs: [.output[] | {type, url}]}' "$WORK/manifest.json"
# download one file and count lines
URL=$(jq -r '.output[] | select(.type=="Patient") | .url' "$WORK/manifest.json" | head -1)
curl -s "$URL" -o "$WORK/patient.ndjson"; wc -l "$WORK/patient.ndjson"; head -c 300 "$WORK/patient.ndjson"
# cancel/delete
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$STATUS_G"
```

Repeat 4.1 with the S3 output backend on `sqlite` and `postgres` rows (restart HFS
with `HFS_BULK_EXPORT_OUTPUT_BACKEND=s3 HFS_BULK_EXPORT_S3_BUCKET=hfs-export
HFS_BULK_EXPORT_S3_ENDPOINT=http://localhost:9000 HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE=true
HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN=false` plus the MinIO credentials). Manifest URLs
must then be pre-signed MinIO URLs that download.

Also drive one export from the UI: `/ui/bulk-export/new`, then watch the card on
`/ui/bulk-export/active` transition to complete and download a file.

Pass criteria: 4.1–4.4 return `202` + `Content-Location`; manifests list one output
per type; the Patient NDJSON line count equals `Patient?_summary=count`; the group
export contains only the one patient and its active conditions with `SUBSETTED` tag;
4.5 is `400`; on `mongodb`/`s3` rows all kick-offs return `501` (record as N/A).

---

## 9. T5 — Create a ViewDefinition and examine its output

### 9.1 Create via API

```bash
cat > "$WORK/vd-patient.json" <<'JSON'
{ "resourceType": "ViewDefinition", "url": "http://example.org/ViewDefinition/patient_demographics",
  "name": "patient_demographics", "status": "active", "resource": "Patient",
  "select": [ { "column": [
      { "name": "id",        "path": "getResourceKey()", "type": "id" },
      { "name": "gender",    "path": "gender" },
      { "name": "birth_date","path": "birthDate", "type": "date" },
      { "name": "family",    "path": "name.first().family" },
      { "name": "city",      "path": "address.first().city" } ] } ],
  "where": [ { "path": "active.exists().not() or active = true" } ] }
JSON
VD=$(curl -s -X POST $HFS/ViewDefinition -H "$FHIR_CT" -d @"$WORK/vd-patient.json" | jq -r .id); echo $VD
curl -s "$HFS/ViewDefinition?url=http://example.org/ViewDefinition/patient_demographics" | jq .total   # 1
```

### 9.2 Examine output with `$sql-run`, in every format

```bash
run() { curl -s -X POST "$HFS/\$sql-run" -H "$FHIR_CT" -d "{\"resourceType\":\"Parameters\",\"parameter\":[
  {\"name\":\"subjectReference\",\"valueReference\":{\"reference\":\"ViewDefinition/$VD\"}},
  {\"name\":\"_format\",\"valueCode\":\"$1\"},{\"name\":\"_limit\",\"valueInteger\":5}]}" "${@:2}"; }
run json    | jq .                           # array of 5 rows, 5 columns
run csv     | head                           # header row + 5 lines
run ndjson  | wc -l                          # 5
run parquet -o "$WORK/patients.parquet"; python3 -c "import pyarrow.parquet as p;print(p.read_table('$WORK/patients.parquet').schema)" 2>/dev/null || file "$WORK/patients.parquet"
# patient filter
curl -s -X POST "$HFS/\$sql-run" -H "$FHIR_CT" -d '{"resourceType":"Parameters","parameter":[
  {"name":"subjectReference","valueReference":{"reference":"ViewDefinition/'$VD'"}},
  {"name":"patient","valueString":"Patient/'$PID'"},{"name":"_format","valueCode":"json"}]}' | jq length   # 1
```

Cross-check a row against the source: pick an `id` from the JSON output and confirm
`GET /Patient/{id}` has the same `gender`, `birthDate`, and family name.

### 9.3 Create a second ViewDefinition in the UI editor

Open `/ui/sql/view-definitions`, click *Create New*, and build an Observation view
(`resource: Observation`, columns `id`, `patient_id` = `subject.getReferenceKey(Patient)`,
`code` = `code.coding.first().code`, `value` = `value.ofType(Quantity).value`,
`effective` = `effective.ofType(dateTime)`), named `observation_flat` with url
`http://example.org/ViewDefinition/observation_flat`.

- Introduce a typo (`"colum"`) and confirm the lint panel flags it with a fix.
- Use completion in the FHIRPath field (`getRes…`) and confirm it offers `getResourceKey()`.
- Press *Run* — the results grid shows Observation rows.
- *Save* — reload `/ui/sql/view-definitions?vd={id}` and confirm it persisted; capture `VD2`.

Pass criteria: API create returns `201`; canonical search finds it; all four
`$sql-run` formats return well-formed output with the expected row count; the parquet
file has a schema with the five columns; the UI lint/complete/run/save cycle works.

---

## 10. T6 — `$sql-export` with a ViewDefinition, a SQL query, and a SQL view

`$sql-export` is asynchronous and takes one or more `subject` parts. A subject may be a
**ViewDefinition**, a **SQLQuery Library**, or a **SQLView Library**. Cover each kind,
and every output format (`ndjson` default, `csv`, `json`, `parquet`).

### 10.1 Create the Library subjects (API or `/ui/sql/queries` and `/ui/sql/views`)

```bash
b64() { printf '%s' "$1" | base64 | tr -d '\n'; }
LIB_TYPES=http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes
# SQLView: a query whose result is a table others can select from
QV=$(curl -s -X POST $HFS/Library -H "$FHIR_CT" -d '{"resourceType":"Library","name":"female_patients","status":"active",
 "url":"http://example.org/Library/female_patients",
 "type":{"coding":[{"system":"'$LIB_TYPES'","code":"sql-view"}]},
 "relatedArtifact":[{"type":"depends-on","resource":"http://example.org/ViewDefinition/patient_demographics","label":"pd"}],
 "content":[{"contentType":"application/sql","data":"'$(b64 "SELECT id, birth_date, city FROM pd WHERE gender = 'female'")'"}]}' | jq -r .id)
# SQLQuery: a query over a view definition and the view above, with a bound parameter
QQ=$(curl -s -X POST $HFS/Library -H "$FHIR_CT" -d '{"resourceType":"Library","name":"tall_female_patients","status":"active",
 "url":"http://example.org/Library/tall_female_patients",
 "type":{"coding":[{"system":"'$LIB_TYPES'","code":"sql-query"}]},
 "relatedArtifact":[{"type":"depends-on","resource":"http://example.org/ViewDefinition/observation_flat","label":"obs"},
                    {"type":"depends-on","resource":"http://example.org/Library/female_patients","label":"fp"}],
 "parameter":[{"name":"min_height","use":"in","type":"decimal"}],
 "content":[{"contentType":"application/sql","data":"'$(b64 "SELECT fp.id, fp.city, MAX(obs.value) AS height FROM fp JOIN obs ON obs.patient_id = fp.id WHERE obs.code = '8302-2' AND obs.value > :min_height GROUP BY fp.id, fp.city")'"}]}' | jq -r .id)
echo "view=$QV query=$QQ"
```

Sanity-run both with `$sql-run` (`subjectReference` → `Library/$QV`; for the query add
`{"name":"parameters","resource":{"resourceType":"Parameters","parameter":[{"name":"min_height","valueDecimal":150}]}}`
inside the subject) and confirm rows come back. Also open both in `/ui/sql/views` and
`/ui/sql/queries`: the SQL pane must show the decoded SQL, and *Run* returns rows.

### 10.2 Kick off one export per subject kind, cycling the formats

```bash
sqlexport() { # $1 = format, rest = subject part JSON fragments
  local subjects="${*:2}"
  curl -s -D "$WORK/hdr" -o "$WORK/body.json" -X POST "$HFS/\$sql-export" -H "$FHIR_CT" -H 'Prefer: respond-async' \
    -d "{\"resourceType\":\"Parameters\",\"parameter\":[{\"name\":\"_format\",\"valueCode\":\"$1\"},$subjects]}"
  grep -i '^content-location' "$WORK/hdr" | tr -d '\r' | cut -d' ' -f2; }
SUBJ_VD='{"name":"subject","part":[{"name":"name","valueString":"patients"},{"name":"subjectReference","valueReference":{"reference":"ViewDefinition/'$VD'"}}]}'
SUBJ_VIEW='{"name":"subject","part":[{"name":"name","valueString":"female_patients"},{"name":"subjectReference","valueReference":{"reference":"Library/'$QV'"}}]}'
SUBJ_QUERY='{"name":"subject","part":[{"name":"name","valueString":"tall"},{"name":"subjectReference","valueReference":{"reference":"Library/'$QQ'"}},{"name":"parameters","resource":{"resourceType":"Parameters","parameter":[{"name":"min_height","valueDecimal":150}]}}]}'

J1=$(sqlexport ndjson  "$SUBJ_VD")                          # 6.a ViewDefinition, ndjson
J2=$(sqlexport csv     "$SUBJ_QUERY")                       # 6.b SQL query, csv
J3=$(sqlexport parquet "$SUBJ_VIEW")                        # 6.c SQL view, parquet
J4=$(sqlexport json    "$SUBJ_VD,$SUBJ_QUERY,$SUBJ_VIEW")   # 6.d all three in one job, json
J5=$(sqlexport ndjson  "$SUBJ_VD"); curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$J5"   # 6.e cancel; a later poll of $J5 is 404
# negative: missing Prefer header -> 400 ; `_limit` -> 400 ; `parameters` on a ViewDefinition -> 400
curl -s -o /dev/null -w '%{http_code}\n' -X POST "$HFS/\$sql-export" -H "$FHIR_CT" -d '{"resourceType":"Parameters","parameter":['"$SUBJ_VD"']}'

for J in $J1 $J2 $J3 $J4; do
  until [ "$(curl -s -o /dev/null -w '%{http_code}' "$J")" = 303 ]; do sleep 2; done
  RES=$(curl -s -o /dev/null -w '%{redirect_url}' "$J")
  curl -s "$RES" | jq -c '[.parameter[] | select(.name=="output") | {name: (.part[]|select(.name=="name").valueString), url: (.part[]|select(.name=="url").valueUri)}]'
done
```

Download each output URL (`/export/{job-id}/{filename}`) and inspect:

- ndjson: `wc -l` equals `Patient?_summary=count` (minus inactive patients).
- csv: header row matches the column names; open in a spreadsheet.
- parquet: readable with `pyarrow`/`duckdb`; schema has `id, birth_date, city`.
- json: one array per subject; the combined job (6.d) yields three output entries
  named `patients`, `tall`, `female_patients`.
- `tall` rows all have `height > 150` and belong to female patients (join-check two ids).

### 10.3 The same from the UI

`/ui/sql/export/new`: pick the ViewDefinition, the SQL query (fill `min_height`), and
the SQL view as subjects, choose *csv* with header on, kick off, then watch
`/ui/sql/export` — the card polls to *complete* and lists downloadable outputs.
Use *Run again* on the card and *Remove* on the finished copy. Restart HFS and confirm
the old cards resolve to *cancelled* with the reaper explanation (not an error).

Optional S3 sink on `sqlite`/`postgres`: restart with `HFS_EXPORT_SINK=s3
HFS_EXPORT_S3_BUCKET=hfs-sql-export HFS_EXPORT_S3_REGION=us-east-1` (MinIO credentials
and `AWS_ENDPOINT_URL=http://localhost:9000`) and repeat 6.a; output URLs must be
pre-signed and downloadable.

Pass criteria: every kick-off is `202` + `Content-Location`; polls go `202` → `303`;
result manifests list the expected outputs; files parse in their declared format;
negative cases are `400`; cancel makes the status URL `404`. This step is expected to
work on **all eight backends** (in-DB on SQLite/Postgres/Mongo, in-process on S3);
record the wall-clock time of 6.a per backend.

---

## 11. T7 — Add a subscription and deliver a notification

Uses the R4 backport (the default version is R4). If the row is also checked on R5
(`HFS_DEFAULT_FHIR_VERSION=R5`), use the native `SubscriptionTopic`/`Subscription`
shapes from `crates/hfs/tests/subscriptions/run_external_subscriptions_smoke.sh`.

### 11.1 Start a rest-hook receiver

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

### 11.2 Create the topic (R4 backport: a `Basic` resource) and the subscription

```bash
TOPIC=http://example.org/topics/encounter-start
curl -s -o /dev/null -w '%{http_code}\n' -X POST $HFS/Basic -H "$FHIR_CT" -d '{"resourceType":"Basic",
 "code":{"coding":[{"system":"http://hl7.org/fhir/fhir-types","code":"SubscriptionTopic"}]},
 "extension":[
  {"url":"http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url","valueUri":"'$TOPIC'"},
  {"url":"http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.title","valueString":"Encounter created"},
  {"url":"http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger","extension":[
     {"url":"resource","valueUri":"http://hl7.org/fhir/StructureDefinition/Encounter"},
     {"url":"supportedInteraction","valueCode":"create"}]}]}'

SUB=$(curl -s -X POST $HFS/Subscription -H "$FHIR_CT" -d '{"resourceType":"Subscription","status":"requested",
 "reason":"manual matrix","meta":{"profile":["http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-subscription"]},
 "criteria":"'$TOPIC'",
 "channel":{"type":"rest-hook","endpoint":"http://127.0.0.1:9999/webhook","payload":"application/fhir+json",
   "header":["Authorization: Bearer manual-token"],
   "_payload":{"extension":[{"url":"http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content","valueCode":"id-only"}]}}}' | jq -r .id)
sleep 2
cat "$WORK/webhook.log"                     # handshake notification received
curl -s "$HFS/Subscription/$SUB" | jq .status                    # "active" (persisted by the engine)
curl -s "$HFS/Subscription/$SUB/\$status" | jq .                 # Parameters (R4 backport) status=active
```

### 11.3 Trigger and verify delivery

```bash
for i in 1 2 3; do curl -s -o /dev/null -X POST $HFS/Encounter -H "$FHIR_CT" -d '{"resourceType":"Encounter","status":"in-progress",
  "class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"},"subject":{"reference":"Patient/'$PID'"}}'; done
sleep 3; wc -l "$WORK/webhook.log"          # 1 handshake + 3 event notifications
tail -1 "$WORK/webhook.log" | jq '.auth, .body.entry[0].resource.parameter'   # bearer header forwarded; event-notification with events-since-start counter
curl -s "$HFS/Subscription/$SUB/\$events" | jq .        # event log lists 3 events
curl -s -o /dev/null -X POST $HFS/Condition -H "$FHIR_CT" -d '{"resourceType":"Condition","subject":{"reference":"Patient/'$PID'"}}'
sleep 2; wc -l "$WORK/webhook.log"          # unchanged: Condition does not match the topic
```

Failure path: kill the receiver, create two more Encounters, wait ~30 s, confirm the
subscription's error streak grows (visible in T8) and, after restarting the receiver,
retries with backoff deliver the queued notifications.

Optional: websocket channel with `websocat` (`GET /Subscription/{id}/$get-ws-binding-token`,
then connect to `/ws` and send `bind-with-token`) per the smoke script.

Pass criteria: handshake arrives; status flips `requested` → `active` in storage
(check `_history` shows the version bump); each Encounter create yields exactly one
notification with the configured `Authorization` header; non-matching resources do
not notify; `$status` and `$events` answer.

---

## 12. T8 — Subscription activity dashboard

Open `$HFS/ui/subscriptions` while the subscription from T7 is active.

Verify:

1. The four status cards: **active** = 1, **failing** = 0 (or 1 during the failure
   path), **idle** = 0, **delivered in 24 h** = number of notifications sent in T7.
2. The table row for the subscription shows: topic short name `encounter-start`
   (hover shows the canonical URL), channel `rest-hook`, endpoint
   `http://127.0.0.1:9999/webhook`, status chip `active`, the event counter equal to
   `$events`, the *Last 24 hrs* count and a sparkline, and the failure streak `0`.
3. Create three more Encounters, reload the page: delivered-24h and the event counter
   advance; the sparkline gains a point in the current half-hour bucket.
4. Run the failure path from T7: the chip becomes `error`, the *failing* card becomes 1
   and the streak column counts consecutive failures; recovering the receiver clears it.
5. Restart HFS: the engine rehydrates (`HFS_SUBSCRIPTION_REHYDRATE=true`) and the row
   returns as `active` without re-creating anything. Check the log for
   `Failed to persist subscription status transition` — it must not appear.
6. Negative: start HFS with `HFS_SUBSCRIPTIONS_ENABLED=false` and open the page; it
   renders the explained *unavailable* state naming the env var, and the sidebar entry
   is still present.

Also glance at `$HFS/ui` (the main dashboard) and `/ui/status` after T2–T7: stat cards
reflect the imported counts plus the resources created in T3, T4, T7.

Pass criteria: all six checks hold; no browser console errors; the page is usable
without JavaScript (plain reload shows the same figures).

---

## 13. Recording results

For each backend row, attach to the release issue:

- `$WORK/build.log` tail, `hfs-<backend>.log`, `import-<backend>.log`.
- The T2 timing line (`IMPORTED n resources from m bundles`, plus `real` time).
- One sample of each T4 manifest and each T6 result manifest.
- Screenshots of `/ui` after import, `/ui/sql/export` with a completed card, and
  `/ui/subscriptions` after T8 step 3.
- For any `❌`: the request, the response body (OperationOutcome), and the log excerpt,
  filed as an issue and linked from the matrix cell.

## 14. Known expectations and gotchas

- **Bulk export on MongoDB/S3** returns `501`: no job store exists yet. Expected.
- **`$reindex` on `s3` standalone** returns `501` (no search index). Expected.
- **Elasticsearch composites** are eventually consistent unless
  `HFS_COMPOSITE_SYNC_MODE=synchronous` *and* `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`
  are set, as they are in section 5. Without them T2 counts and T3 may lag.
- **Body size and timeout**: Synthea bundles exceed the 10 MB default body limit; the
  import will fail with `413` unless `HFS_MAX_BODY_SIZE` is raised.
- **`$sql-export` job list is per-process**: the UI's list comes from the per-user
  settings document; a restart turns old cards into *cancelled*. Expected.
- **Rest-hook to loopback** is blocked unless
  `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true`.
- **One AWS credential chain per process**: with MinIO as the primary store, the S3
  export/sink buckets must also live in MinIO.
- **R6 fixtures**: any cargo build rewrites files under `crates/fhir/tests/data`;
  never `git commit -a` after building.
- Auth stays off for this pass; when auth is on, `$export`, `$bulk-submit`,
  `$sql-export`, `$purge`, and `$reindex` need their `system/*` scopes.
