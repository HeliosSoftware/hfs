# MongoDB `search_index` indexes

HFS keeps two kinds of index on the `search_index` collection.

- **Inline** indexes (`idx_search_composite`, `idx_search_resource`, `idx_search_composite_slot_probe`) are created at every boot before the server serves. The slot probe index is partial and includes only composite rows.
- **Generation 4** is the current generation: nine partial value indexes (`idx_search_date_v3` and eight named `idx_search_*_v2`), built by HFS **after** boot in one `createIndexes` command that scans the collection once, plus contained rows living in their own collection since generation 3 (see "Contained rows" below) rather than a partial index on `search_index`. MongoDB 4.2 and later do not block reads or writes during the value-index build. When every generation-2 value index is ready, HFS drops the nine generation-1 value indexes (`idx_search_string`, `idx_search_token`, ...); once the contained-row move is also done, HFS records `search_indexes.generation: 4` in the `schema_version` document.

Why: a generation-1 value index carried one entry for every row of the collection, even rows that had no value of that type, and no value index carried `resource_id`, so every search fetched one document per matching key. Generation-2 indexes are partial (one entry per row that has the value) and end in `resource_id`, so a value-filtered scan is covered. Issues #1059 and #1084 have the measurements.

## Date ranges (generation 4)

Since #1391 a date row stores the range it denotes, `[value_date, value_date_end)`, and a date search bounds either end (`gt` and `ge` bound `value_date_end`, for example). `idx_search_date_v3` carries `value_date_end` between `value_date` and `resource_id` so those searches stay covered; it is still partial on `value_date` existing. It replaces generation 2's `idx_search_date_v2`, which HFS drops once every generation-4 index is ready (under the same mode rules as the generation-1 drop). Until then date searches keep using `idx_search_date_v2`, fetching documents to check `value_date_end`. Every prefix is a covered scan of `idx_search_date_v3` (`docsExamined: 0`). The prefixes with two alternatives (`ge`, `le`, `ne`), and a comma list of date values, are sent as an `$or` at the top of the filter with every arm repeating the tenant, resource type and parameter name; nested under those shared conditions MongoDB 5.0 reads the documents instead of scanning the index.

### Upgrading: reindex the date rows

Rows written before #1391 have no `value_date_end`. They never match the prefixes that bound the end (`eq`, `ne`, `gt`, `ge`, `le`, `eb`, `ap`) until they are rewritten. A Period indexed before #1391 is also still two independent point rows, so even `lt` and `sa` compare each of its ends on its own until it is rewritten (point values are unaffected by those two). Run `$reindex` after the upgrade. HFS reminds you: when the builder records generation 4 on a database that was at an earlier generation (or had none recorded) and `search_index` is not empty, it logs one `warn` naming `$reindex`. The check reads the recorded generation and the collection's metadata count, never the rows, and only runs on that transition: it is silent on a new empty database and on every boot once generation 4 is recorded.

## `HFS_MONGODB_INDEX_BUILD`

| value | behaviour |
|---|---|
| `background` (default) | Boot returns immediately. The build runs in the background and is logged at `info` when it starts and finishes. |
| `inline` | Boot waits for the build. Use for tests, developer databases and small deployments. |
| `off` | Nothing is built or dropped. Each missing generation-2 index is logged at `warn`. Each generation-1 index still present once generation 2 is complete is also logged at `warn`, naming the `dropIndex` command to remove it. Use when you pre-build in a maintenance window. |

## Upgrading a large deployment

Deploy the binary. The server serves on generation 1 while the build runs. Disk peaks at the size of both generations, then drops when the old ones are removed. Writes are slower during the build by the cost of maintaining both sets. Nothing needs to be scheduled.

To build in a window of your choosing instead, run the pre-build script first; a database that already has every generation-2 index makes the builder a no-op at boot:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-v2.mongosh.js

## If the build fails

The log carries the server's error at `error` level. Nothing has been dropped; searches keep using generation 1. Fix the cause (usually disk) and restart: every step is idempotent, and a completed index is skipped.

If a generation-2 name exists with a different key spec, HFS refuses to build or drop anything and logs both specs. Drop or rename that index by hand. A spec that differs only in the numeric type of its key values (for example one built by mongosh, which stores doubles where HFS stores integers) is not treated as a conflict.

## Contained rows (generation 3)

