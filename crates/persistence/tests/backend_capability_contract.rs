//! Contract tests for what each backend *advertises* via [`BackendCapability`].
//!
//! # Why this file exists
//!
//! Issue #369: the PostgreSQL backend advertised `SchemaPerTenant` and
//! `DatabasePerTenant` and implemented neither — it is shared-schema only, via a
//! `tenant_id` discriminator column. Nothing caught it, because nothing had ever
//! asserted what a backend declares. The `CapabilityMatrix` under `tests/common/`
//! looks like it fills that role but cannot: no test target declares
//! `mod common;`, so cargo never compiles that directory (see
//! `tests/backend_error_handling.rs:44`). Its fate is tracked in #306 / #361.
//!
//! # Why it is shaped this way
//!
//! Every assertion runs against a **constructor-free** `declared_capabilities()`
//! associated function rather than a live backend instance. That is not a
//! stylistic choice: `PostgresBackend::new` is async and eagerly verifies
//! connectivity, so an instance-based assertion for the backend this issue is
//! *about* could only run with a live PostgreSQL — i.e. never on an ordinary
//! pull request. Nothing here needs a database, Docker, MinIO, AWS credentials,
//! or the network.
//!
//! For the same reason there is no `supports(c) == capabilities().contains(&c)`
//! sweep: both delegate to one declaration per backend, so such a test would be
//! a tautology asserting something other than what it claims to guard.
//!
//! S3 is the exception that proves the semantics — its tenant-placement topology
//! depends on the configured `S3TenancyMode`, so it is asserted per mode here and
//! its *instance* composition is covered in `src/backends/s3/tests.rs` with a
//! mock client.

// Each backend's assertions are gated on its own feature; the shared helpers
// below would be dead code if none were enabled, so the whole file compiles
// away in that configuration.
#![cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch",
    feature = "s3"
))]

use helios_persistence::core::BackendCapability;

/// The mutually exclusive tenant-placement topologies.
///
/// A conforming backend instance declares **exactly one** of these — see the
/// doc comment on [`BackendCapability`].
const TENANCY_CAPABILITIES: [BackendCapability; 3] = [
    BackendCapability::SharedSchema,
    BackendCapability::SchemaPerTenant,
    BackendCapability::DatabasePerTenant,
];

/// Asserts a declaration equals its golden list exactly, naming the difference
/// in both directions on failure.
///
/// Compared as sets so list order is not load-bearing, but *exactly*, so that
/// adding or removing a claim requires deliberately editing the golden list in
/// this file — which is the point.
fn assert_declares_exactly(
    backend: &str,
    declared: &[BackendCapability],
    golden: &[BackendCapability],
) {
    let declared_set: std::collections::HashSet<_> = declared.iter().copied().collect();
    let golden_set: std::collections::HashSet<_> = golden.iter().copied().collect();

    assert_eq!(
        declared.len(),
        declared_set.len(),
        "{backend} declares a duplicate capability: {declared:?}"
    );

    let unexpected: Vec<_> = declared_set.difference(&golden_set).copied().collect();
    let missing: Vec<_> = golden_set.difference(&declared_set).copied().collect();

    assert!(
        unexpected.is_empty() && missing.is_empty(),
        "{backend} capability declaration drifted.\n  \
         unexpectedly declared: {unexpected:?}\n  \
         no longer declared:    {missing:?}\n\
         A capability must describe what this backend actually implements — never \
         planned or aspirational behavior. If this is a deliberate change, update \
         the golden list in tests/backend_capability_contract.rs in the same commit."
    );
}

