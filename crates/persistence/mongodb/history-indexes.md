# MongoDB history indexes (`resource_history`)

Every version of every resource is a row in `resource_history`. Schema init gives the collection these indexes (`ensure_history_indexes`, `crates/persistence/src/backends/mongodb/schema.rs`):

| Name | Key | Unique | Serves (measured plans below) |
|---|---|---|---|
| `idx_history_identity` | `{ tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }` | yes | `vread`, `list_versions`, the covered version-id read and the document fetches of instance history, its count, `purge`, and the uniqueness of one row per version |
| `idx_history_type_updated` | `{ tenant_id: 1, resource_type: 1, last_updated: -1, id: -1 }` | no | type history (`history_type`), `count_deltas_by_bucket`, and the document fetch of instance history with `_since` when the type window is the narrower scan |
| `idx_history_system_updated` | `{ tenant_id: 1, last_updated: -1, resource_type: -1, id: -1 }` | no | system history (`history_system`), `activity_histogram`, `latest_write_marker`, `count_deltas_by_type_and_bucket` |

A fourth index, `idx_history_resource_updated` (`{ tenant_id: 1, resource_type: 1, id: 1, last_updated: -1 }`), was created by every build before #2353 and is dropped at every boot from #2353 on (there is no schema version change).

## Why `idx_history_resource_updated` went away

It had the same equality prefix `{ tenant_id, resource_type, id }` as `idx_history_identity` and added `last_updated: -1` after it. The only readers that filter on all three fields are `vread`, `list_versions`, `history_instance`, `history_instance_count` and `purge`, and each reads one resource's rows. Instance history does not ask the server for an order or a page. Since #2375 `history_instance` (`storage.rs`) reads the resource's version ids with a covered read of `idx_history_identity` (no document), sorts them by number and applies the version cursor in Rust, then fetches the documents of the next `count + 1` versions by `version_id` (more batches while the deleted-version, `_since` and `before` filters drop rows); before #2375 it fetched every matching row of the resource on every page. In both cases the index's `last_updated` order served no sort. None of the readers passes a `hint`. The type, system and dashboard readers have tighter indexes of their own (`idx_history_type_updated`, `idx_history_system_updated`), so the index was never their winning plan.