Rows extracted from a resource's `contained` entries live in their own collection, `search_index_contained` (#1160). A standard search reads only `search_index`, so it can no longer match a container through a same-type contained resource; `_contained=true|both` searches read `search_index_contained`. Its three indexes are created inline at every boot, so running the script below is optional; it exists for operators who prefer to bootstrap a fresh database by script before first boot:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained.mongosh.js

On the first boot of a generation-3 binary HFS moves any contained rows still in `search_index` into the new collection, in pages of 1,000, in every `HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build), records `search_indexes.contained_rows_moved: true`, and then drops the old partial `idx_search_contained` from `search_index` under the usual mode rules (in `off` mode it warns and names the `dropIndex` command). Most deployments have no contained rows, so the move is one empty find. On a deployment that has contained resources and takes writes during boot, run that first generation-3 boot with `HFS_MONGODB_INDEX_BUILD=inline`, or run `$reindex` afterwards: a container updated while the background move is paging can get its pre-update contained rows re-inserted.

## Downgrading

A binary from before generation 2 creates the nine generation-1 indexes at boot, inline, and will not serve until they exist. If they have been dropped, that boot rebuilds all nine before serving. Before rolling back, recreate them in the background:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-v1-rollback.mongosh.js

A binary from before generation 3 reads contained matches from `search_index` only, so `_contained` searches return nothing for rows that were moved (standard searches are unaffected). Before rolling back, copy the rows back and recreate the partial index:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained-rollback.mongosh.js

That script also resets the `schema_version` generation record, so rolling forward again re-runs the contained-row move instead of skipping it.

A binary from before generation 4 needs no script: it sees `idx_search_date_v2` missing and rebuilds it in the background like any other missing generation-2 index, and ignores `idx_search_date_v3`.

All four scripts are generated from `crates/persistence/src/backends/mongodb/search_index_catalog.rs`; a unit test fails if they drift.

## Composite parameters

Composite search (`code-value-quantity`, `component-code-value-quantity`, ...) uses the existing value indexes for matching (#1206). The extractor writes one row per component value in `search_index`, or in `search_index_contained` for a contained resource. Rows in one composite instance share `param_name` (the composite's code) and `composite_group` (the base-instance index). Each row also has a `composite_slot`, the component's position among components of the same type. The existing value indexes bound each component predicate by its type and value.

For standard searches, the driver arm is the component filter with the lowest probe count. Each candidate batch is then checked against the other components within the same `composite_group`. Contained searches group matching rows by contained entity and `composite_group`. When components share a type, both paths require the matching `composite_slot`, so `A$B` does not match rows for `B$A`. SQLite also groups components by `composite_group`, but its rows have no slot, so same-type components remain ambiguous there. PostgreSQL stores a per-component slot.

Older MongoDB composite rows lack `composite_slot`. A same-type composite query that encounters a matching old row fails with an error asking for `$reindex` rather than silently omitting a match. The preflight probe uses `idx_search_composite_slot_probe` on `search_index` or `idx_search_contained_composite_slot_probe` on `search_index_contained`. Both indexes lead with tenant, searched resource type and parameter name, then `composite_slot`. They include only rows where `composite_group` exists, but retain rows with a missing slot. HFS builds them before serving, including when `HFS_MONGODB_INDEX_BUILD=off`; on a large database, build them ahead of deployment to avoid a longer startup. The contained pre-build script above includes its probe index. To pre-build the standard probe index, run this in `mongosh` against the HFS database:

```javascript
db.search_index.createIndex(
  { tenant_id: 1, resource_type: 1, param_name: 1, composite_slot: 1 },
  {
    name: "idx_search_composite_slot_probe",
    partialFilterExpression: { composite_group: { $exists: true } }
  }
);
```

After every writer runs the slot-writing version, send `POST /$reindex` for the affected tenant. With no body, `clearExisting` defaults to `false`; the job rewrites the resources' index rows without clearing the entire tenant index first. The response has a `jobId`. Poll `GET /$reindex-status/{job_id}` on the same server node until its `status` parameter is `completed` and `errorCount` is zero. The operation requires the `system/reindex` scope when authentication is enabled.

Use the tenant-wide route even when the repeated-type parameter belongs to `Observation`. A contained Observation is indexed under its container's resource type, so `POST /Observation/$reindex` does not rebuild its rows when the container is another type. Keep older writers from writing to that tenant during and after the rebuild; they would create rows without slots again.

An arity mismatch — a value with fewer or more `$`-separated parts than the parameter declares (e.g. `code-value-quantity=8302-2`, one part for a two-component parameter) — is a 400 (`InvalidComposite`) on MongoDB. SQLite and Postgres instead return an empty page for the same query.
