# MongoDB `search_index` indexes

HFS keeps two kinds of index on the `search_index` collection.

- **Inline** indexes (`idx_search_composite`, `idx_search_resource`, `idx_search_composite_slot_probe`) are created at every boot before the server serves. The slot probe index is partial and includes only composite rows.
- **Generation 4** is the current generation: nine partial value indexes (`idx_search_date_v3` and eight named `idx_search_*_v2`), built by HFS **after** boot in one `createIndexes` command that scans the collection once, plus contained rows living in their own collection since generation 3 (see "Contained rows" below) rather than a partial index on `search_index`. MongoDB 4.2 and later do not block reads or writes during the value-index build. When every generation-2 value index is ready, HFS drops the nine generation-1 value indexes (`idx_search_string`, `idx_search_token`, ...); once the contained-row move is also done, HFS records `search_indexes.generation: 4` in the `schema_version` document.

Why: a generation-1 value index carried one entry for every row of the collection, even rows that had no value of that type, and no value index carried `resource_id`, so every search fetched one document per matching key. Generation-2 indexes are partial (one entry per row that has the value) and end in `resource_id`, so a value-filtered scan is covered. Issues #1059 and #1084 have the measurements.

## Date ranges (generation 4)

Since #1391 a date row stores the range it denotes, `[value_date, value_date_end)`, and a date search bounds either end (`gt` and `ge` bound `value_date_end`, for example). `idx_search_date_v3` carries `value_date_end` between `value_date` and `resource_id` so those searches stay covered; it is still partial on `value_date` existing. It replaces generation 2's `idx_search_date_v2`, which HFS drops once every generation-4 index is ready (under the same mode rules as the generation-1 drop). Until then date searches keep using `idx_search_date_v2`, fetching documents to check `value_date_end`. Every prefix is a covered scan of `idx_search_date_v3` (`docsExamined: 0`). The prefixes with two alternatives (`ge`, `le`, `ne`), and a comma list of date values, are sent as an `$or` at the top of the filter with every arm repeating the tenant, resource type and parameter name; nested under those shared conditions MongoDB 5.0 reads the documents instead of scanning the index.

### Upgrading: reindex the date rows

Rows written before #1391 have no `value_date_end`. They never match the prefixes that bound the end (`eq`, `ne`, `gt`, `ge`, `le`, `eb`, `ap`) until they are rewritten. A Period indexed before #1391 is also still two independent point rows, so even `lt` and `sa` compare each of its ends on its own until it is rewritten (point values are unaffected by those two). Run `$reindex` after the upgrade. HFS reminds you: when the builder records generation 4 on a database that was at an earlier generation (or had none recorded) and `search_index` already held rows when the server started, it logs one `warn` naming `$reindex`. HFS reads the recorded generation and the collection's metadata count at startup, before it accepts writes, never the rows, so rows written while the first build runs in `background` mode do not count. On a database with no generation recorded and an empty `search_index`, it also records `search_indexes.empty_at_generation`, so a restart, or an `off`-mode database built later, stays silent before the first build records generation 4; recording the generation removes that field. If the startup check fails, HFS logs a different `warn` saying it could not tell, naming `$reindex` for a database upgraded from before #1391. It is silent on a new database and on every boot once generation 4 is recorded, including a later upgrade from generation 4 to a newer generation. The startup check counts only `search_index`. Contained-resource date rows written between #1160 and #1391 (generation 3) also lack `value_date_end` but live in `search_index_contained`, which the check does not count, so a database whose only such rows are contained ones gets no reminder. `$reindex` rewrites both collections.

## `HFS_MONGODB_INDEX_BUILD`

| value | behaviour |
|---|---|
| `background` (default) | Boot returns immediately. The build runs in the background and is logged at `info` when it starts and finishes. |
| `inline` | Boot waits for the build. Use for tests, developer databases and small deployments. |
| `off` | Nothing is built or dropped. Each missing generation-2 index is logged at `warn`. Each generation-1 index still present once generation 2 is complete is also logged at `warn`, naming the `dropIndex` command to remove it. Use when you pre-build in a maintenance window. |

## Upgrading a large deployment

Deploy the binary. The server serves on generation 1 while the build runs. Disk peaks at the size of both generations, then drops when the old ones are removed. Writes are slower during the build by the cost of maintaining both sets. Nothing needs to be scheduled.

To build in a window of your choosing instead, run the pre-build script first; a database that already has every generation-2 index makes the builder a no-op at boot:

    export HFS_MONGODB_DATABASE="${HFS_MONGODB_DATABASE:-helios}"
    mongosh "$HFS_MONGODB_URL" --file docs/mongodb/search-index-v2.mongosh.js

Every script on this page runs this way. mongosh connects with the URL HFS uses, unchanged, so a URL with a query string such as `mongodb://h:27017/?replicaSet=rs0` works. The script then selects the database named by `HFS_MONGODB_DATABASE`, and stops before it changes anything if that variable is unset. HFS uses `helios` when no database name is set, hence the default above.

## If the build fails

The log carries the server's error at `error` level. Nothing has been dropped; searches keep using generation 1. Fix the cause (usually disk) and restart: every step is idempotent, and a completed index is skipped.

