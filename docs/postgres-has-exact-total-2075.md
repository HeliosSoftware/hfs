# PostgreSQL Patient `_has` exact totals (#2075)

The eligible native search
`Patient?_has:Observation:patient:code=http://loinc.org|8302-2&_count=5&_total=accurate`
now prefilters code-negative sources during its broad count. The measured corpus
returns the same five ordered Patients and exact total **11,705**, inside the
existing 30-second PostgreSQL timeout. Page SQL and `_id`-restricted counts retain
their existing strategy. [Primary measurements](evidence/2075/summary.json)
record the settings, timings, plan counters and prepared-SQL fingerprints.

## Scope and query strategy

The native gate remains R4, denormalized indexes, effective core tenant registry
definitions, and one plain `_has:Observation:patient:code=system|code` terminal.
Only `search_count()` with no ordinary parameters uses the new prefilter.
Unsupported shapes, layouts, versions and registry overrides retain their paths.
There is no schema migration, index, timeout or configuration change.

The [predicate builder](../crates/persistence/src/backends/postgres/search/native_has.rs)
shares the exact source guard between page, scoped count and broad count.
Tenant boundaries, live source/target resources, non-contained patient/code rows,
same-Observation membership, requested/implicit systems and the raw relative
`subject.reference` comparison remain mandatory. Duplicate rows do not duplicate
Patients; absolute/versioned references retain their existing exclusion.

For a broad count, an uncorrelated scalar subquery reads token-index candidates
once and builds a JSONB membership map keyed by `hashtext(resource_id)::text`.
The source is checked against that map before opening its exact code slice.
Contained/stale rows and hash collisions may admit extra candidates; the common
exact guard still decides membership. The map never establishes a match itself.

A usable map is allowed for at most **262,144 input rows**, including duplicates.
The scan and aggregate consume up to **262,145 rows** (`LIMIT 262145`) to detect
overflow. Hash keys have at most 11 characters, bounding aggregate growth
independently of resource-ID length. An empty input yields `{}` and skips all
exact probes. Overflow yields NULL, which disables only the prefilter and retains
the original exact evaluation. This preserves correctness for arbitrarily dense
tokens, without promising an improvement above the cap.

A direct `IN` prefilter was rejected: at constrained `work_mem` the planner could
replace hashed membership with repeated candidate rescans. The scalar map stays
one InitPlan under the exercised custom/generic and low-memory plans. The corpus
map serializes to **3,153,504 bytes**; peak aggregate memory was not measured.

## Corpus and measurement method

The existing corpus contains 11,705 live Patients and 7,699,987 live Observations,
using PostgreSQL 16.15, schema 49, R4 denormalized indexes and tenant `default`.
The normal settings remained:

| Setting | Value |
| --- | --- |
| `statement_timeout` | 30 seconds |
| `plan_cache_mode` | `force_custom_plan` |
| `shared_buffers` | 128 MiB |
| `effective_cache_size` | 4 GiB |
| `work_mem` | 4 MiB |

The first planning count was canceled after **30.001 seconds** with uncontrolled
cache state. Subsequent planning probes warmed database/filesystem caches.
No restart, cache reset, corpus write, statistics update or index build was used.
There is no controlled cold-cache measurement or cold-cache guarantee here.

The before count came from the actual Rust prepared statement before the change.
The implemented backend count capture and implemented HTTP count capture match
byte for byte; the before and implemented count statements differ. The page
statement is unchanged. Three before/after rounds replayed these statements
sequentially in read-only transactions, retaining the original bindings and
30-second timeout. No owned builds ran during QA, no other client query was
active at its start, and no owned statement remained active at the end.

These corpus measurements are **SQL replay, not full-corpus HTTP latency**.
Separate HTTP integration tests verify response construction on disposable data.

## Observed results

Times below are client-observed SQL seconds. Count-plus-page includes both
statements in sequence, without HTTP overhead.

| Round | Before count | After count | Before count + page | After count + page |
| --- | ---: | ---: | ---: | ---: |
| 1 | 3.242 | 1.530 | 3.305 | 1.591 |
| 2 | 2.787 | 1.322 | 2.841 | 1.376 |
| 3 | 2.348 | 1.415 | 2.402 | 1.478 |