/// The anti-drift property.
///
/// Re-adding a second tenancy claim to a backend cannot be made green by
/// *adding* to a list here — the only passing edit is to also remove the
/// existing one, which is a loud, reviewable semantic claim ("this backend no
/// longer does shared-schema") rather than a quiet addition.
fn assert_exactly_one_tenancy_claim(backend: &str, declared: &[BackendCapability]) {
    let claimed: Vec<_> = TENANCY_CAPABILITIES
        .iter()
        .copied()
        .filter(|c| declared.contains(c))
        .collect();

    assert_eq!(
        claimed.len(),
        1,
        "{backend} must declare exactly one tenant-placement topology, but declares {claimed:?}. \
         SharedSchema, SchemaPerTenant and DatabasePerTenant are mutually exclusive descriptions \
         of one deployment, not a menu."
    );
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use helios_persistence::backends::postgres::PostgresBackend;

    /// Regression test for issue #369.
    ///
    /// PostgreSQL is shared-schema only: one `resources` table keyed by
    /// `(tenant_id, resource_type, id)`, with no schema or database switching in
    /// any code path. It previously advertised `SchemaPerTenant` and
    /// `DatabasePerTenant` — an isolation guarantee that did not exist.
    #[test]
    fn postgres_declares_shared_schema_only() {
        let declared = PostgresBackend::declared_capabilities();

        assert!(
            !declared.contains(&BackendCapability::SchemaPerTenant),
            "PostgreSQL does not implement schema-per-tenant (no SET search_path / CREATE SCHEMA \
             exists in the backend); advertising it overstates tenant isolation. See #369."
        );
        assert!(
            !declared.contains(&BackendCapability::DatabasePerTenant),
            "PostgreSQL does not implement database-per-tenant (no CREATE DATABASE or per-tenant \
             pool exists in the backend); advertising it overstates tenant isolation. See #369."
        );
        assert!(declared.contains(&BackendCapability::SharedSchema));

        assert_exactly_one_tenancy_claim("postgres", &declared);
    }

    #[test]
    fn postgres_declares_exactly_its_golden_capabilities() {
        assert_declares_exactly(
            "postgres",
            &PostgresBackend::declared_capabilities(),
            &[
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
                BackendCapability::TypeHistory,
                BackendCapability::SystemHistory,
                BackendCapability::BasicSearch,
                BackendCapability::DateSearch,
                BackendCapability::ReferenceSearch,
                BackendCapability::FullTextSearch,
                BackendCapability::Sorting,
                BackendCapability::OffsetPagination,
                BackendCapability::CursorPagination,
                BackendCapability::Transactions,
                BackendCapability::OptimisticLocking,
                BackendCapability::PessimisticLocking,
                BackendCapability::BulkExport,
                BackendCapability::BulkSubmitIngest,
                BackendCapability::BulkSubmitRestWorker,
                BackendCapability::Include,
                BackendCapability::Revinclude,
                BackendCapability::SharedSchema,
            ],
        );
    }
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use super::*;
    use helios_persistence::backends::sqlite::SqliteBackend;

    #[test]
    fn sqlite_declares_shared_schema_only() {
        let declared = SqliteBackend::declared_capabilities();

        assert!(!declared.contains(&BackendCapability::SchemaPerTenant));
        assert!(
            !declared.contains(&BackendCapability::DatabasePerTenant),
            "SQLite has no per-tenant database file or ATTACH logic; it separates tenants by a \
             tenant_id discriminator column only."
        );
        assert_exactly_one_tenancy_claim("sqlite", &declared);
    }

    #[test]
    fn sqlite_declares_exactly_its_golden_capabilities() {
        assert_declares_exactly(
            "sqlite",
            &SqliteBackend::declared_capabilities(),
            &[
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
                BackendCapability::TypeHistory,
                BackendCapability::SystemHistory,
                BackendCapability::BasicSearch,
                BackendCapability::DateSearch,
                BackendCapability::ReferenceSearch,
                BackendCapability::Sorting,
                BackendCapability::OffsetPagination,
                BackendCapability::Transactions,
                BackendCapability::OptimisticLocking,
                BackendCapability::BulkExport,
                BackendCapability::BulkSubmitIngest,
                BackendCapability::BulkSubmitRestWorker,
                BackendCapability::Include,
                BackendCapability::Revinclude,
                BackendCapability::SharedSchema,
            ],
        );
    }
}

#[cfg(feature = "mongodb")]
mod mongodb {
    use super::*;
    use helios_persistence::backends::mongodb::MongoBackend;

    #[test]
    fn mongodb_declares_shared_schema_only() {
        let declared = MongoBackend::declared_capabilities();

        assert!(!declared.contains(&BackendCapability::SchemaPerTenant));
        assert!(!declared.contains(&BackendCapability::DatabasePerTenant));
        assert_exactly_one_tenancy_claim("mongodb", &declared);
    }

    #[test]
    fn mongodb_declares_exactly_its_golden_capabilities() {
        assert_declares_exactly(
            "mongodb",
            &MongoBackend::declared_capabilities(),
            &[
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
                BackendCapability::TypeHistory,
                BackendCapability::SystemHistory,
                BackendCapability::BasicSearch,
                BackendCapability::DateSearch,
                BackendCapability::ReferenceSearch,
                BackendCapability::Sorting,
                BackendCapability::OffsetPagination,
                BackendCapability::CursorPagination,
                BackendCapability::Transactions,
                BackendCapability::OptimisticLocking,
                BackendCapability::SharedSchema,
            ],
        );
    }
}

#[cfg(feature = "elasticsearch")]
mod elasticsearch {
    use super::*;
    use helios_persistence::backends::elasticsearch::ElasticsearchBackend;

