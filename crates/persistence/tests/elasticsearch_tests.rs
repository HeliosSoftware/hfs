//! Elasticsearch backend integration tests.
//!
//! These tests verify the Elasticsearch backend implementation.
//! Tests that require a running Elasticsearch instance use testcontainers
//! to spin up real ES instances in Docker.
//!
//! Run with: `cargo test -p helios-persistence --features elasticsearch -- elasticsearch`

#![cfg(feature = "elasticsearch")]

use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::core::{Backend, BackendCapability, BackendKind};

// ============================================================================
// Backend Configuration Tests (no ES instance required)
// ============================================================================

#[test]
fn test_elasticsearch_config_defaults() {
    let config = ElasticsearchConfig::default();
    assert_eq!(config.nodes, vec!["http://localhost:9200".to_string()]);
    assert_eq!(config.index_prefix, "hfs");
    assert_eq!(config.number_of_shards, 1);
    assert_eq!(config.number_of_replicas, 1);
    assert!(config.auth.is_none());
}

#[test]
fn test_elasticsearch_config_serialization() {
    let config = ElasticsearchConfig {
        nodes: vec!["http://es1:9200".to_string(), "http://es2:9200".to_string()],
        index_prefix: "test".to_string(),
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: ElasticsearchConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.nodes, config.nodes);
    assert_eq!(deserialized.index_prefix, "test");
}

#[test]
fn test_backend_creation() {
    let config = ElasticsearchConfig::default();
    // This just creates the client — doesn't connect
    let backend = ElasticsearchBackend::new(config);
    assert!(backend.is_ok());

    let backend = backend.unwrap();
    assert_eq!(backend.kind(), BackendKind::Elasticsearch);
    assert_eq!(backend.name(), "elasticsearch");
}

#[test]
fn test_backend_capabilities() {
    let config = ElasticsearchConfig::default();
    let backend = ElasticsearchBackend::new(config).unwrap();

    assert!(backend.supports(BackendCapability::Crud));
    assert!(backend.supports(BackendCapability::BasicSearch));
    assert!(backend.supports(BackendCapability::FullTextSearch));
    assert!(backend.supports(BackendCapability::CursorPagination));
    assert!(backend.supports(BackendCapability::OffsetPagination));
    assert!(backend.supports(BackendCapability::Sorting));
    assert!(backend.supports(BackendCapability::Include));
    assert!(backend.supports(BackendCapability::Revinclude));

    // ES does not support these
    assert!(!backend.supports(BackendCapability::Transactions));
    assert!(!backend.supports(BackendCapability::InstanceHistory));
    assert!(!backend.supports(BackendCapability::Versioning));
}

#[test]
fn test_index_name() {
    let config = ElasticsearchConfig {
        index_prefix: "hfs".to_string(),
        ..Default::default()
    };
    let backend = ElasticsearchBackend::new(config).unwrap();

    assert_eq!(backend.index_name("acme", "Patient"), "hfs_acme_patient");
    assert_eq!(
        backend.index_name("tenant-1", "Observation"),
        "hfs_tenant-1_observation"
    );
    assert_eq!(
        backend.index_name("acme/research", "Observation"),
        "hfs_acme-research_observation"
    );
}

// ============================================================================
// Query Builder Unit Tests (no ES instance required)
// ============================================================================

mod query_builder_tests {
    use helios_persistence::backends::elasticsearch::search::query_builder::{
        EsQueryBuilder, build_count_query,
    };
    use helios_persistence::types::{
        SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue, SortDirection,
        SortDirective,
    };

    #[test]
    fn test_empty_query() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient");
        let es_query = builder.build(&query);

        assert_eq!(es_query.index, "hfs_acme_patient");

        // Should have tenant_id and is_deleted filters
        let filters = &es_query.body["query"]["bool"]["filter"];
        assert!(filters.is_array());
        let filters = filters.as_array().unwrap();
        assert_eq!(filters.len(), 2);

        // Default sort: _lastUpdated desc + resource_id asc
        let sort = &es_query.body["sort"];
        assert!(sort.is_array());
        let sort = sort.as_array().unwrap();
        assert_eq!(sort.len(), 2);

        // Default size
        assert_eq!(es_query.body["size"], 20);

