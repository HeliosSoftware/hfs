# Search-modifier FHIR spec-compliance plan

**Branch:** `fix/search-modifier-spec-compliance`
**Goal:** Bring `hfs` search-modifier handling into compliance with the FHIR
search spec (https://build.fhir.org/search.html), across all search-capable
backends (SQLite, PostgreSQL, Elasticsearch, MongoDB).

This addresses the deviations documented in `docs/ui-requirements.md` §7.2.

## Background / key finding

`SearchModifier::is_valid_for(param_type)` in
`crates/persistence/src/types/search_params.rs` is **not enforced at runtime** —
it has no production call sites. Request handling is done by per-**type**
parameter handlers (`crates/persistence/src/backends/*/search/...`), and each
backend rejects unsupported combinations ad hoc. Values are indexed by the
extractor into typed columns (`value_string`, `value_token_*`, `value_reference`,
`value_uri`, …), so a modifier can only act on data the extractor actually
indexed. Some spec widenings therefore require extractor/schema changes, not
just handler tweaks.

## Authoritative target (FHIR build spec) — modifier → allowed param types

| Modifier | Allowed types (spec) | Current `hfs` | Gap |
|----------|----------------------|---------------|-----|
| `:missing` | all | all | none |
| `:exact` | string | string | none |
| `:contains` | string, reference, uri | string | + reference, uri |
| `:text` | string, token, reference | token (string on ES only) | + string (all), + reference |
| `:in` | token | token, uri | narrow to token |
| `:not-in` | token | token, uri (returns 501) | narrow to token; decide semantics |
| `:not` | token | "all" (impl token-only) | narrow table to token |
| `:above` / `:below` | reference, token, uri | token, uri | + reference |
| `:of-type` | token | token (✅ `of-type` + `ofType`) | done |
| `:code-text` | reference, token | unimplemented | implement token + reference |
| `:text-advanced` | reference, token | string, token | + reference; reconcile string |
| `:identifier` | reference | reference | none |
| `:[type]` | reference | reference | none |
| `:iterate` | `_include`/`_revinclude` | same | none |

## Phasing (each phase = its own commit(s), tests included)

Each phase keeps the validation gate and the actual implementation **consistent**
— a `(modifier, type)` combination is only marked valid once a backend honors it.

- **Phase 0** — Make `is_valid_for` spec-accurate for already-implemented combos
  (fix `:not` "all" → token-only overclaim; keep `:missing` = all). Wire it as a
  single 400 gate in the REST search pipeline; align CapabilityStatement
  per-type modifier advertisements. No new search behavior.
- **Phase 1** — `:contains` on uri + reference (handler-only; substring match).
- **Phase 2** — `:text` on string across all backends (currently ES-only).
- **Phase 3** — `:code-text` (token first; then reference) — currently a no-op.
- **Phase 4** — reference-typed text/hierarchy modifiers (`:text`/`:text-advanced`
  on reference need `Reference.display` indexed; `:above`/`:below` on reference
  need canonical/hierarchical handling). **Requires extractor + schema changes.**
- **Phase 5** — narrow `:in`/`:not-in` to token-only; decide `:not-in`
  (negated ValueSet expansion vs documented 501).

## Per-phase backend checklist

For each behavior change, touch: SQLite handler, Postgres handler, Elasticsearch
handler, MongoDB handler, the backend `capabilities()` modifier lists, the
`is_valid_for` arm, and tests (unit per handler + integration in
`crates/rest/tests/search_integration.rs` / `crates/persistence/tests/`).

## Status

- [x] `:of-type` spelling alias (`of-type` + `ofType`).
- [x] **Phase 0** — `:not` token-only; central 400 validation gate; capability
  audit (drop `:not-in` 501-advertisement) across all 4 backends.
- [x] **Phase 1** — `:contains` on uri + reference (all backends).
- [x] **Phase 2** — `:text` on string (all backends).
- [x] **Phase 3** — `:code-text` on token (all backends); also filled PG token `:text`.
- [x] **Phase 4** — `Reference.display` indexed (SQLite/PG v8→v9 migrations, ES
  mapping, Mongo); reference `:text`/`:code-text` (all backends).
- [x] **Phase 5** — `:in`/`:not-in` token-only; `:not-in` returns 501
  unconditionally.
- [x] **Postgres `:identifier`** — implemented (was advertised but broken).

- [x] **MongoDB `SearchCapabilityProvider`** — Mongo now advertises exactly the
  modifiers it honors.
- [x] **Reference `:above`/`:below`** — URL/path-prefix hierarchy (SQLite/PG/ES;
  Mongo honestly 400s).

### Remaining (deferred by design decision)

- [ ] Reference `:text-advanced` — needs FTS over `Reference.display` across
  backends; disproportionate for a doubly-niche modifier. `is_valid_for` keeps
  it strict, so the gate returns an honest 400 rather than a degraded result.
- [ ] Canonical `|version` comparison for reference `:above`/`:below` — current
  implementation is URL/path-prefix only.
- [ ] `:text-advanced` is implemented for token on SQLite (FTS5) and ES only;
  PG/Mongo don't advertise it.

Validation: SQLite (full REST + unit) and Postgres (Docker integration) suites
green on every commit; Elasticsearch and MongoDB compile-checked with logic
mirroring the validated backends (CI runs their Docker matrix).
