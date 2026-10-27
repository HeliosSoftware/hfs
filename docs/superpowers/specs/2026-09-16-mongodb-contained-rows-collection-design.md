# MongoDB: contained-resource index rows move to their own collection

Issue: #1160. Backend: `helios-persistence`, MongoDB only.

## Problem

On MongoDB a standard search (`_contained` off, the default) matches a container
resource through the values of its **contained** resources whenever the
contained resource has the same type as the container. `Observation?code=X`
returns an Observation whose only `code = X` lives inside a contained
Observation. SQLite and PostgreSQL exclude contained rows from standard searches
(`is_contained = 0` / `is_contained = FALSE` in every `search_index` subquery);
MongoDB does not.

Cause: contained rows are written into the same `search_index` collection as the
container's own rows, with the container's `resource_type` and `resource_id`,
plus `is_contained: true`, `contained_type` and `contained_local_id`
(`build_contained_index_document`, `storage.rs`). Every standard-search envelope
is `{tenant_id, resource_type, param_name, <value predicate>}` and nothing else,
so a same-type contained row satisfies it.

The obvious fix, `is_contained: {$ne: true}` in the envelope, is not acceptable:
`is_contained` is not a key of any generation-2 value index (#1084), so the
predicate becomes a residual filter and every matching row is fetched again,
undoing the covered scans that #1162 delivered.

## Decision

Contained rows move to a second collection, `search_index_contained`. The
standard search never reads that collection, so it excludes contained rows by
construction: no envelope changes, no index rebuilds, nothing to measure. The
`_contained` search pipeline reads the new collection with a plain compound
index instead of today's partial `idx_search_contained` on `search_index`.

Alternatives rejected: adding `is_contained` as a trailing key of the nine
value indexes and filtering with `$ne: true` keeps scans covered but rebuilds
every value index on every existing deployment (29 minutes on the 11M-resource
corpus, hours beyond that); a sentinel `resource_type` on contained rows avoids
the rebuild but puts a hack in the field that deletes and the contained pipeline
key on.

## Data model

### `search_index` (unchanged shape, contained rows gone)

Rows are exactly today's own-value rows. After the migration no row in this
collection carries `is_contained`, `contained_type` or `contained_local_id`.
Generation-2 value indexes, `idx_search_composite` and `idx_search_resource`
are unchanged. `idx_search_contained` is dropped from this collection.

### `search_index_contained` (new)

One row per (container, contained entity, parameter, value), the same document
`build_contained_index_document` produces today minus `is_contained` (every row
in this collection is contained by definition):

| field | meaning |
|---|---|
| `tenant_id` | tenant |
| `resource_type`, `resource_id` | the **container** (unchanged meaning, so deletes and the container fetch keep their keys) |
| `contained_type` | the contained resource's type |
| `contained_local_id` | the contained resource's `#id` |
| `param_name`, `param_type`, value fields, `composite_group` | as in `search_index` |

Index, created inline at boot next to `idx_search_composite` and
`idx_search_resource` (the collection is small; contained resources are rare):

```
idx_search_contained  { tenant_id: 1, contained_type: 1, param_name: 1,
                        resource_type: 1, resource_id: 1, contained_local_id: 1 }
```

Same keys as today's partial index minus `is_contained`, so the contained
pipeline's `$group` still reads its key from the index. A second index
`idx_search_contained_resource { tenant_id, resource_type, resource_id }` serves
the delete-by-container paths, mirroring `idx_search_resource`.

Constants: `MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION = "search_index_contained"`
next to `SEARCH_INDEX_COLLECTION`.

## Write paths

`search_index_documents(tenant, type, id, resource)` today returns one `Vec` of
own rows followed by contained rows. It returns a pair
`(own: Vec<Document>, contained: Vec<Document>)`; every caller inserts `own`
into `search_index` and `contained` into `search_index_contained`, each only
when non-empty, inside the same session when one is active. Callers:

- `index_resource` (create, update, restore, `$reindex` through `ReindexTarget`).
- `index_resource_in_bundle_transaction` (transaction bundles).
- `bulk_ingest::write_search_index` (batched bulk submit ingest).

The `_contained` extraction (`extract_contained`) is unchanged.

## Delete paths

Every place that deletes `search_index` rows by `{tenant_id, resource_type, resource_id}`
issues the same `delete_many` on `search_index_contained` (same session when
one is active):

- `delete_search_index` (before every re-index, and on delete).
- `delete_search_index_in_bundle_transaction`.
- The admin resource purge (the path in `storage.rs` that deletes a resource
  from `resources`, `resource_history` and `search_index` by id).
- `bulk_ingest::write_search_index` stale-row clearing (`resource_id: {$in: ids}`).
- Tenant purge: `search_index_contained` joins the list of collections deleted
  by `tenant_id`.

`search_offloaded` deployments skip both collections exactly as they skip
`search_index` today.

## Read path: the `_contained` pipeline

`contained_page` (`search_impl.rs`) aggregates on `search_index_contained`
instead of `search_index`. Its `$match` drops `is_contained: true` and keeps
`{tenant_id, contained_type, $or: <value branches>}`; the `$group` stages,
the `both` mode, `_total`, paging, dedupe and the container fetch on
`resources` are unchanged.

Standard search, `:missing`, `_sort` by parameter, `_include`/`_revinclude`,
`_has` and `search_count` are untouched; they read `search_index`, which no
longer holds contained rows.

## Migration and index generation

`SEARCH_INDEX_GENERATION` becomes 3. Generation 3 is generation 2 with
`idx_search_contained` removed from the `search_index` catalog:
`generation2_specs()` is renamed `current_specs()`, the contained spec leaves
it, and a `superseded_contained_spec()` (exact keys and partial filter, so the
rollback script can recreate it) joins the superseded set the builder drops.
The two new inline indexes live in a separate `contained_specs()` list that
`ensure_search_indexes` creates on the new collection at boot.

A new boot step, run by `SearchIndexBuilder` before its index work and in every
`HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build):

1. If `schema_version.search_indexes.contained_rows_moved` is already `true`,
   skip.
2. Otherwise page through `search_index` rows with `is_contained: true` (served
   by the old partial index while it still exists, a full-collection scan of a
   small set otherwise), and for each page `insert_many` the rows into
   `search_index_contained` with `is_contained` removed, then `delete_many`
   those `_id`s from `search_index`. Page size 1,000; idempotent per page
   (`insert_many` with `ordered: false`, duplicate-key errors on `_id` ignored)
   so a crash mid-way resumes cleanly on the next boot.
3. Record `contained_rows_moved: true` on the `schema_version` document.

Then the existing generation logic drops `idx_search_contained` from
`search_index` and records `generation: 3`. In `off` mode the builder warns
about the superseded index as it does today for generation-1 names; the
operator drops it by hand. The corpus and most deployments have no contained
rows, so step 2 is a single empty find.

The pre-build script `docs/mongodb/search-index-v2.mongosh.js` is generated
from the catalog and pinned by a unit test; it regenerates without
`idx_search_contained`, and a new `docs/mongodb/search-index-contained.mongosh.js`
(also pinned) creates the two inline indexes for operators who bootstrap by
script. `docs/mongodb/search-indexes.md` gains a "Contained rows" section:
what moved, that the move runs at boot in every mode, and the downgrade note.

### Downgrade

A pre-change binary reads contained matches from `search_index` only, so
`_contained` searches return nothing for rows that were moved; standard searches
are unaffected (and regain the bug). Recovery on the old binary: `$reindex`,
which rewrites every row from `resources`, or the rollback script
`docs/mongodb/search-index-contained-rollback.mongosh.js` (copies rows back with
`is_contained: true` and recreates the partial index; pinned by a unit test
like the others).

## Error handling

- Insert or delete failures on the new collection surface as the same
  `internal_error("Failed to ... search index entries")` as today; a create or
  update fails as a whole, and inside a bundle transaction the session aborts.
- The migration step logs progress every page at `info` and fails the boot the
  way the schema bootstrap fails today if MongoDB rejects a write; a partial
  move is safe to retry.
- The builder's refusal rule (a catalog name present with a different spec)
  extends to the new collection's two indexes.

## Tests

Unit (no MongoDB):
- `search_index_documents` splits own and contained rows: a resource with a
  same-type contained entity yields the container's rows in `own` and the
  contained rows, without `is_contained`, in `contained`.
- Catalog: generation-3 specs contain no `idx_search_contained` on
  `search_index`; the contained collection's two specs have the keys above;
  the three mongosh scripts on disk match the catalog.
- Builder: the superseded list contains `idx_search_contained`; `off` mode
  warns and drops nothing.

Integration (`mongodb_tests.rs`, testcontainers):
- **The bug.** An Observation `holder` whose only `code = X` is inside a
  contained Observation: `Observation?code=X` returns nothing;
  `Observation?code=X&_contained=true` returns `holder`; `_contained=both`
  returns `holder` once. The existing `_contained=both` dedupe test's fixture
  comment (which documents the leak) is rewritten so the test exercises a real
  dedupe: a container that matches on its own value and through a contained
  entity.
- Cross-type containment still works: an Observation containing a Patient,
  `Patient?name=...&_contained=true` returns the Observation as container;
  `Patient?name=...` returns nothing.
- Update and delete of a container leave no rows in either collection for its
  id; transaction-bundle create/delete likewise; bulk-submit ingest of a
  resource with contained entities writes to both collections and a re-ingest
  clears the old rows.
- Tenant purge and resource purge empty `search_index_contained` for the tenant
  / resource.
- Migration: seed `search_index` with contained-shaped rows the old way
  (`is_contained: true`), boot the backend, assert the rows are in
  `search_index_contained` without `is_contained`, gone from `search_index`,
  `contained_rows_moved: true` and `generation: 3` recorded, `idx_search_contained`
  absent from `search_index` and present on the new collection; a second boot
  issues no `createIndexes` and moves nothing.
- Covered scans: the three existing `*_is_a_covered_v2_scan` tests keep
  passing unchanged (the standard search's plan is not touched).
- `$reindex` of a tenant with contained resources rebuilds both collections.

## Out of scope

- Cross-type containment semantics (already correct).
- The `_contained` pipeline's own performance (#1059 covered it).
- Composite search on MongoDB (#1206).
