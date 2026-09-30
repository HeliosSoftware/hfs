# PostgreSQL broad Observation search benchmark

This report accompanies [issue #1580](https://github.com/HeliosSoftware/hfs/issues/1580).
The candidate meets the approved **warm HTTP p95 below 3 seconds** target in all 21 groups on this corpus; the baseline meets it in 16.
Exact totals, ordered IDs and paging are preserved. Supplemental single-pass browser timings are reported below; original manual before/after QA was not run and that limitation was accepted.

## Corpus and environment

Baseline commit: `4427ae16c5dda7338e43032ac49d7c4e79b9875f`; both binaries were frozen release builds of HFS 0.2.3 with `R4,postgres,ui`.
The corpus contains **7,699,987 Observation** resources, approximately 18.9 million resources and 269 million search-index rows, using the Denormalized layout.
Both benchmarks used schema 44. Existing schema 42→44 preparation included a roughly 92-minute backfill and index work before either benchmark; that preparation is separate from this fix.
The fix adds no migration, index, persistent GUC, product or UX change.

PostgreSQL 16 ran with a 32 GiB container memory limit and no CPU quota on a host with 8 physical/16 logical CPUs and about 62 GiB RAM.
This differs from the issue's 18 GB/14 CPU environment and cannot establish its performance there.
Effective settings were `shared_buffers=8GiB`, `effective_cache_size=24GiB`, and `work_mem=16MB` for both stages.
The first plan inventory recorded 4/12 GiB; pre-existing `postgresql.auto.conf` settings explain the effective 8/24 GiB values. No tuning was introduced by the fix.
The application pool retained its default `force_custom_plan`; a standalone SQL session's role default of `auto` does not describe that pool.
Both stages used 24 connections, 600-second statement/request limits and a 60-second pool-wait limit.
Bulk-local background workers were disabled in both stages; default dashboard work with its 30-second cadence remained enabled.
No build or test competed with measured requests.

## Requests and method

All requests use `/Observation` and `_count=20`; the filters below are shown before URL encoding.
Each form was exercised as `_total=accurate`, `_total=none`, and `_summary=count` without a conflicting total mode.

| Case | Filter | Exact matches |
|---|---|---:|
| unfiltered | Unfiltered | 7,699,987 |
| token_system | `code=http://loinc.org\|8302-2` | 175,355 |
| token_bare | `code=8302-2` | 175,355 |
| token_quantity | `code=8302-2&value-quantity=gt150` | 146,589 |
| token_quantity_unit | `code=8302-2&value-quantity=gt150\|\|cm` | 146,589 |
| token_quantity_ucum | `code=8302-2&value-quantity=lt50\|http://unitsofmeasure.org\|cm` | 200 |
| composite | `code-value-quantity=http://loinc.org\|8302-2$gt150` | 146,589 |

Three exploratory repetitions preceded 30 serial warm samples per group: **630 successful responses per stage**, with no errors or timeouts.
Each stage contains 420 actual first-page responses and 210 summary responses without entries. An empty summary ID array is not a page observation.
HTTP timings run from request start through the complete response body, excluding client JSON parsing, using identity compression and persistent HTTP/1.1 where allowed. They do not measure first-page visibility in a browser.
The current PostgreSQL path still performs COUNT and page retrieval for `_summary=count`, then removes entries during REST serialization. An isolated SQL COUNT is therefore a diagnostic, not equivalent HTTP work.

An external harness obtained predicates from the actual `PostgresQueryBuilder` and used the actual COUNT/page wrappers in `search_impl.rs`; it did not hand-copy SQL predicates.
Runtime capture verified all 14 statement/typed-binding pairs and 35 execute correlations for each binary.
Separate correctness runs made 122 GETs per stage: default 20-row pages and two-row `_id`-sorted next/previous walks for six filtered forms, plus seven summaries.
Exact count and ordered-ID fingerprints matched between stages, including all warm and eligible cold responses.

## Warm HTTP results

All cells are **baseline → candidate seconds**, rounded to three decimals; calculations use unrounded samples.
The first table reports empirical nearest-rank p95 (the 29th sorted value of 30) for every group, including controls and small regressions.

| Case | Accurate p95 | None p95 | Summary=count p95 |
|---|---:|---:|---:|
| unfiltered | 0.610 → 0.599 | 0.001 → 0.001 | 0.586 → 0.578 |
| token_system | 1.288 → 1.208 | 0.002 → 0.002 | 1.215 → 1.194 |
| token_bare | 0.972 → 0.962 | 0.001 → 0.002 | 0.943 → 0.909 |
| token_quantity | 6.399 → 2.612 | 3.213 → 1.286 | 6.380 → 2.554 |
| token_quantity_unit | 4.985 → 2.578 | 2.491 → 1.247 | 4.942 → 2.496 |
| token_quantity_ucum | 1.507 → 0.977 | 0.705 → 0.486 | 1.434 → 1.014 |
| composite | 1.032 → 1.014 | 0.002 → 0.002 | 0.995 → 0.996 |

Observed maxima are retained here as well; the p95 target is not a claim that every possible request completes within 3 seconds.

| Case | Accurate maximum | None maximum | Summary=count maximum |
|---|---:|---:|---:|
| unfiltered | 0.649 → 0.601 | 0.001 → 0.001 | 0.625 → 0.598 |
| token_system | 14.611 → 1.224 | 0.002 → 0.002 | 1.285 → 1.267 |
| token_bare | 1.027 → 0.976 | 0.002 → 0.002 | 0.989 → 0.922 |
| token_quantity | 12.434 → 2.673 | 3.301 → 1.303 | 6.628 → 2.604 |
| token_quantity_unit | 18.839 → 2.641 | 2.571 → 1.376 | 4.953 → 2.542 |
| token_quantity_ucum | 14.270 → 0.987 | 0.709 → 0.487 | 1.435 → 1.054 |
| composite | 1.186 → 1.046 | 0.002 → 0.002 | 1.049 → 1.028 |

The plain code+quantity accurate p95 falls from 6.399 to 2.612 seconds; unit-qualified accurate falls from 4.985 to 2.578, and UCUM accurate from 1.507 to 0.977.
Unfiltered, single-token and composite controls retain byte-identical COUNT/page SQL and typed binds. Their timing variation is not attributed to this change.

## PostgreSQL-buffer-cold results

Cold means independently restarted PostgreSQL shared buffers, with OS cache retained; HTTP includes reconnect. It is not an OS-cold measurement.
Each stage preserves 64 successful attempts: 63 eligible samples, three per group, and one attempt contaminated by dashboard work starting before GET.
An explicit replacement sample restores that group's coverage without deleting its contaminated attempt.
Eligibility is evaluated at GET start; dashboard work beginning during GET remains in the samples (four baseline attempts and three candidate attempts).
SQL cold capture used separate passes with HFS stopped and 42 independent restarts per stage, three for each of 14 statements.
Three samples support only **median [minimum, maximum]**, not a cold p95 or tail guarantee. All values below are seconds, baseline → candidate.

| Case | Accurate median [min,max] | None median [min,max] | Summary=count median [min,max] |
|---|---:|---:|---:|
| unfiltered | 1.068 [1.066, 1.085] → 1.061 [1.056, 1.062] | 0.006 [0.006, 0.006] → 0.006 [0.006, 0.007] | 1.068 [1.056, 1.088] → 1.059 [1.059, 1.072] |
| token_system | 2.033 [2.027, 2.883] → 2.017 [2.012, 2.967] | 0.008 [0.008, 0.009] → 0.009 [0.008, 0.009] | 2.073 [2.059, 2.086] → 2.025 [2.022, 2.035] |
| token_bare | 1.123 [1.099, 1.140] → 1.116 [1.100, 1.158] | 0.009 [0.008, 0.009] → 0.009 [0.009, 0.009] | 1.111 [1.108, 1.136] → 1.111 [1.101, 1.141] |
| token_quantity | 8.059 [7.724, 10.031] → 3.164 [3.147, 3.262] | 4.674 [4.641, 4.674] → 2.004 [2.003, 2.037] | 7.768 [7.646, 7.781] → 3.142 [3.117, 3.160] |
| token_quantity_unit | 6.988 [6.367, 10.469] → 3.697 [3.660, 3.721] | 4.149 [4.137, 4.169] → 2.533 [2.521, 2.613] | 6.349 [6.329, 6.923] → 3.738 [3.655, 3.756] |
| token_quantity_ucum | 1.831 [1.803, 1.922] → 1.243 [1.224, 1.256] | 1.169 [1.160, 1.191] → 0.778 [0.777, 0.781] | 1.817 [1.794, 1.855] → 1.246 [1.237, 1.280] |
| composite | 1.720 [1.714, 1.729] → 1.726 [1.711, 1.797] | 0.009 [0.009, 0.009] → 0.009 [0.009, 0.009] | 1.762 [1.719, 1.785] → 1.736 [1.709, 1.770] |

Cold accurate medians remain **3.164 seconds for plain quantity and 3.697 seconds for unit-qualified quantity**, above 3 seconds. The accepted latency target applies to warm HTTP only.

## Mechanism and plan evidence

`PostgresQueryBuilder::foldable_groups` recognizes only Denormalized `Observation` with exactly two distinct parameters: one `code` Token and one `value-quantity` Quantity with `Gt` or `Lt`.
Each must have one value, no modifier, chain or composite components, and must build a simple positive membership arm.
Reverse chains, lists, compartments, contained searches, extra or repeated parameters and other quantity prefixes retain their existing behavior; existing repeated-name folding remains unchanged.
Single-token/composite fast paths and Legacy compatibility are preserved. Sort, cursor and offset processing retain the shared outer search contract.

`adaptive_code_quantity_membership` materializes a policy probe for more than **1,024 matching token index rows**, including duplicate/stale rows rather than distinct live resources.
Broad matches use the original arms under `INTERSECT`, deduplicating IDs before resource lookups; rare matches retain the original live-resource conjunction under a mutually exclusive `UNION ALL` branch.
Eligible bare numeric quantity arms also have an inexpensive existence guard. Unit/canonical arms do not use that guard.
Tenant/type, soft-delete filters, original raw/canonical OR semantics and typed bindings remain intact. Code-first emission rebuilds offsets correctly even when input parameters are reversed.

The final COUNT and page plans show the same reduced work; table cells are before → after.
Root buffer accesses mean inclusive Shared Hit + Read counters, not unique blocks; child counters are not summed.

| Case | Resource PK probes | Per-resource token probes | COUNT root buffer accesses | Page root buffer accesses | Temp read/write blocks |
|---|---:|---:|---:|---:|---:|
| token_quantity | 423,382 → 146,589 | 0 → 0 | 2,404,338 → 1,020,410 | 2,404,344 → 1,020,416 | 1,025/2,140 → 0/0 |
| token_quantity_unit | 146,589 → 146,589 | 146,589 → 0 | 1,649,108 → 890,633 | 1,649,114 → 890,639 | 0/0 → 0/0 |
| token_quantity_ucum | 14,993 → 200 | 14,993 → 0 | 259,155 → 96,155 | 259,161 → 96,161 | 0/0 → 0/0 |

Each active intersection arm executes once; inactive fallback descendants have loops zero. Policy reads 1,025 token rows once; the plain quantity guard reads one row once.
Quantity scans remain: plain/unit/UCUM arms produce 423,382/146,589/14,993 IDs, intersected with 175,355 token IDs to yield 146,589/146,589/200.
The page still visits qualifying live resources and sorts before returning its lookahead row; no new early LIMIT is claimed.
Baseline plain dedup spills across five batches; candidate dedup uses one batch without disk spill at this cardinality.
Estimates still underestimate quantities (56,625/285/276 versus 423,382/146,589/14,993 actual) and candidate outer dedup estimates 200 even when 146,589 rows occur.
Runtime policy does not repair statistics. Individual EXPLAIN execution-time ratios are not causal speedups: captures have different cache misses and include instrumentation overhead. Shared reads can be served by OS cache.

The 1,024-row threshold is an internal measured heuristic, not a universal optimum. Selective prototypes verified missing-code and empty-bare-quantity guards; rare positive codes were not benchmarked on this corpus.
Duplicates can route a small unique set through the broad branch. Unit/UCUM empty ranges, larger result sets, memory pressure, generic plans and other cache/hardware distributions remain performance limits.

## Automated validation and reproduction

Focused checks passed **138 Rust tests**, including four new tests for typed-binding order, narrow eligibility, exact semantics/paging through both runtime branches, and bounded active-arm work on a 20,000-resource fixture.
They cover raw/canonical quantities, historical NULL canonical values, unit-code/display differences, multi-valued indices, duplicate/stale rows, deleted resources, tenant collisions, totals accurate/estimate/none, cursor/offset and fallback forms.
Plan assertions check work and loops without forcing a planner, exact plan tree or timing deadline. Fixtures use isolated PostgreSQL testcontainers, separate from the measured corpus.
The release build and focused commands were run serially with `CARGO_BUILD_JOBS=4`:

```bash
cargo build --locked --no-default-features --features R4,postgres,ui -p helios-hfs --release
cargo test --locked -p helios-persistence --features postgres --lib backends::postgres::search::query_builder::tests -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --lib fast_path_tests -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_code_quantity_intersection -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_quantity -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_composite -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_composite -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_by_token -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_tenant_isolation -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_cursor_paging -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_search_cursor_with_custom_sort -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_comma_list_is_or_and_repeated_param_is_and -- --test-threads=1
cargo test --locked -p helios-persistence --features postgres --test postgres_tests postgres_integration_same_id_different_tenants -- --test-threads=1
```

The existing automated UI smoke passed one test in 2.9 seconds with pinned Playwright 1.49.1 and Chromium revision 1148 against a separate small PostgreSQL fixture.
It verifies a three-match result with a two-row page and explicit `_total=none`; it does not establish corpus performance or manual visual acceptance.
From `crates/ui/e2e`, after installing its pinned dependencies/browser, set `HFS_E2E_BASE_URL` to an isolated HFS origin and run:

```bash
npx playwright test tests/queries.spec.ts --project=chromium --grep 'the results header shows the match count, not the page size'
```

For a new benchmark, freeze both release binaries, keep schema/settings/corpus identical, generate SQL from the actual builder and verify runtime SQL/binds before sampling.
Measure all seven forms/modes serially with 30 warm requests each; keep every sample and classify cold attempts at GET start with separate SQL restarts.
Raw HTTP samples, correctness fingerprints, SQL/bind captures, JSON plans, cold eligibility and command logs are retained as external benchmark evidence; there is no tracked benchmark runner to invoke here.

## Supplemental automated browser QA

Original manual before/after QA in existing Chrome was not run; this limitation was explicitly accepted, and supplemental Playwright QA was authorized. Headless Chromium used fresh contexts, JavaScript enabled and a 1492×780 viewport.
One serial pass per stage measured UI button activation through the updated result DOM plus two animation frames, with buffers not reset. These are single observations, not warm p95; tracing, instrumentation, routing and screenshots can affect duration.
All 30 observations per stage showed loading and matching URLs, ordered IDs, counts and headers: seven first pages of 20 rows, seven next/previous pairs, seven empty exact-count summaries, plus quantity estimate (146,589) and none (20+) checks. All non-GETs were blocked before transport, including 16 recent-history PATCHes per stage; persistence was excluded. Expected console/page errors from blocked writes were retained, with no functional search failures or corpus writes.

| Case | First-page visible seconds, baseline → candidate |
|---|---:|
| unfiltered | 0.641 → 0.625 |
| token_system | 1.976 → 1.859 |
| token_bare | 1.043 → 0.993 |
| token_quantity | 7.819 → 2.926 |
| token_quantity_unit | 5.659 → 3.043 |
| token_quantity_ucum | 1.778 → 1.160 |
| composite | 1.126 → 1.126 |

The candidate unit-qualified observation was **3.043 seconds**, above the visible below-3-second target; its cause is undetermined. This pass establishes functional coverage, not that all pages are visible within 3 seconds. The separately measured warm HTTP target remains 21/21 groups.

A separate candidate-only focused warm UI verification then ran 30 serial unit-qualified accurate requests with 20-row pages in a fresh context and a new runtime from the same frozen binary. Headless Chromium, JavaScript, 1492×780 viewport, tracing, routing, loading/result screenshots and activation-to-DOM-plus-two-frame timing were unchanged; normal background work remained enabled and buffers were not reset. Initial Resources preparation and one predeclared unit warm-up (4.025 seconds) were retained separately and excluded from statistics. All 30 samples preserved loading, 146,589 matches, headers and ordered IDs, and were below 3 seconds: nearest-rank p95 **2.693 seconds** (rank 29/30), p50 2.526, minimum 2.461 and maximum 2.776. All 31 recent-history PATCHes were blocked, with expected errors retained. This is not a paired baseline comparison or a browser p95 for all seven forms; the original 3.043-second observation remains unchanged and unexplained.
