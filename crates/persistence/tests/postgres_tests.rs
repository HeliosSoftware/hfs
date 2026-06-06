//! PostgreSQL backend integration tests.
//!
//! These tests verify the PostgreSQL backend implementation.
//! Tests that require a running PostgreSQL instance use testcontainers
//! to spin up real PostgreSQL instances in Docker.
//!
//! Run with: `cargo test -p helios-persistence --features postgres -- postgres`

#![cfg(feature = "postgres")]

use helios_persistence::backends::postgres::PostgresConfig;
use helios_persistence::core::{BackendCapability, BackendKind};

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
    assert_eq!(config.max_connections, 10);
    assert_eq!(config.connect_timeout_secs, 5);
    assert_eq!(config.statement_timeout_ms, 30000);
    assert!(!config.search_offloaded);
    assert!(config.schema_name.is_none());
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

// NOTE: These tests verify capability declarations. They cannot construct a
// PostgresBackend without a real database (the constructor connects immediately).
// We verify via config + trait bounds instead.

#[test]
fn test_postgres_config_backend_kind() {
    // Verify BackendKind::Postgres exists and is usable
    let kind = BackendKind::Postgres;
    assert_eq!(format!("{}", kind), "postgres");
}

#[test]
fn test_postgres_expected_capabilities() {
    // Verify the capability enum variants that PostgreSQL should support exist
    let expected = [
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
        BackendCapability::Include,
        BackendCapability::Revinclude,
        BackendCapability::SharedSchema,
        BackendCapability::SchemaPerTenant,
        BackendCapability::DatabasePerTenant,
    ];
    // This is a compile-time check: all variants exist
    assert!(!expected.is_empty());
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
        // Default string search is starts-with (case-insensitive via ILIKE)
        assert!(fragment.sql.contains("ILIKE"));
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
        assert!(fragment.sql.contains("ILIKE"));
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
        assert!(fragment.sql.contains("value_string ILIKE"));
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
        assert!(fragment.sql.contains("> $"));
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
        match &fragment.params[0] {
            SqlParam::Float(f) => assert!((f - 0.5).abs() < f64::EPSILON),
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
    use helios_persistence::core::history::{HistoryParams, InstanceHistoryProvider};
    use helios_persistence::core::{Backend, BackendCapability, BackendKind, ResourceStorage};
    use helios_persistence::error::{ResourceError, StorageError};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::{Mutex, OnceCell};

    /// Shared PostgreSQL container reused across all tests in this module.
    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; dropped at process exit.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();
    static BULK_EXPORT_TEST_LOCK: Mutex<()> = Mutex::const_new(());

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

    // ========================================================================
    // Backend Health Check Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_health_check() {
        let backend = create_backend().await;

        let result = backend.health_check().await;
        assert!(result.is_ok(), "Health check failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn postgres_integration_backend_kind() {
        let backend = create_backend().await;

        assert_eq!(backend.kind(), BackendKind::Postgres);
        assert_eq!(backend.name(), "postgres");
    }

    #[tokio::test]
    async fn postgres_integration_capabilities() {
        let backend = create_backend().await;

        assert!(backend.supports(BackendCapability::Crud));
        assert!(backend.supports(BackendCapability::Versioning));
        assert!(backend.supports(BackendCapability::InstanceHistory));
        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::Transactions));
        assert!(backend.supports(BackendCapability::Include));
        assert!(backend.supports(BackendCapability::Revinclude));
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
        use helios_persistence::search::ReindexableStorage;

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
        use helios_persistence::search::ReindexableStorage;

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
        use helios_persistence::search::ReindexableStorage;

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
}
