# Helios FHIR Server (HFS)

A high-performance FHIR server built in Rust.

An open test server is available at https://hfs.heliossoftware.com/ for experimentation and evaluation.

## Features

- Full FHIR RESTful API support 
- Multiple FHIR version support
- Pluggable storage backends (SQLite, PostgreSQL, MongoDB)
- Content negotiation (JSON)
- Conditional operations with ETag support
- Multi-tenant support via X-Tenant-ID header
- CORS support

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build with default features (R4 + SQLite)
cargo build --release -p helios-hfs

# Build with all FHIR versions
cargo build --release -p helios-hfs --features R4,R4B,R5,R6,sqlite
```

## Usage

### Running the Server

```bash
# Run with default settings (R4, SQLite, port 8080)
./target/release/hfs

# Specify a different port
./target/release/hfs --port 3000

# Use an in-memory database
./target/release/hfs --database-url :memory:

# Enable debug logging
./target/release/hfs --log-level debug
```

### Command Line Options

```
Usage: hfs [OPTIONS]

Options:
      --port <PORT>              Server port [env: HFS_SERVER_PORT=] [default: 8080]
      --host <HOST>              Host to bind [env: HFS_SERVER_HOST=] [default: 127.0.0.1]
      --log-level <LOG_LEVEL>    Log level (error, warn, info, debug, trace)
                                 [env: HFS_LOG_LEVEL=] [default: info]
      --database-url <URL>       Database connection URL [env: DATABASE_URL=]
      --data-dir <PATH>          Path to FHIR data directory containing search parameter
                                 definitions [env: HFS_DATA_DIR=] [default: ./data]
      --max-body-size <BYTES>    Maximum request body size [env: HFS_MAX_BODY_SIZE=]
                                 [default: 134217728]
      --request-timeout <SECS>   Request timeout in seconds [env: HFS_REQUEST_TIMEOUT=]
                                 [default: 30]
      --enable-cors              Enable CORS [env: HFS_ENABLE_CORS=] [default: true]
      --cors-origins <ORIGINS>   Allowed CORS origins [env: HFS_CORS_ORIGINS=] [default: *]
  -h, --help                     Print help
  -V, --version                  Print version
