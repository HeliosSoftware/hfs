# FHIR Search — Implementation Assessment

This document assesses the Helios FHIR Server (HFS) implementation of FHIR Search against the
[FHIR R4+ Search specification](https://build.fhir.org/search.html). It is the narrative companion
to the **Backend Capability Matrix** in [`../README.md`](../README.md): the matrix gives the
per-backend ✓/◐/○/✗ grid; this document explains *what* each capability means, *where* it is
implemented (REST layer vs. persistence backend), and *what is missing*.

Last reconciled against the code: see git history of this file. Evidence is cited as
`crate/path:line` where useful.

## How a search request flows

```
HTTP request
  → helios-rest: parse query string → build SearchQuery        (crates/rest/src/extractors/)
  → helios-rest: terminology pre-processing (:in expansion)     (crates/rest/src/handlers/search.rs)
  → helios-persistence: SearchProvider::search(tenant, query)   (per-backend search_impl.rs)
  → helios-rest: post-process (_summary / _elements subsetting) (crates/rest/src/responses/subsetting.rs)
  → Bundle with self / next / previous links
```

The REST layer is **version-agnostic and backend-agnostic**: it parses essentially the full search
grammar into a `SearchQuery` (`crates/persistence/src/types/search_params.rs`). What actually
executes depends on the configured backend. Most gaps are therefore in the backends, not in REST.

Supported backends for search: **SQLite** (reference implementation), **PostgreSQL**, **MongoDB**
(partial native), **Elasticsearch** (search-optimized secondary). **S3** is storage-only and
returns `UnsupportedCapability` for all search operations (`backends/s3/storage.rs`). Cassandra and
Neo4j are not implemented.

## 1. Search parameter types

| Type | SQLite | PostgreSQL | MongoDB | Elasticsearch | Notes |
|------|:------:|:----------:|:-------:|:-------------:|-------|
| string | ✓ | ✓ | ✓ | ✓ | prefix (default), `:exact`, `:contains` |
| token | ✓ | ✓ | ✓ | ✓ | `system\|code`, `\|code`, `system\|`, code-only |
| reference | ✓ | ✓ | ✓ | ✓ | type modifier + `:identifier` (SQLite/ES) |
| date | ✓ | ✓ | ✓ | ✓ | precision-aware ranges + all prefixes |
| number | ✓ | ✓ | ✓ | ✓ | implicit-precision ranges + all prefixes |
| quantity | ✓ | ✓ | ✓ | ✓ | value comparison + optional system/unit on all backends |
| uri | ✓ | ✓ | ✓ | ✓ | exact + `:above`/`:below` prefix matching |
| composite | ✓ | ✓ | ✗ | ✓ | SQLite/PG group by `composite_group`; ES uses one nested object per instance; Mongo returns no condition |

The `resource` and `special` parameter types from the spec are modeled in the `SearchParamType`
enum but have no dedicated execution path beyond the special common parameters below.

**Composite (SQLite, PostgreSQL):** works end-to-end. The REST layer resolves each component's
type and code from the registry (by the component `definition` URL); the extractor indexes every
composite instance as a set of `search_index` rows sharing a `composite_group`; and the backend
matches with `GROUP BY resource_id, composite_group HAVING <every component present>`, so all
components must be satisfied within the same instance. Elasticsearch still matches the composite
name only (◐).

**Choice types (`value[x]`):** the extractor evaluates FHIRPath against schema-less JSON, where a
cast such as `value as Quantity` / `value.ofType(Quantity)` cannot resolve to the stored
`valueQuantity` field. `rewrite_choice_types` in `search/extractor.rs` rewrites these casts to the
concrete element name (`valueQuantity`, `medicationCodeableConcept`, `occurrenceDateTime`, …)
before evaluation. This fixed both composite value components and plain `value[x]` parameters
(e.g. `value-quantity`), which previously indexed nothing.

## 2. Search modifiers

| Modifier | SQLite | PostgreSQL | MongoDB | Elasticsearch |
|----------|:------:|:----------:|:-------:|:-------------:|
| `:missing` | ✓ | ✓ | ✗ | ✓ |
| `:exact` | ✓ | ✓ | ✓ | ✓ |
| `:contains` | ✓ | ✓ | ✓ | ✓ |
| `:text` | ✓ | ◐¹ | ✗ | ✓ |
| `:not` | ✓ | ✓ | ✗ | ✓ |
| `:of-type` | ✓ | ✓ | ✗ | ✓ |
| `:text-advanced` | ✓ | ✗ | ✗ | ✓ |
| `:above` / `:below` (URI) | ✓ | ✓ | ✗ | ✓ |
| `:above` / `:below` (token hierarchy) | †³ | †³ | †³ | †³ |
| `:in` / `:not-in` | †² | †² | †² | †² |
| `:identifier` (reference) | ✓ | ✗ | ✗ | ✓ |
| `:[type]` (reference) | ✓ | ✗ | ✓ | ✓ |
| `:code-text` | ✗ | ✗ | ✗ | ✗ |

¹ PostgreSQL implements `_text`/`_content` full-text search via `tsvector`, but the token `:text`
  modifier itself is not wired up.
² `:in` is expanded by the REST layer against a configured terminology server before the query
  reaches the backend (`crates/rest/src/handlers/search.rs`); `:not-in` returns `501 Not
  Implemented`. No backend resolves either modifier natively.
³ Token `:above`/`:below` (`code:below=system|code`) is resolved at the REST layer: the code and its
  descendants (`is-a`, for `:below`) or ancestors (`generalizes`, for `:above`) are expanded via the
  terminology server's `$expand`, then matched as a plain token OR list. Works on every backend when
  `HFS_TERMINOLOGY_SERVER` is configured; URI `:above`/`:below` is separate and native (above).

The REST layer parses **all** of these modifiers (`crates/rest/src/extractors/search_query_builder.rs`)
regardless of backend; unsupported ones either no-op, error, or (for some ES/Mongo cases) return no
matches. MongoDB fails closed with an explicit error on unsupported modifiers; Elasticsearch tends
to silently match nothing — see Known Limitations.

## 3. Comparator prefixes

All nine prefixes (`eq`, `ne`, `gt`, `lt`, `ge`, `le`, `sa`, `eb`, `ap`) are parsed by REST and
honored by the date / number / quantity handlers on SQLite, PostgreSQL, MongoDB, and Elasticsearch.
Prefixes are only extracted for ordered types; a token value such as `appended` is preserved
verbatim and not misread as the `ap` prefix (regression-tested in the REST extractor).

## 4. Special / common parameters

| Parameter | Where handled | SQLite | PostgreSQL | MongoDB | Elasticsearch |
|-----------|---------------|:------:|:----------:|:-------:|:-------------:|
| `_id` | backend | ✓ | ✓ | ✓ | ✓ |
| `_lastUpdated` | backend | ✓ | ✓ | ✓ | ✓ |
| `_tag` / `_profile` / `_security` / `_source` | backend (token/uri) | ✓ | ✓ | ✓ | ✓ |
| `_text` (narrative) | backend FTS | ✓ | ✓ | ✗ | ✓ |
| `_content` (full content) | backend FTS | ✓ | ✓ | ✗ | ✓ |
| `_filter` | backend | ✓ | ✗ | ✗ | ✗ |
| `_has` (reverse chaining) | REST + backend | ✓ | ✓ | ✗ | ✗ |
| `_type` (system search) | REST | ✓ | ✓ | ✓ | ✓ |
| `_list` | passthrough param | ○ | ○ | ○ | ○ |
| `_query` | — | ✗ | ✗ | ✗ | ✗ |
| `_contained` / `_containedType` | stripped by REST | ✗ | ✗ | ✗ | ✗ |

`_filter` is parsed and executed only by the SQLite backend (full expression parser in
`backends/sqlite/search/filter_parser.rs`). Note the REST layer does not give `_filter` special
handling; SQLite picks it up as a recognized parameter name. On other backends `_filter` is
effectively a no-op.

## 5. Chaining, reverse chaining, include/revinclude

| Capability | SQLite | PostgreSQL | MongoDB | Elasticsearch |
|------------|:------:|:----------:|:-------:|:-------------:|
| Forward chained params (N-level) | ✓ | ✓ | ◐ | ◐ |
| Reverse chaining (`_has`), incl. **nested** | ✓ | ✓ | ◐ | ◐ |
| `_include` / `_revinclude` | ✓ | ✓ | ✓ | ✓ |
| `:iterate` on include | ✓¹ | ✓¹ | parsed | ✓ (inline) |
| `_include=Type:*` wildcard | ✓ | ✓ | ✓ | ✓ |

SQLite and PostgreSQL resolve chains natively, via nested `search_index` subqueries with
configurable depth limits (✓). For all other backends (◐), the REST layer resolves chained and
reverse-chained parameters before the backend search runs: `search::resolve_chains` issues one
plain `search()` per chain hop against the same backend and folds the result into an `_id`
filter — application-side joins. So chained and `_has` queries work end-to-end over HTTP on every
searchable backend, including Elasticsearch and MongoDB; the per-backend distinction is whether the
join is pushed into the backend (SQLite/PG) or performed by the REST layer.

**Nested `_has`** (`_has:Observation:subject:_has:Provenance:target:agent=X`) is resolved
recursively by `resolve_reverse_chain`: the inner chain selects the qualifying source resources by
id, then the outer level collects their references to the base type. A reverse-depth cap
(`ChainConfig::max_reverse_depth`, default 4) is enforced.

**Include resolution (`_include`/`_revinclude`).** Elasticsearch and MongoDB populate `included`
inside their own `search()`. SQLite and Postgres do not — so the REST handler resolves includes via
the backend-agnostic `core::resolve_includes_iterative` whenever the backend left `included` empty.
References are extracted through the search-parameter registry's FHIRPath expression (so a parameter
whose name differs from its JSON field — e.g. Patient `organization` → `managingOrganization` —
resolves correctly), and the referenced/referencing resources are fetched with `search()`.

