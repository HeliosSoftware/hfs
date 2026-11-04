//! Persisted terminology relations for SQL-on-FHIR SQL queries (#1824).
//!
//! When a SQLQuery or SQLView Library that depends on a `ValueSet` or
//! `ConceptMap` is written, the REST layer has the terminology server expand
//! the value set (or returns the map), flattens it into the spec's fixed-column
//! relation, and hands it to a [`SofTerminologyStore`]. The store keeps one
//! table per (tenant, canonical), shared by every Library that depends on that
//! canonical, and records which Library references which canonical. A table is
//! dropped when the last Library referencing it is deleted or rewritten
//! without the dependency.
//!
//! Every backend with a SQL-on-FHIR runner implements it (#2185): SQLite and
//! PostgreSQL in one transaction, MongoDB and S3 by compare-and-swap of a
//! per-tenant registry (see `backends::sof_terminology_cas`). Composites hand
//! out their primary's store. The contract is checked on every backend by
//! `tests/common/sof_terminology_suite.rs`.
//!
//! `$sql-run` and `$sql-export` read the stored rows back with
//! [`SofTerminologyStore::load`] and copy them into the request's in-memory SQL
//! engine under the dependency's label, so the terminology server stays off
//! the query path.

use async_trait::async_trait;
use helios_sof::sqlquery::TerminologyRelation;

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// Persists terminology relations per tenant and canonical, reference-counted
/// by the Libraries that depend on them.
#[async_trait]
pub trait SofTerminologyStore: Send + Sync {
    /// Makes `relations` the complete set of terminology dependencies of
    /// Library `library_id`, as `(canonical, relation)` pairs.
    ///
    /// Every relation passed is (re)written, replacing any table already held
    /// for that canonical. References the Library held to canonicals not in
    /// `relations` are released, and every table left with no referencing
    /// Library is dropped. Passing an empty list releases the Library
    /// entirely (its delete, or an update that removed its last terminology
    /// dependency). The whole change is atomic.
    async fn set_library_dependencies(
        &self,
        tenant: &TenantContext,
        library_id: &str,
        relations: Vec<(String, TerminologyRelation)>,
    ) -> StorageResult<()>;

    /// The stored relation for `canonical`, if any Library materialized one.
    async fn load(
        &self,
        tenant: &TenantContext,
        canonical: &str,
    ) -> StorageResult<Option<TerminologyRelation>>;
}
