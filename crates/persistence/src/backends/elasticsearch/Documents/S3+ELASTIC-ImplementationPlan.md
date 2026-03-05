# S3 + Elasticsearch Polyglot Persistence (MVP) Implementation Plan

## Summary
Deliver `s3-elasticsearch` as a first-class backend mode using existing `CompositeStorage` search-offloading architecture: S3 is canonical for CRUD/version/history; Elasticsearch is the only search engine for this mode (no S3 object scanning for search).  
The plan is additive and keeps existing `sqlite-elasticsearch` and `postgres-elasticsearch` behavior intact.

## Current-State Assessment
- Already exists:
- S3 primary backend with CRUD/version/history/bulk and explicit `SearchProvider` unsupported behavior in [s3/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/s3/storage.rs).
- Elasticsearch backend with document schema/indexing/search and tenant+resource index naming in [elasticsearch/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/storage.rs), [elasticsearch/schema.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/schema.rs), [elasticsearch/backend.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/backend.rs).
- Composite write-through + sync retry infrastructure in [composite/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/storage.rs) and [composite/sync.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/sync.rs).
- Feature flags `s3` and `elasticsearch` already present in [persistence/Cargo.toml](/Users/acruzgon/Documents/helios/hfs/crates/persistence/Cargo.toml).
- Missing to make S3+ES first-class:
- Backend selection and startup wiring for `s3-elasticsearch` in [rest/config.rs](/Users/acruzgon/Documents/helios/hfs/crates/rest/src/config.rs) and [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs).
- ES registry bootstrap equivalent to SQLite/Postgres shared registry (S3 has no native search registry to share).
- Composite search routing for a primary that cannot search (S3): avoid any S3 `search()` execution in this mode.
- Explicit sync semantics, idempotency policy, and failure observability for S3→ES propagation.
- Reindex/repair procedure to rebuild ES from S3 current objects.
- End-to-end MinIO+ES integration coverage and docs that mark S3+ES as implemented.

## Architecture Decisions (Tradeoffs + Default)
1. Source of truth:
- Option A: ES as source for reads/search.
- Option B (Default): S3 is canonical for all reads/writes/history; ES is secondary search index.
2. Write ordering:
- Option A: ES first, then S3.
- Option B (Default): S3 first, then ES sync. Prevents search index containing records not durably persisted.
3. Partial failure behavior:
- Option A: fail request if ES sync fails.
- Option B (Default for MVP): primary write succeeds; ES sync is best-effort with retries + observable failure status; operator repair via reindex.
4. Sync execution mode:
- Option A: asynchronous queue worker.
- Option B (Default for MVP): synchronous write-through for `s3-elasticsearch` mode to minimize indexing lag and simplify correctness.
5. ES idempotency/conflict handling:
- Option A: plain upsert (last-write-wins).
- Option B (Default): deterministic doc IDs plus version-aware guards (ignore stale updates, tolerate duplicate replay).
6. Tenancy:
- Option A: prefix-per-tenant only.
- Option B (Default): support both S3 tenancy modes; ES remains per-tenant index pattern with tenant filter in documents.
7. Search scope for MVP:
- Default: all FHIR search operations in this mode execute in ES; unsupported advanced features remain unsupported (no fallback by scanning S3).
8. History/version indexing policy:
- Default: ES stores only current live documents; `_history`/`vread` served only by S3.

## Milestones

### M1 — First-Class Mode Wiring
1. Add `S3Elasticsearch` backend mode and aliases (`s3-elasticsearch`, `s3-es`) in [rest/config.rs](/Users/acruzgon/Documents/helios/hfs/crates/rest/src/config.rs); purpose: selectable mode parity with existing `*-elasticsearch`; tests: update parse/display/unit tests in same file.
2. Add `start_s3_elasticsearch()` path in [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs); purpose: instantiate S3 primary + ES search backend + `CompositeStorage` (`primary("s3", BackendKind::S3)` + `search_backend("es", BackendKind::Elasticsearch)`); tests: startup/config smoke test in new [hfs/tests/storage_mode_s3_es.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/tests/storage_mode_s3_es.rs).
3. Build shared ES search registry at startup (from configured FHIR version/data directory) and pass to `ElasticsearchBackend::with_shared_registry`; files: [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs), optionally [elasticsearch/backend.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/backend.rs) if helper extraction is centralized; tests: registry-load unit tests for startup helper.
4. Expand S3 env parsing used by both `s3` and `s3-elasticsearch` startup paths (endpoint URL, allow_http, force_path_style, tenancy mode); file: [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs); tests: env parsing table tests in new [hfs/tests/s3_config_env_tests.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/tests/s3_config_env_tests.rs).
5. Documentation checkpoint M1 at [crates/persistence/docs/s3-elasticsearch/M1.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/docs/s3-elasticsearch/M1.md).

