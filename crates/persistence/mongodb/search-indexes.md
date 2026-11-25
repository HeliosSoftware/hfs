# MongoDB `search_index` indexes

HFS keeps two kinds of index on the `search_index` collection.

- **Inline** indexes (`idx_search_composite`, `idx_search_resource`, `idx_search_composite_slot_probe`) are created at every boot before the server serves. The slot probe index is partial and includes only composite rows.
- **Generation 8** is the current generation. It is generation 7 plus two partial indexes: `idx_search_quantity_high` on `(tenant_id, resource_type, param_name, value_quantity_high, value_quantity_value, resource_id)`, partial on `value_quantity_value` (so a row not yet backfilled is a null-high key rather than absent), and `idx_search_quantity_range` on `(tenant_id, resource_type, param_name, value_quantity_range_high, value_quantity_value, resource_id)`, partial on `value_quantity_range_high` existing. They serve quantity searches bounded on the high end of a stored interval, the Range arm of `ap`, and a descending quantity sort; see [Quantity intervals (generation 8)](#quantity-intervals-generation-8). It is built in the same background `createIndexes` as the others.
- **Generation 7** (the base of generation 8) is generation 6 plus one partial index, `idx_search_date_end` on `(tenant_id, resource_type, param_name, value_date_end, value_date, resource_id)`, partial on `value_date` like `idx_search_date_v3`. It orders date rows by the end of their range. That serves the descending date sort, see [Sorting past 300,000 matches](#sorting-past-300000-matches-generation-7), and the date filters bounded on the end of the range (`gt`, and one arm of `ge`/`ne`), see [Date ranges](#date-ranges-generation-4). It is built in the same background `createIndexes` as the others.
- **Generation 6** (the base of generation 7) is generation 5 plus one partial index, `idx_search_composite_tuple_qty`, over the `tuple_code` of token+quantity composite rows; see [Token+quantity composites (generation 6)](#tokenquantity-composites-generation-6). It is built in the same background `createIndexes` as the others.
- **Generation 5** (the base of generation 6) is generation 4 plus one partial index, `idx_search_reference_target_v2` on `(tenant_id, resource_type, param_name, value_reference_target, resource_id)`, which holds only rows that carry `value_reference_target` (absolute `http(s)` references, #2394). It is built in the same background `createIndexes` as the others; see "Reference normalization (generation 5)" below.
- **Generation 4** (the base of generation 5) is nine partial value indexes (`idx_search_date_v3` and eight named `idx_search_*_v2`), built by HFS **after** boot in one `createIndexes` command that scans the collection once, plus contained rows living in their own collection since generation 3 (see "Contained rows" below) rather than a partial index on `search_index`. MongoDB 4.2 and later do not block reads or writes during the value-index build. When every generation-2 value index is ready, HFS drops the nine generation-1 value indexes (`idx_search_string`, `idx_search_token`, ...); once the contained-row move is also done, HFS records `search_indexes.generation: 4` in the `schema_version` document.

Why: a generation-1 value index carried one entry for every row of the collection, even rows that had no value of that type, and no value index carried `resource_id`, so every search fetched one document per matching key. Generation-2 indexes are partial (one entry per row that has the value) and end in `resource_id`, so a value-filtered scan is covered. Issues #1059 and #1084 have the measurements.

## Date ranges (generation 4)

Since #1391 a date row stores the range it denotes, `[value_date, value_date_end)`, and a date search bounds either end (`gt` and `ge` bound `value_date_end`, for example). `idx_search_date_v3` carries `value_date_end` between `value_date` and `resource_id` so those searches stay covered; it is still partial on `value_date` existing. It replaces generation 2's `idx_search_date_v2`, which HFS drops once every generation-4 index is ready (under the same mode rules as the generation-1 drop). Until then date searches keep using `idx_search_date_v2`, fetching documents to check `value_date_end`. Every prefix is a covered scan (`docsExamined: 0`). Since #2601, once `idx_search_date_end` is built, a single-arm filter bounded only on the range end (`gt`) is hinted to `idx_search_date_end`, where `value_date_end > x` is a seek on its leading key instead of a scan of every key of the parameter in `idx_search_date_v3`. The prefixes that mix an end-bounded arm with a start-bounded one (`ge` and `ne`) are sent as a root `$or` with no hint, and so is a comma list of values that contains `gt`, `ge` or `ne`; MongoDB then plans each arm and chooses its index (typically `idx_search_date_end` for the end-bounded arm and `idx_search_date_v3` for the start-bounded one, but either may serve any arm). When the search is paged through the batched seed path, the cursor boundary is repeated inside each arm of that unhinted `$or` so it stays at the root; other paths wrap the filter with the boundary in an `$and`. A wrapped `gt` keeps the `idx_search_date_end` hint (the `resource_id` bound does not change its range on `value_date_end`); a wrapped `$or` (`ge`, `ne`, comma lists) stays unhinted. Under `idx_search_date_v3` the wrapped forms read about every key of the parameter (997k keys on 1M second-resolution rows, against 59k). `eq`, `lt`, `le`, `sa`, `eb` and `ap`, and a comma list without `gt`, `ge` or `ne`, keep the `idx_search_date_v3` hint. While `idx_search_date_end` is missing or still building, every query keeps the previous hint, and a hint the server rejects (index dropped while running) is retried without a hint. The prefixes with two alternatives (`ge`, `le`, `ne`), and a comma list of date values, are sent as an `$or` at the top of the filter with every arm repeating the tenant, resource type and parameter name; nested under those shared conditions MongoDB 5.0 reads the documents instead of scanning the index.

### Upgrading: reindex the date rows

Rows written before #1391 have no `value_date_end`. They never match the prefixes that bound the end (`eq`, `ne`, `gt`, `ge`, `le`, `eb`, `ap`) until they are rewritten. A Period indexed before #1391 is also still two independent point rows, so even `lt` and `sa` compare each of its ends on its own until it is rewritten (point values are unaffected by those two). Run `$reindex` after the upgrade. HFS reminds you: when the builder records generation 4 on a database that was at an earlier generation (or had none recorded) and `search_index` already held rows when the server started, it logs one `warn` naming `$reindex`. HFS reads the recorded generation and the collection's metadata count at startup, before it accepts writes, never the rows, so rows written while the first build runs in `background` mode do not count. On a database with no generation recorded and an empty `search_index`, it also records `search_indexes.empty_at_generation`, so a restart, or an `off`-mode database built later, stays silent before the first build records generation 4; recording the generation removes that field. If the startup check fails, HFS logs a different `warn` saying it could not tell, naming `$reindex` for a database upgraded from before #1391. It is silent on a new database and on every boot once generation 4 is recorded, including a later upgrade from generation 4 to a newer generation. The startup check counts only `search_index`. Contained-resource date rows written between #1160 and #1391 (generation 3) also lack `value_date_end` but live in `search_index_contained`, which the check does not count, so a database whose only such rows are contained ones gets no reminder. `$reindex` rewrites both collections.

## Quantity intervals (generation 8)

Since #2602 a quantity row is the closed interval `[value_quantity_value, value_quantity_high]`. A `Quantity` or `Age` is a point (`value_quantity_high` equals `value_quantity_value`); a `Range` (for example `Condition.onsetRange` for `onset-age`, or a `useContext` `valueRange` for `context-quantity`) stores its `low` in `value_quantity_value` and its `high` in `value_quantity_high`. A missing bound is open: the finite sentinel `-1.7976931348623157e308` (`f64::MIN`) for a missing `low` and `1.7976931348623157e308` (`f64::MAX`) for a missing `high`. Quantity prefixes compare the whole interval, like the date prefixes compare a stored range: `eq` when the interval lies inside the search value's implicit-precision range, `ne` when it does not, `gt`/`ge`/`eb` on the high end, `lt`/`le`/`sa` on the low end, and `ap` when it overlaps the approximation window. Searches for a point row give the results they gave before.

`idx_search_quantity_high` leads with `value_quantity_high`, so a search bounded on the high end and a descending sort (which orders by the interval end) are index scans. A descending quantity sort over more matches than the id-set cap walks that index only once the whole current build has finished, like the descending date sort.

`value_quantity_range_high` is a sparse field: the writer sets it, to the same value as `value_quantity_high`, only on a row that is a proper `Range` (high different from low), never on a point. `ap` is two groups, sent as an `$or`: a point or Range whose low lies in the approximation window (`value_quantity_value` between the window ends, served by `idx_search_quantity_v2`), or a proper Range that starts below the window and reaches it (`value_quantity_value < lo` and `value_quantity_range_high >= lo`, served by `idx_search_quantity_range`, which holds no points). A partial index cannot compare two fields, which is why the field exists; the `$gte` on it also implies the row is a proper Range. No hint is used. Composite token+quantity rows are points only, so the tuple path keeps just the first group.

### Upgrading: backfill and reindex

At every start, in every `HFS_MONGODB_INDEX_BUILD` mode (`off` included), HFS looks for a quantity row without `value_quantity_high` with one `find_one` on `search_index` and one on `search_index_contained`. A clean database pays only those two probes. If a row is found, HFS sets `value_quantity_high = value_quantity_value` on every such row, in batches of 10,000 rows ordered by `_id`, with progress logged after each batch, and logs a `warn!` with the number of rows healed: a pre-#2602 instance is probably still writing to the database. Otherwise it logs a `debug!` with 0 (nothing at the default level). A failed batch is retried, then the error is returned and the next boot tries again. Until the pass finishes, quantity searches that bound the high end miss the rows it has not reached. The schema version is 18, so a pre-#2602 binary (schema version 17) refuses to start on the database. An old instance that was already running and wrote after the last new instance booted leaves rows without a high until the next boot of a new instance.

A `Range` indexed before #2602 produced no quantity row, so it is not searchable until it is rewritten (and, having no row, it has no `value_quantity_range_high` either; the Range index needs no backfill). Run `$reindex` after the upgrade. When the builder records generation 8 on a database that was at an earlier recorded generation, it logs one `warn` naming `$reindex`; it is silent on a new database and on every boot once generation 8 is recorded. `Quantity` and `Age` rows need nothing.

## `HFS_MONGODB_INDEX_BUILD`

| value | behaviour |
|---|---|
| `background` (default) | Boot returns immediately. The build runs in the background and is logged at `info` when it starts and finishes. |
| `inline` | Boot waits for the build. Use for tests, developer databases and small deployments. |
| `off` | No indexes are built or dropped. Each missing or still-building current-generation index is logged at `warn`; either condition reports `Skipped` and records no generation. Superseded indexes are also logged at `warn`, naming the `dropIndex` command to remove them. Use when you pre-build in a maintenance window. Readiness is checked at startup only: restart HFS after the pre-build finishes to enable the newly available indexes. |

## Upgrading a large deployment

Deploy the binary. The server serves on generation 1 while the build runs. Disk peaks at the size of both generations, then drops when the old ones are removed. Writes are slower during the build by the cost of maintaining both sets. Nothing needs to be scheduled.

To build in a window of your choosing instead, run the pre-build script first; a database that already has every generation-2 index makes the builder a no-op at boot:

    export HFS_MONGODB_DATABASE="${HFS_MONGODB_DATABASE:-helios}"
    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/search-index-v2.mongosh.js

Every script on this page runs this way. mongosh connects with the URL HFS uses, unchanged, so a URL with a query string such as `mongodb://h:27017/?replicaSet=rs0` works. The script then selects the database named by `HFS_MONGODB_DATABASE`, and stops before it changes anything if that variable is unset. HFS uses `helios` when no database name is set, hence the default above.

## If the build fails

The log carries the server's error at `error` level. Nothing has been dropped; searches keep using generation 1. Fix the cause (usually disk) and restart: every step is idempotent, and a completed index is skipped.

If a generation-2 name exists with a different key spec, HFS refuses to build or drop anything and logs both specs. Drop or rename that index by hand. A spec that differs only in the numeric type of its key values (for example one built by mongosh, which stores doubles where HFS stores integers) is not treated as a conflict.

## Contained rows (generation 3)

Rows extracted from a resource's `contained` entries live in their own collection, `search_index_contained` (#1160). A standard search reads only `search_index`, so it can no longer match a container through a same-type contained resource; `_contained=true|both` searches read `search_index_contained`. Its three indexes are created inline at every boot, so running the script below is optional; it exists for operators who prefer to bootstrap a fresh database by script before first boot:

    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/search-index-contained.mongosh.js

On the first boot of a generation-3 binary HFS moves any contained rows still in `search_index` into the new collection, in pages of 1,000, in every `HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build), records `search_indexes.contained_rows_moved: true`, and then drops the old partial `idx_search_contained` from `search_index` under the usual mode rules (in `off` mode it warns and names the `dropIndex` command). Most deployments have no contained rows, so the move is one empty find. On a deployment that has contained resources and takes writes during boot, run that first generation-3 boot with `HFS_MONGODB_INDEX_BUILD=inline`, or run `$reindex` afterwards: a container updated while the background move is paging can get its pre-update contained rows re-inserted.

## Reference normalization (generation 5)

Since #2394 (schema version 17) `search_index.value_reference` is written without a `/_history/N` suffix, and an absolute `http(s)://<base>/Type/id[/_history/N]` reference also gets `value_reference_target: "Type/id"`. Relative references add no field, so the new partial index grows only with the number of absolute references. `_revinclude` matches with exact `$in` on `value_reference` and `value_reference_target`, and bare-id reference filters (`subject=p1`) use the same two fields, with no regex. A plain `Type/id` filter (`subject=Patient/p1`) stays an exact match on `value_reference` and does not reach absolute URLs; the other backends do match them.

The `SearchIndexBuilder` rewrites existing rows of `search_index` and `search_index_contained` at start, after the contained-row move and in every `HFS_MONGODB_INDEX_BUILD` mode. It walks `_id` in pages, resumes from `search_indexes.reference_normalize_last_id`, and sets `search_indexes.references_normalized` at the end. It is idempotent. Readers use the exact form only when the marker is set and `idx_search_reference_target_v2` is built; before that, and with `HFS_MONGODB_INDEX_BUILD=off` (which does not build the index), they keep the regex arms, so results are complete throughout.

To rerun the backfill, use `reference-normalize-rollback.mongosh.js`; see [Crossed 17](schema-version.md#crossed-17-reference-normalization).

## Search offloaded to Elasticsearch (`mongodb-elasticsearch`)

When MongoDB runs with search offloaded to Elasticsearch, every search goes to Elasticsearch and `search_index` and `search_index_contained` stay empty (offloaded writes only `$unset` the `missing_params` fields). Nothing reads the indexes below, so in this mode HFS neither builds them nor requires them:

- the inline `search_index` indexes (`idx_search_composite`, `idx_search_resource`, `idx_search_composite_slot_probe`) and the `search_index_contained` indexes, which are not created at boot;
- the twelve generation-7 value indexes, because `SearchIndexBuilder` does not run (it reports `Skipped`, in every `HFS_MONGODB_INDEX_BUILD` mode);
- `idx_resources_missing_params` and `idx_resources_missing_params_fp`, because `MissingIndexBuilder` does not run, as with `HFS_MONGODB_MISSING_METADATA=false`;
- `idx_resources_live_id`, which only standard search reads (see [Liveness of candidate ids](#liveness-of-candidate-ids-resourcesidx_resources_live_id)), and which boot does not create in this mode.

Readiness requires only the `resources` indexes (`idx_resources_identity`, `idx_resources_type_scan`). A node whose `search_index` indexes are missing or dropped stays ready. The standalone MongoDB backend is unchanged: it still builds and requires all of them, because `:missing` and standard search read them.

HFS does not drop indexes that already exist, so a deployment moved to this mode, or a mongo-es deployment created before this change, keeps them (and the disk they use). They are harmless, and keeping them lets you return to the standalone backend without a rebuild. To reclaim the space, drop them by hand once you are sure you will not run the standalone backend on this database. In `mongosh` against the HFS database:

    db.search_index.dropIndexes()
    db.search_index_contained.dropIndexes()
    db.resources.dropIndex("idx_resources_missing_params")
    db.resources.dropIndex("idx_resources_missing_params_fp")

`dropIndexes()` with no argument drops every index on the collection except `_id`; the two collections are empty in this mode. If you only want the missing-params saving, drop the last two and leave the others. `dropIndex` fails with "index not found" for an index that was never built; that is safe to ignore. If you later switch the database back to the standalone backend, the next boot recreates the inline indexes and rebuilds the rest, and you must run `$reindex` because the search rows were never written.


## Liveness of candidate ids (`resources.idx_resources_live_id`)

A search that has chosen resource ids from `search_index` still asks which of them are live. Those reads filter `resources` on tenant, type, `is_deleted: false` and an `id` list, and read back only `id`:

- the liveness check of a filtered or deep parameter sort (`resource_level_ids`, `live_ids_among`);
- the walk over resources without a sort key, in `id` order (`param_sorted_filtered_window_over`);
- the count, the count-and-page and the live-id find of each candidate batch (`BatchedIndexPlan::match_batch`).

`idx_resources_identity` (`tenant_id, resource_type, id`) has no `is_deleted`, so each of them fetched the whole document to read one boolean: `/Observation?_sort=-date&_offset=290000` fetched 290,046 documents for 20 entries (#2601). Since #2601 boot creates

    { tenant_id: 1, resource_type: 1, is_deleted: 1, id: 1 }   name: idx_resources_live_id

and those reads hint it, so they are covered index scans (`docsExamined: 0`). It is not a `search_index` generation and records no schema version: an older build neither reads nor drops it. With search offloaded to Elasticsearch it is not created, because no search reads it.

**Build.** Boot creates it inline with the other `resources` indexes, before the server serves, so the first boot of an existing store waits for one scan of `resources`. Measured on a `mongo:9.0.8` copy of the benchmark store (2,273,836 `resources` documents, 2.4 KB average, 3.5 GB WiredTiger cache): 4.8 s, and the index takes 91 MB on disk (`idx_resources_identity`: 196 MB). The scan also pushes part of the search working set out of the cache, so the first searches after that boot are slower until it is read back.

**Write cost.** One more index entry per resource insert and per deletion or restore (`is_deleted` is a key); an update that keeps `is_deleted` leaves its key unchanged. Measured on the same `mongo:9.0.8` server, copying 100,000 benchmark resources into a collection with the `resources` indexes HFS creates, without and with the index (order A, B, B, A, batches of 1,000): inserts took 4,909 and 4,917 ms without it and 5,321 and 4,840 ms with it (mean +3%, inside the run-to-run noise), and 10,000 deletion flips by id 33.7 s and 31.2 s without it against 50.3 s and 31.3 s with it (the first run with it was an outlier; the second pair is equal). The index holds about 70 bytes per resource (6-7 MB per 100,000).

**Without the index.** The hint is not required for readiness. If the index is missing (dropped by hand), a hinted read is rejected and runs once more on `idx_resources_identity`, which returns the same ids with a fetch, and logs a warning. The next boot recreates the index. To remove it on purpose (an older build ignores it):

    db.resources.dropIndex("idx_resources_live_id")

## Downgrading

These steps cover the `search_index` generations. The MongoDB schema version is recorded separately: builds from #2352 on refuse to start on a database whose `schema_version` records a newer version than their own, so a rollback across a schema version also needs the steps in [Rolling back to an older build](schema-version.md#rolling-back-to-an-older-build).

A binary from before generation 2 creates the nine generation-1 indexes at boot, inline, and will not serve until they exist. If they have been dropped, that boot rebuilds all nine before serving. Before rolling back, with `HFS_MONGODB_DATABASE` exported as under [Upgrading a large deployment](#upgrading-a-large-deployment), recreate them in the background:

    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/search-index-v1-rollback.mongosh.js

A binary from before generation 3 reads contained matches from `search_index` only, so `_contained` searches return nothing for rows that were moved (standard searches are unaffected). Before rolling back, with `HFS_MONGODB_DATABASE` still exported, copy the rows back and recreate the partial index:

    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/search-index-contained-rollback.mongosh.js

That script also resets the `schema_version` generation record, so rolling forward again re-runs the contained-row move instead of skipping it.

A binary from before generation 5 needs no script: it ignores `idx_search_reference_target_v2` and `value_reference_target`, and still matches exact `Type/id` references on normalized rows. For rows it writes afterwards, see [Crossed 17](schema-version.md#crossed-17-reference-normalization).

A binary from before generation 4 needs no script: it sees `idx_search_date_v2` missing and rebuilds it in the background like any other missing generation-2 index, and ignores `idx_search_date_v3`.

A binary from before generation 6 runs without a script: it ignores `idx_search_composite_tuple_qty` and `tuple_code`, and records its own generation, 5 (or 4 before #2394), at boot, after which no instance of the newer build trusts the token+quantity markers. It writes composite rows without `tuple_code`, so the tuple path stops for every tenant; see [Rollback and restore](#rollback-and-restore). To leave nothing of generation 6 behind (the index still costs space, and the markers stay until the newer build boots again), run, with `HFS_MONGODB_DATABASE` exported:

    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/composite-tuple-rollback.mongosh.js

It deletes the `composite_tuple_code_v2` markers first, then drops `idx_search_composite_tuple_qty`, then lowers a recorded generation 6 to 5. It leaves `tuple_code` on the rows: an older build ignores it, and the newer build rebuilds the index and takes the tuple path again only after the activating `$reindex`.

A binary from before generation 7 needs no script: it ignores `idx_search_date_end` and records its own generation at boot. To remove the index, in `mongosh` against the HFS database:

    db.search_index.dropIndex("idx_search_date_end")
    db.schema_version.updateOne({ _id: "schema_version", "search_indexes.generation": 7 }, { $set: { "search_indexes.generation": NumberInt(6) } })

A binary from before generation 8 needs no script: it ignores `idx_search_quantity_high`, `idx_search_quantity_range`, `value_quantity_high` and `value_quantity_range_high`, compares only `value_quantity_value` (the low end of a `Range`), and records its own generation at boot. To remove the index and lower the recorded generation, with `HFS_MONGODB_DATABASE` exported:

    mongosh "$HFS_MONGODB_URL" --file crates/persistence/mongodb/quantity-high-rollback.mongosh.js

Going back to generation 5, run these before `composite-tuple-rollback.mongosh.js`, which lowers only a recorded generation 6.

The first four scripts are generated from `crates/persistence/src/backends/mongodb/search_index_catalog.rs`; a unit test fails if they drift. `composite-tuple-rollback.mongosh.js` is hand-written; a unit test checks that the index and marker names it uses match the code.

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

## Missing-parameter metadata

MongoDB stores `missing_params` on each `resources` document, outside the
FHIR payload. It is a sorted, deduplicated array of supported search-parameter
codes for which extraction found no indexed value. For example,
`missing_params: ["method"]` records that this resource has no indexed
`method` value. It does not create a separate `search_index` row for absence.
An empty array is different from an absent field: an absent field means the
resource has no absence metadata and requires the compatibility path.
Parameters that fail extraction are not recorded as confirmed absence.
Other parameters can still produce rows and metadata, so a present field does
not prove that every parameter extracted successfully. Affected missing-field
results can remain incomplete until the extraction problem is corrected and
the resource is rewritten or reindexed. This metadata describes top-level
resources; it does not add contained `:missing` support. Transaction and
batch conditional criteria (`ifNoneExist`, conditional update/delete URLs)
evaluate `:missing=true` with the same semantics inside the transaction's
session (#2003): the metadata predicate with the value-index fallback when
the fingerprint is fresh, the absence of a value row otherwise. As in search
(#2004), the check runs on each batch of candidates the other criteria
produced, so its cost follows the candidates, not the resource type; criteria
with nothing else to drive from (only `_id`, `_lastUpdated` and
`:missing=true`) scan the type in batches and stop at the second match.
`ifNoneExist` sees resources written earlier in the same bundle; URL-borne
criteria are resolved before any write, as the R4 overlap rule requires.

Next to the array, `missing_params_fp` stamps the definition fingerprint it
was computed from, and the `idx_resources_missing_params_fp` index serves the
per-search staleness probe — see "Definition changes" below.

The shared `idx_resources_missing_params` multikey index has this key order:

```javascript
{ tenant_id: 1, resource_type: 1, is_deleted: 1,
  missing_params: 1, last_updated: 1, id: 1 }
```

The probe's index is scalar and partial, holding only documents that carry
the array:

```javascript
{ tenant_id: 1, resource_type: 1, is_deleted: 1, missing_params_fp: 1 }
// partialFilterExpression: { missing_params: { $exists: true } }
```

On an existing deployment the first build of either index scans every
`resources` document (about 10.5 GB at 12M resources), so HFS builds both
after boot under `HFS_MONGODB_INDEX_BUILD`, in their own task beside the
`search_index` builder (#2001):

| value | behaviour |
|---|---|
| `background` (default) | Boot returns and the node serves while the index builds; the build is logged at `info` when it starts and finishes. |
| `inline` | Boot waits for the build. |
| `off` | Nothing is built. A missing (or still-building) index is logged at `warn` with the command that builds it. |

To pre-build it in a maintenance window, run this in `mongosh` against the HFS
database, then restart:

```javascript
db.resources.createIndex(
  { tenant_id: 1, resource_type: 1, is_deleted: 1,
    missing_params: 1, last_updated: 1, id: 1 },
  { name: "idx_resources_missing_params" }
);
db.resources.createIndex(
  { tenant_id: 1, resource_type: 1, is_deleted: 1, missing_params_fp: 1 },
  { name: "idx_resources_missing_params_fp",
    partialFilterExpression: { missing_params: { $exists: true } } }
);
```

Until the builder has seen both indexes built and visible, every
`:missing=true` and `:missing=false` search uses the value-index route that
unsupported parameters use, and never names the index in a hint. Results are
the same on both routes; only the plan differs. HFS checks once per boot: an
index that an operator builds while the node runs (with `off`, or after a
failed build) is used from the next start.

Readiness (`/health/ready`, #1686) follows the same rule. While the index is
absent or building, the node is ready: no query hints it. Once the builder has
reported it ready, readiness requires it, like the other hinted indexes: if it
is later dropped or hidden, the node reports not ready, naming the index, until
it is restored (unhidden, or rebuilt and the node restarted). An index under
this name with a different key or options is logged at `error` and left
alone; searches keep the value-index route.

The array and its index entries add storage and write work; an unchanged
numeric schema version does not mean the stored representation is unchanged.

### Search and write behavior

For supported `parameter:missing=true` searches, HFS uses absence metadata on
resources and checks the existing typed value index for resources whose
metadata field is absent. Old and new resources can therefore coexist without
requiring an immediate reindex. This fallback uses stored index rows, not a
fresh extraction from the FHIR payload. Unsupported metadata parameters retain
their existing search behavior. `:missing=false` uses typed value presence.
Ordinary queries do not acquire a missing-metadata lookup, although writes
still maintain the array and its index.

Selective positive predicates keep candidate-driven execution. When the
planner has chosen a metadata-aware resource plan, search and count retain it
instead of first collecting every matching resource ID. Eligible ID-sorted
pages with positive predicates use bounded candidate checks; timestamp pages
can use the metadata index. Pages still enforce tenant, type and deletion
scope. This does not guarantee that every search is covered or faster.

Create, update, restore, transaction writes and non-deferred bulk ingestion
carry metadata from the same extraction that produces value-index rows.
Deferred bulk ingestion (`defer_indexing`) or offloaded indexing leaves
metadata unknown and removes any old array on update; a whole-resource
extraction failure does the same. A transaction Bundle that defers its
value-index rows (#1975) leaves the metadata of the resources of deferred
types it creates, updates or restores unknown until its post-commit step, or
the deferred-index repair, writes it together with the rows from one
extraction (#2035: an update or a restore removes the array its previous
version carried and extracts nothing in the transaction). Until then the
resource still carries the deferral's `search_rows_pending` marker, and a
`:missing=true` search on the metadata route leaves it out, as every
row-driven search does, rather than listing it as missing every parameter. Its
deletes extract nothing and still write `missing_params: []` in the
transaction, and the step writes the same `[]` (see
[deferred search indexing for large transaction Bundles](../README.md#deferred-search-indexing-for-large-mongodb-transaction-bundles)).
Reindex updates metadata only when the resource version still matches the
version read for that page.

Reindex can write metadata even when that resource's value-index insertion
fails. The resource is reported as failed, but its index may remain incomplete
until a successful rewrite. Check job errors and repair failed resources;
metadata is not proof that all value-index writes succeeded. Metadata-update
time is included in the insert timing statistic.

### Single-parameter extraction failures (#2002)

A parameter that fails extraction on a resource — a custom SearchParameter
whose expression errors, such as `.single()` over a repeating element — does
not fail the resource. Every other parameter's rows and the resource's
metadata are written (the failed parameter recorded neither as present nor as
missing, as above), and `$reindex` reports the resource OK. Instead the job
aggregates one warning per failing parameter: `GET /$reindex-status/{job_id}`
lists a `parameterWarning` part carrying the parameter code, how many
resources it failed on, and one example error, so an operator can fix or
retire the definition without wading through per-resource errors. The
warnings do not mark the job failed and do not appear in the terminal
`AuditEvent`'s outcome. A whole-resource extraction failure (the resource
could not be converted at all) still reports that resource as a job error,
exactly as an insert failure does.

### Upgrade and definition maintenance

1. Drain older writers before allowing metadata-aware writers to update the
   same resources. Older writers do not maintain or invalidate this field;
   they can leave stale metadata on a resource previously written by a newer
   node. The same restriction applies to a writer downgrade.
2. Start the upgraded nodes. They serve while `idx_resources_missing_params`
   builds (see above); wait for the `resources index build complete` log line
   before judging `:missing=true` latency. Existing resources without metadata
   remain searchable through the compatibility path while their value indexes
   remain valid.
3. Populate metadata for existing resources with a metadata-only reindex
   (#2008), which writes `missing_params` and leaves `search_index` and
   `search_index_contained` untouched:

   ```http
   POST /$reindex
   Content-Type: application/fhir+json

   {"resourceType": "Parameters", "parameter": [
     {"name": "metadataOnly", "valueBoolean": true},
     {"name": "skipExistingMetadata", "valueBoolean": true}
   ]}
   ```

   Poll `GET /$reindex-status/{job_id}` on the same node and resolve reported
   failures before treating the backfill as complete. A full `POST /$reindex`
   (no parameters) populates metadata too, but also deletes and rewrites every
   value-index row.

### Metadata-only reindex

`metadataOnly` runs the same walk, extraction and version guard as a rebuild:
each page's resources are extracted exactly as a rebuild would extract them,
and `missing_params` is written only where the stored version is still the
one the page read. It writes nothing to `search_index` or
`search_index_contained`. Because the metadata comes from the same extraction
that produced the value-index rows, `:missing=true` results are unchanged by
the backfill as long as those rows were written under the current
definitions; only the plan moves from the compatibility path to
`idx_resources_missing_params`.

| Parameter | Effect |
|---|---|
| `metadataOnly` | Write only `missing_params`. Cannot be combined with `clearExisting`, `clearOnly` or a bulk index rebuild. Combines with a resource type (`POST /{type}/$reindex`) and with `idStart`/`idEnd`. |
| `skipExistingMetadata` | With `metadataOnly`, leave alone resources whose `missing_params` already carries the current `missing_params_fp`. Resources without the array, with an unstamped array (written before #1999) or with a stale stamp are written. Every resource in scope is still read and extracted; only the write is skipped. |

`$reindex-status` reports `metadataOnly: true` and `metadataUpdated`, the
number of resources whose metadata was written; `entriesCreated` stays `0`.
A resource whose extraction fails has its metadata removed, as in a rebuild,
and is listed as an error. On a deployment where Elasticsearch owns search
(`mongodb-elasticsearch`), `missing_params` is never read and
`metadataOnly` is `501 Not Implemented`, as it is on every other backend.

### Definition changes (#1999)

Every writer stamps `missing_params_fp` next to `missing_params`: a
deterministic fingerprint of the tenant's active definitions for the resource
type (and its abstract bases) at write time. The first time a process stamps
a fingerprint it records, in the `missing_params_fingerprints` collection,
one digest per parameter code under that fingerprint. The collection holds
one small document per distinct set of definitions and is never rewritten.

Before each `:missing=true` search or count the server probes
`idx_resources_missing_params_fp` for the live resources of the tenant and
type whose array is unstamped or whose stamp differs from the current
registry's fingerprint. The probe reads one key when it finds an unstamped
array and otherwise one key per distinct stale stamp, never one per resource.
Then, per parameter:

- no stale or unstamped array: the metadata route answers;
- only stamps whose recorded digest for this parameter equals the current
  one: the metadata route answers — those arrays were computed from the same
  definition, so an unrelated SearchParameter change leaves the parameter on
  its fast path;
- otherwise (a new or changed parameter, a stamp missing from the catalog,
  any unstamped array, more than 32 stale stamps, or a failed probe): the
  value-index route answers.

Creating, changing or retiring a SearchParameter therefore gives correct
results immediately, on every node, with no manual steps: a node still
holding the older registry stamps its writes with the older fingerprint,
which the other nodes compare parameter by parameter. A search path that did
not probe, or a definition change since the probe, takes the value-index
route.

The stamp cannot repair value-index rows: after changing an existing
definition's *expression*, resources written under the old expression answer
from their old rows (exactly as ordinary value searches do) until a
`POST /$reindex` of the affected types rewrites them. `$reindex` also
restamps the metadata, which restores the metadata route for the new or
changed parameter; until then `:missing=true` on that parameter pays the
value-index route, which is about the cost it had before #1984.

Resources written before the stamp existed (or by a node that predates it)
carry `missing_params` without `missing_params_fp`. The probe finds them in
one key and every `:missing=true` on the type takes the value-index route, so
results are correct after an upgrade without any manual step. Run the
metadata-only reindex from the upgrade steps above (`metadataOnly`, with
`skipExistingMetadata`) to stamp them and restore the metadata route without
rewriting the value index.

Recovery, when metadata is suspected wrong despite the stamp (a restore from
backup, a manual edit): `$unset` both fields together for the affected tenant
and resource types and `$reindex`. Do not merely remove one parameter name
from the array, and do not unset one field without the other: a present array
with a current stamp is trusted. Include every affected type when a
definition applies to an abstract base or multiple resource types.

```javascript
db.resources.updateMany(
  { tenant_id: "TENANT_ID", resource_type: { $in: ["Observation"] } },
  { $unset: { missing_params: "", missing_params_fp: "" } }
);
```

## Composite parameters

Composite search (`code-value-quantity`, `component-code-value-quantity`, ...) uses the existing value indexes for matching (#1206). The extractor writes one row per component value in `search_index`, or in `search_index_contained` for a contained resource. Rows in one composite instance share `param_name` (the composite's code) and `composite_group` (the base-instance index). Each row also has a `composite_slot`, the component's position among components of the same type. The existing value indexes bound each component predicate by its type and value.

For standard searches without `:missing` or `:not` siblings, the planner picks, for each value, the non-`ne` component its bounded probe finds smallest (when every arm exceeds 10,000 rows, the saturated arms are recounted at doubling limits up to 100,000; the first one when none of those finishes). That arm only discovers candidates: bounded candidate ids, pages in resource order, or batches in id order, as for any other parameter. Every batch is then checked against all components within the same `composite_group`, through finds hinted onto `idx_search_composite` and bounded to the batch's ids. These planned composite searches do not collect every matching id and do not stop at 300,000 matches (#1896). Totals stay exact, so `_total=accurate` and counts still read every match. Some composite searches keep the earlier candidate collector and its 300,000-id cap: those with a `:missing` or `:not` sibling, a `_sort` on a search parameter, or a positive `_id` condition (literal, or resolved from a chain, `_has` or `_list`), and pages that no bounded plan serves. The last are broad pages, whose discovery arm is too wide for candidate ids, that sort by anything other than none, `_lastUpdated`, `_lastUpdated,_id` or a single `_id` (for example `_sort=_id,_lastUpdated`), and broad pages before the search indexes report ready that are neither a count nor sorted by `_id` alone. The collector drives from the component filter with the lowest probe count and pair-checks 512-id batches through the same hinted finds. A resource-order page whose first batches match rarely can still ask the collector for its id set, as for any parameter; when that set is over the cap, the page resumes the scan instead of returning 422. A `_sort=_id` composite page groups the discovery arm's rows on the server before its first batch, so its server-side cost follows the size of that arm. Positive reference `:identifier` (resolved to its targets first) and compartment membership are planned as plain filters, and a compartment can drive. Contained searches group matching rows by contained entity and `composite_group`. When components share a type, both paths require the matching `composite_slot`, so `A$B` does not match rows for `B$A`. SQLite and PostgreSQL unfolded composite rows now retain these per-type slots too (SQLite schema 38, PostgreSQL schema 48), so repeated-type ordinary and contained SQL searches preserve positions. PostgreSQL ordinary Denormalized searches retain folded rows with at most two Token or Number components and one of each other family; shapes beyond that remain unsupported for ordinary or `_contained=both` searches and cannot be repaired by reindexing. Its Legacy reader accepts slotted unfolded rows and valid transitional folded Token/Number rows. Contained SQL searches use unfolded rows in either layout.

Bounded complement and unfiltered sort (#2285): a `:not` or `:missing=true` search with no positive parameter, with or without a compartment, no longer loads every live id of the type. It walks a cursor in 512-id batches (live resources, or compartment rows checked for liveness), filters each batch with the same `:missing`, `:not` and compartment checks, and stops as soon as more than 300,000 ids match. Past that cap, a search with a query plan falls back to the plan; one without a plan answers 422. Memory is bounded by the cap plus one batch whatever the size of the type; time is still proportional to the type when the complement is small, and is bounded by `HFS_MONGODB_SEARCH_TIMEOUT_MS`. A `_sort` by a typed search parameter with no filter walks the sort's value index (see [Sorting past 300,000 matches](#sorting-past-300000-matches-generation-7)); otherwise it asks the server for only the first `offset + _count + 1` ids (resources without a sort key come after those that have one, in id order) and answers 422 when `offset + _count` exceeds 300,000. These sorts issue no page cursor, so `_offset` is the only way to page; add a filter to reach deeper. `_total` is counted only when requested. No index is added or changed, so write cost and storage are unchanged.

SQL's legacy guard is conservative for positional matching of the parameter: any ambiguous unfolded row in the tenant/type/parameter and requested top-level or contained scope causes a named 400 asking for `$reindex`, even for a value with no matches or count zero. Ordinary composite `:missing=true|false` is presence-only and bypasses SQL positional shape checks and legacy-slot probes; contained `:missing` remains unsupported. MongoDB's guard below is value-dependent. SQL startup builds sparse legacy-probe indexes; PostgreSQL's ordinary family-aware probes are created only when initializing Legacy, while its contained probe exists in both layouts. These builds can lengthen the first startup after upgrading. See [SQL composite upgrade recovery](../README.md#sql-composite-slots-and-upgrade-recovery).

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

For SQL and MongoDB, after every writer runs the slot-writing version, send `POST /$reindex` for the affected tenant. With no body, `clearExisting` defaults to `false`; the job rewrites the resources' index rows without clearing the entire tenant index first. The response has a `jobId`. Poll `GET /$reindex-status/{job_id}` on the same server node until its `status` parameter is `completed` and `errorCount` is zero. The operation requires the `system/reindex` scope when authentication is enabled.

Use the tenant-wide route even when the repeated-type parameter belongs to `Observation`. A contained Observation is indexed under its container's resource type, so `POST /Observation/$reindex` does not rebuild its rows when the container is another type. Keep older writers from writing to that tenant during and after the rebuild; they would create rows without slots again.

An arity mismatch — a value with fewer or more `$`-separated parts than the parameter declares (e.g. `code-value-quantity=8302-2`, one part for a two-component parameter) — is a 400 (`InvalidComposite`) on every backend, naming the received and expected part counts (#1236).

## Sorting past 300,000 matches (generation 7)

A filtered `_sort` by one search parameter first collects matching ids and
orders them (`$min` of each resource's values ascending, `$max` descending,
and the range end for a descending date). If collection exceeds 300,000
matches, the supported single-key sorts instead walk the sort parameter's
value index (#2588). A descending date uses `idx_search_date_end`.

An unfiltered sort by one typed search parameter (`_sort=<param>` or
`_sort=-<param>` with no other parameter) skips the collection step entirely
when the parameter's value index is built (#2601). It runs the same walk with
no filters and stops after `offset + _count + 1` live ids, instead of grouping
every row of the parameter with `$min`/`$max` and sorting the result. Ids, tie
order, missing-value placement, `has_next` and the total are the same as the
grouped path, which still serves the request when the index is missing, still
building, or dropped while running.

The walk serves both directions of every type with a value field: date
(ascending by start through `idx_search_date_v3`, descending by end through
`idx_search_date_end`, once the full build is done), string, token (by code),
number, quantity (by `value_quantity_value`), reference and uri. They are
equivalent to the grouped answer for one reason: the first row of a resource in
key order holds its `$min` (ascending) or `$max` (descending), so the walk keeps
each resource at its first row and ignores its later ones, however many values
it has.

The walk reads its rows from a covered index scan: the row filter is
`<value field>: {$ne: null}` alone, which the index key answers, so the sort-key
cursors examine index keys and no document (`docsExamined: 0`). An earlier form
added `$exists: true`, which an index key cannot answer, and so fetched the
`search_index` document of every row it passed; on 689,080 Observations that
made 100,021 rows of `-date` cost 4.5 to 9 s instead of 45 to 55 ms, and turned
the WiredTiger cache over 2.5 times per deep request. The filtered walk over the
cap (#2588) opens the same cursors and is covered the same way.

There is no limit on the offset other than the 300,000-row id cap, which still
answers 422 past it: the covered walk is cheaper than the grouped aggregate at
every offset up to it. Single-request server and driver time on 689,080 real
Observations (`_sort=-date`, mongo:9.0.8, one request at a time, a replay of
both code paths): offset 100,000 651 ms against 1,854 ms grouped, offset
290,000 1,944 ms against 3,619 ms. Page 1 takes a few milliseconds against
about 0.9 s grouped.

The walk sizes its batches to the window. The first batch reads and checks the
liveness of about `offset + _count + 1` ids (`unfiltered_walk_batch`: the
window, a sixteenth more for stale rows, at least 32, at most 10,000), so page
1 of a small type checks 32 ids, not 512; later batches follow what the window
still lacks, and double while the window waits for an open tie or a batch
yields less than half its ids. The sort-key cursor reads 128 to 2,048 rows per
round trip, by the same window. Once the window holds what it needs, the walk
stops at the first row of a new key, so it never enters the group that follows
the page.

The walk does have a guard on the equal-key groups it reads whole, to order
them by id. A parameter with few distinct values (`status`, `gender`) has a
group of hundreds of thousands of rows. When a group reaches 512 rows (128
when the window already holds every id it needs, only waits for that group to
end, and the walk reads one stream) the walk counts it once, with a covered
count bounded just above what it may read (`sort_key_rows_upto`, about 0.13
microseconds a key). If more of its rows remain than the window still lacks
plus `max(UNFILTERED_WALK_TIE_ROWS, offset + _count + 1)`
(`UNFILTERED_WALK_TIE_ROWS` is 8,192), the group is past the walk's allowance:
unless the read by id below closes the window in it, grouping is cheaper and
the grouped aggregate answers with the same page, after at most 512 rows of
the group, one count and the reads by id, if tried. A group within the
allowance that the read by id does not close is walked to its end, so a deep
window inside a long group (`_sort=code&_offset=30000`,
`_sort=subject&_offset=10000` on the same data) is walked instead of abandoned
after 8,192 rows with the walk thrown away. For a descending date the count
sees only the rows with a range end, a lower bound of the group: it can let the
walk finish a group, never change the page.

A first group that `unfiltered_first_group_window` declined because it holds
fewer members than the window, when that showed its size (it ends in the first
cursor batch, its bounded count, or a read that was not cut) and the size is
within any allowance, is treated as counted: the walk neither counts it again
nor cuts its batches at 512 rows of it, so its liveness checks are not split
there either.

The count also tells the walk when the group its window ends in is much longer
than the window still needs: more rows than those already read, plus what the
window lacks and a quarter more, plus 256 (`unfiltered_closes_group_by_id`),
and the read below could hold what the window lacks (what it lacks plus a
sixteenth is at most 10,000 rows). Then, before finishing or abandoning the
group, the walk reads it by id (`unfiltered_group_closed_by_id`): it pushes the
batch's rows of earlier keys, takes the ids ordered before the group (the open
tie of the group itself is dropped), and reads the group's rows on the walk's
own index, with the same key filter as the count, sorted by `resource_id` in
the request's id direction. Resources the walk already met at an earlier key
are not members and are skipped, as are a resource's repeated rows; the rest
are checked for liveness in one command, and the first ones that fill the
window close it. Ids the walk already knows live are not checked again, but
only those of the current batch and of the group's open tie; members after the
one that completes the window from those known ids are not checked at all. This
is the grouped answer: every key before the group is complete, and the group's
members follow in id order.

There are at most two reads. The first asks for what the window lacks scaled by
the rows the walk read of the group per member it met in them, plus a
sixteenth (at least 32, at most 10,000): a group whose rows are mostly
resources already met at an earlier key, or repeated, is read far enough at
once. A read cut before enough live members is followed by one read of 10,000
rows, which checks only the members not checked yet. When the walk met no
member in the group's rows, what the window lacks would need more than 10,000
rows, or the group ends with too few live members, the walk goes on exactly as
without the reads, and still hands a group past its allowance to the grouped
aggregate; when the index was dropped, the grouped aggregate answers. A
descending date or quantity with any row without an end does not read by id,
for the reason given below. This serves a window that crosses from a first
group into a long second one (`CarePlan?_sort=status&_offset=1700`,
`Condition?_sort=clinical-status&_offset=8600`), which used to read the
first group by id in vain, walk it again, walk into the second group and then
either finish it row by row or, when it was too long, group the whole
parameter.

The walk does not fetch a sort-key cursor batch only to reach the probe.
Before a row that would cost a round trip (its stream is empty and not
exhausted), when the walk's batch already holds what the window lacks: in the
walk's first batch, it checks the rows in hand first; and when the window is
full and waits for an open tie it has not counted (one stream), it reads that
group by id at once instead of fetching more of it, and counts the group only
if that read leaves it open, to choose between finishing it and the grouped
aggregate as the probe's count would have. A window just past the first group
that the first batch covers (`Patient?_sort=gender&_offset=520`) so costs the
batch's check and one read by id, without a count or a second cursor batch.
Later batches are not cut at the cursor's round trips: a first group of
deleted resources is still checked in doubling batches, not once per cursor
batch.

Inside a batch, when the open tie already holds every id the window lacks, a
row of the tie's key whose id cannot displace the tie's worst id is dropped by
the window whatever its check says, and a row of another key completes the
window whatever its check says; such rows are not sent to the liveness (or
membership) check, and a batch with none left sends no command.

Numeric keys compare `-0.0` equal to `0.0`, as the server orders and matches
them; telling them apart split a group of zeros into alternating groups and
lost its id order.

A window inside the first group is not walked at all
(`unfiltered_first_group_window`). Before the walk starts it looks at the
sort-key cursor's first batch, which the walk requests anyway and keeps. When
that batch opens with at least 128 rows of one key `K`
(`UNFILTERED_WALK_MIN_CURSOR_BATCH`), does not show the group ending before
`offset + _count + 1` distinct resources, a descending date or quantity has no
row without an end (its null-end stream is empty), a group that goes on past
the batch has at least `offset + _count + 1` rows (one bounded count, so a
window that crosses the group's end costs a count instead of a read and a
liveness check), and `offset + _count + 1` is at most 10,000,
the group is read by id: one covered `find` of the rows with key `K` on the
walk's own index (a descending date: `value_date_end = K` on
`idx_search_date_end`; a descending quantity: `value_quantity_high = K` on
`idx_search_quantity_high`), projected to `resource_id`, sorted by `resource_id`
in the request's id direction and limited to the window plus a sixteenth
(`unfiltered_group_read_limit`, which is not cut at 10,000, so a window close to
it keeps room for stale rows), then one liveness check of those ids. A resource's
repeated rows are adjacent in that order and count once. This is the grouped
answer: no resource has a key before `K`, so the group's members are exactly
the resources with a row of `K`, and they are ordered by id; resources after
the group, and the keyless tail, are not reached. When the group holds fewer
live ids than the window (a short group, stale rows of deleted resources) or the
index was dropped, the walk runs unchanged from the buffered batch. On real data
this serves `CarePlan?_sort=-date`, where 1,653 of 3,580 rows share the open end
`9999-12-31` (the walk used to finish that group and check 1,655 ids in seven
batches), and `Immunization?_sort=status` with one status over 14,927 rows (the
walk used to give up after four batches and group). The sort by
`resource_id` is index-provided only when nothing sits between the key and
`resource_id` in the index (string, number, reference, uri); for the others
(token, date, quantity) the server sorts the group's covered keys with a top-k
sort bounded by the limit.

These keep the grouped path: a sort whose parameter type is unknown
(the `value_string` fallback), composite and special parameters, and sorts on
more than one key.

The walk reads rows in value order. The first row for a resource supplies
its sort key; later rows for that resource are ignored. Membership and
resource-level conditions are checked in batches of at most 512 candidates
(the unfiltered walk, which only checks liveness, sizes its batches to the
window as described above).
Equal keys retain only the matching ids needed for `offset + _count + 1`,
ordered by the requested id tie-breaker. The extra match determines
`has_next`. Matches without a sort value follow in id order, read from
`resources`. Page order agrees with the collected path.

The amount scanned depends on where matches occur in sort order, repeated
values, and equal-key groups. A small page can still require reading the
whole sort range: overall selectivity alone does not bound that work.
The walk also follows the initial collection attempt, which has already
passed the match cap. Its retained page and membership batches are bounded,
but the set used to recognize previously seen resource ids grows with the
number of distinct keyed resources scanned. The configured search deadline
can stop new work; it is not a memory limit.

The path answers 422 when `offset + _count` exceeds 300,000, as the
unfiltered sort does. Multiple sort keys keep the collected path and its
cap. `_total` uses the same search without its sort; when that count also
requires the capped collector, the request still answers 422. This
includes a FHIR-version-restricted accurate total over the cap.

Existing value-index plans remain usable while the new date-end index is
built, provided their actual index definitions and required maintenance
are verified. The new descending-date walk, the unfiltered `-date` walk and the `idx_search_date_end` hint of `gt`, `ge` and `ne` date filters wait for the full
build: until then those filters keep the `idx_search_date_v3` hint and the
unfiltered `-date` sort keeps the grouped path (the other unfiltered sorts need
only their existing value index). If the walk's required
indexes are unavailable, an over-cap sort keeps the 422 response. Other
within-cap searches keep their existing execution path.

Date rows written before #1391 have no `value_date_end`. They are null-end
keys of `idx_search_date_end` and sort by their start, as on the collected
path. A descending date walk reads them through a second cursor
(`value_date_end: null`), merged with the first by key. That cursor is not
covered: a null in the index key does not tell a null end from a missing one,
so the server fetches each such row. It examines one key and no document once
the date rows are reindexed; on a database that still holds many such rows, a
deep unfiltered `-date` page fetches each of them, as every walk did before
the cursors were covered.

## Token+quantity composites (generation 6)

A token+quantity composite (`code-value-quantity`, `combo-code-value-quantity`, `component-code-value-quantity`) normally discovers candidates through one component and pair-checks the other within the same `composite_group` (see [Composite parameters](#composite-parameters)). Generation 6 lets `code=X AND value>N` be one covered range scan instead:

- When HFS writes the rows of a two-component composite, the quantity row of each group also carries `tuple_code`, an array with every code form of the group's token rows: `c:<code>` for the code alone, and `s<byte length of system>:<system>|<code>` for each system and code pair. The length prefix keeps a `|` inside a system or code from making two different pairs collide. A group whose token rows have no code (a `CodeableConcept` with only `text`) gets no `tuple_code`.
- `idx_search_composite_tuple_qty` is `{ tenant_id, resource_type, param_name, tuple_code, value_quantity_value, resource_id }`, partial on `tuple_code` existing. The builder creates it in the background with the other value indexes.

The `tuple_code` format changed during development: the first form joined system and code with a bare `|`, and its markers used the id `<tenant>|composite_tuple_code`. No release wrote it. HFS never reads those markers, so a database written by such a build uses pair validation until `$reindex` rewrites its rows in the current form.

### Turning the tuple path on: `$reindex` per tenant

Rows written before generation 6, or by an older instance, have no `tuple_code` and would silently drop out of a tuple scan. The planner therefore uses the index only for a tenant, or a tenant and resource type, that a completed `$reindex` marked complete. The markers are documents in the `search_layout` collection, `_id` `<tenant>|composite_tuple_code_v2` or `<tenant>|<type>|composite_tuple_code_v2`. A new database and a new tenant have none, so they use pair validation (correct, slower) until their first `$reindex`.

A run writes a marker only when it finishes with no error, and only when it rebuilt every resource of its types:

| Run | Marker |
|---|---|
| `POST /$reindex`, with or without `clearExisting` | tenant |
| `POST /{type}/$reindex` | that type |
| `idStart` / `idEnd`, `clearOnly`, `metadataOnly`, a run limited to some search parameters or resources | none |
| A run that is cancelled, fails, or skips a resource it could not read or index | none |

Before writing the marker, HFS checks each type in scope for a quantity row of a token+quantity composite without `tuple_code` whose group has a coded token row. If it finds one (an older instance wrote it while the run went on), it writes no marker for that type, or no tenant marker, and logs a `warn`. The check reads each composite quantity row of the scope once. It sees only rows written before it reads them: a row an older instance writes after that is the case [Deployment order](#deployment-order) rules out.

After writing the markers, HFS reads `resources` once for the tenant. If the tenant has no resource left, a tenant purge ran during the run (on any instance), and HFS removes the markers it just wrote, so a tenant id reused later does not start out marked. A tenant that was empty when the run ended gets no marker either; that only means pair validation until its next `$reindex`.

The planner also needs the recorded generation to be 6 or more, the value indexes ready, one composite value, no modifier, a token part that is a code or `system|code`, and a quantity prefix other than `ne`. Any other search keeps pair validation. Markers are cached per instance for 30 seconds.

To check a tenant, in `mongosh` against the HFS database:

```javascript
db.schema_version.findOne({ _id: "schema_version" }).search_indexes.generation  // 7
db.search_layout.find({ tenant_id: "<tenant>" })
```

### Deployment order

1. Deploy the new binary to every instance.
2. Wait until `search_indexes.generation` reads 6 or more (the background build is done; this build records 7).
3. Make sure no instance of an older binary is running against the database.
4. Run `$reindex` for each tenant, or `POST /Observation/$reindex` (and the other types with token+quantity composites).

A `$reindex` that completes before step 2 is done normally loses its markers: when HFS records generation 6 over generation 5 (or lower) it first deletes every marker (see [Rollback and restore](#rollback-and-restore)), so run it again after step 2.

An older instance that is still running writes rows without `tuple_code`. It records its generation only when it boots, so HFS cannot see it. If it writes after a tenant was marked, those rows are missing from tuple-path searches of that tenant until the next `$reindex`. Do not run the activating `$reindex` while one is alive.

### Rollback and restore

- **Rollback.** An older binary records its own generation at boot: 5 for a build with #2394 and without this one, 4 before #2394. Every instance of this binary stops trusting the markers within 30 seconds, and searches fall back to pair validation. When this binary records generation 6 again, it first deletes every marker; if it cannot delete them, it records nothing and the markers stay untrusted. After the return, run the activating `$reindex` again, in the order above. To leave generation 6 for good (or before handing the database to a generation-5 build), run `composite-tuple-rollback.mongosh.js`; see [Downgrading](#downgrading).
- **Restore.** A `mongorestore` into a database whose tenants are marked, or any tool that writes `search_index` directly, leaves rows without `tuple_code` behind a marker. After a restore, delete the markers and reindex: `db.search_layout.deleteMany({ layout: "composite_tuple_code_v2" })`, then `$reindex` per tenant. A tenant purge deletes that tenant's markers, including one written by a `$reindex` of that tenant that finishes after the purge.
