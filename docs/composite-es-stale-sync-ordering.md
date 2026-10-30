# Ordering protection for composite Elasticsearch syncs (#2288)

Status: design. Code references are at `origin/main` @ `1384699f6`.
The ABAB numbers are not measured yet (see
`composite-es-stale-sync-ordering-bench.md`); the tables below say so.

## 1. Problem

In a composite deployment (primary such as SQLite/Postgres/MongoDB plus
Elasticsearch as search secondary) every ES write is a plain
`PUT _doc/{id}` and every delete is a plain `DELETE _doc/{id}`. The invariant
comment at `backends/elasticsearch/storage.rs:725-745` states this is
deliberate: HFS never sends `op_type=create`, an external `version`, or
`if_seq_no`/`if_primary_term`, so a retry can never receive a `409`.

The cost is that nothing orders writes to the same id:

- **Stale overwrite.** Two syncs of one id (v2 and v3) run concurrently, or
  the repair (`composite/sync_failures.rs`, `resync_current_state`) races the
  async worker. If v2 lands last, the index holds v2 while the primary holds
  v3, permanently and without any record.
- **Resurrected delete.** A late sync of v1 after the delete was applied
  recreates the document. Even a native ES delete only remembers the version
  for `index.gc_deletes` (60 s by default).
- The repair narrows the race by rereading the primary after pushing (up to
  `REPAIR_MAX_ROUNDS = 3`), but that is a best-effort mitigation, not a guard
  in ES: two concurrent syncs outside the repair (sync/hybrid mode,
  `$reindex`, ingest sink, bulk-submit, several HFS processes) are unprotected.

## 2. Options evaluated

| Option | Retries | Concurrent syncs outside repair | Write cost | Verdict |
|---|---|---|---|---|
| A. Repair-path compare only | unchanged | **not covered** | none | rejected: the race lives in the plain writers |
| B. `_update` with a painless script comparing `_source.version_id` | idempotent | covered, works on legacy docs | realtime get + script + reindex on every write | rejected: clear write-path cost |
| C. `if_seq_no`/`if_primary_term` | a retry after an applied write gets 409 | covered | needs a read before every write | rejected: reads first, and does not express FHIR order |
| D. `version_type=external` | every resend gets 409 | covered | none | rejected: breaks idempotent retries |
| **E. `version_type=external_gte` + tombstones** | **resend of same version accepted** | **covered** | **none extra (check happens in the existing `_id` lookup)** | **chosen** |

## 3. Chosen design

### 3.1 External version

Every write that carries a numeric FHIR `version_id` sends
`version = EXT_BASE + version_id` with `version_type=external_gte`, where
`EXT_BASE = 1 << 32`. `version_id` comes from `StoredResource::version_id()`;
the primaries (SQLite, Postgres, MongoDB) generate an integer that grows by
one per id, including on delete and recreate.

- Equal version: accepted, document rewritten unchanged (idempotent retry).
- Higher version: replaces the document.
- Lower version: ES answers `409 version_conflict_engine_exception`.

### 3.2 Retry semantics: 409 means "superseded"

A 409 with `version_conflict_engine_exception` is treated as **success with
outcome superseded**: the call returns Ok, the sync-failure ledger is cleared,
nothing is retried, and a `debug` log/metric records it. This applies to the
single-document path (`send_write_once`, currently classified a permanent
failure at `:919-926`) and to per-item `_bulk` results (`send_bulk_index`,
`:2395-2517`). Other 4xx remain permanent; 429/5xx remain retryable.

The rewritten invariant comment must say: retries are safe because a resend
carries the same version (`external_gte` accepts it), and a 409 now means
"something newer is already indexed".

### 3.3 Migration without reindex: `EXT_BASE`

Existing documents carry an *internal* `_version` that counts every PUT
(retries and reindex included), so it is usually larger than the FHIR
version. Without an offset the first guarded write over an old document would
receive a 409 and be lost. Offsetting by `2^32` puts every external version
above any inherited internal counter, so no reindex, new index or alias is
needed. The mapping reconcile works on the same index.

### 3.4 Deletes as versioned tombstones

