# SQL-on-FHIR Integration Plan for HFS

## Overview

The standalone `sof-cli` and `sof-server` already exist in `crates/sof`. This plan describes integrating SQL-on-FHIR operations directly into the HFS server, bringing all operations up to date with the current SQL-on-FHIR v2 specification, and building the `$viewdefinition-export` async export system with an in-memory job controller designed to scale toward Kafka and SQS-based backends.

**Spec References:**
- [Operations & Capability](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/operations-capability.html)
- [$viewdefinition-export](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/OperationDefinition-ViewDefinitionExport.html)
- [$sqlquery-run](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/OperationDefinition-SQLQueryRun.html)

---

## Decisions Made

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Export job state | In-memory (`DashMap`) | First controller; acceptable for v1 |
| Export file storage | In-memory (served from RAM) | Consistent with in-memory controller |
| SQL engine | DataFusion *(pending confirmation)* | Arrow-native, pure Rust, no FFI; Arrow v54 already in `crates/sof` |
| $viewdefinition-run | Keep alongside export *(pending confirmation)* | Sync for interactive use; async export for bulk |
| Data access (TB-scale) | Stream from persistence + `source=` fallback *(pending confirmation)* | Add cursor trait to persistence; use `source=` for external data |
| Feature flag | `sof` feature in `helios-rest` and `helios-hfs` | Opt-in, composable with existing flags |

---

## Spec Summary

### $viewdefinition-run (Synchronous — existing)
- Already implemented in `sof-server`. Migrate into HFS.
- `POST /ViewDefinition/$viewdefinition-run`
- Synchronous request/response. Returns data immediately.
- Supports CSV, NDJSON, JSON, Parquet output formats.

### $viewdefinition-export (Asynchronous — new)
- `POST /$viewdefinition-export`
- `POST /ViewDefinition/$viewdefinition-export`
- `POST /ViewDefinition/{id}/$viewdefinition-export`
- **FHIR Async Pattern:**
  1. Client POSTs with `Prefer: respond-async` → server returns `202 Accepted` + `Content-Location` polling URL
  2. Client polls the status URL
  3. Server returns `202` + `Retry-After` while processing, then `303 See Other` on completion
  4. Client follows redirect to get a Parameters resource with output file download URLs
  5. Output files must remain available for ≥24 hours
- **Input:** ViewDefinition(s), `_format`, `patient`, `group`, `_since`, `source`, `clientTrackingId`
- **Output:** Parameters resource with `exportId`, `status`, `location`, and `output[].location` file URLs

### $sqlquery-run (Synchronous — new)
- `POST /$sqlquery-run`
- `POST /Library/$sqlquery-run`
- `POST /Library/{id}/$sqlquery-run`
- Materializes ViewDefinitions referenced in the Library resource as tables
- Executes SQL query against those tables
- **Input:** Library resource (inline or by reference) with SQL text, optional parameters
- **Output:** Results in `json`, `ndjson`, `csv`, `parquet`, or `fhir` format
- **Security:** Parameter binding is mandatory — no string interpolation allowed

---

## Architecture

### Crates Modified

| Crate | Change |
|-------|--------|
| `crates/rest` | Add `sof` feature; add SOF routes, handlers, job store, export controller trait |
| `crates/hfs` | Add `sof` feature passthrough; wire job store into AppState at startup |
| `crates/persistence` | Add `StreamingProvider` trait for cursor-based resource iteration |
| `crates/sof` | Add DataFusion integration for `$sqlquery-run`; no breaking changes to existing API |

### New Files

```
crates/rest/src/handlers/sof/
  mod.rs                        — module exports
  viewdefinition_run.rs         — sync $viewdefinition-run (migrated from sof-server)
  viewdefinition_export.rs      — async $viewdefinition-export handlers (kickoff, status, cancel, download)
  sqlquery_run.rs               — sync $sqlquery-run handler

crates/rest/src/sof/
  job_store.rs                  — in-memory job store (DashMap<String, ExportJob>)
  export_controller.rs          — ExportController trait + InMemoryExportController impl
  streaming_source.rs           — data source abstraction (persistence cursor vs external URL)

crates/sof/src/
  sqlquery.rs                   — DataFusion integration: ViewDef → RecordBatch → SQL execution

crates/persistence/src/core/
  streaming.rs                  — StreamingProvider trait (cursor-based resource streaming)
```

---

## Implementation Phases

### Phase 0 — Persistence: `StreamingProvider` Trait

Add cursor-based streaming to the persistence layer so export jobs can iterate over terabytes of FHIR data without loading it all into memory.

