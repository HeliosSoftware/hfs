# FHIRPath

The `helios-fhirpath` crate is a complete implementation of the [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/). It ships two executables and can also be embedded as a library.

## CLI — `fhirpath-cli`

```bash
# Basic expression
fhirpath-cli -e "Patient.name.family" -r patient.json

# With a context expression
fhirpath-cli -c "Patient.name" -e "family" -r patient.json

# With variables
fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0

# Debug parse tree (no resource needed)
fhirpath-cli -e "Patient.name.given.first()" --parse-debug-tree

# Read resource from stdin
cat patient.json | fhirpath-cli -e "Patient.name.family" -r -

# Specify FHIR version explicitly
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r patient.json
```

## HTTP Server — `fhirpath-server`

The server is compatible with [FHIRPath Lab](https://fhirpath-lab.com/).

```bash
# Start with defaults (port 3000)
cargo run --bin fhirpath-server

# Custom configuration
FHIRPATH_SERVER_PORT=8080 FHIRPATH_SERVER_HOST=0.0.0.0 cargo run --bin fhirpath-server
```

### Endpoints

| Method | URL | Description |
|--------|-----|-------------|
| POST | `/` | Evaluate FHIRPath (auto-detects FHIR version) |
| POST | `/r4` | R4-specific evaluation |
| POST | `/r4b` | R4B-specific evaluation |
| POST | `/r5` | R5-specific evaluation |
| POST | `/r6` | R6-specific evaluation |
| GET | `/health` | Health check |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `FHIRPATH_SERVER_PORT` | 3000 | Server port |
| `FHIRPATH_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `FHIRPATH_LOG_LEVEL` | info | Log level |
| `FHIRPATH_ENABLE_CORS` | true | Enable CORS |
| `FHIRPATH_CORS_ORIGINS` | `*` | Allowed origins |
| `FHIRPATH_TERMINOLOGY_SERVER` | *(none)* | Terminology server URL |

## Features

- 100+ built-in functions across all FHIRPath categories
- Parser built with [chumsky](https://github.com/zesterer/chumsky) for excellent error messages
- Comprehensive function library with version-aware type checking
- Auto-detects FHIR version from input data
