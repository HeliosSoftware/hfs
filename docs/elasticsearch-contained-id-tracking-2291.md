# Elasticsearch secondary: track contained document ids (#2291)

Status: design. Base: `origin/main` @ `1384699f6`. Line numbers refer to
`crates/persistence/src/backends/elasticsearch/storage.rs` unless stated.

## 1. Problem

In the Elasticsearch (ES) secondary, a resource's `contained[]` entries are
indexed as separate documents (`is_contained=true`, `container_id`,
`container_type`), each in the index of its own type, with `_id`
`document_id(type, contained_resource_id(container_id, local_id))`, for
example `Medication_Obs1#med1`.

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

The container document gets `contained_doc_ids: ["Medication_Obs1#med1", ...]`,
the ES `_id`s of its contained documents. It is always written, as `[]` when
there are none. The type of each contained document, and so its `_index`, is
derived from the id prefix.

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
  schema v3).
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
3. `Absent` or `Legacy`: run the repair sweep (section 4), then write as above
   without the concurrency check.

A resource without `contained` whose ES document has `[]` produces a
container-only `_bulk` and no `_delete_by_query`.

`delete`: fetch; `Known`: one `_bulk` with a `delete` per contained id and the
container `delete`. `Absent`/`Legacy`: sweep as today. A `delete` item that
answers 404 counts as success.

Creations (`index_new`, `index_many`) only fill the field; no GET.

### 3.4 `_bulk` generalization

`send_bulk_index` (`:2211`) becomes `send_bulk(ops: &[BulkOp])` with
`BulkAction::Index { doc, if_seq_no, if_primary_term }` and
`BulkAction::Delete`. It keeps `chunk_ranges`, the 429 resend, and the split on
timeout/413. Changes:

- Item results are read from `items[i]["index"|"delete"]`.
- `BulkFailure` gains a `conflict` flag (409).
- Ops of one owner stay in order: contained documents and deletes before the
  container, never split so the container lands in an earlier chunk than its
  contained documents. A container that is written must imply its contained
  documents were written.

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
`old - new` to the same `_bulk`. `Legacy`/`Absent` owners get no deletes from
the page; a `Legacy` owner is swept per owner.

This also applies to rebuild pages (`write_search_entries_page`). The comment
at `:2730-2739` says the rebuild must not delete first, because that would
drop the resource from search between delete and write. Deleting only ids that
the resource no longer has respects that: current documents are never
removed. It is also required for correctness of the new field: a rebuild that
overwrote `contained_doc_ids` with only the current ids would lose track of
stale documents forever. Implemented: rebuild pages do the `_mget` and
delete obsolete ids too. The one difference from sync pages is a `Legacy`
container: a sync page sweeps it once, a rebuild page leaves it as it was
(a rebuild never swept), so a stale document of a `Legacy` container is only
removed by the next single write. (Fallback if the `_mget` cost matters in the
benchmark: do not write the field on rebuild pages.)

## 4. The sweep as a repair path

`delete_contained_docs` and the #2203 semaphore stay unchanged. It runs only
when:

| Trigger | Why |
|---|---|
| `Legacy` container | Written before v3; swept once, then the field exists. |
| `Absent` container on update/delete | No ids to read; keeps `delete_of_an_absent_document_still_sweeps_its_contained_documents`, a repeated delete, and the exhausted-sweep test. |
| 409 on the container (`if_seq_no`) | Concurrent write; sweep, then rewrite without the check. |
| Any failed item for an owner | `_bulk` is not atomic. Sweep best-effort, then return the error. |
| Sweep also fails | Try to rewrite the container without `contained_doc_ids`, so the next write sees `Legacy` and sweeps; return the error. |

Without the last step, a failed delete of an obsolete id after the container
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
- **Reads/search:** none expected (3.1); verified by the search leg.

## 7. Tests (summary)

- No `_delete_by_query` on update/delete of a resource without `contained`.
- `Legacy` container sweeps once; 409 and item failure trigger repair and an
  error.
- Bundle update dropping a contained entry leaves no stale document.
- Kept green: E-10 (`:639`), lost sweep (`:776`), exhausted sweep (`:709`),
  `sweeps_in_flight_are_limited`, `a_throttled_sweep_is_repeated_alone`.
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

## 8. Benchmark (ABAB, #2180 rule)

Arm A `1384699f6` (`hfs-es2286-base`), arm B this branch (`hfs-es2291-br`),
`sqlite-es`, `i3es2` settings (`HFS_COMPOSITE_SYNC_MODE=synchronous`,
`WRITE_REFRESH=wait_for`, `REFRESH_INTERVAL=200ms`), k6 `crud`, 300 VUs.

**Status: not conclusive, not valid for merge.** The runs below were taken on a
host shared with other benchmark sessions (load average 15-22 on 8 cores), and
two of the six were cut at the time cap. Insert and search legs have not been
run.

| run | req/s | p50 ms | p95 ms | state |
|---|---|---|---|---|
| A-1 | 913.9 | 275.0 | 924.0 | complete |
| A-2 | 1147.6 | 234.7 | 657.8 | complete |
| A-3 | 527.6 | 305.9 | 1957.3 | partial (cut) |
| B-1 | 1704.1 | 171.2 | 385.2 | partial (cut) |
| B-2 | 1593.4 | 175.0 | 361.3 | complete |
| B-3 | 125.4 | 704.0 | 9677.3 | partial (cut); hfs 0.2 and ES 0.5 cores, i.e. host starvation, 0 errors |

B-1/B-2 are clearly better than A-1/A-2 (+40..+86 % req/s, p95 -45..-61 %), but
B-3 and A-3 were starved by the host and the ABAB rule (every B better than
every A) is not met. The four clean pairs for crud, insert and search must be
re-run on a quiet host before merge; the PR must carry those numbers, with the
429 and WARN counts.
