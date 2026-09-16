# MongoDB `search_index` generation 2: covered partial indexes, background build, bounded `_contained` search

Issues: [#1084](https://github.com/HeliosSoftware/hfs/issues/1084) (no value-typed index carries `resource_id`, no search plan can be covered) and [#1059](https://github.com/HeliosSoftware/hfs/issues/1059) (`_contained=true|both` scans the whole tenant's `search_index`).

Status: approved design, 2026-09-15. Implementation plan to follow.

## 1. Problem

Every value-filtered search on the MongoDB backend fetches one document per matching index key, because `resource_id` is not in any value-typed index. Every `_contained` search scans the tenant's entire `search_index`, because neither `is_contained` nor `contained_type` is indexed. Both fixes need new indexes on a collection whose builds are, today, awaited inline at every boot with no timeout, so a large deployment would sit in startup for the length of the build.

Measured on the 11.2M-resource corpus (MongoDB 7.0.40, tenant `default`):

| fact | value |
|---|---|
| `search_index` rows | 227,973,345 |
| `search_index` data, compressed | 14.2 GB |
| `search_index` indexes, total | 64.3 GB across 12 indexes |
| each value index (`string`, `token`, `date`, `number`, `quantity`, `reference`, `uri`, `token_display`, `identifier_type`) | 5.1 to 6.0 GB, none sparse or partial |
| `Observation?code=8302-2` (175,355 rows) | 175,358 keys, 175,355 docs fetched, 8 to 12 s |
| `Observation?status=final` (7.7M rows) | 7,699,979 keys, 7,699,978 docs fetched, 67 s |
| `countDocuments({tenant, is_contained: true})` | exceeds a 900 s limit; no index reaches those rows |

Every `search_index` row populates exactly one value field, yet every value index carries an entry for all 228M rows: `idx_search_uri` is as large as `idx_search_string`. Nine of the twelve indexes are mostly null keys.

Verified on a throwaway collection in the same server: an index declared with `partialFilterExpression: {value_x: {$exists: true}}` is chosen by the planner for equality, `$in`, range and anchored-regex predicates on `value_x`, and with `resource_id` as the trailing key the scan is fully covered (`totalDocsExamined: 0`) for a projection of `resource_id`.

Also measured, and deliberately not a driver of this design: the system-first key order of `idx_search_token` costs nothing on this corpus for a bare `code=8302-2` (175,358 keys for 175,355 rows), because `Observation.code` has a single system. Postgres measured a 1,350x difference on multi-system parameters (`class`), so the code-first order is taken here because the index is being rebuilt anyway, not because it was measured to matter on this data.

## 2. Decisions already made

- **Build model:** HFS builds large indexes itself, in the background, after boot. Operators may pre-build out of band; that makes the builder a no-op.
- **New names, never changed keys.** MongoDB answers `IndexKeySpecsConflict` (86) to a different key spec under an existing name, and `create_index_with` propagates it, so an in-place change would fail every deployed boot. Generation 2 lives under new names; generation 1 is dropped afterwards.
- **Partial indexes.** Every generation-2 value index is partial on its own leading value field existing. This is what turns "add `resource_id` to nine indexes" from a 64 GB to roughly 15 GB footprint.
- **#1059 scope:** the contained index and the query-path fixes (server-side paging, batched container fetch, single resolution for `_contained=both`), in one change.
- **`idx_search_composite` and `idx_search_resource` are untouched.** Both lead with `resource_id`; the first is hinted by name in the id-materialisation path, the second serves reindex deletes.

## 3. Index catalog

`crates/persistence/src/backends/mongodb/schema.rs` replaces the eleven inline `create_index` calls in `ensure_search_indexes` with a declarative catalog:

```rust
pub(crate) struct SearchIndexSpec {
    pub name: &'static str,
    pub keys: Document,                       // insertion order is key order
    pub partial: Option<Document>,            // partialFilterExpression
    pub build: IndexBuild,                    // Inline | Background
}
pub(crate) enum IndexBuild { Inline, Background }
```

Generation 2 (all `Background`, all on `search_index`). `T` is shorthand for the leading triple `tenant_id, resource_type, param_name`:

| name | keys | partial filter |
|---|---|---|
| `idx_search_string_v2` | T, `value_string`, `resource_id` | `value_string` exists |
| `idx_search_token_v2` | T, `value_token_code`, `value_token_system`, `resource_id` | `value_token_code` exists |
| `idx_search_date_v2` | T, `value_date`, `resource_id` | `value_date` exists |
| `idx_search_number_v2` | T, `value_number`, `resource_id` | `value_number` exists |
| `idx_search_quantity_v2` | T, `value_quantity_value`, `value_quantity_unit`, `resource_id` | `value_quantity_value` exists |
| `idx_search_reference_v2` | T, `value_reference`, `resource_id` | `value_reference` exists |
| `idx_search_uri_v2` | T, `value_uri`, `resource_id` | `value_uri` exists |
| `idx_search_token_display_v2` | T, `value_token_display`, `resource_id` | `value_token_display` exists |
| `idx_search_identifier_type_v2` | T, `value_identifier_type_system`, `value_identifier_type_code`, `resource_id` | `value_identifier_type_system` exists |
| `idx_search_contained` | `tenant_id`, `contained_type`, `is_contained`, `param_name`, `resource_type`, `resource_id`, `contained_local_id` | `is_contained` is `true` |

Unchanged and `Inline`: `idx_search_composite` (`tenant_id, resource_type, resource_id, param_name, composite_group`) and `idx_search_resource` (`tenant_id, resource_type, resource_id`).

Superseded, dropped by the builder once every `_v2` twin is ready: `idx_search_string`, `idx_search_token`, `idx_search_date`, `idx_search_number`, `idx_search_quantity`, `idx_search_reference`, `idx_search_uri`, `idx_search_token_display`, `idx_search_identifier_type`.

Notes on individual specs:

- The contained index is partial on `is_contained: true`, so it holds only contained rows. Non-contained rows never carry the field (`build_contained_index_document` is the only writer), which is why a plain sparse index would not have helped and a partial one does. The trailing `resource_type, resource_id, contained_local_id` let the contained pipeline's `$group` read its key from the index.
- `_id` and `_lastUpdated` rows written by `index_minimal_fallback_documents` carry `value_token_code` and `value_date` respectively, so they land in `idx_search_token_v2` and `idx_search_date_v2` as they land in the v1 indexes today.
- The `$or` filter shapes emitted by `search_impl.rs` are unchanged. The token filter's three shapes (`code`, `system|code`, `system|`) need no edit for the code-first key order; the first two bind on the leading value keys. The third, `system|` with no code, becomes an unbounded middle-key scan of the parameter slice. It is rare and accepted; if it appears in practice, a small partial index on `value_token_system` can be added under the same catalog without touching anything else.
- `regex` predicates on `value_string` and `value_reference` keep working: an anchored regex derives bounds on the partial index exactly as on the full one (verified in the probe).

## 4. Background builder

### 4.1 Boot

`initialize_schema_async` keeps creating every `Inline` index across all collections, then returns. `MongoBackend::init_schema` spawns the `SearchIndexBuilder` task after `initialize_schema_async` and before the registry reload, and stores its `JoinHandle` on the backend so tests and the `inline` mode can await it. `migrate_schema_async` is left as it is: nothing calls it today, and this design does not start calling it.

### 4.2 The task

Steps, in order, each logged at `info`:

1. **Inspect.** `listIndexes` on `search_index`. For each `Background` spec: present with identical keys and options and no `buildUUID` means done; present with a `buildUUID` means another process is building it, so wait for it rather than issuing a second build; present with different keys or options is a hard error (see 4.4); absent means to build.
2. **Build.** One `createIndexes` command carrying every absent spec. MongoDB 4.2+ builds all indexes in one command with a single collection scan and takes no long-lived lock, so reads and writes proceed. The command returns when the build finishes. The task awaits it with no timeout: the work is bounded by one scan and the server is already serving.
3. **Drop.** Only when every generation-2 spec is present and ready, drop each superseded v1 name through the existing `drop_index_if_present`, one at a time, logging each.
4. **Record.** Update the schema-version document with `search_indexes: { generation: 2, completed_at: <ISO instant> }`. The next boot still runs step 1 (it is one cheap command and it is what makes the whole task idempotent), but finds nothing to build or drop.

The task never touches indexes on other collections and never touches `idx_search_composite` or `idx_search_resource`.

### 4.3 Modes

`HFS_MONGODB_INDEX_BUILD`, read into `MongoBackendConfig::index_build`:

| value | behaviour |
|---|---|
| `background` (default) | as above; boot returns before step 2 completes |
| `inline` | `init_schema` awaits the task before returning, so the server does not serve until generation 2 is ready. For tests, developer databases and small deployments |
| `off` | steps 1 and 4 only. Every absent generation-2 index is logged as a `warn` naming the index and the mongosh command that builds it. Nothing is built and nothing is dropped |

The MongoDB test helpers (`mongodb_tests.rs::create_backend*` and the shared testcontainer fixtures) set `inline`, because existing explain-based tests assert winning plans immediately after boot and must not race the build.

### 4.4 Failures

- `createIndexes` fails (disk full, build killed, connection lost): log at `error` with the server message, exit the task. Nothing has been dropped; every query keeps running on generation 1; the next boot retries because every step is idempotent.
- A generation-2 name exists with different keys or options: log at `error` naming the index and both key specs, exit the task without building or dropping. This is the only case the builder refuses to touch, because it means a person built something under our name.
- The process is killed mid-build: MongoDB 7.0 aborts and cleans up an in-progress build on restart (or completes it on a replica set with a primary), and step 1 on the next boot sees whichever state resulted.
- `drop_index_if_present` fails for a reason other than "not found": log at `error`, exit the task. Generation 2 is complete and used; the leftover v1 index costs writes until the next boot retries the drop.

The task exposes nothing new over HTTP. Its state is observable through the log and the `search_indexes` field of the schema-version document. A health or metrics surface is out of scope here and belongs with `helios-observability` if it is wanted.

### 4.5 What query code has to know

Nothing. The planner chooses between v1 and v2 while both exist. The single hint by name in `search_impl.rs` targets `idx_search_composite`, which is not in the catalog change. No `hint` is added for the v2 indexes: the probe showed the planner selecting the partial index unaided for every predicate shape the filter builders emit.

## 5. Contained search rewrite

`search_impl.rs::matching_contained` and `search_contained` are replaced by one pipeline and one batched fetch.

### 5.1 Pipeline

```text
$match  { tenant_id, contained_type, is_contained: true, $or: [ <per-parameter branch> ... ] }
$group  { _id: { rtype: $resource_type, rid: $resource_id, lid: $contained_local_id },
          names: { $addToSet: $param_name } }                    # always per contained entity
$match  { names: { $all: [<distinct parameter names>] } }        # only when more than one parameter; the AND holds within one entity
$group  { _id: { rtype: $_id.rtype, rid: $_id.rid } }            # container return only: collapse to one slot per container
$sort   { _id.rtype: 1, _id.rid: 1, _id.lid: 1 }
$facet  { page:  [ { $skip: offset }, { $limit: count } ],
          total: [ { $count: n } ] }                              # total branch only when _total is requested
```

The `$match` fields are written in the index key order so the prefix `tenant_id, contained_type, is_contained` binds, and each `$or` branch carries `param_name` plus its value predicate as today (`build_search_index_filter("", "", param)` minus the tenant and type fields). `$group` reads `resource_type`, `resource_id` and `contained_local_id` from the index keys. Grouping is two-stage (implementation rulings 2026-09-15): the first `$group` is always per contained entity so a multi-parameter AND is evaluated within one entity; in container-return mode a second `$group` collapses to one slot per container, so `_total` and page boundaries count containers rather than entities. `offset` and `count` come from `_offset` and `_count`, defaulting as they do in `search()`.

### 5.2 Container fetch

One `find` on `resources` for the page: `{ tenant_id, is_deleted: false, $or: [ { resource_type: rtype, id: rid } ... ] }` over the distinct `(rtype, rid)` pairs of the page, hinted onto `idx_resources_identity`. Results are placed back into pipeline order by a map keyed on `(rtype, rid)`. For `ContainedReturn::Contained`, `extract_contained_resource` and `build_contained_stored` run on the fetched containers exactly as today.

### 5.3 `_contained=both`

Resolve each source once, page each on the server, concatenate top-level before contained:

```text
top_total   = count of the standard search (query with contained=Off, contained_return=Container)
if offset < top_total:
    top_page       = standard search with offset, count
    remaining      = count - len(top_page)
    contained_page = pipeline with offset 0, limit remaining      # skipped when remaining == 0
else:
    top_page       = []
    contained_page = pipeline with offset - top_total, limit count
total (when requested) = top_total + contained_total
```

Contained results whose container already appears in the top-level page are dropped from the contained page, as today (`top_urls` check). There is no refill (implementation ruling 2026-09-15): an offset-based refill would fetch exactly the next page's first keys and re-emit them there, so a page may instead come back short by the number of dual matches. Dedupe is against the current top-level page only, as before; and `total` counts a dual-match container in both sources. `top_total` reuses the standard search's `_total` path, so the standard search runs once with `_total=accurate` semantics rather than a separate count; a missing total there is an internal error. The top-level portion keeps the standard search's default order (newest first); the contained portion is sorted by its group key.

### 5.4 Unchanged

`ContainedMode`, `ContainedReturn`, `extract_contained_resource`, `build_contained_stored`, the `_contained` parameter parsing in `helios-rest`, and the SQLite, Postgres and Elasticsearch contained paths.

## 6. Rollout

**Upgrade of a running deployment.** Deploy the binary. The server serves immediately on generation 1. The builder logs the start of one `createIndexes` covering every v2 spec, MongoDB scans `search_index` once, and the v1 indexes are dropped when the command returns. Disk peaks at v1 plus v2 during the build (about 64 plus 15 GB on the corpus) and settles at v2. Writes during the build are slower by the cost of maintaining both generations.

**Operator pre-build.** `docs/` gains a mongosh script whose `createIndexes` document is generated from the catalog (a unit test asserts the two agree), for operators who want the build in a window of their choosing or want `HFS_MONGODB_INDEX_BUILD=off`. A pre-built database makes step 2 a no-op.

**Downgrade.** An older binary calls `create_index` inline for the nine v1 names at boot. If they have been dropped, that boot rebuilds them synchronously before serving, for as long as nine full-collection scans take. The upgrade notes say so and give the mongosh command that recreates the v1 set in the background before a rollback.

**Fresh deployments and tests.** `search_index` is empty at first boot; the build is instant in either mode.

## 7. Testing

Unit, in `schema.rs`:

- every `Background` spec has a partial filter whose field is the spec's leading value key, or is the contained spec;
- every value spec ends in `resource_id`;
- no generation-2 name collides with a generation-1 name, and the superseded list is exactly the nine v1 value names;
- the mongosh pre-build script in `docs/` is byte-equal to the catalog's generated `createIndexes` document.

Integration, against a testcontainer (skip without Docker, and the report must show they ran):

- fresh database: after `inline` boot, `listIndexes` on `search_index` equals the generation-2 set plus the two unchanged indexes and `_id_`;
- database seeded with the v1 set and data: after boot, v2 present and ready, v1 absent, schema document records generation 2;
- a v2 name pre-created with different keys: builder exits with an error log, v1 still present, no v2 built;
- `off` mode on a v1 database: one warning per missing v2 index, nothing built, nothing dropped;
- second boot on a completed database: builder logs "nothing to do", no `createIndexes` issued (asserted through the log capture pattern already used in `mongodb_tests.rs`).

Explain-based, following the existing `explain` tests in `mongodb_tests.rs`:

- a date-range search and a bare token search report `totalDocsExamined: 0` on their `_v2` index;
- a `_contained=true` search reports `idx_search_contained` as the winning plan's index.

Contained behaviour:

- `_count` and `_offset` paging across a contained result larger than one page, with stable order between pages;
- `_total` equals the number of distinct contained matches;
- `_contained=both` paging across the boundary between top-level and contained results, including the case where a container appears in both;
- `_contained=true&_containedType=contained` still returns the extracted contained resources.

Corpus, recorded in the PR after the build completes on `hfs-mongo`:

- `Observation?code=8302-2` and `Observation?status=final`: keys, docs examined, ms, before and after;
- the contained-row count that currently exceeds 900 s;
- `search_index` `totalIndexSize` before and after.

## 8. Out of scope

- The `$or`-must-be-bounded rule for filter shapes (landed with #1083).
- A health or metrics surface for the build (belongs with `helios-observability`).
- `migrate_schema_async` and the `StorageBackend::migrate` path, which nothing calls.
- Elasticsearch, Postgres and SQLite: Postgres already has partial, `INCLUDE (resource_id)` indexes and a contained partial index; this design brings MongoDB to the same shape.
- Bounding `_revinclude` or `ResourceScan` (#1060, #1061).