A delete no longer removes the document. It writes a minimal document
`{resource_type, resource_id, tenant_id, version_id: N, last_updated,
fhir_version, is_deleted: true}` (no `content`, no `search_params`) with
`version = EXT_BASE + N`, where `N` is the tombstone version the primary
assigned.

- A native ES delete keeps its version only for `index.gc_deletes`. A tombstone
  is a real document, so it protects without a time limit.
- `is_deleted` is already mapped (`schema.rs:118`) and most queries already
  filter `is_deleted:false` (`search/query_builder.rs:105-110`,
  `search_impl.rs:1457,1490`, `storage.rs:1699,1861,2993,3044`).
- Recreate after delete uses `N+1 > N` and is accepted.

New API surface (contract task): `ReindexTarget::tombstone_search_entries`
(default delegates to `delete_search_entries`),
`ResourceStorage::delete_returning_version` (default calls `delete`, returns
`None`), `ResourceError::Gone.deleted_version`, `SyncEvent::Delete.version_id`.
The primaries already compute the tombstone version (SQLite and MongoDB as a
Rust local, Postgres in SQL, S3 via `mark_deleted()`); they only need to return
it. The composite dispatch (`sync.rs:1152-1169`) sends a Delete that has a
version and an `as_search_index()` target to `tombstone_search_entries`.

Callers and the version they use: `delete` uses `delete_returning_version`;
`delete_versioned` uses the value the primary returns (else `expected+1`);
conditional delete on the primary branch uses snapshot+1; `delete_with_match`
and `sync_bundle_results` use the returned value or the entry's etag.

### 3.5 Reads must never expose a tombstone

Audit result: `read`, `count`, `count_all_types`, `list_resource_types`,
`fetch_resources_page` and everything built by `EsQueryBuilder::build` already
filter. Gaps to close:

- `latest_write_marker` (`storage.rs:1974-2025`) filters only by tenant.
- `_contained` materialisation (`search_impl.rs:1123-1135`) uses a plain `ids`
  query.
- `parse_stored_resource` (`:2054`) and `parse_hit_to_stored_resource`
  (`search_impl.rs:1611`) turn a missing `content` into `{}`.
- `upsert`'s existence GET (`:1484-1534`) ignores `is_deleted`; on a tombstone it
  must answer "new".

### 3.6 Bulk and `$reindex`

`send_bulk_index` adds `"version"` and `"version_type":"external_gte"` to the
action line when the version is known (it is, via
`StoredResource::version_id()`, for `write_search_page`, `index_many`,
`write_synced_entries_page` and `write_search_entries_page`). A 409 item counts
as success. A stale `$reindex` page therefore cannot overwrite a newer
document or a tombstone.

### 3.7 Contained documents

Contained docs are separate ES documents (`_id = {Type}_{container}#{local}`).

- They are written only if the container write was accepted; a 409 on the
  container ends the write with no sweep and no contained writes.
- Each contained doc is indexed with `external_gte` using its container's
  version.
- The sweep (`delete_contained_docs`, a `_delete_by_query` today with no
  version condition) is bounded to `container_version < v` (or field absent,
  for legacy docs) so a late v2 writer cannot delete v3's contained docs.
- New field `container_version: long`, only on contained documents.
- After writing contained docs on the single-document path, a realtime GET of
  the container checks it is still at `v`; otherwise sweep
  `container_version < current`. This avoids orphans of a container deleted in
  between. Only writes with `contained[]` pay for it.

### 3.8 Cases that stay unguarded (same behaviour as today)

A write or delete goes out **without** `version`/`version_type`, with a `warn!`
and a metric, when:

- the version does not parse as `u64`;
- the version is the `"1"` fallback of `stored_from_synced_body`
  (`composite/storage.rs:122-128`); guarding it would reject valid updates;
- a delete has no known version (e.g. `bulk_submit::read_ingested` pushes a
  Delete on any read error, transient ones included, `bulk_submit.rs:396-421`);
- standalone ES paths `index_new`, `upsert`, `update`: `update` computes its own
  `current+1`, and `index_new`/`upsert` fall back to `"1"`. The code treats ES as
  a search secondary only (`storage.rs:2106`), so these stay unguarded.