```

`hfs --version` prints the crate version and, when the binary was built from a
git checkout, the commit it was cut from — `hfs 0.2.1 (git 1a2b3c4d5)`. The
same identity is advertised to FHIR clients as `CapabilityStatement.software`
on `GET /metadata` (the commit as an extension on `software`) and reported by
`GET /health`. When building from a source archive with no `.git`, set
`HFS_GIT_SHA=<sha>` in the build environment to record the commit.

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | 8080 | Server port |
| `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HFS_BASE_URL` | http://localhost:8080 | Public HTTP(S) base for response links and `Location` headers |
| `HFS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `HFS_UI_ENABLED` | true | Serve the web UI at `/ui`; `false` for headless deployments (`/ui` then returns 404 + OperationOutcome) |
| `DATABASE_URL` | fhir.db | Database connection string |
| `HFS_DATA_DIR` | ./data | Path to FHIR data directory (search parameters) |
| `HFS_SEARCH_PARAM_CACHE_TTL` | 3600 | Seconds between refreshes of the in-memory SearchParameter registry from storage; a param POSTed to one cluster node becomes visible to others within this interval. `0` disables the refresh. |
| `HFS_MAX_BODY_SIZE` | 134217728 | Max request body size (bytes; applies to the decompressed body for compressed requests). The 128 MiB default covers the largest per-patient Synthea Bundle (#1662); a batch or transaction body is parsed whole in memory, so lower it on small hosts. A bundle that size takes minutes to process (the largest, 27,056 entries, took about 2 minutes on a release build with SQLite), and `HFS_REQUEST_TIMEOUT` can answer 408 before it finishes: raise that too when posting large bundles. |
| `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `HFS_DASHBOARD_RECONCILE_SECS` | 30 | Seconds between Home dashboard reconcile passes against storage (CLI `--dashboard-reconcile-interval-secs`). Whole seconds, must be `> 0`; `0` or a non-numeric value is a startup error. A tenant's full recount is additionally spaced to at most every max(interval, 10 × its last recount duration); also how soon a failed background seed is retried. |
| `HFS_DASHBOARD_REFRESH_SECS` | 5 | Seconds between refreshes of a Home dashboard whose figures are moving (approximate, or an import running) (CLI `--dashboard-refresh-secs`). Re-reads in-memory counters only, never storage. Whole seconds, must be `> 0` and `<=` `HFS_DASHBOARD_IDLE_REFRESH_SECS`; otherwise a startup error. |
| `HFS_DASHBOARD_IDLE_REFRESH_SECS` | 10 | Seconds between watch ticks of a Home dashboard whose figures are settled (exact, no import running), so an open tab notices an import starting (CLI `--dashboard-idle-refresh-secs`). Whole seconds, must be `> 0` and `>=` `HFS_DASHBOARD_REFRESH_SECS`; otherwise a startup error. |
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_CORS_ORIGINS` | * | Allowed CORS origins |
| `HFS_CORS_METHODS` | GET,POST,PUT,DELETE,OPTIONS | Allowed HTTP methods |
| `HFS_CORS_HEADERS` | Content-Type,Authorization,X-Requested-With | Allowed headers |
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_TERMINOLOGY_SERVER` | (none) | Terminology server URL for `:in`/`:not-in` modifiers and FHIRPath `memberOf()`/`subsumes()` |
| `HFS_COMPOSITE_SYNC_MODE` | `asynchronous` | Composite-store write sync mode for ES-backed backends (`sqlite-elasticsearch`, `postgres-elasticsearch`, `mongodb-elasticsearch`, `s3-elasticsearch`). One of `asynchronous`, `synchronous`, `hybrid`. With `asynchronous` (default) the write returns as soon as the primary commits and the search backend is updated on a background worker — lowest latency, but a follow-up search can race the indexing. Use `synchronous` when callers need read-your-write semantics (e.g. integration tests, bulk-load flows that immediately search). Ignored when the storage backend has no search secondary. |
| `HFS_ELASTICSEARCH_REFRESH_INTERVAL` | `1s` | Elasticsearch index `refresh_interval` for ES-backed backends. Controls how quickly indexed documents become searchable when no per-write refresh is requested. `-1` disables periodic refresh entirely. Set when an index is created and reconciled at startup onto every existing `{HFS_ELASTICSEARCH_INDEX_PREFIX}_*` index, overwriting a value set by hand with `PUT _settings`. With `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` the interval is the price of every interactive write, so keep it short (`1s` or less); a long value makes each write wait for the next refresh (#1749). |
| `HFS_ELASTICSEARCH_WRITE_REFRESH` | `false` | The `refresh` parameter applied to Elasticsearch index/delete operations. One of `false` (no per-write refresh), `wait_for` (block each write until the affected shards refresh), or `true` (force a refresh per write; expensive, low-volume deployments only). |
| `HFS_ELASTICSEARCH_MAX_TERMS_COUNT` | `65536` | Elasticsearch `index.max_terms_count`: the most values a single `terms` query may carry. A chained or `_has` search whose terminal hop resolved more ids than this is sent as several `terms` clauses of at most this many values each, instead of being refused (#1548). Applied to new indices through the index template. |
| `HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT` | `50000` | Elasticsearch `index.mapping.nested_objects.limit`: the maximum nested objects a single document may contain, summed across every nested search-parameter field. Elasticsearch's own default of 10000 rejects larger documents, so a resource with very many indexed values (for example a Synthea `Provenance` with 13,554 `target` references) is stored but never searchable. Applied to new indices through the index template, and raised at startup on existing indices that are below it; the setting is dynamic, so no reindex is needed for resources that already indexed. |
| `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS` | `30000` | Timeout of every Elasticsearch HTTP request, in milliseconds, including `_bulk`. A `_bulk` request that times out is split in half and resent, down to a single document, so raise this only when one large document still times out on its own. |
| `HFS_ELASTICSEARCH_BULK_MAX_BYTES` | `10485760` (10 MiB) | Byte cap on one `_bulk` request body, applied together with the 500-operation cap, for bulk creates and every rebuild. Without it, 500 Synthea `Provenance` resources (~108 KB each) made a ~54 MB request that exceeded the request timeout. A single document larger than the cap is sent alone. |
| `HFS_ELASTICSEARCH_BULK_CONCURRENCY` | `1` | How many `_bulk` requests of one page are in flight at once. `1` sends them one at a time, as before #1125. Raising it shortens a rebuild when the cluster is not the bottleneck; splitting after a `413` or a timeout, and `429` back-off, stay sequential within the request that caused them. |
| `HFS_ELASTICSEARCH_CONTAINED_SWEEP_CONCURRENCY` | `0` (built-in 16) | Most contained-document sweeps in flight at once. On every update and delete the Elasticsearch writer runs a `_delete_by_query` over the tenant's indices to remove the container's stale `contained[]` documents; each sweep opens a scroll context on every shard it visits and the cluster allows `search.max_open_scroll_context` (500) in all, so an unbounded burst is answered `429 too_many_scroll_contexts`. A sweep the cluster still refuses is repeated on its own (4 rounds, jittered back-off) instead of redoing the whole sync event. The sweep always runs on update and delete. |
| `HFS_ELASTICSEARCH_REINDEX_PREPARE_THREADS` | `0` | Threads that extract search values for one `$reindex` page on the Elasticsearch writer, which is what the composite deployments' post-import rebuild runs through. `0` uses cores − 1, clamped to 1–4; `1` extracts on the page's own thread, as before #1250; an explicit value is capped at 64. Pages of fewer than 16 resources, and any page on a current-thread runtime, extract inline. |
| `HFS_REINDEX_BATCH_BYTES` | `33554432` (32 MiB) | Byte cap on one page of the automatic rebuild, on top of `HFS_REINDEX_BATCH_SIZE`. `0` means count only; with a cap set, a page of ~108 KB `Provenance` resources stays near the cap instead of holding ~108 MB in memory. Honoured by the SQLite source (a page may exceed the cap by one resource) and by the PostgreSQL and MongoDB sources (a page never exceeds the cap unless it holds a single resource); the Elasticsearch and S3 sources page by count only. |
| `HFS_REINDEX_PREFETCH` | `true` | Whether the search-index rebuild fetches the next page from the primary while the current one is written, for `POST $reindex` and the automatic post-import rebuild alike. Honoured wherever the source allows the overlap: standalone MongoDB and `mongo-es` (id phase), and `sqlite-es`, `postgres-es` and `s3-es`, where search is offloaded so the page read and the Elasticsearch write touch disjoint stores; standalone SQLite and PostgreSQL keep the serial loop. `false` restores the strictly serial fetch → write loop on every backend (#1250). `HFS_MONGODB_REINDEX_PREFETCH` remains the MongoDB-specific switch underneath. |
| `HFS_ELASTICSEARCH_REINDEX_REFRESH` | *(unset)* | The `refresh` parameter for the `_bulk` writes of `$reindex` and of the deferred post-import rebuild: `false`, `wait_for`, or `true`. Unset follows `HFS_ELASTICSEARCH_WRITE_REFRESH`. Set `false` to let a rebuild skip the per-request refresh wait while ordinary writes keep `wait_for`. |
| `HFS_REINDEX_BATCH_SIZE` | `1000` | Page size of the automatic deferred rebuild that follows a bulk import, on every backend. `POST $reindex` keeps its own `batchSize` parameter (default 100), capped at 10,000. |

Read-after-write search on an ES-backed composite needs **both**
`HFS_COMPOSITE_SYNC_MODE=synchronous` and
`HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`. See
[Search visibility on Elasticsearch-backed composites](../persistence/README.md#search-visibility-on-elasticsearch-backed-composites)
in the persistence crate for why either setting alone still leaves a window.

Elasticsearch `_bulk` failures are handled per shape. A request that times out,
or is answered `413`, `408` or `504`, is split in half and each half resent,
down to a single document; once a single document times out, the rest of the
page fails as transient instead of being split further. A `429`, for the whole
request or for single items, is retried with bounded exponential back-off for
the rejected items only. A per-document `4xx` is permanent and not retried. A
connection error, a whole-request `5xx`, or a `429` that outlasts its retries
fails every document of that request as transient. Every failed document is
recorded by `Type/id`, with Elasticsearch's `error.type` and `reason` when it
gave one, in `GET /$reindex-status/{job_id}`. See
[Bulk writes and rebuilds on Elasticsearch-backed composites](../persistence/README.md#bulk-writes-and-rebuilds-on-elasticsearch-backed-composites).

For `$bulk-submit`, which indexing mode and settings to run per backend are in the [indexing-mode table](../rest/README.md#bulk-submit-indexing-mode-per-storage-backend); the paragraph below is only about the `$reindex` rebuild.

On `sqlite-elasticsearch`, the recommended pair for a bulk import is
`HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` with
`HFS_ELASTICSEARCH_REINDEX_REFRESH=false`: ordinary writes stay
read-your-write, while the rebuild skips the per-request refresh wait.
Measured on a 228,580-resource cut, that took the deferred rebuild from 806 s
to 145 s (1,576 resources/s) with the same result.

**Recommended `HFS_ELASTICSEARCH_REFRESH_INTERVAL`: keep the `1s` default (#1749).**
A long interval does not shorten a bulk import and it makes every interactive
write slower. Measured on `postgres-elasticsearch` (PostgreSQL 16, Elasticsearch
8.15.0 with an 8 GB heap, 8 CPU / 23 GB, `HFS_BULK_SUBMIT_DEFER_INDEXING=false`,
`HFS_COMPOSITE_SYNC_MODE=synchronous`, `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`,
919,752 resources, three runs per value in rotated order):

| `REFRESH_INTERVAL` | Import (mean of 3) | Elasticsearch CPU (mean) | Refreshes | Single `POST /Basic`, p50 |
|---|---|---|---|---|
| `1s` | 223 s | ~95 % | ~365 | 1.0 s |
| `5s` | 220 s | ~75 % | ~144 | 5.0 s |
| `30s` | 220 s | ~40 % | ~95 | not measurable: HFS did not become ready |
| `-1` | not measurable with `wait_for` | | | |

- **Import:** the three intervals are within 2 % of each other, far under the 10 %
  that would justify a dynamic interval, so none is built. The longer interval
  only saves Elasticsearch CPU; the import is limited by HFS and PostgreSQL.
- **Interactive writes:** with `wait_for` a write, a batch of 8 and a transaction
  of 662 entries each take about one interval (1.0 s at `1s`, 5.0 s at `5s`), the
  same with or without an import running. `HFS_ELASTICSEARCH_WRITE_REFRESH=true`
  or `false` returns in under 0.1 s (p95) at any interval, but then a write is not
  searchable on return (`false`) or each write forces a refresh (`true`).
- **A new index takes the configured interval:** the first write of a type never
  written before created `hfs_default_riskassessment` with the configured value,
  so a long interval also slows the first save of each type.
- **`30s` and `-1` with `wait_for` are not usable:** HFS seeds the search
  parameters with `wait_for` writes at startup. On an empty cluster at `30s` that
  took 212 s in two runs, and in two other attempts it was still not ready after
  2 and after 10 minutes (each seed write timing out at the 30 s request timeout
  and being retried); with `-1` the server was not ready after 10 minutes in
  three runs. HFS now logs a warning at startup
  for these combinations.
- **Larger corpus (25 % cut of the full Synthea corpus, 4,738,958 resources in all
  24 types, same machine and settings):** the long interval still does not pay for
  itself. Imported with `1s`: 1,363 s (3,477 resources/s), 2,828 refreshes,
  Elasticsearch CPU ~97 %. With `30s`: 1,328 s (3,568 resources/s, 2.6 % faster),
  391 refreshes, Elasticsearch CPU ~54 %. On the same loaded server, with no
  configuration change, a `POST /Basic`, a batch of 8, a 662-entry transaction and
  the first write of a new type (`RiskAssessment`) took about 1.0 s at `1s` and
  about 30 s (p95 and maximum 60 s) at `30s`.
- **Not measured:** the full 18.96 M-resource corpus (the import rate falls as the
  index grows: an interrupted full run averaged ~2,460 resources/s at 64 %, so the
  25 % figures are optimistic) and the real 8-entry batch and 662-entry transaction
  Bundles (the numbers above use synthetic `Basic` Bundles of the same size). The
  during-import pass overlapped only the end of a 100-bundle import.

A manifest ingested with indexing deferred records that it still owes a
rebuild, on its own row. If the server stops mid-rebuild, the next start
re-fires it and logs `resuming search-index rebuilds left outstanding by an
earlier run`; the marker is cleared once the rebuild has run.

Set `HFS_BASE_URL` to the public address clients can reach. The value may
contain a path prefix. It must be an absolute `http` or `https` URL without
credentials, a query string, or a fragment. HFS rejects invalid values and
does not infer the public address from request forwarding headers.

## FHIR Version Support

Build with specific FHIR versions using feature flags:

```bash
# R4 only (default)
cargo build -p helios-hfs --features R4,sqlite

# R5 only
cargo build -p helios-hfs --no-default-features --features R5,sqlite

# Multiple versions
cargo build -p helios-hfs --features R4,R4B,R5,R6,sqlite
```

## Database Backends

### SQLite (Default)

```bash
cargo build -p helios-hfs --features sqlite

# Run with file-based database
./target/release/hfs --database-url ./data/fhir.db

# Run with in-memory database
./target/release/hfs --database-url :memory:
```

### PostgreSQL

```bash
cargo build -p helios-hfs --no-default-features --features R4,postgres

./target/release/hfs --database-url "postgresql://user:pass@localhost/fhir"
```

### MongoDB

```bash
cargo build -p helios-hfs --no-default-features --features R4,mongodb

./target/release/hfs --database-url "mongodb://localhost:27017/fhir"
```

## API Endpoints

| Interaction | Method | URL |
|------------|--------|-----|
| capabilities | GET | `/metadata` |
| read | GET | `/[type]/[id]` |
| vread | GET | `/[type]/[id]/_history/[vid]` |
| update | PUT | `/[type]/[id]` |
| patch | PATCH | `/[type]/[id]` |
| delete | DELETE | `/[type]/[id]` |
| create | POST | `/[type]` |
| search | GET/POST | `/[type]?params` or `/[type]/_search` |
| history (instance) | GET | `/[type]/[id]/_history` |
| history (type) | GET | `/[type]/_history` |
| history (system) | GET | `/_history` |
| batch/transaction | POST | `/` |
| health | GET | `/health` |

History reads (instance, type and system `_history`) accept `_count` (capped at `HFS_MAX_PAGE_SIZE`), `_since` (an RFC 3339 instant with a time zone, such as `2024-01-01T00:00:00Z`; only versions created at or after it are returned) and `_format`, each at most once. FHIR's general parameters `_pretty`, `_summary` and `_elements` are accepted but not applied to history. The other history parameters FHIR defines, `_at`, `_list` and `_sort`, are not implemented. They are refused with `400 Bad Request` and an OperationOutcome with issue code `not-supported`, whatever their value and whatever `Prefer: handling` the request sends, because answering with the unfiltered, default-ordered history would look like a filtered or sorted answer. Use `_since` to narrow a history read by time. Any other parameter is refused with `400` (issue code `invalid`) under `Prefer: handling=strict` and ignored otherwise: unlike a searchset, a history Bundle has nowhere to report an ignored parameter. A `_count` that is not a non-negative integer, a `_since` that is not an instant, or a repeated `_count`, `_since` or `_format` is a `400` with issue code `invalid`. History Bundles carry a `self` link and, when there is another page, a `next` link that keeps the request's `_count`, `_since` and `_format`: follow `next` to read the following page. Its `_cursor` is opaque, so use the link as given; a `_cursor` that does not decode, or a repeated one, is a `400` with issue code `invalid`.

## Examples

### Create a Patient

```bash
curl -X POST http://localhost:8080/Patient \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Patient",
    "name": [{"family": "Smith", "given": ["John"]}],
    "birthDate": "1970-01-01"
  }'
