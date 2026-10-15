# Terminology in SQL — backend parity plan (#1824 follow-up)

Status: proposal, revision 2, 2026-10-15. Follows PR #2065
(`feat/1824-terminology-in-sql`), whose design is in
`docs/issue-1824-terminology-in-sql-plan.md`. Revision 1 rebuilt Slice B
around the composite's existing visibility contract, deferred the S3 and
relation caches, and corrected the harness interface and the export target
(§8). Revision 2 reworks Slice B's contract and tests: visibility is
established before a job's first resolution rather than on `NotFound`,
covers `ViewDefinition`, is placed in entry preparation and the pre-pass and
never in the transaction executor, the unreachable-HTS scenario is split in
two, and the e2e matrix keeps its consistency settings and gains a focused
terminology run under default policies (§9). Revision 2.1 closes three
call-site gaps in that contract: ordinary batch entries, the direct and
conditional PATCH handlers, and subject resolution ahead of the graph (§10).

## 1. Where we stand

PR #2065 is backend-agnostic by construction. Every storage touchpoint goes
through the persistence traits and nothing in the diff names a backend:

| Touchpoint | Code | Trait call |
|---|---|---|
| Resolve `Type/id` | `references.rs`, `terminology_materialize.rs` | `ResourceStorage::read` |
| Resolve a canonical (`url`, `url\|version`) | `references.rs::candidates` | `SearchProvider::search` on `url` + `version`; `scan_candidates` when there is no search |
| Discover the stored Libraries a written ValueSet must serve (T2) | `terminology_materialize.rs` (`dependants`, bounded by `MAX_LIBRARIES_SCANNED`) | `search` on `Library` |
| Store a materialized expansion / fetched definition | `terminology_materialize.rs::apply_materialization` | `create` / `update` through the normal write path (events, audit, subscriptions) |
| Conditional create pre-search | `create.rs` | `search` with `If-None-Exist` |
| Transaction pre-pass | `bundle_overlay.rs`, `batch.rs` | runs **before** persistence; the only persistence change is `PatchCandidateValidator::prepare_patch_candidate` on the shared `core::transaction` executor used by sqlite, postgres and mongodb |
| `$sql-run` / `$sql-export` | `graph.rs::PlanNode::Relation` | none: rows are flattened once per job and inserted into the in-memory SQLite engine (`sqlquery/engine.rs`); the backend only supplies ViewDefinition rows through its own runner |

Behaviour when a dependency cannot be resolved, which matters for §2:

- On the **write path** a canonical that resolution does not find is treated
  as absent (`Lookup::NotFound` → `Need::Fetch`): fetched from HTS and stored
  when `HFS_TERMINOLOGY_SERVER` is set, otherwise the Library is stored with
  that dependency left alone.
- On the **query path** an unresolved dependency fails planning with `404`
  (`graph.rs`, `RestError::NotFound` over the four dependency kinds).

What is **not** covered: every automated test in the PR runs on in-memory
SQLite (`sof_sqlquery_terminology.rs`, 12 tests; `sof_terminology_materialize.rs`,
26 tests, with a mock HTS). The e2e spec runs on SQLite in `ui-tests.yml`, and
will run on seven backends in the nightly `ui-tests-matrix.yml` once merged.
That matrix runs the ES composites with `HFS_COMPOSITE_SYNC_MODE=synchronous`
and `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`. Those are suite-wide
read-after-write settings that the workflow explains
(`.github/workflows/ui-tests-matrix.yml:376`): ordinary search, the
SearchParameter CRUD spec and the UI rails all create and immediately search.
They stay. The consequence for #1824 is only that the matrix does not
exercise the asynchronous path (§2, risk 1); that needs the Rust ES suite
(Slice A) and a focused terminology e2e run under default policies (Slice E).

Deployable backends (the `ui-tests-matrix` and `inferno-subscription` lists):
`sqlite`, `postgres`, `mongodb`, `s3`, `sqlite-elasticsearch`,
`postgres-elasticsearch`, `mongodb-elasticsearch`, `s3-elasticsearch`.

## 2. Backend-specific risks, ranked

