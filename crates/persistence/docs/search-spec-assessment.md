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
| Reverse chaining (`_has`) | ✓ | ✓ | ◐ | ◐ |
| `_include` | ✓ | ✓ | ✓ | ✓ |
| `_revinclude` | ✓ | ✓ | ✓ | ✓ |
| `:iterate` on include | parsed | parsed | parsed | parsed |

SQLite and PostgreSQL resolve chains natively, via nested `search_index` subqueries with
configurable depth limits (✓). For all other backends (◐), the REST layer resolves chained and
reverse-chained parameters before the backend search runs: `search::resolve_chains` issues one
plain `search()` per chain hop against the same backend and folds the result into an `_id`
filter — application-side joins. So chained and `_has` queries work end-to-end over HTTP on every
searchable backend, including Elasticsearch and MongoDB; the per-backend distinction is whether the
join is pushed into the backend (SQLite/PG) or performed by the REST layer.

## 6. Result control (paging, sort, total, summary, elements)

These are parsed and largely orchestrated by the REST layer.

| Parameter | Status | Notes |
|-----------|--------|-------|
| `_count` | ✓ | page size; `_offset`/`_cursor` for paging |
| `_sort` | ✓ | SQLite/PG sort by any indexed param; see below |
| `_total` | ✓ | `none` / `estimate` / `accurate` parsed and applied |
| `_summary` | ✓ | `true`/`text`/`data`/`count`/`false`, applied in `subsetting.rs` |
| `_elements` | ✓ | applied post-search with nested-path support |
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
   unsupported; Bundles omit `first`/`last` paging links. `:code-text` (newer spec modifier) is
   unsupported everywhere.

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
