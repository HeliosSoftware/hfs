---
name: work-with-sof
description: Work on SQL-on-FHIR, ViewDefinition processing, sof-cli, sof-server, SOF HTTP configuration, output formats, or parquet export. Use for helios-sof changes, ViewDefinitionTrait behavior, transformation tests, and SOF server endpoints.
---

# SQL-on-FHIR

Use this when working in `helios-sof`, `sof-cli`, `sof-server`, or ViewDefinition transformation code.

## Server Environment

| Variable | Default | Description |
|---|---|---|
| `SOF_SERVER_PORT` | `8080` | Server port |
| `SOF_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `SOF_LOG_LEVEL` | `info` | Log level |
| `SOF_MAX_BODY_SIZE` | `10485760` | Max request body size in bytes, applied after decompression |
| `SOF_REQUEST_TIMEOUT` | `30` | Request timeout in seconds |
| `SOF_ENABLE_CORS` | `true` | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed origins |
| `SOF_TERMINOLOGY_SERVER` | none | Terminology server URL for FHIRPath memberOf and subsumes |

## API Endpoints

- `GET /metadata`: return CapabilityStatement.
- `GET /health`: health check.
- `POST /$sql-run`: execute ViewDefinition transformation.

`$sql-run` accepts these parameters in the request body or query:

- `_format`: csv, ndjson, json, or parquet.
- `header`: CSV header control, true or false.
- `subjectResource`: ViewDefinition resource.
- `resource`: FHIR resources to transform.
- `patient`: filter by patient reference.
- `_limit`: limit results, 1 to 10000.
- `_since`: filter by modification time.

Parameter precedence is request body, then query params, then Accept header.

## Structural Validation (Lint) — the Single Source of Truth (#821)

`helios_sof::lint::lint_view_definition` is the one engine that answers "is
this JSON document a well-formed ViewDefinition": it walks a raw
`serde_json::Value` (never a typed `helios_fhir` resource — a document being
edited is often not valid enough to deserialize) and returns every problem it
finds, located by RFC 6901 JSON pointer — unknown/missing/wrong-typed/empty
keys, a `select` with no output or more than one iteration directive,
duplicate column names, FHIRPath syntax errors, and undeclared `%constant`
references. It is deliberately structural and syntactic only; it never
evaluates FHIRPath, resolves terminology, or touches storage.

Every `Diagnostic` carries `args` (the values its English `message`
interpolates — `message` itself is always English, never localized here) and
`fixes` (structural, pointer-addressed edits a client believes resolve it —
`rename-key`/`remove-key`/`set-string`; never a text position, since this
module never sees source text). `node_keys` exposes the same key model these
checks are built on, for anything that wants "what keys are valid here" (the
`helios-ui` completion endpoint, below).

`$sql-run` lints the inline ViewDefinition JSON as received, before any typed
parse — a round-trip through the typed `Parameters`/`ViewDefinition` structs
would silently drop unknown keys, since no generated struct denies them, so
linting *after* that parse would never catch `unknown-key` at all. A lint
failure responds `422` with one `OperationOutcome.issue` per diagnostic,
`issue.expression` carrying the pointer and `issue.details.coding` a code
under `http://heliossoftware.com/fhir/CodeSystem/view-definition-lint`
matching `Diagnostic::code`. The crate's own `validate_view_definition`
delegates to the same lint (serializing the typed struct back to JSON first),
so `sof-cli` and `pysof` see identical rules and messages — one engine, not
three. A few checks that depend on evaluation context rather than structure
(`collection: false` outside `forEach`, cross-branch column consistency in
`unionAll`) stay as `validate_view_definition`'s own residual checks; the
lint does not model them.

The ViewDefinition editor's own `POST /ui/sql/view-definitions/lint` and
`POST /ui/sql/view-definitions/complete` endpoints (`helios-ui`, see
`/work-with-ui`) build directly on this module — `/lint` translates `code` +
`args` into the negotiated locale and adds a translated `label` to each fix;
`/complete` answers "what fits here" from the same key model plus
`helios_fhirpath`'s function/constant/variable catalogs, re-exported through
`helios_sof::lint` so `helios-ui` never takes a direct `helios-fhirpath`
dependency of its own.

## Parquet Export

```bash
# CLI
cargo run --bin sof-cli -- --view view.json --bundle data.json --format parquet

# Server
curl -X POST http://localhost:8080/\$sql-run \
  -H "Content-Type: application/json" \
  -d '{"_format": "parquet", "subjectResource": {...}, "resource": [...]}'
```

Parquet type mapping follows Pathling conventions:

| SOF type | Parquet type |
|---|---|
| boolean | BOOLEAN |
| string, code, uri | UTF8 |
| integer | INT32 |
| decimal | FLOAT64 |
| dateTime, date | UTF8 |

Arrays map to Arrow List types. All fields are OPTIONAL. Snappy compression is the default.

## Async Export in the HFS Server

The `hfs` binary embeds SQL-on-FHIR via `helios-rest` and exposes the async
`$sql-export` operation. These run in the
background and write tabular shards to an *export sink*, configured with the
`HFS_EXPORT_*` variables below (distinct from the standalone `sof-server`
`SOF_*` vars above, and from Bulk Data Export's `HFS_BULK_EXPORT_*`).

The subsystem is gated by `HFS_SOF_ENABLED`, and the configured storage backend
must provide an in-DB SOF runner (`sqlite` or `postgres`) — there is no
in-process fallback.

| Variable | Default | Description |
|---|---|---|
| `HFS_SOF_ENABLED` | `true` | Master switch for SQL-on-FHIR ops (`$sql-run`, `$sql-export`) |
| `HFS_EXPORT_SINK` | `fs` | Output sink: `fs` (local filesystem) or `s3` |
| `HFS_EXPORT_DIR` | `./exports` | Root directory for the `fs` sink |
| `HFS_EXPORT_S3_BUCKET` | *(none)* | S3 bucket — required when `HFS_EXPORT_SINK=s3` |
| `HFS_EXPORT_S3_REGION` | *(AWS chain)* | AWS region override for the `s3` sink |
| `HFS_EXPORT_PRESIGN_TTL_SECS` | `86400` | Pre-signed download-URL lifetime for `s3`, seconds (spec requires ≥ 24h) |
| `HFS_EXPORT_MAX_CONCURRENCY` | `4` | Maximum concurrent export jobs |
| `HFS_EXPORT_SHARD_ROWS` | `500000` | Target rows per output shard; larger results split across files |
| `HFS_EXPORT_CONTROLLER` | `memory` | Job-controller backend (`memory` in-process; `kafka`/`sqs` reserved) |
| `HFS_EXPORT_OUTPUT_TTL` | `86400` | Retention (seconds) for a finished job's output + bookkeeping; the cleanup reaper then deletes shards and drops the job (later polls/downloads → `404`) |
| `HFS_EXPORT_CLEANUP_INTERVAL` | `300` | Cleanup-reaper scan interval, seconds (clamped to ≥ 1) |

Cancelling a job (`DELETE` on the status URL) or a mid-run failure deletes that
job's partial shards immediately; the reaper reclaims *completed* jobs once they
age past `HFS_EXPORT_OUTPUT_TTL`. Full `HFS_EXPORT_*` reference lives in the
[helios-rest README](../../../crates/rest/README.md#sql-on-fhir-async-export).

## Testing

- Unit tests live in `src/` files.
- Integration tests live in `tests/`.
- ViewDefinition examples are embedded in test files.
- For version-independent logic, use enum wrappers such as `SofViewDefinition` and traits such as `ViewDefinitionTrait`, `BundleTrait`, and `ResourceTrait`.