### M2 — Search Routing, Sync Semantics, Idempotency
1. Ensure `s3-elasticsearch` never executes S3 search paths by forcing search delegation to configured search backend; file: [composite/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/storage.rs); functions: `execute_routed_search`, `execute_primary_search`, `search_count`; tests: new [composite_s3_elasticsearch_tests.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/tests/composite_s3_elasticsearch_tests.rs) with assertion that S3 search capability is not required.
2. Harden sync observability by evaluating per-backend `SyncStatus` results and logging failures with tenant/resource/event metadata; files: [composite/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/storage.rs), [composite/sync.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/sync.rs); tests: sync failure unit tests in [composite/sync.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/sync.rs).
3. Make ES indexing idempotent and stale-update-safe in sync path (doc-id deterministic, duplicate replay safe, stale update ignored); file: [elasticsearch/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/storage.rs), with supporting event metadata in [composite/sync.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/sync.rs); tests: ES storage unit/integration tests in [elasticsearch_tests.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/tests/elasticsearch_tests.rs).
4. Make delete propagation idempotent in sync flow (missing doc treated as already converged); files: [composite/sync.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/composite/sync.rs), [elasticsearch/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/storage.rs); tests: delete replay tests in new composite S3+ES test module.
5. Documentation checkpoint M2 at [crates/persistence/docs/s3-elasticsearch/M2.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/docs/s3-elasticsearch/M2.md).

### M3 — Reindex/Repair and End-to-End Validation
1. Add an S3→ES reindex utility that scans S3 current objects, batches resources, and replays index writes into ES; files: new [s3/reindex_es.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/s3/reindex_es.rs), [s3/mod.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/s3/mod.rs), reuse helpers in [s3/storage.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/s3/storage.rs); tests: unit tests for key parsing/pagination and batch replay behavior.
2. Expose a practical operator trigger (startup flag or explicit API hook) for “rebuild ES from S3” with `clear_existing` and resource-type filters; files: [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs) and docs; tests: integration test scenario invoking reindex and verifying post-rebuild search.
3. Add MinIO+Elasticsearch integration tests using testcontainers; file: new [minio_s3_elasticsearch_tests.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/tests/minio_s3_elasticsearch_tests.rs); gating: `RUN_MINIO_S3_ES_TESTS=1`; scenarios:
- CRUD roundtrip with reads from S3.
- Search served by ES after write-through sync.
- Delete propagation removes/search-excludes resource.
- Tenant isolation across both stores.
- Reindex rebuild restores ES after index wipe.
4. Stabilize integration tests with explicit ES refresh and retry/backoff helpers to handle refresh timing; files: [minio_s3_elasticsearch_tests.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/tests/minio_s3_elasticsearch_tests.rs), [elasticsearch/backend.rs](/Users/acruzgon/Documents/helios/hfs/crates/persistence/src/backends/elasticsearch/backend.rs) (`refresh_index` usage).
5. Documentation checkpoint M3 at [crates/persistence/docs/s3-elasticsearch/M3.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/docs/s3-elasticsearch/M3.md).

### M4 — Docs Finalization + Release Readiness
1. Mark S3+ES as implemented and document search-offloading semantics in [crates/persistence/README.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/README.md); include write path, failure semantics, reindex runbook, and unsupported/deferred items.
2. Update user-facing backend docs and env var tables in [README.md](/Users/acruzgon/Documents/helios/hfs/README.md), [hfs/main.rs](/Users/acruzgon/Documents/helios/hfs/crates/hfs/src/main.rs), [rest/config.rs](/Users/acruzgon/Documents/helios/hfs/crates/rest/src/config.rs).
3. Add local dev recipe (MinIO + ES) and verification commands to [crates/persistence/README.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/README.md).
4. Consolidated implementation write-up at [crates/persistence/docs/s3-elasticsearch/IMPLEMENTATION.md](/Users/acruzgon/Documents/helios/hfs/crates/persistence/docs/s3-elasticsearch/IMPLEMENTATION.md).