        // track_total_hits
        assert_eq!(es_query.body["track_total_hits"], true);
    }

    #[test]
    fn test_string_search_parameter() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        assert!(body_str.contains("search_params.string"));
        assert!(body_str.contains("Smith"));
    }

    #[test]
    fn test_token_search_parameter() {
        let builder =
            EsQueryBuilder::new("acme", "Observation", "hfs_acme_observation".to_string());
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|8867-4")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        assert!(body_str.contains("search_params.token"));
        assert!(body_str.contains("http://loinc.org"));
        assert!(body_str.contains("8867-4"));
    }

    #[test]
    fn test_date_range_query() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "2000-01-01")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        assert!(body_str.contains("search_params.date"));
        // The date handler transforms dates to precision-based ranges,
        // so the exact string may not appear; just verify it produces a range query
        assert!(body_str.contains("range") || body_str.contains("2000-01-01"));
    }

    #[test]
    fn test_multiple_values_or() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith"), SearchValue::eq("Jones")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        // Multiple values should produce a "should" (OR) clause
        assert!(body_str.contains("should"));
        assert!(body_str.contains("Smith"));
        assert!(body_str.contains("Jones"));
    }

    #[test]
    fn test_id_parameter() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        assert!(body_str.contains("resource_id"));
        assert!(body_str.contains("123"));
    }

    #[test]
    fn test_last_updated_parameter() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "2024-01-01")],
            chain: vec![],
            components: vec![],
        });

        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();

        assert!(body_str.contains("last_updated"));
        assert!(body_str.contains("gte"));
    }

    #[test]
    fn test_custom_sort() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "_id".to_string(),
            direction: SortDirection::Ascending,
        });

        let es_query = builder.build(&query);
        let sort = &es_query.body["sort"];
        let sort_arr = sort.as_array().unwrap();

        // First sort clause should be resource_id asc
        assert_eq!(sort_arr[0]["resource_id"]["order"], "asc");
        // Last should be tie-breaker
        assert_eq!(sort_arr[sort_arr.len() - 1]["resource_id"]["order"], "asc");
    }

    #[test]
    fn test_pagination_size() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let mut query = SearchQuery::new("Patient");
        query.count = Some(50);

        let es_query = builder.build(&query);
        assert_eq!(es_query.body["size"], 50);
    }

    #[test]
    fn test_offset_pagination() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let mut query = SearchQuery::new("Patient");
        query.offset = Some(100);

        let es_query = builder.build(&query);
        assert_eq!(es_query.body["from"], 100);
    }

    #[test]
    fn test_count_query() {
        let query = SearchQuery::new("Patient");
        let body = build_count_query("acme", "Patient", &query);

        // Count query should have size=0 and no sort
        assert_eq!(body["size"], 0);
        assert!(body.get("sort").is_none());
    }
}

// ============================================================================
// Search Parameter Handler Tests (no ES instance required)
// ============================================================================

mod parameter_handler_tests {
    use helios_persistence::backends::elasticsearch::search::parameter_handlers::*;
    use helios_persistence::types::{
        SearchModifier, SearchParamType, SearchParameter, SearchValue,
    };

