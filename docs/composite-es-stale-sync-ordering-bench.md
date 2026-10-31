# Bench and verification record for #2288 (composite ES stale-sync ordering)

Companion to `composite-es-stale-sync-ordering.md`. Branch
`feat/2288-composite-stale-syncs-can`, baseline A = `1384699f6`.

## 1. Correctness: `composite_stale_sync_ordering` against real Elasticsearch

Command (debug build, own testcontainers ES 7.17.29):

```
cargo test -p helios-persistence --features elasticsearch --test composite_stale_sync_ordering
```

Result on the branch head: **12 passed, 0 failed** (about 17 s):

```
cargo test -p helios-persistence --features elasticsearch,sqlite --test composite_stale_sync_ordering
```

Covered: `late_v1_does_not_replace_v2`, `late_page_does_not_replace_newer_documents`,
`deleted_resource_is_not_resurrected_by_a_late_sync`,
`deleted_resource_is_not_resurrected_after_gc_deletes_expired` (the tombstone
protects with `index.gc_deletes=0s`, so it does not rely on Elasticsearch's own
delete memory), `composite_delete_is_not_undone_by_a_late_sync`,
`composite_update_is_not_overwritten_by_a_late_sync_of_the_old_version`,
`resending_the_same_write_is_idempotent`,
`a_document_written_many_times_still_accepts_the_next_version` (migration),
`recreate_after_delete_is_visible`, and the three contained-document tests.

History of this run, for the record:

- The two delete tests first used `ResourceStorage::delete`, which stays a
  physical delete (it is what `purge` uses), so they did not exercise the
  tombstone; they now call `tombstone_search_entries(.., "2")`, the call the
  composite makes for a versioned delete.
- `elasticsearch_tests::es_integration_sqlite_es_reindex_writes_only_elasticsearch`
  exposed a regression of the first version of the guard: after a `$reindex`
  with `clearExisting` (delete-by-query) Elasticsearch still remembered a version
  above the one the rebuild writes, so the rebuild's 409 was read as
  "superseded" and the 25 documents were dropped. A refused write is now
  "superseded" only when a realtime GET finds a stored document; otherwise it is
  written again without the guard. Wiremock tests cover the single and the bulk
  path (`a_409_for_an_absent_document_is_written_again_without_the_guard`,
  `a_conflicting_item_with_no_stored_document_is_written_again`).

### Red run on the baseline

The same file run on a detached worktree of `1384699f6` (only change: the three
`tombstone_search_entries(.., "2")` calls became the baseline's physical
`delete`, since that method does not exist there):

```
cargo test -p helios-persistence --features elasticsearch,sqlite --test composite_stale_sync_ordering
```

Result: **9 failed, 3 passed**. Failing on the baseline (the bug):
`late_v1_does_not_replace_v2`, `late_page_does_not_replace_newer_documents`,
`deleted_resource_is_not_resurrected_by_a_late_sync`,
`deleted_resource_is_not_resurrected_after_gc_deletes_expired`,
`composite_delete_is_not_undone_by_a_late_sync`,
`composite_update_is_not_overwritten_by_a_late_sync_of_the_old_version`, and the
three contained-document tests. The three that pass on both are the
non-regression checks (`resending_the_same_write_is_idempotent`,
`a_document_written_many_times_still_accepts_the_next_version`,
`recreate_after_delete_is_visible`).

## 2. Performance (ABAB): not measured

No ABAB numbers exist yet. Nothing below is an estimate.

Why not run here:

- The only documented way to run the crud, insert and search legs on the
  Synthea corpus is the `fhir-benchmark.yml` workflow dispatch. Dispatching it
  is an external action that needs explicit approval, and the benchmark
  baseline run 39657850695 must not be overlapped by a second dispatch.
- A local substitute on this host would not meet the "same host, same corpus"
  evidence rule: the box is shared (8 cores, 23 GB, other worktrees' MongoDB
  containers and builds running), and `.github/scripts/fhir-bench/README.md`
  warns that overlapping jobs can halve crud and search numbers.

Protocol to run once approved (A = `1384699f6`, B = branch head):

1. Build `hfs` for A and B with `--features R4,sqlite,elasticsearch`.
2. Same corpus and ES 8.15 for both. Order A B A B A B A B (four pairs).
3. Legs: crud, insert, search. Modes: `sqlite-elasticsearch` with
   `es_sync_mode` synchronous and asynchronous.
4. Per run record p50, p95, throughput; per pair the % change; across the four
   pairs the spread. `compare_legs.py` builds the tables.
5. Before and after the crud leg: `GET _cat/indices?v&h=index,docs.count,store.size`
   for the tombstone cost (crud deletes create tombstones that stay).
6. Pass criterion (agreed in the plan): no worsening beyond the run-to-run
   spread. Expected: no change, because `external_gte` adds no I/O (the version
   check is on the existing `_id` lookup); extra cost only on writes with
   `contained[]` (one realtime GET) and on delete (a small document instead of
   removal).

Results table (to fill):

| Leg / mode | Metric | A (abs) | B (abs) | Change % | Spread A / B |
|---|---|---|---|---|---|
| crud sync | p50 / p95 / throughput | not measured | not measured | | |
| crud async | p50 / p95 / throughput | not measured | not measured | | |
| insert sync | p50 / p95 / throughput | not measured | not measured | | |
| insert async | p50 / p95 / throughput | not measured | not measured | | |
| search sync | p50 / p95 | not measured | not measured | | |
| search async | p50 / p95 | not measured | not measured | | |

Index size before / after crud leg: not measured.