¹ `:iterate` transitively follows includes of already-included resources (depth-capped, deduped) via
  `resolve_includes_iterative`. Both spellings are accepted: `_include=Obs:subject:iterate` and the
  spec's `_include:iterate=Obs:subject`. `_include=Type:*` expands at query-build time to one
  directive per reference search parameter of `Type`.

## 6. Result control (paging, sort, total, summary, elements)

These are parsed and largely orchestrated by the REST layer.

| Parameter | Status | Notes |
|-----------|--------|-------|
| `_count` | ✓ | page size; `_offset`/`_cursor` for paging |
| `_sort` | ✓ | SQLite/PG sort by any indexed param; see below |
| `_total` | ✓ | `accurate`/`estimate` populate `Bundle.total` (incl. SQLite/PG via `search_count`); `none`/absent omit it |
| `_summary` | ✓ | `true`/`text`/`data`/`count`/`false`, applied in `subsetting.rs`; adds `SUBSETTED` `meta.tag` |
| `_elements` | ✓ | applied post-search with nested-path support; adds `SUBSETTED` `meta.tag` |
| `_include` / `_revinclude` | ✓ | see §5 |
| `_maxresults` | ✗ | not handled |
| `_score` | ✗ | bundle field exists but is never populated |
| Bundle `self` link | ✓ | echoes executed params |
| `next` / `previous` links | ✓ | cursor-based |
| `first` / `last` links | ✗ | not generated |