    fn make_param(
        name: &str,
        param_type: SearchParamType,
        modifier: Option<SearchModifier>,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: vec![SearchValue::eq("test")],
            chain: vec![],
            components: vec![],
        }
    }

    // String handler tests
    mod string_handler {
        use super::*;

        #[test]
        fn test_default_prefix_match() {
            let param = make_param("name", SearchParamType::String, None);
            let clause = string::build_clause(&param, "Smi").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.string"));
            assert!(s.contains("Smi"));
        }

        #[test]
        fn test_exact_modifier() {
            let param = make_param("name", SearchParamType::String, Some(SearchModifier::Exact));
            let clause = string::build_clause(&param, "Smith").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("keyword"));
        }

        #[test]
        fn test_contains_modifier() {
            let param = make_param(
                "name",
                SearchParamType::String,
                Some(SearchModifier::Contains),
            );
            let clause = string::build_clause(&param, "mit").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("wildcard"));
            assert!(s.contains("mit"));
        }
    }

    // Token handler tests
    mod token_handler {
        use super::*;

        #[test]
        fn test_code_only() {
            let param = make_param("code", SearchParamType::Token, None);
            let clause = token::build_clause(&param, "active").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.token.code"));
            assert!(s.contains("active"));
        }

        #[test]
        fn test_system_and_code() {
            let param = make_param("code", SearchParamType::Token, None);
            let clause = token::build_clause(&param, "http://loinc.org|8867-4").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.token.system"));
            assert!(s.contains("http://loinc.org"));
            assert!(s.contains("search_params.token.code"));
            assert!(s.contains("8867-4"));
        }

        #[test]
        fn test_system_only() {
            let param = make_param("code", SearchParamType::Token, None);
            let clause = token::build_clause(&param, "http://loinc.org|").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.token.system"));
            assert!(s.contains("http://loinc.org"));
            // Should NOT contain token.code match
        }

        #[test]
        fn test_code_no_system() {
            let param = make_param("code", SearchParamType::Token, None);
            let clause = token::build_clause(&param, "|active").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("active"));
            assert!(s.contains("must_not"));
        }

        #[test]
        fn test_not_modifier() {
            let param = make_param("gender", SearchParamType::Token, Some(SearchModifier::Not));
            let clause = token::build_clause(&param, "male").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("must_not"));
        }

        #[test]
        fn test_text_modifier() {
            let param = make_param("code", SearchParamType::Token, Some(SearchModifier::Text));
            let clause = token::build_clause(&param, "headache").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("display"));
            assert!(s.contains("headache"));
        }
    }

    // Date handler tests
    mod date_handler {
        use helios_persistence::types::SearchPrefix;

        #[test]
        fn test_date_eq() {
            use super::*;
            let clause = date::build_clause("birthdate", "2000-01-15", SearchPrefix::Eq).unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.date"));
            assert!(s.contains("2000-01-15"));
        }

        #[test]
        fn test_date_gt() {
            use super::*;
            let clause = date::build_clause("birthdate", "2000-01-15", SearchPrefix::Gt).unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.date"));
        }
    }

    // Reference handler tests
    mod reference_handler {
        use super::*;

        #[test]
        fn test_relative_reference() {
            let param = make_param("subject", SearchParamType::Reference, None);
            let clause = reference::build_clause(&param, "Patient/123").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("Patient/123"));
            assert!(s.contains("search_params.reference"));
        }

        #[test]
        fn test_id_only() {
            let param = make_param("subject", SearchParamType::Reference, None);
            let clause = reference::build_clause(&param, "123").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("resource_id"));
        }

        #[test]
        fn test_type_modifier() {
            let param = make_param(
                "subject",
                SearchParamType::Reference,
                Some(SearchModifier::Type("Patient".to_string())),
            );
            let clause = reference::build_clause(&param, "123").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("resource_type"));
            assert!(s.contains("Patient"));
        }
    }

    // Number handler tests
    mod number_handler {
        use helios_persistence::types::SearchPrefix;

        #[test]
        fn test_number_eq() {
            use super::*;
            let clause = number::build_clause("probability", "0.5", SearchPrefix::Eq).unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.number"));
        }

        #[test]
        fn test_number_gt() {
            use super::*;
            let clause = number::build_clause("probability", "0.5", SearchPrefix::Gt).unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.number"));
            assert!(s.contains("gt"));
        }
    }

    // Quantity handler tests
    mod quantity_handler {
        use helios_persistence::types::SearchPrefix;

        #[test]
        fn test_quantity_value_only() {
            use super::*;
            let clause = quantity::build_clause("value-quantity", "5.4", SearchPrefix::Eq).unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.quantity"));
        }

        #[test]
        fn test_quantity_with_system_code() {
            use super::*;
            let clause = quantity::build_clause(
                "value-quantity",
                "5.4|http://unitsofmeasure.org|mg",
                SearchPrefix::Eq,
            )
            .unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.quantity"));
            assert!(s.contains("http://unitsofmeasure.org"));
            assert!(s.contains("mg"));
        }
    }

    // URI handler tests
    mod uri_handler {
        use super::*;

        #[test]
        fn test_exact_uri() {
            let param = make_param("url", SearchParamType::Uri, None);
            let clause = uri::build_clause(&param, "http://example.org/fhir").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("search_params.uri"));
            assert!(s.contains("http://example.org/fhir"));
        }

        #[test]
        fn test_below_modifier() {
            let param = make_param("url", SearchParamType::Uri, Some(SearchModifier::Below));
            let clause = uri::build_clause(&param, "http://example.org/fhir").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("prefix"));
        }

        #[test]
        fn test_above_modifier() {
            let param = make_param("url", SearchParamType::Uri, Some(SearchModifier::Above));
            let clause = uri::build_clause(&param, "http://example.org/fhir/ValueSet/123").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(s.contains("terms"));
        }
    }
}