All rounds and total modes preserved the ordered first five IDs; accurate count
was always 11,705. Page replay also returned the sixth pagination sentinel.
After-change `_total=none` page statements took 0.053–0.054 seconds. The summary
records an ID digest without publishing resource identifiers. The initial timeout
and these repeated executions are different cache conditions, not a speedup ratio.

The measured count plans reduced exact code probes from **151,430 to 11,709**.
The latter include four collision-induced extra probes, checked by the exact
guard. All after plans scan the 175,355 token candidates once. Root shared
hit-plus-read buffers fell from 1,250,855 to 344,758; these counters are inclusive.

| After count plan | Executor seconds | Candidate scan loops |
| --- | ---: | ---: |
| Custom, 4 MiB `work_mem` | 1.254 | 1 |
| Custom, 64 KiB `work_mem` | 1.199 | 1 |
| Generic, 4 MiB `work_mem` | 1.298 | 1 |

The 64 KiB plan wrote ten temporary blocks; the other listed plans wrote none.
These observations establish the measured cases, not universal plan or latency
guarantees for different data distributions, cache states or candidate densities.

## Verification and reproduction

Sixteen focused cases passed: two new unit tests, four existing native-gate/unit
tests, two new persistence tests, one existing membership/pagination test, one
existing scoped-probe test and six HTTP tests. The physical regression executes
32 actual prepared-plan cases: sparse/dense/absent tokens, distinct/duplicate
overflow, custom/generic plans and 4 MiB/64 KiB memory. It observes one InitPlan,
bounded input, 1/0 exact probes below the cap and correct 33/0 fallback probes on
its overflow fixture. A real hash collision must not add a matching Patient.

The semantic cases cover tenants, repeated automatic-plan bindings, implicit and
wrong systems, duplicate entries, deleted resources, contained-only codes,
split-source code/reference matches, index/body disagreement and pagination.
HTTP returns five of seven matches, with total seven for accurate/estimate,
omitted total for none, and no entries for summary count. R4B-only compilation,
formatting and diff checks passed. Full workspace/CI preflight was not run.

From the repository root, Docker-backed tests capture the SQL actually prepared:

```bash
export HFS_DATA_DIR="$PWD/data"
export CARGO_BUILD_JOBS=4
cargo test --locked --release -p helios-persistence --features postgres --lib native_1579_
cargo test --locked --release -p helios-persistence --features postgres --lib native_2075_
cargo test --locked --release -p helios-persistence --features postgres --test postgres_tests postgres_2075_ -- --test-threads=1 --nocapture
cargo test --locked --release -p helios-persistence --features postgres --test postgres_tests postgres_1579_native_has_membership_count_ids_and_pagination -- --test-threads=1
cargo test --locked --release -p helios-persistence --features postgres --test postgres_tests postgres_1625_code_negative_sources_skip_body_guard_and_bound_probes -- --test-threads=1
cargo test --locked --release -p helios-rest --features postgres --test chained_search_resolution -- --test-threads=1 --nocapture
cargo check --locked -p helios-persistence --no-default-features --features R4B,postgres
cargo fmt --all -- --check
git diff --check
```

The [HTTP test](../crates/rest/tests/chained_search_resolution.rs) emits actual
count/page SQL and four `text` parameter types; page SQL has `LIMIT 6`. The
[persistence test](../crates/persistence/tests/postgres_tests.rs) emits the count
and analyzed plans. Use those captured statements verbatim, rather than rebuilding
SQL from the predicate description. Prepare them as `count_stmt` and `page_stmt`
with four text parameters. Summary SQL digests hash UTF-8 text after trimming
surrounding whitespace and the terminal semicolon. Replay on the authorized
corpus connection:

```sql
BEGIN READ ONLY;
SET LOCAL statement_timeout = '30s';
SET LOCAL plan_cache_mode = 'force_custom_plan';
EXECUTE count_stmt('default', 'Patient', 'http://loinc.org', '8302-2');
EXECUTE page_stmt('default', 'Patient', 'http://loinc.org', '8302-2');
EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
  EXECUTE count_stmt('default', 'Patient', 'http://loinc.org', '8302-2');
ROLLBACK;
```

Run page alone for none, count then page for accurate, and count alone for summary.
Compare the first five IDs, exact count and statement fingerprints; interleave
three rounds and record cache/load conditions. Record generic/64 KiB plan probes
separately, using transaction-local settings without changing normal runtime.
