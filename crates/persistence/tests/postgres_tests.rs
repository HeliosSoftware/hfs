//! PostgreSQL backend integration tests.
//!
//! These tests verify the PostgreSQL backend implementation.
//! Tests that require a running PostgreSQL instance use testcontainers
//! to spin up real PostgreSQL instances in Docker.
//!
//! Run with: `cargo test -p helios-persistence --features postgres -- postgres`

#![cfg(feature = "postgres")]

use helios_persistence::backends::postgres::PostgresConfig;
use helios_persistence::core::BackendKind;

// T2 cluster-harness helpers shared by the cluster suites. Included by
// `#[path]` rather than via `mod common;` so this test binary compiles only
// the harness it uses.
#[path = "common/cluster_harness.rs"]
mod cluster_harness;

/// The backend-agnostic `ifMatch` scenarios (issue #311), shared verbatim with
/// the SQLite suite that owns the file.
///
/// Declared at the top level rather than inside `mod postgres_integration`
/// because `#[path]` on a module nested in an *inline* module resolves relative
/// to `tests/postgres_tests/postgres_integration/`, which does not exist. At the
/// crate root it resolves relative to `tests/`, which does.
#[path = "transactions/if_match_suite.rs"]
mod if_match_suite;

// ============================================================================
// Backend Configuration Tests (no PostgreSQL instance required)
// ============================================================================

#[test]
fn test_postgres_config_defaults() {
    let config = PostgresConfig::default();
    assert_eq!(config.host, "localhost");
    assert_eq!(config.port, 5432);
    assert_eq!(config.dbname, "helios");
    assert_eq!(config.user, "helios");
    assert!(config.password.is_none());
    // Derived from the core count (cores * 4, clamped) rather than a fixed 10, which
    // throttled search badly under concurrent load — see #224. Assert the contract
    // (the clamp bounds), not the machine-dependent value.
    assert!(
        (16..=64).contains(&config.max_connections),
        "pool size {} outside the 16..=64 clamp",
        config.max_connections
    );
    assert_eq!(config.connect_timeout_secs, 5);
    assert_eq!(config.statement_timeout_ms, 30000);
    assert!(!config.search_offloaded);
}

#[test]
fn test_postgres_config_serialization() {
    let config = PostgresConfig {
        host: "pg-server".to_string(),
        port: 5433,
        dbname: "test_db".to_string(),
        user: "test_user".to_string(),
        password: Some("secret".to_string()),
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: PostgresConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.host, "pg-server");
    assert_eq!(deserialized.port, 5433);
    assert_eq!(deserialized.dbname, "test_db");
    assert_eq!(deserialized.user, "test_user");
    assert_eq!(deserialized.password, Some("secret".to_string()));
}

// ============================================================================
// Backend Capability Tests (no PostgreSQL instance required)
// ============================================================================

// NOTE: capability *declarations* are asserted in
// `tests/backend_capability_contract.rs`, against the constructor-free
// `PostgresBackend::declared_capabilities()`. They live there rather than here
// because a `PostgresBackend` cannot be constructed without a real database
// (the constructor connects immediately), and because the assertions are
// cross-backend.
//
// A `test_postgres_expected_capabilities` used to live here. It listed the
// capabilities by hand and then asserted `!expected.is_empty()` — which passes
// for any non-empty list, so it verified nothing while reading like a contract.
// Worse, its hand list was a third copy of the false `SchemaPerTenant` /
// `DatabasePerTenant` claim corrected in #369. Deleted rather than repaired.

#[test]
fn test_postgres_config_backend_kind() {
    // Verify BackendKind::Postgres exists and is usable
    let kind = BackendKind::Postgres;
    assert_eq!(format!("{}", kind), "postgres");
}

// ============================================================================
// Query Builder Unit Tests (no PostgreSQL instance required)
// ============================================================================

mod query_builder_tests {
    use helios_persistence::backends::postgres::search::query_builder::{
        PostgresQueryBuilder, SqlParam,
    };
    use helios_persistence::types::{
        SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
    };

    #[test]
    fn test_empty_query_returns_none() {
        let query = SearchQuery::new("Patient");
        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_none());
    }

    #[test]
    fn test_id_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("id = $"));
        assert_eq!(fragment.params.len(), 1);
        match &fragment.params[0] {
            SqlParam::Text(s) => assert_eq!(s, "123"),
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_string_parameter_default() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Default string search is starts-with. `LIKE`, not `ILIKE`: both the stored
        // column and the bound pattern are already case-folded by `fold_text`, and
        // `ILIKE` cannot use a btree index (#224). The raw-column fallback is wrapped
        // in `lower()` so un-backfilled rows keep matching case-insensitively.
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE"),
            "string search must target the indexed folded expression: {}",
            fragment.sql
        );
        assert!(!fragment.sql.contains("ILIKE"));
        assert!(fragment.sql.contains("param_name = 'name'"));
        // Parameter should be "Smith%"
        match &fragment.params[0] {
            SqlParam::Text(s) => assert!(s.ends_with('%')),
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_string_parameter_exact() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Exact),
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Exact match should use = not ILIKE
        assert!(fragment.sql.contains("value_string = $"));
    }

    #[test]
    fn test_string_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("mit")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE")
        );
        assert!(!fragment.sql.contains("ILIKE"));
        // Parameter should be "%mit%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_token_parameter_code_text() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::CodeText),
            values: vec![SearchValue::eq("Heart")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_display ILIKE"));
        // starts-with: param is "Heart%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(!s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_token_parameter_text_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("Heart")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_display ILIKE"));
        // contains: param is "%Heart%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_string_parameter_text() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("mit")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Accent-folded substring match against the indexed folded expression.
        assert!(fragment.sql.contains("value_string_folded"));
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE")
        );
        assert!(!fragment.sql.contains("ILIKE"));
        // Substring match: param wrapped as %mit%
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_reference_parameter_below_above() {
        use helios_persistence::types::SearchModifier;

        for modifier in [SearchModifier::Below, SearchModifier::Above] {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: Some(modifier),
                values: vec![SearchValue::eq("http://x.org/Questionnaire/q")],
                chain: vec![],
                components: vec![],
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
            assert!(fragment.sql.contains("value_reference"));
            assert!(fragment.sql.contains("|| '/%'"));
        }
    }

    #[test]
    fn test_reference_parameter_identifier() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Identifier),
            values: vec![SearchValue::eq("http://hospital.org|12345")],
            chain: vec![],
            components: vec![],
        });

        let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
        // Correlated subquery against the target's 'identifier' index rows.
        assert!(fragment.sql.contains("param_name = 'identifier'"));
        assert!(fragment.sql.contains("idx.value_token_system"));
        assert!(fragment.sql.contains("idx.value_token_code"));
    }

    #[test]
    fn test_uri_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("example.org")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_uri ILIKE"));
    }

    #[test]
    fn test_reference_parameter_text_and_code_text() {
        use helios_persistence::types::SearchModifier;

        for (modifier, expect_leading_pct) in [
            (SearchModifier::Text, true),
            (SearchModifier::CodeText, false),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: Some(modifier),
                values: vec![SearchValue::eq("John")],
                chain: vec![],
                components: vec![],
            });

            let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
            assert!(fragment.sql.contains("value_reference_display ILIKE"));
            // :text wraps as %John%; :code-text is starts-with John%
            assert_eq!(
                fragment.sql.contains("'%' || $3 || '%'"),
                expect_leading_pct
            );
        }
    }

    #[test]
    fn test_reference_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("patient-1")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_reference ILIKE"));
    }

    #[test]
    fn test_token_system_and_code() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|8867-4")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_system"));
        assert!(fragment.sql.contains("value_token_code"));
        assert_eq!(fragment.params.len(), 2);
    }

    #[test]
    fn test_token_code_only() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_code"));
        assert_eq!(fragment.params.len(), 1);
    }

    #[test]
    fn test_token_system_only() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_system"));
        assert!(!fragment.sql.contains("value_token_code"));
        assert_eq!(fragment.params.len(), 1);
    }

    #[test]
    fn test_date_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "2000-01-01")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_date"));
        // gt now matches strictly after the day → value_date >= (next day).
        assert!(fragment.sql.contains(">= $"));
    }

    #[test]
    fn test_number_parameter() {
        let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
            name: "probability".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "0.5")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_number"));
        assert!(fragment.sql.contains(">= $"));
        // ge matches from the low boundary of the implicit range: "0.5" has
        // precision 0.1, so the bound is 0.5 - 0.05 = 0.45.
        match &fragment.params[0] {
            SqlParam::Float(f) => assert!((f - 0.45).abs() < 1e-9),
            _ => panic!("Expected Float param"),
        }
    }

    #[test]
    fn test_quantity_parameter() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::eq("5.4||mg")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_quantity_value"));
        assert!(fragment.sql.contains("value_quantity_unit"));
    }

    #[test]
    fn test_reference_parameter() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_reference"));
    }

    #[test]
    fn test_uri_parameter() {
        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: None,
            values: vec![SearchValue::eq("http://example.org/fhir/ValueSet/123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_uri"));
    }

    #[test]
    fn test_uri_below_modifier() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org/fhir")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("LIKE"));
    }

    #[test]
    fn test_last_updated_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "2024-01-01")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("last_updated"));
        assert!(fragment.sql.contains(">= $"));
    }

    #[test]
    fn test_multiple_values_or() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123"), SearchValue::eq("456")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Multiple _id values should be OR'd
        assert!(fragment.sql.contains("OR"));
        assert_eq!(fragment.params.len(), 2);
    }

    #[test]
    fn test_multiple_parameters_and() {
        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq("Smith")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("male")],
                chain: vec![],
                components: vec![],
            });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Different parameters should be AND'd
        assert!(fragment.sql.contains("AND"));
    }

    #[test]
    fn test_prefix_operators() {
        // Test all prefix-to-operator mappings by using date search
        let prefixes_and_ops = vec![
            (SearchPrefix::Eq, "="),
            (SearchPrefix::Ne, "!="),
            (SearchPrefix::Gt, ">"),
            (SearchPrefix::Lt, "<"),
            (SearchPrefix::Ge, ">="),
            (SearchPrefix::Le, "<="),
        ];

        for (prefix, expected_op) in prefixes_and_ops {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(prefix, "2024-01-01")],
                chain: vec![],
                components: vec![],
            });

            let result = PostgresQueryBuilder::build_search_query(&query, 0);
            assert!(result.is_some(), "Failed for prefix {:?}", prefix);
            let fragment = result.unwrap();
            assert!(
                fragment
                    .sql
                    .contains(&format!("last_updated {} $", expected_op)),
                "Expected operator '{}' for prefix {:?}, got SQL: {}",
                expected_op,
                prefix,
                fragment.sql
            );
        }
    }
}

// ============================================================================
// Integration Tests (requires Docker for testcontainers)
// ============================================================================

/// Integration tests that require a real PostgreSQL instance via testcontainers.
///
/// These tests are behind `#[cfg(feature = "postgres")]` and require Docker.
/// They mirror the patterns in sqlite_tests.rs.
///
/// Run with:
///   cargo test -p helios-persistence --features postgres -- postgres_integration
///
/// Skip if no Docker:
///   cargo test -p helios-persistence --features postgres -- --skip postgres_integration
#[cfg(test)]
mod postgres_integration {
    use std::path::PathBuf;

    use helios_fhir::FhirVersion;
    use serde_json::json;

    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::SettingsStore;
    use helios_persistence::core::history::{HistoryParams, InstanceHistoryProvider};
    use helios_persistence::core::{Backend, BackendCapability, BackendKind, ResourceStorage};
    use helios_persistence::error::{ConcurrencyError, ResourceError, StorageError};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::{Mutex, OnceCell};

    /// Shared PostgreSQL container reused across all tests in this module.
    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary. NOTE: a `static` is
        /// never dropped, so `Drop for ContainerAsync` — testcontainers' only
        /// container-removal path — never runs. The container outlives the test
        /// process and is reaped in CI by its `github.run_id` label.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();
    static BULK_EXPORT_TEST_LOCK: Mutex<()> = Mutex::const_new(());
    // Serializes the cluster-jobs T2 suite: `claim_next` is deliberately
    // cross-tenant, so parallel tests sharing the container would claim (and
    // finish) each other's queued jobs.
    static CLUSTER_JOBS_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = Postgres::default()
                    .with_label("github.run_id", &run_id)
                    .start()
                    .await
                    .expect("Failed to start PostgreSQL container");

                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("Failed to get host port");

                let host = container
                    .get_host()
                    .await
                    .expect("Failed to get host")
                    .to_string();