If a generation-2 name exists with a different key spec, HFS refuses to build or drop anything and logs both specs. Drop or rename that index by hand. A spec that differs only in the numeric type of its key values (for example one built by mongosh, which stores doubles where HFS stores integers) is not treated as a conflict.

## Contained rows (generation 3)

Rows extracted from a resource's `contained` entries live in their own collection, `search_index_contained` (#1160). A standard search reads only `search_index`, so it can no longer match a container through a same-type contained resource; `_contained=true|both` searches read `search_index_contained`. Its three indexes are created inline at every boot, so running the script below is optional; it exists for operators who prefer to bootstrap a fresh database by script before first boot:

    mongosh "$HFS_MONGODB_URL" --file docs/mongodb/search-index-contained.mongosh.js

On the first boot of a generation-3 binary HFS moves any contained rows still in `search_index` into the new collection, in pages of 1,000, in every `HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build), records `search_indexes.contained_rows_moved: true`, and then drops the old partial `idx_search_contained` from `search_index` under the usual mode rules (in `off` mode it warns and names the `dropIndex` command). Most deployments have no contained rows, so the move is one empty find. On a deployment that has contained resources and takes writes during boot, run that first generation-3 boot with `HFS_MONGODB_INDEX_BUILD=inline`, or run `$reindex` afterwards: a container updated while the background move is paging can get its pre-update contained rows re-inserted.

## Downgrading

These steps cover the `search_index` generations. The MongoDB schema version is recorded separately: builds from #2352 on refuse to start on a database whose `schema_version` records a newer version than their own, so a rollback across a schema version also needs the steps in [Rolling back to an older build](schema-version.md#rolling-back-to-an-older-build).

A binary from before generation 2 creates the nine generation-1 indexes at boot, inline, and will not serve until they exist. If they have been dropped, that boot rebuilds all nine before serving. Before rolling back, with `HFS_MONGODB_DATABASE` exported as under [Upgrading a large deployment](#upgrading-a-large-deployment), recreate them in the background:

    mongosh "$HFS_MONGODB_URL" --file docs/mongodb/search-index-v1-rollback.mongosh.js

A binary from before generation 3 reads contained matches from `search_index` only, so `_contained` searches return nothing for rows that were moved (standard searches are unaffected). Before rolling back, with `HFS_MONGODB_DATABASE` still exported, copy the rows back and recreate the partial index:

    mongosh "$HFS_MONGODB_URL" --file docs/mongodb/search-index-contained-rollback.mongosh.js

That script also resets the `schema_version` generation record, so rolling forward again re-runs the contained-row move instead of skipping it.

A binary from before generation 4 needs no script: it sees `idx_search_date_v2` missing and rebuilds it in the background like any other missing generation-2 index, and ignores `idx_search_date_v3`.

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
types it creates unknown until its post-commit step, or the deferred-index
repair, writes it together with the rows from one extraction. Until then the
resource still carries the deferral's `search_rows_pending` marker, and a
`:missing=true` search on the metadata route leaves it out, as every
row-driven search does, rather than listing it as missing every parameter. Its
updates and deletes still write metadata in the transaction, and the step
rewrites it after the SearchParameter reload (see
[deferred search indexing for large transaction Bundles](../../crates/persistence/README.md#deferred-search-indexing-for-large-mongodb-transaction-bundles)).
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

Bounded complement and unfiltered sort (#2285): a `:not` or `:missing=true` search with no positive parameter, with or without a compartment, no longer loads every live id of the type. It walks a cursor in 512-id batches (live resources, or compartment rows checked for liveness), filters each batch with the same `:missing`, `:not` and compartment checks, and stops as soon as more than 300,000 ids match. Past that cap, a search with a query plan falls back to the plan; one without a plan answers 422. Memory is bounded by the cap plus one batch whatever the size of the type; time is still proportional to the type when the complement is small, and is bounded by `HFS_MONGODB_SEARCH_TIMEOUT_MS`. A `_sort` by a search parameter with no filter asks the server for only the first `offset + _count + 1` ids (resources without a sort key come after those that have one, in id order) and answers 422 when `offset + _count` exceeds 300,000. These sorts issue no page cursor, so `_offset` is the only way to page; add a filter to reach deeper. `_total` is counted only when requested. No index is added or changed, so write cost and storage are unchanged.

SQL's legacy guard is conservative for positional matching of the parameter: any ambiguous unfolded row in the tenant/type/parameter and requested top-level or contained scope causes a named 400 asking for `$reindex`, even for a value with no matches or count zero. Ordinary composite `:missing=true|false` is presence-only and bypasses SQL positional shape checks and legacy-slot probes; contained `:missing` remains unsupported. MongoDB's guard below is value-dependent. SQL startup builds sparse legacy-probe indexes; PostgreSQL's ordinary family-aware probes are created only when initializing Legacy, while its contained probe exists in both layouts. These builds can lengthen the first startup after upgrading. See [SQL composite upgrade recovery](../../crates/persistence/README.md#sql-composite-slots-and-upgrade-recovery).

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
