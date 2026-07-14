//! Storage backend implementations for the Helios Terminology Server.
//!
//! Each sub-module provides a concrete implementation of [`TerminologyBackend`]
//! gated by the corresponding feature flag.
//!
//! | Module     | Feature    | Type                          |
//! |------------|------------|-------------------------------|
//! | `sqlite`   | `sqlite`   | `SqliteTerminologyBackend`    |
//! | `postgres` | `postgres` | `PostgresTerminologyBackend`  |
//!
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteTerminologyBackend;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "postgres")]
pub use postgres::PostgresTerminologyBackend;

/// Precedence among `code_systems` rows sharing one canonical URL.
///
/// This is the single definition of "which row wins when the caller did not pin
/// a version". Both backends embed it verbatim, so SQLite and Postgres cannot
/// drift apart. It assumes the candidate table is named/aliased `code_systems`.
///
/// Tiers, in order:
///
/// 1. **`content`** — a usable definition beats a `fragment`/`example`, which
///    beats a `not-present` stub. Load-bearing: VSAC and the FHIR core packages
///    ship empty `not-present` stubs for SNOMED/LOINC whose `version` string
///    (`"current"`) text-sorts *above* a real edition (`"20260501"`), so this
///    tier is the only thing keeping `version=current` resolving to the real
///    import. See `docs/ig-publisher-compatibility.md`.
/// 2. **has concepts** — a populated row beats an empty one, for the same reason.
/// 3. **`authority_rank`** — the original beats a re-published copy. This is the
///    tier that fixes the `terminology.hl7.org` collisions: `hl7.fhir.r4.core`
///    re-ships 746 THO CodeSystems stamped with the *FHIR release* version
///    (`4.0.1`), which outsorts the code system's own version (`1.0.0`) under any
///    ordering — lexicographic or semver — even though the copy is truncated.
///    Provenance is the only thing that separates those rows; nothing intrinsic
///    to them does. `NULL` (a row imported before the column existed) coalesces
///    to 0, so an un-re-imported database keeps its previous behaviour.
/// 4. **`version`** — highest version string wins, as before.
/// 5. **`id`** — a stable, deterministic final tiebreak.
///
/// Tier 3 sits *below* 1 and 2 deliberately: if a package that owns a canonical
/// ships only an empty stub of it while some other package ships a populated
/// copy, the populated row should still win — being authoritative is worthless
/// if the row has no concepts to validate against.
///
/// `alias` is how the `code_systems` table is named in the enclosing query
/// (`"code_systems"` when unaliased, or e.g. `"s"` / `"cs"`).
///
/// Every query that has to choose one row for a canonical URL must order by
/// this. When they disagree, the server becomes internally incoherent — e.g.
/// `$validate-code` resolving against the authoritative row while `$lookup`
/// reads `resource_json` from the truncated copy. That incoherence is what
/// let this bug class persist.
pub(crate) fn cs_precedence_order_by(alias: &str) -> String {
    format!(
        "(CASE COALESCE({a}.content, 'complete') \
              WHEN 'complete'    THEN 0 \
              WHEN 'supplement'  THEN 0 \
              WHEN 'fragment'    THEN 1 \
              WHEN 'example'     THEN 1 \
              WHEN 'not-present' THEN 2 \
              ELSE 1 END), \
         (CASE WHEN EXISTS \
             (SELECT 1 FROM concepts hc WHERE hc.system_id = {a}.id) \
             THEN 0 ELSE 1 END), \
         COALESCE({a}.authority_rank, 0), \
         COALESCE({a}.version, '') DESC, \
         {a}.id",
        a = alias
    )
}

/// Precedence among `value_sets` rows sharing one canonical URL.
///
/// The ValueSet analogue of [`cs_precedence_order_by`]. `value_sets` has no
/// `content` column and no concept rows, so only the provenance and version
/// tiers apply: original beats re-published copy, then highest version, then a
/// stable `id`.
///
/// This matters for the same reason: `hl7.fhir.r4.core` re-ships 603 THO
/// ValueSets under canonical URLs it does not own.
pub(crate) fn vs_precedence_order_by(alias: &str) -> String {
    format!(
        "COALESCE({a}.authority_rank, 0), \
         COALESCE({a}.version, '') DESC, \
         {a}.id",
        a = alias
    )
}

/// True when `ver` is the sentinel `current` (case-insensitive).
///
/// IG Publisher, VSAC stubs, and CQL-generated Library `dataRequirement` entries
/// validate SNOMED with `version=current`. HTS resolves that to the best loaded
/// edition (complete row with concepts), not a literal version string match.
/// See `docs/ig-publisher-compatibility.md` and `fetch_versions` ordering in
/// `sqlite/code_system.rs` / `postgres/code_system.rs`.
pub(super) fn code_system_version_is_current(ver: &str) -> bool {
    ver.eq_ignore_ascii_case("current")
}
