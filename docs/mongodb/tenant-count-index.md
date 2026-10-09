# MongoDB tenant count index (`idx_resources_live_tenant`)

The cross-tenant resource count behind the Tenants UI, `GET /admin/tenants`, the existence check of `DELETE /admin/tenants/{id}` and the console tenant metrics (`count_by_tenant`) is one aggregate over `resources`:

    [{ $match: { is_deleted: false } }, { $group: { _id: "$tenant_id", n: { $sum: 1 } } }]

Since schema v12 (#1910) the `resources` collection has an index for it:

    { is_deleted: 1, tenant_id: 1 }   name: idx_resources_live_tenant

`is_deleted` comes first, so `{is_deleted: false}` is an index bound and deleted resources are skipped without being read. `tenant_id` is the only other field the aggregate needs, so `$group` reads it from the index key. The plan is a covered index scan (`PROJECTION_COVERED <- IXSCAN`, `docsExamined: 0`) whose cost grows with the number of live resources, not with the bytes of the collection. Before v12 no `resources` index led with `is_deleted` and the plan was a `COLLSCAN` that read every document, deleted ones included, in full.

The aggregate does not hint the index. While it is missing or still building, the planner falls back to the collection scan, still bounded by `HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS`, so an upgrade never turns the count into a hint error.

## Measurements

Local `mongo:5.0.6` single-node replica set, 256 MB WiredTiger cache, Apple M1. The fixture has 611,035 resource documents (545,804 live, 65,231 soft-deleted) in seven tenants, 2.2 KB average (1.26 GB uncompressed, 392 MB on disk). Medians of five runs.

| Plan | Keys examined | Docs examined | Latency | Read into cache |
|---|---:|---:|---:|---:|
| Before: `COLLSCAN` | 0 | 611,035 | 583 ms | 1,280 MB per run |
| After: covered `IXSCAN` on `idx_resources_live_tenant` | 545,804 | 0 | 258 ms | 0 MB |

The plans come from `explain("executionStats")` of the exact pipeline. Before, with the backend's other three `resources` indexes only:

    winningPlan: PROJECTION_SIMPLE <- COLLSCAN
    totalKeysExamined: 0   totalDocsExamined: 611035   nReturned: 545804

After, with `idx_resources_live_tenant` (no rejected plans; the hinted run is identical):

    winningPlan: PROJECTION_COVERED <- IXSCAN(idx_resources_live_tenant)  { is_deleted: 1, tenant_id: 1 }
    totalKeysExamined: 545804   totalDocsExamined: 0   nReturned: 545804

The collection is five times the cache, so the scan before re-read the whole collection into the cache on every run, evicting the working set of other requests. The index is 2.7 MB at this size and stays in cache.

Alternatives measured on the same fixture:

- A partial `{tenant_id: 1}` index with `is_deleted: false` is not used by the aggregate (it has no `tenant_id` predicate). Hinted, it needs a `FETCH` per live resource to re-check `is_deleted` (545,804 documents, 709 ms), slower than the collection scan.
- Discovering tenants (`distinct`) and counting each one needs no new index at all: on the existing `idx_resources_type_scan` (`COUNT <- IXSCAN`, 0 documents examined) it took 235 ms with seven tenants, close to the chosen index's 258 ms. With this index the per-tenant counts become a `COUNT_SCAN` and the whole took 95 ms (71 ms with the raw `count` command). Against that, the index-free variant avoids the boot-time build and the bulk-insert cost below; the price is N + 1 commands instead of one, and the one budgeted command (`HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS`, #1828) would need a client-side deadline across all of them. Its cost also grows with the number of tenants: with this index it was slower than the single aggregate at 2,000 tenants (365 ms against 268 ms; 123 ms against 264 ms at 200). Those tenant-count runs used the `mongo` shell inside the database container over loopback, so they include no network latency; every tenant adds a real round trip in HFS, so they are a lower bound for this approach. Kept as a possible follow-up, not adopted.

The other tenant-scoped queries that filter on `is_deleted` without a resource type keep their plans with the index visible or hidden (per-type counts and the type list stay on `idx_resources_type_scan`). The one that changes is a tenant's total live count, which becomes a `COUNT_SCAN` of this index (136 ms to 47 ms for the 268k-resource tenant).

Write cost of the extra index (one more B-tree key per resource; a soft delete moves the key from the `false` to the `true` range). Arms interleaved on a fresh collection with the other three `resources` indexes, medians of three rounds, run twice with the arm order swapped:

| Write | Without | With | Change |
|---|---:|---:|---:|
| `insertMany` of 500 (bulk ingest shape), docs/s | 23,923 / 23,622 | 23,050 / 23,033 | −3.6 % / −2.5 % |
| `insertOne` (create shape), ops/s | 267 / 244 | 240 / 257 | −10.1 % / +5.3 % |
| soft delete (`updateOne` of `is_deleted`), ops/s | 241 / 242 | 241 / 247 | 0 % / +2.1 % |

Bulk inserts pay about 3 %. Single-document writes wait on the journal commit (about 4 ms each here), and their difference follows the arm order rather than the index, so it is within this harness's noise.

The large-store confirmation (about 19M resources) is pending, and the fixture above does not certify the plan or the timings at that size. A linear extrapolation of 258 ms for 546k live keys gives roughly 9 s for 19M live resources from cache, and the index build on the existing collection is estimated at 30 s or more; both are estimates, not measurements. Follow-up on the large store, before relying on these numbers:

1. Build the index on the existing collection with the `mongosh` command under *Rolling it out* and record its wall time and the bytes read into cache (`db.serverStatus().wiredTiger.cache["bytes read into cache"]` before and after).
2. Run `explain("executionStats")` of the pipeline above and check the plan is `PROJECTION_COVERED <- IXSCAN(idx_resources_live_tenant)` with `totalDocsExamined: 0`.
3. Time the aggregate (median of five runs) and its cache reads, warm and after a `mongod` restart, and replace the extrapolation in this paragraph with the measured figures.

## Rolling it out on an existing store

Schema init creates the index at boot, before the server serves, like the other `resources` indexes. On an existing collection that is one index build:

- **Cost:** one scan of the collection plus a sort of the keys. On the fixture above it took 1.0 s and read 1.3 GB into the cache. It scales with the collection's size; budget a full collection read for a large store (minutes from disk is plausible at tens of GB).
- **Blocking:** MongoDB 4.2 and later hold the exclusive collection lock only at the start and end of the build, so reads and writes continue during it. Boot, however, waits for the build to finish.
- **Not deferrable by `HFS_MONGODB_INDEX_BUILD`:** that setting (`background` / `inline` / `off`) only governs the `search_index` indexes. This index is built inline at boot with the other `resources` indexes regardless, so `HFS_MONGODB_INDEX_BUILD=off` does not avoid the collection scan.
- **Replica sets:** on MongoDB 4.4 and later the build commits only when the default commit quorum (all voting, data-bearing members) has built it, so a secondary that is down or lagging keeps boot waiting too.
- **Disk:** the index is small (about 4.5 bytes per document here, thanks to prefix compression on repeated keys).

Because boot waits for the build (on a replica set, for every voting, data-bearing member), keep boot short on a large store by building it in a window of your choosing before deploying; boot then finds it and does nothing:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" --eval \
      'db.resources.createIndex({ is_deleted: 1, tenant_id: 1 }, { name: "idx_resources_live_tenant" })'

Rolling back needs nothing: an older binary ignores the index. Drop it with `db.resources.dropIndex("idx_resources_live_tenant")` to stop paying its write cost.
