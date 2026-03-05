# S3 + Elasticsearch Polyglot Persistence (`s3-elasticsearch`) - Implementation

## Scope
Deliver `s3-elasticsearch` as a first-class composite backend:
- Primary source of truth: S3
- Search engine: Elasticsearch
- No S3 scan-based search fallback

## Architecture Decisions

1. Source of truth:
- Default: S3 is canonical for CRUD/read/version/history.
- Tradeoff: search may temporarily lag on partial sync failures; mitigated via retry and reindex.

2. Write ordering:
- Default: write to S3 first, then propagate to Elasticsearch.
- Tradeoff: avoids phantom indexed records not durably persisted, at the cost of possible short-lived index lag.

3. Partial failure semantics:
- Default (MVP): primary write success is preserved; ES sync failures are observable and repairable.
- Mitigation: per-backend sync failure logging + operator reindex utility.

4. Idempotency/conflict behavior:
- Deterministic document identity (`resource_type` + `id` per tenant index).
- Version-aware upsert:
  - duplicate replay ignored
  - stale updates ignored
- Delete replay idempotent (missing doc treated as converged).

5. Tenancy:
- S3 supports prefix-per-tenant and bucket-per-tenant modes.
- ES uses tenant-specific index naming with sanitization-safe segments and tenant metadata filtering.

6. History policy:
- ES indexes current searchable resource state.
- S3 remains authoritative for `_history`/`vread`.

## Milestone Deliverables

## M1
- First-class mode wiring:
  - `crates/rest/src/config.rs`
  - `crates/hfs/src/main.rs`
  - `crates/hfs/Cargo.toml`
- Added startup mode, shared search registry bootstrap, and expanded S3 env parsing.

## M2
- Search routing and sync semantics:
  - `crates/persistence/src/composite/storage.rs`
  - `crates/persistence/src/composite/sync.rs`
  - `crates/persistence/src/backends/elasticsearch/storage.rs`
  - `crates/persistence/src/backends/elasticsearch/backend.rs`
- Added S3-primary forced ES search routing, sync observability, idempotent indexing/deletes.

## M3
- Reindex + integration:
  - `crates/persistence/src/backends/s3/reindex_es.rs`
  - `crates/persistence/src/backends/s3/mod.rs`
  - `crates/persistence/tests/minio_s3_elasticsearch_tests.rs`
- Added startup-triggerable S3->ES reindex and MinIO+ES end-to-end suite.

## Operational Configuration

Required mode:
- `HFS_STORAGE_BACKEND=s3-elasticsearch`

S3 options:
- `HFS_S3_TENANCY_MODE`
- `HFS_S3_BUCKET`
- `HFS_S3_TENANT_BUCKET_MAP`
- `HFS_S3_DEFAULT_SYSTEM_BUCKET`
- `HFS_S3_REGION`
- `HFS_S3_PREFIX`
- `HFS_S3_ENDPOINT_URL`
- `HFS_S3_FORCE_PATH_STYLE`
- `HFS_S3_ALLOW_HTTP`
- `HFS_S3_VALIDATE_BUCKETS`

ES options:
- `HFS_ELASTICSEARCH_NODES`
- `HFS_ELASTICSEARCH_INDEX_PREFIX`
- `HFS_ELASTICSEARCH_USERNAME`
- `HFS_ELASTICSEARCH_PASSWORD`

Reindex trigger:
- `HFS_S3_ES_REINDEX_ON_STARTUP`
- `HFS_S3_ES_REINDEX_BATCH_SIZE`
- `HFS_S3_ES_REINDEX_CLEAR_EXISTING`
- `HFS_S3_ES_REINDEX_RESOURCE_TYPES`

## Testing and CI

Fast/unit:
- `cargo test -p helios-rest`
- `cargo test -p helios-hfs --features s3,elasticsearch`
- `cargo test -p helios-persistence --features s3,elasticsearch --test composite_s3_elasticsearch_tests`
- `cargo test -p helios-persistence --features s3,elasticsearch --test elasticsearch_tests -- --skip es_integration`

Container integration:
- `RUN_MINIO_S3_ES_TESTS=1 cargo test -p helios-persistence --features s3,elasticsearch --test minio_s3_elasticsearch_tests`

Stabilization:
- Explicit ES refresh in tests
- Retry/backoff (`assert_eventually`) for near-real-time indexing windows
- Unique tenant/index prefixes for isolation

## Risks & Mitigations

1. ES drift from S3 after partial sync failure.
- Mitigation: metadata-rich sync failure logs + startup/triggered reindex utility.

2. Out-of-order or duplicate sync events.
- Mitigation: version-aware idempotent upsert and idempotent delete handling.

3. ES refresh timing flakiness in tests.
- Mitigation: explicit refresh and retry loops before assertions.

4. Tenant IDs containing index-unsafe characters.
- Mitigation: ES index segment sanitization while retaining tenant metadata semantics.

5. Search parameter registry changes after data already indexed.
- Mitigation: document operational expectation to reindex after registry-changing startup/config updates.

## Acceptance Criteria Checklist

- [x] `s3-elasticsearch` parses and is selectable where `HFS_STORAGE_BACKEND` is interpreted.
- [x] HFS boots with S3 primary + ES search secondary via `CompositeStorage`.
- [x] Search/count requests in `s3-elasticsearch` execute against ES; S3 search is not required.
- [x] Create/update/delete sync from S3 primary into ES is covered by tests.
- [x] Multi-tenant behavior is covered in MinIO+ES integration tests.
- [x] Reindex utility rebuilds ES from S3 current objects.
- [x] Existing sqlite/postgres ES composite paths remain unchanged by design and pass targeted regression tests.
- [x] Documentation updated in persistence and root README plus milestone docs.

