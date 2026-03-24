# Use Cases and Recipes

Concrete patterns for common HFS workflows, drawn from the CLI tools, Python bindings, and FHIRPath engine.

---

## Building a Clinical ETL Pipeline

**Goal:** Extract patient demographics from a large NDJSON export, transform to Parquet, and load into a data warehouse.

### Step 1 — Write a ViewDefinition

```json
{
  "resourceType": "ViewDefinition",
  "from": {"resourceType": "Patient"},
  "select": [{
    "column": [
      {"name": "id",         "path": "getResourceKey()"},
      {"name": "birth_date", "path": "birthDate"},
      {"name": "gender",     "path": "gender"},
      {"name": "last_name",  "path": "name.where(use='official').family"},
      {"name": "first_name", "path": "name.where(use='official').given.first()"},
      {"name": "zip",        "path": "address.where(use='home').postalCode"}
    ]
  }]
}
```

### Step 2 — Process with pysof (Python)

```python
import pysof

# Most memory-efficient: file-to-file streaming
stats = pysof.process_ndjson_to_file(
    view_definition=open("patient-view.json").read(),
    input_path="export.ndjson",
    output_path="patients.parquet",
    format="parquet"
)
print(f"Wrote {stats['rows_written']} rows")
```

### Step 3 — For very large files: chunked processing

```python
import pysof
import pyarrow as pa
import pyarrow.parquet as pq

writer = None

for chunk in pysof.ChunkedProcessor(
    view_definition=open("patient-view.json").read(),
    ndjson_path="large-export.ndjson",
    chunk_size=1000
):
    table = pa.Table.from_pylist(chunk["rows"])
    if writer is None:
        writer = pq.ParquetWriter("patients.parquet", table.schema)
    writer.write_table(table)

if writer:
    writer.close()
```

### Step 4 — Alternatively, use sof-cli

```bash
sof-cli \
  --view patient-view.json \
  --bundle export.ndjson \
  --format parquet \
  --parquet-compression zstd \
  -o patients.parquet
```

### Incremental loads with `--since`

Re-run the pipeline for only records modified after the last run:

```bash
sof-cli \
  --view patient-view.json \
  --bundle export.ndjson \
  --since 2024-06-01T00:00:00Z \
  --format parquet \
  -o patients-delta.parquet
```

---

## Real-Time Analytics with sof-server

**Goal:** Serve on-demand tabular transforms over an HTTP API for a dashboard or BI tool.

### Start the server

```bash
SOF_SERVER_PORT=8080 sof-server
```

### Send a transform request

```bash
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -d '{
    "_format": "json",
    "_since": "2024-01-01T00:00:00Z",
    "_limit": 500,
    "viewResource": {
      "resourceType": "ViewDefinition",
      "from": {"resourceType": "Observation"},
      "select": [{"column": [
        {"name": "patient",   "path": "subject.reference"},
        {"name": "loinc",     "path": "code.coding.where(system=\"http://loinc.org\").code.first()"},
        {"name": "value",     "path": "value.quantity.value"},
        {"name": "unit",      "path": "value.quantity.unit"},
        {"name": "effective", "path": "effective.dateTime"}
      ]}]
    },
    "resource": [ ... ]
  }'
```

### Parquet output for analytics tools

```bash
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -H "Accept: application/octet-stream" \
  -d '{"_format": "parquet", "viewResource": {...}, "resource": [...]}' \
  -o result.parquet
```

---

## Data Quality Validation with FHIRPath

**Goal:** Validate a set of FHIR resources against clinical invariants.

### Check that every Patient has a birth date

```bash
fhirpath-cli -e "Patient.birthDate.exists()" -r patient.json
```

### Validate a local reference points to a contained resource

```bash
fhirpath-cli -e \
  "reference.startsWith('#').not() or (\$context.reference.substring(1) in \$resource.contained.id)" \
  -r resource.json
```

### Find observations missing a subject

```bash
fhirpath-cli -e "subject.exists().not()" -r observations.json
```

### Validate phone format with a regex

```bash
fhirpath-cli \
  -e "Patient.telecom.where(system='phone').value.all(matches('^[0-9()+\\- ]{7,20}$'))" \
  -r patient.json
```

### Batch validation in a shell loop

```bash
for f in data/*.json; do
  result=$(fhirpath-cli -e "Patient.name.where(use='official').family.exists()" -r "$f")
  if [ "$result" = "false" ]; then
    echo "FAIL: $f is missing an official family name"
  fi
done
```

---

## Patient Cohort Analysis

**Goal:** Identify patients meeting a clinical criterion using FHIRPath.

### Find all systolic blood pressure readings above 140

```fhirpath
Observation.where(
  code.coding.system = 'http://loinc.org' and
  code.coding.code = '8480-6'
).where(value.quantity.value > 140)
```

```bash
fhirpath-cli \
  -e "Observation.where(code.coding.system='http://loinc.org' and code.coding.code='8480-6').where(value.quantity.value > 140)" \
  -r observations.json
```

### Extract patients on active medications

```fhirpath
MedicationRequest.where(status = 'active').subject.reference
```

### Find active diagnoses with a SNOMED code

```fhirpath
Condition.where(
  clinicalStatus.coding.code = 'active' and
  code.coding.system = 'http://snomed.info/sct'
).code.coding.code
```

### ViewDefinition for a cohort report

Combine SQL-on-FHIR to produce a flat cohort table:

```json
{
  "resourceType": "ViewDefinition",
  "from": {"resourceType": "Condition"},
  "select": [{
    "column": [
      {"name": "patient_id",    "path": "subject.reference"},
      {"name": "condition_code","path": "code.coding.where(system='http://snomed.info/sct').code.first()"},
      {"name": "onset",         "path": "onset.dateTime"},
      {"name": "status",        "path": "clinicalStatus.coding.code.first()"}
    ]
  }]
}
```

```bash
sof-cli --view cohort-view.json --bundle conditions.ndjson --format csv -o cohort.csv
```
