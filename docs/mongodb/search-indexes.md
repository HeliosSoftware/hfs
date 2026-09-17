# MongoDB `search_index` indexes

HFS keeps two kinds of index on the `search_index` collection.

- **Inline** indexes (`idx_search_composite`, `idx_search_resource`) are created at every boot before the server serves. They are cheap to build.
- **Generation 3** is the current generation: nine partial value indexes named `idx_search_*_v2`, built by HFS **after** boot in one `createIndexes` command that scans the collection once, plus contained rows living in their own collection since generation 3 (see "Contained rows" below) rather than a partial index on `search_index`. MongoDB 4.2 and later do not block reads or writes during the value-index build. When every generation-2 value index is ready, HFS drops the nine generation-1 value indexes (`idx_search_string`, `idx_search_token`, ...); once the contained-row move is also done, HFS records `search_indexes.generation: 3` in the `schema_version` document.

Why: a generation-1 value index carried one entry for every row of the collection, even rows that had no value of that type, and no value index carried `resource_id`, so every search fetched one document per matching key. Generation-2 indexes are partial (one entry per row that has the value) and end in `resource_id`, so a value-filtered scan is covered. Issues #1059 and #1084 have the measurements.

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

Rows extracted from a resource's `contained` entries live in their own collection, `search_index_contained` (#1160). A standard search reads only `search_index`, so it can no longer match a container through a same-type contained resource; `_contained=true|both` searches read `search_index_contained`. Its two indexes are created inline at every boot, so running the script below is optional; it exists for operators who prefer to bootstrap a fresh database by script before first boot:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained.mongosh.js

On the first boot of a generation-3 binary HFS moves any contained rows still in `search_index` into the new collection, in pages of 1,000, in every `HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build), records `search_indexes.contained_rows_moved: true`, and then drops the old partial `idx_search_contained` from `search_index` under the usual mode rules (in `off` mode it warns and names the `dropIndex` command). Most deployments have no contained rows, so the move is one empty find. On a deployment that has contained resources and takes writes during boot, run that first generation-3 boot with `HFS_MONGODB_INDEX_BUILD=inline`, or run `$reindex` afterwards: a container updated while the background move is paging can get its pre-update contained rows re-inserted.

## Downgrading

A binary from before generation 2 creates the nine generation-1 indexes at boot, inline, and will not serve until they exist. If they have been dropped, that boot rebuilds all nine before serving. Before rolling back, recreate them in the background:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-v1-rollback.mongosh.js

A binary from before generation 3 reads contained matches from `search_index` only, so `_contained` searches return nothing for rows that were moved (standard searches are unaffected). Before rolling back, copy the rows back and recreate the partial index:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained-rollback.mongosh.js

That script also resets the `schema_version` generation record, so rolling forward again re-runs the contained-row move instead of skipping it.

All four scripts are generated from `crates/persistence/src/backends/mongodb/search_index_catalog.rs`; a unit test fails if they drift.
