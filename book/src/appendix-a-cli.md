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
| `HFS_DEFAULT_FHIR_VERSION` | `R4` (a build without `R4` uses its only enabled version, and refuses to start without this setting if it enables more than one) | `R4`, `R4B`, `R5`, `R6` |
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

### Polling history with `_since`

A client that polls `GET /_history?_since={instant}` (or the type or instance
history) can **miss entries for good** in two ways: an entry can become visible
after a poll that already covered its `lastUpdated`, and a history response
holds only one page.

**The commit-time gap.** On every backend, HFS gives a write its `lastUpdated`
before other readers can see the write, and concurrent writes can land out of
order, so a later `lastUpdated` can become visible while an earlier one is
still being written. For one ordinary write the gap is short. On SQLite,
PostgreSQL and MongoDB it can be much longer for a transaction Bundle: each
entry gets its `lastUpdated` when HFS processes that entry, but no entry is
visible until the whole Bundle commits. A large Bundle can run for a long time
(minutes on SQLite), so its entries can appear after a poll that already
covered their `lastUpdated`. S3 does not accept transaction Bundles, but it
writes the entries of a batch Bundle several at a time.

To avoid the gap, start each window earlier than the newest
`response.lastModified` you have seen; until you have seen one, keep your
starting `_since`. That value comes from the HFS server's clock, so your own
clock does not matter. How much earlier:

- On every backend, by at least `HFS_REQUEST_TIMEOUT` (default `30` seconds).
  On PostgreSQL and MongoDB that is enough: a transaction Bundle still running
  at that point is rolled back, so none of its entries appears later.
- On SQLite, by the largest of three:
  - `HFS_REQUEST_TIMEOUT`;
  - 30 seconds, since an update is stamped before it waits for the write lock,
    that wait can last 30 seconds, and the request timeout cannot cut it
    short; and
  - the longest transaction Bundle any client sends, since once a Bundle is
    running the timeout does not reliably cut it off, so it can commit well
    after `HFS_REQUEST_TIMEOUT`.
- While a `$bulk-submit` runs, by more than its longest batch takes. The
  worker is not a request, so no request timeout bounds it. On SQLite and
  PostgreSQL it commits each batch as one transaction. On MongoDB it stamps a
  whole batch with one time, then writes the batch in several commands. S3
  writes each entry on its own, so there is no batch-wide gap beyond the
  request-timeout overlap.
- When several HFS instances share one database, add the largest clock
  difference between them: each instance stamps `lastUpdated` from its own
  clock.

**The page limit.** HFS answers a history request with one page, newest first,
and puts no `next` link in the Bundle. On SQLite, PostgreSQL and MongoDB the
Bundle's `total` is the number of entries on that page, so it cannot tell you a
page was cut off; on S3 it counts every match, so a `total` larger than the
number of entries means the page was cut off. A page holds `_count` entries:
`20` by default (`HFS_DEFAULT_PAGE_SIZE`) and at most `HFS_MAX_PAGE_SIZE`
(default `1000`). When a window holds more versions than that, the page keeps
the newest and drops the oldest, and a later window that starts at the newest
`lastModified` may never return them. A single large transaction Bundle can
exceed that cap by itself, and the overlap above widens every window, so a busy
server can fill a page even when you poll often.

- Pass `_count` up to `HFS_MAX_PAGE_SIZE`. HFS lowers a larger `_count` to
  `HFS_MAX_PAGE_SIZE` without telling you.
- Treat a response with as many entries as the effective page size as cut off.
  That is the `_count` you sent, or `HFS_MAX_PAGE_SIZE` if that is smaller: a
  response to a larger `_count` stops at the cap, so it never reaches `_count`
  entries. Poll often enough that a window stays under the effective page
  size, or accept this as a known limit: a history request cannot yet be
  paged.

**Reading the entries.**

- Expect entries you have already seen: `_since` includes the instant you
  pass, and the windows overlap. Dedupe on resource type, id and version. Take
  the type and id from `fullUrl` and the version from `response.etag`
  (`W/"{version}"`): a `DELETE` entry has no `resource`, so
  `resource.meta.versionId` is not always there.
- Percent-encode the instant you send in `_since` (write `+` as `%2B`), or
  send it in UTC with a `Z` suffix. HFS writes `lastModified` with a `+00:00`
  offset, and an unencoded `+` in a query string becomes a space, so HFS
  answers `400`.

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
| `--since <RFC3339>` | | — | Filter resources modified at or after this time |
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
| `SOF_MAX_BODY_SIZE` | `10485760` | Max request body (bytes; applies to the decompressed body for compressed requests) |
| `SOF_REQUEST_TIMEOUT` | `30` | Request timeout (seconds) |
| `SOF_ENABLE_CORS` | `true` | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed CORS origins |

### Endpoint

`POST /$sql-run`

| Parameter | Type | Description |
|-----------|------|-------------|
| `_format` | string | `csv`, `json`, `ndjson`, `parquet` |
| `header` | boolean | Include CSV header (`true` / `false`) |
| `subjectResource` | object | ViewDefinition resource |
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
