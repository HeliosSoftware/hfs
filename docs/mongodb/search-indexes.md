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

## How `$reindex` walks a type (#1403)

`$reindex` and the rebuild that follows a fast-load import page each resource type in two phases:

1. **Id order.** Every live resource whose `last_updated` is older than the walk's *floor*, in `id` order on `idx_resources_identity`. Value-index keys end in the resource id, so this order keeps each index's inserts clustered instead of random. That is the difference between a cache-resident rebuild and the 17-hour Observation rebuild of #1403.
2. **Catch-up rounds.** Every live resource stamped at or after the floor, in `last_updated` order on `idx_resources_type_scan`. A round stops at a ceiling of *round start + margin*, or just past the newest live `last_updated` if a resource is stamped later than that. A round ends only when a query finds nothing more in its range. Another round runs, three at most, when the previous one took longer than half the margin.

The floor is the earlier of two times: the type's newest live `last_updated` at the start plus 1 ms, and the start time minus the margin (two minutes). On a type nobody writes to while it is walked, every resource is written exactly once.

A resource may be updated while the page holding its old version is being written. It is then read again after the update and indexed from its newest version. In the id-order phase that guarantee is exact, given two assumptions:

- the clocks of the HFS processes writing to one database agree within one minute;
- a write commits within one minute of its `last_updated`. MongoDB aborts a transaction after `transactionLifetimeLimitSeconds`, 60 s by default. A bulk-ingest batch is stamped when it is planned and can take longer under memory pressure, so do not run `$reindex` over a live non-deferred import of the same type. The walk reads from the primary.

Inside a catch-up round, the guarantee holds as long as the update is stamped later than the documents already on the page.

Outside those limits, a resource can keep stale search rows until it is next written or reindexed. The same holds when a resource is written again while the last round runs; the log then says `mongodb reindex catch-up stopped at its round limit`. That warning is expected and harmless when an import of the same type overlaps the rebuild: the follow-up generation (logged as `merged deferred reindex work into the pending generation`) walks the type again. A resource deleted while its page is being written can keep search rows. Searches never return it, because they only read live resources.

`mongodb reindex found live resources stamped in the future` means that some live resources carry a `last_updated` later than the walk's start plus the margin, usually from an HFS node whose clock ran ahead. They are still indexed.

The walk's position lives in memory: after a restart, a rebuild starts every type from the beginning.

## Composite parameters

Composite search (`code-value-quantity`, `component-code-value-quantity`, ...) needs no index of its own (#1206). The extractor writes one `search_index` row per component value, all sharing `param_name` = the composite's own code and a `composite_group` (the base-instance index) — the same layout the generation-2 value indexes already cover per component type, so a component predicate is index-bounded exactly like a plain parameter of that type.

The driver arm for a composite value is its most selective component's filter (lowest probe count); a resource that matches the driver arm has only proven *one* component, so every batch of candidates is re-checked by fetching `(resource_id, composite_group)` pairs per component, bounded to that batch, and intersecting them — a resource matches only if every component is satisfied by a row in the *same* `composite_group`. This mirrors SQLite's `GROUP BY resource_id, composite_group HAVING ...`.

Because no slot is stored per component, a composite whose two components share the same type is ambiguous in the rows themselves: 24 of the 46 R4 composites pair two Token components (e.g. `code-value-concept` and `component-code-value-concept`), so a query value `A$B` also matches a resource whose code is `B` and value is `A` — the rows for both components look alike and only differ by which row's `composite_group` they land in, not by which component they came from. SQLite shares this ambiguity (same non-slotted layout); Postgres does not, because it stores a per-component slot.

An arity mismatch — a value with fewer or more `$`-separated parts than the parameter declares (e.g. `code-value-quantity=8302-2`, one part for a two-component parameter) — is a 400 (`InvalidComposite`) on MongoDB. SQLite and Postgres instead return an empty page for the same query.
