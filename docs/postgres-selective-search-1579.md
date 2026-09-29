# Patient and composite PostgreSQL search (#1579)

This change addresses only query 2 from #1579: an Observation search with one
plain `patient` reference and one `code-value-quantity` token/quantity composite.
It keeps the original composite predicate and binds, while reading the composite
rows for each candidate resource before applying that predicate. Page, count and
ID searches use the same builder.

The other issue requests remain separate work. Query 1 still enumerates
Observation bodies through the reverse-chain resolver. Its contained-only
negative control also exposed an existing false positive. Query 3 did not
reproduce excessive composite-index work in this fixture; its raw/canonical UCUM
quantity branches remain intact.

## Reproducible fixture

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

## Isolated requests and SQL capture

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

## Diagnosis and alternatives

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
existing SQL. Legacy retains its group aggregate. This guard adds no public
search behavior or schema migration.

## Verified HTTP before and after

The HFS binary was rebuilt with the same R4/PostgreSQL features and restarted
against the same fixture, database, resource timestamps and runtime
configuration. No fixture reload occurred. Each request ran in isolation twice;
PostgreSQL activity was checked before every request. These are local HTTP
elapsed times in milliseconds, including count when requested:

| Query | Total mode | Before run 1 | Before run 2 | After run 1 | After run 2 | Results / total |
|---|---|---:|---:|---:|---:|---|
| 1: `_has`, count 5 | none | 115.572 | 109.532 | 125.133 | 112.446 | 5 / absent |
| 1: `_has`, count 5 | accurate | 107.882 | 110.693 | 107.523 | 106.838 | 5 / 15 |
| 2: patient + composite | none | 47.967 | 46.002 | 5.125 | 5.141 | 3 / absent |
| 2: patient + composite | accurate | 90.408 | 91.051 | 8.356 | 7.978 | 3 / 3 |
| 3: patient + code + quantity | none | 7.266 | 7.018 | 7.454 | 6.869 | 8 / absent |
| 3: patient + code + quantity | accurate | 12.356 | 11.965 | 12.030 | 11.702 | 8 / 8 |

The after-state's actual page/count statements and bindings were captured again,
and all first-run statements were replayed with
`EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`. Query 2 preserves exactly
`a-000`, `a-001`, `a-002` and accurate total three. Page buffers fall from 11096
to 1055; count buffers from 11090 to 1049. The composite index's 165 broad probes
and 10561 buffers are replaced by 165 resource-index probes and 520 buffers,
retrieving one composite row per candidate before evaluating the residual.
Generated next/previous links at count one return all three unique resources in
both total modes; previous returns the first page exactly.

Query 3 retains identical SQL/bindings, eight IDs, and 1089/1083 page/count
buffers. Query 1 still performs three Observation page searches before returning
Patients, including when total is omitted. Its existing contained-only false
positive remains: accurate total 15 versus the semantic control's expected 14.
Only query 2's demonstrated mechanism is repaired here; variations in the other
requests' elapsed times do not represent additional optimizations.

## Automated validation

```bash
cargo test -p helios-persistence --features postgres --lib backends::postgres::search::query_builder::tests
cargo test -p helios-persistence --features postgres --test postgres_tests 1579 -- --nocapture
cargo test -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_composite
cargo test -p helios-persistence --features postgres --test postgres_tests postgres_integration_composite_
cargo test -p helios-persistence --features postgres --test postgres_tests postgres_integration_quantity_
cargo test -p helios-rest --features postgres --test postgres_selective_search
cargo fmt --all -- --check
git diff --check
```

Builder tests pin the fence, original predicates/binds, reversed parameter order,
cursor bind offsets, and fallback forms. Persistence regressions cover three
unique resources despite duplicate composite groups, explicit units, system,
tenant, soft delete, accurate total/search_count, ID search, OR/repeats and
next/previous/offset pages. The physical regression examines the statements the
backend actually prepared, and checks bounded per-resource index work. Its
controlled fixture includes the seven other parameter names the Observation
writer produces: a two-row-per-resource skeleton already picked an efficient
original plan, while the representative nine-row density reproduced the broad
composite rescans. The final remediation run uses 2294 buffers for count and
page rather than 11945 under custom plans, and 2294 rather than 28239 under
generic plans. Neighboring runs differ by a few buffer hits (2294–2295 after,
11945–11946 custom before, 28234–28239 generic before). These counters describe
that synthetic physical fixture, separately from the actual HTTP fixture's
1049/1055 count/page buffers above. The HTTP regression sends the literal issue
parameter through REST and follows generated pagination links.

These small-fixture findings do not establish timings for the original 244 GB
corpus or meet its production latency thresholds. Those measurements remain
unverified.