```

### Read a Patient

```bash
curl http://localhost:8080/Patient/123
```

### Search for Patients

```bash
curl "http://localhost:8080/Patient?family=Smith"
```

### Get CapabilityStatement

```bash
curl http://localhost:8080/metadata
```

## Batch and Transaction Support

HFS supports both batch and transaction bundles for processing multiple operations in a single request.

> **Note:** Transaction bundles require ACID transaction support. The default SQLite backend fully supports transactions. If using other backends, check the capability matrix in the persistence crate documentation.

### Transaction Bundle (Atomic)

All operations succeed or all fail. Entries are processed in FHIR-specified order: DELETE → POST → PUT → GET.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "transaction",
    "entry": [
      {
        "fullUrl": "urn:uuid:patient-1",
        "resource": {
          "resourceType": "Patient",
          "name": [{"family": "Smith"}]
        },
        "request": {
          "method": "POST",
          "url": "Patient"
        }
      },
      {
        "resource": {
          "resourceType": "Observation",
          "subject": {"reference": "urn:uuid:patient-1"},
          "code": {"text": "Blood Pressure"}
        },
        "request": {
          "method": "POST",
          "url": "Observation"
        }
      }
    ]
  }'
```

The `urn:uuid:patient-1` reference is automatically resolved to the actual Patient ID after creation.