## Testing & CI Plan
- Unit/fast:
- `cargo test -p helios-rest` for backend mode parsing changes.
- `cargo test -p helios-persistence --features s3,elasticsearch --test composite_s3_elasticsearch_tests`.
- Docker integration (opt-in):
- `RUN_MINIO_S3_ES_TESTS=1 cargo test -p helios-persistence --features s3,elasticsearch --test minio_s3_elasticsearch_tests`.
- Regression safety:
- Run existing `elasticsearch_tests`, `minio_s3_tests`, and baseline composite tests to ensure no regression.
- Stabilization:
- Unique tenant IDs + index prefixes per test.
- Explicit ES refresh before assertions.
- `assert_eventually` retries for post-write search visibility.

## Operational Plan
- New/updated configuration knobs:
- `HFS_STORAGE_BACKEND=s3-elasticsearch`.
- S3: `HFS_S3_TENANCY_MODE`, `HFS_S3_BUCKET`, `HFS_S3_TENANT_BUCKET_MAP`, `HFS_S3_DEFAULT_SYSTEM_BUCKET`, `HFS_S3_REGION`, `HFS_S3_PREFIX`, `HFS_S3_ENDPOINT_URL`, `HFS_S3_FORCE_PATH_STYLE`, `HFS_S3_ALLOW_HTTP`, `HFS_S3_VALIDATE_BUCKETS`.
- ES: `HFS_ELASTICSEARCH_NODES`, `HFS_ELASTICSEARCH_INDEX_PREFIX`, `HFS_ELASTICSEARCH_USERNAME`, `HFS_ELASTICSEARCH_PASSWORD`.
- Reindex: `HFS_S3_ES_REINDEX_ON_STARTUP` (or equivalent trigger), `HFS_S3_ES_REINDEX_BATCH_SIZE`, `HFS_S3_ES_REINDEX_CLEAR_EXISTING`.
- Local dev stack recipe (to document):
- MinIO container + bucket bootstrap.
- Elasticsearch single-node container (`xpack.security.enabled=false` for local).
- HFS start command with `s3-elasticsearch` and local endpoint settings.
- Observability:
- Structured logs for sync failures/retries and lag indicators.
- Surface sync status counters (`pending_events`, `total_errors`) in debug logs and health diagnostics docs.
- Health expectation: primary (S3) healthy + ES healthy required for search readiness.

## Risks & Mitigations
- ES drift from S3 after partial failures; mitigation: retry + explicit reindex utility + failure counters/logs.
- Out-of-order sync events causing stale index state; mitigation: version-aware idempotent indexing.
- Test flakiness from ES refresh timing; mitigation: explicit refresh + retries/backoff.
- Hierarchical tenant IDs producing invalid ES index names; mitigation: sanitize tenant index segment while preserving raw `tenant_id` field filter.
- Runtime custom SearchParameter lifecycle with S3 primary is not fully equivalent to SQL primaries; mitigation: document MVP scope and require reindex after startup registry changes.

## Acceptance Criteria Checklist
- [ ] `s3-elasticsearch` parses and is selectable everywhere `HFS_STORAGE_BACKEND` is interpreted.
- [ ] HFS boots in `s3-elasticsearch` mode with S3 as primary + ES as search secondary using `CompositeStorage`.
- [ ] All search requests in this mode execute against ES; S3 search is never used.
- [ ] Create/update/delete on S3 are propagated to ES and validated by integration tests.
- [ ] Multi-tenant isolation is verified across S3 object layout and ES index/query filtering.
- [ ] Reindex procedure rebuilds ES from S3 current objects and passes integration test.
- [ ] Existing `sqlite-elasticsearch` and `postgres-elasticsearch` modes remain functional.
- [ ] Docs updated in root and persistence READMEs, plus milestone docs under `/crates/persistence/docs/s3-elasticsearch/`.
- [ ] Formatting/lint/tests pass for touched crates (`cargo fmt --all`, `cargo clippy` with project flags, affected `cargo test` targets).

## Assumptions & Defaults
- Canonical backend name is `s3-elasticsearch`; alias `s3-es`.
- MVP prioritizes correctness and observability over asynchronous queue complexity.
- S3 remains authoritative for history/versioning; ES indexes only current searchable state.
- No S3 scan-based search fallback will be implemented.
