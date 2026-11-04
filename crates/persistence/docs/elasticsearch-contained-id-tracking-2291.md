# Elasticsearch secondary: track contained document ids (#2291)

Status: design. Base: `origin/main` @ `1384699f6`. Line numbers refer to
`crates/persistence/src/backends/elasticsearch/storage.rs` unless stated.

## 1. Problem

In the Elasticsearch (ES) secondary, a resource's `contained[]` entries are
indexed as separate documents (`is_contained=true`, `container_id`,
`container_type`), each in the index of its own type, with `_id`
`document_id(type, contained_resource_id(container_type, container_id, local_id))`,
for example `Medication_Observation_Obs1#med1` (see section 3.7 for the id
format and why it names the container's type).

Today the container document does not record which contained documents exist,
so every rewrite must discover them with a query:

- `update` (`:1719`, sweep at `:1780`), `upsert` (`:1452`, sweep at `:1582`),
  `write_search_entries` (`:2865`, sweep at `:2917`) and `delete` (`:1805`,
  sweep at `:1832`) call `delete_contained_docs` (`:1030`), a
  `_delete_by_query` with `refresh=true` over `hfs_{tenant}_*`, even when the
  resource has no `contained`. Only `index_new` (`:1231`) skips it.
- Contained documents are indexed with one HTTP call each (`:1111-1145`).
- The bulk page path (`write_search_page`, `:2555`, used by
  `write_synced_entries_page` `:2855`, rebuild pages, and bulk ingest) never
  deletes anything, so it leaves stale contained documents behind (section 6).

The sweep is already bounded (#2203: semaphore, `SWEEP_ATTEMPTS`, backoff on
429), but it still scans every index of the tenant and opens scroll contexts
(about 15 per sweep), which is what produces 429 `too_many_scroll_contexts`
under load.

## 2. Discarded experiments and alternatives

| Idea | Outcome | Why it is not this design |
|---|---|---|
| Skip the sweep when the primary says there was no `contained` (#2180 E-01) | Inserts +12.6 % but CRUD -50.8 % (429 `too_many_scroll_contexts` from the sweeps that still ran) | Skipping on a hint changes how many sweeps run, not what a sweep costs; the remaining ones collapse. |
| Same skip, variant (E-02) | Discarded | Same hint, same failure mode. |
| E-09 | Discarded | Out of scope here (sweep concurrency, owned by #2203). |
| Skip with a `had_contained` hint (E-10) | "The skip never fires in the benchmark": k6 events carry `None`. Regression test kept | A hint that is absent on real traffic cannot be the source of truth. |
| Old ids from the primary's previous version | Not reliable | After a failed sync of a version that dropped `contained`, the primary no longer lists the documents ES still holds. Test `a_failed_sync_of_the_version_that_dropped_contained_does_not_strand_its_documents` (`elasticsearch_contained_sweep_real.rs:639`) covers it. Also `write_search_entries` does not receive the previous version. |
| Store only a count | Rejected | Tells us that documents exist, not which; the sweep stays. |
| One grouped sweep with `terms container_id` | Rejected | Still a tenant-wide `_delete_by_query`. |

The common lesson: the only reliable record of which contained documents ES
holds is ES itself, so the container document must carry it.

## 3. Design

### 3.1 New field

The container document gets
`contained_doc_ids: ["Medication_Observation_Obs1#med1", ...]`, the ES `_id`s
of its contained documents. It is always written, as `[]` when
there are none. The type of each contained document, and so its `_index`, is
derived from the id prefix. The list is a hint that makes a rewrite cheap; it
is **not** what keeps stale documents out of search results (section 3.6).

Mapping (`schema.rs`):

```json
"contained_doc_ids": { "type": "keyword", "index": false, "doc_values": false }
```

It lives only in `_source`: it cannot be searched or sorted on, and costs no
inverted index or doc values. `SCHEMA_VERSION` goes 2 -> 3 (rule in
`schema.rs`: bump with any edit to the `mappings` half). The addition is
additive, so `reconcile_index` / `reconcile_index_mappings` apply it in place;
no `$reindex` is needed.

Reads are unaffected: read, vread and search hits take only `content` and
metadata from `_source` (`parse_stored_resource` `:2053`,
`parse_hit_to_stored_resource` `search_impl.rs:1611`), `_count` uses a query
and the search query does not reference the field.

### 3.2 Prior state

`fetch_contained_ids(tenant, type, id)` does a real-time `GET` of the container
with `_source_includes=contained_doc_ids`. A GET by `_id` is real time (no
refresh needed) and opens no scroll context. It returns:

- `Absent`: 404.
- `Legacy`: the document exists but has no `contained_doc_ids` (written before
  schema v3), or has one with an entry that is unreadable (not a string) or
  does not have the shape `{Type}_..#..`. Such a list cannot be trusted to name
  what exists, so it is treated as unknown and resolved by a sweep instead of
  skipping the entries it cannot read.
- `Known { ids, seq_no, primary_term }`.

`upsert` already does a GET for the existence check (`:1484`); it reuses that
GET and adds `_source_includes` instead of issuing a second one (the test
`create_or_update_retries_the_existence_check` counts GETs).

A batched variant uses `_mget` (one request per page).

### 3.3 Single-resource writes

For `update`, `upsert` and `write_search_entries`:

1. Fetch the prior state (3.2).
2. `Known`: `obsolete = old_ids - new_ids`. Send one `_bulk` with, in order:
   `delete` for each obsolete id, `index` for each current contained document,
   and last the `index` of the container, with `if_seq_no`/`if_primary_term`.
3. `Absent`: nothing was recorded and nothing is swept (on `update` and
   `upsert`); write as above, without deletes and without the concurrency
   check. `Legacy`: run the sweep first (the old list is unknown; section 4),
   then write as for `Absent`.

A resource without `contained` whose ES document has `[]` produces a
container-only `_bulk` and no `_delete_by_query`.

`delete`: fetch; `Known` with ids: one `_bulk` with a `delete` per contained id
and the container `delete`. `Known` with no ids: the container `delete` alone.
`Absent` or `Legacy`: the container `delete`, then the sweep as today (a delete
repeated after a partial failure finds an `Absent` container and must still
remove what is left). A `delete` item that answers 404 counts as success.

Creations (`index_new`, `index_many`) only fill the field; no GET.

### 3.4 `_bulk` generalization

`send_bulk_index` (`:2211`) becomes `send_bulk(ops: &[BulkOp])` with
`BulkAction::Index { doc, if_seq_no, if_primary_term }` and
`BulkAction::Delete`. It keeps `chunk_ranges`, the 429 resend, and the split on
timeout/413. Changes:

- Item results are read from `items[i]["index"|"delete"]`.
- `BulkFailure` gains a `conflict` flag (409).
- The ops of one owner are sent in a fixed order (deletes, contained documents,
  the container last), but that order is **not a guarantee**. Within one
  `_bulk` Elasticsearch distributes the items per shard and runs them in
  parallel across shards, and it answers an item that hit a full write queue
  with 429, which the backend resends later and separately. So a container can
  be applied while one of its contained documents is not yet (or never is).
  Correctness does not depend on the order: it rests on the read-side filter
  (3.6) and on the repair after a failed item (section 4).

### 3.5 Bundle sync and rebuild pages

`sync_bundle_results` (`composite/storage.rs:955`) turns every non-delete entry,
updates included, into `SyncEvent::created`. In synchronous/hybrid mode, and in
async mode for runs of two or more, those reach `write_synced_entries_page` ->
`write_search_page`, which indexes the current contained documents and never
deletes. Example: Observation `o1` has `#p1` and `#p2`; a transaction PUTs `o1`
with only `#p1`. The document `Patient_o1#p2` stays and
`Patient?_contained=true` still returns it. (A single-resource PUT is fine: it
reaches `write_search_entries`, which sweeps today.)

Fix in `write_search_page`: one `_mget` of the page's containers
(`_source_includes=contained_doc_ids`), then add `delete` ops for
`old - new` to the same `_bulk`. `Absent` owners get no deletes from the page;
a `Legacy` owner is swept per owner when the page is a composite sync page
(`sweep_legacy`).

This also applies to rebuild pages (`write_search_entries_page`). The comment
at `:2730-2739` says the rebuild must not delete first, because that would
drop the resource from search between delete and write. Deleting only ids that
the resource no longer has respects that: current documents are never
removed. It is also required for correctness of the new field: a rebuild that
overwrote `contained_doc_ids` with only the current ids would lose track of
stale documents forever. Implemented: rebuild pages do the `_mget` and
delete obsolete ids of `Known` containers too. The one difference from sync
pages is a `Legacy` container: a sync page sweeps it once; a rebuild page does
not sweep (a rebuild never did). Consequence: a rebuild or `$reindex`
**without `clearExisting`** writes `contained_doc_ids` and so turns a `Legacy`
container into `Known` **without sweeping** the stale contained documents that
already existed for it. Nobody records those documents any more, so nothing
removes them by id afterwards. They are not visible, though: their `version_id`
no longer equals the container's, and the read-side filter (3.6) drops them. A
rebuild with `clearExisting` (#2278) deletes the search documents before it
writes, so it leaves none behind. (Fallback if the `_mget` cost matters in the
benchmark: do not write the field on rebuild pages.)

### 3.6 Read-side guarantee (version filter)

**Why the list cannot be the guarantee.** `contained_doc_ids` is read, then a
new list is written; between the two another writer can act, and a `_bulk` is
neither atomic nor ordered (3.4). Counterexample, for a container `C` whose
list is `[x]`:

1. Writer A reads `[x]` and sends a `_bulk`: `delete x`, `index y`, `index C`
   with list `[y]`.
2. Elasticsearch applies `index C` (list `[y]`), but the `index y` item is
   rejected with 429 and is resent later, on its own.
3. Writer C reads `C`, sees `[y]`, deletes `y` (404, it does not exist yet) and
   writes `C` with `[]`.
4. A's resend arrives and creates `y`.

Now `y` exists, no container lists it, and no write will ever look for it: an
orphan that nobody registers. The same shape appears with partial `_bulk`
failures and with the `409` retry. Making the list more careful does not close
it; the next reader must not need the list to be right.

**The filter.** In `_contained=true|both` searches (`contained_keys`,
`search_impl.rs`), a contained document stands only if its container exists
(same tenant, not `is_deleted`) and the document's `version_id` equals the
container's current `version_id`. The container's state comes from one
`POST /_mget?realtime=false` per batch of at most 500 containers
(`container_versions`, `storage.rs`), asking `_source` = `version_id`,
`tenant_id`, `is_deleted`, each container once. A document without a
`version_id`, a container that is missing (`found: false`, or its index does
not exist), belongs to another tenant, or is deleted: the document is left
out. The filter runs **before** de-duplication, so under
`_containedType=container` a stale hit cannot hide a current hit of the same
container/key, and the container still appears once.

In the counterexample, `y` was written with the version of the write that A
started; the container is now at the version of C's write, so `y` is a version
behind and never matches. A document left behind by a write that is no longer
the container's current one has a different `version_id`, so it is invisible
without anyone having to know it exists. The list remains useful to delete such
documents cheaply on the next rewrite; it no longer decides what a search
shows.

**What the filter does not give: protection against false negatives.** It
protects against showing what should not be shown, not against hiding what
should be shown. If a *late resend* of an older writer's item (for example an
`index x` of version 2 that was throttled and is resent after version 3 wrote
`x`) overwrites a current child, that child now carries version 2 under a
version-3 container and is hidden until the container is rewritten.

**In-process lock.** Within one HFS process this cannot happen: every write of
a container (`upsert`, `update`, `write_search_entries`, `delete`/`purge`, and
each owner of a sync or rebuild page) takes a lock keyed by
`(tenant, type, id)` before it reads what the container records and keeps it
until its `_bulk`, resends after a `429` included, is done. The locks are a
fixed table of 8192 mutexes selected by hash (two containers may share one and
then only take turns; the table does not grow with the data). A page takes the
locks of its owners in ascending order without repeats, so two pages sharing
owners in opposite order cannot wait for each other. Writes of different
resources do not wait for each other. Tests:
`two_concurrent_updates_of_one_resource_do_not_interleave_even_across_a_429_resend`,
`a_page_and_an_update_of_one_of_its_resources_serialize_without_deadlock`
(both fail with the lock disabled), `concurrent_writes_of_different_resources_are_not_serialized`
and `two_pages_sharing_resources_in_opposite_order_do_not_deadlock` (this one
checks only that there is no deadlock).

**Not covered:** writers in different HFS processes. Closing that needs
`version_type=external_gte` with the numeric version on children and container,
treating a child's 409 as a no-op; follow-up, not part of this change.

### 3.7 Id format and container-type collisions

The contained document's `_id` is
`{ContainedType}_{ContainerType}_{ContainerId}#{local}`, built by
`contained_resource_id(container_type, container_id, local_id)` and
`document_id`. Before, the container's type was not part of it
(`{ContainedType}_{ContainerId}#{local}`), so `Observation/1` and `Condition/1`
that both contain a `Medication` with local id `m` wrote and deleted the same
document `Medication_1#m`: one container overwrote the other's contained
document, and a delete by id for one removed the other's. FHIR type names and
ids contain no `_`, so the pieces stay apart.

Compatibility: ids registered in the old format in an existing
`contained_doc_ids` still have the `{Type}_..#..` shape and are still deleted
by id when the container is rewritten or deleted. Old-format documents that no
container records are covered by the sweep (`Legacy`) or hidden by the version
filter. A list whose entries do not have that shape is `Legacy` (3.2).

### 3.8 Orphan collector

`ElasticsearchBackend::collect_orphans(tenant, container_type)`
(`orphans.rs`) physically removes contained documents whose container does not
exist (or is deleted) or has another `version_id`: the same test as the read
filter, applied to delete. It never runs a tenant-wide `_delete_by_query`.

Per page of 500 children (`search_after` over `container_type`, `container_id`,
`resource_type`, `resource_id`, all keyword with doc values): one search under a
permit of the sweep gate (#2203); one `container_versions` lookup for the
distinct containers; for the suspects, the locks of their containers (3.6) and
then a permit, a real-time `_mget` of those containers, and one `_bulk` that
deletes by id only the children that are still orphans. The real-time re-read
under the lock is what keeps a child that a running write has just put in
place before its container, because the container is written last.

It is hooked at the end of a completed whole-type `$reindex` run without
`clearExisting`, `clear_only`, `metadata_only`, named ids or an id range
(`ReindexTarget::collect_contained_orphans`, after the `sweep_orphans` of
#2278, which covers resources the source no longer holds; this covers children
of containers that are alive). A failure is logged and never fails the run.
It is idempotent. Limits: writers of other processes do not take the lock (the
effect is a child hidden until the next write, as in 3.6); `search_after`
without a point in time can skip or repeat a document, which the next run
repairs; it walks every child of each reindexed type.

## 4. The sweep as a repair path

`delete_contained_docs` and the #2203 semaphore stay unchanged. It runs only
when:

| Trigger | Why |
|---|---|
| `Legacy` container | Written before v3; swept once, then the field exists. |
| `Absent` or `Legacy` container on **delete** | No usable ids to read; keeps `delete_of_an_absent_document_still_sweeps_its_contained_documents`, a repeated delete, and the exhausted-sweep test. An `Absent` container is **not** swept on `update`/`upsert`: there is nothing recorded to clean. |
| Unreadable or malformed list | Classified `Legacy` (3.2). |
| 409 on the container (`if_seq_no`) | Concurrent write; sweep, then rewrite without the check. If that second send fails permanently, repair as in the next row; a transient failure is only reported. |
| **Permanently** failed `_bulk` item of an owner that had recorded ids the write no longer lists | `_bulk` is not atomic: the container may list documents that were deleted, or omit ones that still exist. Sweep best-effort, then return the error. **Never after a transient failure** (503, exhausted 429, timeout): the cluster is not answering, each sweep would wait out its retries while a sync page is held (#1749), and under saturation it would add a scroll-context sweep per failed write, the failure mode of #2180 E-01. The retry rewrites the state, and the filter hides the documents meanwhile. |
| Sweep finds the cluster unavailable | A page stops repairing its remaining owners, and the container is not rewritten without its list (that write would fail too). |
| Sweep fails for another reason | Try to rewrite the container without `contained_doc_ids`, so the next write sees `Legacy` and sweeps; return the error. |
| `purge` | Always sweeps after `delete`, also for a container whose list is `[]` or that no longer exists: a purge must leave nothing, and a child the list does not name would come back visible if the resource is recreated under the same version (a primary that restarts its versions after a purge). |
| `delete` of a container with recorded ids whose `_bulk` fails permanently | Sweeps once before returning the error. After a transient failure it does not: a repeated delete finds the container absent and sweeps. |

The repair is clean-up, not correctness. A contained document the container no
longer lists was written by a write that is not the container's current one, so
it carries another `version_id` and the filter (3.6) keeps it out of search
results.

Even if every repair fails, the read-side filter (3.6) keeps what the repair
would have removed out of search results.

Without the unlist step, a failed delete of an obsolete id after the container
got the new list would leave that document unrecorded: the retry
(`sync_failures.rs:615` -> `write_search_entries`) would read the new list and
never delete it. The remaining window (ES unreachable for the `_bulk`, the
sweep and the strip) is stated here as a known limit; the write returns an
error and the sync-failure repair path retries.

## 5. Semantics changes

- **Refresh.** The sweep forces `refresh=true`. Delete-by-id uses
  `write_refresh_param()`, like the rest of the write. With
  `HFS_ELASTICSEARCH_WRITE_REFRESH=false` an obsolete contained document stays
  visible until the next refresh. Documented, not worked around.
- **Mapping reconcile failure.** `reconcile_index` logs and does not block. If
  adding `contained_doc_ids` fails, ES maps the field dynamically as an
  indexed keyword (with doc values). That is only a storage/write-cost
  overhead: the field is never queried, sorted or returned in `content`, and
  the read/write logic is identical. Accepted risk, no dedicated test (a test
  would only assert ES's dynamic-mapping default); the next successful
  reconcile does not change an existing dynamic mapping, so a `$reindex` is
  the way to fix such an index.
- **Error text.** A failed write says it could not bring the container and its
  contained documents in step and asks to "repeat the write or reindex the
  resource" (a repeat repairs it; a reindex rewrites it). Neither is needed for
  a search to be right: stale documents are hidden by the version filter.
- **Mixed versions.** Old binaries ignore the field (they do not read it) and
  keep sweeping; a document they rewrite loses the field (they do not write
  it), becomes `Legacy`, and is swept once by a new binary.

## 6. Cost

- **Write:** removes the tenant-wide `_delete_by_query` (refresh and scroll
  contexts) for resources without `contained`; adds one real-time GET per
  update/delete (per page: one `_mget`); contained documents move from one call
  each into the same `_bulk`.
- **Storage:** about 30 bytes per contained document in the container's
  `_source`; no index or doc values. Resources without `contained` store `[]`.
- **Reads/search:** plain searches are unaffected (3.1). `_contained=true|both`
  searches pay one extra `POST /_mget?realtime=false` per 500 distinct
  containers of the hits, after the enumeration; to be measured in the search
  leg of the benchmark (section 8).

## 7. Tests (summary)

- No `_delete_by_query` on update/delete of a resource without `contained`.
- `Legacy` container sweeps once; 409 and item failure trigger repair and an
  error.
- Bundle update dropping a contained entry leaves no stale document.
- Kept green: E-10 (`:639`), lost sweep (`:776`), exhausted sweep (`:709`),
  `sweeps_in_flight_are_limited`, `a_throttled_sweep_is_repeated_alone`.
- Read-side filter (wiremock, `elasticsearch_search_wiremock.rs`): a hit whose
  `version_id` differs from the container's is left out; a missing, foreign-
  tenant or deleted container, and a hit without `version_id`, leave their
  documents out; under `_containedType=container` a stale hit does not hide a
  current one and the container appears once; the lookup is one
  `POST /_mget?realtime=false` naming each container once.
- Reviewed on purpose: `:900` and `:987` of
  `elasticsearch_storage_write_wiremock.rs`.

### Tests fail without the change

The branch's four ES test files, run unchanged against `1384699f6`
(`--features R4,elasticsearch,sqlite`): real ES 7 of 9 fail (2 pass) —
including `updates_and_deletes_run_no_sweep_and_leave_no_stale_contained_documents`
("an update of a resource without contained[] runs no sweep") and
`a_bundle_update_that_drops_a_contained_entry_leaves_no_stale_document`;
bulk wiremock 2 of 24 fail; storage-write wiremock 10 of 33 fail; the
sync-creates-timeout file passes. All pass on the branch.

## 8. Benchmark (A/B, bounded)

The owner asked for a bounded A/B on this machine, not the full ABAB of #2180.
One A then one B, the same host, ES container and corpus.

- Arm A: `origin/main` `5efe73801` (`hfs-es2291-mainA`), which the branch
  already contains. Arm B: this branch at `1dc6fa974` (`hfs-es2291-v2`), i.e.
  with the read-side version filter, the new id format and the wider repair.
- `sqlite-es`, `HFS_COMPOSITE_SYNC_MODE=synchronous`, `WRITE_REFRESH=wait_for`,
  `REFRESH_INTERVAL=200ms`; suites `insert,crud,search` in one run per arm,
  `BENCH_CAP_S=98` (about 33 s per suite), 300 VUs for crud; run under
  `measurelock.sh`. Host load at the start of A was 2.1; at the start of B it
  was 13 (something else started during A), so B ran under more load than A.
  All six suites `complete`, 0 k6 errors.

| leg | metric | A (main) | B (branch) | change |
|---|---|---|---|---|
| insert | req/s | 262.3 | 261.1 | -0.5 % (noise) |
| insert | p50 / p95 ms | 214.8 / 233.3 | 218.6 / 232.1 | +1.8 % / -0.5 % |
| crud | req/s | 1438.4 | 2042.9 | **+42.0 % better** |
| crud | p50 / p95 / p99 ms | 177.8 / 498.0 / 687.4 | 141.3 / 293.2 / 356.5 | -20.5 % / -41.1 % / -48.1 % |
| crud | ES CPU avg (cores) | 3.68 | 1.81 | -50.7 % |
| search | req/s | 5570.9 | 5860.2 | +5.2 % (about the noise level) |
| search | p50 / p95 / p99 ms | 4.42 / 10.73 / 15.92 | 4.35 / 10.15 / 13.76 | -1.6 % / -5.4 % / -13.6 % |

Memory: HFS RSS peak crud +14.7 % (184 to 211 MB), ES memory peak +6 % to +16 %
(B ran second on the same ES container, so it carries the index data of A's
run; not attributed to the change).

**What this supports.** The targeted path (update/delete without `contained`
no longer runs the tenant-wide `_delete_by_query`) improves clearly: CRUD
throughput up, tail latency down, ES CPU halved, in line with the earlier
rounds (+40 to +86 %). Insert did not change; search did not worsen.

**What it does not show.**
- One pair, about 25 to 33 s per leg, no spread between repeated runs, and B
  ran under more host load than A. The direction is large for crud, but the
  ABAB "each B better than each A" rule was not applied; insert and search
  differences are inside run-to-run noise (about 5 %).
- The k6 `search` workload does not issue `_contained=true|both` searches, so
  the cost of the extra `_mget` of 3.6 is **not measured here**. It is paid
  only by `_contained` searches (one `_mget` of the distinct containers per
  hit batch, up to `max_result_window`), not by ordinary reads or writes.
- 429 and WARN counts were not collected for this pair.

### 8.1 Second round, after the review fixes and the merge of main

Same method (one A then one B per pair, no repeats of the same arm), now on the
code of the branch after the review fixes: arm A `origin/main` `5ec98f887`
(`hfs-es2291-mainA2`), arm B branch `ffea86c8f` (`hfs-es2291-v3`). Two pairs:
pair 1 with `insert,crud,search`, pair 2 with `crud` only. All runs
`complete`, 0 k6 errors. **Host load was high and uneven** (load average 5 to
19 at the start of the runs: other sessions were working on this machine), so
read the numbers as direction, not as exact sizes.

| leg | metric | A (main) | B (branch) | change |
|---|---|---|---|---|
| crud, pair 1 | req/s | 1290.2 | 2018.7 | +56.5 % |
| crud, pair 1 | p50 / p95 / p99 ms | 212.4 / 635.2 / 793.4 | 135.5 / 307.1 / 400.5 | -36.2 % / -51.6 % / -49.5 % |
| crud, pair 2 | req/s | 1528.1 | 2243.6 | +46.8 % |
| crud, pair 2 | p50 / p95 / p99 ms | 185.7 / 472.2 / 658.1 | 139.5 / 269.5 / 319.6 | -24.9 % / -42.9 % / -51.4 % |
| crud | ES CPU avg (cores), pair 1 / 2 | 3.66 / 3.86 | 2.15 / 1.95 | -41.5 % / -49.4 % |
| insert, pair 1 | req/s | 259.4 | 259.9 | +0.2 % |
| insert, pair 1 | p50 / p95 ms | 214.8 / 241.6 | 219.0 / 232.2 | +2.0 % / -3.9 % |
| search, pair 1 | req/s | 4884.3 | 6441.3 | +31.9 % |
| search, pair 1 | p50 / p95 / p99 ms | 5.06 / 11.93 / 18.58 | 4.00 / 9.03 / 12.89 | -20.9 % / -24.3 % / -30.6 % |

Together with the first round (+42.0 % crud req/s), crud improved in all three
pairs (+42.0 %, +56.5 %, +46.8 %; p95 -41.1 %, -51.6 %, -42.9 %; ES CPU
roughly halved). That is the path this change targets, and in every pair B is
better than A.

- **Insert** is unchanged in both rounds (within about 2 %).
- **Search** is not worse. The +31.9 % in pair 1 is not credited to the change:
  ordinary searches are not touched by it, and A's search ran while the host
  load was 19, so it is host noise; the first round saw +5.2 %.
- **Memory:** HFS RSS peak is 7 to 15 % higher in B in crud and search (for
  example 192.8 to 210.3 MB); it was +14.7 % in round 1 as well. It is
  consistent but small in absolute terms, and not explained here.
- **Not measured, still:** the extra `_mget` of `_contained` searches (the k6
  workload issues none), and the 429 / WARN counts. The spread between repeated
  runs of the *same* arm was not collected; each arm ran twice only for crud
  (A: 1290 and 1528; B: 2019 and 2244 req/s), and the ranges do not overlap.

### 8.2 Cost of `_contained` searches (the path the filter adds work to)

The k6 workload issues no `_contained` searches, so a direct measurement was
made (script and raw results: `~/tmp/g-2291/contained-bench/`, not in the
repository). Corpus: 3000 `Observation`, each with two contained resources (a
`Medication` and an `Observation`); the searches return 600 (`Medication`) or
500 (`Observation`) hits. Sequential requests, 30 warm-up and 300 measured per
variant, `sqlite-es`, `i3es2` settings, under `measurelock.sh`, two passes of
each arm (load average 1.3 to 2.9 at the start; other sessions were active).
Every arm returned the same number of results (600, 500 and 429 for the
control).

p50 / p95 in ms, the two passes of each arm side by side:

| variant | A: main | B: first version (every container checked) | C: final (lazy check) |
|---|---|---|---|
| `Medication`, `_count=10` | 8.7/10.7 and 8.4/10.0 | 20.4/24.6 and 30.4/47.9 | 10.2/12.1 and 10.1/12.1 |
| `Medication`, `_count=100` | 21.1/28.6 and 20.3/27.1 | 32.3/42.3 and 41.7/54.3 | 25.2/30.9 and 25.0/29.7 |
| `Medication`, `_containedType=contained`, `_count=10` | 9.2/13.2 and 7.5/9.1 | 18.0/23.1 and 19.6/26.4 | 9.1/10.4 and 9.4/11.5 |
| `Medication`, `_containedType=contained`, `_count=100` | 8.7/12.5 and 9.0/10.6 | 18.9/24.7 and 22.0/29.5 | 13.1/15.2 and 13.3/16.0 |
| `Observation` as container, `_count=10` | 9.0/14.2 and 8.7/10.9 | 18.8/23.1 and 20.9/28.2 | 10.1/11.7 and 10.5/13.0 |
| `Observation` as container, `_count=100` | 20.8/28.1 and 20.7/27.1 | 31.3/39.1 and 50.7/100.1 | 24.5/30.1 and 25.5/29.5 |
| control, no `_contained`, `_count=10` | 1.0/1.4 and 1.1/1.5 | 1.1/1.5 and 1.0/1.4 | 1.0/1.3 and 1.0/1.1 |

- The first version (B) checked the container of every hit before cutting the
  page: `_contained` searches were 50 % to 260 % slower than main in p50 (10 to
  20 ms more). That was too much for a read path.
- The final version (C) checks containers in steps, in order, and stops when
  `offset + count` entries are found (without `_total`; with `_total` it
  checks all, because it must count). Against main the p50 is now +12 % to
  +26 % with `_count=10` (about +1 to +2 ms) and +18 % to +50 % with
  `_count=100` (about +4 to +5 ms), the same in both passes. It is not free:
  that is the price of the guarantee, and only `_contained` searches pay it.
  Searches without `_contained` are unchanged (control within noise).
- A `_total` request, and `search_count`, still look up every container; they
  were not measured separately here and keep the cost of B.

### 8.3 429 and WARN counts

From the k6 runs of 8.1 (host log of each run): none of the four runs of the
branch has a WARN, an ERROR, a 429, a `too_many_scroll_contexts` or a
"Secondary sync failed" line; the runs of main have 143 to 220 WARN lines
(retried sweeps) and up to four lines with a 429.

### 8.4 Final code: crud after the lazy check, the removal of the repair and the lock

Two more A->B pairs, `crud` only, with the code that is proposed for merge:
arm A `origin/main` `5ec98f887` (`hfs-es2291-mainA2`), arm B the branch at
`689c899aa` (`hfs-es2291-v5`). Same host, ES container and settings as 8.1; all
four runs `complete`, 0 k6 errors. The host was busy again between the runs
(load average 7 to 18 at the start of A1, A2 and B2), so read the sizes as
direction.

| | metric | A (main) | B (branch) | change |
|---|---|---|---|---|
| pair 1 | req/s | 1578.5 | 2205.0 | +39.7 % |
| pair 1 | p50 / p95 / p99 ms | 186.4 / 453.2 / 623.5 | 141.7 / 283.3 / 349.8 | -24.0 % / -37.5 % / -43.9 % |
| pair 2 | req/s | 1597.4 | 2185.2 | +36.8 % |
| pair 2 | p50 / p95 / p99 ms | 184.2 / 453.4 / 612.7 | 147.9 / 273.3 / 363.2 | -19.7 % / -39.7 % / -40.7 % |
| pairs 1 / 2 | ES CPU avg (cores) | 3.62 / 3.67 | 2.10 / 1.93 | -42.1 % / -47.4 % |
| pairs 1 / 2 | HFS CPU avg (cores) | 1.06 / 1.07 | 1.26 / 1.27 | +19.6 % / +18.2 % |
| pairs 1 / 2 | HFS RSS peak MB | 211.8 / 201.0 | 224.1 / 230.2 | +5.8 % / +14.5 % |

Together with 8.1 and the first round of 8.2's predecessor, crud improved in
all five pairs measured on the branch (+36.8 % to +56.5 % req/s; p95 -37.5 % to
-51.6 %), the in-process lock did not take that away, and ES CPU is about half.
HFS CPU and RSS are higher in B (it does more of the work that ES did), as in
the earlier rounds. Logs: B has no WARN, ERROR, 429 or "Secondary sync failed"
line in either run; A has 166 and 144 WARN lines and 3 and 1 lines with a 429.

Not re-measured with this final code: insert, search and the `_contained`
searches of 8.2 (the lock and the removal of the repair do not touch the read
path; the lazy check of 8.2 is what the last `_contained` numbers measure).

## 9. Residual risks and follow-ups

- **Hidden garbage.** The filter hides orphaned and stale contained
  documents; it does not remove them. A whole-type `$reindex` without
  `clearExisting` now ends with the orphan collector (3.8), which removes them.
  Between reindexes they take disk and count in index statistics.
- **Brief false negatives.** The container lookup is `realtime=false`, so it
  sees the container as of the last refresh. For less than one
  `refresh_interval` after a write, a freshly written contained document can
  be hidden (its container, as searchable, is still the previous version) or
  a deleted one shown. The enumeration `_search` has the same horizon, so the
  two are normally consistent; the case is a document that refreshed before its
  container did.
- **Same version, different content.** A seed that reaches ES without the
  primary's metadata (no `meta.versionId`) can repeat a `version_id` for
  different content; contained documents of the older content would then pass
  the filter. The primary's versions are what the filter relies on.
- **`WRITE_REFRESH=false`.** Delete-by-id and the repair sweeps do not force a
  refresh (section 5), so removal is visible after the next refresh; the filter
  is what hides stale documents in the meantime.
- **`index_new` / `index_many`** only fill the field and do not read the prior
  state: a create over an existing document (a re-sent create, a replay) does
  not delete documents the previous incarnation recorded; the filter hides
  them.
- **Cost of the container lookup** on `_contained` searches: measured in 8.2.
- **Writers in different processes racing on one resource** can leave a current
  child hidden (see 3.6); the in-process lock covers one process only.
  Follow-up: external versioning of children and container.
- **A transient failure leaves recorded stale ids unswept.** If a delete of an
  obsolete id fails transiently after the container got the new list, no
  repair runs (4). The document has the previous version, so the filter hides
  it, unless the version did not change, which for a changed `contained`
  array means a primary that repeats a `version_id` for different content.
- **Mapping.** If the additive reconcile of `contained_doc_ids` fails
  transiently (`reconcile_index` only logs), the first document carrying the
  field makes Elasticsearch map it dynamically (`text` + `keyword`), and the
  later `keyword`/`index:false` mapping is rejected as a conflict, leaving the
  index at the old schema version and rejecting future schema bumps. Reads are
  not affected (the field is read from `_source`). A `dynamic_templates` entry
  was tried and removed: it travels in the same `PUT _mapping` as the explicit
  property, so it does not reach an index whose reconcile failed and protects
  nothing. The remedy is not to write the field into an index not confirmed at
  schema 3 (treat it as `Legacy`), or a `$reindex`; neither is done here.
- **Standalone Elasticsearch without a primary** (library use, tests; no mode
  of the server binary): versions are numbered by the secondary. A write whose
  container item is rejected permanently leaves its children at the next
  version; a retry with different contained content computes the same version
  and does not delete the first attempt's extra child, which then matches the
  container's version and shows. With a primary, versions are unique per
  content and the children are hidden.
- **A closed container index** is treated like a missing one by the lookup:
  `_contained` searches silently return fewer results (200) instead of failing.
  It follows the rule for `index_not_found`, but a closed index is not an
  absence the cluster states; kept, and noted here.
- **Migration.** Contained documents written by releases where `update` did
  not re-index children (#1382) may carry a `version_id` different from their
  container's and are now hidden, where before they were shown. A
  `$reindex` of the affected types restores them; not verified against real
  data.
- **Cost and fragility of the container lookup.** `container_versions` issues
  one `_mget` per 500 containers, in sequence, before a page is cut, also for
  a small `_count`; an item error other than a missing index (a closed index)
  fails the search. To be measured, and parallelized if it shows.