                // Initialize schema once on the shared container.
                let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("data"))
                    .unwrap_or_else(|| PathBuf::from("data"));

                let config = PostgresConfig {
                    host: host.clone(),
                    port,
                    dbname: "postgres".to_string(),
                    user: "postgres".to_string(),
                    password: Some("postgres".to_string()),
                    max_connections: 5,
                    data_dir: Some(data_dir),
                    ..Default::default()
                };

                let backend = PostgresBackend::new(config)
                    .await
                    .expect("Failed to create PostgresBackend");

                backend
                    .init_schema()
                    .await
                    .expect("Failed to initialize schema");

                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    /// Creates a PostgresBackend connected to the shared testcontainers PostgreSQL instance.
    ///
    /// Schema is initialized once when the shared container starts; `init_schema()` is
    /// idempotent (uses CREATE TABLE IF NOT EXISTS).
    async fn create_backend() -> PostgresBackend {
        let pg = shared_pg().await;

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        };

        PostgresBackend::new(config)
            .await
            .expect("Failed to create PostgresBackend")
    }

    /// Creates a tenant with a unique ID suffix to isolate tests sharing the same database.
    fn create_tenant(id: &str) -> TenantContext {
        let unique_id = format!("{}_{}", id, uuid::Uuid::new_v4().simple());
        TenantContext::new(TenantId::new(&unique_id), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn statement_timeout_applies_to_every_pooled_connection() {
        let pg = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        const POOL_SIZE: usize = 10;
        const TIMEOUT_MS: u64 = 250;

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: POOL_SIZE,
            statement_timeout_ms: TIMEOUT_MS,
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend =
            std::sync::Arc::new(PostgresBackend::new(config).await.expect("create backend"));

        // Hold POOL_SIZE clients across a barrier so the pool is forced to open
        // every physical connection before any is released. deadpool creates
        // connections lazily, so a serial check could pass while exercising only
        // one connection. The regression this guards (#285): the pre-fix code ran
        // `SET statement_timeout` on the single connection borrowed inside
        // `PostgresBackend::new`, so every connection created lazily afterwards
        // inherited the server default (usually 0 = uncapped). Shipping the GUC
        // in the connection startup packet makes every connection carry it, which
        // is what each task asserts below.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(POOL_SIZE));
        let mut handles = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            let backend = backend.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                let client = backend.get_client().await.expect("get_client");
                barrier.wait().await;
                let row = client
                    .query_one("SELECT current_setting('statement_timeout')", &[])
                    .await
                    .expect("current_setting");
                let value: String = row.get(0);
                value
            }));
        }

        for (i, h) in handles.into_iter().enumerate() {
            let value = h.await.expect("task panicked");
            assert_eq!(
                value,
                format!("{TIMEOUT_MS}ms"),
                "pooled connection #{i} reported statement_timeout={value:?}, \
                 expected {TIMEOUT_MS}ms — the GUC did not reach every connection"
            );
        }
    }

    // ========================================================================
    // CRUD Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_create_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        let result = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await;
        assert!(result.is_ok(), "Create failed: {:?}", result.err());

        let created = result.unwrap();
        assert_eq!(created.resource_type(), "Patient");
        assert!(!created.id().is_empty());
        assert_eq!(created.version_id(), "1");
    }

    #[tokio::test]
    async fn postgres_integration_create_with_id() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "patient-123",
            "name": [{"family": "Jones"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(created.id(), "patient-123");
    }

    #[tokio::test]
    async fn postgres_integration_create_duplicate_fails() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "duplicate-id"
        });

        backend
            .create(&tenant, "Patient", patient.clone(), FhirVersion::default())
            .await
            .unwrap();

        let result = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn postgres_integration_read_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "ReadTest"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert!(read.is_some());

        let resource = read.unwrap();
        assert_eq!(resource.id(), created.id());
        assert_eq!(resource.content()["name"][0]["family"], "ReadTest");
    }

    #[tokio::test]
    async fn postgres_integration_read_nonexistent() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let read = backend
            .read(&tenant, "Patient", "does-not-exist")
            .await
            .unwrap();
        assert!(read.is_none());
    }

    #[tokio::test]
    async fn postgres_integration_exists() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        assert!(
            backend
                .exists(&tenant, "Patient", created.id())
                .await
                .unwrap()
        );
        assert!(
            !backend
                .exists(&tenant, "Patient", "nonexistent")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn postgres_integration_update_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Original"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let updated_content = json!({
            "resourceType": "Patient",
            "name": [{"family": "Updated"}]
        });

        let updated = backend
            .update(&tenant, &created, updated_content)
            .await
            .unwrap();

        assert_eq!(updated.version_id(), "2");
        assert_eq!(updated.content()["name"][0]["family"], "Updated");
    }

    #[tokio::test]
    async fn postgres_integration_create_or_update() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Create via upsert
        let patient = json!({"resourceType": "Patient", "name": [{"family": "First"}]});
        let (resource, was_created) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(was_created);
        assert_eq!(resource.id(), "upsert-id");

        // Update via upsert
        let patient2 = json!({"resourceType": "Patient", "name": [{"family": "Second"}]});
        let (resource2, was_created2) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient2,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(!was_created2);
        assert_eq!(resource2.content()["name"][0]["family"], "Second");
    }

    /// A `PUT` onto a deleted id restores the resource instead of failing.
    ///
    /// FHIR permits a deleted resource to be brought back by a subsequent
    /// update (http.html#delete). The restore continues the existing version
    /// chain — v1 create, v2 delete, v3 restore — rather than resetting to
    /// "1", and the resource is readable and searchable again afterwards.
    /// This mirrors `crud::delete_tests::test_delete_is_soft_delete`, which
    /// covers the same path on SQLite.
    #[tokio::test]
    async fn postgres_integration_create_or_update_restores_deleted() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Original"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();
        assert_eq!(created.version_id(), "1");

        backend.delete(&tenant, "Patient", &id).await.unwrap();

        let (restored, _created_new) = backend
            .create_or_update(
                &tenant,
                "Patient",
                &id,
                json!({"resourceType": "Patient", "name": [{"family": "Restored"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert_eq!(restored.content()["name"][0]["family"], "Restored");
        assert_eq!(
            restored.version_id(),
            "3",
            "restore should continue the version chain (v1 create, v2 delete, v3 restore)"
        );
        assert!(!restored.is_deleted());

        // The resource is live again: readable, and the restore is the current
        // version.
        let read = backend
            .read(&tenant, "Patient", &id)
            .await
            .unwrap()
            .expect("restored resource must be readable");
        assert_eq!(read.version_id(), "3");
        assert_eq!(read.content()["name"][0]["family"], "Restored");
        assert!(backend.exists(&tenant, "Patient", &id).await.unwrap());

        // History keeps every version, including the deletion — which is only
        // returned when `include_deleted` is set (deleted versions are filtered out
        // by default on every backend).
        let history = backend
            .history_instance(
                &tenant,
                "Patient",
                &id,
                &HistoryParams::new().include_deleted(true),
            )
            .await
            .unwrap();
        assert_eq!(
            history.items.len(),
            3,
            "history should hold create, delete and restore"
        );
        assert_eq!(history.items[0].resource.version_id(), "3");
        assert!(!history.items[0].resource.is_deleted());
        assert!(
            history.items[1].resource.is_deleted(),
            "the middle version is the deletion"
        );
    }

    /// Restoring a deleted resource requires update permission.
    #[tokio::test]
    async fn postgres_integration_restore_deleted_requires_permission() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();
        backend.delete(&tenant, "Patient", &id).await.unwrap();

        // Same tenant, read-only permissions.
        let read_only =
            TenantContext::new(tenant.tenant_id().clone(), TenantPermissions::read_only());
        let result = backend
            .create_or_update(
                &read_only,
                "Patient",
                &id,
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await;
        assert!(
            matches!(&result, Err(StorageError::Tenant(_))),
            "restore without update permission must be refused, got {:?}",
            result.as_ref().map(|(r, _)| r.version_id())
        );
    }

    /// A restored SearchParameter re-enters the tenant's registry overlay.
    ///
    /// Deleting a custom SearchParameter unregisters it; bringing it back with
    /// a PUT has to reload the stored-parameter cache the way a create does,
    /// or the parameter stays invisible to search until the process restarts.
    #[tokio::test]
    async fn postgres_integration_restored_search_parameter_reenters_registry() {
        use helios_persistence::core::SearchProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let search_param = json!({
            "resourceType": "SearchParameter",
            "id": "pg-restore-sp",
            "url": "http://example.org/fhir/SearchParameter/pg-restore-sp",
            "name": "pgrestoresp",
            "status": "active",
            "code": "pgrestoresp",
            "base": ["Observation"],
            "type": "token",
            "expression": "Observation.code"
        });

        backend
            .create_or_update(
                &tenant,
                "SearchParameter",
                "pg-restore-sp",
                search_param.clone(),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        {
            let reg = backend.search_param_registry(&tenant);
            let registry = reg.read();
            assert!(registry.get_param("Observation", "pgrestoresp").is_some());
        }

        backend
            .delete(&tenant, "SearchParameter", "pg-restore-sp")
            .await
            .unwrap();
        {
            let reg = backend.search_param_registry(&tenant);
            let registry = reg.read();
            assert!(
                registry.get_param("Observation", "pgrestoresp").is_none(),
                "deleted SearchParameter should be unregistered"
            );
        }

        backend
            .create_or_update(
                &tenant,
                "SearchParameter",
                "pg-restore-sp",
                search_param,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let reg = backend.search_param_registry(&tenant);
        let registry = reg.read();
        assert!(
            registry.get_param("Observation", "pgrestoresp").is_some(),
            "restored SearchParameter should be registered again"
        );
    }

    #[tokio::test]
    async fn postgres_integration_delete_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();

        let read_result = backend.read(&tenant, "Patient", created.id()).await;
        match read_result {
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
            Ok(None) => {}
            other => {
                panic!("Expected Gone error or None, got: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn postgres_integration_delete_nonexistent_fails() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let result = backend.delete(&tenant, "Patient", "nonexistent").await;
        assert!(result.is_err());
    }

    // ========================================================================
    // Tenant Isolation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_isolation() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant A can see it
        assert!(
            backend
                .exists(&tenant_a, "Patient", created.id())
                .await
                .unwrap()
        );

        // Tenant B cannot see it
        assert!(
            !backend
                .exists(&tenant_b, "Patient", created.id())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn postgres_integration_same_id_different_tenants() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient_a = json!({"resourceType": "Patient", "name": [{"family": "A"}]});
        let patient_b = json!({"resourceType": "Patient", "name": [{"family": "B"}]});

        backend
            .create_or_update(
                &tenant_a,
                "Patient",
                "shared-id",
                patient_a,
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create_or_update(
                &tenant_b,
                "Patient",
                "shared-id",
                patient_b,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let read_a = backend
            .read(&tenant_a, "Patient", "shared-id")
            .await
            .unwrap()
            .unwrap();
        let read_b = backend
            .read(&tenant_b, "Patient", "shared-id")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read_a.content()["name"][0]["family"], "A");
        assert_eq!(read_b.content()["name"][0]["family"], "B");
    }

    // ========================================================================
    // Version Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_version_increments() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let v1 = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(v1.version_id(), "1");

        let v2 = backend
            .update(&tenant, &v1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        assert_eq!(v2.version_id(), "2");

        let v3 = backend
            .update(&tenant, &v2, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        assert_eq!(v3.version_id(), "3");
    }

    // ========================================================================
    // Count Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_count_resources() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 0..5 {
            let patient = json!({"resourceType": "Patient", "id": format!("p{}", i)});
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let count = backend.count(&tenant, Some("Patient")).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn postgres_integration_count_by_tenant() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        for _ in 0..3 {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create(&tenant_a, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        for _ in 0..2 {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create(&tenant_b, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 3);
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 2);
    }

    // ========================================================================
    // Console Dashboard count_* Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_count_by_types() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-by-types");

        // Seed a small deterministic dataset: 2 Patients, 1 Observation.
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let counts = backend
            .count_by_types(&tenant, &["Patient", "Observation", "Encounter"])
            .await
            .unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
        // A type with zero rows is ABSENT from the result, not a 0 row.
        assert!(!map.contains_key("Encounter"));
    }

    #[tokio::test]
    async fn postgres_integration_count_all_types() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-all-types");

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let counts = backend.count_all_types(&tenant).await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
    }

    #[tokio::test]
    async fn postgres_integration_count_by_day() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-by-day");

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // `since` = start of today (UTC midnight), built the same way the handler
        // does; `today` is derived from the same clock so this stays date-robust.
        let since = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let today = chrono::Utc::now().date_naive();

        let rows = backend
            .count_by_day(&tenant, "Patient", since)
            .await
            .unwrap();
        let today_row = rows
            .iter()
            .find(|r| r.day == today)
            .expect("today bucket should be present");
        assert_eq!(today_row.count, 2);
    }

    /// The history-backed delta rule on real Postgres: create `+1`, update `0`,
    /// delete `-1`, on epoch-aligned buckets. Mirrors the SQLite unit test, so the
    /// two backends are held to the same bucketing contract.
    #[tokio::test]
    async fn postgres_integration_count_deltas_by_bucket() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-deltas");

        let first = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .update(&tenant, &first, json!({"active": true}))
            .await
            .unwrap();

        let since = chrono::Utc::now() - chrono::Duration::minutes(5);
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();

        assert_eq!(
            rows.iter().map(|r| r.delta).sum::<i64>(),
            2,
            "two creates and one update net to +2"
        );
        assert!(
            rows.iter().all(|r| r.bucket_start.timestamp() % 60 == 0),
            "buckets are epoch-aligned to their width"
        );

        backend
            .delete(&tenant, "Patient", first.id())
            .await
            .unwrap();
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();
        assert_eq!(rows.iter().map(|r| r.delta).sum::<i64>(), 1);
    }

    #[tokio::test]
    async fn postgres_integration_activity_histogram() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-activity-histogram");

        // 3 writes for this tenant -> 3 resource_history rows.
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let since = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let cells = backend.activity_histogram(&tenant, since).await.unwrap();
        assert!(!cells.is_empty());
        // Total across returned cells equals the number of writes seeded.
        let total: u64 = cells.iter().map(|c| c.count).sum();
        assert_eq!(total, 3);
    }

    #[tokio::test]
    async fn postgres_integration_console_count_by_tenant() {
        let backend = create_backend().await;
        // Unique tenant IDs isolate this cross-tenant aggregate from every other
        // test sharing the same PostgreSQL container.
        let tenant_a = create_tenant("console-count-by-tenant-a");
        let tenant_b = create_tenant("console-count-by-tenant-b");

        // tenant-a: 3 resources, tenant-b: 2 resources.
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Cross-tenant admin aggregate: takes NO TenantContext. Look up by the
        // actual (uuid-suffixed) tenant IDs so other tests' rows never interfere.
        let counts = backend.count_by_tenant().await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get(tenant_a.tenant_id().as_str()), Some(&3));
        assert_eq!(map.get(tenant_b.tenant_id().as_str()), Some(&2));
    }

    #[tokio::test]
    async fn postgres_integration_is_cluster_shared() {
        let backend = create_backend().await;
        assert!(backend.is_cluster_shared());
    }

    // ========================================================================
    // Tenant Registry Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_registry_crud() {
        let backend = create_backend().await;
        assert!(backend.supports_tenant_registry());

        // Unique ids isolate this test from others sharing the database. The
        // shared uuid base plus "-a"/"-b" suffixes make the id ASC tie-break
        // deterministic when both rows land in the same created_at second.
        let base = uuid::Uuid::new_v4().simple().to_string();
        let id_a = format!("registry-{}-a", base);
        let id_b = format!("registry-{}-b", base);

        assert!(backend.get_tenant(&id_a).await.unwrap().is_none());

        // Register two tenants, one with a display name.
        let acme = backend
            .register_tenant(&id_a, Some("Acme Corp"))
            .await
            .unwrap();
        assert_eq!(acme.id, id_a);
        assert_eq!(acme.display_name.as_deref(), Some("Acme Corp"));
        assert!(!acme.created_at.is_empty());

        let beta = backend.register_tenant(&id_b, None).await.unwrap();
        assert_eq!(beta.id, id_b);
        assert_eq!(beta.display_name, None);

        // get_tenant round-trips the registered records.
        assert_eq!(backend.get_tenant(&id_a).await.unwrap(), Some(acme));
        assert_eq!(backend.get_tenant(&id_b).await.unwrap(), Some(beta));

        // The database is shared across tests, so only assert on our own rows:
        // both are present, ordered a before b (created_at ASC, id ASC).
        let all = backend.list_tenants().await.unwrap();
        let pos_a = all.iter().position(|t| t.id == id_a);
        let pos_b = all.iter().position(|t| t.id == id_b);
        assert!(pos_a.is_some(), "registered tenant {} not listed", id_a);
        assert!(pos_b.is_some(), "registered tenant {} not listed", id_b);
        assert!(pos_a < pos_b, "expected {} to sort before {}", id_a, id_b);

        // Duplicate registration is an error (handler pre-checks for 409).
        assert!(backend.register_tenant(&id_a, None).await.is_err());

        // Deregister removes the row; repeat and unknown ids report nothing
        // removed.
        assert!(backend.deregister_tenant(&id_a).await.unwrap());
        assert!(backend.get_tenant(&id_a).await.unwrap().is_none());
        assert!(!backend.deregister_tenant(&id_a).await.unwrap());
        assert!(
            !backend
                .deregister_tenant(&format!("never-registered-{}", base))
                .await
                .unwrap()
        );

        let remaining = backend.list_tenants().await.unwrap();
        assert!(!remaining.iter().any(|t| t.id == id_a));
        assert!(remaining.iter().any(|t| t.id == id_b));

        backend.deregister_tenant(&id_b).await.unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_purge_tenant_data() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("purge-tenant-a");
        let tenant_b = create_tenant("purge-tenant-b");

        let mut a_ids = Vec::new();
        for i in 0..3 {
            let patient = json!({
                "resourceType": "Patient",
                "name": [{"family": format!("Purge{}", i)}]
            });
            let created = backend
                .create(&tenant_a, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
            a_ids.push(created.id().to_string());
        }
        let b_created = backend
            .create(
                &tenant_b,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Kept"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Purge removes tenant-a's data only, reporting its current-row count.
        let removed = backend
            .purge_tenant_data(tenant_a.tenant_id().as_str())
            .await
            .unwrap();
        assert_eq!(removed, 3);

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 0);
        for id in &a_ids {
            assert!(
                backend
                    .read(&tenant_a, "Patient", id)
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        // tenant-b's data is intact.
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 1);
        assert!(
            backend
                .read(&tenant_b, "Patient", b_created.id())
                .await
                .unwrap()
                .is_some()
        );
    }

    // ========================================================================
    // Batch Read Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_read_batch() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let ids: Vec<String> = (0..3).map(|i| format!("batch-{}", i)).collect();
        for id in &ids {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create_or_update(&tenant, "Patient", id, patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let batch = backend
            .read_batch(&tenant, "Patient", &id_refs)
            .await
            .unwrap();

        assert_eq!(batch.len(), 3);
    }

    // ========================================================================
    // History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_instance_history() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "name": [{"family": "V1"}]});
        let v1 = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let v2 = backend
            .update(
                &tenant,
                &v1,
                json!({"resourceType": "Patient", "name": [{"family": "V2"}]}),
            )
            .await
            .unwrap();

        let _v3 = backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "name": [{"family": "V3"}]}),
            )
            .await
            .unwrap();

        let history = backend
            .history_instance(&tenant, "Patient", v1.id(), &HistoryParams::default())
            .await
            .unwrap();

        assert!(
            history.items.len() >= 3,
            "Expected at least 3 history entries, got {}",
            history.items.len()
        );
    }

    // ========================================================================
    // Content Preservation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_content_preserved() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Smith", "given": ["John", "Jacob"]}],
            "birthDate": "1990-01-15",
            "gender": "male",
            "active": true,
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "MRN-001"
            }],
            "address": [{
                "city": "Springfield",
                "state": "IL"
            }]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read.content()["name"][0]["family"], "Smith");
        assert_eq!(read.content()["name"][0]["given"][0], "John");
        assert_eq!(read.content()["name"][0]["given"][1], "Jacob");
        assert_eq!(read.content()["birthDate"], "1990-01-15");
        assert_eq!(read.content()["gender"], "male");
        assert_eq!(read.content()["active"], true);
        assert_eq!(read.content()["identifier"][0]["value"], "MRN-001");
        assert_eq!(read.content()["address"][0]["city"], "Springfield");
    }

    // ========================================================================
    // Search Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_search_by_name() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "Search by name should find the patient"
        );
        assert_eq!(result.resources.items[0].id(), "p1");
    }

    #[tokio::test]
    async fn postgres_integration_string_search_is_accent_insensitive() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "accent-pg",
                    "name": [{ "family": "Müller" }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        for q in ["muller", "Müller", "MULLER"] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "family".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq(q)],
                chain: vec![],
                components: vec![],
            });
            let result = backend.search(&tenant, &query).await.unwrap();
            assert_eq!(
                result.resources.items.len(),
                1,
                "accent-insensitive family search '{q}' should match 'Müller'"
            );
        }
    }

    #[tokio::test]
    async fn postgres_integration_quantity_search_ucum_equivalence() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-mass-pg",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "x" }] },
                    "valueQuantity": { "value": 1, "unit": "g", "system": "http://unitsofmeasure.org", "code": "g" }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::eq("1000|http://unitsofmeasure.org|mg")],
            chain: vec![],
            components: vec![],
        });
        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "UCUM-equivalent quantity (1000 mg) should match stored 1 g"
        );
        assert_eq!(result.resources.items[0].id(), "obs-mass-pg");
    }

    #[tokio::test]
    async fn postgres_integration_search_by_token() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "gender": "male"
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "Search by gender should find the patient"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_sort_by_id() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Create in non-sorted insertion order to prove ORDER BY is applied.
        for id in ["p3", "p1", "p2"] {
            let patient = json!({ "resourceType": "Patient", "id": id });
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        // Ascending _sort=_id
        let asc = SearchQuery::new("Patient").with_sort(SortDirective::parse("_id"));
        let result = backend.search(&tenant, &asc).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["p1", "p2", "p3"],
            "_sort=_id should return ascending id order"
        );

        // Descending _sort=-_id
        let desc = SearchQuery::new("Patient").with_sort(SortDirective::parse("-_id"));
        let result = backend.search(&tenant, &desc).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["p3", "p2", "p1"],
            "_sort=-_id should return descending id order"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_cursor_with_custom_sort() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchParamType, SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Insert out of order; page size 1 to force keyset paging across pages.
        for (id, family) in [
            ("p-charlie", "Charlie"),
            ("p-alice", "Alice"),
            ("p-bob", "Bob"),
        ] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": id, "name": [{ "family": family }] }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let mut collected = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let mut q = SearchQuery::new("Patient")
                .with_sort(
                    SortDirective::parse("family").with_param_type(Some(SearchParamType::String)),
                )
                .with_count(1);
            q.cursor = cursor.clone();
            let result = backend.search(&tenant, &q).await.unwrap();
            for r in &result.resources.items {
                collected.push(r.id().to_string());
            }
            match result.resources.page_info.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        // Keyset paging must yield the full set in family order, no dups/gaps.
        assert_eq!(
            collected,
            vec!["p-alice", "p-bob", "p-charlie"],
            "cursor paging with custom sort must preserve global order"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_sort_by_indexed_param() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchParamType, SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, family) in [
            ("p-charlie", "Charlie"),
            ("p-alice", "Alice"),
            ("p-bob", "Bob"),
        ] {
            let patient = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{ "family": family }],
            });
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let collect_ids = |result: helios_persistence::core::SearchResult| {
            result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect::<Vec<_>>()
        };

        let asc = SearchQuery::new("Patient").with_sort(
            SortDirective::parse("family").with_param_type(Some(SearchParamType::String)),
        );
        let ids = collect_ids(backend.search(&tenant, &asc).await.unwrap());
        assert_eq!(
            ids,
            vec!["p-alice", "p-bob", "p-charlie"],
            "sort by family asc"
        );

        let desc = SearchQuery::new("Patient").with_sort(
            SortDirective::parse("-family").with_param_type(Some(SearchParamType::String)),
        );
        let ids = collect_ids(backend.search(&tenant, &desc).await.unwrap());
        assert_eq!(
            ids,
            vec!["p-charlie", "p-bob", "p-alice"],
            "sort by family desc"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_missing_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "with-gender", "gender": "male" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "no-gender" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let missing = |present: &str| {
            SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq(present)],
                chain: vec![],
                components: vec![],
            })
        };

        let result = backend.search(&tenant, &missing("true")).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(ids, vec!["no-gender"], "gender:missing=true → no-gender");

        let result = backend.search(&tenant, &missing("false")).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["with-gender"],
            "gender:missing=false → with-gender"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_not_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, gender) in [("male1", Some("male")), ("female1", Some("female"))] {
            let mut patient = json!({ "resourceType": "Patient", "id": id });
            if let Some(g) = gender {
                patient["gender"] = json!(g);
            }
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }
        // A patient with no gender at all should also be returned by :not.
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "none1" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["female1", "none1"],
            "gender:not=male → non-male incl. resources with no gender"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_composite_code_value_quantity() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-bp",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
            "valueQuantity": { "value": 107, "unit": "mmHg", "system": "http://unitsofmeasure.org" }
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let query = |value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "code-value-quantity".to_string(),
                param_type: SearchParamType::Composite,
                modifier: None,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "code".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Quantity,
                        param_name: "value-quantity".to_string(),
                    },
                ],
            })
        };

        let result = backend
            .search(&tenant, &query("8480-6$ge100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "code + value match → 1 hit"
        );
        assert_eq!(result.resources.items[0].id(), "obs-bp");

        let result = backend
            .search(&tenant, &query("8480-6$ge200"))
            .await
            .unwrap();
        assert!(result.resources.items.is_empty(), "value too low → no hit");

        let result = backend
            .search(&tenant, &query("9999-9$ge100"))
            .await
            .unwrap();
        assert!(result.resources.items.is_empty(), "code mismatch → no hit");
    }

    /// A composite must match only when every component is satisfied *within the
    /// same* `composite_group`.
    ///
    /// `Observation.component` yields one composite group per component entry, so
    /// a blood-pressure panel indexes systolic (8480-6, 120) as group 0 and
    /// diastolic (8462-4, 80) as group 1 — each with its own code row and value
    /// row. A query for "diastolic > 100" must NOT match: the resource has a code
    /// row for 8462-4 (group 1) and a value row > 100 (group 0, the systolic 120),
    /// but never both in one group.
    ///
    /// Any rewrite that pushes the component predicates into the WHERE clause
    /// without correlating on `composite_group` returns this resource — a silent
    /// false positive that ships as a 100x speedup and returns the wrong patients.
    /// The single-group fixture in the test above cannot detect that class of bug.
    #[tokio::test]
    async fn postgres_integration_composite_components_must_share_a_group() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Blood-pressure panel: systolic 120, diastolic 80 — two composite groups.
        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-bp-panel",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "85354-9" }] },
            "component": [
                {
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
                    "valueQuantity": { "value": 120, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
                },
                {
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8462-4" }] },
                    "valueQuantity": { "value": 80, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
                }
            ]
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let query = |value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "component-code-value-quantity".to_string(),
                param_type: SearchParamType::Composite,
                modifier: None,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "component-code".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Quantity,
                        param_name: "component-value-quantity".to_string(),
                    },
                ],
            })
        };

        // The cross-group false positive: diastolic's code (group 1) + systolic's
        // value (group 0). Must NOT match.
        let result = backend
            .search(&tenant, &query("8462-4$gt100"))
            .await
            .unwrap();
        assert!(
            result.resources.items.is_empty(),
            "diastolic is 80, not >100 — a match here means component predicates \
             leaked across composite groups (systolic's 120 satisfied the value \
             while diastolic satisfied the code)"
        );

        // Both components satisfied within group 0 (systolic 120 > 100) → match.
        let result = backend
            .search(&tenant, &query("8480-6$gt100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "systolic 8480-6 = 120 > 100, both components in group 0 → 1 hit"
        );
        assert_eq!(result.resources.items[0].id(), "obs-bp-panel");

        // Both components satisfied within group 1 (diastolic 80 < 100) → match.
        let result = backend
            .search(&tenant, &query("8462-4$lt100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "diastolic 8462-4 = 80 < 100, both components in group 1 → 1 hit"
        );
    }

    /// Each value of a composite OR-list must be satisfied on its own; components
    /// must not pair up *across* values.
    ///
    /// The fixture's only component is (8480-6, 250). For
    /// `8480-6$lt60,8462-4$gt200`: value 1 needs code 8480-6 AND value < 60 (it is
    /// 250 — no); value 2 needs code 8462-4 (absent — no). Collapsing both values
    /// into one subquery with a merged HAVING lets value 1's code satisfy the token
    /// leg while value 2's `>200` is satisfied by the same row's 250 → false positive.
    #[tokio::test]
    async fn postgres_integration_composite_or_list_values_are_isolated() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-single-high",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "85354-9" }] },
            "component": [{
                "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
                "valueQuantity": { "value": 250, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
            }]
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let multi_value = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "component-code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![
                SearchValue::eq("8480-6$lt60"),
                SearchValue::eq("8462-4$gt200"),
            ],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "component-code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "component-value-quantity".to_string(),
                },
            ],
        });

        let result = backend.search(&tenant, &multi_value).await.unwrap();
        assert!(
            result.resources.items.is_empty(),
            "neither OR-value is satisfied on its own (8480-6 is 250 not <60; there \
             is no 8462-4) — a match means components paired across OR-values"
        );
    }

    /// String search must still match rows whose `value_string_folded` is NULL.
    ///
    /// That column arrived in schema v10 and is populated **only on write** — the
    /// migration never backfilled it (there is no `UPDATE search_index` anywhere),
    /// so every row indexed before the upgrade has NULL there. The search predicate
    /// therefore falls back to the raw `value_string`, and that fallback must stay
    /// case-insensitive.
    ///
    /// This is a tripwire, not a feature test: it is green today and must stay
    /// green. It fails the moment someone "optimizes" the predicate to
    /// `value_string_folded LIKE $n` (NULL never matches → patients silently vanish
    /// from results) or to `COALESCE(value_string_folded, value_string) LIKE $n`
    /// (the raw branch loses its case-folding → `name=smith` stops finding "Smith").
    /// Both are silent wrong-results bugs, not errors.
    #[tokio::test]
    async fn postgres_integration_string_search_matches_unbackfilled_rows() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("unbackfilled");
        // `create_tenant` suffixes a UUID for isolation, so the effective tenant id
        // must be read back off the context — the literal passed in is only a prefix.
        // Seeding `search_index` with the bare literal violates the FK to `resources`.
        let tenant_id = tenant.tenant_id().as_str();

        // No `name` element, so the writer indexes no `name` row for this Patient —
        // leaving the field clear for the hand-written legacy row below.
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "id": "legacy-p1", "gender": "female" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // A pre-v10 row: raw value only, `value_string_folded` left NULL.
        insert_search_index(
            tenant_id,
            "Patient",
            "legacy-p1",
            "name",
            "value_string",
            "Smith",
        )
        .await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            // Lowercase query against a capitalized stored value: only the
            // case-insensitive fallback can match this.
            values: vec![SearchValue::eq("smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "a row with value_string='Smith' and value_string_folded=NULL (i.e. \
             written before the v10 migration) must still match name=smith — the \
             COALESCE fallback and its case-folding are load-bearing"
        );
        assert_eq!(result.resources.items[0].id(), "legacy-p1");
    }

    // ========================================================================
    // Backend Health Check Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_health_check() {
        let backend = create_backend().await;

        let result = backend.health_check().await;
        assert!(result.is_ok(), "Health check failed: {:?}", result.err());

        // The `/_readiness` probe delegates to `Backend::health_check` via the
        // `ResourceStorage::readiness_check` override; a live pg must report ready.
        let readiness = ResourceStorage::readiness_check(&backend).await;
        assert!(
            readiness.is_ok(),
            "readiness_check failed on a live postgres: {:?}",
            readiness.err()
        );
    }

    #[tokio::test]
    async fn postgres_integration_backend_kind() {
        let backend = create_backend().await;

        assert_eq!(backend.kind(), BackendKind::Postgres);
        assert_eq!(backend.name(), "postgres");
    }

    /// Instance-level tenancy + delegation contract for PostgreSQL (#369).
    ///
    /// `tests/backend_capability_contract.rs` pins the constructor-free
    /// `PostgresBackend::declared_capabilities()`. This is the other half — that
    /// the *instance* answers the same thing through `supports()`. The #369
    /// defect lived in a hand-rolled `matches!` ladder inside `supports()`, so a
    /// declaration test alone would not have caught it. PostgreSQL is the one
    /// backend whose instance assertions need a live database, and it always has
    /// one: `create_backend()` panics rather than skipping, and CI makes
    /// `DOCKER_HOST` mandatory.
    #[tokio::test]
    async fn postgres_integration_capabilities() {
        let backend = create_backend().await;

        assert!(backend.supports(BackendCapability::Crud));
        assert!(backend.supports(BackendCapability::Versioning));
        assert!(backend.supports(BackendCapability::InstanceHistory));
        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::Transactions));
        assert!(backend.supports(BackendCapability::BulkExport));
        assert!(backend.supports(BackendCapability::BulkSubmitIngest));
        assert!(backend.supports(BackendCapability::BulkSubmitRestWorker));
        assert!(backend.supports(BackendCapability::Include));
        assert!(backend.supports(BackendCapability::Revinclude));

        // The #369 regression, asserted on the live instance rather than the
        // declaration: PostgreSQL is shared-schema only.
        assert!(backend.supports(BackendCapability::SharedSchema));
        assert!(
            !backend.supports(BackendCapability::SchemaPerTenant),
            "PostgreSQL has no SET search_path / CREATE SCHEMA path; declaring schema-per-tenant \
             overstates isolation. See #369."
        );
        assert!(
            !backend.supports(BackendCapability::DatabasePerTenant),
            "PostgreSQL has no CREATE DATABASE / per-tenant pool; declaring database-per-tenant \
             overstates isolation. See #369."
        );

        // supports() must agree with capabilities() for every declared
        // capability — the delegation the #369 fix relies on, checked live.
        for capability in backend.capabilities() {
            assert!(
                backend.supports(capability),
                "postgres capabilities() lists {capability:?} but supports() denies it — the two \
                 have drifted apart (#369)."
            );
        }
    }

    // ========================================================================
    // Additional Content Preservation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_unicode_content() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "日本語", "given": ["名前"]}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read.content()["name"][0]["family"], "日本語");
        assert_eq!(read.content()["name"][0]["given"][0], "名前");
    }

    // ========================================================================
    // Additional Tenant Isolation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_isolation_read() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant A can read
        let read_a = backend
            .read(&tenant_a, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_a.is_some());

        // Tenant B cannot read
        let read_b = backend
            .read(&tenant_b, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_b.is_none());
    }

    #[tokio::test]
    async fn postgres_integration_tenant_isolation_delete() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant B cannot delete tenant A's resource
        let result = backend.delete(&tenant_b, "Patient", created.id()).await;
        assert!(result.is_err());

        // Resource still exists for tenant A
        assert!(
            backend
                .exists(&tenant_a, "Patient", created.id())
                .await
                .unwrap()
        );
    }

    // ========================================================================
    // Additional Batch Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_read_batch_ignores_other_tenant() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create_or_update(
                &tenant_a,
                "Patient",
                "a-patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create_or_update(
                &tenant_b,
                "Patient",
                "b-patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let ids = ["a-patient", "b-patient"];
        let batch = backend
            .read_batch(&tenant_a, "Patient", &ids)
            .await
            .unwrap();

        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].id(), "a-patient");
    }

    // ========================================================================
    // Detailed History Tests - Instance
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_instance_detailed() {
        use helios_persistence::core::history::HistoryMethod;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "name": [{"family": "Smith"}]});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let v2 = backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "name": [{"family": "Jones"}]}),
            )
            .await
            .unwrap();

        let _v3 = backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "name": [{"family": "Brown"}]}),
            )
            .await
            .unwrap();

        let params = HistoryParams::new();
        let history = backend
            .history_instance(&tenant, "Patient", created.id(), &params)
            .await
            .unwrap();

        // Should have 3 versions, newest first
        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].resource.version_id(), "3");
        assert_eq!(history.items[1].resource.version_id(), "2");
        assert_eq!(history.items[2].resource.version_id(), "1");

        // Check methods
        assert_eq!(history.items[0].method, HistoryMethod::Put);
        assert_eq!(history.items[1].method, HistoryMethod::Put);
        assert_eq!(history.items[2].method, HistoryMethod::Post);

        // Check content
        assert_eq!(
            history.items[0].resource.content()["name"][0]["family"],
            "Brown"
        );
        assert_eq!(
            history.items[2].resource.content()["name"][0]["family"],
            "Smith"
        );
    }

    #[tokio::test]
    async fn postgres_integration_history_instance_count() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let v2 = backend
            .update(&tenant, &created, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        let _v3 = backend
            .update(&tenant, &v2, json!({"resourceType": "Patient"}))
            .await
            .unwrap();

        let count = backend
            .history_instance_count(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn postgres_integration_history_with_delete() {
        use helios_persistence::core::history::HistoryMethod;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "id": "hist-patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "id": "hist-patient"}),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", "hist-patient")
            .await
            .unwrap();

        let params = HistoryParams::new().include_deleted(true);
        let history = backend
            .history_instance(&tenant, "Patient", "hist-patient", &params)
            .await
            .unwrap();

        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].method, HistoryMethod::Delete);
        assert_eq!(history.items[0].resource.version_id(), "3");
    }

    #[tokio::test]
    async fn postgres_integration_history_tenant_isolation() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient", "id": "hist-shared"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(
                &tenant_a,
                &created,
                json!({"resourceType": "Patient", "id": "hist-shared"}),
            )
            .await
            .unwrap();

        // Tenant A sees history
        let history_a = backend
            .history_instance(&tenant_a, "Patient", "hist-shared", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        // Tenant B sees nothing
        let history_b = backend
            .history_instance(&tenant_b, "Patient", "hist-shared", &HistoryParams::new())
            .await
            .unwrap();
        assert!(history_b.items.is_empty());
    }

    // ========================================================================
    // Type History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_type() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "tp1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p2 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "tp2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update p1
        let _p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"resourceType": "Patient", "id": "tp1"}),
            )
            .await
            .unwrap();

        // Create an observation (different type)
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();

        // Should have 3 entries for Patient (p1 v1, p1 v2, p2 v1)
        assert_eq!(history.items.len(), 3);

        for entry in &history.items {
            assert_eq!(entry.resource.resource_type(), "Patient");
        }
    }

    #[tokio::test]
    async fn postgres_integration_history_type_count() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        let _p2 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let patient_count = backend
            .history_type_count(&tenant, "Patient")
            .await
            .unwrap();
        assert_eq!(patient_count, 3);

        let obs_count = backend
            .history_type_count(&tenant, "Observation")
            .await
            .unwrap();
        assert_eq!(obs_count, 1);
    }

    #[tokio::test]
    async fn postgres_integration_history_type_tenant_isolation() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant_b,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history_a = backend
            .history_type(&tenant_a, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        let history_b = backend
            .history_type(&tenant_b, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_b.items.len(), 1);
    }

    // ========================================================================
    // System History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_system() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "sp1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": "so1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Encounter",
                json!({"resourceType": "Encounter", "id": "se1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update patient
        let _p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"resourceType": "Patient", "id": "sp1"}),
            )
            .await
            .unwrap();

        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();

        // Should have 4 entries total
        assert_eq!(history.items.len(), 4);

        let types: std::collections::HashSet<_> = history
            .items
            .iter()
            .map(|e| e.resource.resource_type())
            .collect();
        assert!(types.contains("Patient"));
        assert!(types.contains("Observation"));
        assert!(types.contains("Encounter"));
    }

    #[tokio::test]
    async fn postgres_integration_history_system_count() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let count = backend.history_system_count(&tenant).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn postgres_integration_history_system_tenant_isolation() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant_a,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant_b,
                "Encounter",
                json!({"resourceType": "Encounter"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history_a = backend
            .history_system(&tenant_a, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        let history_b = backend
            .history_system(&tenant_b, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_b.items.len(), 1);

        assert_eq!(backend.history_system_count(&tenant_a).await.unwrap(), 2);
        assert_eq!(backend.history_system_count(&tenant_b).await.unwrap(), 1);
    }

    // ========================================================================
    // Additional Search Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_search_index_on_create() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "search-test-1",
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "MRN12345"
            }],
            "name": [{"family": "TestFamily", "given": ["TestGiven"]}],
            "birthDate": "1990-01-15"
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://example.org/mrn|MRN12345")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "search-test-1");
    }

    #[tokio::test]
    async fn postgres_integration_search_index_on_delete() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "search-delete-1",
            "identifier": [{"system": "http://example.org", "value": "DEL123"}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("DEL123")],
            chain: vec![],
            components: vec![],
        });

        let result_before = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result_before.resources.items.len(), 1);

        backend
            .delete(&tenant, "Patient", "search-delete-1")
            .await
            .unwrap();

        let result_after = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result_after.resources.items.len(),
            0,
            "Deleted resource should not be searchable"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_string_prefix() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "name-1",
                    "name": [{"family": "Smith", "given": ["John"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "name-2",
                    "name": [{"family": "Smithson", "given": ["Jane"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "name-3",
                    "name": [{"family": "Johnson", "given": ["Bob"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            2,
            "Should find 2 patients with name starting with Smith"
        );

        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"name-1"));
        assert!(ids.contains(&"name-2"));
    }

    #[tokio::test]
    async fn postgres_integration_search_date() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "date-1",
                    "birthDate": "1990-01-15"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "date-2",
                    "birthDate": "2000-06-20"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::eq("1990-01-15")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "date-1");
    }

    #[tokio::test]
    async fn postgres_integration_search_reference() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-1",
                    "subject": {"reference": "Patient/patient-1"},
                    "code": {"coding": [{"code": "8867-4"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-2",
                    "subject": {"reference": "Patient/patient-1"},
                    "code": {"coding": [{"code": "9279-1"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-3",
                    "subject": {"reference": "Patient/patient-2"},
                    "code": {"coding": [{"code": "8867-4"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/patient-1")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 2);

        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"obs-1"));
        assert!(ids.contains(&"obs-2"));
    }

    #[tokio::test]
    async fn postgres_integration_search_reference_identifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Patient with a known identifier, plus a decoy patient.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p-ident-1",
                    "identifier": [{"system": "http://hospital.org", "value": "MRN-42"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p-ident-2",
                    "identifier": [{"system": "http://hospital.org", "value": "MRN-99"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        for (oid, pid) in [("obs-i1", "p-ident-1"), ("obs-i2", "p-ident-2")] {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": oid,
                        "subject": {"reference": format!("Patient/{pid}")},
                        "status": "final"
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // subject:identifier matches the observation whose subject patient has
        // the given identifier.
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Identifier),
            values: vec![SearchValue::eq("http://hospital.org|MRN-42")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(ids, vec!["obs-i1"]);
    }

    #[tokio::test]
    async fn postgres_integration_search_tenant_isolation() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "tenant-iso-1",
                    "identifier": [{"system": "http://example.org", "value": "UNIQUE123"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("UNIQUE123")],
            chain: vec![],
            components: vec![],
        });

        let result_a = backend.search(&tenant_a, &query).await.unwrap();
        assert_eq!(result_a.resources.items.len(), 1);

        let result_b = backend.search(&tenant_b, &query).await.unwrap();
        assert_eq!(
            result_b.resources.items.len(),
            0,
            "Tenant B should not see tenant A's resources"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_multiple_parameters() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "multi-1",
                    "name": [{"family": "Smith"}],
                    "gender": "male"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "multi-2",
                    "name": [{"family": "Smith"}],
                    "gender": "female"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq("Smith")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("male")],
                chain: vec![],
                components: vec![],
            });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "AND across params should find only 1 patient"
        );
        assert_eq!(result.resources.items[0].id(), "multi-1");
    }

    // ========================================================================
    // Conditional Operations Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_conditional_create() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-12345"}],
            "name": [{"family": "Original"}]
        });

        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                patient,
                "identifier=http://hospital.org/mrn|MRN-12345",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalCreateResult::Created(_)),
            "First conditional create should succeed"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_create_exists() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-EXISTS"}],
            "name": [{"family": "Original"}]
        });

        // First create
        backend
            .conditional_create(
                &tenant,
                "Patient",
                patient.clone(),
                "identifier=http://hospital.org/mrn|MRN-EXISTS",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Second conditional create - should return existing
        let patient2 = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-EXISTS"}],
            "name": [{"family": "Duplicate"}]
        });

        let result2 = backend
            .conditional_create(
                &tenant,
                "Patient",
                patient2,
                "identifier=http://hospital.org/mrn|MRN-EXISTS",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result2, ConditionalCreateResult::Exists(_)),
            "Second conditional create should return existing resource"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_create_multiple_matches() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://system-a.org", "value": "SHARED-VALUE"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://system-b.org", "value": "SHARED-VALUE"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"value": "SHARED-VALUE"}]
                }),
                "identifier=SHARED-VALUE",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalCreateResult::MultipleMatches(_)),
            "Should report multiple matches"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_update() {
        use helios_persistence::core::{ConditionalStorage, ConditionalUpdateResult};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
                    "name": [{"family": "Original"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let updated_patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
            "name": [{"family": "Updated"}]
        });

        let result = backend
            .conditional_update(
                &tenant,
                "Patient",
                updated_patient,
                "identifier=http://hospital.org/mrn|MRN-UPDATE-1",
                false,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalUpdateResult::Updated(_)),
            "Conditional update should find and update resource"
        );

        if let ConditionalUpdateResult::Updated(updated) = result {
            assert_eq!(
                updated.content()["name"][0]["family"].as_str(),
                Some("Updated")
            );
        }
    }

    #[tokio::test]
    async fn postgres_integration_conditional_delete() {
        use helios_persistence::core::{
            ConditionalDeleteResult, ConditionalStorage, SearchProvider,
        };
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-DELETE-1"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let result = backend
            .conditional_delete(
                &tenant,
                "Patient",
                "identifier=http://hospital.org/mrn|MRN-DELETE-1",
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalDeleteResult::Deleted),
            "Conditional delete should find and delete resource"
        );

        // Verify deletion by searching
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-DELETE-1")],
            chain: vec![],
            components: vec![],
        });

        let search_result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            search_result.resources.items.is_empty(),
            "Resource should be deleted"
        );
    }

    // ========================================================================
    // Reindex Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_reindex_list_types() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": "o1", "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let types = backend.list_resource_types(&tenant).await.unwrap();
        assert!(types.contains(&"Patient".to_string()));
        assert!(types.contains(&"Observation".to_string()));
        assert_eq!(types.len(), 2);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_count() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 1..=5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("patient-{}", i)
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let count = backend.count_resources(&tenant, "Patient").await.unwrap();
        assert_eq!(count, 5);

        let count = backend
            .count_resources(&tenant, "Observation")
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_fetch_page() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 1..=10 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("patient-{:02}", i)
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Fetch first page (5 resources)
        let page1 = backend
            .fetch_resources_page(&tenant, "Patient", None, 5)
            .await
            .unwrap();
        assert_eq!(page1.resources.len(), 5);
        assert!(page1.next_cursor.is_some());

        // Fetch second page using cursor
        let page2 = backend
            .fetch_resources_page(&tenant, "Patient", page1.next_cursor.as_deref(), 5)
            .await
            .unwrap();
        assert_eq!(page2.resources.len(), 5);

        // Ensure no duplicates between pages
        let page1_ids: Vec<&str> = page1.resources.iter().map(|r| r.id()).collect();
        let page2_ids: Vec<&str> = page2.resources.iter().map(|r| r.id()).collect();
        for id in &page1_ids {
            assert!(!page2_ids.contains(id), "Duplicate ID found: {}", id);
        }

        // Fetch third page (should be empty or have no more cursor)
        let page3 = backend
            .fetch_resources_page(&tenant, "Patient", page2.next_cursor.as_deref(), 5)
            .await
            .unwrap();
        assert!(page3.resources.is_empty() || page3.next_cursor.is_none());
    }

    /// Inserts a row directly into the search_index table. Mirrors what the
    /// SQLite chain tests do for the same purpose — exercises the chain SQL
    /// without depending on the FHIRPath extractor's full coverage. Connects
    /// to the shared testcontainer with its own tokio-postgres client because
    /// `PostgresBackend::get_client` is crate-private.
    async fn insert_search_index(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        param_name: &str,
        column: &str,
        value: &str,
    ) {
        let pg = shared_pg().await;
        let conn_str = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port,
        );
        let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let sql = format!(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, {col}) \
             VALUES ($1, $2, $3, $4, $5)",
            col = column,
        );
        client
            .execute(
                &sql,
                &[
                    &tenant_id,
                    &resource_type,
                    &resource_id,
                    &param_name,
                    &value,
                ],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_resolve_chain_multi_level() {
        use helios_persistence::core::ChainedSearchProvider;

        // Mirror sqlite/search_impl.rs::test_resolve_chain_multi_level for
        // Postgres. Three-level chain: Observation?subject.organization.name=Hospital.
        let backend = create_backend().await;
        let tenant = create_tenant("chain-multi");
        let tenant_id = tenant.tenant_id().as_str();

        backend
            .create(
                &tenant,
                "Organization",
                json!({"id": "org1", "name": "General Hospital"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "managingOrganization": {"reference": "Organization/org1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        insert_search_index(
            tenant_id,
            "Organization",
            "org1",
            "name",
            "value_string",
            "General Hospital",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Patient",
            "p1",
            "organization",
            "value_reference",
            "Organization/org1",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "subject",
            "value_reference",
            "Patient/p1",
        )
        .await;

        let ids = backend
            .resolve_chain(
                &tenant,
                "Observation",
                "subject.organization.name",
                "Hospital",
            )
            .await
            .unwrap();

        assert_eq!(ids, vec!["o1".to_string()]);
    }

    #[tokio::test]
    async fn postgres_integration_resolve_reverse_chain_terminal() {
        use helios_persistence::core::ChainedSearchProvider;
        use helios_persistence::types::{ReverseChainedParameter, SearchValue};

        // _has:Observation:subject:code=8867-4 — find patients referenced by
        // Observations whose code matches.
        let backend = create_backend().await;
        let tenant = create_tenant("reverse-chain");
        let tenant_id = tenant.tenant_id().as_str();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o2", "subject": {"reference": "Patient/p2"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "subject",
            "value_reference",
            "Patient/p1",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o2",
            "subject",
            "value_reference",
            "Patient/p2",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "code",
            "value_token_code",
            "8867-4",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o2",
            "code",
            "value_token_code",
            "other",
        )
        .await;

        let rc = ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "code",
            SearchValue::eq("8867-4"),
        );
        let ids = backend
            .resolve_reverse_chain(&tenant, "Patient", &rc)
            .await
            .unwrap();
        assert_eq!(ids, vec!["p1".to_string()]);
    }

    // ========================================================================
    // Bulk Export — Phase 2 multi-instance job state on Postgres.
    // ========================================================================

    use chrono::Utc;
    use helios_persistence::core::bulk_export::{
        BulkExportStorage, ExportRequest, ExportStatus, StartExportInput, TypeExportProgress,
    };
    use helios_persistence::core::bulk_export_worker::{
        ExportClaimStrategy, ExportWorkerStorage, LeaseError, WorkerId,
    };
    use std::time::Duration as StdDuration;

    fn export_input(request: ExportRequest) -> StartExportInput {
        StartExportInput {
            request,
            transaction_time: Utc::now(),
            request_url: "http://localhost/$export".to_string(),
            owner_subject: Some("pg-test".to_string()),
            fhir_version: FhirVersion::default(),
        }
    }

    /// Claims jobs in a loop until the lease for `target` is returned;
    /// releases any other jobs claimed along the way. Robust to concurrent
    /// tests sharing the testcontainers PostgreSQL instance.
    async fn claim_specific(
        backend: &helios_persistence::backends::postgres::PostgresBackend,
        worker_id: &WorkerId,
        target: &helios_persistence::core::bulk_export::ExportJobId,
        lease_duration: StdDuration,
    ) -> helios_persistence::core::bulk_export_worker::ExportJobLease {
        for _ in 0..100 {
            match backend.claim_next(worker_id, lease_duration).await.unwrap() {
                Some(lease) if &lease.job_id == target => return lease,
                Some(other) => {
                    // Drain other tests' jobs out of the queue by completing
                    // them (so the claim ordering moves on instead of
                    // looping back to the same job after `release`).
                    let _ = backend
                        .finish_export_job(
                            &other.tenant,
                            &other.job_id,
                            &other.worker_id,
                            other.fencing_token,
                        )
                        .await;
                }
                None => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
        panic!("never claimed the expected job");
    }

    #[tokio::test]
    async fn postgres_integration_export_claim_skip_locked() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-claim");

        let job_id = backend
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new(format!("pg-worker-a-{}", uuid::Uuid::new_v4()));
        let lease_a =
            claim_specific(&backend, &worker_a, &job_id, StdDuration::from_secs(60)).await;
        assert!(lease_a.fencing_token >= 1);

        // Worker A finishes via the fenced ExportWorkerStorage.
        backend
            .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();
        backend
            .update_export_type_progress(
                &tenant,
                &job_id,
                &worker_a,
                lease_a.fencing_token,
                &TypeExportProgress::new("Patient"),
            )
            .await
            .unwrap();
        backend
            .finish_export_job(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Complete);
    }

    #[tokio::test]
    async fn postgres_integration_export_stale_worker_fenced_out() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-fence");

        let job_id = backend
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        // Worker A takes a very short lease, then Worker B reclaims.
        let worker_a = WorkerId::new(format!("pg-stale-a-{}", uuid::Uuid::new_v4()));
        let lease_a =
            claim_specific(&backend, &worker_a, &job_id, StdDuration::from_millis(1)).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let worker_b = WorkerId::new(format!("pg-stale-b-{}", uuid::Uuid::new_v4()));
        let lease_b =
            claim_specific(&backend, &worker_b, &job_id, StdDuration::from_secs(60)).await;
        assert!(lease_b.fencing_token > lease_a.fencing_token);

        // Worker A is fenced out from every mutation.
        assert!(matches!(
            backend
                .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            backend
                .finish_export_job(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));

        // Worker B can still finish.
        backend
            .finish_export_job(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_export_count_active_and_expire() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-cleanup");

        for _ in 0..2 {
            backend
                .start_export(&tenant, export_input(ExportRequest::system()))
                .await
                .unwrap();
        }
        assert_eq!(backend.count_active_exports(&tenant).await.unwrap(), 2);

        // Nothing is expired yet.
        let expired_now = backend
            .list_expired_exports(Utc::now(), StdDuration::from_secs(3600), 100)
            .await
            .unwrap();
        // Only completed/error/cancelled jobs can expire — these are accepted.
        assert!(expired_now.is_empty());
    }

    // ========================================================================
    // Per-user settings store
    // ========================================================================

    /// A user key unique to each test, so tests sharing the database don't
    /// collide on the single-row-per-user `user_settings` table.
    fn unique_user_key(prefix: &str) -> String {
        format!("{}|{}", prefix, uuid::Uuid::new_v4().simple())
    }

    /// `delete_settings` removes the row and reports whether one existed —
    /// the primitive the #270 legacy-key migration uses to move a document
    /// rather than leave a duplicate copy behind.
    #[tokio::test]
    async fn postgres_integration_settings_delete_is_idempotent() {
        let backend = create_backend().await;
        let user = unique_user_key("delete");

        // Absent is not an error, and reports "nothing removed".
        assert!(!backend.delete_settings(&user).await.unwrap());

        backend
            .put_settings(&user, json!({"theme": "dark"}), None)
            .await
            .unwrap();
        assert!(backend.get_settings(&user).await.unwrap().is_some());

        assert!(backend.delete_settings(&user).await.unwrap());
        assert!(backend.get_settings(&user).await.unwrap().is_none());
        assert!(!backend.delete_settings(&user).await.unwrap());
    }

    #[tokio::test]
    async fn postgres_integration_settings_get_missing_is_none() {
        let backend = create_backend().await;
        let user = unique_user_key("missing");
        assert!(backend.get_settings(&user).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn postgres_integration_settings_put_get_and_version() {
        let backend = create_backend().await;
        let user = unique_user_key("round-trip");
        let doc = json!({"theme": "dark", "recentQueries": {"Patient": ["name=smith"]}});

        let stored = backend
            .put_settings(&user, doc.clone(), None)
            .await
            .unwrap();
        assert_eq!(stored.version, 1);

        let fetched = backend.get_settings(&user).await.unwrap().unwrap();
        assert_eq!(fetched.document, doc);
        assert_eq!(fetched.version, 1);

        let second = backend
            .put_settings(&user, json!({"theme": "light"}), None)
            .await
            .unwrap();
        assert_eq!(second.version, 2);
    }

    #[tokio::test]
    async fn postgres_integration_settings_patch_merges_and_deletes() {
        let backend = create_backend().await;
        let user = unique_user_key("patch");
        backend
            .put_settings(
                &user,
                json!({"theme": "dark", "defaultTenant": "acme"}),
                None,
            )
            .await
            .unwrap();

        let patched = backend
            .patch_settings(
                &user,
                json!({"theme": "light", "defaultTenant": null}),
                None,
            )
            .await
            .unwrap();
        assert_eq!(patched.document, json!({"theme": "light"}));
        assert_eq!(patched.version, 2);
    }

    #[tokio::test]
    async fn postgres_integration_settings_optimistic_lock() {
        let backend = create_backend().await;
        let user = unique_user_key("lock");
        backend
            .put_settings(&user, json!({"a": 1}), None)
            .await
            .unwrap(); // version 1

        // Stale precondition is rejected.
        let err = backend
            .put_settings(&user, json!({"a": 2}), Some(0))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
        ));

        // Matching precondition succeeds.
        let ok = backend
            .put_settings(&user, json!({"a": 2}), Some(1))
            .await
            .unwrap();
        assert_eq!(ok.version, 2);
    }

    // ========================================================================
    // _contained / _containedType search
    // ========================================================================

    async fn seed_contained(backend: &PostgresBackend, tenant: &TenantContext) {
        backend
            .create(
                tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs1",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "#p1" },
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{ "family": "Smith", "given": ["Contained"] }],
                        "gender": "male"
                    }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "top1", "name": [{ "family": "Smith" }] }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    fn contained_name_query(
        mode: helios_persistence::types::ContainedMode,
        ret: helios_persistence::types::ContainedReturn,
    ) -> helios_persistence::types::SearchQuery {
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };
        let mut q = SearchQuery::new("Patient");
        q.contained = mode;
        q.contained_return = ret;
        q.parameters.push(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });
        q
    }

    #[tokio::test]
    async fn postgres_integration_contained_off_excludes_contained() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::Off, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Patient/top1"]);
    }

    #[tokio::test]
    async fn postgres_integration_contained_returns_container() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::On, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Observation/obs1"]);
    }

    #[tokio::test]
    async fn postgres_integration_contained_type_contained_returns_contained() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::On, ContainedReturn::Contained),
            )
            .await
            .unwrap();
        assert_eq!(result.resources.items.len(), 1);
        let r = &result.resources.items[0];
        assert_eq!(r.resource_type(), "Patient");
        assert_eq!(r.id(), "p1");
        assert_eq!(r.content()["name"][0]["given"][0], "Contained");
    }

    #[tokio::test]
    async fn postgres_integration_contained_both_merges() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::Both, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let mut urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        urls.sort();
        assert_eq!(urls, vec!["Observation/obs1", "Patient/top1"]);
    }

    #[tokio::test]
    async fn postgres_integration_supports_contained_search() {
        use helios_persistence::core::SearchProvider;
        let backend = create_backend().await;
        assert!(backend.supports_contained_search());
    }

    // ========================================================================
    // Cluster T2 Tests
    // ========================================================================

    /// T2 cross-instance suite for schema initialization: four independently
    /// constructed backends (fresh pools, not a cloned `Arc`) cold-start
    /// concurrently against one fresh, empty database.
    ///
    /// DoD rows — exclusivity: init is serialized by the advisory lock
    /// (without it, a losing instance aborts on a pg_type duplicate-key error
    /// from racing `CREATE TABLE IF NOT EXISTS`); visibility: create via
    /// handle A → readable via handle B; isolation: wrong tenant sees nothing.
    #[tokio::test]
    async fn postgres_integration_cluster_concurrent_cold_start_schema_init() {
        let pg = shared_pg().await;

        // A database no other test touches — this must be a true cold start.
        let dbname = format!("cold_start_{}", uuid::Uuid::new_v4().simple());
        let admin_conn = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port,
        );
        let (admin, connection) = tokio_postgres::connect(&admin_conn, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin
            .execute(format!("CREATE DATABASE {}", dbname).as_str(), &[])
            .await
            .expect("create cold-start database");

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let mut backends = Vec::new();
        for _ in 0..4 {
            let config = PostgresConfig {
                host: pg.host.clone(),
                port: pg.port,
                dbname: dbname.clone(),
                user: "postgres".to_string(),
                password: Some("postgres".to_string()),
                max_connections: 2,
                data_dir: Some(data_dir.clone()),
                ..Default::default()
            };
            backends.push(
                PostgresBackend::new(config)
                    .await
                    .expect("Failed to create PostgresBackend"),
            );
        }

        // All four handles start init from a shared barrier to force the race.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(backends.len()));
        let tasks: Vec<_> = backends
            .into_iter()
            .map(|backend| {
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    backend.init_schema().await.map(|_| backend)
                })
            })
            .collect();
        let mut handles = Vec::new();
        for task in tasks {
            handles.push(
                task.await
                    .expect("init task panicked")
                    .expect("every instance must survive a concurrent cold start"),
            );
        }

        // Init converged: exactly one schema_version row.
        let cold_conn = format!(
            "host={} port={} user=postgres password=postgres dbname={}",
            pg.host, pg.port, dbname,
        );
        let (cold, connection) = tokio_postgres::connect(&cold_conn, tokio_postgres::NoTls)
            .await
            .expect("connect to cold-start db");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let rows = cold
            .query("SELECT version FROM schema_version", &[])
            .await
            .expect("query schema_version");
        assert_eq!(
            rows.len(),
            1,
            "racing inits must not duplicate schema_version rows"
        );
        assert!(rows[0].get::<_, i32>(0) >= 1);

        // Visibility: create via handle A → readable via handle B (same tenant).
        let tenant = create_tenant("cluster-cold-start");
        let created = handles[0]
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let read_b = handles[1]
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_b.is_some());

        // Isolation: a different tenant on handle B sees nothing.
        let other_tenant = create_tenant("cluster-cold-start-other");
        let read_other = handles[1]
            .read(&other_tenant, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_other.is_none());
    }

    // ========================================================================
    // T2 cluster calibration suite — bulk-export job store
    // ========================================================================
    //
    // The bulk-export job store is already cluster-safe; these
    // tests calibrate the `cluster_harness` helpers against it, so every row
    // must pass green immediately. If a row is red, either the harness is
    // unfaithful or the "already safe" boundary is drawn wrong — both are
    // exactly what calibration exists to flush out (methodology §5).
    // ========================================================================

    use crate::cluster_harness as harness;
    use helios_persistence::core::bulk_export::{ExportJobId, ExportProgress};
    use helios_persistence::error::BulkExportError;

    /// Maps `get_export_status` to the harness's `Option` shape: a job that
    /// doesn't exist *for this tenant* is `None`; any other error is a bug.
    async fn export_status_across(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> Option<ExportProgress> {
        match backend.get_export_status(tenant, job_id).await {
            Ok(progress) => Some(progress),
            Err(StorageError::BulkExport(BulkExportError::JobNotFound { .. })) => None,
            Err(other) => panic!("unexpected error reading export status: {other:?}"),
        }
    }

    /// Claims and finishes every eligible job so an exclusivity race starts
    /// from an empty queue. Leftover `accepted` jobs from other suites
    /// sharing the container would otherwise let both racers win on
    /// different jobs.
    async fn drain_export_queue(backend: &PostgresBackend) {
        let reaper = WorkerId::new(format!("cluster-drain-{}", uuid::Uuid::new_v4()));
        while let Some(lease) = backend
            .claim_next(&reaper, StdDuration::from_secs(60))
            .await
            .unwrap()
        {
            backend
                .finish_export_job(
                    &lease.tenant,
                    &lease.job_id,
                    &lease.worker_id,
                    lease.fencing_token,
                )
                .await
                .expect("draining a leftover export job");
        }
    }

    /// DoD rows: visibility + isolation (strategy §4).
    #[tokio::test]
    async fn postgres_integration_cluster_bulk_export_visible_and_isolated_across_handles() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-vis");

        let job_id = handles
            .a
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        // Visibility: created via instance A, observable via instance B.
        let progress = harness::assert_visible(
            export_status_across(&handles.b, &tenant, &job_id).await,
            "export job created via instance A",
        );
        assert_eq!(progress.status, ExportStatus::Accepted);

        // Isolation: another tenant on instance B sees nothing.
        let other_tenant = create_tenant("cluster-vis-other");
        harness::assert_wrong_tenant_hidden(
            export_status_across(&handles.b, &other_tenant, &job_id).await,
            "export job",
        );
    }

    /// DoD row: exclusivity — two instances race `claim_next` on one queued
    /// job and exactly one wins (`FOR UPDATE SKIP LOCKED`).
    #[tokio::test]
    async fn postgres_integration_cluster_bulk_export_claim_exclusive_across_handles() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-claim");

        drain_export_queue(&handles.a).await;

        let job_id = handles
            .a
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new(format!("cluster-claim-a-{}", uuid::Uuid::new_v4()));
        let worker_b = WorkerId::new(format!("cluster-claim-b-{}", uuid::Uuid::new_v4()));
        let (a, b) = (handles.a, handles.b);
        let (lease_a, lease_b) = harness::race2(
            async move {
                a.claim_next(&worker_a, StdDuration::from_secs(60))
                    .await
                    .unwrap()
            },
            async move {
                b.claim_next(&worker_b, StdDuration::from_secs(60))
                    .await
                    .unwrap()
            },
        )
        .await;

        harness::assert_exactly_one(&lease_a, &lease_b, "the claim on one queued job");
        let winner = lease_a.or(lease_b).unwrap();
        assert_eq!(winner.job_id, job_id);
        assert_eq!(
            winner.tenant.tenant_id().as_str(),
            tenant.tenant_id().as_str()
        );
    }

    /// DoD row: fencing — after a release/reclaim moves the lease to another
    /// instance, the stale holder's heartbeat and guarded writes are refused.
    /// Deterministic: the reclaim uses `release`, not lease expiry, so there
    /// are no sleeps (methodology §4, coverage-safe).
    #[tokio::test]
    async fn postgres_integration_cluster_bulk_export_stale_handle_fenced_after_release() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-fence");

        let job_id = handles
            .a
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        // Instance A claims, then releases (graceful shutdown).
        let worker_a = WorkerId::new(format!("cluster-fence-a-{}", uuid::Uuid::new_v4()));
        let lease_a =
            claim_specific(&handles.a, &worker_a, &job_id, StdDuration::from_secs(60)).await;
        ExportClaimStrategy::release(&handles.a, lease_a.clone())
            .await
            .unwrap();

        // Instance B reclaims; the fencing token moves past A's.
        let worker_b = WorkerId::new(format!("cluster-fence-b-{}", uuid::Uuid::new_v4()));
        let lease_b =
            claim_specific(&handles.b, &worker_b, &job_id, StdDuration::from_secs(60)).await;
        assert!(lease_b.fencing_token > lease_a.fencing_token);

        // The stale handle is fenced out of heartbeat and guarded writes.
        assert!(matches!(
            handles.a.heartbeat(&lease_a).await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            handles
                .a
                .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));

        // The current holder is unaffected.
        handles
            .b
            .finish_export_job(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
    }

    /// DoD row: durability — dropping the creating handle (simulated
    /// redeploy) must not lose the job; a fresh handle still sees it.
    #[tokio::test]
    async fn postgres_integration_cluster_bulk_export_survives_handle_drop() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let tenant = create_tenant("cluster-durable");

        let first = create_backend().await;
        let job_id = first
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();
        drop(first);

        let replacement = create_backend().await;
        let progress = harness::assert_visible(
            export_status_across(&replacement, &tenant, &job_id).await,
            "export job after its creating handle was dropped",
        );
        assert_eq!(progress.status, ExportStatus::Accepted);
    }

    // ========================================================================
    // T2 cluster suite — F5 resource version-id race
    // ========================================================================
    //
    // `update`/`delete` are version-guarded (CAS) and bump the version inside
    // one transaction with the history insert, so two instances racing
    // unconditional writes can never both assign the same version_id or lose
    // a history row. `create_or_update` retries transient CAS losses, keeping
    // unconditional PUT last-writer-wins at the API surface.
    // ========================================================================

    /// Asserts a resource's history holds every version exactly once,
    /// contiguous from 1 — no duplicate version_ids, no lost history rows.
    async fn assert_history_versions(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_len: u64,
    ) {
        let history = backend
            .history_instance(
                tenant,
                resource_type,
                id,
                &HistoryParams::new().include_deleted(true),
            )
            .await
            .unwrap();
        let mut versions: Vec<u64> = history
            .items
            .iter()
            .map(|e| e.resource.version_id().parse().unwrap())
            .collect();
        versions.sort_unstable();
        assert_eq!(
            versions,
            (1..=expected_len).collect::<Vec<_>>(),
            "history must hold every version exactly once"
        );
    }

    /// DoD rows (F5): exclusivity + isolation — two instances race
    /// unconditional `update`s from the same version-1 snapshot. The CAS lets
    /// exactly one through; the loser gets `VersionConflict` instead of
    /// silently losing an update or 500ing on a duplicate history row, and a
    /// fresh read-then-retry converges on version 3.
    #[tokio::test]
    async fn postgres_integration_cluster_resource_update_race_no_lost_version() {
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-f5-upd");

        let created = handles
            .a
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "V1"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();

        let (a, b) = (handles.a, handles.b);
        let (tenant_a, tenant_b) = (tenant.clone(), tenant.clone());
        let (snapshot_a, snapshot_b) = (created.clone(), created);
        let (res_a, res_b) = harness::race2(
            async move {
                a.update(
                    &tenant_a,
                    &snapshot_a,
                    json!({"resourceType": "Patient", "name": [{"family": "FromA"}]}),
                )
                .await
            },
            async move {
                b.update(
                    &tenant_b,
                    &snapshot_b,
                    json!({"resourceType": "Patient", "name": [{"family": "FromB"}]}),
                )
                .await
            },
        )
        .await;

        let (ok_a, err_a) = match res_a {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        };
        let (ok_b, err_b) = match res_b {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        };
        harness::assert_exactly_one(&ok_a, &ok_b, "the version-guarded update");
        let winner = ok_a.or(ok_b).unwrap();
        assert_eq!(winner.version_id(), "2");
        let loser = err_a.or(err_b).unwrap();
        assert!(
            matches!(
                loser,
                StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })
            ),
            "loser must see VersionConflict, got {loser:?}"
        );

        // A fresh read-then-retry from a third instance converges.
        let verifier = create_backend().await;
        let current = verifier
            .read(&tenant, "Patient", &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.version_id(), "2");
        let retried = verifier
            .update(
                &tenant,
                &current,
                json!({"resourceType": "Patient", "name": [{"family": "Retry"}]}),
            )
            .await
            .unwrap();
        assert_eq!(retried.version_id(), "3");

        assert_history_versions(&verifier, &tenant, "Patient", &id, 3).await;

        // Isolation: the resource does not exist for another tenant.
        harness::assert_wrong_tenant_hidden(
            verifier
                .read(&create_tenant("cluster-f5-other"), "Patient", &id)
                .await
                .unwrap(),
            "patient",
        );
    }

    /// DoD row (F5): unconditional PUT stays last-writer-wins — two instances
    /// race `create_or_update` on one existing resource; the bounded CAS
    /// retry absorbs the losing race so both callers succeed, with distinct
    /// version_ids and a complete history.
    #[tokio::test]
    async fn postgres_integration_cluster_resource_create_or_update_race_both_succeed() {
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-f5-put");

        let created = handles
            .a
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "V1"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();

        let (a, b) = (handles.a, handles.b);
        let (tenant_a, tenant_b) = (tenant.clone(), tenant.clone());
        let (id_a, id_b) = (id.clone(), id.clone());
        let (res_a, res_b) = harness::race2(
            async move {
                a.create_or_update(
                    &tenant_a,
                    "Patient",
                    &id_a,
                    json!({"resourceType": "Patient", "name": [{"family": "FromA"}]}),
                    FhirVersion::default(),
                )
                .await
            },
            async move {
                b.create_or_update(
                    &tenant_b,
                    "Patient",
                    &id_b,
                    json!({"resourceType": "Patient", "name": [{"family": "FromB"}]}),
                    FhirVersion::default(),
                )
                .await
            },
        )
        .await;

        let (stored_a, created_a) = res_a.expect("create_or_update must absorb CAS races");
        let (stored_b, created_b) = res_b.expect("create_or_update must absorb CAS races");
        assert!(
            !created_a && !created_b,
            "both racers must take the update arm"
        );
        assert_ne!(
            stored_a.version_id(),
            stored_b.version_id(),
            "racing writers must never assign the same version"
        );

        let verifier = create_backend().await;
        let current = verifier
            .read(&tenant, "Patient", &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.version_id(), "3");
        assert_history_versions(&verifier, &tenant, "Patient", &id, 3).await;
    }

    /// DoD row (F5): an unconditional `update` racing an unconditional
    /// `delete` can interleave either way, but the CAS keeps the invariants:
    /// the delete always lands, no duplicate version_id, no lost history row,
    /// and the resource ends deleted.
    #[tokio::test]
    async fn postgres_integration_cluster_resource_update_delete_race_keeps_history_coherent() {
        let handles = harness::two_handles(create_backend).await;
        let tenant = create_tenant("cluster-f5-del");

        let created = handles
            .a
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "V1"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();

        let (a, b) = (handles.a, handles.b);
        let (tenant_a, tenant_b) = (tenant.clone(), tenant.clone());
        let id_b = id.clone();
        let (res_update, res_delete) = harness::race2(
            async move {
                a.update(
                    &tenant_a,
                    &created,
                    json!({"resourceType": "Patient", "name": [{"family": "FromA"}]}),
                )
                .await
            },
            async move { b.delete(&tenant_b, "Patient", &id_b).await },
        )
        .await;

        // The delete retries CAS losses internally, so it always lands.
        res_delete.expect("racing delete must succeed");

        // The update either beat the delete (then the delete re-read and
        // bumped again) or found the row gone.
        let expected_versions = match res_update {
            Ok(stored) => {
                assert_eq!(stored.version_id(), "2");
                3
            }
            Err(err) => {
                assert!(
                    matches!(err, StorageError::Resource(ResourceError::NotFound { .. })),
                    "update racing a delete must see NotFound, got {err:?}"
                );
                2
            }
        };

        let verifier = create_backend().await;
        match verifier.read(&tenant, "Patient", &id).await {
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
            other => panic!("resource must end deleted, got {other:?}"),
        }
        assert_history_versions(&verifier, &tenant, "Patient", &id, expected_versions).await;
    }

    // ========================================================================
    // T2 cluster suite — unified cluster job store (A1 substrate)
    // ========================================================================
    //
    // The `cluster_jobs` table + `ClusterJobStore` back the SoF `$export`
    // (#169) and reindex (A2) surfaces. Every test takes
    // `CLUSTER_JOBS_TEST_LOCK`: `claim_next` is deliberately cross-tenant,
    // so parallel tests sharing the container would claim each other's jobs.
    //
    // Nested module: `ClusterJobStore` and `ExportClaimStrategy` both give
    // `PostgresBackend` a `claim_next`/`heartbeat`/`release`, so the two
    // traits must never be in scope together or every call is ambiguous.
    // ========================================================================

    mod cluster_jobs_suite {
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        use serde_json::json;

        use helios_persistence::core::{
            ClusterJobState, ClusterJobStore, ClusterLeaseError, JobKind, ResourceStorage, WorkerId,
        };

        use super::{CLUSTER_JOBS_TEST_LOCK, create_backend, create_tenant};
        use crate::cluster_harness as harness;

        /// The server-facing seam: the store comes off the backend the same
        /// way `sof_runner()` does.
        pub(super) fn store_of(backend: &impl ResourceStorage) -> Arc<dyn ClusterJobStore> {
            backend
                .cluster_job_store()
                .expect("postgres backs a cluster job store")
        }

        /// Claims and finishes every eligible job of `kind` so an exclusivity
        /// or fencing test starts from an empty queue (same rationale as
        /// `drain_export_queue`).
        pub(super) async fn drain_cluster_jobs(store: &dyn ClusterJobStore, kind: JobKind) {
            let reaper = WorkerId::new(format!("cluster-jobs-drain-{}", uuid::Uuid::new_v4()));
            while let Some((lease, _payload)) = store
                .claim_next(kind, &reaper, StdDuration::from_secs(60))
                .await
                .unwrap()
            {
                store
                    .complete(&lease, json!({"drained": true}))
                    .await
                    .expect("draining a leftover cluster job");
            }
        }

        /// DoD rows: visibility + isolation — a job enqueued via instance A is
        /// observable (status and list) via instance B, and invisible to
        /// another tenant.
        #[tokio::test]
        async fn postgres_integration_cluster_jobs_visible_and_isolated_across_handles() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let tenant = create_tenant("cluster-jobs-vis");

            let job_id = a
                .enqueue(&tenant, JobKind::SofExport, json!({"probe": "vis"}))
                .await
                .unwrap();

            let rec = harness::assert_visible(
                b.get_status(&tenant, &job_id).await.unwrap(),
                "cluster job enqueued via instance A",
            );
            assert_eq!(rec.state, ClusterJobState::Queued);
            assert_eq!(rec.payload, json!({"probe": "vis"}));
            assert!(!rec.cancel_requested);

            let listed = b.list_jobs(&tenant, JobKind::SofExport).await.unwrap();
            assert_eq!(listed.len(), 1, "list_jobs must see the job via instance B");

            harness::assert_wrong_tenant_hidden(
                b.get_status(&create_tenant("cluster-jobs-vis-other"), &job_id)
                    .await
                    .unwrap(),
                "cluster job",
            );
            assert!(
                b.list_jobs(&create_tenant("cluster-jobs-vis-other"), JobKind::SofExport)
                    .await
                    .unwrap()
                    .is_empty(),
                "tenant isolation violated: cluster job listed under the wrong tenant"
            );
        }

        /// DoD row: exclusivity — two instances race `claim_next` on one
        /// queued job and exactly one wins (`FOR UPDATE SKIP LOCKED`).
        #[tokio::test]
        async fn postgres_integration_cluster_jobs_claim_exclusive_across_handles() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let tenant = create_tenant("cluster-jobs-claim");

            drain_cluster_jobs(a.as_ref(), JobKind::SofExport).await;

            let job_id = a
                .enqueue(&tenant, JobKind::SofExport, json!({"probe": "claim"}))
                .await
                .unwrap();

            let worker_a = WorkerId::new(format!("cluster-jobs-a-{}", uuid::Uuid::new_v4()));
            let worker_b = WorkerId::new(format!("cluster-jobs-b-{}", uuid::Uuid::new_v4()));
            let (racer_a, racer_b) = (Arc::clone(&a), Arc::clone(&b));
            let (claim_a, claim_b) = harness::race2(
                async move {
                    racer_a
                        .claim_next(JobKind::SofExport, &worker_a, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
                async move {
                    racer_b
                        .claim_next(JobKind::SofExport, &worker_b, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
            )
            .await;

            harness::assert_exactly_one(&claim_a, &claim_b, "the claim on one queued cluster job");
            let (lease, payload) = claim_a.or(claim_b).unwrap();
            assert_eq!(lease.job_id, job_id);
            assert_eq!(payload, json!({"probe": "claim"}));
            assert_eq!(
                lease.tenant.tenant_id().as_str(),
                tenant.tenant_id().as_str()
            );
        }

        /// DoD row: fencing — after a release/reclaim moves the lease to
        /// another instance, every stale-holder write is refused with
        /// `LeaseLost` while the current holder is unaffected. Deterministic:
        /// release, not expiry.
        #[tokio::test]
        async fn postgres_integration_cluster_jobs_stale_handle_fenced_after_release() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let tenant = create_tenant("cluster-jobs-fence");

            drain_cluster_jobs(a.as_ref(), JobKind::SofExport).await;

            let job_id = a
                .enqueue(&tenant, JobKind::SofExport, json!({"probe": "fence"}))
                .await
                .unwrap();

            let worker_a = WorkerId::new(format!("cluster-fence-a-{}", uuid::Uuid::new_v4()));
            let (lease_a, _) = a
                .claim_next(JobKind::SofExport, &worker_a, StdDuration::from_secs(60))
                .await
                .unwrap()
                .expect("instance A claims the queued job");
            assert_eq!(lease_a.job_id, job_id);
            a.release(lease_a.clone()).await.unwrap();

            let worker_b = WorkerId::new(format!("cluster-fence-b-{}", uuid::Uuid::new_v4()));
            let (lease_b, _) = b
                .claim_next(JobKind::SofExport, &worker_b, StdDuration::from_secs(60))
                .await
                .unwrap()
                .expect("instance B reclaims after release");
            assert_eq!(lease_b.job_id, job_id);
            assert!(lease_b.fencing_token > lease_a.fencing_token);

            assert!(matches!(
                a.heartbeat(&lease_a).await,
                Err(ClusterLeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                a.update_progress(&lease_a, json!({"pct": 1})).await,
                Err(ClusterLeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                a.complete(&lease_a, json!({})).await,
                Err(ClusterLeaseError::LeaseLost { .. })
            ));

            b.heartbeat(&lease_b).await.unwrap();
            b.complete(&lease_b, json!({"ok": true})).await.unwrap();

            let rec = a.get_status(&tenant, &job_id).await.unwrap().unwrap();
            assert_eq!(rec.state, ClusterJobState::Completed);
            assert_eq!(rec.result, Some(json!({"ok": true})));
        }

        /// DoD row: durability — dropping the enqueueing handle (simulated
        /// redeploy) must not lose the job; a fresh handle sees and can claim
        /// it.
        #[tokio::test]
        async fn postgres_integration_cluster_jobs_survive_handle_drop() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let tenant = create_tenant("cluster-jobs-durable");

            let first_backend = create_backend().await;
            let first = store_of(&first_backend);
            drain_cluster_jobs(first.as_ref(), JobKind::Reindex).await;
            let job_id = first
                .enqueue(&tenant, JobKind::Reindex, json!({"probe": "durable"}))
                .await
                .unwrap();
            drop(first);
            drop(first_backend);

            let replacement_backend = create_backend().await;
            let replacement = store_of(&replacement_backend);
            let rec = harness::assert_visible(
                replacement.get_status(&tenant, &job_id).await.unwrap(),
                "cluster job after its enqueueing handle was dropped",
            );
            assert_eq!(rec.state, ClusterJobState::Queued);

            let (lease, payload) = replacement
                .claim_next(
                    JobKind::Reindex,
                    &WorkerId::new(format!("cluster-durable-{}", uuid::Uuid::new_v4())),
                    StdDuration::from_secs(60),
                )
                .await
                .unwrap()
                .expect("a fresh handle claims the surviving job");
            assert_eq!(lease.job_id, job_id);
            assert_eq!(payload, json!({"probe": "durable"}));
        }

        /// Cross-instance cancel: the worker holds the lease on instance A,
        /// the cancel arrives via instance B — pollers see `cancelled`
        /// immediately, A's worker observes `cancel_requested`, and its
        /// terminal writes are refused (a cancelled job cannot be
        /// resurrected).
        #[tokio::test]
        async fn postgres_integration_cluster_jobs_cancel_via_other_handle_reaches_worker() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let tenant = create_tenant("cluster-jobs-cancel");

            drain_cluster_jobs(a.as_ref(), JobKind::SofExport).await;

            let job_id = a
                .enqueue(&tenant, JobKind::SofExport, json!({"probe": "cancel"}))
                .await
                .unwrap();
            let (lease, _) = a
                .claim_next(
                    JobKind::SofExport,
                    &WorkerId::new(format!("cluster-cancel-{}", uuid::Uuid::new_v4())),
                    StdDuration::from_secs(60),
                )
                .await
                .unwrap()
                .expect("instance A claims the queued job");
            assert!(!a.cancel_requested(&lease).await.unwrap());

            // Cancel arrives on the other instance; wrong tenant is a no-op.
            assert!(
                !b.cancel(&create_tenant("cluster-jobs-cancel-other"), &job_id)
                    .await
                    .unwrap()
            );
            assert!(b.cancel(&tenant, &job_id).await.unwrap());

            let rec = b.get_status(&tenant, &job_id).await.unwrap().unwrap();
            assert_eq!(rec.state, ClusterJobState::Cancelled);

            assert!(a.cancel_requested(&lease).await.unwrap());
            assert!(matches!(
                a.complete(&lease, json!({})).await,
                Err(ClusterLeaseError::LeaseLost { .. })
            ));
        }
    }

    mod reindex_cluster_suite {
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        use serde_json::json;

        use helios_fhir::FhirVersion;
        use helios_persistence::backends::postgres::PostgresBackend;
        use helios_persistence::core::ResourceStorage;
        use helios_persistence::core::cluster_job_store::{JobKind, WorkerId};
        use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};
        use helios_persistence::tenant::TenantContext;

        use super::cluster_jobs_suite::{drain_cluster_jobs, store_of};
        use super::{CLUSTER_JOBS_TEST_LOCK, create_backend, create_tenant};
        use crate::cluster_harness as harness;

        /// One simulated instance: a fresh backend handle and a reindex
        /// driver wired the way `hfs` wires it in cluster mode (the job
        /// store off the same backend's `cluster_job_store()` seam).
        async fn instance() -> (Arc<PostgresBackend>, ReindexOperation) {
            let backend = Arc::new(create_backend().await);
            let store = store_of(backend.as_ref());
            let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone())
                .with_cluster_store(store);
            (backend, op)
        }

        async fn seed_patients(backend: &PostgresBackend, tenant: &TenantContext, n: usize) {
            for i in 0..n {
                backend
                    .create(
                        tenant,
                        "Patient",
                        json!({"resourceType": "Patient", "name": [{"family": format!("Reindex{i}")}]}),
                        FhirVersion::default(),
                    )
                    .await
                    .expect("seed patient");
            }
        }

        /// DoD rows (A2): visibility + isolation — a reindex started via
        /// instance A is pollable (status and list) via instance B and
        /// invisible to another tenant.
        #[tokio::test]
        async fn postgres_integration_cluster_reindex_visible_and_isolated_across_handles() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let (_backend_a, op_a) = instance().await;
            let (_backend_b, op_b) = instance().await;
            let tenant = create_tenant("cluster-reindex-vis");

            let job_id = op_a
                .start(
                    tenant.clone(),
                    ReindexRequest::for_types(vec!["Patient"]),
                    None,
                )
                .await
                .unwrap();

            let progress = harness::assert_visible(
                op_b.get_progress(&tenant, &job_id).await,
                "reindex job started via instance A",
            );
            assert_eq!(progress.status, ReindexStatus::Queued);

            assert_eq!(op_b.list_jobs(&tenant).await.len(), 1);

            harness::assert_wrong_tenant_hidden(
                op_b.get_progress(&create_tenant("cluster-reindex-vis-other"), &job_id)
                    .await,
                "reindex job",
            );
            assert!(
                op_b.list_jobs(&create_tenant("cluster-reindex-vis-other"))
                    .await
                    .is_empty(),
                "tenant isolation violated: reindex job listed under the wrong tenant"
            );
        }

        /// Cross-instance execution: started via A, claimed and run to
        /// completion by B's worker (one deterministic cycle), completion
        /// visible from A with real progress counters.
        #[tokio::test]
        async fn postgres_integration_cluster_reindex_runs_on_other_instance() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let (backend_a, op_a) = instance().await;
            let (_backend_b, op_b) = instance().await;
            let tenant = create_tenant("cluster-reindex-run");

            drain_cluster_jobs(store_of(backend_a.as_ref()).as_ref(), JobKind::Reindex).await;
            seed_patients(&backend_a, &tenant, 2).await;

            let job_id = op_a
                .start(
                    tenant.clone(),
                    ReindexRequest::for_types(vec!["Patient"]),
                    None,
                )
                .await
                .unwrap();

            let ran = op_b
                .run_next_cluster_job(
                    &WorkerId::new(format!("reindex-t2-{}", uuid::Uuid::new_v4())),
                    StdDuration::from_secs(60),
                )
                .await
                .expect("worker cycle");
            assert!(ran, "B's worker must claim the queued reindex job");

            let progress = op_a
                .get_progress(&tenant, &job_id)
                .await
                .expect("job visible via A after completion");
            assert_eq!(
                progress.status,
                ReindexStatus::Completed,
                "got {progress:?}"
            );
            assert_eq!(progress.processed_resources, 2);
            assert!(progress.entries_created > 0);
        }

        /// Cross-instance cancel: started via A, cancelled via B before any
        /// worker runs — pollers see `Cancelled` everywhere and the job is
        /// no longer claimable.
        #[tokio::test]
        async fn postgres_integration_cluster_reindex_cancel_via_other_instance() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let (backend_a, op_a) = instance().await;
            let (_backend_b, op_b) = instance().await;
            let tenant = create_tenant("cluster-reindex-cancel");

            drain_cluster_jobs(store_of(backend_a.as_ref()).as_ref(), JobKind::Reindex).await;

            let job_id = op_a
                .start(
                    tenant.clone(),
                    ReindexRequest::for_types(vec!["Patient"]),
                    None,
                )
                .await
                .unwrap();

            // Wrong tenant cannot cancel (indistinguishable from missing).
            assert!(
                op_b.cancel(&create_tenant("cluster-reindex-cancel-other"), &job_id)
                    .await
                    .is_err()
            );
            op_b.cancel(&tenant, &job_id).await.unwrap();

            let progress = op_a
                .get_progress(&tenant, &job_id)
                .await
                .expect("cancelled job still pollable");
            assert_eq!(progress.status, ReindexStatus::Cancelled);

            let ran = op_a
                .run_next_cluster_job(
                    &WorkerId::new(format!("reindex-t2-{}", uuid::Uuid::new_v4())),
                    StdDuration::from_secs(60),
                )
                .await
                .expect("worker cycle");
            assert!(!ran, "a cancelled reindex job must not be claimable");
        }

        /// DoD row (A2): durability — the job survives its starting handle.
        #[tokio::test]
        async fn postgres_integration_cluster_reindex_survives_handle_drop() {
            let _guard = CLUSTER_JOBS_TEST_LOCK.lock().await;
            let tenant = create_tenant("cluster-reindex-durable");

            let (backend_a, op_a) = instance().await;
            drain_cluster_jobs(store_of(backend_a.as_ref()).as_ref(), JobKind::Reindex).await;
            let job_id = op_a
                .start(
                    tenant.clone(),
                    ReindexRequest::for_types(vec!["Patient"]),
                    None,
                )
                .await
                .unwrap();
            drop(op_a);
            drop(backend_a);

            let (_backend, op) = instance().await;
            let progress = harness::assert_visible(
                op.get_progress(&tenant, &job_id).await,
                "reindex job after its starting handle was dropped",
            );
            assert_eq!(progress.status, ReindexStatus::Queued);
        }
    }

    // ========================================================================
    // T2 cluster suite — coordinated refresh store (C2 substrate)
    // ========================================================================
    //
    // The `cluster_refresh_cache` table + `ClusterRefreshCache` back the
    // cross-instance single-flight JWKS refresh. The store is deliberately
    // server-global — documents are public upstream material (IdP JWKS keys)
    // shared by every tenant — so the mandatory wrong-tenant DoD row is N/A
    // here (methodology §6); there is no tenant dimension to isolate.
    //
    // No `CLUSTER_JOBS_TEST_LOCK` needed: unlike the cross-tenant
    // `claim_next` queue, refresh keys are per-test UUIDs, so parallel tests
    // cannot interfere.
    // ========================================================================

    mod cluster_refresh_cache_suite {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration as StdDuration;

        use helios_persistence::core::ResourceStorage;
        use helios_persistence::core::cluster_refresh_cache::{
            ClusterRefreshCache, FetchFn, FetchedDocument, RefreshCacheError,
        };

        use super::create_backend;
        use crate::cluster_harness as harness;

        const HOUR: StdDuration = StdDuration::from_secs(3600);

        /// The server-facing seam: the store comes off the backend the same
        /// way `cluster_job_store()` does.
        fn store_of(backend: &impl ResourceStorage) -> Arc<dyn ClusterRefreshCache> {
            backend
                .cluster_refresh_cache()
                .expect("postgres backs a cluster refresh cache")
        }

        fn test_key(label: &str) -> String {
            format!("https://idp.example/{label}/{}/jwks", uuid::Uuid::new_v4())
        }

        /// A fetch closure returning `body` and counting invocations across
        /// instances via a shared counter.
        fn counting_fetch(body: &str, hits: &Arc<AtomicUsize>) -> FetchFn {
            let body = body.to_string();
            let hits = Arc::clone(hits);
            Box::new(move || {
                Box::pin(async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Ok(FetchedDocument {
                        body,
                        max_age_secs: Some(600),
                    })
                })
            })
        }

        /// DoD row: exclusivity (single flight) — two instances racing a
        /// refresh of the same key run exactly one upstream fetch between
        /// them, and both end up holding the same document.
        #[tokio::test]
        async fn postgres_integration_cluster_refresh_single_flight_across_handles() {
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let key = test_key("single-flight");
            let hits = Arc::new(AtomicUsize::new(0));

            let (key_a, key_b) = (key.clone(), key.clone());
            let (fetch_a, fetch_b) = (
                counting_fetch("doc-from-a", &hits),
                counting_fetch("doc-from-b", &hits),
            );
            let (res_a, res_b) = harness::race2(
                async move { a.refresh_with(&key_a, None, HOUR, fetch_a).await },
                async move { b.refresh_with(&key_b, None, HOUR, fetch_b).await },
            )
            .await;

            let doc_a = res_a.expect("instance A refresh succeeds");
            let doc_b = res_b.expect("instance B refresh succeeds");
            assert_eq!(
                hits.load(Ordering::SeqCst),
                1,
                "exactly one upstream fetch across both instances"
            );
            assert_eq!(
                doc_a.body, doc_b.body,
                "both instances hold the winner's document"
            );
            assert_eq!(doc_a.fetched_at, doc_b.fetched_at);
        }

        /// DoD rows: visibility + durability — a document stored via
        /// instance A is reused (no fetch) by a freshly constructed instance
        /// that shares nothing but the database.
        #[tokio::test]
        async fn postgres_integration_cluster_refresh_durable_across_handles() {
            let handles = harness::two_handles(create_backend).await;
            let key = test_key("durability");
            let hits = Arc::new(AtomicUsize::new(0));

            let stored = store_of(&handles.a)
                .refresh_with(&key, None, HOUR, counting_fetch("doc-v1", &hits))
                .await
                .unwrap();
            drop(handles);

            let fresh = create_backend().await;
            let reused = store_of(&fresh)
                .refresh_with(&key, None, HOUR, counting_fetch("doc-v2", &hits))
                .await
                .unwrap();
            assert_eq!(reused.body, "doc-v1");
            assert_eq!(reused.fetched_at, stored.fetched_at);
            assert_eq!(
                hits.load(Ordering::SeqCst),
                1,
                "the fresh instance reused, not fetched"
            );
        }

        /// Watermark row: an instance that already holds the stored document
        /// (watermark == fetched_at) forces a genuine refetch; an instance
        /// still on the older watermark then reuses the newer document.
        #[tokio::test]
        async fn postgres_integration_cluster_refresh_watermark_forces_refetch() {
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let key = test_key("watermark");
            let hits = Arc::new(AtomicUsize::new(0));

            let first = a
                .refresh_with(&key, None, HOUR, counting_fetch("doc-v1", &hits))
                .await
                .unwrap();

            // B holds v1 too (same watermark) and asks for something newer.
            let second = b
                .refresh_with(
                    &key,
                    Some(first.fetched_at),
                    HOUR,
                    counting_fetch("doc-v2", &hits),
                )
                .await
                .unwrap();
            assert_eq!(second.body, "doc-v2");
            assert!(second.fetched_at > first.fetched_at);
            assert_eq!(hits.load(Ordering::SeqCst), 2);

            // A, still on the v1 watermark, sees v2 as newer and reuses it.
            let third = a
                .refresh_with(
                    &key,
                    Some(first.fetched_at),
                    HOUR,
                    counting_fetch("doc-v3", &hits),
                )
                .await
                .unwrap();
            assert_eq!(third.body, "doc-v2");
            assert_eq!(hits.load(Ordering::SeqCst), 2, "reuse must not fetch");
        }

        /// Abort row: a fetch failure on instance A surfaces as `Fetch`,
        /// stores nothing, and releases the advisory lock — instance B's
        /// immediately following refresh fetches successfully.
        #[tokio::test]
        async fn postgres_integration_cluster_refresh_fetch_error_releases_lock() {
            let handles = harness::two_handles(create_backend).await;
            let (a, b) = (store_of(&handles.a), store_of(&handles.b));
            let key = test_key("abort");
            let hits = Arc::new(AtomicUsize::new(0));

            let failing: FetchFn =
                Box::new(|| Box::pin(async { Err("idp unreachable".to_string()) }));
            let err = a.refresh_with(&key, None, HOUR, failing).await.unwrap_err();
            assert!(
                matches!(err, RefreshCacheError::Fetch(ref m) if m == "idp unreachable"),
                "fetch failure surfaces as RefreshCacheError::Fetch"
            );

            // The key is not poisoned and the lock is free: B fetches.
            let doc = b
                .refresh_with(&key, None, HOUR, counting_fetch("doc-after-error", &hits))
                .await
                .unwrap();
            assert_eq!(doc.body, "doc-after-error");
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    // ========================================================================
    // T2 cluster suite — subscriptions substrate (Phase 3, strategy §8;
    // B2 tokens / B4 counters / B5 outbox / B1 fan-out / B3 hydration).
    //
    // Every test drives two freshly constructed handles that share only the
    // database (methodology §6). State/token/hydration tests need no lock —
    // per-test unique tenants keep their rows disjoint; the outbox suite
    // takes SUBSCRIPTION_OUTBOX_TEST_LOCK because `claim_next` is
    // deliberately cross-tenant.
    // ========================================================================
    mod subscription_cluster_suite {
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        use serde_json::json;
        use tokio::sync::Mutex;
        use tokio::time::timeout;

        use helios_fhir::FhirVersion;
        use helios_persistence::backends::postgres::PostgresBackend;
        use helios_persistence::core::ResourceStorage;
        use helios_persistence::core::event_fanout::{FanoutEnvelope, FanoutKind, LifecycleOp};
        use helios_persistence::core::subscription_delivery::{
            DeliveryLeaseError, DeliveryState, NewDelivery, SubscriptionDeliveryOutbox, WorkerId,
        };
        use helios_persistence::core::subscription_state::SubscriptionStateStore;
        use helios_persistence::core::ws_binding_tokens::WsBindingTokenStore;

        use super::{create_backend, create_tenant};
        use crate::cluster_harness as harness;

        static SUBSCRIPTION_OUTBOX_TEST_LOCK: Mutex<()> = Mutex::const_new(());

        /// How long an event-driven await may take before the test fails —
        /// generous for a loaded CI Docker host, never slept in full.
        const RECV_TIMEOUT: StdDuration = StdDuration::from_secs(30);

        fn state_of(backend: &PostgresBackend) -> Arc<dyn SubscriptionStateStore> {
            backend
                .subscription_state_store()
                .expect("postgres backs a subscription state store")
        }

        fn tokens_of(backend: &PostgresBackend) -> Arc<dyn WsBindingTokenStore> {
            backend
                .ws_binding_token_store()
                .expect("postgres backs a ws binding token store")
        }

        fn outbox_of(backend: &PostgresBackend) -> Arc<dyn SubscriptionDeliveryOutbox> {
            backend
                .subscription_delivery_outbox()
                .expect("postgres backs a delivery outbox")
        }

        fn delivery(sub: &str) -> NewDelivery {
            NewDelivery {
                subscription_id: sub.to_string(),
                event_number: Some(1),
                notification_type: "event-notification".to_string(),
                bundle: json!({"resourceType": "Bundle", "type": "subscription-notification"}),
            }
        }

        /// DoD row (B4): exclusivity/atomicity — two instances racing the
        /// event-number increment observe distinct consecutive values, so
        /// `eventNumber` stays monotonic and gap-free cluster-wide.
        #[tokio::test]
        async fn postgres_integration_cluster_subscription_state_increment_race_gap_free() {
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("sub-state-race");
            let (a, b) = (state_of(&handles.a), state_of(&handles.b));

            let (tenant_a, tenant_b) = (tenant.clone(), tenant.clone());
            let (got_a, got_b) = harness::race2(
                async move { a.next_event_number(&tenant_a, "sub-1").await.unwrap() },
                async move { b.next_event_number(&tenant_b, "sub-1").await.unwrap() },
            )
            .await;

            let mut numbers = [got_a, got_b];
            numbers.sort_unstable();
            assert_eq!(
                numbers,
                [1, 2],
                "racing increments must observe distinct consecutive values"
            );
        }

        /// DoD rows (B4): visibility + isolation — failures scattered across
        /// instances accumulate in one row; a status flip via A is observed
        /// via B; another tenant sees nothing.
        #[tokio::test]
        async fn postgres_integration_cluster_subscription_state_shared_and_isolated() {
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("sub-state-vis");
            let (a, b) = (state_of(&handles.a), state_of(&handles.b));

            // Failures scattered across instances still accumulate (the
            // "dead endpoint never reaches the off threshold" disease).
            assert_eq!(a.record_failure(&tenant, "sub-1").await.unwrap(), 1);
            assert_eq!(b.record_failure(&tenant, "sub-1").await.unwrap(), 2);
            assert_eq!(a.record_failure(&tenant, "sub-1").await.unwrap(), 3);

            a.set_status(&tenant, "sub-1", "error").await.unwrap();
            let seen_via_b = harness::assert_visible(
                b.get(&tenant, "sub-1").await.unwrap(),
                "state row written via instance A",
            );
            assert_eq!(seen_via_b.consecutive_failures, 3);
            assert_eq!(seen_via_b.status.as_deref(), Some("error"));

            // A success via B resets the shared counter for both.
            b.reset_failures(&tenant, "sub-1").await.unwrap();
            assert_eq!(
                a.get(&tenant, "sub-1")
                    .await
                    .unwrap()
                    .unwrap()
                    .consecutive_failures,
                0
            );

            let other_tenant = create_tenant("sub-state-other");
            harness::assert_wrong_tenant_hidden(
                b.get(&other_tenant, "sub-1").await.unwrap(),
                "subscription state row",
            );
        }

        /// DoD row (B4): durability — counters survive the writing handle.
        #[tokio::test]
        async fn postgres_integration_cluster_subscription_state_survives_handle_drop() {
            let tenant = create_tenant("sub-state-durable");
            {
                let writer = create_backend().await;
                let state = state_of(&writer);
                assert_eq!(state.next_event_number(&tenant, "sub-1").await.unwrap(), 1);
                assert_eq!(state.next_event_number(&tenant, "sub-1").await.unwrap(), 2);
            } // writer dropped

            let fresh = create_backend().await;
            let state = state_of(&fresh);
            assert_eq!(
                state.next_event_number(&tenant, "sub-1").await.unwrap(),
                3,
                "a fresh instance continues the sequence, no reset"
            );
        }

        /// DoD rows (B1 substrate): a websocket notification bundle stored
        /// via instance A is loadable via instance B by its fan-out key, and
        /// invisible to another tenant.
        #[tokio::test]
        async fn postgres_integration_cluster_notification_events_roundtrip_across_handles() {
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("sub-events");
            let (a, b) = (state_of(&handles.a), state_of(&handles.b));

            let bundle = json!({
                "resourceType": "Bundle",
                "type": "subscription-notification",
                "entry": [{"request": {"url": "Encounter/enc-1"}}]
            });
            a.put_notification_event(&tenant, "sub-1", 42, &bundle)
                .await
                .unwrap();

            let loaded = harness::assert_visible(
                b.get_notification_event(&tenant, "sub-1", 42)
                    .await
                    .unwrap(),
                "notification bundle stored via instance A",
            );
            assert_eq!(loaded, bundle);

            let other_tenant = create_tenant("sub-events-other");
            harness::assert_wrong_tenant_hidden(
                b.get_notification_event(&other_tenant, "sub-1", 42)
                    .await
                    .unwrap(),
                "notification bundle",
            );

            // Subscription delete via B removes the stored bundles too.
            b.delete(&tenant, "sub-1").await.unwrap();
            assert!(
                a.get_notification_event(&tenant, "sub-1", 42)
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        /// DoD row (B2): exclusivity — mint on A, redeem exactly once on B;
        /// the second redeem (any instance) fails. A zero-TTL token is
        /// expired on arrival (DB clock — no sleeps).
        #[tokio::test]
        async fn postgres_integration_cluster_ws_token_mint_on_a_redeem_once_on_b() {
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("ws-token");
            let (a, b) = (tokens_of(&handles.a), tokens_of(&handles.b));

            let (token, expires_at) = a
                .mint(&tenant, "sub-1", StdDuration::from_secs(30))
                .await
                .unwrap();
            assert!(expires_at > chrono::Utc::now());

            let redeemed = b.redeem(&token).await.unwrap();
            assert_eq!(
                redeemed,
                Some((tenant.tenant_id().as_str().to_string(), "sub-1".to_string())),
                "a token minted via A must redeem via B"
            );
            assert!(
                a.redeem(&token).await.unwrap().is_none(),
                "second redeem must fail on every instance"
            );

            // Expired-on-arrival: zero TTL, judged on the database clock.
            let (expired, _) = a.mint(&tenant, "sub-1", StdDuration::ZERO).await.unwrap();
            assert!(
                b.redeem(&expired).await.unwrap().is_none(),
                "an expired token must not redeem"
            );
        }

        /// DoD row (B5): exclusivity — two instances race `claim_next` on
        /// one queued delivery and exactly one wins.
        #[tokio::test]
        async fn postgres_integration_cluster_outbox_claim_exclusive_across_handles() {
            let _guard = SUBSCRIPTION_OUTBOX_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("outbox-claim");

            outbox_of(&handles.a)
                .enqueue(&tenant, delivery("sub-1"))
                .await
                .unwrap();

            let worker_a = WorkerId::new(format!("outbox-a-{}", uuid::Uuid::new_v4()));
            let worker_b = WorkerId::new(format!("outbox-b-{}", uuid::Uuid::new_v4()));
            let (a, b) = (outbox_of(&handles.a), outbox_of(&handles.b));
            let (claim_a, claim_b) = harness::race2(
                async move {
                    a.claim_next(&worker_a, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
                async move {
                    b.claim_next(&worker_b, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
            )
            .await;

            harness::assert_exactly_one(&claim_a, &claim_b, "the claim on one queued delivery");
            let winner = claim_a.or(claim_b).unwrap();
            assert_eq!(winner.subscription_id, "sub-1");
            assert_eq!(winner.attempts, 1, "the claim is the first attempt");
            assert_eq!(
                winner.lease.tenant.tenant_id().as_str(),
                tenant.tenant_id().as_str(),
                "the lease carries the enqueueing tenant"
            );

            // Leave nothing claimable for the next test.
            outbox_of(&handles.b).complete(&winner.lease).await.unwrap();
        }

        /// DoD rows (B5): durability + fencing — a delivery claimed by a
        /// worker whose lease expires (and whose handle is dropped) is
        /// reclaimed by a fresh instance under a bumped fencing token; the
        /// zombie's terminal write is refused. Deterministic: the "expiry"
        /// is a zero-length lease, no sleeps.
        #[tokio::test]
        async fn postgres_integration_cluster_outbox_reclaimed_after_worker_death() {
            let _guard = SUBSCRIPTION_OUTBOX_TEST_LOCK.lock().await;
            let tenant = create_tenant("outbox-durable");

            let stale = {
                let dying = create_backend().await;
                let outbox = outbox_of(&dying);
                outbox.enqueue(&tenant, delivery("sub-1")).await.unwrap();
                let worker = WorkerId::new(format!("outbox-dying-{}", uuid::Uuid::new_v4()));
                outbox
                    .claim_next(&worker, StdDuration::ZERO)
                    .await
                    .unwrap()
                    .expect("the enqueued delivery is claimable")
            }; // the claiming handle is dropped mid-delivery

            let fresh = create_backend().await;
            let outbox = outbox_of(&fresh);
            let worker = WorkerId::new(format!("outbox-fresh-{}", uuid::Uuid::new_v4()));
            let reclaimed = outbox
                .claim_next(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .expect("the expired-lease delivery must be reclaimable");
            assert_eq!(reclaimed.lease.id, stale.lease.id);
            assert_eq!(reclaimed.attempts, 2, "the reclaim is a new attempt");
            assert!(reclaimed.lease.fencing_token > stale.lease.fencing_token);

            // The dead worker's lease is fenced out of every terminal write.
            assert!(matches!(
                outbox.complete(&stale.lease).await,
                Err(DeliveryLeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                outbox.fail(&stale.lease, "zombie").await,
                Err(DeliveryLeaseError::LeaseLost { .. })
            ));

            outbox.complete(&reclaimed.lease).await.unwrap();
            let record = outbox
                .get(&tenant, reclaimed.lease.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.state, DeliveryState::Delivered);
        }

        /// DoD rows (B5): retry scheduling + isolation — a retryable failure
        /// re-queues with a future due time (not claimable until then, on
        /// the DB's schedule column), and another tenant cannot read the
        /// row.
        #[tokio::test]
        async fn postgres_integration_cluster_outbox_retry_schedule_and_isolation() {
            let _guard = SUBSCRIPTION_OUTBOX_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("outbox-retry");
            let (a, b) = (outbox_of(&handles.a), outbox_of(&handles.b));

            let id = a.enqueue(&tenant, delivery("sub-1")).await.unwrap();
            let worker = WorkerId::new(format!("outbox-retry-{}", uuid::Uuid::new_v4()));
            let claimed = a
                .claim_next(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();

            let next_attempt = chrono::Utc::now() + chrono::Duration::seconds(3600);
            a.release_for_retry(&claimed.lease, next_attempt, "503 from endpoint")
                .await
                .unwrap();

            // Visible via B with the recorded error and schedule...
            let record = harness::assert_visible(
                b.get(&tenant, id).await.unwrap(),
                "outbox row released via instance A",
            );
            assert_eq!(record.state, DeliveryState::Queued);
            assert_eq!(record.attempts, 1);
            assert_eq!(record.last_error.as_deref(), Some("503 from endpoint"));

            // ...but not claimable before its due time.
            let early_worker = WorkerId::new(format!("outbox-early-{}", uuid::Uuid::new_v4()));
            assert!(
                b.claim_next(&early_worker, StdDuration::from_secs(60))
                    .await
                    .unwrap()
                    .is_none(),
                "a future next_attempt_at must not be claimable"
            );

            let other_tenant = create_tenant("outbox-retry-other");
            harness::assert_wrong_tenant_hidden(
                b.get(&other_tenant, id).await.unwrap(),
                "outbox row",
            );
            // The row stays queued an hour out — invisible to every other
            // outbox test in this binary's lifetime, no drain needed.
        }

        /// DoD row (B1): an envelope published via instance A's fan-out
        /// reaches instance B's subscriber over LISTEN/NOTIFY — two fully
        /// independent fan-outs sharing only the database. Event-driven
        /// await under a timeout, no sleeps.
        #[tokio::test]
        async fn postgres_integration_cluster_fanout_publish_reaches_other_handle() {
            let handles = harness::two_handles(create_backend).await;
            let fanout_a = handles
                .a
                .subscription_fanout()
                .expect("postgres backs a fan-out");
            let fanout_b = handles
                .b
                .subscription_fanout()
                .expect("postgres backs a fan-out");

            let mut envelopes_b = fanout_b.subscribe();
            let mut wakes_b = fanout_b.subscribe_outbox_wake();
            // B must be LISTENing before A publishes, or the NOTIFY is
            // (correctly, by contract) lost.
            timeout(RECV_TIMEOUT, fanout_b.ready())
                .await
                .expect("instance B's LISTEN session must establish");

            let envelope = FanoutEnvelope::new(FanoutKind::Lifecycle {
                tenant: "tenant-fanout".into(),
                rtype: "Subscription".into(),
                rid: "sub-1".into(),
                op: LifecycleOp::Upsert,
            });
            fanout_a.publish(&envelope).await.unwrap();
            let received = timeout(RECV_TIMEOUT, envelopes_b.recv())
                .await
                .expect("envelope must arrive on instance B")
                .unwrap();
            assert_eq!(received, envelope);

            fanout_a.publish_outbox_wake().await.unwrap();
            timeout(RECV_TIMEOUT, wakes_b.recv())
                .await
                .expect("wake hint must arrive on instance B")
                .unwrap();
        }

        /// DoD rows (B3): hydration enumerates current subscription/topic
        /// resources across ALL tenants — including tenants never touched by
        /// the tenant registry — pre-filters `Basic` to backport topics, and
        /// tenant-checks the single-resource read.
        #[tokio::test]
        async fn postgres_integration_cluster_hydration_lists_across_tenants() {
            let handles = harness::two_handles(create_backend).await;
            let tenant_a = create_tenant("hydrate-a");
            let tenant_b = create_tenant("hydrate-b");

            handles
                .a
                .create(
                    &tenant_a,
                    "SubscriptionTopic",
                    json!({
                        "resourceType": "SubscriptionTopic",
                        "id": "topic-1",
                        "url": "http://example.org/topics/encounter",
                        "status": "active"
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            handles
                .a
                .create(
                    &tenant_b,
                    "Subscription",
                    json!({
                        "resourceType": "Subscription",
                        "id": "sub-1",
                        "status": "requested"
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            // A backport topic Basic (must be listed)...
            handles
                .a
                .create(
                    &tenant_a,
                    "Basic",
                    json!({
                        "resourceType": "Basic",
                        "id": "basic-topic",
                        "code": {"coding": [{
                            "system": "http://hl7.org/fhir/fhir-types",
                            "code": "SubscriptionTopic"
                        }]}
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            // ...and an unrelated Basic (must be pre-filtered out in SQL).
            handles
                .a
                .create(
                    &tenant_a,
                    "Basic",
                    json!({
                        "resourceType": "Basic",
                        "id": "basic-unrelated",
                        "code": {"coding": [{
                            "system": "http://example.org/other",
                            "code": "SomethingElse"
                        }]}
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();

            // Enumerate via the OTHER instance (shared-DB visibility).
            let source = handles
                .b
                .subscription_hydration_source()
                .expect("postgres backs a hydration source");
            let listed = source
                .list_current(&["Subscription", "SubscriptionTopic", "Basic"])
                .await
                .unwrap();

            let ours: Vec<_> = listed
                .iter()
                .filter(|r| {
                    r.tenant_id == tenant_a.tenant_id().as_str()
                        || r.tenant_id == tenant_b.tenant_id().as_str()
                })
                .collect();
            assert_eq!(
                ours.len(),
                3,
                "topic + subscription + backport Basic; unrelated Basic filtered in SQL"
            );
            assert!(
                ours.iter().any(|r| r.resource_type == "Subscription"
                    && r.tenant_id == tenant_b.tenant_id().as_str()),
                "hydration must cross tenants (registry-independent)"
            );
            assert!(
                !ours.iter().any(|r| r.resource_id == "basic-unrelated"),
                "a non-topic Basic must not be hauled into memory"
            );
            for resource in &ours {
                assert!(
                    !resource.fhir_version.is_empty(),
                    "hydrated rows carry their FHIR version for parser selection"
                );
            }

            // Tenant-checked single read (the lifecycle re-read path).
            let read = harness::assert_visible(
                source
                    .get_current(&tenant_a, "SubscriptionTopic", "topic-1")
                    .await
                    .unwrap(),
                "topic resource read back for its owner",
            );
            assert_eq!(read.content["url"], "http://example.org/topics/encounter");
            harness::assert_wrong_tenant_hidden(
                source
                    .get_current(&tenant_b, "SubscriptionTopic", "topic-1")
                    .await
                    .unwrap(),
                "another tenant's topic resource",
            );
        }
    }

    // ========================================================================
    // Phase 4 (E1): durable composite secondary-backend sync outbox T2 suite.
    //
    // Same two-fresh-handles-over-one-shared-store contract as the other
    // cluster suites. `claim_next` is cross-tenant, so the exclusivity/
    // fencing/retry tests take COMPOSITE_SYNC_OUTBOX_TEST_LOCK.
    // ========================================================================
    mod composite_sync_cluster_suite {
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        use helios_fhir::FhirVersion;
        use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
        use helios_persistence::composite::{CompositeConfig, CompositeStorage, SyncMode};
        use helios_persistence::core::composite_sync_outbox::{
            CompositeSyncOutbox, NewSyncOutboxEntry, SyncLeaseError, SyncOperation,
            SyncOutboxState, WorkerId,
        };
        use helios_persistence::core::{BackendKind, ResourceStorage};
        use serde_json::json;
        use tokio::sync::Mutex;

        use super::{create_backend, create_tenant, shared_pg};
        use crate::cluster_harness as harness;

        static COMPOSITE_SYNC_OUTBOX_TEST_LOCK: Mutex<()> = Mutex::const_new(());

        fn outbox_of(backend: &PostgresBackend) -> Arc<dyn CompositeSyncOutbox> {
            backend
                .composite_sync_outbox()
                .expect("postgres backs a composite sync outbox")
        }

        fn entry(backend_id: &str, resource_id: &str) -> NewSyncOutboxEntry {
            NewSyncOutboxEntry {
                backend_id: backend_id.to_string(),
                operation: SyncOperation::Create,
                resource_type: "Patient".to_string(),
                resource_id: resource_id.to_string(),
                content: Some(json!({"resourceType": "Patient", "id": resource_id})),
                version: None,
                tenant_id: "t1".to_string(),
                fhir_version: Some("R4".to_string()),
            }
        }

        /// DoD row: exclusivity — two instances race `claim_next` on one
        /// queued entry and exactly one wins.
        #[tokio::test]
        async fn postgres_integration_cluster_composite_sync_claim_exclusive_across_handles() {
            let _guard = COMPOSITE_SYNC_OUTBOX_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("composite-sync-claim");

            outbox_of(&handles.a)
                .enqueue(&tenant, entry("es", "p1"))
                .await
                .unwrap();

            let worker_a = WorkerId::new(format!("csync-a-{}", uuid::Uuid::new_v4()));
            let worker_b = WorkerId::new(format!("csync-b-{}", uuid::Uuid::new_v4()));
            let (a, b) = (outbox_of(&handles.a), outbox_of(&handles.b));
            let (claim_a, claim_b) = harness::race2(
                async move {
                    a.claim_next(&worker_a, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
                async move {
                    b.claim_next(&worker_b, StdDuration::from_secs(60))
                        .await
                        .unwrap()
                },
            )
            .await;

            harness::assert_exactly_one(&claim_a, &claim_b, "the claim on one queued entry");
            let winner = claim_a.or(claim_b).unwrap();
            assert_eq!(winner.backend_id, "es");
            assert_eq!(winner.resource_id, "p1");
            assert_eq!(winner.attempts, 1, "the claim is the first attempt");
            assert_eq!(
                winner.lease.tenant.tenant_id().as_str(),
                tenant.tenant_id().as_str(),
                "the lease carries the enqueueing tenant"
            );

            // Leave nothing claimable for the next test.
            outbox_of(&handles.b).complete(&winner.lease).await.unwrap();
        }

        /// DoD rows: durability + fencing — an entry claimed by a worker
        /// whose lease expires (and whose handle is dropped) is reclaimed by
        /// a fresh instance under a bumped fencing token; the zombie's
        /// terminal write is refused. Deterministic: the "expiry" is a
        /// zero-length lease, no sleeps.
        #[tokio::test]
        async fn postgres_integration_cluster_composite_sync_reclaimed_after_worker_death() {
            let _guard = COMPOSITE_SYNC_OUTBOX_TEST_LOCK.lock().await;
            let tenant = create_tenant("composite-sync-durable");

            let stale = {
                let dying = create_backend().await;
                let outbox = outbox_of(&dying);
                outbox.enqueue(&tenant, entry("es", "p1")).await.unwrap();
                let worker = WorkerId::new(format!("csync-dying-{}", uuid::Uuid::new_v4()));
                outbox
                    .claim_next(&worker, StdDuration::ZERO)
                    .await
                    .unwrap()
                    .expect("the enqueued entry is claimable")
            }; // the claiming handle is dropped mid-apply

            let fresh = create_backend().await;
            let outbox = outbox_of(&fresh);
            let worker = WorkerId::new(format!("csync-fresh-{}", uuid::Uuid::new_v4()));
            let reclaimed = outbox
                .claim_next(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .expect("the expired-lease entry must be reclaimable");
            assert_eq!(reclaimed.lease.id, stale.lease.id);
            assert_eq!(reclaimed.attempts, 2, "the reclaim is a new attempt");
            assert!(reclaimed.lease.fencing_token > stale.lease.fencing_token);

            // The dead worker's lease is fenced out of every terminal write.
            assert!(matches!(
                outbox.complete(&stale.lease).await,
                Err(SyncLeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                outbox.fail(&stale.lease, "zombie").await,
                Err(SyncLeaseError::LeaseLost { .. })
            ));

            outbox.complete(&reclaimed.lease).await.unwrap();
            let record = outbox
                .get(&tenant, reclaimed.lease.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.state, SyncOutboxState::Applied);
        }

        /// DoD rows: retry scheduling + isolation — a retryable failure
        /// re-queues with a future due time (on the DB's schedule column,
        /// not claimable until then), and another tenant cannot read the
        /// row.
        #[tokio::test]
        async fn postgres_integration_cluster_composite_sync_retry_schedule_and_isolation() {
            let _guard = COMPOSITE_SYNC_OUTBOX_TEST_LOCK.lock().await;
            let handles = harness::two_handles(create_backend).await;
            let tenant = create_tenant("composite-sync-retry");
            let (a, b) = (outbox_of(&handles.a), outbox_of(&handles.b));

            let id = a.enqueue(&tenant, entry("es", "p1")).await.unwrap();
            let worker = WorkerId::new(format!("csync-retry-{}", uuid::Uuid::new_v4()));
            let claimed = a
                .claim_next(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();

            let next_attempt = chrono::Utc::now() + chrono::Duration::seconds(3600);
            a.release_for_retry(&claimed.lease, next_attempt, "connection refused")
                .await
                .unwrap();

            let record = harness::assert_visible(
                b.get(&tenant, id).await.unwrap(),
                "outbox row released via instance A",
            );
            assert_eq!(record.state, SyncOutboxState::Queued);
            assert_eq!(record.attempts, 1);
            assert_eq!(record.last_error.as_deref(), Some("connection refused"));

            let early_worker = WorkerId::new(format!("csync-early-{}", uuid::Uuid::new_v4()));
            assert!(
                b.claim_next(&early_worker, StdDuration::from_secs(60))
                    .await
                    .unwrap()
                    .is_none(),
                "a future next_attempt_at must not be claimable"
            );

            let other_tenant = create_tenant("composite-sync-retry-other");
            harness::assert_wrong_tenant_hidden(
                b.get(&other_tenant, id).await.unwrap(),
                "outbox row",
            );
            // The row stays queued an hour out — invisible to every other
            // outbox test in this binary's lifetime, no drain needed.
        }

        /// End-to-end wiring: two independently constructed `CompositeStorage`
        /// handles (Postgres primary + a genuinely separate "secondary"
        /// database standing in for Elasticsearch, so applying a create
        /// doesn't collide with the primary's own row) prove
        /// `sync_asynchronous` actually durably enqueues — a write via A
        /// survives without A ever running a worker — and a fresh handle's
        /// `run_next_composite_sync` (B) claims and applies it to its own
        /// secondary connection.
        #[tokio::test]
        async fn postgres_integration_cluster_composite_storage_durably_syncs_across_handles() {
            // CompositeStorage::new() wires the outbox unconditionally
            // (E1), so even without start_sync_workers() this test's
            // enqueue lands in the same cross-tenant-claimable table the
            // other tests in this suite race on.
            let _guard = COMPOSITE_SYNC_OUTBOX_TEST_LOCK.lock().await;
            let pg = shared_pg().await;
            let dbname = format!("composite_sync_secondary_{}", uuid::Uuid::new_v4().simple());
            let admin_conn = format!(
                "host={} port={} user=postgres password=postgres dbname=postgres",
                pg.host, pg.port,
            );
            let (admin, connection) = tokio_postgres::connect(&admin_conn, tokio_postgres::NoTls)
                .await
                .expect("connect to shared pg");
            tokio::spawn(async move {
                let _ = connection.await;
            });
            admin
                .execute(format!("CREATE DATABASE {}", dbname).as_str(), &[])
                .await
                .expect("create secondary database");

            let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("data"))
                .unwrap_or_else(|| PathBuf::from("data"));

            async fn secondary_backend(
                pg_host: &str,
                pg_port: u16,
                dbname: &str,
                data_dir: &std::path::Path,
            ) -> Arc<dyn ResourceStorage + Send + Sync> {
                let config = PostgresConfig {
                    host: pg_host.to_string(),
                    port: pg_port,
                    dbname: dbname.to_string(),
                    user: "postgres".to_string(),
                    password: Some("postgres".to_string()),
                    max_connections: 2,
                    data_dir: Some(data_dir.to_path_buf()),
                    ..Default::default()
                };
                let backend = PostgresBackend::new(config)
                    .await
                    .expect("secondary backend should connect");
                backend
                    .init_schema()
                    .await
                    .expect("secondary schema should initialize");
                Arc::new(backend) as Arc<dyn ResourceStorage + Send + Sync>
            }

            let build_composite = || async {
                let mut backends = std::collections::HashMap::new();
                backends.insert(
                    "primary".to_string(),
                    Arc::new(create_backend().await) as Arc<dyn ResourceStorage + Send + Sync>,
                );
                backends.insert(
                    "es".to_string(),
                    secondary_backend(&pg.host, pg.port, &dbname, &data_dir).await,
                );
                let config = CompositeConfig::builder()
                    .primary("primary", BackendKind::Postgres)
                    .search_backend("es", BackendKind::Elasticsearch)
                    .sync_mode(SyncMode::Asynchronous)
                    .build()
                    .unwrap();
                CompositeStorage::new(config, backends).unwrap()
            };

            // Handle A: enqueue only, no worker started — proves durability
            // does not depend on the writer's own instance running a worker.
            let composite_a = build_composite().await;
            let tenant = create_tenant("composite-storage-e2e");
            composite_a
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "sync-e2e-1"}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();

            // Handle B: a fresh instance drains the row A enqueued.
            let composite_b = build_composite().await;
            let worker_id = WorkerId::new(format!("csync-e2e-{}", uuid::Uuid::new_v4()));
            let mut drained = false;
            for _ in 0..20 {
                if composite_b
                    .run_next_composite_sync(&worker_id, StdDuration::from_secs(30))
                    .await
                {
                    drained = true;
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(50)).await;
            }
            assert!(drained, "B's worker cycle must claim the row A enqueued");

            // The secondary now genuinely has the resource — applied by B,
            // not A.
            let secondary_check = secondary_backend(&pg.host, pg.port, &dbname, &data_dir).await;
            let read = secondary_check
                .read(&tenant, "Patient", "sync-e2e-1")
                .await
                .unwrap();
            assert!(
                read.is_some(),
                "the secondary must have the resource after B's worker applied it"
            );
        }
    }

    // ========================================================================
    // Backend error handling — a reachable but misconfigured store
    // ========================================================================
    //
    // `tests/backend_error_handling.rs` covers every backend's *unreachable*
    // case with no server at all. PostgreSQL has a gap that cannot be closed
    // there: `PostgresBackend::new` eagerly verifies connectivity, so an
    // unreachable server fails at construction and the caller never obtains a
    // backend to drive. That leaves every per-operation error arm in
    // `postgres/storage.rs` — the `internal_error(..)` mapping behind `read`,
    // `count`, `create` and friends — completely unexercised, which is exactly
    // the class of uncovered code that motivated this work.
    //
    // Reaching those arms needs a server that answers but cannot serve the
    // query, so this test lives here, with the shared container. It mirrors the
    // SQLite `unmigrated_store_surfaces_backend_error` test: connect to a
    // database whose schema was never created (`new()` runs no DDL —
    // `init_schema()` is a separate, opt-in call) and drive the core operations.
    // Every one of them hits `relation "resources" does not exist`.

    /// Operations against a reachable database with no schema must surface a
    /// backend error — never a misleading success.
    #[tokio::test]
    async fn postgres_integration_unmigrated_store_surfaces_backend_error() {
        let pg = shared_pg().await;

        // A database of our own, so we can leave it unmigrated without disturbing
        // the schema the rest of this suite shares.
        let dbname = format!("unmigrated_{}", uuid::Uuid::new_v4().simple());
        let admin_conn = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port,
        );
        let (admin, connection) = tokio_postgres::connect(&admin_conn, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        // `batch_execute` uses the simple query protocol. `execute` would use the
        // extended one, which wraps the statement in an implicit transaction —
        // and CREATE DATABASE cannot run inside a transaction block.
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create an empty database");

        let backend = PostgresBackend::new(PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname,
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 2,
            ..Default::default()
        })
        .await
        .expect("the server is reachable, so construction must succeed");

        // init_schema() is deliberately NOT called.

        let tenant = create_tenant("unmigrated");

        let read = backend.read(&tenant, "Patient", "does-not-exist").await;
        assert!(
            !matches!(read, Ok(None)),
            "read against an unmigrated database returned Ok(None) — a store we \
             could not query must not be indistinguishable from one where the \
             resource is genuinely absent"
        );
        assert!(
            matches!(read, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {read:?}"
        );

        let count = backend.count(&tenant, Some("Patient")).await;
        assert!(
            !matches!(count, Ok(0)),
            "count against an unmigrated database returned Ok(0) — 'zero resources' \
             is a claim about data we never successfully queried"
        );
        assert!(
            matches!(count, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {count:?}"
        );

        let create = backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient" }),
                FhirVersion::default(),
            )
            .await;
        assert!(
            matches!(create, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {create:?}"
        );
    }
    /// The `import` / `metadata` kickoff directives must survive the PostgreSQL
    /// round-trip and reach the worker, and `merge` must actually merge.
    ///
    /// This is the Postgres half of the coverage in
    /// `core::bulk_submit_worker::tests` (which runs on SQLite): the plumbing is
    /// shared but the SQL is not, so a typo in the new columns would otherwise
    /// only surface in production.
    #[tokio::test]
    async fn postgres_bulk_submit_import_directives_round_trip() {
        use helios_persistence::core::{
            BulkProcessingOptions, BulkSubmitProvider, IMPORT_MODE_PARAMETER_URL, ImportMode,
            ManifestFetchParams, NdjsonEntry, SubmissionId, SubmitClaimStrategy,
            SubmitWorkerStorage,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("bulk_submit_import");
        let sub_id = SubmissionId::generate("pg-import-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://provider/m.json"), None)
            .await
            .unwrap();

        let directives = vec![(IMPORT_MODE_PARAMETER_URL.to_string(), "merge".to_string())];
        let metadata = vec![("https://ex/context".to_string(), "batch-7".to_string())];
        backend
            .set_manifest_fetch_params(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                ManifestFetchParams {
                    fhir_base_url: Some("https://provider/fhir"),
                    import_directives: &directives,
                    metadata: &metadata,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Claim the manifest the way the worker does, then read its view back.
        let lease = backend
            .claim_next_manifest(
                &helios_persistence::core::WorkerId::new("pg-import-worker"),
                std::time::Duration::from_secs(60),
            )
            .await
            .unwrap()
            .expect("claimable manifest");
        let view = backend.get_manifest_for_worker(&lease).await.unwrap();
        assert_eq!(view.import_directives, directives);
        assert_eq!(view.metadata, metadata);
        assert_eq!(
            ImportMode::from_directives(&view.import_directives),
            ImportMode::Merge
        );

        // And the resolved mode changes what ingestion writes.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "pg-merge-1",
                    "gender": "female",
                    "name": [{"family": "Stale"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let entry = NdjsonEntry::new(
            1,
            "Patient",
            json!({"resourceType": "Patient", "id": "pg-merge-1", "name": [{"family": "New"}]}),
        );
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                vec![entry],
                &BulkProcessingOptions::new().with_import_mode(ImportMode::Merge),
            )
            .await
            .unwrap();
        assert!(results[0].is_success());

        let stored = backend
            .read(&tenant, "Patient", "pg-merge-1")
            .await
            .unwrap()
            .expect("patient still stored");
        assert_eq!(stored.content()["name"], json!([{"family": "New"}]));
        assert_eq!(
            stored.content()["gender"],
            json!("female"),
            "merge must retain elements the submission omitted"
        );
    }

    // ========================================================================
    // Issue #311 — `ifMatch` on bundle entries, on a real PostgreSQL instance
    //
    // PostgreSQL carried both #311 defects (batch arm ignored `ifMatch`; the
    // field was compared as one opaque string), and its batch and transaction
    // arms are separate code paths from SQLite's. Running the *shared* suite
    // here is what makes "the backends agree" a checked claim rather than an
    // assumption — the assertions are literally the same function bodies.
    //
    // Each test gets its own tenant, since the whole binary shares one
    // container database and the scenarios reuse fixed resource ids.
    // ========================================================================

    /// Expands to a `#[tokio::test]` running one shared scenario against the
    /// shared PostgreSQL container under an isolated tenant.
    macro_rules! pg_if_match_test {
        ($test_name:ident, $scenario:ident) => {
            #[tokio::test]
            async fn $test_name() {
                let backend = create_backend().await;
                let tenant = create_tenant(concat!("if_match_", stringify!($scenario)));
                super::if_match_suite::$scenario(&backend, &tenant).await;
            }
        };
    }

    pg_if_match_test!(
        postgres_integration_batch_put_honors_stale_if_match,
        batch_put_honors_stale_if_match
    );
    pg_if_match_test!(
        postgres_integration_batch_put_accepts_matching_if_match,
        batch_put_accepts_matching_if_match
    );
    pg_if_match_test!(
        postgres_integration_multi_valued_if_match_matches_any_member,
        multi_valued_if_match_matches_any_member
    );
    pg_if_match_test!(
        postgres_integration_multi_valued_if_match_fails_when_no_member_matches,
        multi_valued_if_match_fails_when_no_member_matches
    );
    pg_if_match_test!(
        postgres_integration_strong_form_if_match_matches_weak_etag,
        strong_form_if_match_matches_weak_etag
    );
    pg_if_match_test!(
        postgres_integration_if_match_on_absent_resource_fails_instead_of_creating,
        if_match_on_absent_resource_fails_instead_of_creating
    );
    pg_if_match_test!(
        postgres_integration_star_if_match_requires_an_existing_resource,
        star_if_match_requires_an_existing_resource
    );
    pg_if_match_test!(
        postgres_integration_malformed_if_match_fails_closed,
        malformed_if_match_fails_closed
    );
    pg_if_match_test!(
        postgres_integration_batch_put_if_match_on_deleted_resource_fails,
        batch_put_if_match_on_deleted_resource_fails
    );
    pg_if_match_test!(
        postgres_integration_batch_delete_if_match_on_deleted_resource_fails,
        batch_delete_if_match_on_deleted_resource_fails
    );
    pg_if_match_test!(
        postgres_integration_batch_delete_honors_stale_if_match,
        batch_delete_honors_stale_if_match
    );
    pg_if_match_test!(
        postgres_integration_batch_delete_accepts_matching_if_match,
        batch_delete_accepts_matching_if_match
    );
    pg_if_match_test!(
        postgres_integration_transaction_delete_honors_stale_if_match,
        transaction_delete_honors_stale_if_match
    );
    pg_if_match_test!(
        postgres_integration_transaction_delete_accepts_matching_if_match,
        transaction_delete_accepts_matching_if_match
    );
}
