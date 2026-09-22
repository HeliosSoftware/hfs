# Search spec-conformance fixes — plan & status

**Branch:** `fix/search-spec-conformance-fixes` (off `main`)
**Source:** the Part-A findings from the search-conformance assessment of `hfs`
against https://build.fhir.org/search.html. Part-B (doc-only) corrections live on
the `docs/ui-requirements` branch.

Each item notes the spec basis, the fix, the files touched, and backend coverage.
"Default path" = SQLite, the zero-config default backend.

## A1 — Parameter types & special params
- **A1a — Reject unsupported `_query` / `_list` / `_contained` / `_containedType`
  / `_score`.** Today they are silently ignored (HTTP 200, unfiltered). FHIR
  servers must not silently drop a constraint the client asked for. Fix: reject
  with `400` (these are known-but-unimplemented control params), in the REST
  search handler, for all backends. Files: `crates/rest/src/handlers/search.rs`.
- **A1b — Search-value escaping.** FHIR escapes `, | $ \` in values with `\`.
  `hfs` did no unescaping, so a literal comma always split OR-values. Fix:
  split the comma-separated value list on *unescaped* commas and unescape `\,`
  and `\\` in each value. Files:
  `crates/rest/src/extractors/search_query_builder.rs`. (Pipe/`$` escaping inside
  token `system|code` and composite values remains a follow-up — tracked below.)
- **A1c — `_filter` parse failure.** A malformed `_filter` was dropped with only a
  logged warning, leaving an unfiltered superset. Implemented as **fail-closed**:
  a malformed `_filter` now emits a match-nothing condition (`1 = 0`) so the
  client gets zero results rather than wrong ones. (A true `400` would require
  threading a `Result` through `QueryBuilder::build`, used by production + tests —
  tracked as a follow-up.) Files:
  `crates/persistence/src/backends/sqlite/search/query_builder.rs`.

## A2 — Modifiers
- **A2b — `:text-advanced` applicability.** Spec (build.fhir.org): **reference +
  token**. `hfs` had **string + token** (wrong in both directions). Fix:
  `SearchModifier::is_valid_for` → `Token | Reference`. Files:
  `crates/persistence/src/types/search_params.rs`.
- **A2c — Token `:in` / `:above` / `:below` with no terminology server.** Today
  they silently fall through to literal/no-op matching and return 200 with wrong
  results. Fix: when no `HFS_TERMINOLOGY_SERVER` is configured, reject these token
  modifiers with `501` (fail loud), mirroring the existing `:not-in` handling.
  Files: `crates/rest/src/handlers/search.rs`.
- **A2d — `:code` token modifier removed entirely.** `:code` was a non-spec
  `hfs` invention with no FHIR basis. Its SQL (`value_token_code = ?`) is
  byte-for-byte identical to a plain code match (`code=X`), and it actively
  *breaks* `system|code` values (it matches the literal `"system|code"` as a
  code). Redundant at best, harmful at worst. Removed from the `SearchModifier`
  enum, parser, validity table, the SQLite token handler, the Mongo no-op arm,
  and the SQLite/Mongo `modifiers_for_type` advertisements (with regression-guard
  tests asserting it stays gone). Files:
  `crates/persistence/src/types/search_params.rs`,
  `crates/persistence/src/backends/sqlite/search/parameter_handlers/token.rs`,
  `crates/persistence/src/backends/{sqlite,mongodb}/backend.rs`,
  `crates/persistence/src/backends/mongodb/search_impl.rs`.
- *No change (verified conformant):* `:contains` (reference/string/uri) and
  `:above`/`:below` (reference/token/uri) match build.fhir.org — the earlier
  "deviation" reading was incorrect.

## A3 — Prefixes
- **`sa` / `eb` applicability.** Spec: **date + quantity** (not number). `hfs`'s
  `SearchPrefix::is_valid_for` returned date-only. Fix: `Date | Quantity`. (The
  request path performs no prefix/type validation, so this table is advisory
  today; correcting it removes the latent inconsistency and is used by the
  CapabilityStatement-adjacent logic.) Files:
  `crates/persistence/src/types/search_params.rs`.

## A4 — Result controls
- **A4a — Unsupported `_sort` field.** Today an unknown/unsortable sort field
  silently falls back to `id`. Fix: under `Prefer: handling=strict`, reject an
  unknown sort field with `400`; under lenient, keep the fallback (spec-allowed)
  but it is now visible via the validation path. Files:
  `crates/rest/src/handlers/search.rs`.
- **A4b — `first` pagination link.** Search Bundles emitted `self` + `next` /
  `previous` but never `first` / `last`. Fix: add a `first` link (the self URL
  with `_cursor` / `_offset` stripped). `last` is intentionally **not** added:
  under keyset (cursor) paging — the default — the last page is not cheaply
  computable, which is why it stays absent. Files:
  `crates/persistence/src/core/search.rs`.

## A5 — Includes, chaining, compartments
- **A5a — Compartment multi-param membership (correctness bug).** For target
  types that join a compartment via several reference params (e.g.
  AllergyIntolerance via `patient` / `recorder` / `asserter`), `hfs` applied only
  the **first** param, silently dropping legitimate members. Fix: a new
  `SearchQuery.compartment` field carrying *all* membership params + the
  reference, emitted as one OR'd subquery
  (`… param_name IN (p1,p2,…) AND value_reference = ref`). Files:
  `crates/persistence/src/types/search_params.rs` (model),
  `crates/persistence/src/backends/sqlite/search/query_builder.rs` (default path),
  `crates/rest/src/handlers/compartment.rs` (handler). **Backend coverage:**
  SQLite (default) implemented; Postgres/ES/Mongo to follow the same field
  (tracked as follow-up — they retain first-param behavior until wired).
- **A5b — All-types compartment `/{type}/{id}/*`.** Returns `400` today. Plan:
  iterate the compartment's member target types (bounded by the compartment
  definition) and union per-type searches. Larger surface (routing + multi-type
  bundle merge); tracked as follow-up, not in the first cut.
- **A5c — Forward-chain depth cap.** The active application-side resolver has no
  cap (reverse `_has` is capped at 4). Fix: apply the same `ChainConfig`
  forward-depth cap in the application-side resolver. Files:
  `crates/persistence/src/search/chain_resolver.rs`.

## A6 — CapabilityStatement
- **`_text` / `_content` type.** Advertised as `string`; spec type is `special`.
  Fix → `special`. Files: `crates/rest/src/handlers/capabilities.rs`.
- **Per-type search params.** `/metadata` advertised only the 7 common params for
  every type. Fix: emit each resource type's real search params from the loaded
  `SearchParameterRegistry` (already reachable from the handler via
  `state.storage().search_param_registry()`). Files:
  `crates/rest/src/handlers/capabilities.rs`.

## A7 — Combining values (comma-lists)
- **A7a — MongoDB: comma-separated date/number values were ANDed, not
  ORed (#1062).** Spec (build.fhir.org/search.html#combining): a
  comma-separated value list on one parameter is OR; only *repeated*
  parameters (`?p=a&p=b`) AND. `build_search_index_filter` singled out
  `Date`/`Number` for `$and` instead of `$or`. Because the predicate is
  evaluated against a single `search_index` document (one value of one
  parameter per resource), a disjoint list (`?date=2019,2021`) could never
  match any row — `distinct` returned empty, and `matching_resource_ids`'
  empty-set short circuit then emptied the *entire* result, including any
  other parameter in the query. A range-shaped list
  (`?date=ge2020,le2021`) accidentally read as a closed range, since one
  row's value can satisfy both bounds. The sibling `_lastUpdated` path
  (`build_resource_last_updated_conditions`, resolved against the resource
  document rather than `search_index`) had the same defect. Fix: always
  join with `$or`; `_lastUpdated`'s multiple values now wrap into one `$or`
  document instead of one `$and`-ed document per value, so the repeated
  form (two separate `_lastUpdated` parameters) still ANDs unchanged.
  **Behavior change:** `?date=ge2020,le2021` no longer means "in
  2020-2021" — it now widens to "any date >= 2020 OR any date <= 2021".
  The closed-range query is the repeated form,
  `?date=ge2020&date=le2021`, which is unaffected. Quantity was already
  correct (never in the AND list) and is unchanged; SQLite and
  Elasticsearch were already correct (OR for every type) and are
  unchanged. PostgreSQL had the same class of defect, more broadly
  (quantity included); it was fixed in #1300, and its repeated form was
  folded in #1416 (A7b).
  MongoDB was the outlier here, not an exception to a settled convention:
  the REST extractor, SQLite, Elasticsearch, and the UI's own query
  builder already narrate a comma-separated date range as OR (e.g.
  `crates/ui/e2e/tests/queries.spec.ts:1186` —
  `"birthdate is on or before “1979-12-31” or birthdate is on or
  after “1980-01-02”"`, exercised by the `mongodb`
  backend lane of `.github/workflows/ui-tests-matrix.yml`); those e2e
  assertions narrate builder hydration only, not result counts, so they
  are unaffected by this fix either way.
  Files: `crates/persistence/src/backends/mongodb/search_impl.rs`.
- **A7b - PostgreSQL: comma-separated values were ANDed, fixed in #1300, and
  the repeated form now folds into one set intersection (#1416).** Same class
  of defect as A7a and wider: `build_date_condition`, `build_number_condition`,
  `build_quantity_condition`, and `build_last_updated_condition` each folded
  their per-value conditions with `.and()`, while string, token, reference,
  uri, composite, `_id`, and full-text already ORed. Each condition was its own
  `id IN (SELECT ...)` sublink, so the AND applied *across rows*:
  `date=2019,2021` read as "has a 2019 date and a 2021 date", usually empty but
  without MongoDB's whole-result wipeout, which is why the defect was easy to
  miss here. #1300 replaced the four copies of the fold with one `or_values`
  fold: a parameter whose values are all membership tests emits ONE sublink
  with the predicates ORed inside (following `build_token_condition`), and
  `_lastUpdated`, which compares a `resources` column, ORs its ranges
  directly. A single value builds byte-identical SQL, `validate_date_values`
  rejects an uninterpretable value before a query is built, and a list of only
  invalid values is still `FALSE`. The comma list is OR; the repeated form
  (`?p=a&p=b`, separate `SearchParameter`s ANDed in `build_search_query_for`)
  is the conjunction, as the spec requires.
  **Performance follow-up (#1416).** The repeat was still built as two
  `id IN (...)` sublinks, which is two semi-joins; PostgreSQL 16 could run the
  pair as a nested loop that re-executed one arm's date index scan once per
  candidate of the other. On a 24,000-Patient corpus that was 18,000 rescanning
  probes of `idx_search_date_recent`, 7.4M buffers, and a 33.4 s page against a
  shipped 30 s `statement_timeout`. #1416 folds eligible repeats into ONE
  membership test whose arms are the occurrences' own selects joined by
  `INTERSECT`, a set operation evaluated once per arm; the same page then takes
  21.3 ms and 15,360 buffers, and the rescanning node is absent from every
  capture. Method and numbers: `docs/postgres-repeated-date-benchmark.md`; the
  durable CI check is sections BV to BZ of
  `.github/scripts/pg-search-plans.sql`.
  **The fold is generic, not Date-specific.** It keys on the parameter name:
  any name that appears two or more times, all of whose occurrences build as a
  simple positive `search_index` membership test on that name, folds, whatever
  the type; a denormalized composite folds too because it is one row predicate.
  If any occurrence is ineligible the whole name keeps the previous
  per-occurrence conjunction, so no constraint is dropped or re-read: an
  occurrence that builds no condition, a value the builder fails closed on (a
  bare `FALSE`), `:missing`, `:not`, `_id` and `_lastUpdated` column
  predicates, full-text, `:identifier`, the compartment clause, and the legacy
  composite aggregate (a `GROUP BY ... HAVING` aggregate, not a row predicate)
  all remain unfused. Test coverage: `query_builder::tests` pins the shapes,
  the placeholder order, the eligible forms, and representative ineligible
  ones: an unparseable value, both `:missing` values, `:not`, and the legacy
  composite aggregate. The gate's own test adds the wrapper-like exclusions
  that have no query-level case, such as the compartment clause's
  `param_name IN (...)` list and a set-expression arm. `search_impl.rs`'s
  fast-path test pins that a set operation is still not extractable as a
  single index predicate, and
  `postgres_integration_comma_list_is_or_and_repeated_param_is_and` pins the
  end-to-end semantics: multi-valued `Encounter.period`, `search_count`, both
  `_total` modes, cursor and offset paging, and tenant isolation.

## Out of first cut (tracked follow-ups)
- A5a for Postgres / Elasticsearch / MongoDB query builders.
- A5b all-types compartment search.
- A1b full `|` / `$` escaping inside token & composite value parsing.
- Per-param `modifier` arrays in the CapabilityStatement.