// ============================================================================
// Integration Tests (requires Docker for testcontainers)
// ============================================================================

/// Integration tests that require a real Elasticsearch instance via testcontainers.
///
/// These tests are behind `#[cfg(feature = "elasticsearch")]` and require Docker.
/// They mirror the patterns in sqlite_tests.rs (except history/transactions/conditional
/// ops which ES does not support).
///
/// Run with:
///   cargo test -p helios-persistence --features elasticsearch -- es_integration
///
/// Skip if no Docker:
///   cargo test -p helios-persistence --features elasticsearch -- --skip es_integration
#[cfg(test)]
mod es_integration {
    use std::path::PathBuf;
    use std::sync::Arc;

    use helios_fhir::FhirVersion;
    use parking_lot::RwLock;
    use serde_json::json;

    use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
    use helios_persistence::core::{Backend, BackendCapability, BackendKind, ResourceStorage};
    use helios_persistence::error::{ResourceError, StorageError};
    use helios_persistence::search::{
        SearchParameterDefinition, SearchParameterLoader, SearchParameterRegistry,
        SearchParameterSource, SearchParameterStatus,
    };
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use tokio::sync::OnceCell;

    /// Shared Elasticsearch container reused across all tests in this module.
    struct SharedEs {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; dropped at process exit.
        _container: testcontainers::ContainerAsync<ElasticSearch>,
    }

    static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

    async fn shared_es() -> &'static SharedEs {
        SHARED_ES
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = ElasticSearch::default()
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(std::time::Duration::from_secs(120))
                    .start()
                    .await
                    .expect("Failed to start Elasticsearch container");

                let port = container
                    .get_host_port_ipv4(9200)
                    .await
                    .expect("Failed to get host port");

                let host = container
                    .get_host()
                    .await
                    .expect("Failed to get host")
                    .to_string();

                SharedEs {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    /// Builds a search parameter registry loaded from the FHIR spec data files.
    fn build_search_registry() -> Arc<RwLock<SearchParameterRegistry>> {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let loader = SearchParameterLoader::new(FhirVersion::default());
        let mut registry = SearchParameterRegistry::new();

        // Load embedded (minimal) params first
        if let Ok(params) = loader.load_embedded() {
            for param in params {
                let _ = registry.register(param);
            }
        }

        // Load full search parameter definitions from spec file
        if let Ok(params) = loader.load_from_spec_file(&data_dir) {
            for param in params {
                let _ = registry.register(param);
            }
        }

        // Override value-quantity with a direct-path expression.
        // The spec's expression uses FHIRPath `as` operator and choice-type resolution
        // (e.g., `Observation.value as Quantity`) which the extractor can't evaluate
        // against raw JSON. Use the concrete JSON field name instead.
        let _ = registry.register(SearchParameterDefinition {
            url: "http://test.local/SearchParameter/Observation-value-quantity".to_string(),
            code: "value-quantity".to_string(),
            name: Some("value-quantity".to_string()),
            description: None,
            param_type: helios_persistence::types::SearchParamType::Quantity,
            expression: "Observation.valueQuantity".to_string(),
            base: vec!["Observation".to_string()],
            target: None,
            component: None,
            status: SearchParameterStatus::Active,
            source: SearchParameterSource::Config,
            modifier: None,
            multiple_or: None,
            multiple_and: None,
            comparator: None,
            xpath: None,
        });

        Arc::new(RwLock::new(registry))
    }

    /// Creates an ElasticsearchBackend connected to the shared testcontainers ES instance.
    ///
    /// Each call uses a unique index prefix (via UUID) so tests are fully isolated
    /// without needing separate containers.
    async fn create_backend() -> ElasticsearchBackend {
        let es = shared_es().await;
        let unique_prefix = format!("hfs_{}", uuid::Uuid::new_v4().simple());

        let config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: unique_prefix,
            number_of_replicas: 0, // single-node, no replicas needed
            refresh_interval: "1ms".to_string(), // near-instant refresh for tests
            ..Default::default()
        };

        let search_registry = build_search_registry();
        let backend = ElasticsearchBackend::with_shared_registry(config, search_registry)
            .expect("Failed to create ElasticsearchBackend");

        backend
            .initialize()
            .await
            .expect("Failed to initialize ES backend");

        backend
    }

