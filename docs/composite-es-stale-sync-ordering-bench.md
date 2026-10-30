# Bench and verification record for #2288 (composite ES stale-sync ordering)

Companion to `composite-es-stale-sync-ordering.md`. Branch
`feat/2288-composite-stale-syncs-can`, baseline A = `1384699f6`.

## 1. Correctness: `composite_stale_sync_ordering` against real Elasticsearch

Command (debug build, own testcontainers ES 7.17.29):

```
cargo test -p helios-persistence --features elasticsearch --test composite_stale_sync_ordering
```

Result with tA and tB applied: **11 passed, 1 failed** (12 tests, 20 s).

Passing: `late_v1_does_not_replace_v2`, `late_page_does_not_replace_newer_documents`,
`deleted_resource_is_not_resurrected_by_a_late_sync`,
`composite_delete_is_not_undone_by_a_late_sync`,
`composite_update_is_not_overwritten_by_a_late_sync_of_the_old_version`,
`resending_the_same_write_is_idempotent`,
`a_document_written_many_times_still_accepts_the_next_version`,
`recreate_after_delete_is_visible`, and the three contained-document tests.

Failing: `deleted_resource_is_not_resurrected_after_gc_deletes_expired`
(`composite_stale_sync_ordering.rs:452`, `still deleted`: left `Some("1")`,
right `None`).

Cause: a test defect, not a code defect. The test deletes with
`ResourceStorage::delete`, which tA deliberately kept as a physical delete (it is
also what `purge` uses). A physical delete leaves only ES's own deleted-version
memory, which `index.gc_deletes=0s` removes, so the late v1 is accepted. The
composite path does not use that call for versioned deletes: it sends
`tombstone_search_entries(tenant, type, id, "2")`. The same defect means
`deleted_resource_is_not_resurrected_by_a_late_sync` (`:409`) passes only thanks
to the default 60 s `gc_deletes`, so it does not yet prove the tombstone.

Required fix (t2's file, not touched here): in both tests replace
`es.delete(&t, "Patient", "p1")` with
`es.tombstone_search_entries(&t, "Patient", "p1", "2")`. Lines `:466`, `:502`,
`:597` use the same call and should be checked for the same intent.

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