1. **ES composites: search lags the primary, and three of our steps are
   searches (correctness).** A composite's search answers from Elasticsearch.
   It lags the primary on two axes: writes still in the asynchronous sync
   queue, and writes ES has accepted but not refreshed
   (`WriteRefreshPolicy::False` is the default). `wait_for` closes only the
   second axis. The composite already exposes a hook that closes both,
   `CompositeStorage::ensure_writes_visible`
   (`crates/persistence/src/composite/storage.rs:1661`, #1047), and the batch
   handler already calls it before resolving conditional references
   (`batch.rs`). The terminology code does not call it, so under default
   policies:
   - **Write path, canonical resolution.** A ValueSet written by phase B, by
     the T2 transform, by an earlier batch entry, or by the previous request,
     is not yet searchable. The materializer treats it as absent: with HTS
     configured it fetches the definition from HTS and **stores a second
     ValueSet with the same canonical**, tagged `fetched-from-terminology-server`;
     without HTS the Library is stored with the dependency unmaterialized and
     the next `$sql-run` answers `404`. The walk resolves every `depends-on`
     entry through the same lookup before classifying it, so a just-written
     `ViewDefinition` or SQL-view `Library` is "absent" in the same way, and
     with HTS configured it triggers a definition lookup that has no business
     happening for a ViewDefinition.
   - **Write path, T2 discovery.** The Library search that finds the stored
     Libraries a written ValueSet must serve can miss a Library written
     moments earlier, so the ValueSet is stored without an expansion.
   - **Query path, missing.** `$sql-run` resolving a canonical right after
     the write that created it answers `404` until the index catches up.
   - **Both paths, stale.** The lag is not only "missing". Composite search
     returns the body ES holds (`canonical_candidates` keeps `r.content()`
     from the hit), and `select_newest` ranks by the `lastUpdated` ES
     reports. Update an already-searchable ValueSet from membership A to B
     and run the query immediately: resolution **succeeds with A**. Delete
     it and the hit still answers; store a newer version with the same URL
     and the older one still wins. No error is raised, so nothing keyed on
     `NotFound` can catch any of these.
   None of this is deterministic, and none of it is covered by a test.
2. **Standalone S3: canonical resolution is a scan (performance).**
   `scan_candidates` lists every ValueSet / ConceptMap of the tenant and reads
   each one to compare `url`, on every request that resolves a canonical, with
   no cross-request cache. Cost is O(artifacts) GETs per resolution; an
   `$sql-run` against a tenant with thousands of ValueSets will be slow or hit
   the request timeout. Correct, but needs a number before we decide.
3. **S3: no transactions.** `BundleProvider::supports_atomic_transactions`
   is false, a transaction Bundle is `501` before our pre-pass runs. Batch is
   per-entry and works. Needs a test that pins the 501 and that batch
   materialization behaves on S3 like elsewhere.
4. **MongoDB: document size.** The 16 MB BSON limit is above our 8 MiB
   `HFS_SOF_TERMINOLOGY_MAX_ARTIFACT_BYTES`, but BSON overhead on a 200k-row
   `expansion.contains` is not measured. Transactions need a replica set
   (the testcontainers harness in `patient_scope_compartment_mongodb.rs`
   already does that).
5. **ES composites: cost of large expansions in ES, and what the body is
   used for.** The ES document carries the resource twice: raw `content`
   (mapped `enabled: false`, stored and not indexed) and `content_text` (the
   resource's strings, indexed for `_content` search), see
   `backends/elasticsearch/schema.rs`. Composite search returns resources
   built from ES hits (`parse_hit_to_stored_resource`,
   `backends/elasticsearch/search_impl.rs:1432`), and canonical resolution
   reads **those** bodies, not the primary's. So a 100k-member expansion is a
   multi-MB ES document per write, and it cannot simply be dropped from ES:
   that would make the composite's search return ValueSets without their
   expansion, and would change `_content` semantics. Measure, do not
   optimise, in this round.
6. **PostgreSQL: none expected.** Same SQL search path as SQLite, JSONB
   storage; the only question is write latency of an 8 MiB JSONB row under
   `HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS`.

## 3. Work slices

Each slice is one PR. B is the only one that changes runtime behaviour in
this round; the rest are tests, measurements and docs. Caches (S3 canonical,
relation rows) are deferred until correctness is proven and the measurements
say they are needed (§7).

### Slice A — backend-parametric integration tests

Goal: the 38 SQLite scenarios run unchanged on postgres, mongodb and an ES
composite, plus the S3-specific expectations, with **no duplicated test
bodies**: SQLite invokes the same shared scenarios.

Harness. `Backend` (`core/backend.rs`) has an associated `Connection` type
and does not expose CRUD, search or bundles, so it is not the fixture type.
Two workable shapes; pick one at implementation time:

- **Generic builder.** `async fn server<S>(backend: Arc<S>, config: ServerConfig, principal: Option<Principal>) -> TestServer`
  with the same bounds the router puts on `AppState<S>`, so every backend
  that can serve HFS can serve the scenarios.
- **Scenario-facing fixture trait** (preferred if the bounds get long):
  ```
  trait TerminologyFixture {
      async fn server(&self, config: ServerConfig, principal: Option<Principal>) -> TestServer;
      fn kind(&self) -> BackendKind;
      fn supports_transactions(&self) -> bool;   // S3: false → those scenarios assert 501
      fn tenant(&self) -> TenantContext;         // unique per scenario, never shared
  }
  ```
  Scenarios take `&dyn TerminologyFixture`. The `principal` input is what
  the scope scenarios (`materialization_needs_the_callers_value_set_update_scope`)
  need.

Composites must be built the way production builds them: the
`composite_read_your_writes.rs` helper (`CompositeStorage::new(config, backends)`
with `.sync_mode(SyncMode::Asynchronous)` and `.start_sync_workers()`), not a
synchronous shortcut. The ES driver must run with the **default** write
refresh policy, so that §2 risk 1 is reproduced, not hidden.

Files, following `sof_conformance_postgres.rs` / `sof_conformance_mongodb.rs`
/ `patient_scope_compartment_elasticsearch.rs` (one container per suite,
`container_cleanup`, `#![cfg(feature = "...")]`, Docker-gated):

- `crates/rest/tests/common/terminology_scenarios.rs` — the bodies, moved
  out of the two SQLite files, which become thin drivers.
- `sof_terminology_postgres.rs`
- `sof_terminology_mongodb.rs` (replica set, so transaction scenarios run)
- `sof_terminology_sqlite_elasticsearch.rs` (asynchronous sync, default
  refresh)
- `sof_terminology_s3.rs` (MinIO): transaction scenarios expect `501`;
  batch and direct writes expect parity; one scenario resolves a canonical
  among N stored ValueSets to pin the scan path.

CI: these land in the existing Docker-gated `Test Rust` job (it already has
`DOCKER_HOST`). The job is ~1 h today; four suites with one container each
should add well under 10 minutes.

Exit: every scenario green on sqlite, postgres, mongodb; the ES suite is
allowed to be red only on the fourteen visibility scenarios below, which
Slice B fixes.

### Slice B — visibility on ES composites, built on `ensure_writes_visible`

First, the failing tests, on the asynchronous ES driver from Slice A. All
are separate requests unless stated, HTS is the mock from
`sof_terminology_materialize.rs`, mode is strict:

1. **Creation, already-expanded fixture, HTS unreachable.** `POST ValueSet`
   stored **with an `expansion`**, then `POST Library` referencing it by
   canonical: the Library write succeeds and exactly one ValueSet exists.
   With HTS configured, a stored ValueSet without an expansion always goes
   to the server (`expand` only uses the local expander when no HTS is
   configured), so the fixture must carry its expansion for the unreachable
   server to prove anything. A false `NotFound` then becomes a `Need::Fetch`
   against the unreachable server and the write fails, which is a
   deterministic signal instead of "sometimes two ValueSets".
2. **Creation, compose-only fixture, HTS reachable.** `POST ValueSet` with
   an enumerated compose and no expansion, then `POST Library`: the
   expansion succeeds through the mock's `$expand`, and the mock's
   definition-search endpoints (the ValueSet / ConceptMap fetch) are **not
   called**. The assertion is on the mock's request log.
3. **T2.** `POST Library` then `POST ValueSet`: the ValueSet is stored with
   an expansion.
4. **ViewDefinition.** `POST ViewDefinition` then `POST Library` depending
   on it by canonical, before any ES refresh: the write succeeds and the
   mock HTS sees no definition lookup for the ViewDefinition's URL.
5. **Query after create.** `POST ValueSet` then `$sql-run` of a Library
   that references it: rows, not `404`.
6. **Query after update.** ValueSet searchable with membership A; `PUT` it
   to membership B; `$sql-run` immediately: rows come from B.
7. **Query after delete.** `DELETE` the ValueSet; `$sql-run` immediately:
   `404`, not rows from the deleted body.
8. **Query after a newer version.** Store a second ValueSet with the same
   URL and a later `version`; `$sql-run` with the unpinned canonical:
   the newer one is used (`select_newest`).
9. **Transaction boundary (regression assertion).** A transaction Bundle
   with a Library `PATCH` entry, run against a storage wrapper that counts
   `ensure_writes_visible` and search calls: the count taken during
   `process_transaction_with_patch_validator` is zero for the terminology
   path; the only calls happen in the pre-pass. See the boundary rule below.
10. **Ordinary batch, earlier entry.** A `batch` Bundle whose entry 1 creates
   a ValueSet and whose entry 2 is a `POST Library` (and a variant with a
   `PATCH Library`) depending on it by canonical, run with
   `HFS_BATCH_MAX_CONCURRENCY=1` so entry 1 has completed before entry 2 is
   prepared: entry 2 resolves the ValueSet entry 1 stored, one definition
   exists, no HTS definition lookup. Batch entries are independent per the
   spec and run with bounded concurrency in submission order, so the
   guarantee the test pins is "whatever completed before this entry's
   preparation is visible to it", which a single barrier at batch start
   cannot give.
11. **Direct `PATCH Library`.** Store a ValueSet; `PATCH` an existing Library
   to add a `depends-on` entry for it, before any refresh: the patched
   Library is materialized against the stored ValueSet, no duplicate, no
   definition lookup. Same for the conditional form (`PATCH Library?url=`).
12. **Direct `PATCH ValueSet` (T2 discovery).** Store a Library depending on
   a ValueSet; `PATCH` that ValueSet's compose **and remove its existing
   `expansion` in the same patch** (T2 keeps an expansion the candidate
   already carries, `terminology_materialize.rs:1197`, so a patch that
   leaves it in place proves nothing), before any refresh: the stored
   ValueSet carries a fresh expansion because discovery found the Library.
   Same for the conditional form.
13. **Subject resolution, `$sql-run`.** `POST Library` (or `ViewDefinition`)
   then `$sql-run` by `subjectCanonical` immediately: rows, not `404`.
   `PUT` the Library's SQL, then `$sql-run` by `subjectCanonical`: the new
   SQL and its dependencies, not the previous body. Both alongside the
   inline-subject forms of tests 5 to 8.
14. **Subject resolution, `$sql-export`.** The same create and update pair
   through `$sql-export` with `subjectCanonical` subjects: the kick-off
   resolves the stored subject, and the manifest reflects the current body.

Then the contract. `ensure_writes_visible(tenant, types)` drains the
asynchronous sync queue and refreshes the search backend for those types; on
consistent backends it is free (a composite with synchronous sync pays only
the refresh; SQLite, PostgreSQL and MongoDB answer `Ok(())`). Because a
stale hit is a successful read, the call is made **before the first
resolution of a request or job**, never in reaction to a result:

| Step | Where the call goes | Types |
|---|---|---|
| Direct `POST` / `PUT` of a Library or ValueSet | `create.rs` / `update.rs`, in entry preparation immediately before `transform_for_direct_write` / `plan_for_direct_write` | `ValueSet`, `ConceptMap`, `Library`, `ViewDefinition` |
| Direct and conditional `PATCH` | `patch.rs::patch_handler` and `patch.rs::conditional_patch_handler` both run T1/T2 themselves (`transform_for_direct_write` → `plan_for_direct_write` → `apply_for_direct_write`); the call goes after the handler's preconditions (`If-Match`, conditional selection) and before `transform_for_direct_write` | same four |
| Ordinary `batch` Bundle | **per entry**, in `batch.rs::process_batch_entry` and `batch.rs::process_batch_patch`, immediately before each entry's `transform_for_direct_write` / `plan_for_direct_write`. These paths never reach `terminology_pre_pass`, which runs only from `process_transaction`. A single call at batch start would also miss what earlier entries completed, since entries run with bounded concurrency in submission order (`HFS_BATCH_MAX_CONCURRENCY`) | same four |
| Transaction pre-pass | `batch.rs::terminology_pre_pass`, next to the existing `resolve_conditional_references` call, before anything executes; this is where `OverlayThenStorage { live: true }` reads storage and fills the snapshot | same four |
| T2 dependant-Library discovery | covered by the rows above: `dependants` runs from every preparation path after the call | `Library` (included) |
| `$sql-run` | `run.rs`, once per request, **before `resolve_subject`** (`run.rs:197`): the subject itself comes from a canonical search when it is `subjectCanonical`, and it runs before the graph is built, so a barrier in `graph.rs` alone leaves a just-stored Library at `404` and an updated one contributing stale SQL and `depends-on`. The `JobResolver` then starts with the index already current | same four |
| `$sql-export` | `export.rs`, once per kick-off, before the first `resolve_subject` of the subject list (`export.rs:380`) and therefore before the `JobResolver` it builds (`export.rs:333`) is used; one call covers every subject of the job | same four |
| Conditional create pre-search | `create.rs` | out of scope: a general `If-None-Exist` property, already covered by `ConditionalStorage` on the composite (`storage.rs:247`) |

One barrier per request or job, placed before the first stored lookup that
request makes, whichever handler it is; the rows above enumerate every
handler that performs such a lookup today. The types include
`ViewDefinition` on every row because the walk resolves
every `depends-on` canonical through the same lookup before it knows the
kind, and the planner tries all four types in `DEPENDENCY_RESOURCE_TYPES`
order; a ViewDefinition written in the previous request is as invisible as
a ValueSet and, with HTS configured, a miss sends a definition lookup to HTS.

**Boundary: the executor never makes a live visibility call.** The planner
runs a second time while a transaction is open: `prepare_terminology`
(`batch.rs:151`), through `PatchCandidateValidator::prepare_patch_candidate`,
re-plans a `PATCH`ed Library with `OverlayThenStorage { live: false }`, which
answers only from the snapshot the pre-pass took. That is the right design:
the composite delegates the transaction to the primary and queues sync
events only after it commits (`sync_bundle_results`), so there is nothing of
this bundle to drain, and a barrier there would wait on workers while the
primary's write transaction is held. So the barrier is placed in entry
preparation and the pre-pass only, the executor keeps using the prepared
snapshot, and `plan` / `walk` / `dependants` themselves stay free of
visibility calls so the executor-time re-plan cannot acquire one by
accident. Test 9 pins this.

Why not `wait_for` alone: it does not drain the asynchronous sync queue, so
the write may not have reached ES at all. Why not `NotFound`-then-retry: it
answers only the missing case; tests 6 to 8 return a body and no error. Why
not blind retry-with-backoff: it spends the materialization budget on a
race we can close explicitly, and it masks a genuine missing reference.

Cost to state in the PR: one barrier plus one refresh over four types per
Library or ValueSet write and per `$sql-run` / `$sql-export` job on an ES
composite; zero elsewhere. The write path already pays an HTS round trip.
The query path pays a refresh it did not pay before, once per job (not per
dependency, not per export subject); Slice F scenario 2 records it, cold
and repeated.

Exit: the fourteen scenarios green on the asynchronous ES driver. The fix is
composite-generic (`CompositeStorage::ensure_writes_visible` routes to
whichever backend answers the search), so `postgres-es`, `mongodb-es` and
`s3-es` inherit it; their regression ring is the Rust ES suite plus the
focused e2e run of Slice E, not the pinned matrix.

### Slice C — standalone S3 scan cost: measure, then decide

Measure first (Slice F, scenario 4). Then:

- If one resolution stays under ~100 ms at 1,000 artifacts: document the
  behaviour and stop.
- Otherwise the choice is between a cache and a restriction, and **the
  cache is not designed in this round** (§7). A correct one needs: keys that
  carry the requested business version (`url|version`) so a cached `url|1`
  never answers `url|2`; revalidation for an unpinned canonical, which must
  find a newer resource with the same URL, including one created by another
  HFS instance; deletion and replacement handling; and cross-instance
  invalidation. Re-reading a cached id by `Type/id` satisfies none of these.
- The restriction alternative: accept only `Type/id` references on backends
  without search and answer `422` naming the canonical. Visible behaviour
  change; needs sign-off.

### Slice D — Mongo and ES document-size measurements

- Mongo: store a ValueSet at the artifact cap (8 MiB JSON) and at the member
  cap (200k rows) through the write path; assert the write succeeds and the
  read round-trips. Record the BSON size.
- ES composites: measure the two contributions separately, raw `content`
  (stored, not indexed) and `content_text` (indexed), for the same two
  sizes, plus write latency with asynchronous sync. Removing `expansion`
  from the ES document is **not** proposed: composite search returns ES
  bodies, so it would require fetching complete resources from the primary
  and preserving `_content` semantics. If the numbers are bad, that is a
  separate design.

### Slice E — e2e matrix

`sql-libraries.spec.ts` already runs on the seven matrix legs nightly after
merge, and the matrix's `HFS_COMPOSITE_SYNC_MODE=synchronous` and
`HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` on the ES legs **stay as they
are**: the workflow explains them as protection for ordinary search, the
SearchParameter CRUD spec and the UI rails (`ui-tests-matrix.yml:376`), none
of which a terminology barrier fixes.

What Slice E adds is a **focused terminology run under default policies**:
on each ES leg, after the pinned full suite, boot a second HFS with the sync
mode and write refresh left at their defaults and run only
`sql-libraries.spec.ts` against it. That run is the e2e counterpart of the
Rust ES suite: it fails before Slice B and passes after, and it fails again
if someone moves a visibility call. It is a separate step (or matrix entry)
so a red there is attributable to terminology and does not change what the
pinned suite proves.

### Slice F — performance measurement, per backend

A repeatable script plus a results page, following the existing
`docs/*-benchmark.md` convention (for example
`docs/sof-parquet-streaming-benchmark.md`).

Scenarios, each at expansion sizes 1k / 10k / 100k / 200k members:

1. **Library write latency** (`PUT Library`, strict mode, HTS configured):
   HTS `$expand` time versus HFS store time, and on ES composites the
   `ensure_writes_visible` cost from Slice B. Pass: under the 20 s budget at
   200k on every backend; record where the 8 MiB artifact cap trips first.
2. **`$sql-run` latency**: Library joining `Condition.code` against the
   relation over 100k Conditions. Measure cold and repeated calls; the
   resolver is per-job, so repeats should cost the same. On the ES composite
   split out the per-job `ensure_writes_visible` from Slice B (barrier plus
   refresh) so its share of the job is a number. This is the input for the
   relation-cache question in §7.
3. **`$sql-export` with 5 subjects** sharing one ValueSet. What #2065
   shares is dependency resolution and the flattened rows (`JobResolver`,
   `Arc<[Value]>`); each subject still gets a fresh SQLite engine,
   materializes its dependencies into it and executes its SQL. So measure
   **kick-off resolution** separately from **worker execution**, and assert
   one fetch, one flatten and one shared row allocation across subjects. Do
   not expect total latency near a single subject's.
4. **Canonical resolution cost on S3**: `$sql-run` against a tenant holding
   100 / 1,000 / 5,000 ValueSets, canonical reference. This is Slice C's
   input.

Backends: sqlite, postgres, mongodb, sqlite-elasticsearch, s3 (MinIO).
Report write latency, run latency, stored document size, and for the
composite the ES index size delta split into `content` and `content_text`.

### Slice G — docs

- Book `ch06-sql-on-fhir.md`: a "Backend notes" table (search-based
  resolution, S3 scan, S3 transactions `501`, the composite visibility
  guarantee from Slice B and its cost).
- `work-with-sof` and `test-hfs` skills: how to run the per-backend suites.

### Slice A — what the first run found (2026-10-15)

Implemented on `feat/1824-backend-parity-a` (stacked on #2065):
`crates/rest/tests/common/terminology_scenarios.rs` holds the 38 bodies and
the `terminology_suite!` macro; `sof_terminology.rs` (SQLite),
`sof_terminology_postgres.rs`, `sof_terminology_mongodb.rs`,
`sof_terminology_sqlite_elasticsearch.rs` and `sof_terminology_s3.rs` are
the drivers. The harness is a generic `build` with the router's bounds, a
fresh tenant per server, and `Hfs` capability flags the scenarios branch on.

- **PostgreSQL** exposed an ordering race in
  `batch_entries_materialize_independently`, not a terminology defect:
  batch entries run concurrently in submission order, so entry 4 resolved
  the ValueSet entry 3 was still writing. SQLite serialised them by
  accident. The scenario now runs with `batch_max_concurrency: 1`; it tests
  independence of entries, not ordering. Ordered batches are scenario 10 of
  Slice B.
- **Standalone S3**: `If-None-Exist` evaluates `identifier` and `_id` only,
  so a conditional create by `url` is `400` before anything is written; and
  with no pre-search the match is decided inside the create, so phase A runs
  first (D13) and an unreachable terminology server fails the request with
  `422` where SQLite answers `200` from the match. Both are pinned on the
  no-search flag. The second is a documented divergence, left as is: moving
  phase A after the create would leave a Library stored unmaterialized in
  strict mode when phase A then fails.
- **Composites with offloaded search** refuse any conditional entry inside
  a transaction with `501` (`batch.rs`), before the terminology pre-pass
  sees it. Pinned on the `conditional_in_transaction` flag.
- **Asynchronous ES composite**: 30 of 38 green; the 8 below fail for the
  reason §2 risk 1 predicts, are expanded `#[ignore]` with that reason, and
  become Slice B's first failing tests together with scenarios 1 to 14:
  `put_library_expands_its_stored_enumerated_value_set_locally_and_tags_it`
  (write after write), `post_value_set_after_the_library_is_transformed_...`
  (T2), `transitive_dependencies_through_a_sqlview_...` (run gets the
  pre-expansion body: the stale-success case), `batch_entries_materialize_independently`
  (ordered batch), `transaction_with_value_set_and_library_commits_once_...`
  and `two_in_bundle_versions_of_one_url_are_two_candidates` (run right
  after the transaction: `404`), `transaction_deleting_a_value_set_...` and
  `deleting_the_newest_stored_version_in_the_bundle_...` (run after a
  delete: the deleted body still answers).
- Harness verification reads (`find_by_url`, `all_libraries`) make writes
  visible before searching: they check what the server stored, not when the
  index saw it. Seeds do the same after writing.

## 4. Order and dependencies

1. Slice A, postgres and mongodb drivers first: no fix expected, immediate
   parity signal, and the shared-scenario refactor that everything else
   builds on.
2. Slice A, ES driver with asynchronous sync, which yields Slice B's three
   failing tests.
3. Slice B.
4. Slice F scenarios 1 to 3 on sqlite, postgres, mongodb, sqlite-es.
5. Slice F scenario 4, then Slice C's decision.
6. Slices D and G.

Slice A alone answers "does it work everywhere". B is the only runtime
change this round. C's cache and the relation cache are explicitly deferred.

## 5. Acceptance

| Backend | Scenarios | Extra |
|---|---|---|
| sqlite | 38 green through the shared scenarios | — |
| postgres | 38 green | write latency at 200k recorded |
| mongodb | 38 green | transactions on a replica set; BSON size at caps |
| sqlite-elasticsearch | 38 + 14 visibility scenarios green with **asynchronous sync and default refresh** | `ensure_writes_visible` cost recorded on writes and per `$sql-run` job; ES size split recorded |
| s3 (MinIO) | direct, batch, caps, scopes green; transaction scenarios pin `501` | scan cost measured; C decided |
| postgres-es, mongodb-es, s3-es | nightly matrix green with its existing settings, **and** the focused `sql-libraries.spec.ts` run under default policies green on every ES leg | no dedicated Rust suite; B's fix is composite-generic and proven asynchronous on sqlite-es |

## 6. Local prerequisites

- Branch build: `cd ~/1824-hfs && . ~/.local/toolchain/env.sh && cargo build --release --bin hfs --features postgres,mongodb,elasticsearch,s3`
- PostgreSQL 18 runs locally: `. ~/.local/pg18/env.sh`.
- MongoDB, Elasticsearch and MinIO need Docker Desktop started on the Windows
  side; it was not reachable from this WSL session on 2026-10-15. The
  persistence and rest test suites use testcontainers once it is.
- A terminology server with real code systems for Slice F: the bundled seed
  has no SNOMED or LOINC, so either import SNOMED into a local `hts` or point
  `HFS_TERMINOLOGY_SERVER` at the hosted HTS if it has them loaded.
- Data for the join scenario: a slice of the Synthea corpus under
  `~/941-manual-test/matrix/corpus`, loaded with `$bulk-submit`.
- Port 8080 is the Firestore emulator; use another port for HFS.

## 7. Deferred, and what each needs before it is reconsidered

1. **S3 canonical cache.** Needs the design in Slice C (version-carrying
   keys, revalidation of unpinned canonicals across instances, deletion and
   replacement, cross-instance invalidation) and a Slice F number showing the
   scan is too slow.
2. **Relation cache on the `$sql-run` path.** Needs Slice F scenario 2 to
   show the per-job flatten dominates, and a key based on **resource
   revision identity** (`versionId` / `lastUpdated` of the stored ValueSet),
   not its business `version`: a ValueSet's business version can stay the
   same while its expansion changes. It would be the first cross-request
   cache on that path.
3. **Standalone S3: `Type/id` only.** A behaviour change; needs the team's
   call (open question 1).

Open questions for the team:

1. Is standalone S3 a backend we want canonical references to work on at
   scale, or is `Type/id` acceptable there?
2. Slice B keeps the visibility calls in entry preparation and the batch
   pre-pass, on the hook the batch handler already uses. Is this the moment
   to make "search sees my own acknowledged writes" a general composite
   property instead? That is bigger and touches every handler.

## 8. Revision 1 corrections, for the record

- Slice B no longer proposes request-local primary resolution; it could not
  make the separate-request test deterministic and missed T2 discovery and
  the query path. It is rebuilt on `ensure_writes_visible`.
- Removing `expansion` from ES documents is withdrawn as an optimisation:
  composite search reads ES bodies.
- The S3 `(tenant, type, url)` cache is withdrawn: it omitted the business
  version and could not see newer resources, including from other instances.
- The Slice A harness no longer names `Arc<dyn Backend>`; `Backend` has an
  associated connection type and lacks the CRUD/search/bundle surface. The
  builder now carries the principal, unique tenants, and production-like
  composite setup, and SQLite runs the shared scenarios.
- The export target is restated: shared resolution and rows, per-subject
  engines; measure kick-off and workers separately.
- Test counts corrected to 12 + 26. The write-path symptom corrected: an
  unresolved dependency is fetched from HTS or left alone on write, and
  `404` is the query-planning answer.

## 9. Revision 2 corrections, for the record

- Query-path visibility is no longer `NotFound`-only. Composite search
  returns ES bodies and `select_newest` ranks by what ES reports, so a stale
  hit is a successful read. The call is made once per job before the
  `JobResolver`'s first fetch fills its cache; update, delete and
  newer-version scenarios sit alongside the creation one.
- The type list adds `ViewDefinition` on every row: the walk resolves every
  `depends-on` canonical before classifying it, and a miss with HTS
  configured sends a definition lookup the ViewDefinition should never
  cause. A `POST ViewDefinition` then `POST Library` scenario pins it.
- The call sites are entry preparation (`create.rs` / `update.rs`) and
  `terminology_pre_pass`, not `plan`. `prepare_terminology` re-plans a
  `PATCH`ed Library inside the open transaction with `live: false`, which
  must keep answering from the pre-pass snapshot; a regression assertion
  counts zero visibility calls during execution.
- The unreachable-HTS scenario is split: already-expanded fixture with HTS
  unreachable (write succeeds, one definition), and compose-only fixture
  with the mock reachable (expansion succeeds, definition-search endpoints
  not called).
- Slice E no longer proposes removing the matrix's synchronous / `wait_for`
  settings; they protect ordinary search, SearchParameter CRUD and the UI
  rails. It adds a focused `sql-libraries.spec.ts` run under default
  policies on the ES legs instead.

## 10. Revision 2.1 corrections, for the record

- The batch barrier was placed only in `terminology_pre_pass`, which runs
  from `process_transaction` alone. Ordinary batch entries prepare their own
  writes in `process_batch_entry` and `process_batch_patch`; the call is now
  per entry there, and scenario 10 pins an entry resolving what an earlier
  entry stored.
- The direct and conditional `PATCH` handlers in `patch.rs` were omitted
  although both run T1/T2 themselves; they are now rows of their own, with
  scenarios 11 and 12 (Library PATCH adding a stored dependency, ValueSet
  PATCH needing discovery of a stored Library).
- The query-path barrier moved from `graph.rs` to `run.rs` before
  `resolve_subject` and to `export.rs` before the first subject resolution,
  because a `subjectCanonical` subject is itself a canonical search that
  runs before the graph exists. Scenarios 13 and 14 cover create and update
  of the subject through `$sql-run` and `$sql-export`.
