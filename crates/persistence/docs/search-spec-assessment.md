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
| quantity | ✓ | ✓ | ✗ | ✓ | MongoDB rejects with `UnsupportedParameterType` |
| uri | ✓ | ✓ | ✓ | ✓ | exact + `:above`/`:below` prefix matching |
| composite | ✓ | ✓ | ✗ | ◐ | SQLite/PG evaluate components; ES matches name only; Mongo returns no condition |

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
| `:above` / `:below` (token hierarchy) | ✗ | ✗ | ✗ | ✗ |
| `:in` / `:not-in` | †² | †² | †² | †² |
| `:identifier` (reference) | ✓ | ✗ | ✗ | ✓ |
| `:[type]` (reference) | ✓ | ✗ | ✓ | ✓ |
| `:code-text` | ✗ | ✗ | ✗ | ✗ |

¹ PostgreSQL implements `_text`/`_content` full-text search via `tsvector`, but the token `:text`
  modifier itself is not wired up.
² `:in` is expanded by the REST layer against a configured terminology server before the query
  reaches the backend (`crates/rest/src/handlers/search.rs`); `:not-in` returns `501 Not
  Implemented`. No backend resolves either modifier natively.

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
| Forward chained params (N-level) | ✓ | ✓ | ✗ | ✗ |
| Reverse chaining (`_has`, nested) | ✓ | ✓ | ✗ | ✗ |
| `_include` | ✓ | ✓ | ✓ | ✓ |
| `_revinclude` | ✓ | ✓ | ✓ | ✓ |
| `:iterate` on include | parsed | parsed | parsed | parsed |

SQLite and PostgreSQL resolve chains via nested `search_index` subqueries with configurable depth
limits. MongoDB returns `ChainedSearchNotSupported` / `ReverseChainNotSupported`; Elasticsearch
silently returns no matches when `param.chain` or `reverse_chains` are present.

## 6. Result control (paging, sort, total, summary, elements)

These are parsed and largely orchestrated by the REST layer.

| Parameter | Status | Notes |
|-----------|--------|-------|
| `_count` | ✓ | page size; `_offset`/`_cursor` for paging |
| `_sort` | ◐ | applied for `_id`/`_lastUpdated` only; see below |
| `_total` | ✓ | `none` / `estimate` / `accurate` parsed and applied |
| `_summary` | ✓ | `true`/`text`/`data`/`count`/`false`, applied in `subsetting.rs` |
| `_elements` | ✓ | applied post-search with nested-path support |
| `_include` / `_revinclude` | ✓ | see §5 |
| `_maxresults` | ✗ | not handled |
| `_score` | ✗ | bundle field exists but is never populated |
| Bundle `self` link | ✓ | echoes executed params |
| `next` / `previous` links | ✓ | cursor-based |
| `first` / `last` links | ✗ | not generated |

**`_sort` detail.** `_sort` is parsed into `SearchQuery.sort` for every backend. The backends map
sort fields to columns via a small allow-list — `_id` → `id`, `_lastUpdated` → `last_updated` —
and fall back to `id` for anything else. So sorting by an arbitrary search parameter (e.g.
`_sort=birthdate`) currently degrades to a stable-but-not-meaningful `id` ordering on all backends.
Sort is applied on the first-page and offset query paths; cursor (keyset) pages always use the
default `_lastUpdated, id` ordering because the keyset `WHERE` comparison depends on it. MongoDB
additionally cannot combine a custom sort with cursor pagination.

## 7. Known limitations & roadmap

Ordered roughly by impact:

1. **Sort by arbitrary search parameter** — unsupported on all backends (only `_id`/`_lastUpdated`).
   Would require sorting on `search_index` values via a join. Cursor pages ignore custom sort.
2. **Terminology-dependent modifiers** — token `:above`/`:below`, `:in`, `:not-in` need a
   terminology server. `:in` is partially handled via REST-side expansion; the rest are not native.
   URI `:above`/`:below` (hierarchical prefix, no service needed) *is* implemented on SQLite/PG/ES.
3. **PostgreSQL modifier gaps** — only the `:text-advanced` modifier remains unimplemented relative
   to SQLite (`:exact`, `:contains`, `:not`, `:missing`, `:of-type`, URI `:above`/`:below`, and
   composite parameters are all supported now).
4. **MongoDB native search gaps** — quantity and composite parameters error out; forward/reverse
   chaining, `_text`/`_content`, and most modifiers beyond `:exact`/`:contains` are unsupported.
5. **Elasticsearch gaps** — composite matches the parameter name only (components not evaluated);
   forward chaining and `_has` silently return nothing rather than erroring; `_filter` unsupported.
6. **REST result params** — `_maxresults`, `_score`, `_query`, `_contained`/`_containedType`
   unsupported; Bundles omit `first`/`last` paging links. `:code-text` (newer spec modifier) is
   unsupported everywhere.

SQLite is the most complete backend and serves as the reference for the others; PostgreSQL is now
at near-parity (only `:text-advanced` remains).