**New file:** `crates/persistence/src/core/streaming.rs`

```rust
#[async_trait]
pub trait StreamingProvider: ResourceStorage {
    async fn stream_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        chunk_size: usize,
    ) -> Result<impl Stream<Item = Result<Vec<serde_json::Value>, PersistenceError>>>;
}
```

Implementation strategy: **keyset pagination** (order by `id`, cursor on last seen id). This avoids `OFFSET` which degrades at scale.

Implement for:
- `SqliteBackend` (`crates/persistence/src/backends/sqlite/`)
- `PostgresBackend` (`crates/persistence/src/backends/postgres/`)

---

### Phase 1 — Feature Flag & Dependency Wiring

1. **`crates/rest/Cargo.toml`**: Add `sof` feature; add `helios-sof` and `datafusion` as optional dependencies under that feature.
2. **`crates/hfs/Cargo.toml`**: Add `sof = ["helios-rest/sof"]` feature passthrough.
3. **`crates/rest/src/state.rs`**: Add `export_controller: Option<Arc<dyn ExportController>>` to `AppState<S>` behind `#[cfg(feature = "sof")]`.
4. **`crates/rest/src/config.rs`**: Add `HFS_SOF_ENABLED` env var (auto-enabled when `sof` feature is compiled in).
5. **`crates/hfs/src/main.rs`**: Initialize `InMemoryExportController` and attach to `AppState` when `sof` feature is active.

---

### Phase 2 — Route Registration

**File:** `crates/rest/src/routing/fhir_routes.rs`

Add under `#[cfg(feature = "sof")]`:

```
POST /ViewDefinition/$viewdefinition-run              → viewdefinition_run::handler
POST /$viewdefinition-export                          → viewdefinition_export::kickoff
POST /ViewDefinition/$viewdefinition-export           → viewdefinition_export::kickoff
POST /ViewDefinition/:id/$viewdefinition-export       → viewdefinition_export::kickoff
GET  /$export-status/:export_id                       → viewdefinition_export::status
DELETE /$export-status/:export_id                     → viewdefinition_export::cancel
GET  /$export-result/:export_id/:file_name            → viewdefinition_export::download
POST /$sqlquery-run                                   → sqlquery_run::handler
POST /Library/$sqlquery-run                           → sqlquery_run::handler
POST /Library/:id/$sqlquery-run                       → sqlquery_run::handler
```

---

### Phase 3 — ExportController Trait (Pluggable for Kafka/SQS)

**File:** `crates/rest/src/sof/export_controller.rs`

The `ExportController` trait is the extension point for future async backends. The in-memory implementation is the first impl; Kafka and SQS controllers implement the same interface.

```rust
#[async_trait]
pub trait ExportController: Send + Sync + 'static {
    async fn submit(&self, job: ExportJob) -> Result<String, ExportError>; // returns export_id
    async fn status(&self, export_id: &str) -> Result<ExportJobStatus, ExportError>;
    async fn cancel(&self, export_id: &str) -> Result<(), ExportError>;
    async fn get_output_file(&self, export_id: &str, file_name: &str) -> Result<Bytes, ExportError>;
}
```

**`ExportJobStatus` variants:**
- `Accepted`
- `InProgress { progress: Option<String>, retry_after: u64 }`
- `Completed { output: Vec<ExportOutput>, end_time: DateTime<Utc> }`
- `Failed { outcome: OperationOutcome }`
- `Cancelled`

**`InMemoryExportController`:**
- `DashMap<String, Arc<Mutex<ExportJobState>>>` — job state
- `DashMap<String, HashMap<String, Bytes>>` — in-memory file storage
- Spawns a `tokio::task` per job that streams data and writes output into the in-memory map
- Background cleanup task (every 30 min) purges jobs and files older than 24 hours

**Future Kafka controller design note:**
- `submit()` publishes an export request message to a Kafka topic
- Workers consume the topic, process chunks, and write output to object storage (S3/GCS)
- `status()` / `get_output_file()` query a shared state store (Redis or Postgres) updated by workers
- This architecture prevents any single instance from being a memory bottleneck

---

### Phase 4 — $viewdefinition-export Handlers

**File:** `crates/rest/src/handlers/sof/viewdefinition_export.rs`

#### `kickoff` handler
1. Parse Parameters body: `view`, `_format`, `patient`, `group`, `_since`, `source`, `clientTrackingId`
2. Validate all ViewDefinitions **before** accepting (return `422` if invalid — spec requirement)
3. Construct `ExportJob`, call `controller.submit(job).await` → get `export_id`
4. Return `202 Accepted` with `Content-Location: /$export-status/{export_id}`

