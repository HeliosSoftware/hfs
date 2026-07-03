---
name: work-with-fhirpath
description: Work on FHIRPath parser, evaluator, CLI, HTTP server, expressions, terminology integration, or FHIRPath tests. Use for fhirpath-cli, fhirpath-server, FHIRPATH_* configuration, parser debug output, and expression evaluation behavior.
---

# FHIRPath

Use this when working in `helios-fhirpath`, `helios-fhirpath-support`, the FHIRPath CLI, or the FHIRPath HTTP server.

## CLI Usage

```bash
# Basic expression evaluation
cargo run --bin fhirpath-cli -- -e "Patient.name.family" -r patient.json

# With context expression
cargo run --bin fhirpath-cli -- -c "Patient.name" -e "family" -r patient.json

# With variables
cargo run --bin fhirpath-cli -- -e "value > %threshold" -r observation.json --var threshold=5.0

# Parse debug tree, no resource needed
cargo run --bin fhirpath-cli -- -e "Patient.name.given.first()" --parse-debug-tree

# Read from stdin
cat patient.json | cargo run --bin fhirpath-cli -- -e "Patient.name.family" -r -

# Specify FHIR version
cargo run --bin fhirpath-cli -- --fhir-version R5 -e "Patient.name.family" -r patient.json
```

## HTTP Server

The FHIRPath server is compatible with fhirpath-lab.

```bash
# Start with defaults, port 3000
cargo run --bin fhirpath-server

# Custom configuration
FHIRPATH_SERVER_PORT=8080 FHIRPATH_SERVER_HOST=0.0.0.0 cargo run --bin fhirpath-server
```

| Method | URL | Description |
|---|---|---|
| POST | `/` | Evaluate FHIRPath and auto-detect FHIR version |
| POST | `/r4`, `/r4b`, `/r5`, `/r6` | Version-specific evaluation |
| GET | `/health` | Health check |

## Environment

| Variable | Default | Description |
|---|---|---|
| `FHIRPATH_SERVER_PORT` | `3000` | Server port |
| `FHIRPATH_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `FHIRPATH_LOG_LEVEL` | `info` | Log level |
| `FHIRPATH_ENABLE_CORS` | `true` | Enable CORS |
| `FHIRPATH_CORS_ORIGINS` | `*` | Allowed origins |
| `FHIRPATH_MAX_BODY_SIZE` | `10485760` | Max request body size in bytes, measured after decompression |
| `FHIRPATH_TERMINOLOGY_SERVER` | none | Terminology server URL |

Request bodies with gzip, deflate, br, or zstd `Content-Encoding` are decompressed before parsing. Unsupported encodings return `415`. Responses are compressed when the client sends `Accept-Encoding`.

## Tests

- FHIRPath test cases live under `crates/fhirpath/tests/`.
- Official FHIR test cases come from the `fhir-test-cases` repository.
- Test expressions independently with `fhirpath-cli` before debugging broader server behavior.
