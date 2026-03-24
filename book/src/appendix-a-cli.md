# Appendix A — CLI Reference

Complete reference for all HFS command-line binaries.

---

## hfs — FHIR REST Server

```bash
hfs
```

No command-line flags. All configuration is via environment variables.

### Environment variables

See the full reference in [Environment Variables](configuration/environment-variables.md). Key variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | `8080` | Port to listen on |
| `HFS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HFS_LOG_LEVEL` | `info` | `error`, `warn`, `info`, `debug`, `trace` |
| `HFS_STORAGE_BACKEND` | `sqlite` | `sqlite`, `sqlite-es`, `postgres`, `pg-es`, `s3`, `s3-es` |
| `HFS_DATABASE_URL` | `fhir.db` | SQLite path or PostgreSQL connection string |
| `HFS_DEFAULT_FHIR_VERSION` | `R4` | `R4`, `R4B`, `R5`, `R6` |
| `HFS_DEFAULT_TENANT` | `default` | Default tenant ID |
| `HFS_TENANT_ROUTING_MODE` | `header_only` | `header_only`, `url_path`, `both` |

### API endpoints

| Interaction | Method | Path |
|-------------|--------|------|
| Capabilities | `GET` | `/metadata` |
| Read | `GET` | `/[type]/[id]` |
| Version read | `GET` | `/[type]/[id]/_history/[vid]` |
| Update | `PUT` | `/[type]/[id]` |
| Patch | `PATCH` | `/[type]/[id]` |
| Delete | `DELETE` | `/[type]/[id]` |
| Create | `POST` | `/[type]` |
| Search (GET) | `GET` | `/[type]?params` |
| Search (POST) | `POST` | `/[type]/_search` |
| Instance history | `GET` | `/[type]/[id]/_history` |
| Type history | `GET` | `/[type]/_history` |
| System history | `GET` | `/_history` |
| Batch / transaction | `POST` | `/` |
| Health | `GET` | `/health` |

---

## fhirpath-cli — FHIRPath Expression Evaluator

```bash
fhirpath-cli [OPTIONS] -e <EXPRESSION>
```

| Flag | Description |
|------|-------------|
| `-e, --expression <EXPR>` | FHIRPath expression to evaluate **(required)** |
| `-r, --resource <FILE>` | Path to FHIR resource JSON file, or `-` to read from stdin |
| `-c, --context <EXPR>` | Context expression to scope evaluation |
| `--var <NAME=VALUE>` | Set an environment variable (repeatable) |
| `--fhir-version <VER>` | FHIR version: `R4`, `R4B`, `R5`, `R6` (default: `R4`) |
| `--parse-debug-tree` | Print the parse tree and exit |
| `--terminology-server <URL>` | Override the default terminology server |
| `-h, --help` | Print help |

### Examples

```bash
fhirpath-cli -e "Patient.name.family" -r patient.json
fhirpath-cli -c "Patient.name" -e "family" -r patient.json
fhirpath-cli -e "value > %threshold" -r obs.json --var threshold=5.0
cat patient.json | fhirpath-cli -e "Patient.name.family" -r -
fhirpath-cli -e "Patient.name.first()" --parse-debug-tree
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r r5-patient.json
```

---

## fhirpath-server — FHIRPath HTTP Server

```bash
fhirpath-server
```

No command-line flags. All configuration is via environment variables.

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `FHIRPATH_SERVER_PORT` | `3000` | Port to listen on |
| `FHIRPATH_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `FHIRPATH_LOG_LEVEL` | `info` | Log level |
| `FHIRPATH_ENABLE_CORS` | `true` | Enable CORS |
| `FHIRPATH_CORS_ORIGINS` | `*` | Allowed CORS origins |
| `FHIRPATH_TERMINOLOGY_SERVER` | *(none)* | Terminology server URL |

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/` | Evaluate (auto-detect version) |
| `POST` | `/r4` | Evaluate as R4 |
| `POST` | `/r4b` | Evaluate as R4B |
| `POST` | `/r5` | Evaluate as R5 |
| `POST` | `/r6` | Evaluate as R6 |
| `GET` | `/health` | Health check |

---

## sof-cli — SQL-on-FHIR CLI

```bash
sof-cli [OPTIONS]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--view <FILE>` | `-v` | *(stdin)* | Path to ViewDefinition JSON file |
| `--bundle <FILE>` | `-b` | *(stdin)* | Path to FHIR Bundle or NDJSON file |
| `--source <URL>` | `-s` | — | URL-based data source (`file://`, `http://`, `s3://`, `gs://`, `azure://`) |
| `--format <FMT>` | `-f` | `csv` | Output format: `csv`, `json`, `ndjson`, `parquet` |
| `--no-headers` | | false | Omit CSV header row |
| `--output <FILE>` | `-o` | *(stdout)* | Output file path |
| `--since <RFC3339>` | | — | Filter resources modified after this time |
| `--limit <N>` | | — | Limit results (1–10000) |
| `--fhir-version <VER>` | | `R4` | FHIR version: `R4`, `R4B`, `R5`, `R6` |
| `--chunk-size <N>` | | 1000 | Chunk size for NDJSON streaming |
| `--skip-invalid` | | false | Skip invalid JSON lines instead of failing |
| `--parquet-row-group-size <MB>` | | 256 | Parquet row group size (64–1024 MB) |
| `--parquet-page-size <KB>` | | 1024 | Parquet page size (64–8192 KB) |
| `--parquet-compression <ALG>` | | `snappy` | `none`, `snappy`, `gzip`, `lz4`, `brotli`, `zstd` |
| `--max-file-size <MB>` | | 1000 | Max Parquet output file size; splits when exceeded |
| `-h, --help` | | | Print help |

### Notes

- `--bundle` / `-b` and `--source` / `-s` are mutually exclusive
- Either `--view` or `--bundle` may be read from stdin, but not both simultaneously
- `--source` accepts `file://`, relative paths, absolute paths, `http://`, `https://`, `s3://`, `gs://`, `azure://`

---

## sof-server — SQL-on-FHIR HTTP Server

```bash
sof-server
```

No command-line flags. All configuration is via environment variables.

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SOF_SERVER_PORT` | `8080` | Port to listen on |
| `SOF_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `SOF_LOG_LEVEL` | `info` | Log level |
| `SOF_MAX_BODY_SIZE` | `10485760` | Max request body (bytes) |
| `SOF_REQUEST_TIMEOUT` | `30` | Request timeout (seconds) |
| `SOF_ENABLE_CORS` | `true` | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed CORS origins |

### Endpoint

`POST /ViewDefinition/$viewdefinition-run`

| Parameter | Type | Description |
|-----------|------|-------------|
| `_format` | string | `csv`, `json`, `ndjson`, `parquet` |
| `header` | boolean | Include CSV header (`true` / `false`) |
| `viewResource` | object | ViewDefinition resource |
| `resource` | array | FHIR resources to transform |
| `patient` | string | Filter by patient reference |
| `_limit` | integer | Limit results (1–10000) |
| `_since` | string | Filter by modification time (RFC3339) |

---

## config-advisor — Storage Configuration Advisor

```bash
config-advisor
```

An interactive CLI tool that helps you choose the optimal storage backend configuration for your workload. No flags. Responds to user input with recommendations and explanations.
