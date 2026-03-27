# Quick Start

These examples assume the binaries are on your `PATH` (from a release archive or `target/release/` after a source build).

---

## Running a FHIRPath Expression from the CLI

```bash
# Evaluate an expression against a JSON file
fhirpath-cli -e "Patient.name.family" -r patient.json

# Read from stdin
echo '{"resourceType":"Patient","id":"123","name":[{"family":"Smith"}]}' \
  | fhirpath-cli -e "Patient.name.family" -r -

# Use a context expression to scope the root
fhirpath-cli -c "Patient.name" -e "family" -r patient.json

# Pass a variable
fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0

# Show the parse tree (no resource needed)
fhirpath-cli -e "Patient.name.given.first()" --parse-debug-tree

# Specify FHIR version explicitly
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r patient.json
```

---

## Transforming FHIR Data to CSV with sof-cli

```bash
# Basic transform — FHIR Bundle to CSV (default format)
sof-cli --view patient-view.json --bundle patients.json

# From an NDJSON file
sof-cli --view patient-view.json --bundle patients.ndjson

# Choose output format
sof-cli --view view.json --bundle data.json --format csv
sof-cli --view view.json --bundle data.json --format ndjson
sof-cli --view view.json --bundle data.json --format json
sof-cli --view view.json --bundle data.json --format parquet

# Write to a file instead of stdout
sof-cli --view view.json --bundle data.json --format csv -o output.csv

# Filter by modification time and limit rows
sof-cli --view view.json --bundle data.json --since 2024-01-01T00:00:00Z --limit 100

# Load from S3
sof-cli --view view.json --source s3://my-bucket/fhir-data/patients.ndjson --format csv

# Memory-efficient streaming for large NDJSON files
sof-cli --view view.json --bundle large-data.ndjson --chunk-size 500
```

---

## Starting the SQL-on-FHIR Server

```bash
# Start with defaults (port 8080)
sof-server

# Custom port
SOF_SERVER_PORT=9090 sof-server
```

Send a transform request:

```bash
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -d '{
    "_format": "csv",
    "viewResource": { ... },
    "resource": [ ... ]
  }'
```

---

## Starting the FHIRPath Server

```bash
# Start with defaults (port 3000)
fhirpath-server

# Custom port and host
FHIRPATH_SERVER_PORT=8080 FHIRPATH_SERVER_HOST=0.0.0.0 fhirpath-server
```

Evaluate an expression via HTTP:

```bash
curl -X POST http://localhost:3000/ \
  -H "Content-Type: application/json" \
  -d '{
    "expression": "Patient.name.family",
    "resourceJson": "{\"resourceType\":\"Patient\",\"name\":[{\"family\":\"Smith\"}]}"
  }'
```

Use the version-specific endpoint for explicit version control:

```bash
curl -X POST http://localhost:3000/r4 \
  -H "Content-Type: application/json" \
  -d '{ "expression": "...", "resourceJson": "..." }'
```

---

## Starting the FHIR REST Server

```bash
# Default: R4, SQLite (fhir.db), port 8080
hfs

# Verify the server is running
curl http://localhost:8080/metadata | head -20

# Create a Patient resource
curl -X POST http://localhost:8080/Patient \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Patient","name":[{"family":"Smith","given":["John"]}]}'

# Search for patients by family name
curl "http://localhost:8080/Patient?family=Smith"
```

---

## Next Steps

- [Configure storage backends](configuration/storage-backends.md) for production deployments
- [FHIRPath Expressions](ch05-fhirpath.md) — full CLI and server reference
- [SQL-on-FHIR](ch06-sql-on-fhir.md) — writing ViewDefinitions, all output formats, cloud sources
- [Environment Variables](configuration/environment-variables.md) — tune every server setting
