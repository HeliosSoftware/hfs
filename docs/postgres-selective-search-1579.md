# Selective PostgreSQL searches (#1579)

The final change extends PR #1597's query 2 optimization with a bounded native
query 1 reverse chain and a resource-scoped quantity fence for query 3. It avoids
application-level Observation body enumeration for the eligible Patient `_has`
search, preserves Q2's composite fence, and preserves Q3's complete raw/canonical
quantity predicate. The selective-search fix itself adds no schema migration or index.

The restored full corpus verifies the isolated warm default-runtime targets for
all three query shapes. The original Q2 patient and its references are absent;
the positive Q2 run explicitly substitutes an existing patient with exactly
three `164.1 cm` height observations. No clinical data was added or changed.
First-execution latency remains a separate investigation in [#1625](https://github.com/HeliosSoftware/hfs/issues/1625),
and opt-in generic-plan performance retains the documented limits below.

## Closure verification after integrating main

The signed source commit `803270e4d` merges `main`'s broad Observation optimization
without changing the three selective shapes. Debug verification passed **197
PostgreSQL search unit tests, five issue-specific persistence tests and six HTTP
tests**; formatting and diff checks passed. The following HTTP measurements use
a separately built release HFS binary with R4/PostgreSQL and the default
`force_custom_plan`, an eight-connection pool and no competing database searches.

A stopped full-corpus database was copied with reflinks while its original volume
was mounted read-only. All work used the copy; the original stayed stopped. The
copy received main's normal schema 44→45 migration and patient-export GIN index.
Every captured Q1/Q2/Q3 plan excludes that new index. No `ANALYZE`, clinical
resource creation, or clinical resource updates were performed. PostgreSQL 16.15
used `shared_buffers=4GB` and `effective_cache_size=14GB`, matching the issue's
settings rather than the historical run's 8 GB/24 GB. The container had 14 CPUs and
18 GB RAM; the host had 16 CPUs. Runtime settings and source/binary fingerprints
are in the [artifact manifest](evidence/1579/restored-corpus-2026-09-30/manifest.json).

Exact counts are 11,705 live Patients, 7,699,987 live Observations and 18,957,921 live
resource rows. Subtracting the existing 1,372 SearchParameters and five
CompartmentDefinitions gives 18,956,544, the issue's resource total. Patient and
Observation counts match the issue directly; this does not assert byte identity
with its unavailable original snapshot.

### Q2's existing positive anchor

The original `01a0e8b8-323d-7341-9bfe-81346acf0c01` has zero live Patient rows and
zero indexed patient references. The replacement,
`01a0d68e-81eb-7a42-975a-9c417bb5c260`, was already present in the copy and has 106
live Observation candidates. Independently filtering those resource bodies by
subject, top-level LOINC `8302-2` and raw `valueQuantity.value > 160` yields exactly
three resources, each with 164.1 cm. The original issue does not specify Q2's total
Observation candidate count; 165 belongs to its Q3 anchor. The independent
[body ground truth](evidence/1579/restored-corpus-2026-09-30/body-ground-truth.json)
and [matching resource fields](evidence/1579/restored-corpus-2026-09-30/selected-q2-resource-values.json)
are preserved. Q2 changes only the patient UUID; its composite expression,
threshold, expected cardinality and latency target are unchanged.

### Clean isolated warm HTTP runs

After one recorded accurate-total warmup per shape, each query ran sequentially
twice with `_total=none` and twice with `_total=accurate`. All responses were 200,
all ordered IDs agreed across modes/runs, and no owned statement remained active
after a request. A 0.5 s host monitor captured 15 samples during this pass, with zero
cargo/rustc/clang/lld processes; maximum one-minute host load was 0.177 on 16 CPUs.

| Query | Total mode | Run 1 (s) | Run 2 (s) | Results / total |
|---|---|---:|---:|---|
| Q1 original reverse chain, count 5 | none | 0.005394 | 0.005361 | Five Patients / omitted |
| Q1 original reverse chain, count 5 | accurate | 1.599303 | 1.605634 | Five Patients / 11,705 |
| Q2 existing replacement + original composite | none | 0.215428 | 0.209686 | Three / omitted |
| Q2 existing replacement + original composite | accurate | 0.412942 | 0.413372 | Three / three |
| Q3 original patient + code + quantity in cm | none | 0.287050 | 0.280759 | Eight / omitted |
| Q3 original patient + code + quantity in cm | accurate | 0.559771 | 0.565397 | Eight / eight |

[Complete primary timings](evidence/1579/restored-corpus-2026-09-30/primary-timings.json)
include IDs, statement counts and timestamps. An earlier twelve-request warm
pass also met all targets and is retained separately; its monitor observed
cargo-only activity in three of 16 samples, with no actual compiler processes.
The clean pass was repeated to remove that host-activity uncertainty. Both passes
captured identical SQL and typed binding bytes. Full
[prepared SQL/bindings and EXPLAIN ANALYZE/BUFFERS plans](evidence/1579/restored-corpus-2026-09-30/plans/)
were replayed from the first pass.

The genuine pre-fence Q2 SQL from base `4427ae16c5dda7338e43032ac49d7c4e79b9875f`
was also replayed on the same copy, changing only tenant and patient bindings.
Unlike the small historical fixture, this corpus's old plan starts from 124,126
global composite matches and probes resource and patient index rows 124,126 times.
The new plan starts from 106 patient references and makes 106 scoped composite
probes. The old count's first execution took 44,347.138 ms with uncontrolled cache
state; a subsequent count took 5,165.257 ms, and its page took 4,206.713 ms. Both
returned the required three matches. Old count/page root buffers were
1,407,947/1,407,953, versus 1,239/1,245 in the new plans; new executor times were
238.467/253.318 ms. These are SQL execution measurements, separate from full HTTP
latency, and are not a controlled cold/warm speedup comparison. The old plans
still read shared buffers on subsequent executions. The complete
[baseline SQL/plans](evidence/1579/restored-corpus-2026-09-30/baseline-q2/)
preserve that distinction.

### First-execution and generic-mode boundaries

The recorded first observed accurate-total executions after startup were
Q2 **0.548858 s**, Q3 **2.645491 s**, and Q1 **37.402620 s**, in that order. They
returned the same correct IDs/totals as the later runs. Database/filesystem cache
state and the effect of the preceding schema migration/index build were not
controlled: **these are not controlled cold-cache measurements**, and restarting
HFS alone does not establish cold caches. They materially exceed the warm Q1/Q3
targets and are preserved in [warmup.json](evidence/1579/restored-corpus-2026-09-30/warmup.json).
[#1625](https://github.com/HeliosSoftware/hfs/issues/1625) tracks separate diagnosis
and an explicit first-execution target; no cause is claimed here.

The closure scope is isolated warm performance under the default custom-plan
runtime, with the explicit Q2 anchor substitution above. This is not a guarantee
for arbitrary cache states or opt-in `force_generic_plan`; the latter Q3 limit
remains as measured in the historical supplement below.

## Final implementation

Q1 uses a new query-specific
[`SearchProvider::supports_native_reverse_chains`](../crates/persistence/src/core/search.rs)
hook, defaulting to false. The shared resolver validates the terminal value using
its existing type and modifier handling before retaining an eligible native
query. Other providers and unsupported shapes use the existing resolver.

PostgreSQL opts in only for R4 with denormalized indexes and the single plain
`Patient?_has:Observation:patient:code=system|code` shape. The token must have one
nonempty system and code, equality semantics, and no modifier, escape, whitespace,
OR list, nested `_has`, or forward chain. Effective tenant registry definitions
must retain the core applicable Observation expressions: patient extracts
`Observation.subject.where(resolve() is Patient)` with exactly Patient and Group
targets, and code extracts `Observation.code`. Applicable expression or target
overrides fall back. Unsupported versions, legacy indexes, contained searches,
compartment/list constraints, includes, sorting, and additional filters also fall
back; plain equality `_id` augmentation is supported.

The same correlated predicate serves page and count. Reference IDs are read
inside `referenced_observations AS MATERIALIZED`; a resource-scoped code slice
then supplies a scalar source ID. The source resource lookup depends on that
code-qualified ID, rather than selecting all referenced Observation bodies
first. Tenant boundaries, live source and target resources, and the original raw
`subject.reference = 'Patient/' || resources.id` comparison are preserved.
Absolute and versioned references retain the shared resolver's existing behavior.
Both reference and code index rows must be non-contained: this deliberately fixes
the discovered contained-only false positive for this native shape. `EXISTS`
deduplicates Patients with multiple matching Observations or codings.

The native query works with accurate total, summary count, ID search, cursor
next/previous pages, offsets, and `_id` intersections added by export batching.
The separate fast index page route is disabled for native reverse chains.
[`CompositeStorage`](../crates/persistence/src/composite/storage.rs) opts in only
when page and count delegate to the same single native provider, including the
existing dedicated-provider/primary fallback. Routes merging multiple providers
retain shared resolution.

Q2 keeps PR #1597's per-resource `MATERIALIZED` composite slice and original
predicate/binds. Q3 adds a corresponding
[`scoped_quantity` slice](../crates/persistence/src/backends/postgres/search/query_builder.rs)
for denormalized Observation searches containing exactly one plain equality
patient reference, one equality code token, and one `gt` value-quantity with a
convertible unit. Modifiers, repeated/OR values, chains, composite components,
contained/compartment searches, reverse chains, and list constraints keep the
existing SQL. Legacy also keeps its existing SQL.

The Q3 slice is scoped by tenant, resource type, resource ID, and parameter name
before evaluating the complete original quantity predicate. Raw and canonical
UCUM OR branches, parameter order, and bindings are unchanged. Code and quantity
remain independent fields; the search is not converted into a composite. No
contained-row filter is added to Q3. A unitless comparison keeps its existing
raw semantics and is outside this fence.

## Historical corpus measurements before integrating main

Before and after used the same PostgreSQL 16.15, schema 44, R4 denormalized
snapshot, eight-connection pool, request settings, and SQL-capture proxy overhead.
The backend's existing default is `ForceCustomPlan` (`force_custom_plan`); all
primary HTTP measurements use that setting. Data, resource timestamps, indexes,
and statistics were unchanged. Startup used the existing idempotent table check;
no migration or `ANALYZE` was performed on this corpus.

Each of the three original requests ran sequentially twice with `_total=none`
and twice with `_total=accurate`. SQL and HTTP bounds were 120 and 150 seconds,
respectively, rather than the ticket's 600-second bound. Exact prepared
statements and bindings were captured and replayed with plain JSON EXPLAIN and
`EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`. The final twelve requests all returned
HTTP 200, with no pending owned SQL after any request. Values below are full HTTP
elapsed seconds, including count when requested.

| Query | Total mode | Before 1 | Before 2 | After 1 | After 2 | Final result |
|---|---|---:|---:|---:|---:|---|
| Q1 reverse chain, count 5 | none | >150 (408) | >150 (408) | 0.019271 | 0.007290 | Five ordered Patients; total omitted. |
| Q1 reverse chain, count 5 | accurate | >150 (408) | >150 (408) | 1.896207 | 1.646514 | Same five IDs; total 11,705. |
| Q2 patient + composite | none | 0.271738 | 0.243889 | 0.253276 | 0.249965 | Zero IDs; original anchor absent. |
| Q2 patient + composite | accurate | 0.431323 | 0.410915 | 0.416531 | 0.413935 | Zero IDs; total zero. |
| Q3 patient + code + quantity in cm | none | 55.845159 | 4.034171 | 0.290853 | 0.295141 | Identical eight ordered IDs. |
| Q3 patient + code + quantity in cm | accurate | 7.526081 | 7.516527 | 0.562299 | 0.561944 | Identical eight IDs; total eight. |

Before Q1 executed 122–126 Observation body-page searches per timed-out request
and never completed a Patient page or count. Its bounds are not completion times,
and no before/after Patient ID or total equivalence is claimed. Final Q1 uses one
Patient page statement, or count plus page for accurate total, without those
Observation page searches. All final modes/runs agree on five ordered Patient
IDs; exact prepared execution agrees with the independently measured
code-qualified candidate and supplies the sixth next-page sentinel. Custom and
generic prepared executions agree on all six ordered IDs and total 11,705.
Q3 preserves all eight original ordered IDs in both modes/runs, with exact count
parity; generic prepared execution also agrees.

A separate unitless `gt100` comparison returned the same eight IDs and total in
this snapshot: before 1.328739 s, after 1.346974 s. Its SQL is unchanged. That
coincidence does not establish general equivalence between raw thresholds and
UCUM conversion.

### Actual default custom plans

These are execution times and root shared hit-plus-read buffer counters from
captured prepared statements, separate from HTTP elapsed times.

| Statement | Execution ms | Root buffers | Observed work |
|---|---:|---:|---|
| Q1 accurate count | 1,954.359 | 995,108 | 11,705 outer Patients; code-qualified scalar source lookup. |
| Q1 accurate page | 7.421 | 2,126 | Six rows, including next-page sentinel. |
| Q3 accurate count | 341.343 | 7,030 | 165 patient candidates/scoped quantity probes. |
| Q3 accurate page | 340.339 | 7,036 | Eight rows; full original quantity predicate. |

Q1's source PK condition is `id = (SubPlan 3)`. Its count PK node reports
**122,683 invocations and 58,525 buffer hits**; the page PK node reports 309
invocations and 30 buffer hits. The code result determines the lookup key, and
the reference/code CTEs do not select bodies. Successful non-null source work is
inferred from this dependency and results; EXPLAIN's averaged/rounded Actual
Rows does not establish an exact body-read count. In particular, 11,705 is the
Patient total, not a measured count of source PK invocations or body reads.

An alternative that materialized referenced source bodies before checking code
was rejected after its count exceeded 30 seconds. Boolean and scalar code-fence
variants returned the same results and buffer totals with similar repeated warm
times; the scalar form was chosen for its explicit lookup dependency, without a
claimed timing advantage.

Before Q3, a global raw/canonical quantity BitmapOr drove 160,712 resource PK,
code, and patient probes. Root count/page buffers were 2,831,697/2,831,703. Final
custom-plan quantity slices execute 165 times and use 1,032 buffers. A global
code index-only scan still reads 175,355 entries once and uses 5,030 buffers;
that scan remains valid. Q2 retains its composite fence and uses five/eleven root
count/page buffers for the unavailable-anchor case, which returns zero matches.

Buffer counters are inclusive: root totals and individual node counters must
not be added together.

### Supplementary generic-plan limitation

Forced generic plans were replayed separately with the same captured bindings
and a 30-second bound. They are an opt-in supplement, not the before/after HTTP
configuration.

| Generic statement | Execution ms | Root buffers |
|---|---:|---:|
| Q1 count | 1,991.739 | 995,108 |
| Q1 page | 7.595 | 2,126 |
| Q3 count | 9,390.853 | 2,793,471 |
| Q3 page | 4,153.571 | 2,793,477 |

Generic Q3 remains correct and retains resource-scoped quantity evaluation with
both raw/canonical branches, but loses patient selectivity. It drives 175,355
code candidates, resource PK probes, and scoped quantity probes, then 160,712
patient-reference probes. The first generic count includes 26,785 root read
blocks; even its warm page takes 4.154 seconds. This is a material performance
risk in opt-in generic mode: there is no patient-first or candidate-work bound
for every plan mode. The approved default custom-plan runtime meets the measured
targets; generic Q3 does not meet them.

No cold-cache latency guarantee is established. These historical experiments warmed corpus
buffers, and an earlier cold-ish Q1 fenced count took 6.649 seconds with 28,832
read blocks. Q3's first before request also had a 55.845-second outlier. At the time of these measurements, Q2's
missing three-result acceptance case remained open. The restored-corpus section
above supplies positive evidence using the explicit existing-anchor substitution.

## Historical Q2-only synthetic evidence

The following fixture and measurements were recorded for the initial Q2-only
change on PR #1597. They show Q2's mechanism and three-result controls; they do
not establish final Q1/Q3 performance or the original corpus's three-result Q2
acceptance. The `ANALYZE` instructions apply only to this isolated fixture.

### Historical synthetic fixture

Use an isolated PostgreSQL 16 database and the R4/PostgreSQL HFS binary. This run
used PostgreSQL 16.15, schema 44, `shared_buffers=128MB`, `work_mem=4MB`, the
backend's `force_custom_plan` setting and an HFS pool of eight connections. Disable
automatic conformance seeding (`HFS_SEED_CONFORMANCE=false`) for this manual setup;
startup seeding stalled on advisory locks independently of this issue. Validation
and audit were disabled. Use the same database and resource timestamps for both
measurements and run `ANALYZE resources; ANALYZE search_index` before each series.

Generate resources through HTTP PUT or batch PUT, rather than importing the
production Synthea database:

| Resources | Values |
|---|---|
| Patient `anchor-composite`, 165 Observations `a-000`…`a-164` | First three: LOINC `8302-2`, `164.1 cm`. Rest: code `1234`, `50 cm`, except `a-003`: wrong system, `170 cm`; `a-004`: height, `1.8 m`. |
| Patient `anchor-quantity`, 165 Observations `b-000`…`b-164` | First seven: height, `110`…`116 cm`; eighth: height, `1.64 m`. Negatives: height `100 cm`, `1 m`, `200 kg`, wrong code `164.1 cm`; remaining rows: code `1234`, `50 cm`. |
| 12 Patients `distractor-00`…`distractor-11`, 2000 Observations `d-0000`…`d-1999` | Round-robin subjects; all LOINC height, `164.1 cm`. |
| Patient `deleted-only`, Observation `deleted-height` | Matching height, then DELETE the Observation. |
| Patient `wrong-code-only`, Observation `wrong-height` | Wrong top-level code. |
| Patient `contained-only`, Observation `contained-source` | Wrong top-level code; matching height only inside `contained`. |
| Second tenant with the same two anchor IDs and Observations `a-000`/`b-000` | Matching heights; must not affect primary tenant results. |

A resource generator can use this shape, setting each value from the table:

```python
def observation(id, patient, value=50, unit="cm", code="1234", system="http://loinc.org"):
    return {
        "resourceType": "Observation", "id": id, "status": "final",
        "code": {"coding": [{"system": system, "code": code}]},
        "subject": {"reference": "Patient/" + patient},
        "valueQuantity": {"value": value, "unit": unit,
                          "system": "http://unitsofmeasure.org", "code": unit},
    }
```

The primary tenant has 2350 resource rows (including the soft-deleted row) as loaded by this
recipe. The measured database also retained 1372 core SearchParameters and five
CompartmentDefinitions in the default tenant from initial conformance bootstrap.
That background affects PostgreSQL statistics, so plan choices on a completely
empty database may differ. The physical-plan regression below uses its own
controlled density; resource creation in the semantic and HTTP regressions uses the real write path.
The persistence semantic regression additionally clones one indexed composite
row into a distinct group and verifies two matching groups before checking
resource uniqueness; repeated identical top-level coding alone is deduplicated
by the writer.

### Historical Q2 requests and SQL capture

Run each request twice, first with `_total=none`, then `_total=accurate`. Supply
`X-Tenant-ID` for the synthetic tenant. Do not run concurrent requests. Check
`pg_stat_activity` for pending statements before each request.

```bash
curl --get --silent --show-error --fail-with-body \
  -H 'X-Tenant-ID: fixture-1579' "$HFS_URL/Observation" \
  --data-urlencode 'patient=anchor-composite' \
  --data-urlencode 'code-value-quantity=http://loinc.org|8302-2$gt160' \
  --data-urlencode '_total=accurate' \
  --output q2.json --write-out '%{time_total}\n'
```

Expected IDs are exactly `a-000`, `a-001`, `a-002`, and total is three when
requested. The unitless composite comparison stays a raw comparison: `1.8 m`
does not become a new positive through UCUM conversion.

Capture PostgreSQL execute logs and synthetic parameter values on the isolated
instance (`log_statement=all`, `log_parameter_max_length=-1`), or use the
single-connection regression's `pg_prepared_statements` technique. HFS debug
logging alone does not expose these statements. Replay the actual page and count
SQL with their bindings using `PREPARE`, then:

```sql
SET plan_cache_mode = force_custom_plan;
EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
EXECUTE captured_search('fixture-1579', 'Observation', 'anchor-composite',
                        'http://loinc.org', '8302-2', 160);
```

Compare rows, loops and the root's shared hit + read buffers for both statements.
Do not sum every node's buffer counters: parent counters already include children.
The regression additionally executes generic plans without using wall-clock
thresholds as assertions.

### Historical Q2 diagnosis and alternatives

The before-state already seeks the patient reference index and finds 165
candidates. It then rescans `idx_search_composite_token_quantity` 165 times. That
index orders `resource_id` behind an unconstrained `last_updated`, so the
resource equality does not make these repeated prefix scans cheap. The
composite scan alone touches 10561 buffers; a correct result of three does not
imply the execution is selective.

Each alternative was measured against the same captured SQL, bindings and
fixture. Numbers below are shared hit + read buffers for count / page:

| Strategy | Count | Page | Decision |
|---|---:|---:|---|
| Original memberships | 11090 | 11096 | Repeated broad composite scans. |
| Materialize patient candidates only | 10769 | 10775 | Still rescans the global composite slice. |
| Direct correlated EXISTS | 11090 | 11096 | Resource equality remains a late composite-index key. |
| Resource rows behind `OFFSET 0`, then residual predicate | 1049 | 1055 | Demonstrates the useful boundary. |
| Resource rows behind `WITH … AS MATERIALIZED`, then residual predicate | 1049 | 1055 | Chosen; explicit, documented optimization fence. |

The chosen SQL uses a correlated materialized CTE scoped by tenant, type,
resource ID and parameter name. PostgreSQL seeks `idx_search_resource`, retrieving
one composite row per resource in this fixture, instead of rescanning thousands
of global matches. The unchanged original predicate is evaluated on those rows.
[PostgreSQL 16 documents MATERIALIZED as preventing CTE folding](https://www.postgresql.org/docs/16/queries-with.html).
Materializing the patient candidates alone was insufficient; the fence belongs
around the resource's index rows.

The guard requires exactly these two parameters on Observation, a single plain
patient reference, and a single composite value with token/quantity components,
on the denormalized layout. OR lists, repeated parameters, reference modifiers,
compartment constraints, other composites and global searches retain the
existing SQL. Legacy retains its group aggregate. This Q2 guard adds no public
search behavior or schema migration.

### Historical Q2 HTTP and physical results

The initial Q2-only binary was rebuilt with the same R4/PostgreSQL features and
restarted against that fixture without reloading it. These local HTTP elapsed
times are milliseconds:

| Total mode | Before run 1 | Before run 2 | After run 1 | After run 2 | Results / total |
|---|---:|---:|---:|---:|---|
| none | 47.967 | 46.002 | 5.125 | 5.141 | 3 / absent |
| accurate | 90.408 | 91.051 | 8.356 | 7.978 | 3 / 3 |

Q2 preserved exactly `a-000`, `a-001`, `a-002`. Page/count buffers fell from
11,096/11,090 to 1,055/1,049. The composite index's 165 broad probes and 10,561
buffers became 165 resource-index probes and 520 buffers, with one composite row
per candidate before the residual predicate. Generated next/previous links at
count one returned all three unique resources in both total modes; previous
returned the first page exactly.

The separate controlled Q2 physical regression includes seven other parameter
names produced by the Observation writer. Its representative nine-row density
reproduced broad composite rescans, whereas a two-row skeleton already selected
an efficient original plan. That fixture measured approximately 2,294–2,295
buffers after, versus 11,945–11,946 custom-plan and 28,234–28,239 generic-plan
buffers before. These counters differ from the historical HTTP fixture and from
the final corpus plans above.

The historical fixture also exposed a contained-only Q1 false positive. The
final native Q1 guard corrects that case; the historical resolver timings and
Q3 SQL are superseded by the final implementation and corpus evidence.

## Historical automated validation before integrating main

Final suites and checks passed; the initial combined run is annotated below.
Persistence and REST commands used `--release`, four build jobs, and a shared
external target directory; integration tests used isolated databases. Counts are
tests passed, not corpus measurements.

| Command | Passed |
|---|---:|
| `cargo test --release -p helios-persistence --features postgres --lib backends::postgres::search` | 195 |
| `cargo test --release -p helios-persistence --features postgres --lib --test postgres_tests 1579 -- --test-threads=1` | 8 unit tests; integration rechecked below |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests 1579 -- --test-threads=1` | 5 |
| `cargo test --release -p helios-persistence --features postgres --lib search::chain_resolver` | 27 |
| `cargo test --release -p helios-persistence --features postgres --lib composite::storage` | 76 |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests postgres_integration_quantity_ -- --test-threads=1` | 3 |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests postgres_integration_composite_ -- --test-threads=1` | 2 |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_composite_code_value_quantity -- --test-threads=1` | 1 |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests postgres_integration_resolve_reverse_chain -- --test-threads=1` | 3 |
| `cargo test --release -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_refuses_unresolved_chains -- --test-threads=1` | 1 |
| `cargo test --release -p helios-rest --features postgres --test postgres_selective_search -- --test-threads=1` | 1 |
| `cargo test --release -p helios-rest --features postgres --test chained_search_resolution -- --test-threads=1` | 5 |

The combined `1579` run passed its eight unit tests but initially failed one
overly strict small-fixture PK-plan assertion. PostgreSQL validly used a Hash
Cond depending on the matching-code subplan. The assertion was corrected to
accept optimizer forms that preserve that dependency, and the subsequent
five-test integration run passed. The table records that distinction rather
than claiming the original combined command wholly passed.

Compatibility and formatting checks also passed:

```bash
cargo check --release -p helios-persistence --no-default-features --features R4B,postgres
cargo fmt --all -- --check
git diff --check
```

Builder tests pin Q1 eligibility, effective registry expression/target overrides,
Q1 parameter offsets, Q2's original fence, all six Q3 parameter orders, cursor
bind offsets, full original predicates/binds, and fallback forms. Persistence
regressions cover tenant isolation, live/deleted source and target resources,
contained-only negatives, relative/absolute/versioned references, duplicate
matches, accurate total, ID search, `_id` export augmentation,
next/previous/offset pages, raw/canonical units, wrong code/unit/patient controls,
and legacy fallback. Composite and resolver tests cover single versus merged
routes, provider fallback, terminal validation, and unsupported shapes.

Physical regressions examine the actual prepared page/count statements under
custom and generic modes. Q1 checks code-qualified source lookup dependency;
Q3 checks candidate-bounded quantity work on its controlled fixture, not a
patient-first guarantee on every corpus plan. The HTTP tests verify native
Q1 without Observation body-page enumeration, cover its summary count, and preserve conditional,
system-only chain, export filter, SQLite, and existing Q2 pagination behavior.
The R4B-only check verifies feature compatibility; it is not an R4B native Q1
runtime test. Full-workspace/all-feature preflight was not invoked.