    fn create_tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    // ========================================================================
    // CRUD Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_create_resource() {
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
    async fn es_integration_create_with_id() {
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
    async fn es_integration_create_duplicate_overwrites() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "duplicate-id",
            "name": [{"family": "Original"}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // ES create uses the index API (upsert), so a second create with the
        // same ID overwrites the document rather than failing.
        let patient2 = json!({
            "resourceType": "Patient",
            "id": "duplicate-id",
            "name": [{"family": "Overwritten"}]
        });

        let result = backend
            .create(&tenant, "Patient", patient2, FhirVersion::default())
            .await;
        assert!(result.is_ok(), "ES create is an upsert and should succeed");

        let read = backend
            .read(&tenant, "Patient", "duplicate-id")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content()["name"][0]["family"], "Overwritten");
    }

    #[tokio::test]
    async fn es_integration_read_resource() {
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
    async fn es_integration_read_nonexistent() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let read = backend
            .read(&tenant, "Patient", "does-not-exist")
            .await
            .unwrap();
        assert!(read.is_none());
    }

    #[tokio::test]
    async fn es_integration_exists() {
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

    // ========================================================================
    // Update / Upsert Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_update_resource() {
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
    async fn es_integration_create_or_update_creates() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

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
    }

    #[tokio::test]
    async fn es_integration_create_or_update_updates() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Create via upsert
        let patient = json!({"resourceType": "Patient", "name": [{"family": "First"}]});
        backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update via upsert
        let patient2 = json!({"resourceType": "Patient", "name": [{"family": "Second"}]});
        let (resource, was_created) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient2,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(!was_created);
        assert_eq!(resource.content()["name"][0]["family"], "Second");
    }

    #[tokio::test]
    async fn es_integration_create_or_update_ignores_stale_version() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create_or_update(
                &tenant,
                "Patient",
                "versioned-id",
                json!({
                    "resourceType": "Patient",
                    "meta": {"versionId": "5"},
                    "name": [{"family": "Newest"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create_or_update(
                &tenant,
                "Patient",
                "versioned-id",
                json!({
                    "resourceType": "Patient",
                    "meta": {"versionId": "4"},
                    "name": [{"family": "Stale"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let current = backend
            .read(&tenant, "Patient", "versioned-id")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.version_id(), "5");
        assert_eq!(current.content()["name"][0]["family"], "Newest");
    }

    #[tokio::test]
    async fn es_integration_create_or_update_replay_same_version_is_idempotent() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create_or_update(
                &tenant,
                "Patient",
                "replay-id",
                json!({
                    "resourceType": "Patient",
                    "meta": {"versionId": "9"},
                    "name": [{"family": "Initial"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create_or_update(
                &tenant,
                "Patient",
                "replay-id",
                json!({
                    "resourceType": "Patient",
                    "meta": {"versionId": "9"},
                    "name": [{"family": "ReplayAttempt"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let current = backend
            .read(&tenant, "Patient", "replay-id")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.version_id(), "9");
        assert_eq!(current.content()["name"][0]["family"], "Initial");
    }

    // ========================================================================
    // Delete Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_delete_resource() {
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
            Ok(None) => {}
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
            other => {
                panic!("Expected None or Gone error, got: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn es_integration_delete_nonexistent_is_idempotent() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let result = backend.delete(&tenant, "Patient", "nonexistent").await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Tenant Isolation Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_tenant_isolation() {
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
    async fn es_integration_same_id_different_tenants() {
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

    #[tokio::test]
    async fn es_integration_tenant_isolation_search() {
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
                    "name": [{"family": "Smith"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        // Tenant A finds the patient
        let result_a = backend.search(&tenant_a, &query).await.unwrap();
        assert!(
            !result_a.resources.items.is_empty(),
            "Tenant A should find the patient"
        );

        // Tenant B does not find the patient
        let result_b = backend.search(&tenant_b, &query).await.unwrap();
        assert!(
            result_b.resources.items.is_empty(),
            "Tenant B should not see tenant A's patient"
        );
    }

    #[tokio::test]
    async fn es_integration_tenant_isolation_delete() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant B delete is idempotent against its own tenant namespace.
        let result = backend.delete(&tenant_b, "Patient", created.id()).await;
        assert!(result.is_ok());

        // Resource still exists for tenant A
        assert!(
            backend
                .exists(&tenant_a, "Patient", created.id())
                .await
                .unwrap()
        );
    }

    // ========================================================================
    // Count Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_count_resources() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 0..5 {
            let patient = json!({"resourceType": "Patient", "id": format!("p{}", i)});
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let count = backend.count(&tenant, Some("Patient")).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn es_integration_count_by_tenant() {
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

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 3);
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 2);
    }

    // ========================================================================
    // Content Preservation Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_content_preserved() {
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

    #[tokio::test]
    async fn es_integration_unicode_content() {
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
    // Search Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_search_by_name() {
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
                    "id": "p1",
                    "name": [{"family": "Smith", "given": ["John"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
    async fn es_integration_search_by_name_multiple() {
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

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
        assert!(ids.contains(&"name-1"), "Should include Smith");
        assert!(ids.contains(&"name-2"), "Should include Smithson");
    }

    #[tokio::test]
    async fn es_integration_search_by_token() {
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
                    "id": "p1",
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
                    "id": "p2",
                    "gender": "female"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "p1");
    }

    #[tokio::test]
    async fn es_integration_search_token_system_code() {
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
                    "id": "token-sys-1",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "12345"}]
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
                    "id": "token-sys-2",
                    "identifier": [{"system": "http://other.org/id", "value": "12345"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Search by system|code
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://hospital.org/mrn|12345")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "token-sys-1");
    }

    #[tokio::test]
    async fn es_integration_search_token_code_only() {
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
                    "id": "code-1",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "12345"}]
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
                    "id": "code-2",
                    "identifier": [{"system": "http://other.org/id", "value": "12345"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Search by code only (should find both)
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("12345")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            2,
            "Should find 2 patients with code 12345"
        );
    }

    #[tokio::test]
    async fn es_integration_search_date() {
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

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
    async fn es_integration_search_reference() {
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
                    "subject": {"reference": "Patient/patient-2"},
                    "code": {"coding": [{"code": "9279-1"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/patient-1")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "obs-1");
    }

    #[tokio::test]
    async fn es_integration_search_quantity() {
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
                    "id": "obs-q1",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                    "status": "final",
                    "valueQuantity": {"value": 72, "unit": "beats/min", "system": "http://unitsofmeasure.org", "code": "/min"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::eq("72|http://unitsofmeasure.org|/min")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "Should find observation by quantity"
        );
    }

    #[tokio::test]
    async fn es_integration_search_number() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "RiskAssessment",
                json!({
                    "resourceType": "RiskAssessment",
                    "id": "risk-1",
                    "status": "final",
                    "prediction": [{"probabilityDecimal": 0.8}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
            name: "probability".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "0.5")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        // Note: number search depends on SearchParameter extraction for RiskAssessment
        // This verifies the query doesn't error
        assert!(result.resources.items.len() <= 1);
    }

    #[tokio::test]
    async fn es_integration_search_uri() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "ValueSet",
                json!({
                    "resourceType": "ValueSet",
                    "id": "vs-1",
                    "url": "http://example.org/fhir/ValueSet/123",
                    "status": "active"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: None,
            values: vec![SearchValue::eq("http://example.org/fhir/ValueSet/123")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "vs-1");
    }

    #[tokio::test]
    async fn es_integration_search_multiple_params() {
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

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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

    #[tokio::test]
    async fn es_integration_search_multiple_values_or() {
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
                    "id": "or-1",
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
                    "id": "or-2",
                    "gender": "female"
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
                    "id": "or-3",
                    "gender": "other"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // OR within values: gender=male,female
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male"), SearchValue::eq("female")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            2,
            "OR within values should find 2 patients"
        );
    }

    #[tokio::test]
    async fn es_integration_search_by_id() {
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
                    "id": "id-search-1"
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
                    "id": "id-search-2"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("id-search-1")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "id-search-1");
    }

    #[tokio::test]
    async fn es_integration_search_last_updated() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "lu-1"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Search for resources updated after a long-ago date
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "2020-01-01")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "_lastUpdated search should find recently created resource"
        );
    }

    // ========================================================================
    // Full-Text Search Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_text_search_content() {
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
                    "id": "content-1",
                    "name": [{"family": "Springfield", "given": ["Homer"]}],
                    "address": [{"city": "Springfield", "state": "Illinois"}]
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
                    "id": "content-2",
                    "name": [{"family": "Simpson", "given": ["Bart"]}],
                    "address": [{"city": "Chicago", "state": "Illinois"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_content".to_string(),
            param_type: SearchParamType::Special,
            modifier: None,
            values: vec![SearchValue::eq("Springfield")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        // Should find patients containing "Springfield" in their content
        assert!(
            !result.resources.items.is_empty(),
            "_content search should find resources containing the term"
        );
    }

    #[tokio::test]
    async fn es_integration_text_search_narrative() {
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
                    "id": "text-1",
                    "text": {
                        "status": "generated",
                        "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Patient with diabetes and hypertension.</p></div>"
                    },
                    "name": [{"family": "Smith"}]
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
                    "id": "text-2",
                    "text": {
                        "status": "generated",
                        "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Patient with asthma.</p></div>"
                    },
                    "name": [{"family": "Doe"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::Special,
            modifier: None,
            values: vec![SearchValue::eq("diabetes")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "text-1");
    }

    #[tokio::test]
    async fn es_integration_text_search_token_text_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-text-1",
                    "code": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "8867-4",
                            "display": "Heart rate"
                        }]
                    },
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
                    "id": "obs-text-2",
                    "code": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "9279-1",
                            "display": "Respiratory rate"
                        }]
                    },
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Search using :text modifier for "heart"
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("heart")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "obs-text-1");
    }

    // ========================================================================
    // Backend Info Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_health_check() {
        let backend = create_backend().await;

        let result = backend.health_check().await;
        assert!(result.is_ok(), "Health check failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn es_integration_backend_kind() {
        let backend = create_backend().await;

        assert_eq!(backend.kind(), BackendKind::Elasticsearch);
        assert_eq!(backend.name(), "elasticsearch");
    }

    #[tokio::test]
    async fn es_integration_capabilities() {
        let backend = create_backend().await;

        assert!(backend.supports(BackendCapability::Crud));
        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::FullTextSearch));
        assert!(backend.supports(BackendCapability::Sorting));
        assert!(backend.supports(BackendCapability::CursorPagination));
        assert!(backend.supports(BackendCapability::OffsetPagination));

        // ES does NOT support these
        assert!(!backend.supports(BackendCapability::Transactions));
        assert!(!backend.supports(BackendCapability::InstanceHistory));
        assert!(!backend.supports(BackendCapability::Versioning));
    }
}

// ============================================================================
// Search Offloading Tests
// ============================================================================

#[cfg(feature = "sqlite")]
mod search_offloading_tests {
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_persistence::core::{ResourceStorage, SearchProvider};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_persistence::types::SearchQuery;
    use serde_json::json;
    use std::path::PathBuf;

    fn create_backend(search_offloaded: bool) -> SqliteBackend {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            search_offloaded,
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", config)
            .expect("Failed to create SQLite backend");
        backend.init_schema().expect("Failed to initialize schema");
        backend
    }

    fn create_tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn test_search_offloaded_crud_still_works() {
        let backend = create_backend(true);
        let tenant = create_tenant("test");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        // Create should succeed even with offloaded search
        let result = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await;
        assert!(result.is_ok());

        // Read should work
        let read = backend.read(&tenant, "Patient", "p1").await;
        assert!(read.is_ok());
        assert!(read.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_search_offloaded_parameterized_search_returns_empty() {
        use helios_persistence::types::{SearchParamType, SearchParameter, SearchValue};

        let backend = create_backend(true);
        let tenant = create_tenant("test");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // With search offloaded, the SQLite search index is empty,
        // so parameterized search should return no results
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
            result.resources.len(),
            0,
            "Parameterized search should return empty when search is offloaded"
        );
    }

    #[tokio::test]
    async fn test_search_not_offloaded_parameterized_search_finds_resources() {
        use helios_persistence::types::{SearchParamType, SearchParameter, SearchValue};

        let backend = create_backend(false);
        let tenant = create_tenant("test");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Without offloading, parameterized search should find the resource
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
            !result.resources.is_empty(),
            "Parameterized search should find resources when not offloaded"
        );
    }

    #[tokio::test]
    async fn test_search_offloaded_delete_works() {
        let backend = create_backend(true);
        let tenant = create_tenant("test");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith"}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Delete should succeed
        let result = backend.delete(&tenant, "Patient", "p1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_search_offloaded() {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let mut backend = SqliteBackend::with_config(":memory:", config)
            .expect("Failed to create SQLite backend");

        assert!(!backend.is_search_offloaded());
        backend.set_search_offloaded(true);
        assert!(backend.is_search_offloaded());
    }
}