Every history insert wrote one key into it. That is one key fewer on every create, update and delete, and, inside a transaction Bundle, one key fewer in the dirty cache the transaction holds (the cache budget is what `TooLargeForCache` is about; see the README's cache sizing section).

## Measurements

The plans come from `crates/persistence/tests/mongodb/history_index_plans.rs`, run on a throwaway `mongo:7.0` single-node replica set with its data in RAM. Run it with:

    cargo test -p helios-persistence --features mongodb --test mongodb_tests -- history_index_plans:: --nocapture --test-threads=1

and read the `HISTORY_PLAN` lines. The probe calls each reader through the backend with the database profiler at level 2, rebuilds the recorded `find` or aggregate and runs it under `explain("executionStats")`. `purge` deletes, so it is measured from its own profiler entries, and the whole-type and whole-tenant purges from explain of the `delete` the backend sends. The same database is measured twice, once with `idx_history_resource_updated` and once after dropping it.

The dataset (one tenant): `Patient/hot` with 300 versions, 5,000 other patients with 2 versions each, 1,000 observations, two `Basic` resources with 50 versions each (the purge targets), and a second tenant with 1,000 patients. The tenant holds 11,401 history rows. The probe asserts, for the dropped state:

- `vread`, `list_versions`, instance history (with and without deleted versions, page 2 by version cursor; every command of it) and its count, and `purge` use `idx_history_identity`, with no collection scan and no blocking `SORT`, and read at most the resource's versions plus one key per command;
- the document fetches of instance history with `_since` or `before` use any index but the dropped one, without a collection scan or a blocking `SORT`, and read at most the resource's versions per command;
- (#2375) the first command of every instance-history page uses `idx_history_identity`, examines 0 documents and has no `FETCH` stage (measured as `PROJECTION_COVERED<-IXSCAN`), every later command (a document fetch) examines no more than twice its documents plus two keys, and the documents of a page add up to 51 (`count + 1`) at most, or to the resource's 300 versions for `before`;
- the type, system and dashboard readers and the whole-type and whole-tenant purges have no collection scan, and examine no more keys or docs than with the index present; the sorted finds among them keep their index order (no `SORT` stage);
- with the index present, none of `history_type`, `history_system` and the dashboard readers (`count_deltas_by_bucket`, `count_deltas_by_type_and_bucket`, `activity_histogram`, `latest_write_marker`) names it in its winning plan.

Plans, keys and docs examined, with the index (before) and without it (after):

| Reader | Before: index | Keys / docs | After: index | Keys / docs |
|---|---|---:|---|---:|
| `vread` | identity | 1 / 1 | identity | 1 / 1 |
| `list_versions` | identity | 300 / 300 | identity | 300 / 300 |
| `history_instance` (page 1, page 2 by cursor, without deleted) | identity | 300 / 300 | identity (covered read), then identity | 300 / 0, then 57 / 51 |
| `history_instance_count` | identity (`COUNT_SCAN`) | 301 / 0 | identity (`COUNT_SCAN`) | 301 / 0 |
| `history_instance` with `_since` (51 rows in the window) | `idx_history_resource_updated` | 51 / 51 | identity (covered read), then type_updated | 300 / 0, then 53 / 51 |
| `history_instance` with `_since` older than every row | `idx_history_resource_updated` | 300 / 300 | identity (covered read), then identity | 300 / 0, then 57 / 51 |
| `history_instance` with `before` (50 rows in the window) | `idx_history_resource_updated` | 50 / 50 | identity (covered read), then identity three times | 300 / 0, then 57 / 51, 113 / 102 and 165 / 147 |
| `purge` (count, then delete of 50 versions) | identity | 51 / 0, then 50 / 50 | identity | 51 / 0, then 50 / 50 |
| `purge_all` delete (1,000 rows) | identity | 1,000 / 1,000 | identity | 1,000 / 1,000 |
| `purge_tenant_data` delete (1,000 rows) | identity | 1,000 / 1,000 | identity | 1,000 / 1,000 |
| `history_type` (first page of 10) | type_updated | 11 / 11 | type_updated | 11 / 11 |
| `history_system` (first page of 10) | system_updated | 11 / 11 | system_updated | 11 / 11 |
| `history_type_count`, `history_system_count` | identity (`COUNT_SCAN`) | 10,302 / 0 and 11,402 / 0 | identity (`COUNT_SCAN`) | 10,302 / 0 and 11,352 / 0 |
| `count_deltas_by_bucket` | type_updated | 94 / 94 | type_updated | 94 / 94 |
| `count_deltas_by_type_and_bucket` | system_updated | 95 / 94 | system_updated | 95 / 94 |
| `activity_histogram` | system_updated (covered) | 52 / 0 | system_updated (covered) | 52 / 0 |
| `latest_write_marker` (newest, then recent count) | system_updated (covered) | 1 / 0, then 52 / 0 | system_updated (covered) | 1 / 0, then 52 / 0 |

The Before cells of the `history_instance` rows were measured on the code before #2375, one command each. The After cells are measured with #2375's paging (the probe lists them as `reader#1` for the version-id read and `reader#2`.. for the fetches); on the code before #2375 they read 300 / 300, 53 / 51, 300 / 300 and 300 / 300, one command each (see the section on #2375 below). The #2375 code's own numbers with the index present (the probe's `leg=before` lines on that code) are: page 1, page 2, the page without deleted versions and the wide `_since` 300 / 0 (covered), then 57 / 51 on identity; the narrow `_since` 300 / 0, then 51 / 51 on `idx_history_resource_updated`; `before` 300 / 0, then 50 / 50 three times on `idx_history_resource_updated`. The 50-key difference in `history_system_count` is the 50 rows the before leg's `purge` removed. The bucket counts depend on where the 60 s bucket floor of the probe's `since` falls (between 56 and 106 keys over the probe runs made so far); before and after agree within a run. A first run of the probe on the code before this change gave the same index and the same keys and docs for every reader it covered, apart from the clock-dependent bucket counts. That run did not yet have the `_since`-older-than-every-row reader, which was added afterwards and measured only on the changed code. The probe creates and drops the old index itself, so the product code does not decide either leg.

What changes:

- In the old plans the index served only the time-filtered instance reads (`_since` and `before`). Every other reader, including the unfiltered instance history that REST calls, already chose `idx_history_identity` even with the index present, so for them the drop changes nothing.
- Instance history with `before` read all of the resource's versions after the drop (300 keys and docs here, 50 with the index). With #2375 it still has to look at every version, because its window holds the oldest ones: a covered read of the 300 version ids, then three fetches of 51, 102 and 147 documents (300 documents, 335 keys; 635 keys over four commands). REST never sends `before` (`_at` is parsed and not applied, `crates/rest/src/handlers/history.rs`).
- The document fetch of instance history with `_since` is served either by `idx_history_type_updated` (when the window is narrow: 53 keys here) or by `idx_history_identity` (when it is wide: 57 keys). Both are bounded by one page of documents or by a narrower window scan [inferred: the probe measured only a narrow window and one older than every row]. The drop's trade (a few hundred extra keys on a resource with hundreds of versions that is read with a narrow `_since`) is now the covered read of the resource's version ids, which every page pays: a narrow `_since` costs 300 covered keys plus 53 keys and 51 documents, against 53 keys and 51 documents on the code before #2375. It is not a collection scan or a blocking sort.

Write cost, from the profiler entry of one `create` (`keysInserted` on the `resource_history` insert): 5 keys with the index, 4 without. The 5 are `_id_` and the four named indexes; the 4 are `_id_` and the three that remain.

### Instance history pages (#2375)

Before #2375 `history_instance` issued one `find` on `{ tenant_id, resource_type, id }` plus the history filters, with no sort, limit or projection, and sorted, applied the version cursor and cut the page in Rust, so a 50-row page of a 300-version resource examined 300 keys and 300 documents, on page 1 and on page 2 alike. `version_id` is a decimal string, so a server-side sort or `$lt` on it is lexicographic (`"10"` before `"9"`); a numeric sort needs a new field or index, and #2353 removed an index key from every history insert to save transaction cache. The page now takes three steps and adds no index and no field (`SCHEMA_VERSION` is unchanged):

1. A covered read of the resource's version ids: `find({tenant_id, resource_type, id})` with projection `{ _id: 0, version_id: 1 }`, which `idx_history_identity` answers without examining a document. The deleted-version, `_since` and `before` filters are not part of it (a filter on a field outside the index would force a fetch of every row).
2. In Rust, the ids are ordered by number, then by the stored id, both descending (a non-numeric id counts as 0; #2392), and the version cursor is applied to them: the page starts strictly below the cursor's version id in that order.
3. The documents of the next `count + 1` versions are fetched with `version_id: { $in: [...] }` plus the filters. When the filters drop rows, another batch is fetched: it grows from the last batch's match rate, doubles when a batch matched nothing, and is capped at 1,024 versions. A run of equal numbers is never split between batches. The rows are then sorted in the same order. For canonical ids, the only ones HFS writes, the page, `has_next` and the next cursor are the ones the full read gave. Since #2392 ids that are not canonical numbers (`07`, `c`) page in that total order too, and a cursor that is not exactly `[String(version_id)]` for this instance is `InvalidCursor`.

The page is not a snapshot (the full read was not one either). It relies on history rows being immutable, and on each version's history row being visible no later than the next version's. That holds where a single-resource write commits its `resources` and `resource_history` writes in one transaction (a replica set or a sharded cluster): there, a version written between the two reads is above every cursor of an earlier page and never shows up below it. On a standalone server, and in bulk ingest, the two writes are separate commands, so a slower writer's history row for version `k` can appear after the row for `k + 1`, and a walk whose cursor has already passed `k` skips it, exactly as the full read before #2375 did [inferred from the write order]. A version purged between the two reads is not returned.

Measured with `cargo test -p helios-persistence --features mongodb --test mongodb_tests -- history_index_plans:: --nocapture --test-threads=1` on the same dataset (the lines below are the `after` leg, `idx_history_resource_updated` dropped). On the code before #2375 (this branch's base, #2353 applied), each reader was one command:

```
HISTORY_PLAN leg=after reader=history_instance_all op=query indexes=idx_history_identity chain=FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=300 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_page2 op=query indexes=idx_history_identity chain=FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=300 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_live op=query indexes=idx_history_identity chain=FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=300 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since op=query indexes=idx_history_type_updated chain=FETCH<-IXSCAN(idx_history_type_updated) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, last_updated: -1, id: -1 }" keys=53 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since_wide op=query indexes=idx_history_identity chain=FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=300 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_before op=query indexes=idx_history_identity chain=FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=300 returned=50 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_count op=command indexes=idx_history_identity chain=COUNT_SCAN(idx_history_identity) plan_summary="COUNT_SCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=301 docs=0 returned=300 sort=false collscan=false
```

With #2375, each reader is the version-id read (`#1`) and its fetches (`#2`..):

```
HISTORY_PLAN leg=after reader=history_instance_all#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_all#2 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=57 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_page2#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_page2#2 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=57 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_live#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_live#2 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=57 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since#2 op=query indexes=idx_history_type_updated chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_type_updated) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, last_updated: -1, id: -1 }" keys=53 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since_wide#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_since_wide#2 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=57 docs=51 returned=51 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_before#1 op=query indexes=idx_history_identity chain=PROJECTION_COVERED<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=300 docs=0 returned=300 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_before#2 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=57 docs=51 returned=0 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_before#3 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=113 docs=102 returned=0 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_before#4 op=query indexes=idx_history_identity chain=PROJECTION_SIMPLE<-FETCH<-IXSCAN(idx_history_identity) plan_summary="IXSCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=165 docs=147 returned=50 sort=false collscan=false
HISTORY_PLAN leg=after reader=history_instance_count op=command indexes=idx_history_identity chain=COUNT_SCAN(idx_history_identity) plan_summary="COUNT_SCAN { tenant_id: 1, resource_type: 1, id: 1, version_id: 1 }" keys=301 docs=0 returned=300 sort=false collscan=false
```

| Reader | Before #2375: keys / docs | #2375: keys / docs (commands) |
|---|---:|---:|
| `history_instance_all`, `_page2`, `_live`, `_since_wide` | 300 / 300 | 357 / 51 (300 / 0, then 57 / 51) |
| `history_instance_since` | 53 / 51 | 353 / 51 (300 / 0, then 53 / 51) |
| `history_instance_before` | 300 / 300 | 635 / 300 (300 / 0, then 57 / 51, 113 / 102 and 165 / 147) |
| `history_instance_count` | 301 / 0 | 301 / 0 |

What this buys and what it does not:

- Documents: a page examines `count + 1` documents (51 here) instead of the resource's versions, except where a filter drops the newest versions (`before`: all 300 here, in batches of 51, 102 and 147).
- Keys: they stay proportional to the resource's versions, because step 1 reads every version id of the resource. That read is covered (no document, no fetch stage, no sort), and each fetch examines a few keys more than documents (57 keys for 51 documents; [inferred] the seeks between the `$in` list's point bounds). A narrow `_since` reads more keys than before (353 against 53 here) and the same 51 documents.
- Commands: a page is two commands, plus one for each further batch when a filter drops rows (four for `before` here).
- No hint is passed. The planner chose `idx_history_identity` for the version-id read and for the unfiltered fetches, and `idx_history_type_updated` for the narrow `_since` fetch, as it did before. [inferred] The planner's choice for other window sizes was not measured.
- A data-less history row now fails a page only when that page's fetch reaches it, as in type and system history since #1053.

The equivalence of every page with the old algorithm (resources of 1 to 121 versions, gaps, deleted versions, `_since` and `before` windows, cursors taken from page N, non-numeric version ids) and the document counts are tested in `crates/persistence/tests/mongodb/instance_history_paging.rs`. Two tests there cover the rest of the contract: one walks the same Patient id in two tenants of one database and checks that each walk returns only its own tenant's versions, and one walks a 2,200-version resource so that the 1,024-version batch cap is reached (a `before` window at the old end, whose fetches double up to the cap, and a page of 1,100 rows, whose 1,101 candidates are fetched as 1,024 and 77).

### Ceiling harness (#2252)

`crates/persistence/tests/mongodb/v1_history_ceiling.rs`, run on `mongo:7.0.43` with a 0.5 GB WiredTiger cache, the offloaded shape, the `resource_history` variant only and 2 repeats:

    HFS_TEST_V1_CEILING_WT_CACHE_GB=0.5 HFS_TEST_V1_CEILING_SHAPE=offloaded \
    HFS_TEST_V1_CEILING_VARIANTS=resource_history HFS_TEST_V1_CEILING_REPEATS=2 \
    cargo test -p helios-persistence --features mongodb --test mongodb_tests -- \
      --ignored --nocapture --test-threads=1 --exact v1_history_ceiling::v1_history_copy_ceiling

Before (schema with the index; `history_indexes` of `V1_CEILING_SETUP` lists `idx_history_resource_updated`):

    V1_CEILING_RUN wt_cache_gb=0.5 shape=offloaded variant=resource_history run=1/2 outcome=too_large code=388 phase=resource_history entries=8600 docs=17300 json_bytes=11535756 payload_bson_bytes=12383625 doc_bson_bytes=29008855 dirty_start_bytes=0 dirty_bytes_last=53637426 update_bytes_last=53628689 secs=15.1 wt_rollbacks=1 loadavg_start=1.72 loadavg=2.43 msg="WiredTigerRecordStore::insertRecord -31800: transaction is too large and will not fit in the storage engine cache"
    V1_CEILING_RUN wt_cache_gb=0.5 shape=offloaded variant=resource_history run=2/2 outcome=too_large code=388 phase=resource_history entries=8600 docs=17300 json_bytes=11535756 payload_bson_bytes=12383625 doc_bson_bytes=29008855 dirty_start_bytes=0 dirty_bytes_last=54101970 update_bytes_last=54052949 secs=13.5 wt_rollbacks=1 loadavg_start=2.24 loadavg=2.41 msg="-31800: transaction is too large and will not fit in the storage engine cache"
    V1_CEILING_SUMMARY wt_cache_gb=0.5 shape=offloaded variant=resource_history runs=2 entries_median=8600 entries_min=8600 entries_max=8600 spread_pct=0.0 json_mb_median=11.54 json_mb_min=11.54 json_mb_max=11.54 dirty_bytes_per_entry_median=6264 outcomes=too_large:2 codes=388:2

After (`history_indexes` lists only `idx_history_identity`, `idx_history_type_updated` and `idx_history_system_updated` besides `_id_`):

    V1_CEILING_RUN wt_cache_gb=0.5 shape=offloaded variant=resource_history run=1/2 outcome=too_large code=388 phase=resources entries=8900 docs=17800 json_bytes=11941711 payload_bson_bytes=12818957 doc_bson_bytes=29847250 dirty_start_bytes=0 dirty_bytes_last=55012180 update_bytes_last=54962624 secs=12.2 wt_rollbacks=1 loadavg_start=1.40 loadavg=1.46 msg="-31800: transaction is too large and will not fit in the storage engine cache"
    V1_CEILING_RUN wt_cache_gb=0.5 shape=offloaded variant=resource_history run=2/2 outcome=too_large code=388 phase=resource_history entries=8900 docs=17900 json_bytes=11941711 payload_bson_bytes=12818957 doc_bson_bytes=30014072 dirty_start_bytes=0 dirty_bytes_last=54755696 update_bytes_last=54685857 secs=13.7 wt_rollbacks=1 loadavg_start=1.34 loadavg=2.47 msg="-31800: transaction is too large and will not fit in the storage engine cache"
    V1_CEILING_SUMMARY wt_cache_gb=0.5 shape=offloaded variant=resource_history runs=2 entries_median=8900 entries_min=8900 entries_max=8900 spread_pct=0.0 json_mb_median=11.94 json_mb_min=11.94 json_mb_max=11.94 dirty_bytes_per_entry_median=6167 outcomes=too_large:2 codes=388:2

| | Before | After | Change |
|---|---:|---:|---:|
| Tracked dirty bytes per entry (median) | 6,264 | 6,167 | -97 (-1.5 %) |
| Ceiling entry (`entries_median`; the transaction that no longer fits) | 8,600 | 8,900 | +300 (+3.5 %) |

The harness adds entries in chunks of 100, so +300 is three chunks. Each side is one harness invocation (its own container) of two repeats, and both repeats of each side stopped at the same entry (spread 0.0); a second invocation per side was not run, so the run-to-run noise between invocations is not measured here. Both before runs overflowed in `phase=resource_history`; on the after side, run 1 overflowed in `phase=resources` and run 2 in `phase=resource_history`. The measured effect is the per-entry figure, 97 bytes (-1.5 %). [inferred] That is less than half the 200-250 bytes per index key that the #1976 decision record inferred. The ceiling shift (+300 entries, +3.5 %) is larger than -1.5 % per entry alone would give, and the dirty bytes at the failing entry were higher on the after side (about 54.8-55.0 MB against 53.6-54.1 MB), so treat it as a single-invocation observation, not a measured gain.

## Rolling it out

Schema init (`initialize_schema_async`) drops the index at every boot with `drop_index_if_present`, the helper that also drops the other superseded indexes, and treats "no such index" as success. There is no schema version change (`SCHEMA_VERSION` stays as it was).

- **Every `HFS_MONGODB_INDEX_BUILD` mode:** `background`, `inline` and `off` all drop it. That setting only governs the `search_index` builds; a drop is a catalog change, not a collection scan, and the index that serves the readers (`idx_history_identity`) is already built inline, so there is no window without an index.
- **Cost:** `dropIndexes` takes the exclusive collection lock briefly and waits for transactions open on `resource_history` to finish. On a busy store, or with several nodes booting together, drop it beforehand in a quiet window, then deploy; boot finds nothing to drop:

      mongosh "$HFS_MONGODB_URL" --quiet --eval "db.getSiblingDB('$HFS_MONGODB_DATABASE').resource_history.dropIndex('idx_history_resource_updated')"

- **Rollback:** an older build creates the index again at boot, inline, which is one index build over `resource_history` (a scan of the collection; boot waits for it and `HFS_MONGODB_INDEX_BUILD` does not defer it). Writes pay the extra key again until the next upgrade drops it. The older build is otherwise unaffected; no reader needs the index to be correct. The version is not bumped so that a build with the newer-schema-version guard does not refuse to start on the database after a rollback.
- **`Backend::migrate`:** `migrate_schema_async` runs the index steps only when the recorded version is below `SCHEMA_VERSION`, so on a database already at the current version it does not drop the index. The `hfs` binary calls `init_schema` at every boot, which does.