### Batch Bundle (Independent)

Each operation is processed independently; failures don't affect other entries.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "batch",
    "entry": [
      {
        "request": {
          "method": "GET",
          "url": "Patient/123"
        }
      },
      {
        "request": {
          "method": "DELETE",
          "url": "Patient/456"
        }
      }
    ]
  }'
```

### Current Limitations

The following FHIR bundle features are not yet implemented:
- Conditional reference resolution (`Patient?identifier=12345`)
- PATCH method in bundles
- Prefer header handling (`return=minimal`, etc.)

## Search Parameter Configuration

HFS loads FHIR SearchParameter definitions from JSON bundle files to enable comprehensive search functionality. By default, these files are expected in a `data/` directory relative to the working directory or executable.

### Data Directory Structure

```
data/
├── search-parameters-r4.json   # FHIR R4 SearchParameters (HL7 spec)
├── search-parameters-r4b.json  # FHIR R4B SearchParameters (HL7 spec)
├── search-parameters-r5.json   # FHIR R5 SearchParameters (HL7 spec)
├── search-parameters-r6.json   # FHIR R6 SearchParameters (auto-downloaded at build time)
├── sql-on-fhir-search-parameters.json  # SQL-on-FHIR IG ViewDefinition search params (custom file)
└── *.json                      # Custom SearchParameter files (see below)
```

### Search Parameter Loading

On startup, HFS loads SearchParameters in this order:
1. **Minimal fallback** - Built-in `_id`, `_lastUpdated`, `_tag`, `_profile`, `_security` (always available)
2. **Spec file** - Loads from the appropriate `search-parameters-*.json` based on configured FHIR version
3. **Custom files** - Loads any additional `.json` files in the data directory (not matching `search-parameters-*.json`)
4. **Stored parameters** - Loads any custom SearchParameters POSTed to the server

### Custom SearchParameter Files

You can add custom SearchParameters by placing JSON files in the data directory. Each file can contain:
- A single SearchParameter resource
- An array of SearchParameter resources
- A FHIR Bundle containing SearchParameter resources

Example custom SearchParameter file (`data/custom-search-params.json`):
```json
{
  "resourceType": "SearchParameter",
  "id": "patient-mrn",
  "url": "http://example.org/fhir/SearchParameter/patient-mrn",
  "name": "mrn",
  "status": "active",
  "code": "mrn",
  "base": ["Patient"],
  "type": "token",
  "expression": "Patient.identifier.where(type.coding.code='MR')"
}
```

Or as a Bundle:
```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "SearchParameter",
        "url": "http://example.org/fhir/SearchParameter/patient-mrn",
        ...
      }
    }
  ]
}
```

### SQL-on-FHIR ViewDefinition Search Parameters

`data/sql-on-fhir-search-parameters.json` ships the 17 `ViewDefinition` SearchParameters from the [SQL-on-FHIR IG](http://hl7.org/fhir/uv/sql-on-fhir) as one of these custom Bundle files. Do not edit it by hand — regenerate it from the official IG package with:

```bash
scripts/sync-sof-search-params.py            # pinned IG release
scripts/sync-sof-search-params.py --ci-build # preview the IG CI build
```

The script strips generated narratives and rewrites the composite parameters' legacy `component.definition` canonicals to the core-namespace canonicals the parameters actually declare (an upstream IG inconsistency, reported as [HL7/sql-on-fhir#403](https://github.com/HL7/sql-on-fhir/issues/403); the rewrite becomes a no-op once fixed upstream). Bump `DEFAULT_PACKAGE_URL` in the script when a new IG ballot/release ships.

### Custom Data Directory

Specify a custom location for the data files:

```bash
# Via command line
./target/release/hfs --data-dir /opt/hfs/data

# Via environment variable
HFS_DATA_DIR=/opt/hfs/data ./target/release/hfs
```

If the spec file is missing, HFS logs a warning and continues with minimal fallback parameters. This ensures the server can start even without the full spec files, though search functionality will be limited.

### R6 Automatic Download

When building with the R6 feature enabled, the `search-parameters-r6.json` file is automatically downloaded from the HL7 build server during compilation. The download is skipped if:
- The file already exists and is less than 24 hours old
- The `DOCS_RS` environment variable is set (for docs.rs builds)

## Multi-Tenant Support

Use the `X-Tenant-ID` header to isolate data between tenants:

```bash
curl -H "X-Tenant-ID: clinic-a" http://localhost:8080/Patient
curl -H "X-Tenant-ID: clinic-b" http://localhost:8080/Patient
```