    /// Elasticsearch names indices `{prefix}_{tenant}_{type}`, which looks like a
    /// per-tenant topology. It declares `SharedSchema` because every document
    /// also carries a `tenant_id` field that queries filter on, and the index
    /// split is a naming convention inside one cluster reached by one credential
    /// — a logical partition with no policy or storage boundary. Whether the enum
    /// should grow a topology that fits this case more precisely is a follow-up
    /// to #369; until then this asserts the weaker, safer claim.
    #[test]
    fn elasticsearch_declares_shared_schema_only() {
        let declared = ElasticsearchBackend::declared_capabilities();

        assert!(!declared.contains(&BackendCapability::SchemaPerTenant));
        assert!(!declared.contains(&BackendCapability::DatabasePerTenant));
        assert_exactly_one_tenancy_claim("elasticsearch", &declared);
    }

    #[test]
    fn elasticsearch_declares_exactly_its_golden_capabilities() {
        assert_declares_exactly(
            "elasticsearch",
            &ElasticsearchBackend::declared_capabilities(),
            &[
                BackendCapability::Crud,
                BackendCapability::BasicSearch,
                BackendCapability::DateSearch,
                BackendCapability::QuantitySearch,
                BackendCapability::ReferenceSearch,
                BackendCapability::FullTextSearch,
                BackendCapability::Sorting,
                BackendCapability::CursorPagination,
                BackendCapability::OffsetPagination,
                BackendCapability::Include,
                BackendCapability::Revinclude,
                BackendCapability::SharedSchema,
            ],
        );
    }
}

#[cfg(feature = "s3")]
mod s3 {
    use super::*;
    use helios_persistence::backends::s3::{S3Backend, S3TenancyMode};
    use std::collections::HashMap;

    fn prefix_mode() -> S3TenancyMode {
        S3TenancyMode::PrefixPerTenant {
            bucket: "shared".to_string(),
        }
    }

    fn bucket_mode(default_system_bucket: Option<&str>) -> S3TenancyMode {
        let mut tenant_bucket_map = HashMap::new();
        tenant_bucket_map.insert("tenant-a".to_string(), "bucket-a".to_string());
        tenant_bucket_map.insert("tenant-b".to_string(), "bucket-b".to_string());

        S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket: default_system_bucket.map(str::to_string),
        }
    }

    /// One shared bucket, tenants separated by key prefix — logical isolation.
    #[test]
    fn prefix_per_tenant_declares_shared_schema_only() {
        let declared = S3Backend::declared_capabilities_for(&prefix_mode());

        assert!(declared.contains(&BackendCapability::SharedSchema));
        assert!(
            !declared.contains(&BackendCapability::DatabasePerTenant),
            "PrefixPerTenant shares one bucket across tenants; claiming DatabasePerTenant would \
             overstate isolation. See #369."
        );
        assert_exactly_one_tenancy_claim("s3 (PrefixPerTenant)", &declared);
    }

    /// A dedicated bucket per tenant — genuine physical isolation, so the
    /// `DatabasePerTenant` claim is true and must be kept.
    #[test]
    fn bucket_per_tenant_declares_database_per_tenant_only() {
        for default_system_bucket in [None, Some("system")] {
            let mode = bucket_mode(default_system_bucket);
            let declared = S3Backend::declared_capabilities_for(&mode);

            assert!(declared.contains(&BackendCapability::DatabasePerTenant));
            assert!(
                !declared.contains(&BackendCapability::SharedSchema),
                "BucketPerTenant gives each tenant a dedicated bucket, so SharedSchema is false — \
                 including when a default_system_bucket exists, since cross-tenant storability is \
                 a different axis (supports_user_settings / supports_tenant_registry)."
            );
            assert_exactly_one_tenancy_claim("s3 (BucketPerTenant)", &declared);
        }
    }

    /// Only the tenancy claim may vary with configuration. Config refines *which*
    /// member of a mutually exclusive group is reported; it must never change
    /// anything else.
    #[test]
    fn only_the_tenancy_claim_varies_between_modes() {
        let strip_tenancy = |mode: &S3TenancyMode| -> Vec<BackendCapability> {
            S3Backend::declared_capabilities_for(mode)
                .into_iter()
                .filter(|c| !TENANCY_CAPABILITIES.contains(c))
                .collect()
        };

        assert_eq!(
            strip_tenancy(&prefix_mode()),
            strip_tenancy(&bucket_mode(Some("system"))),
            "S3 tenancy mode must affect only the tenant-placement claim"
        );
    }

    #[test]
    fn s3_declares_exactly_its_golden_capabilities() {
        assert_declares_exactly(
            "s3 (PrefixPerTenant)",
            &S3Backend::declared_capabilities_for(&prefix_mode()),
            &[
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
                BackendCapability::TypeHistory,
                BackendCapability::SystemHistory,
                BackendCapability::OptimisticLocking,
                BackendCapability::CursorPagination,
                BackendCapability::BulkExport,
                BackendCapability::BulkSubmitIngest,
                BackendCapability::SharedSchema,
            ],
        );
    }
}