### 3.9 Purge

`purge`, `purge_all`, `purge_tenant_data` and `clear_search_index*` keep
physically deleting, tombstones included, because the primary restarts its
versions. `purge` currently calls `delete` (`:2118-2134`) and must switch to a
physical delete. Residual risk: a late sync after a purge can re-create the
document. Documented, out of scope.

### 3.10 Repair path

The reread of `resync_current_state` stays as extra defence. When the primary
answers `Gone{deleted_version}` the repair writes a tombstone. The comparison
between rounds must include the deleted version, because today two deleted
states compare equal (`None == None`, `sync_failures.rs:640-646`), so
delete→recreate→delete between two reads goes unnoticed.

## 4. Mapping and storage impact

- One new field, `container_version: long`, on contained documents only
  (BKD index + doc_values, ~8 bytes + index overhead per contained doc).
  Top-level documents do not change. `SCHEMA_VERSION` 2 → 3, additive; the
  existing startup reconcile applies it with `put_mapping`.
- `external_gte` adds no I/O: the check happens in the version lookup that ES
  already performs for `_id`.
- Tombstones: a few hundred bytes each (no `content`, no `search_params`),
  growing with the number of deleted resources. Collection is out of scope.
- Extra write cost exists only for writes with `contained[]` (one realtime GET).

## 5. Test plan (summary)

Late v1 after v2 (read and search); delete then late v1, also with
`gc_deletes=0s`; idempotent resend of document and tombstone; late bulk page
and `$reindex` page; contained late writer and no orphans after container
delete; migration (doc written by unguarded PUTs accepts the first guarded
write); recreate after tombstone; composite repair does not resurrect and a
concurrent update is not overwritten. Wiremock: request carries `version` and
`version_type=external_gte`, 409 version conflict returns Ok, unversioned
writes send neither. Existing tests asserting the old behaviour are updated on
purpose: `index_requests_carry_nothing_that_makes_a_resend_conflict`,
the 409 row of `index_writes_do_not_retry_permanent_failures`, the bulk 409
transient test, `purge_all` counts in `elasticsearch_tests.rs`.

## 6. Performance evidence

Rule: ABAB, four pairs, A = `1384699f6`, B = branch, same host and corpus,
`sqlite-elasticsearch` in synchronous and asynchronous mode, crud, insert and
search legs. This is a correctness change, so the acceptance criterion is "no
worsening beyond the run-to-run spread", not an improvement.

| Leg / mode | Metric | A (abs) | B (abs) | Change % | Spread A / B |
|---|---|---|---|---|---|
| crud sync | p50 / p95 / throughput | not measured | not measured | | |
| crud async | p50 / p95 / throughput | not measured | not measured | | |
| insert sync | p50 / p95 / throughput | not measured | not measured | | |
| insert async | p50 / p95 / throughput | not measured | not measured | | |
| search sync | p50 / p95 | not measured | not measured | | |
| search async | p50 / p95 | not measured | not measured | | |

Index size (`_cat/indices` store.size and docs.count) before and after the crud
leg, to measure tombstone cost: not measured.

Status: the benchmark needs a `fhir-benchmark.yml` dispatch (awaiting approval,
and the baseline run 39657850695 must finish first); the protocol is in the
bench doc. Correctness run against real ES: 11 of 12 tests pass; the failing one
is a test defect described in the bench doc §1.

## 7. Out of scope

The reconciler, lost-sync recording, tombstone garbage collection, and
protection against late syncs after a purge.

## Physical deletes and the 409 recheck

A physical delete (`$reindex` with `clearExisting`, purge, `$expunge`) leaves
the deleted document's version in Elasticsearch until `index.gc_deletes`
expires, and that version is above the FHIR version a rebuild writes. A guarded
write refused with 409 is therefore read as "superseded" only when a document
(or tombstone) is still stored under that id; when a realtime GET finds nothing,
the write is repeated without a guard. This costs one GET per refused write,
which only happens on a conflict, never on the normal write path.
