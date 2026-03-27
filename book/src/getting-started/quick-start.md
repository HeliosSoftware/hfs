# Quick Start

These examples assume you have the binaries on your `PATH` (from a release archive or `target/release/` after a source build).

## Start the FHIR Server

```bash
# Default: R4, SQLite (in-memory), port 8080
./hfs

# Verify it's running
curl http://localhost:8080/metadata
```

## Evaluate a FHIRPath Expression

```bash
# From a file
./fhirpath-cli -e "Patient.name.family" -r patient.json

# From stdin
echo '{"resourceType": "Patient", "id": "123", "name": [{"family": "Smith"}]}' \
  | ./fhirpath-cli -e "Patient.name.family" -r -
```

## Transform FHIR to CSV (SQL-on-FHIR)

```bash
# From a FHIR Bundle
./sof-cli --view examples/patient-view.json --bundle examples/patients.json

# From an NDJSON file
./sof-cli --view examples/patient-view.json --bundle examples/patients.ndjson

# Output as Parquet
./sof-cli --view examples/patient-view.json --bundle examples/patients.json --format parquet
```

## Start the SQL-on-FHIR HTTP Server

```bash
./sof-server
# POST to http://localhost:8080/ViewDefinition/$viewdefinition-run
```

## Start the FHIRPath HTTP Server

```bash
./fhirpath-server
# POST expressions to http://localhost:3000/
```

## Next Steps

- [Configure storage backends](../configuration/storage-backends.md) for production deployments
- [Explore components](../components/README.md) to understand each tool in depth
- [Configure the server](../configuration/environment-variables.md) with environment variables