**`_sort` detail.** `_sort` is parsed into `SearchQuery.sort`; the REST layer resolves each sort
field's type from the registry (`SortDirective.param_type`). On SQLite and PostgreSQL, `_id` and
`_lastUpdated` sort on the `resources` table, while any other indexed parameter sorts on a
correlated subquery into `search_index` — `MIN(value_col)` for ascending, `MAX(value_col)` for
descending (FHIR multi-value sort), with the value column chosen by the parameter type. Cursor
(keyset) pagination is consistent with the sort: the boundary row's sort value is selected, encoded
into the opaque `PageCursor` alongside the id, and the next/previous page applies a keyset `WHERE`
`(sort_expr, id)` comparison on it — so deep paging preserves the order. A multi-field `_sort`
returns a single page (no cursor) rather than risk an inconsistent keyset. MongoDB sorts by
`_id`/`_lastUpdated` only and cannot combine a custom sort with cursor pagination; Elasticsearch
sorts on its mapped fields via `search_after`.

## 7. Known limitations & roadmap

Ordered roughly by impact:

1. **Multi-field `_sort` + cursor** — single-field sorts (and the default `_lastUpdated`) page
   correctly via the keyset cursor on SQLite/PG. A multi-field `_sort` currently returns a single
   page (no cursor); extending the keyset to a multi-key tuple is the remaining work. MongoDB still
   sorts by `_id`/`_lastUpdated` only.
2. **Terminology-dependent modifiers** — `:in` and token `:above`/`:below` are resolved at the REST
   layer via terminology `$expand` (functional when `HFS_TERMINOLOGY_SERVER` is set; no native
   in-backend resolution). `:not-in` still returns `501` — negated value-set filtering is the
   remaining gap.
   URI `:above`/`:below` (hierarchical prefix, no service needed) *is* implemented on SQLite/PG/ES.
3. **PostgreSQL modifier gaps** — only the `:text-advanced` modifier remains unimplemented relative
   to SQLite (`:exact`, `:contains`, `:not`, `:missing`, `:of-type`, URI `:above`/`:below`, and
   composite parameters are all supported now).
4. **MongoDB native search gaps** — composite parameters error out; `_text`/`_content` and most
   modifiers beyond `:exact`/`:contains` are unsupported. (Quantity search is now implemented;
   chained/`_has` work via the REST-layer resolver.)