#### `status` handler
- `InProgress` → `202` + `Retry-After` + optional `X-Progress`
- `Completed` → `303 See Other` redirecting to result Parameters resource
- `Failed` → `500` + OperationOutcome body
- `Cancelled` or unknown → `404` + OperationOutcome

#### `download` handler
- Call `controller.get_output_file(export_id, file_name)`
- Stream bytes with correct `Content-Type` (csv / ndjson / parquet / json)

#### `cancel` handler
- Call `controller.cancel(export_id)`
- Return `202 Accepted` or `404`

#### Background job task (inside `InMemoryExportController::submit`)
For each view in the job:
1. Resolve data: stream from `StreamingProvider` (persistence) or load from `source=` URL via `UniversalDataSource`
2. Feed chunks into `PreparedViewDefinition` + `NdjsonChunkIterator` from `crates/sof`
3. Accumulate output bytes into the in-memory file map
4. Update job status atomically via the `Arc<Mutex<ExportJobState>>`

**Memory note (v1 limitation):** For the in-memory controller, the entire output of an export is held in RAM. For TB-scale data this is not viable — that is the explicit motivation for the Kafka/SQS phase. The streaming source ensures resources are never all in memory at once; only the *output* accumulates.

---

### Phase 5 — $sqlquery-run Handler

**File:** `crates/rest/src/handlers/sof/sqlquery_run.rs`
**File:** `crates/sof/src/sqlquery.rs`

#### DataFusion integration (`crates/sof/src/sqlquery.rs`)

```rust
pub async fn execute_sql_query(
    query_resource: &SqlQueryLibrary,
    parameters: Option<&Parameters>,
    source: Option<&str>,
    format: ContentType,
) -> Result<Vec<u8>, SofError>
```

Steps:
1. Parse `Library.content` for SQL text and declared parameter names/types
2. Load ViewDefinitions from `Library.relatedArtifact`
3. For each ViewDefinition: call existing `process_view_definition_generic()` → `ProcessedResult` → Arrow `RecordBatch`
4. Register each `RecordBatch` as a named `MemTable` in a DataFusion `SessionContext`
5. Safely bind parameters (DataFusion prepared statement — no string interpolation)
6. Execute SQL → collect result `RecordBatch`es
7. Format output using existing SOF formatters; add `fhir` format using the spec's SQL→FHIR type table

**FHIR type mapping (for `_format=fhir`):**

| SQL Type | FHIR value[x] |
|----------|---------------|
| BOOLEAN | valueBoolean |
| INT / INTEGER / SMALLINT | valueInteger |
| BIGINT | valueInteger64 |
| DECIMAL / NUMERIC / FLOAT | valueDecimal |
| CHARACTER variants | valueString |
| DATE | valueDate |
| TIME | valueTime |
| TIMESTAMP | valueDateTime |
| TIMESTAMP WITH TIME ZONE | valueInstant |
| NULL | omitted from row |

#### Handler
1. Extract `queryReference` (resolve from HFS storage) or `queryResource` (inline)
2. Extract `_format`, `parameters`, `source`
3. Call `execute_sql_query(...)`
4. Return raw bytes with correct `Content-Type`, or FHIR Parameters resource for `_format=fhir`

---

### Phase 6 — CapabilityStatement Updates

**File:** `crates/rest/src/handlers/capabilities.rs`

Under `#[cfg(feature = "sof")]`, add to the CapabilityStatement:
- `rest[0].operation[]` entries for `$viewdefinition-run`, `$viewdefinition-export`, `$sqlquery-run`
- `rest[0].resource[]` entries for `ViewDefinition` and `Library` with `read`, `search-type`, `create`, `update`, `delete`

---

## Key Files Reference

| File | Role |
|------|------|
| `crates/rest/src/routing/fhir_routes.rs` | Route registration |
| `crates/rest/src/state.rs` | AppState — add export controller |
| `crates/rest/src/config.rs` | ServerConfig — add SOF env var |
| `crates/hfs/src/main.rs` | Startup wiring |
| `crates/hfs/Cargo.toml` | Feature flag passthrough |
| `crates/rest/Cargo.toml` | `sof` feature + datafusion dep |
| `crates/sof/src/lib.rs` | Reuse `PreparedViewDefinition`, `NdjsonChunkIterator`, `process_view_definition_generic()` |
| `crates/sof/src/parquet_schema.rs` | Reuse for Parquet output |
| `crates/sof/src/data_source.rs` | Reuse `UniversalDataSource` for `source=` param |
| `crates/persistence/src/core/` | Add `StreamingProvider` trait |
| `crates/persistence/src/backends/sqlite/` | Implement `StreamingProvider` |
| `crates/persistence/src/backends/postgres/` | Implement `StreamingProvider` |

