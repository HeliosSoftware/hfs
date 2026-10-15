# Issue #1824 — Terminology in SQL: implementation plan

| | |
|---|---|
| Issue | [HeliosSoftware/hfs#1824](https://github.com/HeliosSoftware/hfs/issues/1824) |
| Upstream spec change | [HL7/sql-on-fhir#407](https://github.com/HL7/sql-on-fhir/pull/407) — **merged 2026-10-12** (commit `97982c3e3`), trial-use, targets 3.0.0 |
| Spec pages read | [terminology-in-sql](https://build.fhir.org/ig/HL7/sql-on-fhir/en/terminology-in-sql.html), [SQLQuery](https://build.fhir.org/ig/HL7/sql-on-fhir/en/StructureDefinition-SQLQuery.html), [SQLView](https://build.fhir.org/ig/HL7/sql-on-fhir/en/StructureDefinition-SQLView.html), [ViewDefinition](https://build.fhir.org/ig/HL7/sql-on-fhir/en/StructureDefinition-ViewDefinition.html) |
| Plan date | 2026-10-14, revision 6 (after review rounds 1 to 6) |
| Status | Draft for review. Nothing implemented yet. |
| Code baseline | `main` at `cf1af0544` |

## Revision log

**Revision 6** addresses the two round-6 findings and the D13 rationale, tagged `[R6-n]`:

| # | Finding | Resolution (section) |
|---|---|---|
| R6-1 | A transaction POST with `ifNoneExist` is indexed as a candidate, but the executor may match an existing resource and discard the submitted one, so the overlay can claim an expansion that the stored resource lacks | A conditional POST (`request.ifNoneExist`) on a ValueSet, ConceptMap or Library is recorded as `Undetermined`, exactly like a conditional PUT or DELETE: strict mode rejects the transaction up front whenever any materialization resolution would occur; the candidate is never indexed under its canonical. Test: a submitted expanded ValueSet whose `ifNoneExist` matches a stored unexpanded one, plus a Library depending on it → strict 422 before persistence, nothing written (§5 PR 2c step 1, §6.14, Q15, Q19) |
| R6-2 | After a conditional `Created`, a phase B failure answers 422 and says "re-PUT the Library", but the caller does not know the server-assigned id | The 422 carries the created Library's `Location`, `ETag` and `Last-Modified` headers exactly as the 201 would, states in the OperationOutcome that the create **committed**, and names the PUT target; a retry of the same conditional POST is explicitly **not** a remedy because the pre-search would match the stored Library and return 200 without materializing. The batch entry form carries the same in `response.location` / `response.etag`. Recovery is tested using only the response (§5 PR 2c table and tests, §6.8, §3.1 step 6, D13) |
| R6-3 | D13's rationale still says orphan cache writes are unacceptable, contradicting permitted case (b) | Rationale rewritten to match the four permitted cases (D13) |
| R6-4 | Summary still described local-first expansion | Summary now states HTS-first when configured, local only as the no-HTS fallback, matching D3 (§0) |
| R6-5 | §6.1 charged rows as string lengths + 64 while D9 and §6.4 charge string bytes × 3 + 128 | One `row_bytes` function (UTF-8 bytes of every string column × 3 + 128) serves both the per-artifact `max_relation_bytes` cap inside the flatteners and the job budget; D9, §6.1, §6.3 and §6.4 say so (§4.2 API, D9, §6.1, §6.4) |

**Revision 5** addresses the three round-5 findings and the D13 wording, tagged `[R5-n]`:

| # | Finding | Resolution (section) |
|---|---|---|
| R5-1 | Evicting artifact JSON lets a later subject refetch changed content | The `JobResolver` caches a `ResolvedArtifact` (kind, identity, and for terminology the schema and shared rows) and classifies once on first fetch; `build_plan` asks the resolver, never the fetcher; test asserts one inner fetch per URL after eviction even when the fetcher would now return different content (§5 PR 1b, §4.3, §6.4) |
| R5-2 | Filtering the single newest storage result after selection hides surviving older versions | Storage exposes **all** canonical candidates; suppression is applied to the union of storage and overlay candidates **before** selection; regression test with stored v1 and v2 where the bundle deletes v2 (§6.14, §5 PR 1c) |
| R5-3 | A transaction PATCH that adds a dependency on an in-bundle compose-only ValueSet is never expanded | The pre-pass provisionally applies each Library PATCH (using the REST layer's existing patch application) to discover dependencies before ValueSet candidates are transformed; the executor's prepared candidate must declare a subset of what was expanded, else strict reject (§5 PR 2c, §6.14) |
| R5-4 | D13 guarantee contradicts ordinary POST, where phase B precedes the Library write | D13 now states the permitted partial-write cases exactly (D13) |

**Revision 4** addresses the four round-4 findings and the stale sentence, tagged `[R4-n]`:

| # | Finding | Resolution (section) |
|---|---|---|
| R4-1 | Overlay keyed by canonical URL loses distinct business versions | Overlay keyed by resource identity (`Type/id`) **and** by `(url, version)`, with a canonical index that lists every in-bundle candidate for a URL; pinned references match exactly, unpinned ones take the latest in execution order; test with two versions and two Libraries (§6.14) |
| R4-2 | Canonical lookups can resurrect deleted or replaced stored artifacts; conditional PUT/DELETE fallback unsafe | The pre-pass resolves each DELETE/PUT target's **previous** stored identity and records it as suppressed; `OverlayThenStorage` filters suppressed identities out of storage results; conditional PUT/DELETE on a terminology artifact or Library in a bundle that performs any materialization resolution is a strict 422 (§5 PR 2c, §6.14) |
| R4-3 | Conditional-create race could materialize dependencies no stored Library declares | For `If-None-Exist`, the atomic conditional create runs **before** phase B; `Exists` discards the planned writes; phase B runs only after `Created`. No orphan writes exist. The asymmetry (a phase B failure after `Created` leaves the Library stored and unmaterialized) is stated and tested (§5 PR 2c, §6.11, D13) |
| R4-4 | Job budget omits retained artifact JSON; per-row overhead understated | `JobResolver` charges retained artifacts and relations separately, drops a terminology artifact's JSON once flattened, uses a conservative per-row factor, and the budget is described as accounting, not an allocation bound (§6.4, D9) |
| R4-5 | Stale sentence said phase A precedes precondition checks | Removed; hook table is the single source (§5 PR 2c) |

**Revision 3** addresses the six round-3 findings, tagged `[R3-n]`:

| # | Finding | Resolution (section) |
|---|---|---|
| R3-1 | Transaction PATCH preparation reads application storage and cannot see earlier uncommitted bundle writes | A `BundleOverlay` built by the pre-pass in the executor's execution order, consulted before storage by every resolution inside the transaction, and updated by each prepared PATCH candidate in turn; tests for a ValueSet created, updated, patched or deleted earlier in the same bundle (§5 PR 2c, §6.14) |
| R3-2 | Conditional-create no-ops can still fail in phase A; the pre-search race is an exception to the no-op guarantee | The `If-None-Exist` pre-search runs **before** phase A, so an existing Library is returned without touching HTS; the guarantee was restated to cover the race (§5 PR 2c, §6.11). *Superseded by R4-3: the race is now eliminated by ordering the atomic create before phase B.* |
| R3-3 | Generated and transformed resources bypass write validation | T2 transforms **before** `check_write`; phase A validates every planned dependency write with `check_write` before phase B; the transaction pre-pass runs before the existing per-entry validation loop; `prepare_patch_candidate` transforms then validates (§5 PR 2c, §6.15) |
| R3-4 | Visited cap counts only ConceptMap targets; eligibility checks and exclusions uncounted; recursion unbounded | Every traversed structure counts (groups, elements, targets, includes, excludes, concepts, `contains` entries), eligibility checks included; nested `contains` is walked with an explicit stack and a depth bound of 32 (§6.1, §6.2, §6.3) |
| R3-5 | Stored-artifact memory bound relies on the request-body limit | Per-artifact byte estimate at plan time for every source (bounded, allocation-free walk), relation bytes accumulated during flattening, and a per-job retained-bytes budget held by the `JobResolver` (D9, §5 PR 2d, §6.4) |
| R3-6 | Versioned exclusion against an unversioned include is ambiguous | Local expansion requires, per system, identical `version` on every include and exclude; anything else is ineligible and goes to HTS or fails clearly (§6.3) |

**Revision 2** addresses the seven round-2 findings, tagged `[R2-n]`:

| # | Finding | Resolution (section) |
|---|---|---|
| R2-1 | Local expansion ignores `compose.inactive`, `lockedDate`, `version: "*"`, abstract concepts | D3 inverted: HTS expands whenever configured; local expansion is only the no-HTS fallback and is ineligible for every rule needing terminology knowledge (§6.3) |
| R2-2 | Dependency writes happen before `If-Match` / conditional-create outcomes are known | Phase B moves to immediately before the storage write, after the precondition is satisfied; conditional create gets a pre-search so a no-op never writes dependencies; tests assert no dependency change on 412 and on `Exists` (§5 PR 2c, §6.11) |
| R2-3 | Transaction pre-pass misses a new Library + new ValueSet pair, and transaction PATCH | Ordered three-step pre-pass over in-bundle candidates; `PatchCandidateValidator` gains a preparing variant that returns the transformed candidate (§5 PR 2c) |
| R2-4 | Injected transaction entries break the positional response mapping | No entries are injected. In a transaction, a definition only HTS holds is a strict 422; D4 applies to direct and batch writes only (§5 PR 2c, D4) |
| R2-5 | ConceptMap and local-expander caps checked after whole arrays | Caps checked at every insertion; separate visited-entry and emitted-member limits (§6.1, §6.2, §6.3, D9) |
| R2-6 | Planning memory unbounded: HTS bodies parsed before the byte cap, stored artifacts bypass it, export holds one row vector per subject | Bounded streaming read of HTS bodies; relations memoized once per job as `Arc<[Value]>` and shared by every subject plan; stored artifacts bounded by traversal caps plus the server's request-body limit (§4.3, §6.4, §6.9, D9, D11) |
| R2-7 | Write-time materialization walks only direct dependencies | Phase A walks the transitive graph through the same iterative walker `build_plan` uses, with its cycle, depth and node limits; recovery test for SQLQuery → stored SQLView → unexpanded ValueSet (§5 PR 2b, §6.13) |

**Revision 1** addresses the nine round-1 review findings in full. Each is tagged `[R1-n]` where it lands:

| # | Finding | Resolution (section) |
|---|---|---|
| R1-1 | Stored expansions do not guarantee one snapshot per export job (term-19) | Per-job memoizing fetcher shared by every subject plan, with a test that updates the ValueSet between two subjects' resolutions (§4.3, §6.9) |
| R1-2 | ValueSet write hook can lose the expansion or hit its own version conflict; Library materialization has no rollback story | T2 transforms the **incoming candidate** and persists it through the normal write once; T1 is two-phase (compute all expansions in memory, then write) with explicit partial-write semantics (§5 PR 2b, §6.11) |
| R1-3 | Automatic terminology writes bypass SMART authorization | Materializer takes the caller's `Principal` and requires the write scopes on the resource types it mutates; restricted-scope tests (§6.12) |
| R1-4 | Completeness check rejects the upstream fixture (`total: 3` counts the abstract parent) | `total` compared against **all** `contains` entries at all depths; members counted separately (§6.1) |
| R1-5 | Hook does not cover batch POST/PUT or transactions | Hooks specified at `process_batch_entry` for POST and PUT, and as a pre-pass over transaction entries that keeps atomicity (§5 PR 2c) |
| R1-6 | 120 s expansion timeout exceeds the 30 s request timeout | Per-write budget derived from `HFS_REQUEST_TIMEOUT`, sequential dependencies share it, documented configuration (§5 PR 2d) |
| R1-7 | Size limits do not bound bytes, planning memory, or inline artifacts | Member cap enforced during traversal for every source, byte cap on the stored resource, `total` checked before `count=total`, planning bound stated (§6.1, §6.5, D9) |
| R1-8 | Local expansion eligibility underspecified | Full eligibility and matching algorithm for includes and excludes, versioned vs unversioned (§6.3) |
| R1-9 | `stream::iter` over borrowed rows is not `'static` | Rows cloned into an owned `Vec` before the stream is built (§6.5) |

---

## 0. Summary

A SQLQuery or SQLView Library may now declare a **ValueSet** or a **ConceptMap** as a `relatedArtifact[type=depends-on]` dependency. The runner exposes each one under its `label` as an ordinary SQL relation with a fixed column set, so code membership and code translation become plain joins.

This plan implements that in HFS under one governing directive (Steve, 2026-10-14):

> When a Library declares a terminology dependency, HFS expands the ValueSet and **saves the expansion in HFS's own database**. HTS is not consulted at query time.

Consequences, in one paragraph: the `$sql-run` / `$sql-export` request path reads relations **only from HFS storage and from inline `context` artifacts**. Expansion happens **when the artifact is declared** (Library write) or **when the ValueSet is stored** (ValueSet write), using HTS as the expansion engine whenever one is configured, and a local expander, limited to enumerated value sets, only when no HTS is configured (D3) `[R6-4]`. The result is written into the stored ValueSet resource's `expansion` element, where it is a frozen, reproducible snapshot. ConceptMaps need no expansion; they are flattened from the stored resource directly.

The work is sliced into four pull requests so that each is reviewable alone and lands value alone:

| PR | Content | Depends on | Size |
|---|---|---|---|
| PR 0 | Declare the already-supported `context` parameter in the published OperationDefinitions (pre-existing gap) | — | S |
| PR 1 | Core: flatteners, plan nodes, fetcher widening, storage/`context` resolution, per-job memo, errors, FHIR typing, tests incl. both upstream examples | — | L |
| PR 2 | Materializer: write-path expansion into HFS storage on every write path, HTS as expansion engine and definition source, authorization, budgets, config | PR 1 | L |
| PR 3 | Surfaces: UI SQL Library editor, OperationDefinition target profiles, book + skill docs | PR 1 (UI), PR 2 (docs) | M |

Section 8 lists the decisions that still need a reviewer's sign-off.

---

## 1. Sources and what each one says

### 1.1 The spec (merged, normative text)

The "Terminology in SQL" page carries 24 numbered conformance statements. The ones that bind the **runner** are:

| ID | Rule | Enforced in |
|---|---|---|
| term-4 | ValueSet relation has at least `system`, `version`, `code`, `display`, `inactive`; `system` and `code` never null | §6.1, §6.5 |
| term-5 / term-11 | Columns SHOULD carry the default type mappings for `uri`, `string`, `code`, `boolean` (closest equivalent allowed) | §6.5 |
| term-6 / term-12 | Runner MAY add columns; authors must not rely on them. We add none. | — |
| term-7 | Rows unique on (`system`, `version`, `code`), two null versions equal | §6.1 |
| term-8 | From an expansion: one row per distinct triple among `contains` at any depth whose `abstract` is not `true` | §6.1 |
| term-10 | ConceptMap relation has the nine named columns; `source_system`, `source_code` never null | §6.2 |
| term-13 | One row per `group.element.target`, plus one per element with `noMap = true` | §6.2 |
| term-14 | `source_system`/`source_version` and `target_system`/`target_version` are `group.source` / `group.target` split at `\|` | §6.2 |
| term-15 | Rows unique on the seven identifying columns, nulls equal | §6.2 |
| term-16 | Reject before any SQL: group without `source`, element or target with `valueSet`, target with `dependsOn` or `product` | §6.2, §6.8 |
| term-17 | Never derive rows from `group.unmapped` | §6.2 |
| term-19 | One content per job; every reference in the job sees the same content | §4.3, §6.9 |
| term-20 | Fail before any SQL when the content cannot be determined: 404 for an unresolvable canonical, 422 for a resolvable artifact whose content cannot be determined | §6.8 |
| term-21 / term-22 | SHOULD record provenance | §6.6 |
| term-23 | A supplied (`context`) ValueSet with an expansion is used as-is | §6.7 |
| term-24 | A supplied expansion carrying an `offset`, or whose `total` exceeds the number of `contains` entries **at all depths**, is rejected | §6.1, §6.7 |

Other spec facts that shape the design:

- The `SQLQuery` and `SQLView` profiles widen `relatedArtifact.resource` to `canonical(ViewDefinition | SQLView | ValueSet | ConceptMap)`.
- `context` admits an inline ValueSet or ConceptMap, matched by canonical URL exactly like a view. Both are leaves.
- The Scope section explicitly allows a "cached expansion" as the source. A stored expansion in HFS is one.
- `cnl-1`: a stored artifact's `url` never contains `|`; the version pin lives only in the reference. HFS's `canonical_matches` already handles `url|version` and `url@version`.
- Default type mappings: `uri`, `code`, `string` → CHARACTER VARYING; `boolean` → BOOLEAN. SQLite TEXT and INTEGER affinities are the permitted "closest equivalent".
- The spec's error table says the 422 carries issue code `invalid`.
- No sql-on-fhir.js conformance tests for this feature exist upstream yet (checked 2026-10-14).

### 1.2 The issue text — corrections after the code read

| Issue says | Reality (verified 2026-10-14) |
|---|---|
| Upstream PR is an open draft | Merged 2026-10-12. Column set and error table are final. |
| Step 7: update `crates/sof/src/lint.rs` near line 507 | No Library lint exists; line 507 is the ViewDefinition field schema. Step 7 is a no-op. |
| sof-server: decide whether in scope | sof-server rejects SQLQuery/SQLView subjects (`crates/sof/src/handlers.rs:54`). Out of scope by construction. |
| Step 6: `in_memory.rs` picks it up unchanged | The export worker gets only runner + tenant + filters + plan. Terminology rows must be **inside the plan** at kick-off. |
| Step 8: widen `context` target profiles | `context` is not declared at all today and a test pins its absence. Declare first (PR 0). |
| R4 vs R5: `source`/`target` vs `sourceScope`/`targetScope` | Resource-level, unused. Group-level differences that matter: R4 `sourceVersion`/`targetVersion`, `target.equivalence`, no `noMap`, no `valueSet`, `dependsOn.property`. HFS defaults to R4. |

Not mentioned by the issue, found in the code read:

- `StorageArtifactFetcher::fetch` tries only ViewDefinition then Library; three error messages say "ViewDefinition or Library".
- `TerminologyServiceClient::expand_value_set` sends `url|version` unsplit, has a 10 s timeout, parses top-level `contains` only.
- `_format=fhir` typing consults only ViewDefinition leaf schemas.
- `DependencyRowCapExceeded` wording advises a ViewDefinition `where`.
- HFS 422s render issue code `processing`; the spec says `invalid`.
- The web UI (`crates/ui/src/sql_libraries.rs`) has its own dependency resolver that knows only ViewDefinition and Library.
- Write paths: direct create/update/patch call `reject_unknown_view_definition_resource`; **batch POST/PUT entries do not** (they go through `process_batch_entry` with only `check_write`); **transactions hand the whole bundle to persistence** (`process_transaction_with_patch_validator`), where only PATCH candidates are validated by the REST layer.
- Handlers see a `TenantContext` with `TenantPermissions::full_access()`; SMART scopes are enforced once in `authz_middleware` against the incoming request. The caller's `Principal` is in request extensions, and `purge.rs` / `reindex.rs` show the handler-level pattern (`Option<&Principal>` + `scopes.permits(resource_type, SmartPermissions)`).
- `HFS_REQUEST_TIMEOUT` defaults to 30 s.
- `ALLOWED_BODY_PARAMS` in `export.rs` already lists `context`. The `@relatedDependency` annotation is a builder concern; nothing to do.

---

## 2. Decisions this plan assumes

| # | Decision | Rationale | Alternative rejected |
|---|---|---|---|
| D1 | **Expansions are materialized into HFS storage; the query path never calls HTS.** | Directive. Reproducibility for free, no HTS timeout on the request path. | HTS as a run-time third source. |
| D2 | **Expansion runs synchronously on the write path**: a Library write that declares a ValueSet dependency, and a ValueSet write some stored Library depends on. | "When someone declares". Deterministic. | Lazy on first `$sql-run` (HTS on request path). Async job (race with first run). |
| D3 `[R2-1]` | **HTS is the expansion engine whenever it is configured.** Local expansion exists only as the no-HTS fallback, restricted to composes that need no terminology knowledge (§6.3), and never asserts `inactive` or abstract status. | Membership rules such as `compose.inactive`, `lockedDate`, `version: "*"` and abstract concepts need a code system; approximating them locally would silently produce wrong membership. | Local-first for enumerated composes (revision 1). |
| D4 `[R2-4]` | **HTS is the source of definitions** HFS does not store, for **direct and batch writes**: fetched once, stored in the tenant (tagged), then expanded. Inside a **transaction** no extra entries are ever injected; a dependency only HTS holds is a strict 422 asking the client to store it first or include it in the bundle. | Otherwise every artifact must be hand-copied. A transaction's response must have exactly one entry per request entry in order, and HFS maps results positionally. | Inject POST entries into the transaction (needs internal-entry tracking through results, audit and notifications). |
| D5 | **R4 `equivalence` is mapped onto R5 `relationship`** (§6.2). | HFS defaults to R4. | Reject R4. |
| D6 | **Terminology leaves count against `HFS_SOF_SQLQUERY_MAX_VDS`.** | They are nodes; size is capped separately. | Separate cap. |
| D7 | **`context` ValueSets are never persisted.** With an expansion: used as-is. Compose-only: local expansion if enumerated, else 422. | Spec: supplied artifact is the client's intent for that request. Keeps HTS off the request path. | Expand via HTS for `context`. |
| D8 | **New 422s carry issue code `invalid`** via `RestError::ValidationFailed { outcome }`. Existing 422 paths unchanged. | Spec table; future conformance tests. | Reuse `UnprocessableEntity` (`processing`). |
| D9 `[R1-7] [R2-5] [R2-6] [R3-4] [R3-5]` | **Size bounds at every boundary.** (1) Member cap `HFS_SOF_TERMINOLOGY_MAX_EXPANSION_MEMBERS` (default 200 000) on distinct emitted rows, checked at every insertion. (2) Visited-structure cap, fixed at 4 × the member cap, counting **every** structure a flattener or eligibility check touches; nested `contains` walked with an explicit stack and a depth bound of 32. (3) Per-artifact byte cap `HFS_SOF_TERMINOLOGY_MAX_ARTIFACT_BYTES` (default 8 MiB): applied while streaming an HTS response (abort before parsing), on the serialized resource before persistence, and **at plan time to every terminology artifact from any source** through a bounded, allocation-free size estimate of the parsed JSON. (4) Per-job retained-bytes **accounting** budget `HFS_SOF_TERMINOLOGY_MAX_JOB_BYTES` (default 128 MiB) `[R4-4]`: the `JobResolver` charges every retained artifact (its `estimate_json_bytes`) and every retained relation through the single `row_bytes` function (UTF-8 bytes of every string column × 3 + 128 per row, a deliberately conservative factor for `serde_json::Value` objects, keys and the dedupe sets) and fails the request when the sum would exceed the budget. The same `row_bytes` is what the flatteners sum against the per-artifact `max_relation_bytes` cap, so the per-artifact cap and the job budget measure the same quantity `[R6-5]`. It is an accounting bound that tracks allocations, not a measured one. (5) `total`, when present, compared against the member cap before any `count=total` re-request. | A 1 M-member expansion stored as one JSON document is ~150 MB (Mongo's document limit is 16 MB). Stored artifacts can arrive through internal callers, imports or older configuration, so a body limit at write time is not a bound. Member counts do not bound string lengths. | Reuse the per-dependency row cap only; count after traversal; trust the request-body limit. |
| D10 | **Strict by default**: an expansion that cannot be produced fails the write with 422. `HFS_SOF_TERMINOLOGY_MATERIALIZE=lenient` warns and continues; `off` disables materialization. | Silently storing an unrunnable Library hides the failure. | Always lenient. |
| D11 `[R1-1] [R2-6]` | **One resolution per job, one copy per artifact**: every subject plan in a `$sql-export` kick-off (and the single plan of a `$sql-run`) resolves through one memoizing resolver that caches both the fetched resource and the **flattened relation** (`Arc<[Value]>` rows + schema). Every plan node referencing the same artifact shares that one allocation. | term-19 is about *content*; two storage reads can straddle a concurrent update. Export memory must not multiply by subject count. | Rely on storage reads being identical; one row vector per subject. |
| D12 `[R1-3]` | **Materialization acts under the caller's authority**: it requires the caller's SMART scopes to permit the writes it performs (`ValueSet` update; `ValueSet`/`ConceptMap` create when fetching from HTS). Without them, strict mode answers 403 naming the missing scope; lenient mode skips materialization with a warning. When auth is disabled there is no principal and the check passes. | A Library-write scope must not become a ValueSet-write scope by side effect. Matches how `purge`/`reindex` enforce their own scopes in the handler. | Treat materialization as a system action authorized by configuration (hides privilege escalation behind a server setting). |
| D13 `[R1-2] [R2-2] [R2-7] [R3-2] [R3-3]` | **Library materialization is two-phase, transitive and validated.** Order on every write path: outer preconditions that can be decided without a write (`If-Match`, the `If-None-Exist` pre-search) → phase A (transitive walk, expansions computed in memory, every planned dependency write validated with `check_write`) → phase B (dependency writes) → outer write. A precondition failure, a conditional-create no-op, or a phase A failure writes nothing and never contacts HTS after a no-op is known. A phase B failure on dependency *k* leaves dependencies 1..k-1 expanded and aborts the outer write. For a conditional create the atomic conditional create itself runs **before** phase B; `Exists` discards the planned writes `[R4-3]`. **Permitted partial writes, stated exactly `[R5-4]`:** (a) no dependency is ever written by a request that is rejected before phase B; (b) on an ordinary POST, PUT or PATCH, phase B precedes the outer write, so a failure of the outer write (or of phase B itself at dependency *k*) can leave expansions in place for a Library that was not stored; they are an idempotent cache for the Library the client will retry, and no other outcome is possible without cross-resource atomicity; (c) on a conditional create, the atomic create precedes phase B, so a phase B failure leaves the Library stored but unmaterialized, reported as 422 that carries the created Library's `Location`/`ETag` and lists the dependencies still to materialize, and a PUT to that `Location` retries `[R6-2]`; (d) no expansion is ever written for a request body that lost a conditional-create race. | Cross-resource atomicity does not exist on every backend. An expansion is an additive, idempotent cache, so an expansion left behind by a failed outer write (case b) is harmless and is consumed by the retry `[R6-3]`. What is not acceptable is writing anything for a request that is already known not to store its body: a rejected precondition, a conditional no-op, a lost race (cases a and d), or touching HTS for such a request. | Compensating deletes; phase B before the conditional create (orphan writes on the race). |

---

## 3. Target behaviour (end to end)

### 3.1 Declaring

```json
"relatedArtifact": [
  { "type": "depends-on", "resource": "https://example.org/ViewDefinition/conditions", "label": "conditions" },
  { "type": "depends-on", "resource": "http://example.org/ValueSet/cardiovascular-disease|2026", "label": "cvd_codes" },
  { "type": "depends-on", "resource": "http://example.org/ConceptMap/sct-to-icd10|2026", "label": "sct_to_icd10" }
]
```

On any write of this Library (POST, PUT, PATCH, batch entry, transaction entry), HFS:

1. Parses the Library (`parse_sqlquery_library`). A Library that is not a SQLQuery/SQLView is a no-op.
2. Checks the caller's scopes against the writes materialization may need (D12).
3. The outer request's own preconditions that need no write are evaluated first: `If-Match` (412 ends here) and, for a conditional create, an `If-None-Exist` pre-search (a match returns the existing Library and ends here, without any HTS contact) `[R3-2]`.
4. Phase A: walks the Library's **transitive** dependency graph (SQLViews included, §6.13) resolving every canonical against storage (four types). A stored **ValueSet without `expansion`** at any depth is expanded in memory (HTS when configured, else §6.3). A canonical in neither storage nor HTS is left alone (a Library write never validated dependencies before). In a direct or batch write, a canonical only HTS holds has its definition fetched (D4) and, for a ValueSet, expanded. Every resource phase B would write is validated with `check_write` now `[R3-3]`.
5. Phase B: writes each new or updated dependency resource through the storage layer (and the write-event path, so subscriptions and audit see it), then the Library. For a conditional create the order is inverted: the atomic conditional create first, then phase B only on `Created` `[R4-3]`.
6. Strict mode: any failure in steps 2–4 aborts with nothing written; a failure in step 5 aborts the Library write, leaving already-written expansions (D13), or, after a conditional `Created`, leaves the Library stored and reports the unmaterialized dependencies in a 422 that carries the created Library's `Location` and `ETag` and says the create committed `[R6-2]`.

### 3.2 Running

`POST /$sql-run` with the subject above:

1. Phase 1 (`build_plan`) walks the graph through a memoizing fetcher. A dependency that resolves to a ValueSet or ConceptMap becomes a **relation leaf**: flattened into rows during Phase 1, rows carried in the plan.
2. Resolution order per URL is unchanged: storage, then `context`, then 404. A stored ValueSet without an expansion is a 422 ("not materialized; re-save the Library or the ValueSet"). A `context` ValueSet without an expansion is expanded locally if enumerated, else 422.
3. Phase 2 (`execute_plan`) creates the fixed-schema table, inserts the rows under the per-dependency row cap, aliases it by label.
4. The subject SQL runs. `_format=fhir` renders `system` as `valueUri`, `code`/`relationship` as `valueCode`, `inactive` as `valueBoolean`.

`$sql-export`: Phase 1 runs at kick-off for every subject through **one** memoizing fetcher (D11); the plan carries the rows into the worker. All terminology failures are kick-off rejections.

### 3.3 Both upstream examples pass verbatim

`CardiovascularDiseasePatientsQuery` returns rows `p1`, `p2`; `ConditionsToIcd10Query` returns four rows with `target_code` `I21`, null, `E14`, null (§7).

---

## 4. Architecture

### 4.1 Components

```
                 Write paths (PR 2)
  POST/PUT/PATCH /Library, /ValueSet ─┐
  batch entries (process_batch_entry) ┼─► handlers::sof::terminology_materialize
  transaction entries (pre-pass)      ┘       ├─ scope check (Principal)                      [D12]
                                              ├─ phase A: resolve deps, expand in memory
                                              │     ├─ local enumerated expander (helios-sof)   [D3]
                                              │     └─ TerminologyServiceClient → HTS            [D3, D4]
                                              └─ phase B: storage.update / create (+ write events), then the outer write

                 Query paths (PR 1)
  $sql-run / $sql-export kick-off
        │
        ▼
  graph::build_plan ──► JobResolver(StorageArtifactFetcher: VD, Library, ValueSet, ConceptMap + context)  [D11]
        │               └► inline `context` list
        ├─ ViewDefinition → PlanNode::Leaf     (unchanged)
        ├─ SQLView        → PlanNode::SqlView  (unchanged)
        └─ ValueSet / ConceptMap → helios_sof::sqlquery::terminology::{flatten_value_set, flatten_concept_map}
                                     └─► PlanNode::Relation { schema, rows }   ◄── no network or storage after this
        ▼
  graph::execute_plan ──► engine.create_table / insert_rows (row cap) / create_view(label)
```

### 4.2 Where code lives

| Crate / module | Responsibility | Status |
|---|---|---|
| `helios-sof` `sqlquery/terminology.rs` | Relation schemas, row types, pure flatteners with traversal caps, local enumerated expander + eligibility, R4→R5 mapping, content errors. **No HTTP, no storage.** | New (PR 1) |
| `helios-sof` `sqlquery/mod.rs` | `DependencyKind::{ValueSet, ConceptMap}`, `SqlQueryError::TerminologyContent` | PR 1 |
| `helios-rest` `handlers/sof/graph.rs` | `classify_dependency`, `PlanNode::Relation`, `JobResolver`, shared `walk_dependency_graph`, `build_plan` arms, fetcher widened, `execute_plan` arm | PR 1 |
| `helios-rest` `handlers/sof/sqlquery.rs`, `export.rs` | error mapping, FHIR typing, one memo fetcher per export job | PR 1 |
| `helios-rest` `handlers/sof/terminology_materialize.rs` | write-path materializer: triggers, scope check, two-phase write, engine selection, persistence, tagging, budget | New (PR 2) |
| `helios-rest` `terminology.rs` | client: expand full resource with completeness + caps, version pin, configurable timeout; read ValueSet / ConceptMap by canonical | PR 2 |
| `helios-rest` `handlers/{create,update,patch,batch}.rs` | hooks (§5 PR 2c) | PR 2 |
| `helios-rest` `config.rs` | env vars (§5 PR 2d) | PR 2 |
| `helios-rest` `handlers/sof/capability.rs` | declare `context`; widen target profiles | PR 0, PR 3 |
| `helios-ui` `sql_libraries.rs` + I/O half in `lib.rs` | dependency kinds, columns card | PR 3 |
| skills + book | docs | PR 3 |

### 4.3 Resolution rules

For one dependency URL `u` (`url`, `url|version`, `url@version`, or `Type/id`):

1. **Storage** (tenant-scoped), resource types in order `ViewDefinition`, `Library`, `ValueSet`, `ConceptMap`. A relative `Type/id` reference tries only its own type. A version pin matches only a resource whose `version` equals it.
2. **`context`**, same matcher. Storage wins over `context` for the same URL (design #568, unchanged).
3. Otherwise `404 Not Found` naming `u`.

**One read per job, one resolved entry per artifact `[R1-1] [R2-6] [R5-1]`.** `JobResolver` wraps the storage fetcher and the inline `context` list and caches, per URL, a `ResolvedArtifact` produced on the **first** fetch: the classification, the identity (`url`, `version`, `Type/id`), and for a ViewDefinition or SQLView the resource itself, for a ValueSet or ConceptMap the flattened relation (`Arc<[Value]>` rows, schema, kind, display name) with the JSON already dropped. `build_plan` only ever asks the resolver for a `ResolvedArtifact`; it never fetches or classifies on its own, so a later plan in the same job cannot refetch. `$sql-run` creates one per request. `$sql-export`'s `extract_subjects_from_body` creates one and passes it to every `prepare_named_sqlquery`, so two subjects that depend on the same ValueSet see the same relation and share the same rows allocation even if a third client updates it between the two plans. Infrastructure errors are not cached. Node dedupe within one plan is still by the URL string (`finished` map), as today.

### 4.4 Relation shapes

**Value set relation** (`VALUE_SET_SCHEMA`):

| column | FHIR type → `ColumnFhirType` | SQLite affinity | null |
|---|---|---|---|
| `system` | `uri` → `String("uri")` | TEXT | never |
| `version` | `string` | TEXT | yes |
| `code` | `code` → `String("code")` | TEXT | never |
| `display` | `string` | TEXT | yes |
| `inactive` | `boolean` → `Boolean` | INTEGER (0/1) | yes (null for active codes) |

**Concept map relation** (`CONCEPT_MAP_SCHEMA`): `source_system` uri, `source_version` string, `source_code` code, `source_display` string, `target_system` uri, `target_version` string, `target_code` code, `target_display` string, `relationship` code. `source_system` and `source_code` never null.

---

## 5. Work breakdown

### PR 0 — declare `context` (pre-existing gap)

Files: `crates/rest/src/handlers/sof/capability.rs`.

- Add to both `sql_run_definition()` and `sql_export_definition()`:
  ```json
  {"name": "context", "use": "in", "min": 0, "max": "*", "type": "CanonicalResource",
   "targetProfile": ["http://hl7.org/fhir/StructureDefinition/ViewDefinition",
                     "http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition/SQLView"],
   "documentation": "Supporting artifacts the server cannot itself resolve, supplied inline and matched by canonical URL against the subject's transitive relatedArtifact graph. Requires POST."}
  ```
  using `helios_sof::canonical::{VIEW_DEFINITION_STRUCTURE_DEFINITION, SQL_VIEW_PROFILE}`.
- Module doc table: remove the `context` row.
- Test `run_definition_bases_on_the_guides_and_omits_what_we_reject`: `context` moves to the supported list; assert the two target profiles.

Done when: CI green; `GET /OperationDefinition/hfs-sql-run` lists `context`.

### PR 1 — core (no network, no write path)

Goal: a Library whose dependency resolves to a stored ValueSet **already carrying an expansion**, or a stored ConceptMap, or either supplied via `context`, runs correctly in `$sql-run` and `$sql-export`, with one read per job.

#### 1a. `helios-sof` — `crates/sof/src/sqlquery/terminology.rs` (new)

```rust
pub const VALUE_SET_COLUMNS: [(&str, &str); 5];   // (name, fhir type code)
pub const CONCEPT_MAP_COLUMNS: [(&str, &str); 9];
pub fn value_set_schema() -> TableSchema;
pub fn concept_map_schema() -> TableSchema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueSetRow { pub system: String, pub version: Option<String>, pub code: String,
                         pub display: Option<String>, pub inactive: Option<bool> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptMapRow { pub source_system: String, pub source_version: Option<String>,
                           pub source_code: String, pub source_display: Option<String>,
                           pub target_system: Option<String>, pub target_version: Option<String>,
                           pub target_code: Option<String>, pub target_display: Option<String>,
                           pub relationship: Option<String> }

/// Caps applied while traversing, never after (D9).
#[derive(Debug, Clone, Copy)]
pub struct FlattenLimits { pub max_members: usize, pub max_visited: usize, pub max_relation_bytes: usize }   // max_visited = 4 × max_members [R2-5] [R3-5]

/// term-7, term-8, term-24. Stops at `max_members + 1` and returns TooManyMembers.
pub fn flatten_value_set(value_set: &Value, limits: FlattenLimits) -> Result<(Vec<ValueSetRow>, usize /*bytes*/), TerminologyContentError>;
/// term-13..17, D5. Accepts R4/R4B/R5/R6 group shapes. Same caps on mappings.
pub fn flatten_concept_map(concept_map: &Value, limits: FlattenLimits) -> Result<(Vec<ConceptMapRow>, usize /*bytes*/), TerminologyContentError>;
/// Bounded, allocation-free estimate of a JSON value's serialized size; stops and returns Err as soon as `cap`
/// is exceeded. Applied to every terminology artifact at plan time, whatever its source [R3-5].
pub fn estimate_json_bytes(value: &Value, cap: usize) -> Result<usize, TerminologyContentError>;
pub fn row_bytes(row: &Value) -> usize;   // Σ UTF-8 bytes of every string column × 3 + 128; the one row measure for max_relation_bytes and the job budget [R6-5]

/// D3/D7. `Ok(None)` when the compose is not eligible (§6.3); `Ok(Some(expanded_value_set))`
/// is the input with an `expansion` added. Pure: `now` and `identifier` are injected.
pub fn expand_enumerated_compose(value_set: &Value, now: DateTime<Utc>, identifier: &str, limits: FlattenLimits)
    -> Result<Option<Value>, TerminologyContentError>;

impl ValueSetRow { pub fn to_json(&self) -> Value }   impl ConceptMapRow { pub fn to_json(&self) -> Value }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminologyContentError {
    NoExpansion,
    IncompleteExpansion { reason: String },
    UnsupportedConceptMap { reason: String },
    TooManyMembers { max: usize },
    TooManyEntries { max: usize },
    TooManyBytes { max: usize },
    NotEnumerated,
}
```

`crates/sof/src/sqlquery/mod.rs`:
- `DependencyKind::ValueSet` (Display `"ValueSet"`), `DependencyKind::ConceptMap` (Display `"ConceptMap"`).
- `SqlQueryError::TerminologyContent { kind: DependencyKind, label: String, url: String, error: TerminologyContentError }`.
- `DependencyRowCapExceeded` Display becomes kind-aware (wording in §6.8).

#### 1b. `graph.rs`

- `enum DependencyArtifact { ViewDefinition, SqlView, SqlQuery, ValueSet, ConceptMap, Other(String) }` + `fn classify_dependency(&Value)`. `classify_subject` untouched: a ValueSet is still not a legal subject.
- `PlanNode::Relation { internal_name: String, kind: DependencyKind, name: String, schema: TableSchema, rows: Arc<[Value]> }` `[R2-6]` (shared, immutable; cloning a plan node clones a pointer).
- `build_plan` gets `limits: FlattenLimits` (from `min(sof_sqlquery_max_source_rows_per_vd, terminology_max_expansion_members)`) and two new arms. Errors become `sqlquery_err_to_rest(SqlQueryError::TerminologyContent{..})`, pushed and aggregated like any Phase 1 error; `failed.insert(url)`. The artifact's `origin` (`Storage` | `Context`) is threaded so D7 can be applied.
- `JobResolver<'a> { inner: &'a dyn ArtifactFetcher, inline: &'a [Value], limits: FlattenLimits, budget: Mutex<ByteBudget>, cache: Mutex<HashMap<String, Arc<ResolvedArtifact>>> }` `[R1-1] [R2-6] [R5-1]` with
  ```rust
  pub(crate) enum ResolvedArtifact {
      ViewDefinition { identity: Identity, resource: Arc<Value> },
      SqlView        { identity: Identity, resource: Arc<Value> },
      SqlQuery       { identity: Identity },                        // rejected as a dependency, cached so the rejection is stable
      Relation       { identity: Identity, kind: DependencyKind, name: String, schema: TableSchema, rows: Arc<[Value]> },
      Unsupported    { resource_type: String },
      NotFound,
      ContentError   { kind: DependencyKind, error: TerminologyContentError },   // cached so every plan reports the same 422
  }
  pub(crate) async fn resolve(&self, url: &str) -> Result<Arc<ResolvedArtifact>, RestError>;   // infra errors uncached
  ```
  On first resolution the resolver fetches (storage, then `context`), classifies, and for a terminology artifact checks the artifact byte cap, flattens under the limits, charges the budget and **drops the JSON**. `build_plan` takes `&JobResolver` and matches on `ResolvedArtifact`; it has no access to the fetcher.
- `StorageArtifactFetcher::fetch`: four types in order; stop at first `Ok` or non-`NotFound` error.
- Messages: "ViewDefinition, Library, ValueSet or ConceptMap" in the 404 and the wrong-type rejection.
- `execute_plan`: new `Relation` arm (§6.5). `labels_by_target` unchanged.

#### 1c. `references.rs`, `sqlquery.rs` and `export.rs`

- `references.rs` `[R5-2]`: split `resolve_by_canonical_url` into `canonical_candidates(state, tenant, resource_type, url) -> Vec<StoredResource>` (every stored resource whose `url`/`version` satisfies `canonical_matches`, on the search path and the scan path alike) and the existing selection (`max_by_key(last_modified)`). The direct resolver keeps its behaviour; `OverlayThenStorage` (§6.14) uses the candidate list.

- `sqlquery_err_to_rest`: `TerminologyContent` → `ValidationFailed` with issue `{severity: error, code: invalid, diagnostics, expression: ["relatedArtifact.where(label='<label>')"]}`. `TooManyMembers` → same shape, code `too-costly`.
- `run_library_subject`: build one `JobResolver` per request. Output typing unchanged; it now sees relation schemas.
- `extract_subjects_from_body` (export): build **one** `JobResolver` before the subject loop and pass it into every `prepare_named_sqlquery`, so every subject's plan nodes share the same `Arc` rows `[R1-1] [R2-6]`.

#### 1d. Tests (PR 1)

Unit, `terminology.rs`:
- flatten_value_set: upstream fixture (abstract parent + 2 nested members, `total: 3`) → **accepted**, 2 rows `[R1-4]`; duplicate triple under two parents → 1 row; `inactive: true` preserved; `inactive: false` preserved as `Some(false)`; missing `version` → null; `offset` → IncompleteExpansion; `total: 4` with 3 entries at all depths → IncompleteExpansion; `total` absent → accepted; non-abstract entry lacking `code` or `system` → IncompleteExpansion with path; empty `contains` → zero rows; no `expansion` → NoExpansion; `max_members: 2` with 3 members → TooManyMembers **without** visiting the 4th entry (fixture whose 4th entry is malformed); 1 000 abstract entries with `max_members: 2` → TooManyEntries at the visited cap, not accepted `[R2-5]`.
- flatten_concept_map (R5): upstream fixture → exactly the 3 rows of the spec table; `target` split; two groups; 7-tuple dedupe; each term-16 rejection names the path; `unmapped` ignored; empty map → zero rows; one element with `max_members + 2` targets → TooManyMembers raised before the element's target array is fully processed (fixture whose last target is malformed) `[R2-5]`.
- flatten_concept_map (R4): `sourceVersion`/`targetVersion` populate the version columns; every `equivalence` code per §6.2; `unmatched` → noMap row; R4 `dependsOn`/`product` rejected.
- expand_enumerated_compose: every eligibility case in §6.3 including `compose.inactive` (both values), `lockedDate`, `version: "*"`, whole-system include, `filter`, `valueSet`, and version disagreement per system → `Ok(None)`; exclusions with agreeing versions; `parameter` entries; identifier/timestamp injected; member cap at insertion; visited cap triggered inside the eligibility check by a compose with more concepts than the cap `[R2-1] [R2-5] [R3-4] [R3-6]`.

Dry graph tests (`graph.rs`, existing `MapFetcher`): ValueSet leaf from fetcher; from `context`; storage wins over `context`; ConceptMap leaf; ValueSet reached through an interior SQLView; label collision VD vs VS; node cap counts the relation (D6); stored VS without expansion → aggregated 422 alongside another error; `context` compose-only eligible → expanded locally; `context` compose-only ineligible → 422; `JobResolver` calls the inner fetcher once and flattens once for a URL requested by two plans, both plans' `Relation` nodes hold the same `Arc` (pointer equality), and a second plan built after the fetcher has been switched to return **different** content still gets the first relation `[R1-1] [R2-6] [R5-1]`; the visited-entry cap rejects a deeply nested abstract-only expansion before the member cap is reached `[R2-5]`.

execute_plan tests: relation over the row cap → `DependencyRowCapExceeded { kind: ValueSet }` with terminology wording; `leaf_schemas` contains the relation schema.

Integration, new `crates/rest/tests/sof_sqlquery_terminology.rs` (harness of `sof_sqlquery_graph.rs`):
- Seed the upstream `conditions` ViewDefinition, four Conditions, the upstream ValueSet (with expansion) and ConceptMap verbatim, both upstream Libraries verbatim. Assert exact rows in `json` and `fhir` (`system` → `valueUri`, `relationship` → `valueCode`, absent `inactive` omitted).
- Same with both artifacts supplied via `context`.
- Stored VS without expansion → 422 `invalid`, diagnostics names the label.
- 404 names the four kinds.
- `$sql-export` of `CardiovascularDiseasePatientsQuery` → manifest rows; a terminology 422 is a kick-off rejection.
- `[R1-1]` `$sql-export` with two subjects depending on one ValueSet, where a test-only `ArtifactFetcher` wrapper updates the ValueSet between the first and second resolution → both outputs reflect the **first** content; a control test without the memo shows the difference.
- `_format=fhir` on an expansion with one inactive member → `valueBoolean`.

### PR 2 — materializer (write path)

#### 2a. Terminology client (`crates/rest/src/terminology.rs`)

`expand_value_set` and `expand_subsumption` (used by `:in` search) are unchanged.

```rust
pub struct ExpandOptions { pub timeout: Duration, pub max_members: usize, pub max_artifact_bytes: usize }
/// POST {base}/ValueSet/$expand with the full ValueSet as inline `valueSet` (+ `valueSetVersion` when the
/// resource has `version`). The response body is read as a stream and aborted once `max_artifact_bytes` is
/// exceeded, before any JSON parsing [R2-6]. Completeness: no `offset`; if `total` is present and
/// > max_members → Err(TooLarge) before anything else; if `total` > entries at all depths → re-request once
/// with `count = total`; still short → Err(Incomplete). Entries are counted with the bounded walk of §6.1.
pub async fn expand_value_set_resource(&self, value_set: &Value, opts: &ExpandOptions) -> Result<Value, TerminologyError>;
/// GET {base}/ValueSet?url=...[&version=...] → newest match or None.
pub async fn read_value_set_by_canonical(&self, url: &str, version: Option<&str>) -> Result<Option<Value>, TerminologyError>;
pub async fn read_concept_map_by_canonical(&self, url: &str, version: Option<&str>) -> Result<Option<Value>, TerminologyError>;
pub fn with_timeout(base_url: String, timeout: Duration) -> Self;   // `:in` keeps `new()` with 10 s
// All three reads apply the same bounded body read [R2-6].
```

Inline `valueSet` rather than `url` so that HTS expands **the definition HFS stores** (storage-first precedence) and so it works whether or not HTS knows the URL.

#### 2b. Materializer (`crates/rest/src/handlers/sof/terminology_materialize.rs`, new) `[R1-2] [R1-3]`

```rust
pub(crate) struct MaterializeBudget { pub deadline: Instant }      // §PR 2d
pub(crate) enum Outcome { AlreadyExpanded, ExpandedLocally, ExpandedByServer, FetchedAndExpanded, FetchedDefinition, NotResolvable, SkippedNoScope, SkippedLenient(String) }

/// Trigger T1 (Library). Phase A only: walks the Library's TRANSITIVE dependency graph [R2-7] through the
/// same iterative walker `build_plan` uses (§6.13) — SQLViews are descended into, cycles, MAX_GRAPH_DEPTH
/// and sof_sqlquery_max_vds apply — and computes every expansion that must be written. Reads storage, HTS
/// (direct/batch only, D4) and `bundle_candidates`; writes nothing. Checks `principal` scopes first (D12).
/// `allow_side_writes = false` inside a transaction: anything that would need a write outside the bundle
/// is reported as NeedsSideWrite instead.
pub(crate) async fn plan_library_materialization<S>(
    state: &AppState<S>, tenant: &TenantContext, principal: Option<&Principal>,
    library: &Value, bundle_candidates: &[Value], allow_side_writes: bool, budget: &MaterializeBudget,
) -> Result<MaterializationPlan, RestError>;

/// Phase B: persists the plan (storage.update for expansions of stored ValueSets; storage.create for
/// definitions fetched from HTS), emitting write events like the direct handlers do. Returns per-dependency
/// outcomes. Stops at the first failure (D13).
pub(crate) async fn apply_materialization<S>(state, tenant, plan: MaterializationPlan) -> Result<Vec<(String, Outcome)>, RestError>;

/// Trigger T2 (ValueSet). Returns the candidate with an `expansion` added when (a) it has none, (b) some stored
/// SQLQuery/SQLView Library in the tenant — or one of `bundle_libraries` [R2-3] — depends on its canonical,
/// and (c) expansion succeeds. The caller then persists the returned candidate through its normal single
/// write. Never writes itself.
pub(crate) async fn materialize_value_set_candidate<S>(
    state, tenant, principal: Option<&Principal>, candidate: Value, bundle_libraries: &[Value], budget: &MaterializeBudget,
) -> Result<(Value, Outcome), RestError>;
```

Why T2 transforms the candidate `[R1-2]`: on POST the resource does not exist yet; on PUT a separate pre-write update would be overwritten by the outer write or collide with its version check. Transforming the incoming body and letting the normal write persist it once removes both problems, keeps `If-Match` semantics intact, and audits one write.

Why T1 is two-phase `[R1-2] [R3-3]`: phase A has no side effects, so every validation failure (scope, unresolvable, unexpandable, over cap, budget exceeded, and **`check_write` on each planned dependency resource** — an HTS-fetched R5-shaped ValueSet that does not validate against the tenant's FHIR version fails here, closed) is reported with nothing written. Phase B writes are each idempotent (an already-expanded ValueSet is `AlreadyExpanded` on retry), so a mid-way failure is recovered by re-issuing the Library write. The 422/403 diagnostics list which dependencies were materialized and which were not.

Engine selection (D3) `[R2-1]`, in order: HTS configured → `expand_value_set_resource` within the remaining budget (HTS applies `compose.inactive`, `lockedDate`, `version: "*"`, abstract and inactive status); else `expand_enumerated_compose` → `Some` → use it; else `NoExpansion` with diagnostics "value set '<url>' has no expansion and cannot be expanded without a terminology server; configure HFS_TERMINOLOGY_SERVER or store the ValueSet with an expansion".

Persistence: set `expansion` on the stored content; add `meta.tag {system: "http://heliossoftware.com/fhir/CodeSystem/sof-terminology", code: "materialized-expansion"}`; `storage.update(tenant, &current, content)` (new history version, intended). HTS-fetched definitions: `storage.create`, tagged `fetched-from-terminology-server`, keeping the artifact's own `url`/`version`. Byte cap (D9) on the serialized content before writing. On a version conflict in phase B, re-read once; if the expansion is now present → `AlreadyExpanded`.

Authorization (D12) `[R1-3]`: `principal.scopes.permits("ValueSet", UPDATE)` for expansions of stored ValueSets; `permits("ValueSet", CREATE)` / `permits("ConceptMap", CREATE)` for HTS-fetched definitions; the exact `SmartPermissions` flags follow `authz_middleware`'s mapping for update and create. `None` principal (auth disabled) passes. Strict → `403` with diagnostics naming the scope; lenient → `SkippedNoScope` + `warn!`.

Failure semantics (D10): `strict` (default) aborts the outer write with the error from §6.8; `lenient` warns and continues; `off` never calls the materializer.

#### 2c. Hooks `[R1-5] [R2-2] [R2-3] [R2-4]`

Order on every path, as the table below fixes it: write-free preconditions first (`If-Match`, `If-None-Exist` pre-search), then phase A, then phase B immediately before the storage call that persists the outer resource. The one exception is the conditional create, where the atomic conditional create precedes phase B `[R4-3] [R4-5]`.

| Path | Phase A | Phase B |
|---|---|---|
| `PUT /Library` (`update.rs`) | **after** `if_match_satisfied` (inside the `If-Match` branch, ~line 213) `[R3-2]`; in the no-`If-Match` branch after the guard | immediately before `.update` (~230) / `create_or_update` |
| `POST /Library` without `If-None-Exist` (`create.rs`) | after line 113 | immediately before `storage.create` (~line 220) |
| `POST /Library` with `If-None-Exist` | **after** a pre-search with the `If-None-Exist` criteria `[R3-2]`: a match → return it, nothing else happens (no phase A, no HTS); multiple → 412 as today; no match → phase A (in memory) | the existing **atomic conditional create first** `[R4-3]`: `Exists` (a concurrent create won) → discard the planned writes, return the existing resource, nothing written; `Created` → phase B. A phase B failure here leaves the Library stored and unmaterialized; the response is the **created-unmaterialized 422** `[R6-2]`: `Location`, `ETag` and `Last-Modified` set exactly as the 201 would set them (`public_url_for_request(tenant, [type, id])`, `ResourceHeaders::from_stored`), body an OperationOutcome whose first issue is the phase B error (code per §6.8) and whose second issue is `severity: information`, `code: informational`, `details.coding = {system: sof-terminology, code: created-unmaterialized}`, `diagnostics: "Library/{id} (version {vid}) was created; its terminology dependencies were not materialized: {urls}. Retry with PUT {Location} (the same conditional POST would match the stored Library and return 200 without materializing)."` |
| `PATCH /Library` (`patch.rs`) | on the patched candidate, after its `If-Match` check | immediately before the storage write of the patched resource |
| `POST/PUT/PATCH /ValueSet` | — | `materialize_value_set_candidate` transforms the candidate **before** `check_write`, so the normal validation validates the final candidate `[R3-3]`; the outer write persists it once. |
| batch POST entry (`process_batch_entry`) | after the conditional-create pre-search (if any), then phase A; ValueSet candidates transformed before the entry's `check_write` (~1323) | immediately before `storage.create` (~1381); with `ifNoneExist`, after the entry's atomic `conditional_create` on `Created` only, as the direct path. A phase B failure after `Created` answers the entry with the created-unmaterialized 422 `[R6-2]`: `response.status = "422"`, `response.location`, `response.etag` and `response.lastModified` from the stored Library, `response.outcome` the same OperationOutcome as the direct path. |
| batch PUT entry | after `check_entry_if_match` succeeded (~1527); ValueSet candidates transformed before `check_write` (~1534) | immediately before `create_or_update` (~1542) |
| batch PATCH entry (`process_batch_patch`) | on the patched candidate, after its `If-Match` check | immediately before `.update` (~1795) |
| transaction (`process_transaction`) | pre-pass, see below | none outside persistence |

Tests for the ordering `[R2-2] [R3-2] [R4-3]`: a `PUT /Library` with a stale `If-Match` → 412 with HTS unreachable and the dependency ValueSet byte-identical before and after; a `POST /Library` whose `If-None-Exist` matches → 200 with HTS unreachable and no dependency change; a `POST /Library` whose `If-None-Exist` matches nothing and whose dependency is unexpandable → 422, nothing written; two competing conditional creates with identical criteria and **different** dependency lists (the second made to lose the race via a storage hook) → the loser's dependencies are never written, only the winner's; a phase B failure injected after `Created` → Library stored, 422 lists the dependency, a re-PUT materializes it. **Recovery from the response alone** `[R6-2]`: the test keeps nothing from the request but its body, reads `Location` and `ETag` from the 422, asserts the OperationOutcome carries the `created-unmaterialized` coding and says the create committed, GETs `Location` → 200 with the submitted Library and no expansion on its dependency, PUTs the same body to `Location` with `If-Match` from the 422's `ETag` → 200 and the dependency ValueSet now carries the expansion, then `$sql-run` on it succeeds; and, as the negative, repeating the identical conditional POST after the 422 → 200 (pre-search match) with the dependency still unexpanded, which is why the diagnostics name the PUT. The batch form repeats the GET and PUT using only `response.location` and `response.etag` of the failed entry.

Transaction pre-pass `[R2-3] [R2-4] [R3-1] [R3-3]` runs **before** `process_transaction`'s existing per-entry `check_write` loop (~line 732) and before `process_transaction_with_patch_validator`, so the validation loop validates the transformed candidates. It walks `entries_for_processing` in the **executor's execution order** (the FHIR transaction order the executors already sort into: DELETE, POST, PUT/PATCH), transforming candidates only, never writing, and builds a `BundleOverlay` (§6.14):

1. **Index in execution order.** For each entry, record into the overlay (§6.14): a DELETE by `Type/id` → read the stored resource and record its identity **and** its `(url, version)` as suppressed `[R4-2]`; a PUT by `Type/id` → read the stored resource (if any) and record its old `(url, version)` as suppressed when the candidate changes either, then record the candidate under its `Type/id` and its `(url, version)`; a POST candidate **without** `ifNoneExist` under its `(url, version)`. Collect in-bundle Library candidates (`bundle_libraries`). A **conditional** PUT or DELETE (criteria-based target) on a ValueSet, ConceptMap or Library whose target is decided only inside the executor, and a POST with `request.ifNoneExist` on one of those types `[R6-1]` (the executor may match a stored resource and discard the submitted body, so indexing the candidate would let the overlay claim an expansion or a dependency list the stored resource does not have), is recorded as `Undetermined` and is **not** indexed under its canonical or id; if the bundle performs any materialization resolution (steps 2–3 or a PATCH preparation) while an `Undetermined` entry exists for a terminology type or Library, strict mode rejects the transaction up front ("conditional operations on ValueSet, ConceptMap or Library cannot be combined with Library materialization in one transaction; use explicit ids"); lenient proceeds `[R4-2]`. A conditional POST on any other resource type, and a conditional POST on a terminology type or Library in a bundle that performs no materialization resolution at all, are untouched `[R6-1]`.
2. **Discover dependencies, including from PATCH entries** `[R5-3]`. `bundle_libraries` holds every POST/PUT Library candidate plus, for every PATCH entry on a Library, a **provisional candidate** obtained by applying the entry's patch (JSON Patch or FHIRPath Patch, through the same application code `patch.rs` uses for direct PATCH) to the Library as the overlay-then-storage view sees it at that point in execution order. The provisional candidate is used only to discover dependency URLs; it is never persisted.
3. **Transform ValueSet candidates.** For each in-bundle ValueSet POST/PUT candidate without an expansion, `materialize_value_set_candidate(candidate, bundle_libraries)`; the entry's resource and the overlay entry are replaced by the result.
4. **Plan Library materialization.** For each in-bundle Library POST/PUT candidate, `plan_library_materialization(library, resolver = overlay-then-storage, allow_side_writes = false)`. Resolution consults the overlay **before** storage, so a ValueSet created, updated or patched earlier in execution order is seen in its in-bundle state, and one deleted earlier is unresolvable. Any dependency that would need a write outside the bundle (a stored unexpanded ValueSet not in the bundle, or a definition only HTS holds) is a strict 422 up front, naming the dependency and the remedy ("include the ValueSet in the transaction, or store it first"); lenient proceeds with a warning.

The same overlay instance is handed to `RestPatchValidator`, so PATCH preparation inside the executor (below) sees the same picture and extends it. For a Library PATCH, the executor's **actual** prepared candidate must declare a dependency set whose terminology members were all expanded in step 3 (or already carried expansions); if a concurrent change between the pre-pass and the executor makes the actual candidate declare something the pre-pass did not see, preparation reports `NeedsSideWrite` and strict mode rolls the transaction back `[R5-3]`.

No entry is ever injected into the bundle `[R2-4]`, so `process_transaction`'s positional zips at lines 766, 810 and 859, the per-entry audit events and the write notifications are untouched, and the response keeps exactly one entry per request entry in order.

Transaction PATCH `[R2-3]`: `PatchCandidateValidator` in `helios-persistence` (`core/transaction.rs:28`) only validates. Add a second method with a default implementation:

```rust
async fn prepare_patch_candidate(&self, tenant, version, resource_type, candidate: Value) -> Result<Value, Value> {
    self.validate_patch_candidate(tenant, version, resource_type, &candidate).await.map(|()| candidate)
}
```

The three transaction executors (sqlite, postgres, mongodb) call `prepare_patch_candidate` in execution order and persist the returned candidate. `RestPatchValidator { validation, overlay: Arc<BundleOverlay> }` overrides it `[R3-1] [R3-3]`: for a ValueSet candidate apply T2 (resolving dependent Libraries through overlay-then-storage); for a Library candidate run phase A through overlay-then-storage with `allow_side_writes = false` and surface `NeedsSideWrite` as the entry's OperationOutcome (rolling the transaction back, as any PATCH refusal does); **then** run `check_write` and the ViewDefinition guard on the transformed candidate; then record the prepared candidate into the overlay under its `Type/id` and `url` so later entries see it. The executors already call the validator at the point where the patched candidate is known, which is inside the same DB transaction; the overlay is what gives the REST layer the uncommitted view, since the REST layer never reads the executor's connection. This is the only persistence-crate change in the plan.

Tests `[R2-3] [R2-4] [R3-1] [R3-3]`: transaction with a new ValueSet + a new Library depending on it (both bundle orders) → one commit, expansion present, response has exactly the request's entries in order; transaction that POSTs a compose-only ValueSet then PATCHes a stored Library to depend on it → the pre-pass discovers the dependency from the provisional PATCH candidate, step 3 expands the ValueSet, the executor's PATCH preparation finds it expanded, and the transaction commits `[R5-3]`; the same transaction where a concurrent direct PUT changes the stored Library between pre-pass and executor so the actual patched candidate declares an extra unexpanded dependency → strict rollback, nothing written `[R5-3]`; transaction that PUTs a new version of a stored ValueSet then PATCHes a Library → the Library sees the PUT candidate, not storage; transaction that DELETEs a ValueSet then POSTs a Library depending on it → strict 422 (unresolvable); transaction PATCH on a ValueSet that a stored Library depends on → expanded and validated; transaction PATCH on a Library adding a dependency on a stored unexpanded ValueSet → strict 422, nothing written; a transformed ValueSet candidate that fails `check_write` (e.g. an HTS expansion violating the tenant's validation profile) → the entry's 422, transaction rolled back; transaction with a Library pinning a canonical only a mock HTS holds → strict 422 before persistence; batch with the same → definition stored via a direct, validated side write.

#### 2d. Budget and config `[R1-6]`

| Variable | Default | Meaning |
|---|---|---|
| `HFS_SOF_TERMINOLOGY_MATERIALIZE` | `strict` | `strict` / `lenient` / `off` (D10) |
| `HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS` | `20` | Total wall time one write may spend materializing **all** its dependencies, sequentially |
| `HFS_SOF_TERMINOLOGY_MAX_EXPANSION_MEMBERS` | `200000` | D9 member cap on distinct emitted rows (visited-structure cap is 4 × this, fixed) |
| `HFS_SOF_TERMINOLOGY_MAX_ARTIFACT_BYTES` | `8388608` | D9 per-artifact byte cap: HTS response stream, serialized resource before persistence, and the plan-time size estimate of any terminology artifact `[R3-5]` |
| `HFS_SOF_TERMINOLOGY_MAX_JOB_BYTES` | `134217728` | D9 per-job retained-bytes budget for flattened relations `[R3-5]` |

Rules: `validate()` rejects a budget ≥ `request_timeout`; the per-call HTS timeout is `min(remaining budget, 30 s)`; the budget is shared across all dependencies of one write and across all entries of one bundle; exceeding it is a strict 422 ("materialization exceeded N s; raise HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS and HFS_REQUEST_TIMEOUT together, or pre-store the expansion"). Documentation states the pairing explicitly. A separate async materialization job is listed as a follow-up (§9) for deployments with very large value sets.

#### 2e. Tests (PR 2)

- Unit: engine selection; tag added; history version incremented; byte cap and member cap each reject with nothing written; strict vs lenient vs off; budget exhaustion after the first of two dependencies writes nothing (phase A) `[R1-2]`; phase B failure on the second dependency leaves the first expanded and the Library unwritten `[R1-2]`.
- Authorization `[R1-3]`: principal with `user/Library.cu` only → strict 403 naming `ValueSet` write scope, lenient 201 + `SkippedNoScope`; principal with `user/*.cud` → materialized; no principal → materialized.
- Integration (SQLite): PUT enumerated ValueSet then PUT Library → ValueSet gains `expansion` + tag, run succeeds; PUT Library first then POST ValueSet → T2 transforms the candidate, single write, `If-Match` on a later PUT still works `[R1-2]`; PUT a Library pinning `url|2026` held only by a mock HTS → definition stored + expanded; HTS down + strict → 422 on the Library write; lenient → 201 and the run 422s; `context` ValueSet never persisted.
- Bundles `[R1-5]`: batch with POST Library and PUT ValueSet entries → both materialized, a failing entry does not fail its siblings; transaction with ValueSet + Library in either order → one commit, expansion present; transaction with a Library depending on a stored unexpanded ValueSet not in the bundle → strict 422 before persistence; transaction whose later entry fails → no expansion written.
- Optional live test gated by `HFS_TERMINOLOGY_SERVER`.

### PR 3 — surfaces

- **UI**: `DependencyLookup` learns `ValueSet/` and `ConceptMap/`; resolver tries four types in REST order; `TableTarget::ValueSet { id, name, expanded: bool, members: Option<usize> }`, `TableTarget::ConceptMap { id, name, mappings: usize }`; kind chip and "no expansion yet" warning; Columns card shows the fixed relation columns; "Add table" picker lists stored ValueSets and ConceptMaps. Playwright: one route-interception test per kind.
- **Capability**: add the ValueSet and ConceptMap StructureDefinition URLs to `context.targetProfile`; update the test.
- **Docs**: `work-with-sof` skill (terminology section, the four env vars, budget pairing with `HFS_REQUEST_TIMEOUT`, scopes needed, refresh procedure, `HFS_SOF_SQLQUERY_MAX_VDS` wording); `book/src/ch06-sql-on-fhir.md`, `book/src/components/sql-on-fhir.md`; `run-hfs-and-hts` skill; `capability.rs` module doc.

---

## 6. Detailed designs

### 6.1 ValueSet expansion → rows (`flatten_value_set`) `[R1-4] [R1-7]`

```
if expansion absent                       → Err(NoExpansion)
if expansion.offset present               → Err(Incomplete "expansion carries offset=N")
visited = 0; members = []; seen = set(); bytes = 0
stack = [(expansion.contains, "expansion.contains", depth 0)]          // explicit stack, no recursion [R3-4]
while (entries, path, depth) = stack.pop():
  if depth > 32 → Err(Incomplete "nesting deeper than 32 at {path}")
  for (i, entry):
    visited += 1 > limits.max_visited → Err(TooManyEntries)             // every entry, abstract or not [R2-5]
    if entry.abstract != true:
      system = non-empty string else Err(Incomplete "{path}[i] has no system")
      code   = non-empty string else Err(Incomplete "{path}[i] has no code")
      key = (system, entry.version, code)
      if seen.insert(key):
        members.push(row); members.len() > limits.max_members → Err(TooManyMembers)
        bytes += row_bytes(row); bytes > limits.max_relation_bytes → Err(TooManyBytes)   // [R3-5]
    if entry.contains is a non-empty array: stack.push((entry.contains, "{path}[i].contains", depth + 1))
if expansion.total is a number and total > visited → Err(Incomplete "total={total} but {visited} entries at all depths")
return (members, bytes)
```

- `total` is compared against **all** `contains` entries at all depths, abstract ones included, which is what term-24 says and what makes the upstream fixture (`total: 3` = 1 abstract + 2 members) pass. Members are counted separately for the cap.
- Limits `[R2-5] [R3-4] [R3-5]`: `max_visited` (4 × `max_members`) counts every `contains` entry at every depth; `max_members` bounds distinct emitted rows at insertion; `max_relation_bytes` bounds the retained row bytes as the sum of `row_bytes` over emitted rows (`row_bytes` = UTF-8 bytes of every string column × 3 + 128, the same function the job budget charges, §6.4) `[R6-5]`; depth is bounded at 32 by the explicit stack. The same function with the same limits serves stored, HTS-returned and `context` artifacts.
- `abstract: true` with a `code` still contributes no row. `inactive` absent → `None`; `false` present → `Some(false)`.

### 6.2 ConceptMap → rows (`flatten_concept_map`)

Shape detection per group:

| Element | R5 / R6 | R4 / R4B |
|---|---|---|
| source system + version | `group.source` canonical split at `\|` | `group.source` + `group.sourceVersion` |
| target system + version | `group.target` split at `\|` | `group.target` + `group.targetVersion` |
| relationship | `target.relationship` | `target.equivalence` mapped |
| no map | `element.noMap == true` | `target.equivalence == "unmatched"` |
| set-valued | `element.valueSet`, `target.valueSet` → reject | n/a |
| conditional | `target.dependsOn[]` → reject | `target.dependsOn[]` → reject |
| products | `target.product[]` → reject | `target.product[]` → reject |

R4 `equivalence` → R5 `relationship`:

| R4 | R5 | Row |
|---|---|---|
| `relatedto` | `related-to` | target |
| `equivalent`, `equal` | `equivalent` | target |
| `wider`, `subsumes` | `source-is-narrower-than-target` | target |
| `narrower`, `specializes` | `source-is-broader-than-target` | target |
| `inexact` | `related-to` | target |
| `unmatched` | — | noMap row |
| `disjoint` | `not-related-to` | target |
| other | — | Err(Unsupported "unknown equivalence") |

```
visit() = visited += 1; visited > max_visited → Err(TooManyEntries)                 // every structure [R3-4]
for (gi, group): visit(); src = split(group.source)+sourceVersion; src.system empty → Err(Unsupported "group[gi] has no source")
  tgt = split(group.target)+targetVersion (may be None)
  for (ei, element): visit(); element.valueSet → Err; code non-empty else Err
    if R5 noMap: emit(noMap row)
    for (ti, target): visit()
      target.valueSet / dependsOn / product → Err(with path)      // presence check, arrays not traversed
      rel = R5 relationship | R4 map(equivalence); R4 unmatched → emit(noMap row), continue
      target.code non-empty else Err
      emit(row)
emit(row) = if dedupe-set inserts: rows.push; rows.len() > max_members → Err(TooManyMembers);
            bytes += row_bytes(row) > max_relation_bytes → Err(TooManyBytes)             // per insertion [R2-5] [R3-5]
group.unmapped ignored; zero groups → zero rows
```

### 6.3 Local expansion eligibility and matching (`expand_enumerated_compose`) `[R1-8] [R2-1] [R2-5]`

Local expansion is used **only when no terminology server is configured** (D3) and, at run time, for a `context` ValueSet (D7). It is eligible if and only if **all** of the following hold; otherwise `Ok(None)` and the caller reports `NoExpansion` (write path) or 422 (`context`):

1. `compose` exists and `compose.include` is a non-empty array.
2. `compose.inactive` is **absent** (either value needs activity knowledge: `false` must exclude inactive codes, `true` must mark them).
3. `compose.lockedDate` is absent (a date-pinned expansion needs code system versions).
4. Every `include` has a non-empty `system`, a non-empty `concept[]`, no `filter`, no `valueSet`, and a `version` that is absent or a concrete string (not `"*"`). An include with `system` and no `concept` is a whole-system selection → ineligible.
5. Every `exclude` likewise: non-empty `system`, non-empty `concept[]`, no `filter`, no `valueSet`, no `version: "*"`.
6. Every concept has a non-empty `code`.
7. Concept `designation` is ignored (does not affect membership).
8. `[R3-6]` For each code system, every include and exclude entry naming it carries the **same** `version` value (all absent, or all the same string). A versioned exclude against an unversioned include (or vice versa) cannot be evaluated without knowing which code system version the include denotes, so it is ineligible.
9. `[R3-4]` The eligibility check itself counts every include, exclude and concept it inspects against `max_visited`, so an enormous compose is rejected without being read in full.

What a local expansion asserts, and does not: it asserts membership exactly as enumerated by the author. It never sets `inactive` and never marks `abstract`, because it cannot know either; the `inactive` column is therefore null for every row, which the spec defines as "the expansion does not say". Any compose that needs more than that is routed to HTS, and without HTS is a clear failure rather than an approximation.

Expansion:

```
members = ordered set keyed by (system, version, code); visited carried over from the eligibility check
for include in compose.include: visit(); for concept in include.concept: visit()
    if members.insert((include.system, include.version, concept.code), display = concept.display)
       and members.len() > max_members → Err(TooManyMembers)                       // at insertion [R2-5]
for exclude in compose.exclude: visit(); for concept in exclude.concept: visit()
    remove exactly (exclude.system, exclude.version, concept.code)   // rule 8 guarantees versions agree per system [R3-6]
expansion = { identifier, timestamp: now, total: members.len(),
              parameter: [ {name:"version", valueUri: "{system}|{version}"} for each distinct versioned include ],
              contains: [ {system, version?, code, display?} ... ] }   // flat, no abstract, no inactive
```

Because of rule 8 there is no versioned-against-unversioned case to resolve: exclusion is an exact key match. Tests cover an unversioned include with a versioned exclude → `Ok(None)` (ineligible), and the same compose through a mock HTS → expanded by HTS.

### 6.4 Plan node and fetcher

`PlanNode::Relation` carries **rows, not the resource**, as `Arc<[Value]>` shared through the `JobResolver` `[R2-6]`: the export worker must not need the flattener's error path, the plan stays `Clone`, and cloning a plan clones a pointer.

Plan-time byte accounting `[R3-5] [R4-4]`: before flattening, every terminology artifact (stored, `context`, or HTS-returned) passes `estimate_json_bytes(artifact, max_artifact_bytes)`; the `JobResolver` charges that estimate to the job budget on caching the artifact, charges the relation's bytes (the flattener's returned sum of `row_bytes`, i.e. UTF-8 bytes of every string column × 3 + 128 per row, the same number already compared against `max_relation_bytes`) on flattening `[R6-5]`, and **drops the terminology artifact's JSON once the relation exists**, keeping the `ResolvedArtifact::Relation` entry (kind, identity, schema, rows) so no later plan in the job needs the JSON again `[R5-1]` (ViewDefinition and SQLView resources, which the plan needs verbatim, stay cached and are charged too). The request fails (422, `too-costly`) when the sum would exceed `HFS_SOF_TERMINOLOGY_MAX_JOB_BYTES`. This is an accounting budget: it tracks every retained structure with a conservative factor, it does not measure allocations. An expansion made mostly of abstract entries is therefore charged for its JSON while parsed, and released when flattened.

Fetcher order `ViewDefinition`, `Library`, `ValueSet`, `ConceptMap`; stop at first `Ok` or non-`NotFound` error. On an S3 backend without search, `resolve_by_scan` runs per type.

### 6.5 Materialization in `execute_plan` `[R1-9]`

```rust
PlanNode::Relation { internal_name, kind, name, schema, rows } => {
    engine.create_table(internal_name, schema)?;
    let shared = Arc::clone(rows);                                                     // Arc<[Value]>, 'static [R1-9] [R2-6]
    let owned = futures::stream::iter((0..shared.len()).map(move |i| Ok(shared[i].clone())));
    let (engine2, _n) = match engine.insert_rows(internal_name, schema, Box::pin(owned),
                                                 limits.max_source_rows_per_vd).await {
        Ok(v) => v,
        Err(SqlQueryError::RowCapExceeded { max }) => return Err(SqlQueryError::DependencyRowCapExceeded {
            label: dependency_label(&labels_by_target, internal_name, name).to_string(), kind: *kind, view: name.clone(), max }),
        Err(e) => return Err(e),
    };
    engine = engine2; leaf_schemas.push(schema.clone());
    engine = materialize_labels(engine, internal_name, &labels_by_target).await?;
}
```

`json_to_sqlite_value` already maps `true/false` → 1/0 and `null` → NULL. The spec's `inactive IS NULL OR inactive = FALSE` works in SQLite.

### 6.6 Provenance (term-21/22)

A materialized expansion carries `expansion.identifier`, `expansion.timestamp` and `expansion.parameter` (from HTS, or synthesized locally). The ValueSet's history records when HFS wrote it. A `debug!` line per relation names URL, version, member count and `expansion.identifier`.

### 6.7 `context` handling (D7)

| `context` artifact | Behaviour |
|---|---|
| ValueSet with `expansion` | flattened as-is (term-23) with `offset`/`total`/cap checks (term-24, D9) |
| ValueSet compose-only, eligible (§6.3) | expanded locally for this request; **not persisted** |
| ValueSet compose-only, not eligible | 422 "supply the ValueSet with an expansion, or store it so the server can materialize it" |
| ConceptMap | flattened as-is |
| Duplicate URL / no `url` | 400 (existing) |

### 6.8 Errors and status codes

| Situation | Status | Issue code |
|---|---|---|
| canonical resolves to nothing | 404 | `not-found` (message widened) |
| stored ValueSet without expansion at run time | 422 | `invalid` |
| supplied expansion incomplete (offset / total) | 422 | `invalid` |
| ConceptMap with dependsOn / product / valueSet / no source | 422 | `invalid` |
| expansion or map over the member cap (any source) | 422 | `too-costly` |
| relation over the per-dependency row cap | 422 | `processing` (existing variant, kind-aware text) |
| materialization failed on a write (strict): unexpandable, HTS error, budget exceeded | 422 | `invalid` (HTS network/5xx: `exception`) |
| serialized expansion over the byte cap | 422 | `too-costly` |
| caller lacks the write scope materialization needs (strict) | 403 | `forbidden` |
| transaction needs a side write it cannot perform (strict) | 422 | `invalid` |
| transaction mixes a conditional POST/PUT/DELETE on a terminology type or Library with materialization (strict) `[R6-1]` | 422 | `invalid` |
| conditional create committed, phase B failed (created-unmaterialized) `[R6-2]` | 422, with `Location`/`ETag`/`Last-Modified` of the created Library | first issue: the phase B error's own code (storage error → `exception`); second issue: `informational` with `details.coding` `created-unmaterialized` |

Aggregated Phase 1 errors keep today's `MultiIssue` 400 (open question Q5).

Kind-aware row cap wording: ValueSet → `dependency '{label}' (ValueSet {name}) has more than {max} members; SQL queries materialize each dependency in full before the query's WHERE runs. Narrow the value set, or raise HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD.` ConceptMap → same with "mappings" / "narrow the concept map". ViewDefinition / SQL View unchanged.

### 6.9 Export `[R1-1]`

Kick-off creates one `JobResolver`; every `prepare_named_sqlquery` resolves through it; relation rows travel in the plans as shared `Arc`s; the worker is unchanged. The memo guarantees term-19 across subjects regardless of concurrent updates and keeps export memory at one copy per artifact `[R2-6]`. Test described in PR 1d.

### 6.10 UI resolver parity

Same four types, same order, same `canonical_matches` semantics as the REST fetcher.

### 6.11 Write-path sequencing (T1 and T2 together) `[R1-2]`

```
direct PUT /Library:
  parse → check_write → If-Match evaluated (412 ends here, HTS never contacted)
        → scope check → phase A (transitive, reads only, planned writes validated with check_write)
        → phase B (dependency writes) → outer Library write                                   [R2-2] [R3-2] [R3-3]
direct POST /Library with If-None-Exist:
  parse → check_write → pre-search → match: return it, nothing written, HTS never contacted
                                   | no match: scope check → phase A (in memory) → atomic conditional create
                                                → Exists: discard planned writes, return existing
                                                → Created: phase B                              [R4-3]
direct PUT /ValueSet:
  candidate = materialize_value_set_candidate(candidate) → check_write(candidate) → outer write (one write, normal If-Match)
batch entry: same as direct, per entry, failure localized to the entry
transaction: pre-pass in execution order over a BundleOverlay (index, conditional POST/PUT/DELETE on terminology types
             or Libraries = Undetermined → strict reject if anything resolves [R6-1]
             → discover deps incl. provisional PATCH candidates
             → transform ValueSet candidates → plan Libraries, no side writes, nothing injected)
             → existing per-entry check_write on the transformed candidates
             → persistence (PATCH candidates prepared through the same overlay, then validated)
             → commits everything or nothing                                                   [R2-3] [R2-4] [R3-1] [R3-3]
```

### 6.12 Authorization `[R1-3]`

| Materialization action | Required scope on the caller's principal |
|---|---|
| add `expansion` to a stored ValueSet | update permission on `ValueSet` |
| store a ValueSet definition fetched from HTS | create permission on `ValueSet` |
| store a ConceptMap definition fetched from HTS | create permission on `ConceptMap` |
| transform the ValueSet candidate the caller is itself writing (T2) | none beyond the caller's own write, already authorized by `authz_middleware` |

Checked in phase A before any read of HTS. `Option<&Principal>` is obtained from request extensions as `purge.rs`/`reindex.rs` do; `None` passes. Audit and subscription write events are emitted for each dependency write exactly as a direct write would emit them, attributed to the same principal.

### 6.13 Transitive materialization at write time `[R2-7]`

Phase A reuses the iterative walker behind `build_plan` rather than a second traversal: refactor the walk into `walk_dependency_graph(resolver, subject, visitor)` where `build_plan`'s visitor builds plan nodes and the materializer's visitor records every ValueSet/ConceptMap reached (at any depth, through SQLViews) and whether it needs expansion. Cycle detection, `MAX_GRAPH_DEPTH` (16) and `sof_sqlquery_max_vds` apply unchanged, so a Library write cannot be made to walk an unbounded graph. The diagnostic "re-save the Library" is then true at every depth.

Test: store ViewDefinition V, an unexpanded enumerated ValueSet X, a SQLView S depending on X; then PUT a SQLQuery Q depending on S. Expect X expanded after the PUT and Q runnable. Also: a cycle S1 → S2 → S1 reached from Q → the write is rejected with the existing cycle diagnostic, nothing written.

### 6.14 `BundleOverlay`: the transaction-scoped resolver `[R3-1] [R4-1] [R4-2]`

```rust
pub(crate) struct BundleOverlay {
    by_id:        Mutex<HashMap<String /*Type/id*/, OverlayEntry>>,
    by_canonical: Mutex<HashMap<String /*url*/, Vec<CanonicalCandidate>>>,   // every in-bundle candidate for a URL, in execution order
    suppressed:   Mutex<HashSet<Identity>>,                                  // stored identities deleted or replaced earlier in the bundle
    undetermined: Mutex<Vec<(BundleMethod, String /*resource type*/)>>,      // conditional PUT/DELETE, and POST with ifNoneExist, whose outcome the executor decides [R6-1]
}
struct CanonicalCandidate { version: Option<String>, id: Option<String>, seq: usize, candidate: Value }
struct Identity { resource_type: String, id: String, url: Option<String>, version: Option<String> }
enum OverlayEntry { Candidate(Value), Deleted }

impl ArtifactFetcher for OverlayThenStorage<'_> {
    // Type/id reference: by_id hit → Candidate / NotFound(Deleted); miss → storage, then NotFound if its identity is suppressed.
    // Canonical reference (url[|version]) [R5-2]:
    //   overlay = by_canonical[url] filtered by canonical_matches (exact version when pinned)
    //   storage = canonical_candidates(...) filtered by canonical_matches, MINUS every candidate whose Identity is in `suppressed`
    //   selection over the union: pinned → the single match (overlay beats storage for the same identity);
    //                             unpinned → an overlay candidate if any (highest seq), else the newest surviving storage candidate.
    //   Suppression is applied to the candidate set before selection, never to a pre-selected winner.
}
```

Rules:

- **Identity, not URL, is the key** `[R4-1]`. Two in-bundle ValueSets with the same `url` and different `version` are two candidates; a reference pinned to either resolves to that one; an unpinned reference resolves to the candidate latest in execution order, mirroring the storage rule "newest wins". Test: a transaction with `vs|1` and `vs|2` and two Libraries, one pinned to each → each Library's plan carries its own version's rows.
- **Supersession is recorded by previous identity and applied before selection** `[R4-2] [R5-2]`. The pre-pass reads the stored resource behind every DELETE and PUT by id before recording, and the canonical lookup removes suppressed identities from the full storage candidate list before choosing, so a deleted or replaced artifact is never resurrected and a surviving older version is never hidden. Tests: DELETE `ValueSet/x` then POST a Library depending on `x`'s canonical (only version) → unresolvable → strict 422; stored `vs` v1 and v2, the bundle deletes v2, an unpinned Library reference → resolves to **v1**; PUT `ValueSet/x` changing `version` from 1 to 2 then a Library pinned to `|1` → unresolvable; pinned to `|2` → resolves to the candidate.
- **Undetermined targets are refused, not guessed** `[R4-2] [R6-1]`. A conditional PUT or DELETE on a terminology type or Library whose target is only decided in the executor, and a conditional POST (`ifNoneExist`) on one of those types whose outcome (`Created` with the submitted body, or `Exists` with a stored resource the bundle never saw) is only decided in the executor, make every canonical and id resolution in that bundle unsafe, so strict mode rejects the transaction before persistence whenever any materialization resolution would occur; lenient proceeds with a warning and no materialization for that bundle. The executor's own `ifNoneExist` handling (`pin_forward_references`, `create_if_none_exist` in `core/transaction.rs`) is untouched. Tests `[R6-1]`: stored `ValueSet/x` (url `vs`, no expansion); a transaction with a POST ValueSet `vs` carrying `ifNoneExist: url=vs` and an expansion, plus a POST Library depending on `vs` → strict 422 before persistence naming the conditional entry, `ValueSet/x` byte-identical afterwards, and in lenient mode the bundle commits with the Library stored, `ValueSet/x` still unexpanded and a warning in the Library entry's outcome; the same bundle without the Library (nothing to resolve) → commits as today.
- `prepare_patch_candidate` records its prepared candidate under `Type/id` and `(url, version)` before returning, so later entries see it.
- Remaining stated limit: a POST without `id` is matched by `(url, version)` only, since its server-assigned id is unknown until commit.

### 6.15 Validation of generated and transformed resources `[R3-3]`

| Resource | Where it is validated | What validates it |
|---|---|---|
| ValueSet candidate transformed by T2 (direct, batch, transaction PATCH) | immediately after the transform, before persistence | `ValidationService::check_write` + the ViewDefinition guard (a no-op for ValueSets), i.e. exactly the path an untransformed write takes |
| ValueSet candidate transformed in the transaction pre-pass | the existing per-entry `check_write` loop, which now runs after the pre-pass | same |
| Stored ValueSet gaining an `expansion` in phase B | at the end of phase A, before any phase B write | `check_write` on the full updated content |
| Definition fetched from HTS (ValueSet or ConceptMap) | at the end of phase A | `check_write` on the fetched resource against the tenant's FHIR version and validation profile; a shape the tenant does not accept fails closed |

A validation failure anywhere in phase A is a 422 with nothing written, carrying the validator's own OperationOutcome issues plus an issue naming the dependency label.

---

## 7. Test fixtures (verbatim from the spec page)

`conditions` ViewDefinition:

```json
{"resourceType":"ViewDefinition","url":"https://example.org/ViewDefinition/conditions","status":"active","resource":"Condition",
 "select":[{"column":[
   {"path":"subject.getReferenceKey(Patient)","name":"patient_id","type":"string"},
   {"path":"code.coding.first().system","name":"system","type":"uri"},
   {"path":"code.coding.first().version","name":"version","type":"string"},
   {"path":"code.coding.first().code","name":"code","type":"code"}]}]}
```

Seed four Conditions for `p1`..`p4` with `sct|22298006`, `icd-10|I21`, `sct|73211009`, `sct|102499006`. ValueSet and ConceptMap: the two JSON blocks from the spec page's Examples section, unchanged (the ValueSet has `total: 3`, one abstract parent, two members). Libraries: `CardiovascularDiseasePatientsQuery` and `ConditionsToIcd10Query` from the upstream `sql-query-examples.fsh`.

| Query | Rows |
|---|---|
| Cardiovascular | `(p1, 22298006)`, `(p2, I21)` |
| ICD-10 | `(p1, 22298006, I21, equivalent)`, `(p2, I21, null, null)`, `(p3, 73211009, E14, source-is-broader-than-target)`, `(p4, 102499006, null, null)` |

Row order not asserted.

---

## 8. Open questions for review

| # | Question | Plan's default |
|---|---|---|
| Q1 | Synchronous write-path expansion (D2) vs an async materialization job for very large value sets. | Synchronous within a 20 s budget; async job as a follow-up. |
| Q2 | T2 expands only when a stored Library depends on the ValueSet, or every ValueSet without an expansion? | Only when depended on. |
| Q3 | Auto-store definitions fetched from HTS (D4)? | Yes, tagged. |
| Q4 | Refresh mechanism. | PUT the ValueSet without `expansion` (T2 re-expands); explicit operation later. |
| Q5 | Aggregated Phase 1 errors mixing 404 and 422 report 400 today. Keep? | Keep. |
| Q6 | Caps: 200 000 members, 8 MiB bytes. | As stated. |
| Q7 | Local expander follows `compose.include.valueSet` into other stored expanded ValueSets? | No (v1). |
| Q8 | HTS default page size for `$expand` when no `count` is sent. | Unverified; handled by the `total` check and one bounded re-request. |
| Q9 | Transaction with a Library depending on a stored unexpanded ValueSet outside the bundle, or on a definition only HTS holds: strict reject (default) or allow a side write / inject an entry? | Strict reject; nothing injected (D4, R2-4). |
| Q10 | D12 policy: caller's scopes (default) vs system-authorized materialization. | Caller's scopes. |
| Q11 | *(resolved in revision 3)* Preconditions that need no write (`If-Match`, `If-None-Exist` pre-search) now run before phase A, so a 412 or a conditional no-op never contacts HTS. | — |
| Q13 | *(resolved in revision 4)* The conditional create now precedes phase B, so no orphan writes exist; the remaining asymmetry (Library stored but unmaterialized after a phase B failure) is stated in D13. | — |
| Q15 | Strict rejection of transactions mixing conditional PUT/DELETE on terminology types or Libraries with materialization (R4-2). Acceptable, or should the pre-pass run the conditional search itself (non-atomic)? | Reject. |
| Q19 | Conditional POST (`ifNoneExist`) on a terminology type or Library in a materializing transaction is likewise rejected (R6-1). The alternative is an executor-side hook for POST entries (a `prepare_create_candidate` beside `prepare_patch_candidate`) that would transform and plan after the in-transaction `find_matches` decides the outcome; it would also lift the Q15 restriction, at the cost of moving materialization (and HTS calls, in lenient mode) inside the open storage transaction. | Reject in v1; executor-side hook as a follow-up. |
| Q16 | Unpinned reference with two in-bundle versions resolves to the latest in execution order (mirrors "newest wins" in storage). Acceptable, or reject as ambiguous? | Latest in execution order. |
| Q17 | Provisional application of Library PATCHes in the pre-pass (R5-3) versus rejecting transaction PATCHes that add terminology dependencies. | Provisional application, with strict rollback when the actual candidate differs. |
| Q18 | D13(b): on ordinary POST/PUT, expansions may remain after a failed outer write. Acceptable as stated, or require the conditional-create ordering (outer write first) everywhere, trading it for case (c)'s stored-but-unmaterialized Library on every path? | As stated. |
| Q14 | Job byte budget default 128 MiB and the fixed 32-level nesting bound. | As stated. |
| Q12 | D3 inversion: with HTS configured, every ValueSet goes to HTS, even a trivial enumerated one. Acceptable latency on Library writes? | Yes; the budget bounds it and HTS answers an enumerated inline compose in milliseconds. |

---

## 9. Out of scope / follow-ups

- Unmatched `context` entry → 400 (spec rule; not implemented for any kind today).
- Async materialization job and an explicit refresh operation.
- Executor-side `prepare_create_candidate` hook so conditional POST/PUT/DELETE on terminology types can coexist with materialization in one transaction (Q15, Q19).
- Local expansion of filter-based composes.
- Upstream sql-on-fhir.js conformance wiring (none published yet).
- Changing existing 422 paths from `processing` to `invalid`.
- `url` vs `url|version` dedupe-by-identity in `build_plan`.
- sof-server.

---

## 10. Review checklist

- [ ] D1–D13 accepted or amended (§2)
- [ ] Q1–Q19 answered (§8)
- [ ] PR slicing accepted (§0)
- [ ] R4 `equivalence` mapping accepted (§6.2)
- [ ] Local expansion eligibility rules accepted (§6.3)
- [ ] Error table accepted (§6.8)
- [ ] Write-path sequencing, precondition ordering and transaction rule accepted (§5 PR 2c, §6.11)
- [ ] Persistence-crate change (`prepare_patch_candidate` on `PatchCandidateValidator`) accepted (§5 PR 2c)
- [ ] Transitive write-time walk via a shared walker accepted (§6.13)
- [ ] `BundleOverlay` identity keying, supersession and undetermined-target rules accepted (§6.14)
- [ ] Conditional-create ordering and the four permitted partial-write cases accepted (D13)
- [ ] Created-unmaterialized 422 shape (`Location`/`ETag` headers, `created-unmaterialized` coding, PUT as the only remedy) accepted (§5 PR 2c, §6.8)
- [ ] Conditional POST treated as `Undetermined` in materializing transactions accepted (§6.14, Q19)
- [ ] `ResolvedArtifact` cache contract (no refetch within a job) accepted (§4.3, §5 PR 1b)
- [ ] Suppression-before-selection and the `canonical_candidates` refactor accepted (§6.14, §5 PR 1c)
- [ ] Validation table for generated/transformed resources accepted (§6.15)
- [ ] Authorization policy accepted (§6.12)
- [ ] Config names, defaults and the budget pairing accepted (§5 PR 2d)
- [ ] Tag system URL `http://heliossoftware.com/fhir/CodeSystem/sof-terminology` accepted
- [ ] Test fixtures accepted (§7)