5. **Elasticsearch gaps** — `_filter` unsupported. (Composite now evaluates components via inline
   nested objects; chained/`_has` now work via the REST-layer resolver — see below.)
6. **REST result params** — `_maxresults`, `_score`, `_query`, `_contained`/`_containedType`
   unsupported; Bundles omit `first`/`last` paging links.
7. **Quantity UCUM canonicalization** (schema v10) — the index stores a dimension-canonical
   value/unit (`value_quantity_canonical_value`/`_unit`) computed via
   `helios_fhirpath::ucum::canonicalize_quantity`, so `1|g` matches `1000|mg`.
   - **SQLite: complete** — writer populates canonical columns; the quantity handler matches the
     canonical columns (search bounds canonicalized to preserve implicit precision) OR the raw unit.
   - **Elasticsearch: complete** — canonical `value`/`unit` stored in the `quantity` nested object;
     the handler ORs a canonical range (bounds canonicalized) with the raw match (integration-tested
     against a live ES container).
   - **Postgres: complete** — writer populates canonical columns; the quantity handler ORs a
     range-based canonical predicate (bounds canonicalized; eq uses an implicit-precision range that
     also absorbs float-conversion noise) with the raw match. Integration-tested against a live PG
     container.
   - A **reindex backfill** (`ReindexRequest`) is required to populate canonical columns for
     resources written before the upgrade; un-reindexed rows fall back to raw unit matching.

   All three searchable backends (SQLite, Postgres, Elasticsearch) now match UCUM-equivalent units.
8. **Accent-insensitive string search** (schema v10) — string search folds case **and** accents via
   NFD + combining-mark stripping (`unicode-normalization`, shared `search::fold_text`), stored in
   `value_string_folded`.
   - **SQLite: complete** — default/`:contains`/`:text` match the folded column ORed with the raw
     (case-insensitive) column; `:exact` stays case/accent-sensitive.
   - **Postgres: complete** — `COALESCE(value_string_folded, value_string) ILIKE` against a folded
     pattern (raw fallback keeps non-accented pre-reindex rows matching case-insensitively).
   - **Elasticsearch: complete** — a `folded` keyword field (written via `fold_text`) is matched
     (wildcard) ORed with the raw field; integration-tested against a live ES container.
   - Pre-reindex rows need the **reindex backfill** for accent-insensitivity.
9. **Canonical `url|version` references** — versioned reference matching is now version-agnostic
   (`Patient/1/_history/2` ⇄ `Patient/1`, via `strip_reference_version`, on SQLite/PG/ES). The
   remaining gap is canonical `url|version` hierarchy for reference `:above`/`:below`; `:identifier`
   also assumes a `Type/id` reference shape.
10. **Ordered-value boundary semantics** — **done** (SQLite, Postgres, Elasticsearch). Comparator
    prefixes now match against the search value's implicit-precision range boundaries per spec:
    `ge → x≥lo`, `le → x<hi`, `gt`/`sa → x≥hi`, `lt`/`eb → x<lo` (range `[lo, hi)`), for number,
    quantity, and date (date falls back to scalar for full-precision instants). Postgres also gained
    implicit-precision `eq`/`ne` ranges for number/date (was exact). Shared `search::range` helper.

**Recently landed (REST layer).** Repeated query parameters are now preserved as FHIR AND semantics
(previously collapsed to last-wins by `HashMap` extraction); `Prefer: handling=strict` rejects
unknown search parameters with `400` on type-level search (lenient default still ignores them);
`_include`/`_revinclude` (with `:iterate` and `Type:*` wildcard) resolve on SQLite/Postgres;
nested `_has` resolves; `Bundle.total` populates on SQLite/Postgres; and `_summary`/`_elements`
responses carry the `SUBSETTED` tag.

**Chaining dispatch (resolved).** Chained (`subject.name=Smith`) and reverse-chained
(`_has:Observation:subject:code=1234`) searches are parsed into `SearchQuery` (`param.chain`,
`reverse_chains`), but the 2-arg `SearchProvider::search` does not act on them. The REST search
handler now calls `search::resolve_chains` first: a backend-agnostic resolver that performs the
chain as application-side joins (one plain `search()` per hop, results folded into an `_id`
filter), then runs the rewritten query. This works for any `SearchProvider`, so chained and `_has`
queries are functional end-to-end on SQLite, PostgreSQL, MongoDB, and Elasticsearch. SQLite and PG
additionally resolve chains natively in-backend.

SQLite is the most complete backend and serves as the reference for the others; PostgreSQL is now
at near-parity (only `:text-advanced` remains).