---

## Verification

```bash
# Build with sof feature
cargo build -p helios-hfs --features "R4,sqlite,sof"

# Run HFS with SOF enabled
cargo run --bin hfs --features "R4,sqlite,sof"

# Test $viewdefinition-run (sync)
curl -X POST http://localhost:8080/ViewDefinition/$viewdefinition-run \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters",...}'

# Kick off async export
curl -X POST http://localhost:8080/$viewdefinition-export \
  -H "Prefer: respond-async" \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters",...}'
# → 202 Accepted + Content-Location header

# Poll status
curl http://localhost:8080/$export-status/{export_id}
# → 202 while in progress, 303 when done

# Download result
curl -L http://localhost:8080/$export-status/{export_id}
# Follows 303 redirect to Parameters resource with output file URLs

# Test $sqlquery-run
curl -X POST http://localhost:8080/$sqlquery-run \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"_format","valueCode":"ndjson"},{"name":"queryResource",...}]}'

# Run tests
cargo test -p helios-rest --features "R4,sqlite,sof"
cargo test -p helios-sof --features "R4"
cargo test -p helios-persistence --features "sqlite"
```

---

## Open Clarifying Questions

The following decisions are still open and need your input before implementation begins:

1. **SQL engine for `$sqlquery-run`:** No analytics SQL engine currently exists in the project. Arrow v54 is already present in `crates/sof`.
   - **DataFusion** — pure Rust, Arrow-native, no FFI, good SQL support, natural fit with existing Arrow usage *(recommended)*
   - **DuckDB** — most powerful SQL dialect, C FFI dependency via `duckdb-rs`
   - **SQLite in-memory** — already in the project via `rusqlite`, but limited analytics SQL (no window functions, limited aggregates)

2. **Coexistence of `$viewdefinition-run` and `$viewdefinition-export` in HFS:**
   - **Keep both** — sync endpoint for interactive/small queries, async export for bulk *(recommended)*
   - **Export only** — only implement the new async endpoint in HFS; sync stays in standalone `sof-server`
   - **Unified with `Prefer` header** — one endpoint that behaves sync or async based on `Prefer: respond-async`

3. **TB-scale data access — how does the export engine read FHIR data from HFS?**
   - **Both: stream from persistence + `source=` param** — add `StreamingProvider` trait to persistence layer; use `source=` when an external path is provided *(recommended)*
   - **External `source=` only** — exports always require a `source=` URL pointing to an S3 bucket or NDJSON dump; no changes to persistence layer
   - **Persistence streaming only** — always stream from HFS storage; `source=` is ignored or unsupported

---

## Gap Analysis

### Implementation Status

All 6 phases are unimplemented. No SOF-related code exists in `crates/rest` or `crates/hfs` yet.

| Phase | Description | Status | Blockers |
|-------|-------------|--------|----------|
| 0 | `StreamingProvider` trait in persistence | ❌ Not started | None |
| 1 | Feature flags, dependency wiring, AppState | ❌ Not started | Phase 0 (for full wiring) |
| 2 | Route registration | ❌ Not started | Phase 1 |
| 3 | ExportController trait + InMemoryImpl | ❌ Not started | Phase 1; `dashmap` dep needed |
| 4 | `$viewdefinition-export` handlers | ❌ Not started | Phase 3 |
| 5 | `$sqlquery-run` + DataFusion | ❌ Not started | SQL engine decision pending |
| 6 | CapabilityStatement updates | ❌ Not started | Phase 2+ |

---

### Reusable Assets (already in codebase)

The following exist in `crates/sof` and are ready to use:

| Asset | Location | Used In |
|-------|----------|---------|
| `PreparedViewDefinition` | `crates/sof/src/lib.rs:1257` | Phase 4 |
| `NdjsonChunkIterator` | `crates/sof/src/lib.rs:1511` | Phase 4 |
| `process_view_definition` (public) | `crates/sof/src/lib.rs:1886` | Phase 4/5 |
| `process_view_definition_generic` (**private**) | `crates/sof/src/lib.rs:1952` | Phase 4/5 — see issue #1 below |
| `UniversalDataSource` / `DataSource` trait | `crates/sof/src/data_source.rs` | Phase 4 |
| `ContentType`, `ProcessedResult`, `RunOptions` | `crates/sof/src/lib.rs` | Phase 4/5 |
| `ParquetOptions`, `format_parquet_multi_file` | `crates/sof/src/lib.rs` | Phase 4/5 |
| Arrow 54.0 + Parquet 54.0 | `crates/sof/Cargo.toml` | Phase 5 |
| Working `$viewdefinition-run` handler | `crates/sof/src/handlers.rs:run_view_definition_handler` | Phase 2 migration source |
| Keyset cursor pagination (`last_updated + id`) | `crates/persistence/src/types/pagination.rs` | Phase 0 |
| `StreamingBulkSubmitProvider` (stream-in pattern) | `crates/persistence/src/core/bulk_submit.rs:1071` | Phase 0 reference |
| `AppState<S>` struct | `crates/rest/src/state.rs` | Phase 1 |
| `ServerConfig` struct | `crates/rest/src/config.rs` | Phase 1 |

---

### Phase-by-Phase Gaps

#### Phase 0 — StreamingProvider Trait

- `StreamingProvider` trait does not exist in `crates/persistence/src/core/`
- The streaming-out pattern (iterate resources for export) is **distinct** from the existing `StreamingBulkSubmitProvider` (stream resources *in* for bulk import)
- Keyset pagination infrastructure is ready in both backends and can be reused directly

#### Phase 1 — Feature Flag & Dependency Wiring

Gaps in `crates/rest/Cargo.toml`:
- No `sof` feature flag
- No `helios-sof` optional dependency
- No `datafusion` optional dependency — **DataFusion is absent from the entire workspace**

Gaps in `crates/hfs/Cargo.toml`:
- No `sof = ["helios-rest/sof"]` passthrough feature

Gaps in `crates/rest/src/state.rs`:
- `AppState<S>` has no `export_controller` field

Gaps in `crates/rest/src/config.rs`:
- `ServerConfig` has no `HFS_SOF_ENABLED` env var

Gaps in `crates/hfs/src/main.rs`:
- No `InMemoryExportController` initialization under `#[cfg(feature = "sof")]`

#### Phase 2 — Route Registration

All 9 planned routes are absent from `crates/rest/src/routing/fhir_routes.rs`. The existing `run_view_definition_handler` in `crates/sof/src/handlers.rs` is the migration source for `$viewdefinition-run` but cannot be copied verbatim — it is coupled to `crates/sof`'s own error types (`ServerError`) and models, and must be adapted to `helios-rest`'s `AppState<S>` and `OperationOutcome` error patterns.

#### Phase 3 — ExportController Trait

The entire `crates/rest/src/sof/` module directory does not exist. All four planned files need to be created from scratch. `dashmap` is not currently a dependency of `crates/rest` and must be added as optional under the `sof` feature.

#### Phase 4 — $viewdefinition-export Handlers

`crates/rest/src/handlers/sof/` does not exist. Depends on Phase 3.

#### Phase 5 — $sqlquery-run Handler

`crates/sof/src/sqlquery.rs` does not exist. DataFusion is the largest new dependency in the plan and is entirely absent from the workspace. This is the highest-risk phase.

#### Phase 6 — CapabilityStatement Updates

`crates/rest/src/handlers/capabilities.rs` exists but has no `#[cfg(feature = "sof")]` blocks or SOF operation entries.

---

### Issues Requiring Resolution Before Implementation

1. **`process_view_definition_generic` is private** — The plan references this function for Phase 4/5. It must be made `pub` or `pub(crate)` before it can be called from `crates/rest`. Alternative: use the public `process_view_definition` with minor API adaptation.

2. **SQL engine for Phase 5 is unconfirmed** — See Open Clarifying Questions above. DataFusion is absent from the workspace; adding it is a significant step.

3. **Cursor key alignment for `StreamingProvider`** — The plan proposes ordering by `id` alone. The existing keyset cursor uses a composite `last_updated + id` key. The `StreamingProvider` implementation should align with the existing pattern.

4. **`dashmap` missing from `crates/rest`** — Required for `InMemoryExportController`. Must be added as an optional dependency under the `sof` feature.

5. **`$viewdefinition-run` migration is non-trivial** — The handler in `crates/sof/src/handlers.rs` uses `crates/sof`'s `ServerError`, `RunQueryParams`, `ValidatedRunParams`, and `SofParameters` types. Migrating it into `crates/rest` requires rewriting the error mapping and parameter extraction to match `helios-rest` conventions.
