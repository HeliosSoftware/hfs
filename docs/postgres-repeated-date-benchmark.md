# PostgreSQL repeated date search benchmark for issue #1416

A repeated search parameter (`?birthdate=ge1980-01-01&birthdate=lt1990-01-01`)
is an AND of two candidate sets. On PostgreSQL 16, building the two occurrences
as two `id IN (SELECT ...)` sublinks let the planner run the pair as a nested
loop that re-executed one arm's index scan once per candidate of the other: on
a 24,000-Patient test corpus the page took 33.4 s and touched 7.4M buffers, and
on the manual-QA corpus from the issue it was reported at roughly 20 s per
statement against a shipped 30 s `statement_timeout`. Since #1416 the two
occurrences build as one membership test over their `INTERSECT`, and the same
page takes about 20 ms and 15,000 buffers. This document records the mechanism,
the measurement, and what the measurement does and does not establish.

The durable CI-side check lives in `.github/scripts/pg-search-plans.sql`,
sections BV to BZ. It captures plan shape, not wall time: the gate is that every
`INTERSECT` arm is an index scan with `Actual Loops = 1` and that
`idx_search_date_recent` (the before shape's rescanning node) appears nowhere.

## What the query is

`Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01` reaches
`PostgresQueryBuilder::build_search_query` as two `SearchParameter`s with the
same name, ANDed one level up in `build_search_query_for`. Each occurrence has
always been built as an `id IN (...)` test over `search_index`, so before #1416
the fragment was two such tests:

```sql
((id IN (SELECT resource_id FROM search_index WHERE ... param_name = 'birthdate' AND value_date >= $3))
 AND
 (id IN (SELECT resource_id FROM search_index WHERE ... param_name = 'birthdate' AND value_date <  $4)))
```

Those are two semi-joins over the same base table, and that is what the planner
turned into a nested loop whose inner side was one arm's date index scan,
re-executed per outer candidate. After #1416 the fragment is a single
membership test over the occurrences' own selects:

```sql
id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = 'birthdate' AND value_date >= $3 INTERSECT SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = 'birthdate' AND value_date < $4)
```

The arms are the per-occurrence selects concatenated verbatim, so every scoping
and every modifier the single-occurrence builders applied survives unchanged.
Both occurrences now sit inside one set-operation input, so one occurrence's
select can no longer be parameterized beneath the other and re-run once per
candidate it produces; in every captured plan the two arms execute once.

The same fragment is used by the page statement (`search_impl.rs`, default
sort, `_count + 1` rows), the count statement (`COUNT(*)`), `search_count`, and
the `_total=accurate` / `_total=estimate` paths. Cursor and `_offset` paging
build the same predicate, and each arm binds `tenant_id`, so the fold does not
weaken tenant isolation.

A parameter that occurs once is untouched: no set operation is built and the
fragment is byte for byte what it was, so the fast path's single-index
predicate extraction still applies to the common query. For a repeat, the
group's occurrences are emitted together in occurrence order and the
placeholders are renumbered to follow the text, so the bind list stays in the
order the placeholders first appear at any starting offset; the builder tests
pin that at both layouts, `2` without a cursor and `4` with one, where the
leading binds are the caller's tenant, type, and keyset key.

## Why the answer is an intersection of resources

The two occurrences cannot be merged into one range predicate or evaluated
against a single `search_index` row. A date parameter can be multi-valued: its
expression can reach a repeating element, or several elements, so one resource
can have several `date` rows, and one row can satisfy `date=ge2019-01-01` while
a different row satisfies `date=le2021-12-31`. Expressing the repeat as "the
resource has a date row in the overlap of the two windows" would drop that
resource; intersected resource-id sets keep it. The identity is the plain
relational one: a resource is in the answer exactly when it is in both arms'
resource-id sets. A resource whose only date row satisfies neither arm, or just
one, is excluded either way.

Since #1391 an `Encounter.period` is no longer such a case: it indexes as one
row holding the range `[value_date, value_date_end)`, and each arm is the FHIR
range-target rule on that row (`ge` is `te > e ∨ eq`, `le` is `ts < s ∨ eq`).
Schema v43 adds `value_date_end` to the `INCLUDE` list of `idx_search_date`
and `idx_search_date_recent`, so the arms stay index-only.

`crates/persistence/tests/postgres_tests.rs` pins the end-to-end semantics: an
Encounter from 2019-01-01 to 2021-06-01 is returned for
`date=ge2019-01-01&date=le2021-12-31`, while Encounters that fail one arm are
not, together with `search_count`, both `_total` modes, cursor and offset
paging, and a look-alike in another tenant.

## Method

The measured SQL is the patched builder's own output, not a re-typed copy. A
scratch Cargo package (outside the repository) called
`PostgresQueryBuilder::build_search_query(&query, 2)` for the query above and
wrote the fragment, then page and count statements wrapped exactly as
`search_impl.rs` wraps them. A verification step re-read the marked statements
from the plan script and reported byte equality: 465 bytes for the page
statement and 382 bytes for the count. The patched crate's builder tests pass
in the same build (`cargo test -p helios-persistence --features postgres --lib
-- query_builder::tests`, 133 passed).

Both states ran in `postgres:16-alpine` (image id `sha256:cf78e76683b9...`)
with `shared_buffers=512MB`, `work_mem=10MB`, `max_wal_size=2GB`,
`track_io_timing=on`, `plan_cache_mode=force_custom_plan`,
`random_page_cost=4`, `default_statistics_target=100`, and `jit=on`. Evidence
sessions raised `statement_timeout` to 900 s so the before state could finish;
HFS itself ships 30 s. Both tables were vacuumed with
`VACUUM (ANALYZE, PARALLEL 0)`. Each `EXPLAIN` used `(ANALYZE, BUFFERS)`, and
the metric quoted throughout is total buffers touched (hit + read), never
`read=` alone and never wall time.

The corpus is a deterministic arithmetic generator over a schema subset:
`resources` with `idx_resources_search`, and `search_index` with the three live
indexes used by this shape (`idx_search_date`, `idx_search_date_recent`,
`idx_search_token_code`) plus the v16 date statistics. Two sizes:

| | scale 1 | scale 2 |
|---|---:|---:|
| Patients | 12,000 | 24,000 |
| `resources` rows | 1,812,000 | 3,624,000 |
| `search_index` rows | 2,124,000 | 4,248,000 |
| `birthdate` rows | 12,000 | 24,000 |
| arm `>= 1980-01-01` / arm `< 1990-01-01` / intersection | 4,500 / 9,000 / 1,500 | 9,000 / 18,000 / 3,000 |
| planner estimate, arm A / arm B | 19 / 1 | 38 / 1 |

The before numbers are the N0 before-state capture prepared for this
implementation, measured on this synthetic corpus with the issue's original
query shape; the after numbers are a second capture on the same corpus, same
image id, same GUCs. The issue's original manual-QA corpus was a different data
set and was not rerun. A same-container cross-check re-ran the before-state
statements on the after-state container so the delta cannot be attributed to
the host.

## Results

| Capture | before ms | after ms | before buffers | after buffers |
|---|---:|---:|---:|---:|
| scale 1 page | 8,144.1 | 11.805 | 1,896,316 | 7,683 |
| scale 1 count | 8,188.5 | 8.077 | 1,896,313 | 7,680 |
| scale 2 page | 33,372.4 | 21.296 | 7,424,506 | 15,360 |
| scale 2 count | 33,832.0 | 18.584 | 7,424,503 | 15,357 |
| scale 2 page, repeat | 32,227.9 | 18.919 | 7,424,506 | 15,360 |
| scale 2 count, repeat | 32,307.8 | 18.229 | 7,424,503 | 15,357 |

Execution time improves by 690x at scale 1 and 1,567x at scale 2 for the page,
and by 1,014x and 1,821x for the count. Buffers improve by 247x and 483x. The
returned pages are byte-identical before and after at both sizes, and the
counts match the ground-truth intersection.

The plan shape is the stronger evidence. Before, the page plan contained
`Index Only Scan using idx_search_date_recent` with `loops = 9000` at scale 1
and `loops = 18000` at scale 2, at 205.7 and 407.4 pages per rescan, plus an
`Index Scan using resources_pkey` probe that many times, returning 0 rows each
time. After, both arms are `Index Only Scan using idx_search_date` with
`loops = 1`, joined by a hashed `SetOp Intersect` with `loops = 1`, and no
capture spilled (`Temp Read/Written Blocks = 0` in every plan, at both 10 MB
and 64 kB `work_mem`). The only remaining node with more than one loop is
`Index Scan using resources_pkey` (1,500 and 3,000 loops): that is the outer
fetch of the returned candidates, and it tracks the intersection size, not an
arm's candidate count.

## Scaling direction

| scale 2 over scale 1 | before | after |
|---|---:|---:|
| page execution time | 4.098x | 1.804x |
| count execution time | 4.132x | 2.301x |
| page buffers | 3.915x | 1.999x |
| count buffers | 3.915x | 2.000x |

Doubling the corpus doubles the after-state buffers (2.00x) and doubles the
only per-candidate work left. Before, buffers grew by 3.92x for a 2x corpus:
the excess is the rescan term, whose loop count follows the other arm's
candidates (9,000 to 18,000) rather than the answer set (1,500 to 3,000).

Two data points plus the plan and buffer evidence are consistent with the
removal of the quadratic term. They are not a formal asymptotic proof, and the
18.9M-resource manual-QA corpus from the issue was not rerun for this document.
That corpus remains the manual confirmation that the old shape could not finish
a page inside the shipped 30 s timeout; the deterministic corpus reproduces the
mechanism and the fix removes the node rather than making it cheaper.

## Controls

Low `work_mem`. Setting `work_mem = '64kB'` leaves the shape unchanged: arms at
`loops = 1`, no spill, and 7,680 buffers at scale 1 (page 9.425 ms, count
8.004 ms) and 15,357 buffers at scale 2 (page 21.254 ms, count 20.942 ms;
repeat 16.427 ms and 16.110 ms). The fix does not depend on a memory budget for
this shape.

Same container, same data. Re-running the before-state statements on the
after-state container against the same 24k corpus gives 32,763.6 ms /
7,424,509 buffers (page) and 32,483.6 ms / 7,424,503 buffers (count), with
`idx_search_date_recent` at 18,000 loops, against 21.296 ms / 15,360 and
18.584 ms / 15,357 for the builder's statements: 1,538x and 1,748x on identical
data. The before-state page ids from that run are byte-identical to the
baseline capture's.

## What a reviewer should take from this

The fold is a performance change with no intended behavior change: the same
rows are returned, the count matches the page, `search_count` and both `_total`
modes agree, paging is unaffected, and tenant scoping is unchanged. The reason
it is an intersection and not a range merge is that date parameters can be
multi-valued on one resource. The gain measured here is scoped to the shape
#1416 reported: arms the planner underestimates, which it then re-runs as a
nested-loop rescan. Limitation 6 covers the opposite case, a repeat whose arms
are both wide.

The fold is generic, not date-specific. It applies to any parameter name that
appears two or more times whose occurrences all build as a simple positive
`search_index` membership test on that name, whatever the parameter type. A
denormalized composite folds as well, because it builds as one row predicate;
the legacy composite layout does not, because it is an aggregate. The date case
is the one benchmarked here. Full details are in
`docs/search-conformance-plan.md`, item A7b.

## Reproducing

The plan-shape check needs a database with the live schema and enough date rows
to matter:

```bash
# Any PostgreSQL 16 with the live schema loaded. Same GUCs as the captures:
docker run -d --name hfs-1416-pg \
  -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=hfs1416 \
  -c shared_buffers=512MB -c work_mem=10MB -c max_wal_size=2GB \
  -c track_io_timing=on -c plan_cache_mode=force_custom_plan postgres:16-alpine

# The whole file, or only the BV to BZ section (split it out at its
# `-- BV-BZ.` comment). The earlier sections assume their own preconditions.
docker exec -i hfs-1416-pg psql -U postgres -d hfs1416 \
  -f - < .github/scripts/pg-search-plans.sql
```

The corpus generator used for the numbers above lived outside the repository
and is not committed, so re-running the file against a different corpus will
reproduce the shapes and not the exact timings. Read BV/BW for the fix and
BX/BY for the control in the same pass; BZ prints the criteria's own arm sizes,
intersection, count, and page ids so the plans can be checked against the
answers.

Behavior is pinned by tests:

```bash
# Builder-level shapes and placeholder order.
cargo test -p helios-persistence --features postgres --lib -- query_builder::tests

# End-to-end semantics, incl. Encounter.period, totals, paging, tenant
# isolation. Uses testcontainers, so Docker must be available.
cargo test -p helios-persistence --features postgres -- \
  postgres_integration_comma_list_is_or_and_repeated_param_is_and
```

## Limitations

1. The schema is a subset: `resources` and `search_index` with the indexes this
   shape can use. `resource_history`, `resource_fts`, the full-text and the
   composite indexes are absent, and no plan for this shape reaches them.
2. The corpus is generated arithmetic, so its selectivity is not the manual-QA
   corpus's. The mechanism is reproduced exactly (one parameterized probe per
   candidate of the other arm), and the fix removes that node rather than
   speeding it up.
3. Single-shot captures on an unloaded host: at these millisecond sizes a wall
   time is noisy (11.8 ms and 9.4 ms for the same statement). The buffer
   counts, the arm loop counts, and the absence of the rescanning node are the
   durable signals.
4. The evidence sessions ran with `statement_timeout = 900s`. That deadline is
   for evidence only; HFS's shipped default is 30 s, which the before state
   exceeded at scale 2 and the after state does not approach.
5. Planner estimates for the first arm differed from the baseline capture by
   one row (19 and 38 versus 20 and 39) because `ANALYZE` sampled differently.
   Nothing in the after-state plan depends on that estimate.
6. A repeat whose arms are both wide is a different case from the one measured
   here. When both occurrences match a very large share of the type there is no
   re-run scan to remove, and the set operation has to consume and deduplicate
   both arms. PostgreSQL 16's `SetOp` is not parallel-aware, so the fold runs
   that step in one process, while the conjunction it replaced could plan as
   parallel hash semi-joins. The fold is not expected to win there: read BV/BW's
   temp blocks, buffers, and arm loop counts against BX/BY in the same pass
   before calling a wide repeat fast or slow.
