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

## Out of first cut (tracked follow-ups)
- A5a for Postgres / Elasticsearch / MongoDB query builders.
- A5b all-types compartment search.
- A1b full `|` / `$` escaping inside token & composite value parsing.
- Per-param `modifier` arrays in the CapabilityStatement.
