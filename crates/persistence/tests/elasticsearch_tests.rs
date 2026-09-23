//! Elasticsearch backend integration tests.
//!
//! These tests verify the Elasticsearch backend implementation.
//! Tests that require a running Elasticsearch instance use testcontainers
//! to spin up real ES instances in Docker.
//!
//! Run with: `cargo test -p helios-persistence --features elasticsearch -- elasticsearch`

#![cfg(feature = "elasticsearch")]

use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
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
    assert_eq!(config.nested_objects_limit, 50_000);
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
fn test_write_refresh_policy_default_is_false() {
    assert_eq!(
        ElasticsearchConfig::default().write_refresh,
        WriteRefreshPolicy::False
    );

    let json = r#"{"nodes": ["http://localhost:9200"]}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.write_refresh, WriteRefreshPolicy::False);
}

/// #1050: a config that omits the field must not fall back to
/// Elasticsearch's own limit of 10000, which drops large resources from search.
#[test]
fn test_nested_objects_limit_defaults_above_elasticsearch_default() {
    assert_eq!(ElasticsearchConfig::default().nested_objects_limit, 50_000);

    let json = r#"{"nodes": ["http://localhost:9200"]}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.nested_objects_limit, 50_000);
}

#[test]
fn test_write_refresh_policy_parsing() {
    assert_eq!(
        "false".parse::<WriteRefreshPolicy>().unwrap(),
        WriteRefreshPolicy::False
    );
    assert_eq!(
        "wait_for".parse::<WriteRefreshPolicy>().unwrap(),
        WriteRefreshPolicy::WaitFor
    );
    assert_eq!(
        "wait-for".parse::<WriteRefreshPolicy>().unwrap(),
        WriteRefreshPolicy::WaitFor
    );
    assert_eq!(
        "true".parse::<WriteRefreshPolicy>().unwrap(),
        WriteRefreshPolicy::True
    );
    assert_eq!(
        " WAIT_FOR ".parse::<WriteRefreshPolicy>().unwrap(),
        WriteRefreshPolicy::WaitFor
    );
    assert!("refresh-me".parse::<WriteRefreshPolicy>().is_err());
    assert!("".parse::<WriteRefreshPolicy>().is_err());
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

/// Index names must be injective in the tenant, because every `_id`-addressed
/// operation (`create`, `update`, `delete`, and the `create_or_update` existence
/// probe) is confined to a tenant *only* by the index it targets — those APIs
/// admit no query filter.
///
/// This replaces a test that asserted `index_name("acme", …)` and
/// `index_name("tenant-1", …)` only. Neither input exercised the derivation's
/// lossy step, so the suite was green while `ACME` and `acme` shared an index
/// (issue #384). The exhaustive property tests live beside the encoder in
/// `backends/elasticsearch/naming.rs`.
#[test]
fn test_index_name_is_injective_in_the_tenant() {
    let config = ElasticsearchConfig {
        index_prefix: "hfs".to_string(),
        ..Default::default()
    };
    let backend = ElasticsearchBackend::new(config).unwrap();

    // Already-safe ids are unchanged, so conforming deployments do not migrate.
    assert_eq!(backend.index_name("acme", "Patient"), "hfs_acme_patient");
    assert_eq!(
        backend.index_name("tenant-1", "Observation"),
        "hfs_tenant-1_observation"
    );

    // Case variants must not collide.
    assert_ne!(
        backend.index_name("ACME", "Patient"),
        backend.index_name("acme", "Patient")
    );
    // A hierarchical id used to produce an illegal name and 500 on every write.
    let hierarchical = backend.index_name("acme/research", "Patient");
    assert!(!hierarchical.contains('/'));
    assert_ne!(hierarchical, backend.index_name("acmeresearch", "Patient"));
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

        // Default size: the default count (20) plus one over-fetched hit (#1079)
        assert_eq!(es_query.body["size"], 21);

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
            param_type: None,
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
        // The query over-fetches by one hit beyond the requested count (#1079)
        assert_eq!(es_query.body["size"], 51);
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

        // `_count` accepts only `query`: anything else the search builder
        // sets (size, sort, track_total_hits, ...) is a parsing_exception.
        let keys: Vec<&String> = body.as_object().expect("object body").keys().collect();
        assert_eq!(keys, vec!["query"], "count body: {body}");
        assert!(
            body["query"]["bool"].is_object(),
            "the tenant filter survives"
        );
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
        fn test_not_modifier_builds_positive_clause() {
            // The negation is applied once by the query builder, around the OR
            // of every value (#473) — the per-value clause stays positive.
            let param = make_param("gender", SearchParamType::Token, Some(SearchModifier::Not));
            let clause = token::build_clause(&param, "male").unwrap();
            let s = serde_json::to_string(&clause).unwrap();
            assert!(!s.contains("must_not"));
            assert!(s.contains("search_params.token.code"));
            assert!(s.contains("male"));
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
/// The backend-agnostic day-precision date-boundary suite (issue #519) — the
/// #456 table that #463 pinned for SQLite only; the ES date handler was
/// explicitly unverified. `#[path]`-included like the other shared suites.
#[path = "search/date_boundary_suite.rs"]
mod date_boundary_suite;

/// The backend-agnostic sub-day precision and date-validation suite (#1293,
/// #1295, #1296, #1297). Same `#[path]` arrangement.
#[path = "search/date_precision_suite.rs"]
mod date_precision_suite;

/// The backend-agnostic `_contained` suite (#1336, #1362, #1363). Same
/// `#[path]` arrangement.
#[path = "search/contained_suite.rs"]
mod contained_suite;

/// The backend-agnostic suite for exponent-form number and quantity search
/// values (#1337). Same `#[path]` arrangement.
#[path = "search/number_exponent_suite.rs"]
mod number_exponent_suite;

/// The backend-agnostic number / quantity validation suite (#1319, #1340).
/// Same `#[path]` arrangement.
#[path = "search/numeric_validation_suite.rs"]
mod numeric_validation_suite;

/// The backend-agnostic `system|code` on `code` elements suite (#1379). Same
/// `#[path]` arrangement.
#[path = "search/token_code_system_suite.rs"]
mod token_code_system_suite;

/// The backend-agnostic empty search value suite (#1380). Same `#[path]`
/// arrangement.
#[path = "search/empty_value_suite.rs"]
mod empty_value_suite;

/// The backend-agnostic modifier parity suite (#1408). Same `#[path]`
/// arrangement.
#[path = "search/modifier_parity_suite.rs"]
mod modifier_parity_suite;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[cfg(test)]
mod es_integration {
    use std::path::PathBuf;
    use std::sync::Arc;

    use helios_fhir::FhirVersion;
    use serde_json::json;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
    };
    use helios_persistence::core::{Backend, BackendCapability, BackendKind, ResourceStorage};
    use helios_persistence::error::{ResourceError, StorageError};
    use helios_persistence::search::{
        SearchParameterDefinition, SearchParameterLoader, SearchParameterSource,
        SearchParameterStatus, TenantSearchRegistries,
    };
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    #[cfg(feature = "postgres")]
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    /// Shared Elasticsearch container reused across all tests in this module.
    struct SharedEs {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; dropped at process exit.
        _container: testcontainers::ContainerAsync<ElasticSearch>,
    }

    static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

    #[cfg(feature = "postgres")]
    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    #[cfg(feature = "postgres")]
    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    #[cfg(feature = "postgres")]
    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = super::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("start PostgreSQL for pg-es reindex");
                let host = container.get_host().await.unwrap().to_string();
                let port = container.get_host_port_ipv4(5432).await.unwrap();
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    /// Startup budget for one Elasticsearch container start attempt. See the
    /// matching constant in `s3_es_tests.rs`: 120s was not enough on the
    /// loaded CI Docker host (run 33636603224).
    const ES_STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

    /// Elasticsearch image tag for the shared test container.
    ///
    /// `testcontainers-modules` defaults to 7.16.1, whose bundled JDK 17.0.1
    /// crashes at startup on cgroup v2 hosts without a mounted controller
    /// (Docker Desktop's linuxkit VM): `NullPointerException: Cannot invoke
    /// "jdk.internal.platform.CgroupInfo.getMountPoint()" because "anyController"
    /// is null` (JDK-8272124, fixed in 17.0.2). Every 7.16.x tag ships the broken
    /// JDK, so pin the latest 7.17 release; it keeps the 7.x API surface and the
    /// `[YELLOW] to [GREEN]` ready message the module waits on (8.x does not log
    /// that line, so a plain tag bump to 8.15.0 would hang the startup wait).
    const ES_IMAGE_TAG: &str = "7.17.29";

    /// How many times to try starting the ES container before giving up.
    const ES_START_ATTEMPTS: usize = 2;

    /// Starts the shared Elasticsearch container, retrying once on a startup
    /// failure so a slow host does not fail the whole suite.
    async fn start_es_container() -> testcontainers::ContainerAsync<ElasticSearch> {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let mut last_err = None;
        for attempt in 1..=ES_START_ATTEMPTS {
            // `SHARED_ES` is a static and never dropped; the cleanup label
            // lets the exit hook remove the container.
            match super::container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag(ES_IMAGE_TAG)
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(ES_STARTUP_TIMEOUT),
            )
            .start()
            .await
            {
                Ok(container) => return container,
                Err(err) => {
                    eprintln!(
                        "Elasticsearch container start attempt {attempt}/{ES_START_ATTEMPTS} failed: {err}"
                    );
                    last_err = Some(err);
                }
            }
        }
        panic!(
            "failed to start Elasticsearch container after {ES_START_ATTEMPTS} attempts: {:?}",
            last_err.expect("at least one attempt ran")
        );
    }

    async fn shared_es() -> &'static SharedEs {
        SHARED_ES
            .get_or_init(|| async {
                let container = start_es_container().await;

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
    fn build_search_registry() -> Arc<TenantSearchRegistries> {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let loader = SearchParameterLoader::new(FhirVersion::default());
        let registries = Arc::new(TenantSearchRegistries::base_only());
        let mut registry = registries.base().write();

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

        drop(registry);
        registries
    }

    /// Creates an ElasticsearchBackend connected to the shared testcontainers ES instance.
    ///
    /// Each call uses a unique index prefix (via UUID) so tests are fully isolated
    /// without needing separate containers.
    async fn create_backend() -> ElasticsearchBackend {
        create_backend_with("1ms", WriteRefreshPolicy::default()).await
    }

    async fn create_backend_with(
        refresh_interval: &str,
        write_refresh: WriteRefreshPolicy,
    ) -> ElasticsearchBackend {
        let es = shared_es().await;
        let unique_prefix = format!("hfs_{}", uuid::Uuid::new_v4().simple());

        let config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: unique_prefix,
            number_of_replicas: 0, // single-node, no replicas needed
            refresh_interval: refresh_interval.to_string(),
            write_refresh,
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

    /// A backend on `index_prefix` with the given nested-object limit, so the
    /// #1050 tests can put two backends on the same indices.
    async fn create_backend_with_nested_limit(
        index_prefix: &str,
        nested_objects_limit: u32,
    ) -> ElasticsearchBackend {
        let es = shared_es().await;
        let config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: index_prefix.to_string(),
            number_of_replicas: 0,
            refresh_interval: "1ms".to_string(),
            nested_objects_limit,
            ..Default::default()
        };
        let backend = ElasticsearchBackend::with_shared_registry(config, build_search_registry())
            .expect("Failed to create ElasticsearchBackend");
        backend
            .initialize()
            .await
            .expect("Failed to initialize ES backend");
        backend
    }

    /// A Synthea-shaped `Provenance` whose `target` array alone holds more
    /// nested reference values than Elasticsearch's default limit of 10000 —
    /// the shape of the 458 resources #1050 found stored but unsearchable.
    fn oversized_provenance(
        tenant: &TenantContext,
        id: &str,
        targets: usize,
    ) -> helios_persistence::types::StoredResource {
        let target: Vec<serde_json::Value> = (0..targets)
            .map(|n| json!({ "reference": format!("Observation/obs-{n}") }))
            .collect();
        helios_persistence::types::StoredResource::from_storage(
            "Provenance",
            id.to_string(),
            "1",
            tenant.tenant_id().clone(),
            json!({
                "resourceType": "Provenance",
                "id": id,
                "target": target,
                "recorded": "2009-02-28T07:56:45.469-05:00",
                "agent": [{ "who": { "reference": "Practitioner/p1" } }]
            }),
            chrono::Utc::now(),
            chrono::Utc::now(),
            None,
            FhirVersion::default(),
        )
    }

    /// #519: the #456 boundary table over the real ES search path.
    #[tokio::test]
    async fn es_day_precision_date_boundaries() {
        let backend = create_backend().await;
        super::date_boundary_suite::day_precision_boundaries(&backend, "date-boundary-519").await;
    }

    /// #1293: a value that is not a date was read as the year 2000 and could
    /// match every resource; here it must be an error. Also pins that a
    /// second-precision value still covers a stored fraction now that the
    /// range is explicit instead of resting on `lte` round-up (#1297), and
    /// that a form-decoded `+` offset no longer reaches Elasticsearch as a
    /// `parse_exception` (#1296).
    #[tokio::test]
    async fn es_sub_day_date_precision_and_validation() {
        let backend = create_backend().await;
        super::date_precision_suite::sub_day_precision_and_validation(
            &backend,
            "date-precision-1293",
        )
        .await;
    }

    /// #1362: every contained resource is a document of its own here, so a
    /// repeated parameter is two clauses on one document.
    #[tokio::test]
    async fn es_contained_repeated_parameters_are_anded() {
        let backend = create_backend().await;
        super::contained_suite::repeated_parameters_are_anded(&backend, "contained-repeated-1362")
            .await;
    }

    /// #1363: `_`-parameters, composites and modifiers are applied under
    /// `_contained`, or refused by name — never dropped.
    #[tokio::test]
    async fn es_contained_criteria_are_applied_or_rejected() {
        let backend = create_backend().await;
        super::contained_suite::criteria_are_applied_or_rejected(
            &backend,
            "contained-criteria-1363",
        )
        .await;
    }

    /// #1383: `_contained` alone is every contained resource of the type;
    /// `_total`, `search_count` and paging agree; compartment membership is
    /// applied; `_has`, `_list` and chains are refused by name.
    #[tokio::test]
    async fn es_contained_unconstrained_and_out_of_band_constraints() {
        let backend = create_backend().await;
        super::contained_suite::unconstrained_and_out_of_band_constraints(
            &backend,
            "contained-gaps-1383",
        )
        .await;
    }

    /// #1407: `_sort` under `_contained` is applied or refused by name, and a
    /// contained resource with nothing indexed but its id is still found.
    #[tokio::test]
    async fn es_contained_sort_and_id_only_contained() {
        let backend = create_backend().await;
        super::contained_suite::sort_and_id_only_contained(&backend, "contained-sort-1407").await;
    }

    /// #1337: `1e2` is one significant figure, `[50, 150)`.
    #[tokio::test]
    async fn es_exponent_values_use_significant_figures() {
        let backend = create_backend().await;
        super::number_exponent_suite::exponent_values_use_significant_figures(
            &backend,
            "number-exponent-1337",
            true,
        )
        .await;
    }

    /// #1340: a number that did not parse made the handler return `None`,
    /// which the query builder filters out, so `probability=abc` returned
    /// every RiskAssessment; and `ltinf` matched every indexed row.
    #[tokio::test]
    async fn es_invalid_numbers_are_rejected_on_every_path() {
        let backend = create_backend().await;
        super::numeric_validation_suite::invalid_numbers_are_rejected_on_every_path(
            &backend,
            "numeric-validation-1340",
        )
        .await;
    }

    /// #1379: `gender=<system>|female` never matched a `code` element.
    #[tokio::test]
    async fn es_system_qualified_tokens_match_code_elements() {
        let backend = create_backend().await;
        super::token_code_system_suite::system_qualified_tokens_match_code_elements(
            &backend,
            "token-code-system-1379",
            true,
        )
        .await;
    }

    /// #1379: the same predicate as a chain terminal.
    #[tokio::test]
    async fn es_system_qualified_tokens_in_chains() {
        let backend = create_backend().await;
        super::token_code_system_suite::system_qualified_tokens_in_chains(
            &backend,
            "token-code-system-chain-1379",
        )
        .await;
    }

    /// #1380: `family=Zzz,` is a prefix match on `""`, which is every family
    /// name; an empty value or alternative is an error on every search path.
    #[tokio::test]
    async fn es_empty_values_are_rejected_on_every_path() {
        let backend = create_backend().await;
        super::empty_value_suite::empty_values_are_rejected_on_every_path(
            &backend,
            "empty-value-1380",
        )
        .await;
    }

    /// #1408: every modifier `SearchModifier::is_valid_for` allows, on every
    /// parameter type.
    #[tokio::test]
    async fn es_modifier_parity() {
        use super::modifier_parity_suite::{Divergence, Expect};

        let backend = create_backend().await;
        super::modifier_parity_suite::every_valid_modifier_agrees_across_backends(
            &backend,
            "modifier-parity-1408",
            &[
                // A short `:of-type` value adds no condition at all: every Patient.
                Divergence {
                    label: "Patient?identifier:ofType=MR|12345",
                    expect: Expect::Ids(&["p1", "p2", "p3", "p4"]),
                },
                Divergence {
                    label: "Patient?identifier:ofType=12345",
                    expect: Expect::Ids(&["p1", "p2", "p3", "p4"]),
                },
                // `:code-text` is a word-prefix match (`match_phrase_prefix`), not a
                // starts-with on the whole display.
                Divergence {
                    label: "Observation?code:code-text=rate",
                    expect: Expect::Ids(&["ob-pat"]),
                },
                // Terminology-backed token modifiers are not refused but degraded:
                // `:in` / `:not-in` match nothing, `:above` / `:below` match the code
                // itself. Unreachable over REST, which expands them or answers 501 first.
                Divergence {
                    label: "Observation?code:in=http://example.org/fhir/ValueSet/a",
                    expect: Expect::Ids(&[]),
                },
                Divergence {
                    label: "Observation?code:not-in=http://example.org/fhir/ValueSet/a",
                    expect: Expect::Ids(&[]),
                },
                Divergence {
                    label: "Observation?code:above=http://loinc.org|1234-5",
                    expect: Expect::Ids(&["ob-pat"]),
                },
                Divergence {
                    label: "Observation?code:below=http://loinc.org|1234-5",
                    expect: Expect::Ids(&["ob-pat"]),
                },
                // `:[type]` filters on the indexed `resource_type`, which a versioned
                // reference does not carry, and a bare id also matches an absolute URL.
                Divergence {
                    label: "Observation?subject:Patient=p1",
                    expect: Expect::Ids(&["ob-abs", "ob-pat"]),
                },
                Divergence {
                    label: "Observation?subject:Patient=Patient/p1",
                    expect: Expect::Ids(&["ob-pat"]),
                },
                Divergence {
                    label: "Observation?subject:Patient=Patient/p1/_history/2",
                    expect: Expect::Ids(&["ob-pat"]),
                },
                Divergence {
                    label: "Observation?subject:Patient=p1,nobody",
                    expect: Expect::Ids(&["ob-abs", "ob-pat"]),
                },
                Divergence {
                    label: "Observation?subject:Patient=p1&code=9999-9",
                    expect: Expect::Ids(&["ob-abs"]),
                },
                // `:identifier` looks for token rows under the reference parameter's own
                // name, which nothing writes: it never matches.
                Divergence {
                    label: "Observation?subject:identifier=http://example.org/mrn|12345",
                    expect: Expect::Ids(&[]),
                },
                Divergence {
                    label: "Observation?subject:identifier=http://example.org/mrn|",
                    expect: Expect::Ids(&[]),
                },
                Divergence {
                    label: "Observation?subject:identifier=12345",
                    expect: Expect::Ids(&[]),
                },
                Divergence {
                    label: "Observation?subject:identifier=http://example.org/mrn|12345&code=1234-5",
                    expect: Expect::Ids(&[]),
                },
            ],
        )
        .await;
    }

    // ========================================================================
    // Index-side date handling (#1314)
    // ========================================================================

    /// The ids a search returns.
    async fn found_ids<S>(
        backend: &S,
        tenant: &TenantContext,
        query: &helios_persistence::types::SearchQuery,
    ) -> std::collections::BTreeSet<String>
    where
        S: helios_persistence::core::SearchProvider,
    {
        backend
            .search(tenant, query)
            .await
            .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"))
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()
    }

    fn param_query(
        resource_type: &str,
        params: &[(&str, helios_persistence::types::SearchParamType, &str)],
    ) -> helios_persistence::types::SearchQuery {
        use helios_persistence::types::{SearchParameter, SearchQuery, SearchValue};
        params.iter().fold(
            SearchQuery::new(resource_type),
            |query, (name, param_type, value)| {
                query.with_parameter(SearchParameter {
                    name: name.to_string(),
                    param_type: *param_type,
                    values: vec![SearchValue::parse(value)],
                    ..Default::default()
                })
            },
        )
    }

    /// A Patient whose `birthDate` is not a date, beside a valid
    /// `deceasedDateTime` and a name.
    fn patient_with_one_bad_date(id: &str, family: &str) -> serde_json::Value {
        json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{ "family": family }],
            "birthDate": "2024-02-30",
            "deceasedDateTime": "2024-03-15T10:30:00Z"
        })
    }

    /// #1314: Elasticsearch rejects a whole document for one value its `date`
    /// mapping cannot parse, so a single bad date failed the write and left
    /// the resource unfindable by anything. The bad value must be skipped and
    /// everything else indexed — including the resource's other dates.
    #[tokio::test]
    async fn es_integration_unparseable_date_is_skipped_not_the_document() {
        use helios_persistence::types::SearchParamType::{Date, String as Str, Token};

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("bad-date-1314");

        backend
            .create(
                &tenant,
                "Patient",
                patient_with_one_bad_date("bad-birthdate", "Baddate"),
                FhirVersion::default(),
            )
            .await
            .expect("a bad date must not fail the write");

        // A Period with one bad end: the other end still indexes.
        backend
            .create(
                &tenant,
                "Encounter",
                json!({
                    "resourceType": "Encounter",
                    "id": "bad-period-start",
                    "status": "finished",
                    "class": { "system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB" },
                    "period": { "start": "not-a-date", "end": "2024-03-15" }
                }),
                FhirVersion::default(),
            )
            .await
            .expect("a bad Period.start must not fail the write");

        // The same through `update` and `create_or_update`.
        backend
            .create_or_update(
                &tenant,
                "Patient",
                "bad-upsert",
                patient_with_one_bad_date("bad-upsert", "Badupsert"),
                FhirVersion::default(),
            )
            .await
            .expect("a bad date must not fail create_or_update");

        let one = |id: &str| std::collections::BTreeSet::from([id.to_string()]);

        // Positive control and the other parameters.
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Patient", &[("family", Str, "Baddate")])
            )
            .await,
            one("bad-birthdate"),
            "found by a string parameter"
        );
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Patient", &[("family", Str, "Badupsert")])
            )
            .await,
            one("bad-upsert"),
        );
        // The other, valid date of the same resource.
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query(
                    "Patient",
                    &[
                        ("family", Str, "Baddate"),
                        ("death-date", Date, "2024-03-15")
                    ]
                )
            )
            .await,
            one("bad-birthdate"),
            "found by its valid date"
        );
        // The bad one indexed nothing: not the year 2000, not now, not March.
        assert!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Patient", &[("birthdate", Date, "ge0001")])
            )
            .await
            .is_empty(),
            "an unparseable birthDate must not be indexed as any date"
        );
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Encounter", &[("class", Token, "AMB")])
            )
            .await,
            one("bad-period-start"),
        );
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Encounter", &[("date", Date, "2024-03-15")])
            )
            .await,
            one("bad-period-start"),
            "the valid end of the Period is indexed"
        );
        assert!(
            found_ids(
                &backend,
                &tenant,
                &param_query("Encounter", &[("date", Date, "lt2024-03-15")])
            )
            .await
            .is_empty(),
            "the bad start of the Period is not"
        );
    }

    /// #1314: every valid FHIR date form indexes, at the instant the shared
    /// parser gives it, and is found by `eq`/`ge`/`lt` at every precision.
    /// Before, `:60` and a fraction longer than nine digits failed the write.
    #[tokio::test]
    async fn es_integration_every_fhir_date_form_indexes_and_is_searchable() {
        use helios_persistence::types::SearchParamType::{Date, Token};

        /// stored value → values that must match it, values that must not.
        const FORMS: &[(&str, &[&str], &[&str])] = &[
            (
                "2024",
                &[
                    "2024",
                    "2024-01",
                    "2024-01-01",
                    "ge2024",
                    "le2024",
                    "lt2025",
                    "2024-01-01T00:00:00Z",
                ],
                &["2023", "2024-02", "lt2024", "gt2024"],
            ),
            (
                "2024-03",
                &["2024-03", "2024", "2024-03-01", "ge2024-03", "lt2024-04"],
                &["2024-02", "2024-03-02", "lt2024-03", "gt2024-03"],
            ),
            (
                "2024-03-15",
                &[
                    "2024-03-15",
                    "2024-03",
                    "ge2024-03-15",
                    "lt2024-03-16",
                    "2024-03-15T00:00:00Z",
                ],
                &["2024-03-14", "lt2024-03-15", "gt2024-03-15"],
            ),
            // Minute precision: not the dateTime datatype, but valid in search.
            (
                "2024-03-15T10:30",
                &[
                    "2024-03-15T10:30",
                    "2024-03-15T10:30:00Z",
                    "2024-03-15",
                    "ge2024-03-15T10:30Z",
                ],
                &["2024-03-15T10:31", "lt2024-03-15T10:30Z"],
            ),
            (
                "2024-03-15T10:30Z",
                &["2024-03-15T10:30Z", "2024-03-15T10:30:00Z"],
                &["2024-03-15T10:29Z"],
            ),
            // Zone-less is UTC.
            (
                "2024-03-15T10:30:45",
                &[
                    "2024-03-15T10:30:45Z",
                    "2024-03-15T10:30",
                    "ge2024-03-15T10:30:45Z",
                    "lt2024-03-15T10:30:46Z",
                ],
                &[
                    "2024-03-15T10:30:46Z",
                    "lt2024-03-15T10:30:45Z",
                    "gt2024-03-15T10:30:45Z",
                ],
            ),
            (
                "2024-03-15T10:30:45Z",
                &["2024-03-15T10:30:45Z", "2024-03-15T16:00:45+05:30"],
                &["2024-03-15T10:30:44Z"],
            ),
            (
                "2024-03-15T10:30:45+05:30",
                &[
                    "2024-03-15T05:00:45Z",
                    "2024-03-15T10:30:45+05:30",
                    "2024-03-15",
                ],
                &["2024-03-15T10:30:45Z"],
            ),
            // 02:30 on the 16th, UTC.
            (
                "2024-03-15T22:30:45-04:00",
                &["2024-03-16T02:30:45Z", "2024-03-16", "ge2024-03-16"],
                &["2024-03-15", "lt2024-03-16"],
            ),
            (
                "2024-03-15T10:30:45+14:00",
                &["2024-03-14T20:30:45Z", "2024-03-14"],
                &["2024-03-15"],
            ),
            (
                "2024-03-15T10:30:45.1Z",
                &[
                    "2024-03-15T10:30:45.1Z",
                    "2024-03-15T10:30:45.100Z",
                    "2024-03-15T10:30:45Z",
                ],
                &["2024-03-15T10:30:45.2Z", "2024-03-15T10:30:45.0Z"],
            ),
            (
                "2024-03-15T10:30:45.123Z",
                &[
                    "2024-03-15T10:30:45.123Z",
                    "2024-03-15T10:30:45Z",
                    "ge2024-03-15T10:30:45.123Z",
                    "lt2024-03-15T10:30:45.124Z",
                ],
                &["2024-03-15T10:30:45.124Z", "lt2024-03-15T10:30:45.123Z"],
            ),
            // Finer than a millisecond: held at the millisecond, and found by
            // the value it was written as.
            (
                "2024-03-15T10:30:45.123456Z",
                &["2024-03-15T10:30:45.123456Z", "2024-03-15T10:30:45.123Z"],
                &["2024-03-15T10:30:45.124Z"],
            ),
            (
                "2024-03-15T10:30:45.123456789Z",
                &["2024-03-15T10:30:45.123456789Z", "2024-03-15T10:30:45.123Z"],
                &["2024-03-15T10:30:45.122Z"],
            ),
            (
                "2024-03-15T10:30:45.1234567891Z",
                &[
                    "2024-03-15T10:30:45.1234567891Z",
                    "2024-03-15T10:30:45.123Z",
                ],
                &["2024-03-15T10:30:45.124Z"],
            ),
            // A leap second is the first instant of the next second.
            (
                "2016-12-31T23:59:60Z",
                &[
                    "2017-01-01T00:00:00Z",
                    "2017",
                    "2016-12-31T23:59:60Z",
                    "ge2017",
                ],
                &["2016-12-31", "2016", "lt2017"],
            ),
            (
                "2024-03-15T10:30:60+05:30",
                &["2024-03-15T05:01:00Z"],
                &["2024-03-15T05:00:59Z"],
            ),
            // Not FHIR, but Elasticsearch took them as written and must go on
            // indexing them where it did.
            (
                "2024-03-15T10Z",
                &["2024-03-15T10:00:00Z"],
                &["2024-03-15T11:00:00Z"],
            ),
            (
                "2024-03-15T10:30:45+0530",
                &["2024-03-15T05:00:45Z"],
                &["2024-03-15T10:30:45Z"],
            ),
            (
                "2024-03-15T10:30:45,123Z",
                &["2024-03-15T10:30:45.123Z"],
                &["2024-03-15T10:30:45.124Z"],
            ),
        ];

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("date-forms-1314");

        for (i, (stored, _, _)) in FORMS.iter().enumerate() {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": format!("form-{i}"),
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": format!("form-{i}") }] },
                        "effectiveDateTime": stored
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("effectiveDateTime {stored} failed the write: {e}"));
        }

        for (i, (stored, matching, not_matching)) in FORMS.iter().enumerate() {
            let id = format!("form-{i}");
            let search = |date: &'static str| {
                let query =
                    param_query("Observation", &[("code", Token, &id), ("date", Date, date)]);
                let (backend, tenant) = (&backend, &tenant);
                async move { found_ids(backend, tenant, &query).await }
            };
            // Positive control: the resource is there, with a date.
            assert_eq!(
                search("ge0001").await.len(),
                1,
                "stored {stored}: not indexed with a date"
            );
            for date in *matching {
                assert_eq!(
                    search(date).await.len(),
                    1,
                    "stored {stored}: date={date} must match"
                );
            }
            for date in *not_matching {
                assert!(
                    search(date).await.is_empty(),
                    "stored {stored}: date={date} must not match"
                );
            }
        }
    }

    /// #1314: a date that is a composite component goes through the same
    /// normalisation: a leap second indexes, and a value that is not a date
    /// costs the instance its date component, not the resource its document.
    #[tokio::test]
    async fn es_integration_composite_date_component_is_normalised_or_skipped() {
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("composite-date-1314");

        for (id, value) in [("leap", "2016-12-31T23:59:60Z"), ("garbage", "not-a-date")] {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": id }] },
                        "valueDateTime": value
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("valueDateTime {value} failed the write: {e}"));
        }

        let composite = |value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "code-value-date".to_string(),
                param_type: SearchParamType::Composite,
                values: vec![SearchValue::eq(value)],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "code".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Date,
                        param_name: "value-date".to_string(),
                    },
                ],
                ..Default::default()
            })
        };
        let one = |id: &str| std::collections::BTreeSet::from([id.to_string()]);

        assert_eq!(
            found_ids(&backend, &tenant, &composite("leap$2017-01-01T00:00:00Z")).await,
            one("leap")
        );
        assert!(
            found_ids(&backend, &tenant, &composite("leap$2016-12-31"))
                .await
                .is_empty()
        );
        assert_eq!(
            found_ids(
                &backend,
                &tenant,
                &param_query(
                    "Observation",
                    &[("code", SearchParamType::Token, "garbage")]
                )
            )
            .await,
            one("garbage"),
            "a bad date component must not cost the resource its document"
        );
        assert!(
            found_ids(&backend, &tenant, &composite("garbage$ge0001"))
                .await
                .is_empty()
        );
    }

    /// #1314 in every `HFS_COMPOSITE_SYNC_MODE`. The composite never failed
    /// the client's write — a secondary's rejection is only logged — so the
    /// symptom there was a resource stored in the primary and missing from
    /// every search, in all three modes.
    #[cfg(feature = "sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn es_integration_composite_bad_date_stays_searchable_in_every_sync_mode() {
        use std::collections::HashMap;

        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
        };
        use helios_persistence::types::SearchParamType::{Date, String as Str};

        let es = shared_es().await;
        let modes = [
            ("sync", SyncMode::Synchronous),
            ("async", SyncMode::Asynchronous),
            (
                "hybrid",
                SyncMode::Hybrid {
                    sync_for_search: true,
                },
            ),
        ];

        for (label, mode) in modes {
            let es_config = ElasticsearchConfig {
                nodes: vec![format!("http://{}:{}", es.host, es.port)],
                index_prefix: format!("hfs_{}", uuid::Uuid::new_v4().simple()),
                number_of_replicas: 0,
                refresh_interval: "1ms".to_string(),
                write_refresh: WriteRefreshPolicy::WaitFor,
                ..Default::default()
            };
            let es_backend = Arc::new(
                ElasticsearchBackend::with_shared_registry(es_config, build_search_registry())
                    .expect("create ES backend"),
            );
            es_backend
                .initialize()
                .await
                .expect("initialize ES backend");

            let sqlite = Arc::new(SqliteBackend::in_memory().expect("create SQLite backend"));
            sqlite.init_schema().expect("init SQLite schema");

            let composite_config = CompositeConfig::builder()
                .primary("sqlite", BackendKind::Sqlite)
                .search_backend("es", BackendKind::Elasticsearch)
                .sync_mode(mode)
                .build()
                .expect("build composite config");

            let mut backends: HashMap<String, DynStorage> = HashMap::new();
            backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
            backends.insert("es".to_string(), es_backend.clone() as DynStorage);

            let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
            search_providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
            search_providers.insert("es".to_string(), es_backend.clone() as DynSearchProvider);

            let composite = CompositeStorage::new(composite_config, backends)
                .expect("create composite storage")
                .with_search_providers(search_providers)
                .with_full_primary(sqlite)
                .start_sync_workers();

            let tenant = create_tenant("bad-date-composite-1314");
            composite
                .create(
                    &tenant,
                    "Patient",
                    patient_with_one_bad_date("bad-birthdate", "Baddate"),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("[{label}] create through composite: {e}"));

            // The asynchronous worker indexes behind the write; poll for it.
            let by_name = param_query("Patient", &[("family", Str, "Baddate")]);
            let mut found = std::collections::BTreeSet::new();
            for _ in 0..50 {
                found = found_ids(&composite, &tenant, &by_name).await;
                if !found.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            assert_eq!(
                found,
                std::collections::BTreeSet::from(["bad-birthdate".to_string()]),
                "[{label}] a resource with one bad date must stay searchable"
            );
            assert_eq!(
                found_ids(
                    &composite,
                    &tenant,
                    &param_query("Patient", &[("death-date", Date, "2024-03-15T10:30:00Z")])
                )
                .await
                .len(),
                1,
                "[{label}] found by its valid date"
            );
        }
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

    /// #1050: Elasticsearch's default nested-object limit of 10000 rejects the
    /// whole document, so a resource with more indexed values than that was
    /// stored but never searchable. With the raised default it indexes through
    /// the `$reindex` page writer.
    #[tokio::test]
    async fn es_integration_resource_over_elasticsearch_default_nested_limit_indexes() {
        use helios_persistence::search::ReindexTarget;

        let backend = create_backend().await;
        let tenant = create_tenant("nested-limit-default");
        let provenance = oversized_provenance(&tenant, "oversized", 12_000);

        let outcomes = backend
            .write_search_entries_page(&tenant, std::slice::from_ref(&provenance))
            .await;
        assert!(
            outcomes[0].is_ok(),
            "a Provenance with 12000 targets must index: {:?}",
            outcomes[0]
        );
        assert!(
            backend
                .read(&tenant, "Provenance", "oversized")
                .await
                .unwrap()
                .is_some(),
            "the document must be in the index"
        );
    }

    /// #1050: the index template only reaches indices created after it, so an
    /// index that already existed at Elasticsearch's limit of 10000 kept
    /// rejecting large resources. Starting a backend with the raised limit must
    /// raise it on that existing index. `ensure_index` never touches an index
    /// that exists, so only the startup pass can explain the second write
    /// succeeding.
    #[tokio::test]
    async fn es_integration_startup_raises_nested_limit_on_existing_index() {
        use helios_persistence::search::ReindexTarget;

        let prefix = format!("hfs_{}", uuid::Uuid::new_v4().simple());
        let tenant = create_tenant("nested-limit-existing");
        let provenance = oversized_provenance(&tenant, "oversized", 12_000);

        // An index created under Elasticsearch's own limit rejects it.
        let before = create_backend_with_nested_limit(&prefix, 10_000).await;
        let rejected = before
            .write_search_entries_page(&tenant, std::slice::from_ref(&provenance))
            .await;
        let error = rejected[0]
            .as_ref()
            .expect_err("a limit of 10000 must reject 12000 nested objects");
        assert!(
            error.to_string().contains("nested"),
            "the rejection must be the nested-object limit, got: {error}"
        );
        // A rejection, not an outage: `$reindex` must not retry it (#1050).
        assert!(
            matches!(
                error,
                helios_persistence::error::StorageError::Backend(
                    helios_persistence::error::BackendError::Internal { .. }
                )
            ),
            "the nested-object rejection must be permanent, got: {error:?}"
        );
        assert!(
            before
                .read(&tenant, "Provenance", "oversized")
                .await
                .unwrap()
                .is_none()
        );

        // A backend started with the raised limit fixes the existing index.
        let after = create_backend_with_nested_limit(&prefix, 50_000).await;
        let outcomes = after
            .write_search_entries_page(&tenant, std::slice::from_ref(&provenance))
            .await;
        assert!(
            outcomes[0].is_ok(),
            "the raised index must accept it: {:?}",
            outcomes[0]
        );
        assert!(
            after
                .read(&tenant, "Provenance", "oversized")
                .await
                .unwrap()
                .is_some()
        );
    }

    /// `$reindex` walks a page at a time and, before #1021, Elasticsearch used
    /// the default trait implementation: one HTTP round trip per resource. This
    /// pins the batched override — every resource of the page indexed, its own
    /// version_id carried, and per-resource outcomes still reported in order —
    /// so the batching can never silently drop or reorder a page.
    #[tokio::test]
    async fn es_integration_reindex_page_writes_every_resource() {
        use helios_persistence::search::ReindexTarget;
        use helios_persistence::types::StoredResource;

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-page-tenant");

        let resources: Vec<StoredResource> = (0..25)
            .map(|n| {
                StoredResource::from_storage(
                    "Patient",
                    format!("page-{n}"),
                    "7",
                    tenant.tenant_id().clone(),
                    json!({
                        "resourceType": "Patient",
                        "id": format!("page-{n}"),
                        "name": [{"family": format!("Paged{n}")}]
                    }),
                    chrono::Utc::now(),
                    chrono::Utc::now(),
                    None,
                    FhirVersion::default(),
                )
            })
            .collect();

        let outcomes = backend.write_search_entries_page(&tenant, &resources).await;
        assert_eq!(
            outcomes.len(),
            resources.len(),
            "one outcome per resource, in page order"
        );
        for (n, outcome) in outcomes.iter().enumerate() {
            assert!(outcome.is_ok(), "resource {n} failed: {outcome:?}");
        }

        // Every document really landed, under its own id and version.
        for n in [0usize, 12, 24] {
            let stored = backend
                .read(&tenant, "Patient", &format!("page-{n}"))
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("page-{n} must be indexed"));
            assert_eq!(
                stored.version_id(),
                "7",
                "the resource's own version is kept"
            );
            assert_eq!(
                stored.content()["name"][0]["family"],
                json!(format!("Paged{n}"))
            );
        }
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn es_integration_pg_source_retry_by_ids() {
        use std::collections::HashSet;
        use std::sync::Mutex;

        use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
        use helios_persistence::core::{DeferredReindexHook, SearchProvider};
        use helios_persistence::error::{BackendError, StorageResult};
        use helios_persistence::search::{
            ReindexOnFinish, ReindexOperation, ReindexSource, ReindexStatus, ReindexTarget,
            ResourcePage,
        };
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
        };
        use tokio::sync::oneshot;

        struct ObservedPgSource {
            pg: Arc<PostgresBackend>,
            requested: Mutex<Vec<Vec<String>>>,
            returned: Mutex<Vec<Vec<String>>>,
            first_lookup: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
        }

        #[async_trait::async_trait]
        impl ReindexSource for ObservedPgSource {
            async fn list_resource_types(
                &self,
                tenant: &TenantContext,
            ) -> StorageResult<Vec<String>> {
                self.pg.list_resource_types(tenant).await
            }

            async fn count_resources(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
            ) -> StorageResult<u64> {
                self.pg.count_resources(tenant, resource_type).await
            }

            async fn fetch_resources_page(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                cursor: Option<&str>,
                limit: u32,
            ) -> StorageResult<ResourcePage> {
                self.pg
                    .fetch_resources_page(tenant, resource_type, cursor, limit)
                    .await
            }

            async fn fetch_resources_by_ids(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                ids: &[String],
            ) -> StorageResult<Vec<StoredResource>> {
                self.requested.lock().unwrap().push(ids.to_vec());
                let barrier = self.first_lookup.lock().unwrap().take();
                if let Some((entered, release)) = barrier {
                    entered.send(()).expect("test receives retry lookup signal");
                    release.await.expect("test releases retry lookup");
                }
                let resources = self
                    .pg
                    .fetch_resources_by_ids(tenant, resource_type, ids)
                    .await?;
                self.returned.lock().unwrap().push(
                    resources
                        .iter()
                        .map(|resource| resource.id().to_string())
                        .collect(),
                );
                Ok(resources)
            }
        }

        struct FailingEsTarget {
            es: Arc<ElasticsearchBackend>,
            fail_once: Mutex<HashSet<String>>,
            attempts: Mutex<Vec<String>>,
        }

        #[async_trait::async_trait]
        impl ReindexTarget for FailingEsTarget {
            async fn delete_search_entries(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                id: &str,
            ) -> StorageResult<u64> {
                self.es
                    .delete_search_entries(tenant, resource_type, id)
                    .await
            }

            async fn write_search_entries(
                &self,
                tenant: &TenantContext,
                resource: &StoredResource,
            ) -> StorageResult<usize> {
                let id = resource.id().to_string();
                self.attempts.lock().unwrap().push(id.clone());
                if self.fail_once.lock().unwrap().remove(&id) {
                    return Err(BackendError::Unavailable {
                        backend_name: "elasticsearch".into(),
                        message: "injected initial write failure".into(),
                    }
                    .into());
                }
                self.es.write_search_entries(tenant, resource).await
            }

            async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
                self.es.clear_search_index(tenant).await
            }
        }

        let pg_fixture = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .join("data");
        let mut pg = PostgresBackend::new(PostgresConfig {
            host: pg_fixture.host.clone(),
            port: pg_fixture.port,
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            data_dir: Some(data_dir),
            ..Default::default()
        })
        .await
        .unwrap();
        pg.init_schema().await.unwrap();
        pg.set_search_offloaded(true);
        let pg = Arc::new(pg);

        let es_fixture = shared_es().await;
        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![format!("http://{}:{}", es_fixture.host, es_fixture.port)],
                    index_prefix: format!("hfs_pg_retry_{}", uuid::Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    refresh_interval: "1ms".into(),
                    write_refresh: WriteRefreshPolicy::WaitFor,
                    ..Default::default()
                },
                pg.tenant_registries().clone(),
            )
            .unwrap(),
        );
        es.initialize().await.unwrap();

        let tenant = create_tenant(&format!("pg-es-retry-{}", uuid::Uuid::new_v4().simple()));
        for id in ["retry-update", "retry-delete", "unrelated"] {
            pg.create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":id,"name":[{"family":"Before"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        }

        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let source = Arc::new(ObservedPgSource {
            pg: pg.clone(),
            requested: Mutex::new(Vec::new()),
            returned: Mutex::new(Vec::new()),
            first_lookup: Mutex::new(Some((entered_tx, release_rx))),
        });
        let target = Arc::new(FailingEsTarget {
            es: es.clone(),
            fail_once: Mutex::new(HashSet::from([
                "retry-update".into(),
                "retry-delete".into(),
            ])),
            attempts: Mutex::new(Vec::new()),
        });
        let operation = Arc::new(ReindexOperation::with_parts(
            source.clone(),
            vec![pg.clone(), target.clone()],
            pg.tenant_registries().clone(),
        ));
        ReindexOnFinish::new(operation.clone())
            .with_batch_size(10)
            .reindex_types(&tenant, vec!["Patient".into()])
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(30), entered_rx)
            .await
            .expect("automatic retry did not fetch by ID")
            .expect("retry lookup signal dropped");

        let original = pg
            .read(&tenant, "Patient", "retry-update")
            .await
            .unwrap()
            .unwrap();
        pg.update(
            &tenant,
            &original,
            json!({"resourceType":"Patient","id":"retry-update","name":[{"family":"After"}]}),
        )
        .await
        .unwrap();
        pg.delete(&tenant, "Patient", "retry-delete").await.unwrap();
        release_tx.send(()).unwrap();

        let jobs = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let jobs = operation.list_jobs();
                if jobs.len() >= 2 && jobs.iter().all(|job| job.status.is_finished()) {
                    break jobs;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("pg-es automatic retry did not finish");
        assert_eq!(jobs.len(), 2);
        let retry = jobs.iter().find(|job| job.resource_scoped).unwrap();
        assert_eq!(retry.status, ReindexStatus::Completed);
        assert_eq!(retry.total_resources, 2);
        assert_eq!(retry.processed_resources, 2);

        let requested = source.requested.lock().unwrap().clone();
        assert_eq!(requested.len(), 1);
        let mut ids = requested[0].clone();
        ids.sort();
        assert_eq!(ids, ["retry-delete", "retry-update"]);
        assert_eq!(
            *source.returned.lock().unwrap(),
            vec![vec!["retry-update".to_string()]]
        );
        let attempts = target.attempts.lock().unwrap().clone();
        assert_eq!(
            attempts.iter().filter(|id| *id == "retry-update").count(),
            2
        );
        assert_eq!(
            attempts.iter().filter(|id| *id == "retry-delete").count(),
            1
        );
        assert_eq!(attempts.iter().filter(|id| *id == "unrelated").count(), 1);

        let updated = es
            .read(&tenant, "Patient", "retry-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.content()["name"][0]["family"], "After");
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".into(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("After")],
            chain: vec![],
            components: vec![],
        });
        let search = es.search(&tenant, &query).await.unwrap();
        assert_eq!(search.resources.items.len(), 1);
        assert_eq!(search.resources.items[0].id(), "retry-update");
        assert!(
            es.read(&tenant, "Patient", "retry-delete")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A resource contributes more than one document when it has `contained`
    /// entries, and the batched page writer has to flatten all of them into the
    /// one `_bulk` request while still reporting a single outcome per resource.
    /// Getting that wrong loses contained resources from `_contained` search on
    /// every rebuild — silently, since the container itself still indexes.
    #[tokio::test]
    async fn es_integration_reindex_page_indexes_contained_resources() {
        use helios_persistence::search::ReindexTarget;
        use helios_persistence::types::StoredResource;

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-contained-tenant");

        let with_contained = StoredResource::from_storage(
            "Observation",
            "obs-contained",
            "3",
            tenant.tenant_id().clone(),
            json!({
                "resourceType": "Observation",
                "id": "obs-contained",
                "status": "final",
                "contained": [{
                    "resourceType": "Patient",
                    "id": "inner",
                    "name": [{"family": "Contained"}]
                }],
                "subject": {"reference": "#inner"},
                "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]}
            }),
            chrono::Utc::now(),
            chrono::Utc::now(),
            None,
            FhirVersion::default(),
        );

        let outcomes = backend
            .write_search_entries_page(&tenant, std::slice::from_ref(&with_contained))
            .await;
        assert_eq!(
            outcomes.len(),
            1,
            "one outcome per resource, not per document"
        );
        assert!(outcomes[0].is_ok(), "{:?}", outcomes[0]);

        assert!(
            backend
                .read(&tenant, "Observation", "obs-contained")
                .await
                .unwrap()
                .is_some(),
            "the container is indexed"
        );
    }

    /// An empty page is a no-op rather than an empty `_bulk` request, which
    /// Elasticsearch rejects.
    #[tokio::test]
    async fn es_integration_reindex_empty_page_is_a_no_op() {
        use helios_persistence::search::ReindexTarget;

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-empty-tenant");
        assert!(
            backend
                .write_search_entries_page(&tenant, &[])
                .await
                .is_empty()
        );
    }

    /// A backend whose `_bulk` bodies are capped at `bulk_max_bytes`, so a test
    /// can cross the byte cap without megabytes of fixtures.
    async fn create_backend_with_bulk_max_bytes(bulk_max_bytes: usize) -> ElasticsearchBackend {
        let es = shared_es().await;
        let config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: format!("hfs_{}", uuid::Uuid::new_v4().simple()),
            number_of_replicas: 0,
            refresh_interval: "1ms".to_string(),
            bulk_max_bytes,
            ..Default::default()
        };
        let backend = ElasticsearchBackend::with_shared_registry(config, build_search_registry())
            .expect("Failed to create ElasticsearchBackend");
        backend
            .initialize()
            .await
            .expect("Failed to initialize ES backend");
        backend
    }

    /// Polls until Elasticsearch counts exactly `expected` resources of
    /// `resource_type` for `tenant`, so a refresh lag cannot fail the test.
    async fn await_es_count(
        backend: &ElasticsearchBackend,
        tenant: &TenantContext,
        resource_type: &str,
        expected: u64,
    ) {
        let mut counted = 0;
        for _ in 0..100 {
            counted = backend.count(tenant, Some(resource_type)).await.unwrap();
            if counted == expected {
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        panic!("Elasticsearch counted {counted} {resource_type} resources, expected {expected}");
    }

    /// #1125: `_bulk` was chunked by 500 operations with no byte cap, so a page
    /// of large resources became one oversized request that failed as a whole
    /// at the transport level. A page above both the operation count and the
    /// byte cap must index every resource, including one document that alone
    /// is larger than the cap and therefore has to travel in its own request.
    #[tokio::test]
    async fn es_integration_reindex_page_over_op_and_byte_caps_indexes_entirely() {
        use helios_persistence::search::ReindexTarget;
        use helios_persistence::types::StoredResource;

        const BULK_MAX_BYTES: usize = 64 * 1024;
        const PATIENTS: usize = 1200;

        let backend = create_backend_with_bulk_max_bytes(BULK_MAX_BYTES).await;
        let tenant = create_tenant("reindex-bulk-caps");
        let padding = "x".repeat(512);
        let now = chrono::Utc::now();

        let mut page: Vec<StoredResource> = (0..PATIENTS)
            .map(|n| {
                StoredResource::from_storage(
                    "Patient",
                    format!("capped-{n}"),
                    "1",
                    tenant.tenant_id().clone(),
                    json!({
                        "resourceType": "Patient",
                        "id": format!("capped-{n}"),
                        "name": [{"family": format!("Capped{n}")}],
                        "text": {
                            "status": "generated",
                            "div": format!("<div xmlns=\"http://www.w3.org/1999/xhtml\">{padding}</div>")
                        }
                    }),
                    now,
                    now,
                    None,
                    FhirVersion::default(),
                )
            })
            .collect();
        let oversized = oversized_provenance(&tenant, "over-the-cap", 4_000);
        assert!(
            serde_json::to_vec(oversized.content()).unwrap().len() > BULK_MAX_BYTES,
            "precondition: the Provenance alone exceeds the byte cap"
        );
        page.insert(PATIENTS / 2, oversized);

        let page_bytes: usize = page
            .iter()
            .map(|r| serde_json::to_vec(r.content()).unwrap().len())
            .sum();
        assert!(
            page.len() > 500,
            "precondition: more operations than one count-capped request"
        );
        assert!(
            page_bytes > 4 * BULK_MAX_BYTES,
            "precondition: the page spans several byte-capped requests ({page_bytes} bytes)"
        );

        let outcomes = backend.write_search_entries_page(&tenant, &page).await;
        assert_eq!(
            outcomes.len(),
            page.len(),
            "one outcome per resource, in page order"
        );
        let failed: Vec<String> = outcomes
            .iter()
            .zip(&page)
            .filter_map(|(outcome, resource)| {
                outcome
                    .as_ref()
                    .err()
                    .map(|e| format!("{}/{}: {e}", resource.resource_type(), resource.id()))
            })
            .collect();
        assert!(
            failed.is_empty(),
            "{} of {} resources failed to index, first: {:?}",
            failed.len(),
            page.len(),
            failed.first()
        );

        await_es_count(&backend, &tenant, "Patient", PATIENTS as u64).await;
        for n in [0, PATIENTS / 2, PATIENTS - 1] {
            assert!(
                backend
                    .read(&tenant, "Patient", &format!("capped-{n}"))
                    .await
                    .unwrap()
                    .is_some(),
                "capped-{n} must be indexed"
            );
        }
        assert!(
            backend
                .read(&tenant, "Provenance", "over-the-cap")
                .await
                .unwrap()
                .is_some(),
            "a document larger than the cap must be sent alone, not dropped"
        );
    }

    /// #1125: `sqlite-es` offloads SQLite's search to Elasticsearch, yet the
    /// rebuild wrote a `search_index`/FTS pair into SQLite that no query reads,
    /// and — the delete side being guarded — accumulated it on every rerun.
    /// With the real offloaded SQLite primary still wired as a writer next to a
    /// real Elasticsearch, a rebuild (plain, then with `clearExisting`) must put
    /// every resource in Elasticsearch and leave both SQLite tables empty.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn es_integration_sqlite_es_reindex_writes_only_elasticsearch() {
        use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
        use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};

        const RESOURCES: usize = 25;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fhir.db");
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let count_rows = |table: &str| -> i64 {
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };

        let sqlite = SqliteBackend::with_config(
            &path,
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                search_offloaded: true,
                ..Default::default()
            },
        )
        .expect("Failed to create SQLite backend");
        sqlite.init_schema().expect("Failed to initialize schema");
        let sqlite = Arc::new(sqlite);
        let es = Arc::new(create_backend().await);
        let tenant = create_tenant("sqlite-es-reindex");

        for n in 0..RESOURCES {
            sqlite
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("sqlite-es-{n}"),
                        "name": [{"family": "Offloaded"}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        assert_eq!(
            count_rows("search_index"),
            0,
            "precondition: an offloaded create indexes nothing locally"
        );
        assert_eq!(count_rows("resource_fts"), 0);

        let op = ReindexOperation::with_parts(
            sqlite.clone(),
            vec![sqlite.clone(), es.clone()],
            sqlite.tenant_registries().clone(),
        );

        for (run, clear_existing) in [(1, false), (2, true)] {
            let mut request = ReindexRequest::for_types(vec!["Patient"]).with_batch_size(10);
            if clear_existing {
                request = request.clear_existing();
            }
            let job_id = op.start(tenant.clone(), request, None).await.unwrap();
            let mut finished = None;
            for _ in 0..600 {
                let progress = op.get_progress(&job_id).await.unwrap();
                if progress.status.is_finished() {
                    finished = Some(progress);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            let progress = finished.unwrap_or_else(|| panic!("run {run}: reindex timed out"));
            assert_eq!(
                progress.status,
                ReindexStatus::Completed,
                "run {run}: {:?}",
                progress.error_message
            );
            assert!(
                progress.errors.is_empty(),
                "run {run}: {:?}",
                progress.errors
            );
            assert_eq!(progress.processed_resources, RESOURCES as u64, "run {run}");

            assert_eq!(
                count_rows("search_index"),
                0,
                "run {run}: the rebuild must not write SQLite's dead search_index"
            );
            assert_eq!(
                count_rows("resource_fts"),
                0,
                "run {run}: the rebuild must not write SQLite's dead FTS table"
            );
            await_es_count(&es, &tenant, "Patient", RESOURCES as u64).await;
        }
        assert!(
            es.read(&tenant, "Patient", "sqlite-es-0")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn es_integration_pg_source_capped_reindex_coverage() {
        use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
        use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};
        use helios_persistence::types::SearchParamType;

        let pg_fixture = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .join("data");
        let mut pg = PostgresBackend::new(PostgresConfig {
            host: pg_fixture.host.clone(),
            port: pg_fixture.port,
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            dbname: "postgres".to_string(),
            data_dir: Some(data_dir),
            search_offloaded: true,
            ..Default::default()
        })
        .await
        .unwrap();
        pg.init_schema().await.unwrap();
        pg.set_search_offloaded(true);
        let pg = Arc::new(pg);

        let es_fixture = shared_es().await;
        let make_es = || {
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![format!("http://{}:{}", es_fixture.host, es_fixture.port)],
                    index_prefix: format!("hfs_{}", uuid::Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    refresh_interval: "1ms".to_string(),
                    write_refresh: WriteRefreshPolicy::WaitFor,
                    ..Default::default()
                },
                pg.tenant_registries().clone(),
            )
            .unwrap()
        };
        let capped_es = Arc::new(make_es());
        let uncapped_es = Arc::new(make_es());
        capped_es.initialize().await.unwrap();
        uncapped_es.initialize().await.unwrap();
        let capped_tenant = create_tenant(&format!("pg-es-cap-{}", uuid::Uuid::new_v4()));
        let uncapped_tenant = create_tenant(&format!("pg-es-base-{}", uuid::Uuid::new_v4()));
        for tenant in [&capped_tenant, &uncapped_tenant] {
            for n in 0..5 {
                let id = format!("pg-es-{n}");
                pg.create(
                    tenant,
                    "Patient",
                    json!({
                        "resourceType":"Patient",
                        "id":id,
                        "name":[{"family":format!("Family{n}")}],
                        "text":{"status":"generated","div":format!("<div>{}</div>", "x".repeat(if n == 2 { 4000 } else { 10 }))}
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            }
        }
        let client = pg.get_client().await.unwrap();
        let sizes: Vec<i64> = client
            .query(
                "SELECT octet_length(data::text)::bigint FROM resources
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                 ORDER BY last_updated, id",
                &[&capped_tenant.tenant_id().as_str()],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(sizes.len(), 5);
        let cap = (sizes[0] + sizes[1]) as u64;
        assert!(sizes.iter().sum::<i64>() as u64 > cap);
        assert!(sizes.iter().any(|size| *size as u64 > cap));
        drop(client);

        for (tenant, es, bytes) in [
            (&capped_tenant, &capped_es, cap),
            (&uncapped_tenant, &uncapped_es, 0),
        ] {
            let started = std::time::Instant::now();
            let op = ReindexOperation::with_parts(
                pg.clone(),
                vec![pg.clone(), es.clone()],
                pg.tenant_registries().clone(),
            );
            let job = op
                .start(
                    tenant.clone(),
                    ReindexRequest::for_types(vec!["Patient"])
                        .with_batch_size(5)
                        .with_batch_bytes(bytes),
                    None,
                )
                .await
                .unwrap();
            let mut completed = None;
            for _ in 0..600 {
                let progress = op.get_progress(&job).await.unwrap();
                if progress.status.is_finished() {
                    completed = Some(progress);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            let progress = completed.expect("pg-es reindex timed out");
            assert_eq!(
                progress.status,
                ReindexStatus::Completed,
                "{:?}",
                progress.error_message
            );
            assert!(progress.errors.is_empty(), "{:?}", progress.errors);
            assert_eq!(progress.processed_resources, 5);
            let terminal_ms = started.elapsed().as_millis();
            await_es_count(es, tenant, "Patient", 5).await;
            for n in 0..5 {
                let id = format!("pg-es-{n}");
                assert!(es.read(tenant, "Patient", &id).await.unwrap().is_some());
                let family = format!("Family{n}");
                let found = found_ids(
                    es.as_ref(),
                    tenant,
                    &param_query("Patient", &[("name", SearchParamType::String, &family)]),
                )
                .await;
                assert!(found.contains(&id), "{family} did not find {id}");
            }
            if std::env::var_os("HFS_1459_MEASURE").is_some() {
                println!(
                    "pg-es measurement: byte_cap={bytes} terminal_ms={terminal_ms} search_ready_ms={}",
                    started.elapsed().as_millis()
                );
            }
        }
        let client = pg.get_client().await.unwrap();
        let mut snapshots = Vec::new();
        for tenant in [&capped_tenant, &uncapped_tenant] {
            let index: i64 = client
                .query_one(
                    "SELECT count(*) FROM search_index WHERE tenant_id = $1",
                    &[&tenant.tenant_id().as_str()],
                )
                .await
                .unwrap()
                .get(0);
            let fts: i64 = client
                .query_one(
                    "SELECT count(*) FROM resource_fts WHERE tenant_id = $1",
                    &[&tenant.tenant_id().as_str()],
                )
                .await
                .unwrap()
                .get(0);
            snapshots.push((index, fts));
        }
        assert!(
            snapshots[0].0 > 0,
            "PostgreSQL target wrote no search entries"
        );
        assert!(snapshots[0].1 > 0, "PostgreSQL target wrote no FTS entries");
        assert_eq!(snapshots[0], snapshots[1]);
    }

    /// What the #1161 tests share: an offloaded SQLite primary and a real
    /// Elasticsearch behind a composite, the `$bulk-submit` job store wired as
    /// `hfs` wires it for deferred indexing — the composite wrapper with its
    /// per-resource ingest sync off — and one manifest of `resources` Patients
    /// ingested through that store and then indexed by the deferred rebuild.
    #[cfg(feature = "sqlite")]
    struct DeferredSubmit {
        _dir: tempfile::TempDir,
        sqlite: Arc<helios_persistence::backends::sqlite::SqliteBackend>,
        es: Arc<ElasticsearchBackend>,
        jobs: helios_persistence::composite::CompositeSubmitJobs,
        tenant: TenantContext,
        submission: helios_persistence::core::SubmissionId,
    }

    /// Builds [`DeferredSubmit`]: ingests `resources` Patients with indexing
    /// deferred, asserts the ingest itself put nothing in Elasticsearch, runs
    /// the per-type rebuild the worker fires after the manifest, and waits
    /// until Elasticsearch holds every one of them.
    #[cfg(feature = "sqlite")]
    async fn deferred_submit_rebuilt(tenant_name: &str, resources: usize) -> DeferredSubmit {
        use std::collections::HashMap;

        use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, CompositeSubmitJobs, DynStorage, SyncMode,
        };
        use helios_persistence::core::{
            BulkProcessingOptions, BulkSubmitJobStore, BulkSubmitProvider, NdjsonEntry,
            SubmissionId, SubmitClaimStrategy, SubmitWorkerStorage, WorkerId,
        };
        use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};

        let dir = tempfile::tempdir().unwrap();
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let sqlite = SqliteBackend::with_config(
            dir.path().join("fhir.db"),
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                search_offloaded: true,
                ..Default::default()
            },
        )
        .expect("Failed to create SQLite backend");
        sqlite.init_schema().expect("Failed to initialize schema");
        let sqlite = Arc::new(sqlite);
        let es = Arc::new(create_backend().await);
        let tenant = create_tenant(tenant_name);

        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));
        let jobs =
            CompositeSubmitJobs::new(sqlite.clone() as Arc<dyn BulkSubmitJobStore>, composite)
                .with_ingest_sync(false);

        let submission = SubmissionId::generate(tenant_name);
        jobs.create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        jobs.add_manifest(&tenant, &submission, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = jobs
            .claim_next_manifest(
                &WorkerId::new(format!("w-{tenant_name}")),
                std::time::Duration::from_secs(60),
            )
            .await
            .unwrap()
            .expect("claimable manifest");
        let entries = (0..resources)
            .map(|n| {
                NdjsonEntry::new(
                    n as u64 + 1,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("{tenant_name}-{n}"),
                        "name": [{"family": "Deferred"}]
                    }),
                )
            })
            .collect();
        jobs.process_entries(
            &tenant,
            &submission,
            &lease.manifest_id,
            entries,
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();
        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert_eq!(
            report.synced, 0,
            "precondition: deferred ingest syncs nothing per resource"
        );
        jobs.finish_manifest(&lease).await.unwrap();
        assert_eq!(
            sqlite.count(&tenant, Some("Patient")).await.unwrap(),
            resources as u64
        );
        await_es_count(&es, &tenant, "Patient", 0).await;

        // The post-manifest rebuild, as `ReindexOnFinish` runs it.
        let op = ReindexOperation::with_parts(
            sqlite.clone(),
            vec![sqlite.clone(), es.clone()],
            sqlite.tenant_registries().clone(),
        );
        let job_id = op
            .start(
                tenant.clone(),
                ReindexRequest::for_types(vec!["Patient"]).with_batch_size(10),
                None,
            )
            .await
            .unwrap();
        let mut finished = None;
        for _ in 0..600 {
            let progress = op.get_progress(&job_id).await.unwrap();
            if progress.status.is_finished() {
                finished = Some(progress);
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        let progress = finished.expect("the deferred rebuild timed out");
        assert_eq!(
            progress.status,
            ReindexStatus::Completed,
            "{:?}",
            progress.error_message
        );
        await_es_count(&es, &tenant, "Patient", resources as u64).await;

        DeferredSubmit {
            _dir: dir,
            sqlite,
            es,
            jobs,
            tenant,
            submission,
        }
    }

    /// #1161: on `sqlite-es` with deferred indexing, a rolled-back submission
    /// used to leave its documents in Elasticsearch — gone from the primary,
    /// still matched by search. With the composite job store kept in the chain
    /// (ingest sync off), every rolled-back create is mirrored as a delete, so
    /// afterwards neither store holds any of them.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn es_integration_sqlite_es_deferred_rollback_leaves_no_orphans() {
        use helios_persistence::core::BulkSubmitRollbackProvider;

        const RESOURCES: usize = 12;
        let s = deferred_submit_rebuilt("deferred-rollback", RESOURCES).await;

        let changes = s
            .jobs
            .list_changes(&s.tenant, &s.submission, 100, 0)
            .await
            .unwrap();
        assert_eq!(changes.len(), RESOURCES, "one recorded create per entry");
        for change in &changes {
            assert!(
                s.jobs
                    .rollback_change(&s.tenant, &s.submission, change)
                    .await
                    .unwrap(),
                "{change:?} was not rolled back"
            );
        }

        assert_eq!(
            s.sqlite.count(&s.tenant, Some("Patient")).await.unwrap(),
            0,
            "the primary reverted every create"
        );
        await_es_count(&s.es, &s.tenant, "Patient", 0).await;
        assert!(
            s.es.read(&s.tenant, "Patient", "deferred-rollback-0")
                .await
                .unwrap()
                .is_none(),
            "a rolled-back resource must not survive in Elasticsearch"
        );
    }

    /// #1161: abort is not a rollback — no primary deletes what the aborted
    /// submission already ingested (#968) — so Elasticsearch must not either,
    /// or the resources become readable by id yet invisible to every search
    /// (#882, #1021). After an abort on the deferred path both stores agree.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn es_integration_sqlite_es_deferred_abort_keeps_search_consistent() {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionStatus};

        const RESOURCES: usize = 8;
        let s = deferred_submit_rebuilt("deferred-abort", RESOURCES).await;
        s.jobs
            .add_manifest(
                &s.tenant,
                &s.submission,
                Some("http://provider/m2.json"),
                None,
            )
            .await
            .unwrap();

        let cancelled = s
            .jobs
            .abort_submission(&s.tenant, &s.submission, "submissionStatus=stopped")
            .await
            .unwrap();
        assert_eq!(cancelled, 1, "the still-pending manifest is cancelled");
        let summary = s
            .jobs
            .get_submission(&s.tenant, &s.submission)
            .await
            .unwrap()
            .expect("submission");
        assert_eq!(summary.status, SubmissionStatus::Aborted);

        let primary = s.sqlite.count(&s.tenant, Some("Patient")).await.unwrap();
        assert_eq!(
            primary, RESOURCES as u64,
            "abort keeps what was ingested (#968)"
        );
        await_es_count(&s.es, &s.tenant, "Patient", primary).await;
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
    async fn es_integration_delete_nonexistent_fails() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let result = backend.delete(&tenant, "Patient", "nonexistent").await;
        assert!(result.is_err());
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

    /// Regression for issue #384, against a real cluster.
    ///
    /// The two isolation tests above use `tenant-a`/`tenant-b`, which differ in a
    /// way the old lossy derivation *preserved* — so they passed while tenants
    /// differing only by case shared an index and a document `_id`, and could
    /// read, overwrite, and delete each other's documents.
    ///
    /// **The lesson to keep: an isolation test must use the tenant pair its
    /// identifier derivation is most likely to conflate.**
    ///
    /// This exercises every `_id`-addressed path the issue names — the
    /// `create_or_update` existence probe, `update`, and `delete` — plus the
    /// glob-scoped paths (`count`, `clear_search_index`), because those derive
    /// their index pattern separately. A fix that escapes `index_name` but leaves
    /// a glob lowercased passes every unit test and fails here.
    #[tokio::test]
    async fn es_integration_case_variant_tenants_are_not_the_same_tenant() {
        let backend = create_backend().await;
        let upper = create_tenant("Acme");
        let lower = create_tenant("acme");

        // 1. `Acme` writes.
        backend
            .create_or_update(
                &upper,
                "Patient",
                "shared-id",
                json!({"resourceType": "Patient", "name": [{"family": "UPPER"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // 2. `acme` must not observe it — this is the existence probe, which
        //    reads a document's version to decide "is this new?".
        assert!(
            !backend
                .exists(&lower, "Patient", "shared-id")
                .await
                .unwrap(),
            "tenant `acme` must not see tenant `Acme`'s document"
        );

        // 3. `acme` writes to the same logical id.
        backend
            .create_or_update(
                &lower,
                "Patient",
                "shared-id",
                json!({"resourceType": "Patient", "name": [{"family": "lower"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // 4. `Acme`'s document must survive intact. On `main` this fails in an
        //    instructive way: the shared document's `_source.tenant_id` becomes
        //    "acme", so `read`'s tenant re-check returns None and the resource
        //    *vanishes* for its owner. Asserting the content catches both the
        //    vanish and the silent-overwrite variants.
        let read_upper = backend
            .read(&upper, "Patient", "shared-id")
            .await
            .unwrap()
            .expect("tenant `Acme`'s document must still exist");
        assert_eq!(read_upper.content()["name"][0]["family"], "UPPER");

        let read_lower = backend
            .read(&lower, "Patient", "shared-id")
            .await
            .unwrap()
            .expect("tenant `acme`'s document must exist");
        assert_eq!(read_lower.content()["name"][0]["family"], "lower");

        // 5. `acme`'s update must not touch `Acme`'s document. Without this, a
        //    fix that hardens `create_or_update` but not `update` would pass.
        backend
            .update(
                &lower,
                &read_lower,
                json!({"resourceType": "Patient", "name": [{"family": "lower-2"}]}),
            )
            .await
            .unwrap();
        assert_eq!(
            backend
                .read(&upper, "Patient", "shared-id")
                .await
                .unwrap()
                .expect("still present after the other tenant's update")
                .content()["name"][0]["family"],
            "UPPER"
        );

        // 6. `acme`'s delete must not remove `Acme`'s document.
        backend
            .delete(&lower, "Patient", "shared-id")
            .await
            .unwrap();
        assert_eq!(
            backend
                .read(&upper, "Patient", "shared-id")
                .await
                .unwrap()
                .expect("tenant `Acme`'s document must survive `acme`'s delete")
                .content()["name"][0]["family"],
            "UPPER"
        );

        // 7. Both directions of the glob-scoped count. Asserting only one would
        //    catch only one of the two ways the name/glob pair can drift apart.
        backend.refresh_index("Acme", "Patient").await.ok();
        backend.refresh_index("acme", "Patient").await.ok();
        assert_eq!(
            backend.count(&upper, Some("Patient")).await.unwrap(),
            1,
            "tenant `Acme` must still count its own document"
        );
        assert_eq!(
            backend.count(&lower, Some("Patient")).await.unwrap(),
            0,
            "tenant `acme` deleted its only document"
        );
    }

    /// The unit tests model Elasticsearch's index-naming rules; this is the
    /// oracle. An encoding can be provably injective and still be rejected by a
    /// real cluster, which no pure test can catch.
    #[tokio::test]
    async fn es_integration_non_conforming_tenant_ids_are_accepted_by_elasticsearch() {
        let backend = create_backend().await;

        // Uppercase (issue #384's collision) and hierarchical (which previously
        // produced an illegal index name and 500'd on every write).
        for tenant_id in ["ACME", "acme/research", "Acme.Corp"] {
            let tenant = create_tenant(tenant_id);
            backend
                .create_or_update(
                    &tenant,
                    "Patient",
                    "p1",
                    json!({"resourceType": "Patient", "name": [{"family": tenant_id}]}),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("write for tenant {tenant_id:?} must succeed: {e}"));

            let read = backend
                .read(&tenant, "Patient", "p1")
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("read back for tenant {tenant_id:?}"));
            assert_eq!(read.content()["name"][0]["family"], tenant_id);
        }
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
    async fn es_integration_search_composite_code_value_quantity() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-bp",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
                    "valueQuantity": { "value": 107, "unit": "mmHg", "system": "http://unitsofmeasure.org" }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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

        // Both components match within the same instance.
        let hit = backend
            .search(&tenant, &query("8480-6$ge100"))
            .await
            .unwrap();
        assert_eq!(hit.resources.items.len(), 1, "code + value match → 1 hit");
        assert_eq!(hit.resources.items[0].id(), "obs-bp");

        // Quantity component fails.
        let miss = backend
            .search(&tenant, &query("8480-6$ge200"))
            .await
            .unwrap();
        assert!(miss.resources.items.is_empty(), "value too low → no hit");

        // Token component fails.
        let miss = backend
            .search(&tenant, &query("9999-9$ge100"))
            .await
            .unwrap();
        assert!(miss.resources.items.is_empty(), "code mismatch → no hit");
    }

    #[tokio::test]
    async fn es_integration_string_search_is_accent_insensitive() {
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
                    "id": "accent-es",
                    "name": [{ "family": "Müller" }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
    async fn es_integration_quantity_search_ucum_equivalence() {
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
                    "id": "obs-mass-es",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "x" }] },
                    "valueQuantity": { "value": 1, "unit": "g", "system": "http://unitsofmeasure.org", "code": "g" }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // 1 g stored; search the equivalent 1000 mg.
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
        assert_eq!(result.resources.items[0].id(), "obs-mass-es");
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
    // Tenant Purge Tests
    // ========================================================================

    #[tokio::test]
    async fn es_integration_purge_tenant_data() {
        let backend = create_backend().await;

        // ES is a search secondary, never the tenant registry of record.
        assert!(!backend.supports_tenant_registry());

        // Unique tenant ids; the suffixed tenant's id starts with tenant A's id
        // plus `_x`, so its index names (e.g. `{prefix}_{tenant_a}_x_patient`)
        // match the purge's `{prefix}_{tenant_a}_*` wildcard. Only the exact
        // `term` filter on tenant_id keeps its data alive.
        let tenant_a_id = format!("acme-{}", uuid::Uuid::new_v4().simple());
        let tenant_suffixed_id = format!("{}_x", tenant_a_id);
        let tenant_b_id = format!("beta-{}", uuid::Uuid::new_v4().simple());

        let tenant_a = create_tenant(&tenant_a_id);
        let tenant_b = create_tenant(&tenant_b_id);
        let tenant_suffixed = create_tenant(&tenant_suffixed_id);

        let mut tenant_a_ids = Vec::new();
        for _ in 0..2 {
            let patient = json!({"resourceType": "Patient"});
            let created = backend
                .create(&tenant_a, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
            tenant_a_ids.push(created.id().to_string());
        }

        let patient_b = backend
            .create(
                &tenant_b,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let patient_suffixed = backend
            .create(
                &tenant_suffixed,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 2);

        // Contained/auxiliary docs may add to the deleted count, so >= 2.
        let purged = backend.purge_tenant_data(&tenant_a_id).await.unwrap();
        assert!(purged >= 2, "expected at least 2 purged docs, got {purged}");

        // Tenant A is empty: counts are zero and reads come back None.
        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 0);
        assert_eq!(backend.count(&tenant_a, None).await.unwrap(), 0);
        for id in &tenant_a_ids {
            assert!(
                backend
                    .read(&tenant_a, "Patient", id)
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        // Tenant B is untouched.
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 1);
        assert!(
            backend
                .read(&tenant_b, "Patient", patient_b.id())
                .await
                .unwrap()
                .is_some()
        );

        // The suffixed tenant survives even though its index names match the
        // purge's index wildcard — the exact term filter decides membership.
        assert_eq!(
            backend
                .count(&tenant_suffixed, Some("Patient"))
                .await
                .unwrap(),
            1
        );
        assert!(
            backend
                .read(&tenant_suffixed, "Patient", patient_suffixed.id())
                .await
                .unwrap()
                .is_some()
        );
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
    async fn es_integration_search_missing_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("missing-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "with-birthdate",
                    "birthDate": "1980-01-15"
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
                    "id": "without-birthdate"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .refresh_index("missing-tenant", "Patient")
            .await
            .unwrap();

        let query = |value: &str| {
            SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![],
            })
        };

        let missing = backend.search(&tenant, &query("true")).await.unwrap();
        let missing_ids: Vec<&str> = missing.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(missing_ids, vec!["without-birthdate"]);

        let present = backend.search(&tenant, &query("false")).await.unwrap();
        let present_ids: Vec<&str> = present.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(present_ids, vec!["with-birthdate"]);

        for (name, param_type) in [
            ("_id", SearchParamType::Token),
            ("_lastUpdated", SearchParamType::Date),
        ] {
            let metadata_query = |is_missing| {
                SearchQuery::new("Patient").with_parameter(SearchParameter {
                    name: name.to_string(),
                    param_type,
                    modifier: Some(SearchModifier::Missing),
                    values: vec![SearchValue::boolean(is_missing)],
                    chain: vec![],
                    components: vec![],
                })
            };

            let missing = backend
                .search(&tenant, &metadata_query(true))
                .await
                .unwrap();
            assert!(missing.resources.items.is_empty(), "{name}:missing=true");

            let present = backend
                .search(&tenant, &metadata_query(false))
                .await
                .unwrap();
            let mut ids: Vec<&str> = present.resources.items.iter().map(|r| r.id()).collect();
            ids.sort();
            assert_eq!(ids, vec!["with-birthdate", "without-birthdate"]);
        }
    }

    /// #990: a resource type that has never been written has no index (indices
    /// are created lazily on first write), so Elasticsearch answers the search
    /// with `index_not_found_exception`. That is a known-empty set, and the
    /// result must say so with `total = Some(0)` — a missing total made the
    /// REST layer emit `"total": null` for `GET /Group` and fail closed on
    /// `GET /Group?_summary=count`.
    #[tokio::test]
    async fn es_integration_search_unindexed_type_reports_zero_total() {
        use helios_persistence::core::{SearchProvider, TextSearchProvider};
        use helios_persistence::types::{ContainedMode, Pagination, SearchQuery, TotalMode};

        let backend = create_backend().await;
        let tenant = create_tenant("unindexed-type-tenant");

        // Plain search: the ES search path always reports a total.
        let result = backend
            .search(&tenant, &SearchQuery::new("Group"))
            .await
            .expect("searching an unindexed type is not an error");
        assert!(result.resources.items.is_empty());
        assert_eq!(
            result.total,
            Some(0),
            "an unindexed type is a known-empty set"
        );
        assert_eq!(result.resources.page_info.total, Some(0));

        // `_total=accurate` (what `_summary=count` implies, #254).
        let mut query = SearchQuery::new("Group");
        query.total = Some(TotalMode::Accurate);
        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.total, Some(0));

        // The count path already agreed; the two must not diverge.
        assert_eq!(backend.search_count(&tenant, &query).await.unwrap(), 0);

        // `_contained` and full-text searches take their own request paths and
        // hit the same missing index.
        let mut contained = SearchQuery::new("Group");
        contained.contained = ContainedMode::On;
        contained.total = Some(TotalMode::Accurate);
        let result = backend.search(&tenant, &contained).await.unwrap();
        assert_eq!(result.total, Some(0));

        let result = backend
            .search_text(&tenant, "Group", "anything", &Pagination::default())
            .await
            .unwrap();
        assert!(result.resources.items.is_empty());
        assert_eq!(result.total, Some(0));
    }

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
    async fn es_integration_write_refresh_wait_for_read_after_write() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend_with("5s", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("raw-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "raw-1",
                    "name": [{"family": "Readafterwrite", "given": ["Direct"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Readafterwrite")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "wait_for write must be searchable immediately"
        );
        assert_eq!(result.resources.items[0].id(), "raw-1");

        backend.delete(&tenant, "Patient", "raw-1").await.unwrap();

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            result.resources.items.is_empty(),
            "wait_for delete must drop out of search immediately"
        );
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn es_integration_composite_synchronous_wait_for_read_after_write() {
        use std::collections::HashMap;

        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
        };
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let es = shared_es().await;
        let unique_prefix = format!("hfs_{}", uuid::Uuid::new_v4().simple());
        let es_config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: unique_prefix,
            number_of_replicas: 0,
            refresh_interval: "5s".to_string(),
            write_refresh: WriteRefreshPolicy::WaitFor,
            ..Default::default()
        };
        let es_backend = Arc::new(
            ElasticsearchBackend::with_shared_registry(es_config, build_search_registry())
                .expect("create ES backend"),
        );
        es_backend
            .initialize()
            .await
            .expect("initialize ES backend");

        let sqlite = Arc::new(SqliteBackend::in_memory().expect("create SQLite backend"));
        sqlite.init_schema().expect("init SQLite schema");

        let composite_config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("build composite config");

        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), es_backend.clone() as DynStorage);

        let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
        search_providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
        search_providers.insert("es".to_string(), es_backend.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers)
            .with_full_primary(sqlite);

        let tenant = create_tenant("raw-composite-tenant");
        composite
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "raw-composite-1",
                    "text": {
                        "status": "generated",
                        "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\">Compositeraw Sync</div>"
                    },
                    "name": [{"family": "Compositeraw", "given": ["Sync"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .expect("create through composite");

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Compositeraw")],
            chain: vec![],
            components: vec![],
        });

        let result = composite
            .search(&tenant, &query)
            .await
            .expect("search through composite");
        assert_eq!(
            result.resources.items.len(),
            1,
            "synchronous sync + wait_for must give read-after-write search"
        );
        assert_eq!(result.resources.items[0].id(), "raw-composite-1");
    }

    /// #1047: a lookup that resolves a write against existing content (a
    /// transaction's conditional reference, an `If-None-Exist` create) must
    /// see every write the composite has already acknowledged, whatever the
    /// Elasticsearch write-refresh policy. This is the default policy
    /// (`false`) with a refresh interval long enough that the index cannot
    /// catch up on its own during the test: the lookup has to ask.
    #[cfg(feature = "sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn es_integration_composite_conditional_lookups_see_unrefreshed_writes() {
        use std::collections::HashMap;

        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
        };
        use helios_persistence::core::{
            ConditionalCreateResult, ConditionalStorage, SearchProvider,
        };
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let es = shared_es().await;
        let unique_prefix = format!("hfs_{}", uuid::Uuid::new_v4().simple());
        let es_config = ElasticsearchConfig {
            nodes: vec![format!("http://{}:{}", es.host, es.port)],
            index_prefix: unique_prefix,
            number_of_replicas: 0,
            refresh_interval: "30s".to_string(),
            write_refresh: WriteRefreshPolicy::False,
            ..Default::default()
        };
        let es_backend = Arc::new(
            ElasticsearchBackend::with_shared_registry(es_config, build_search_registry())
                .expect("create ES backend"),
        );
        es_backend
            .initialize()
            .await
            .expect("initialize ES backend");

        // The production shape: the primary's own index is offloaded to ES.
        let mut sqlite = SqliteBackend::in_memory().expect("create SQLite backend");
        sqlite.set_search_offloaded(true);
        let sqlite = Arc::new(sqlite);
        sqlite.init_schema().expect("init SQLite schema");

        let composite_config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("build composite config");

        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), es_backend.clone() as DynStorage);

        let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
        search_providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
        search_providers.insert("es".to_string(), es_backend.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers)
            .with_full_primary(sqlite);

        let tenant = create_tenant("unrefreshed-composite-tenant");
        let organization = json!({
            "resourceType": "Organization",
            "identifier": [{"system": "urn:zzz:probe", "value": "ORG-PROBE-1047"}],
            "name": "ZZZ Probe Org"
        });
        let created = composite
            .create(
                &tenant,
                "Organization",
                organization.clone(),
                FhirVersion::default(),
            )
            .await
            .expect("create through composite");

        // `If-None-Exist` right after the create: the same criteria a
        // transaction's conditional reference carries.
        let outcome = composite
            .conditional_create(
                &tenant,
                "Organization",
                organization,
                "identifier=urn:zzz:probe|ORG-PROBE-1047",
                FhirVersion::default(),
            )
            .await
            .expect("conditional create through composite");
        match outcome {
            ConditionalCreateResult::Exists(existing) => {
                assert_eq!(existing.id(), created.id());
            }
            ConditionalCreateResult::Created(_) => {
                panic!("conditional create missed the Organization created moments earlier")
            }
            ConditionalCreateResult::MultipleMatches(n) => panic!("unexpected {n} matches"),
        }

        // The primitive the transaction path uses, then the lookup it runs.
        composite
            .ensure_writes_visible(&tenant, &["Organization"])
            .await
            .expect("ensure_writes_visible through composite");
        let query = SearchQuery::new("Organization").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::token(Some("urn:zzz:probe"), "ORG-PROBE-1047")],
            chain: vec![],
            components: vec![],
        });
        let result = composite
            .search(&tenant, &query)
            .await
            .expect("search through composite");
        assert_eq!(
            result.resources.items.len(),
            1,
            "an acknowledged write must be searchable after ensure_writes_visible, \
             without waiting for the refresh interval"
        );
        assert_eq!(result.resources.items[0].id(), created.id());

        // A type with no index yet is not an error: nothing was written to it.
        composite
            .ensure_writes_visible(&tenant, &["Location"])
            .await
            .expect("refreshing a type with no index yet is a no-op");
    }

    /// `create_many` is one `_bulk` request per batch, so under
    /// `refresh=wait_for` a batch pays one refresh wait — not one per
    /// document. With a 5s refresh interval, 40 per-document writes would
    /// take over three minutes; the batch must finish in a few seconds. This
    /// is the shape of the startup conformance seed that stalled the
    /// sqlite-elasticsearch server past its readiness timeout.
    #[tokio::test]
    async fn es_integration_create_many_pays_one_refresh_wait_per_batch() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::SearchQuery;

        let backend = create_backend_with("5s", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("bulk-wait-for");

        let patients: Vec<_> = (0..40)
            .map(|i| {
                json!({
                    "resourceType": "Patient",
                    "id": format!("bulk-{i}"),
                    "name": [{"family": "Bulk", "given": [format!("P{i}")]}]
                })
            })
            .collect();

        let started = std::time::Instant::now();
        let results = backend
            .create_many(&tenant, "Patient", patients, FhirVersion::default())
            .await;
        let elapsed = started.elapsed();

        assert_eq!(results.len(), 40, "one result per input");
        for (i, result) in results.iter().enumerate() {
            let stored = result
                .as_ref()
                .unwrap_or_else(|e| panic!("resource {i} failed: {e}"));
            assert_eq!(stored.id(), format!("bulk-{i}"), "results keep input order");
            assert_eq!(stored.version_id(), "1");
        }
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "40 resources under wait_for with a 5s refresh interval took {elapsed:?}; \
             a batch must not pay one refresh wait per document"
        );

        // wait_for on the bulk request means every document is already
        // searchable — no refresh needed.
        let visible = backend
            .search_count(&tenant, &SearchQuery::new("Patient"))
            .await
            .expect("count after batch create");
        assert_eq!(
            visible, 40,
            "the whole batch is visible once the request returns"
        );
    }

    /// The bulk path carries every document a resource contributes: a
    /// server-assigned id when the resource has none, and the `_contained`
    /// documents, which land in *their* type's index.
    #[tokio::test]
    async fn es_integration_create_many_assigns_ids_and_indexes_contained() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, SearchQuery};

        let backend = create_backend().await;
        let tenant = create_tenant("bulk-contained");

        let results = backend
            .create_many(
                &tenant,
                "Observation",
                vec![
                    json!({
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "no id, server assigns one"}
                    }),
                    json!({
                        "resourceType": "Observation",
                        "id": "obs-with-contained",
                        "status": "final",
                        "code": {"text": "hr"},
                        "contained": [{
                            "resourceType": "Patient",
                            "id": "inner",
                            "name": [{"family": "Containedbulk"}]
                        }],
                        "subject": {"reference": "#inner"}
                    }),
                ],
                FhirVersion::default(),
            )
            .await;
        assert_eq!(results.len(), 2);
        let assigned = results[0].as_ref().expect("first create").id().to_string();
        assert!(!assigned.is_empty(), "server-assigned id");
        assert_eq!(
            results[0].as_ref().unwrap().content()["id"],
            json!(assigned),
            "the returned content carries the assigned id"
        );
        assert_eq!(
            results[1].as_ref().expect("second create").id(),
            "obs-with-contained"
        );

        backend
            .refresh_index("bulk-contained", "Observation")
            .await
            .ok();
        backend
            .refresh_index("bulk-contained", "Patient")
            .await
            .ok();

        let both = backend
            .search(&tenant, &SearchQuery::new("Observation"))
            .await
            .expect("search observations");
        assert_eq!(both.resources.items.len(), 2);

        // The contained Patient is findable through `_contained=true`, which
        // only works if its document reached the Patient index.
        let mut contained = SearchQuery::new("Patient");
        contained.contained = ContainedMode::On;
        let found = backend
            .search(&tenant, &contained)
            .await
            .expect("contained search");
        assert_eq!(
            found.resources.items.len(),
            1,
            "the contained Patient document was written to the Patient index"
        );
    }

    #[tokio::test]
    async fn es_integration_compartment_search() {
        // Compartment membership: a resource joins the Patient compartment if it
        // references the patient via ANY of the membership params (here `subject`
        // OR `performer`). Resources for another patient must be excluded.
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{CompartmentMembership, SearchQuery};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // In the compartment via `subject`.
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-subject",
                    "status": "final",
                    "code": {"text": "hr"},
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // In the compartment via the NON-first param `performer` only (subject
        // points elsewhere). This verifies the OR across membership params.
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-performer",
                    "status": "final",
                    "code": {"text": "hr"},
                    "subject": {"reference": "Patient/p2"},
                    "performer": [{"reference": "Patient/p1"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Not in the compartment (references another patient).
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-other",
                    "status": "final",
                    "code": {"text": "hr"},
                    "subject": {"reference": "Patient/p2"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let mut query = SearchQuery::new("Observation");
        query.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1".to_string(),
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();

        assert!(
            ids.contains(&"obs-subject".to_string()),
            "compartment must include the resource linked via `subject`"
        );
        assert!(
            ids.contains(&"obs-performer".to_string()),
            "compartment must include the resource linked via `performer`"
        );
        assert!(
            !ids.contains(&"obs-other".to_string()),
            "compartment must exclude resources of another patient"
        );
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

    /// `:not` means "no value of the parameter matches" (#473). Three cases
    /// that a per-value or per-row negation gets wrong:
    /// resources whose element is absent must be returned; a multi-valued
    /// element must not leak back in through its other values; and `:not=a,b`
    /// is `NOT (a OR b)`, not `NOT a OR NOT b`.
    #[tokio::test]
    async fn es_integration_search_token_not_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let communication = |codes: &[&str]| {
            codes
                .iter()
                .map(|code| {
                    json!({ "language": { "coding": [{
                        "system": "urn:ietf:bcp:47",
                        "code": code
                    }]}})
                })
                .collect::<Vec<_>>()
        };

        for (id, langs) in [
            ("lang-en", vec!["en-US"]),
            // Multi-valued: holds the excluded code AND another one.
            ("lang-en-es", vec!["en-US", "es"]),
            ("lang-fr", vec!["fr"]),
        ] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "communication": communication(&langs)
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // No communication element at all — has no value, so it matches :not.
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "lang-none" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = |values: Vec<SearchValue>| {
            SearchQuery::new("Patient")
                .with_parameter(SearchParameter {
                    name: "language".to_string(),
                    param_type: SearchParamType::Token,
                    modifier: Some(SearchModifier::Not),
                    values,
                    chain: vec![],
                    components: vec![],
                })
                .with_count(100)
        };

        let ids = |result: &helios_persistence::core::SearchResult| {
            let mut ids: Vec<String> = result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect();
            ids.sort();
            ids
        };

        let result = backend
            .search(&tenant, &query(vec![SearchValue::token(None, "en-US")]))
            .await
            .unwrap();
        assert_eq!(
            ids(&result),
            vec!["lang-fr", "lang-none"],
            "language:not=en-US excludes both en-US patients (incl. the one that also has 'es') \
             and returns the patient with no communication element"
        );

        let result = backend
            .search(
                &tenant,
                &query(vec![
                    SearchValue::token(None, "en-US"),
                    SearchValue::token(None, "fr"),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(&result),
            vec!["lang-none"],
            "language:not=en-US,fr is NOT (en-US OR fr) — only the patient with no language remains"
        );
    }

    /// #1092: `_id` is dispatched by name into a dedicated builder
    /// (`build_id_clause`) that bypassed the generic `:not` handling
    /// entirely, so `_id:not=<id>` returned *only* the resource the caller
    /// asked to exclude — the precise inverse of the request.
    #[tokio::test]
    async fn es_integration_search_id_not_excludes_listed_ids() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue, TotalMode,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for id in ["a", "b", "c"] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": id }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let mut query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "_id".to_string(),
                param_type: SearchParamType::Token,
                modifier: Some(SearchModifier::Not),
                values: vec![SearchValue::eq("b")],
                chain: vec![],
                components: vec![],
            })
            .with_count(100);
        query.total = Some(TotalMode::Accurate);

        let result = backend.search(&tenant, &query).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "c"], "_id:not=b must exclude only b");
        assert_eq!(result.total, Some(2));

        let mut query_two = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "_id".to_string(),
                param_type: SearchParamType::Token,
                modifier: Some(SearchModifier::Not),
                values: vec![SearchValue::eq("a"), SearchValue::eq("b")],
                chain: vec![],
                components: vec![],
            })
            .with_count(100);
        query_two.total = Some(TotalMode::Accurate);

        let result_two = backend.search(&tenant, &query_two).await.unwrap();
        let ids_two: Vec<String> = result_two
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(ids_two, vec!["c"], "_id:not=a,b must exclude both a and b");
        assert_eq!(result_two.total, Some(1));
    }

    /// #1092: modifiers the `_id` builder cannot honour must be rejected
    /// rather than silently degrading to a positive match (mirrors
    /// #1055/#1091's MongoDB `metadata_param_honoured` gate).
    #[tokio::test]
    async fn es_integration_search_id_unsupported_modifier_is_rejected() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::error::{SearchError, StorageError};
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "a" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });

        let err = backend.search(&tenant, &query).await.unwrap_err();
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                    if modifier == "text"
            ),
            "_id:text must be rejected as an unsupported modifier, got: {err:?}"
        );
    }

    /// `_lastUpdated` is dispatched by name into a dedicated builder that
    /// reads only `param.values`, so a modifier other than `:missing` would
    /// be dropped and the value consumed as a plain positive date match. It
    /// must be rejected up front instead, the same way unsupported `_id`
    /// modifiers are (#1092 follow-up).
    #[tokio::test]
    async fn es_integration_search_last_updated_unsupported_modifier_is_rejected() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::error::{SearchError, StorageError};
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "a" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("2020")],
            chain: vec![],
            components: vec![],
        });

        let err = backend.search(&tenant, &query).await.unwrap_err();
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                    if modifier == "not"
            ),
            "_lastUpdated:not must be rejected as an unsupported modifier, got: {err:?}"
        );

        let err = backend.search_count(&tenant, &query).await.unwrap_err();
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                    if modifier == "not"
            ),
            "search_count must reject _lastUpdated:not as well, got: {err:?}"
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
    async fn elasticsearch_integration_quantity_comparators_ignore_search_precision() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Issue #1011: `value-quantity=gt60` must exclude 60.2 kg exactly at
        // the boundary while still matching every value strictly above 60,
        // regardless of how the search value's own precision is written.
        let weights = [
            ("obs-weight-55-4", 55.4),
            ("obs-weight-58-5", 58.5),
            ("obs-weight-60-2", 60.2),
            ("obs-weight-64-5", 64.5),
        ];
        for (id, value) in weights {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
                        "valueQuantity": {
                            "value": value,
                            "unit": "kg",
                            "system": "http://unitsofmeasure.org",
                            "code": "kg"
                        }
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        async fn search_ids(
            backend: &ElasticsearchBackend,
            tenant: &TenantContext,
            prefix: SearchPrefix,
            value: &str,
        ) -> Vec<String> {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "value-quantity".to_string(),
                param_type: SearchParamType::Quantity,
                modifier: None,
                values: vec![SearchValue::new(prefix, value)],
                chain: vec![],
                components: vec![],
            });
            let result = backend.search(tenant, &query).await.unwrap();
            let mut ids: Vec<String> = result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect();
            ids.sort();
            ids
        }

        let gt60 = search_ids(&backend, &tenant, SearchPrefix::Gt, "60").await;
        assert_eq!(gt60, vec!["obs-weight-60-2", "obs-weight-64-5"], "gt60");

        let gt60_0 = search_ids(&backend, &tenant, SearchPrefix::Gt, "60.0").await;
        assert_eq!(
            gt60_0,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "gt60.0 must match gt60 exactly: implicit precision is ignored"
        );

        let le60_2 = search_ids(&backend, &tenant, SearchPrefix::Le, "60.2").await;
        assert_eq!(
            le60_2,
            vec!["obs-weight-55-4", "obs-weight-58-5", "obs-weight-60-2"],
            "le60.2"
        );

        let lt58_5 = search_ids(&backend, &tenant, SearchPrefix::Lt, "58.5").await;
        assert_eq!(lt58_5, vec!["obs-weight-55-4"], "lt58.5");

        let eq60 = search_ids(&backend, &tenant, SearchPrefix::Eq, "60").await;
        assert_eq!(
            eq60,
            vec!["obs-weight-60-2"],
            "eq60 ranges over [59.5, 60.5), which contains 60.2"
        );

        let eq60_0 = search_ids(&backend, &tenant, SearchPrefix::Eq, "60.0").await;
        assert!(
            eq60_0.is_empty(),
            "eq60.0 ranges over [59.95, 60.05), which excludes 60.2: got {eq60_0:?}"
        );

        let gt60_kg = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Gt,
            "60|http://unitsofmeasure.org|kg",
        )
        .await;
        assert_eq!(
            gt60_kg,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "gt60|...|kg (raw branch)"
        );

        // Cross-unit boundary: 60.2 kg canonicalizes to exactly 60200 g, so
        // `ge60200|...|g` must match it (and everything above) through the
        // canonical branch.
        let ge60200_g = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Ge,
            "60200|http://unitsofmeasure.org|g",
        )
        .await;
        assert_eq!(
            ge60200_g,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "ge60200|...|g (canonical branch, cross-unit exact boundary)"
        );
    }

    #[tokio::test]
    async fn elasticsearch_integration_quantity_ne_excludes_precision_range() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Issue #1011: `value-quantity=ne60` must exclude 60.2 kg (inside the
        // implicit-precision range [59.5, 60.5)) while still matching every
        // other value, regardless of how the search value's own precision is
        // written.
        let weights = [
            ("obs-weight-55-4", 55.4),
            ("obs-weight-58-5", 58.5),
            ("obs-weight-60-2", 60.2),
            ("obs-weight-64-5", 64.5),
        ];
        for (id, value) in weights {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
                        "valueQuantity": {
                            "value": value,
                            "unit": "kg",
                            "system": "http://unitsofmeasure.org",
                            "code": "kg"
                        }
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        async fn search_ids(
            backend: &ElasticsearchBackend,
            tenant: &TenantContext,
            prefix: SearchPrefix,
            value: &str,
        ) -> Vec<String> {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "value-quantity".to_string(),
                param_type: SearchParamType::Quantity,
                modifier: None,
                values: vec![SearchValue::new(prefix, value)],
                chain: vec![],
                components: vec![],
            });
            let result = backend.search(tenant, &query).await.unwrap();
            let mut ids: Vec<String> = result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect();
            ids.sort();
            ids
        }

        let ne60 = search_ids(&backend, &tenant, SearchPrefix::Ne, "60").await;
        assert_eq!(
            ne60,
            vec!["obs-weight-55-4", "obs-weight-58-5", "obs-weight-64-5"],
            "ne60 excludes 60.2, which lies in [59.5, 60.5)"
        );

        let ne60_0 = search_ids(&backend, &tenant, SearchPrefix::Ne, "60.0").await;
        assert_eq!(
            ne60_0,
            vec![
                "obs-weight-55-4",
                "obs-weight-58-5",
                "obs-weight-60-2",
                "obs-weight-64-5"
            ],
            "ne60.0 ranges over [59.95, 60.05), which excludes 60.2, so nothing is excluded"
        );

        let ne60_kg = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Ne,
            "60|http://unitsofmeasure.org|kg",
        )
        .await;
        assert_eq!(
            ne60_kg,
            vec!["obs-weight-55-4", "obs-weight-58-5", "obs-weight-64-5"],
            "ne60|...|kg excludes 60.2 through the raw and canonical branches"
        );
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

    /// #892: `ne` must be the complement of the day range, not `eq`.
    #[tokio::test]
    async fn es_integration_search_last_updated_ne_excludes_today() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "lu-ne-1"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let today = created.last_modified().format("%Y-%m-%d").to_string();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let last_updated = |prefix: SearchPrefix| {
            SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(prefix, &today)],
                chain: vec![],
                components: vec![],
            })
        };

        let eq = backend
            .search(&tenant, &last_updated(SearchPrefix::Eq))
            .await
            .unwrap();
        assert!(
            eq.resources.items.iter().any(|r| r.id() == "lu-ne-1"),
            "eq on the creation day must match the resource"
        );

        let ne = backend
            .search(&tenant, &last_updated(SearchPrefix::Ne))
            .await
            .unwrap();
        assert!(
            ne.resources.items.iter().all(|r| r.id() != "lu-ne-1"),
            "ne on the creation day must exclude the resource"
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

        // Full-text search ranks by relevance, so the backend must populate
        // `SearchResult.scores` (-> Bundle.entry.search.score) for the matches.
        assert!(
            !result.scores.is_empty(),
            "full-text search should populate relevance scores"
        );
        for resource in &result.resources.items {
            let score = result.scores.get(&resource.url());
            assert!(
                matches!(score, Some(s) if *s > 0.0),
                "matched resource {} should have a positive relevance score, got {score:?}",
                resource.url()
            );
        }
    }

    #[tokio::test]
    async fn es_integration_contained_search() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            ContainedMode, ContainedReturn, SearchModifier, SearchParamType, SearchParameter,
            SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("contained-test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs1",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "#p1" },
                    "contained": [
                        {
                            "resourceType": "Patient",
                            "id": "p1",
                            "name": [{ "family": "Smith", "given": ["Contained"] }],
                            "gender": "male"
                        },
                        {
                            "resourceType": "Patient",
                            "id": "p2",
                            "name": [{ "family": "Jones", "given": ["Contained"] }],
                            "birthDate": "1980-01-15"
                        }
                    ]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "top1", "name": [{ "family": "Smith" }] }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

        let name_query = |mode: ContainedMode, ret: ContainedReturn| {
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
        };

        // Default (_contained=off): only the top-level Patient.
        let off = backend
            .search(
                &tenant,
                &name_query(ContainedMode::Off, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let off_urls: Vec<String> = off.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(
            off_urls,
            vec!["Patient/top1"],
            "off excludes contained docs"
        );

        // _contained=true: the container is returned.
        let on = backend
            .search(
                &tenant,
                &name_query(ContainedMode::On, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let on_urls: Vec<String> = on.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(on_urls, vec!["Observation/obs1"], "container returned");

        // _containedType=contained: the contained Patient itself.
        let contained = backend
            .search(
                &tenant,
                &name_query(ContainedMode::On, ContainedReturn::Contained),
            )
            .await
            .unwrap();
        assert_eq!(contained.resources.items.len(), 1);
        assert_eq!(contained.resources.items[0].resource_type(), "Patient");
        assert_eq!(contained.resources.items[0].id(), "p1");

        // _contained=both: top-level + container.
        let both = backend
            .search(
                &tenant,
                &name_query(ContainedMode::Both, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let mut both_urls: Vec<String> = both.resources.items.iter().map(|r| r.url()).collect();
        both_urls.sort();
        assert_eq!(both_urls, vec!["Observation/obs1", "Patient/top1"]);

        let missing_query = |is_missing| {
            let mut q = SearchQuery::new("Patient");
            q.contained = ContainedMode::On;
            q.contained_return = ContainedReturn::Contained;
            q.parameters.push(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::boolean(is_missing)],
                chain: vec![],
                components: vec![],
            });
            q
        };

        let missing = backend.search(&tenant, &missing_query(true)).await.unwrap();
        let missing_ids: Vec<&str> = missing.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(missing_ids, vec!["p1"]);

        let present = backend
            .search(&tenant, &missing_query(false))
            .await
            .unwrap();
        let present_ids: Vec<&str> = present.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(present_ids, vec!["p2"]);
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
    // Include Resolution Tests (#1013)
    // ========================================================================

    /// Sorted `(resource_type, id)` pairs, for exact-set assertions regardless
    /// of the order resources were fetched in.
    fn include_type_ids(
        resources: &[helios_persistence::types::StoredResource],
    ) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = resources
            .iter()
            .map(|r| (r.resource_type().to_string(), r.id().to_string()))
            .collect();
        pairs.sort();
        pairs
    }

    /// Seeds the fixture shared by the `_include` delegation tests: `pat-1`
    /// (managed by `org-1`), `org-1`, an Encounter with a real `serviceProvider`
    /// reference (`enc-org`) and one with a conditional reference (`enc-cond`),
    /// both referencing `pat-1` via `subject`.
    async fn seed_include_fixture(backend: &ElasticsearchBackend, tenant: &TenantContext) {
        backend
            .create(
                tenant,
                "Organization",
                json!({
                    "resourceType": "Organization",
                    "id": "org-1",
                    "name": "Include Org"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "pat-1",
                    "name": [{"family": "Include"}],
                    "managingOrganization": {"reference": "Organization/org-1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                tenant,
                "Encounter",
                json!({
                    "resourceType": "Encounter",
                    "id": "enc-cond",
                    "status": "finished",
                    "class": {"code": "AMB"},
                    "subject": {"reference": "Patient/pat-1"},
                    "serviceProvider": {
                        "reference": "Organization?identifier=http://example.org/org|dept-9"
                    }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                tenant,
                "Encounter",
                json!({
                    "resourceType": "Encounter",
                    "id": "enc-org",
                    "status": "finished",
                    "class": {"code": "AMB"},
                    "subject": {"reference": "Patient/pat-1"},
                    "serviceProvider": {"reference": "Organization/org-1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh so the fixture is visible to search()/read().
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    /// `search()` leaves `included` empty, like SQLite and Postgres, so the
    /// REST layer resolves `_include` for Elasticsearch through the shared
    /// `resolve_includes_iterative` path instead of an inline extractor.
    #[tokio::test]
    async fn es_integration_search_does_not_resolve_includes_inline() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{IncludeDirective, IncludeType, SearchQuery};

        let backend = create_backend().await;
        let tenant = create_tenant("include-1013-1");
        seed_include_fixture(&backend, &tenant).await;

        let query = SearchQuery::new("Encounter").with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Encounter".to_string(),
            search_param: "service-provider".to_string(),
            target_type: None,
            iterate: false,
        });

        let result = backend.search(&tenant, &query).await.unwrap();

        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["enc-cond".to_string(), "enc-org".to_string()]);
        assert!(result.included.is_empty());
    }

    /// `IncludeProvider::resolve_includes` delegates to the shared,
    /// registry-driven resolver: a conditional `serviceProvider` reference
    /// never resolves to an included resource, a real one resolves to exactly
    /// its target, and resolving the same target from two source resources
    /// dedupes it.
    #[tokio::test]
    async fn es_integration_include_service_provider_returns_only_targets() {
        use helios_persistence::core::IncludeProvider;
        use helios_persistence::types::{IncludeDirective, IncludeType};

        let backend = create_backend().await;
        let tenant = create_tenant("include-1013-2");
        seed_include_fixture(&backend, &tenant).await;

        let enc_cond = backend
            .read(&tenant, "Encounter", "enc-cond")
            .await
            .unwrap()
            .expect("enc-cond must exist");
        let enc_org = backend
            .read(&tenant, "Encounter", "enc-org")
            .await
            .unwrap()
            .expect("enc-org must exist");

        let service_provider = IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Encounter".to_string(),
            search_param: "service-provider".to_string(),
            target_type: None,
            iterate: false,
        };

        let both = backend
            .resolve_includes(
                &tenant,
                &[enc_cond.clone(), enc_org.clone()],
                std::slice::from_ref(&service_provider),
            )
            .await
            .unwrap();
        assert_eq!(
            include_type_ids(&both),
            vec![("Organization".to_string(), "org-1".to_string())]
        );

        let cond_only = backend
            .resolve_includes(
                &tenant,
                std::slice::from_ref(&enc_cond),
                std::slice::from_ref(&service_provider),
            )
            .await
            .unwrap();
        assert!(cond_only.is_empty());

        let subject = IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Encounter".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        let subjects = backend
            .resolve_includes(&tenant, &[enc_cond, enc_org], &[subject])
            .await
            .unwrap();
        assert_eq!(
            include_type_ids(&subjects),
            vec![("Patient".to_string(), "pat-1".to_string())]
        );
    }

    /// `:iterate` follows references transitively through the same shared
    /// resolver: `Encounter:subject` finds the Patient on the first hop, and
    /// `Patient:organization` (marked `iterate`) then follows that Patient to
    /// its managing Organization.
    #[tokio::test]
    async fn es_integration_include_iterate_follows_included_resources() {
        use helios_persistence::core::IncludeProvider;
        use helios_persistence::types::{IncludeDirective, IncludeType};

        let backend = create_backend().await;
        let tenant = create_tenant("include-1013-3");
        seed_include_fixture(&backend, &tenant).await;

        let enc_org = backend
            .read(&tenant, "Encounter", "enc-org")
            .await
            .unwrap()
            .expect("enc-org must exist");

        let includes = vec![
            IncludeDirective {
                include_type: IncludeType::Include,
                source_type: "Encounter".to_string(),
                search_param: "subject".to_string(),
                target_type: None,
                iterate: false,
            },
            IncludeDirective {
                include_type: IncludeType::Include,
                source_type: "Patient".to_string(),
                search_param: "organization".to_string(),
                target_type: None,
                iterate: true,
            },
        ];

        let included = backend
            .resolve_includes(&tenant, &[enc_org], &includes)
            .await
            .unwrap();

        assert_eq!(
            include_type_ids(&included),
            vec![
                ("Organization".to_string(), "org-1".to_string()),
                ("Patient".to_string(), "pat-1".to_string()),
            ]
        );
    }

    // ========================================================================
    // Cursor Pagination Tests (#1015)
    // ========================================================================

    /// Creates Patients `cp-1..cp-n` (inclusive) in the given tenant.
    async fn create_cursor_paging_patients(
        backend: &ElasticsearchBackend,
        tenant: &TenantContext,
        n: u32,
    ) {
        for i in 1..=n {
            backend
                .create(
                    tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("cp-{i}"),
                        "name": [{"family": "CursorPaging"}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
    }

    /// Collects the resource ids of a search result's page, in page order.
    fn page_ids(result: &helios_persistence::core::SearchResult) -> Vec<String> {
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()
    }

    /// Walks forward through 7 Patients 3 at a time (no explicit `_sort`),
    /// then walks all the way back via `previous_cursor` and confirms every
    /// page is reproduced exactly, including the page-3-to-page-2 hop that a
    /// naive truncate-after-reverse implementation gets wrong (#1015).
    #[tokio::test]
    async fn es_integration_cursor_paging_round_trip_previous() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::SearchQuery;

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("cursor-prev");
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient").with_count(3);

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page1.resources.items.len(), 3);
        assert!(!page1.resources.page_info.has_previous);
        assert!(page1.resources.page_info.previous_cursor.is_none());
        assert!(page1.resources.page_info.has_next);
        assert!(page1.resources.page_info.next_cursor.is_some());

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page2.resources.items.len(), 3);
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.previous_cursor.is_some());
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page3.resources.items.len(), 1);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());
        assert!(page3.resources.page_info.previous_cursor.is_some());

        let page1_ids = page_ids(&page1);
        let page2_ids = page_ids(&page2);
        let page3_ids = page_ids(&page3);
        let mut all_ids = page1_ids.clone();
        all_ids.extend(page2_ids.clone());
        all_ids.extend(page3_ids.clone());
        let mut unique_ids = all_ids.clone();
        unique_ids.sort();
        unique_ids.dedup();
        assert_eq!(unique_ids.len(), 7, "all 7 ids must be distinct");

        // Walk back: page 3 -> page 2 must be exact, including order. This is
        // the case a naive truncate-after-reverse gets wrong: it drops the
        // nearest hit instead of the farthest one.
        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), page2_ids);
        assert!(back2.resources.page_info.has_previous);
        assert!(back2.resources.page_info.has_next);
        assert!(back2.resources.page_info.next_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), page1_ids);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.previous_cursor.is_none());
        assert!(back1.resources.page_info.has_next);

        let again2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&again2), page2_ids);
    }

    /// Same round trip, but with an explicit `_sort=_id` ascending so the
    /// exact page contents (not just their distinctness) can be asserted.
    #[tokio::test]
    async fn es_integration_cursor_paging_round_trip_previous_with_sort() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirection, SortDirective};

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("cursor-prev-sort");
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page1), vec!["cp-1", "cp-2", "cp-3"]);

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4", "cp-5", "cp-6"]);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page3), vec!["cp-7"]);

        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), vec!["cp-4", "cp-5", "cp-6"]);

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(back1.resources.page_info.previous_cursor.is_none());
    }

    /// The three backend primitives `Patient/$everything`'s compartment walk
    /// composes — compartment membership (OR across `subject`/`performer`,
    /// see `es_integration_compartment_search`), cursor-paged results (see
    /// `es_integration_cursor_paging_round_trip_previous`), and a
    /// `_lastUpdated ge` filter — pinned together the way the handler uses
    /// them, since ES has no REST-level `$everything` fixture.
    #[tokio::test]
    async fn es_compartment_query_pages_with_cursor_and_since() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompartmentMembership, SearchParamType, SearchParameter, SearchPrefix, SearchQuery,
            SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("everything-compartment-paging");

        for i in 1..=5 {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "x"},
                        "id": format!("o{i}"),
                        "subject": {"reference": "Patient/p1"}
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"text": "x"},
                    "id": "other",
                    "subject": {"reference": "Patient/p2"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait for index refresh
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let mut q = SearchQuery::new("Observation");
        q.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1".to_string(),
        });
        q.count = Some(2);

        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            q.cursor = cursor.take();
            let page = backend.search(&tenant, &q).await.unwrap();
            seen.extend(page.resources.items.iter().map(|r| r.id().to_string()));
            if page.resources.page_info.has_next {
                cursor = page.resources.page_info.next_cursor.clone();
                assert!(cursor.is_some());
            } else {
                break;
            }
        }
        seen.sort();
        assert_eq!(seen, vec!["o1", "o2", "o3", "o4", "o5"]);

        q.cursor = None;
        q.parameters.push(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "2999-01-01T00:00:00Z")],
            chain: vec![],
            components: vec![],
        });
        let page = backend.search(&tenant, &q).await.unwrap();
        assert!(page.resources.items.is_empty());
    }

    /// #1079: a result set whose size is an exact multiple of `_count` must
    /// not emit a `next` link on the last page, forward via cursor.
    #[tokio::test]
    async fn es_integration_cursor_paging_no_phantom_next_when_last_page_full() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirection, SortDirective};

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("cursor-full-page");
        create_cursor_paging_patients(&backend, &tenant, 6).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(page1.resources.page_info.has_next);

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(!page2.resources.page_info.has_next);
        assert!(page2.resources.page_info.next_cursor.is_none());
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.previous_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.has_next);
    }

    /// #1079: the same exact-multiple scenario via `_offset` instead of a
    /// cursor must also avoid a phantom `next` link on the last page.
    #[tokio::test]
    async fn es_integration_offset_paging_no_phantom_next_when_last_page_full() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirection, SortDirective};

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("offset-full-page");
        create_cursor_paging_patients(&backend, &tenant, 6).await;

        let mut query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });
        query.offset = Some(3);

        let page = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(!page.resources.page_info.has_next);
        assert!(page.resources.page_info.next_cursor.is_none());
        assert!(page.resources.page_info.has_previous);
    }

    /// #1079: when more rows remain beyond the requested count, the page
    /// still contains exactly `_count` items (the extra over-fetched hit is
    /// dropped) and `has_next` stays true.
    #[tokio::test]
    async fn es_integration_forward_page_with_more_rows_keeps_next() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirection, SortDirective};

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("forward-more-rows");
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(page1.resources.page_info.has_next);

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page3), vec!["cp-7"]);
        assert!(!page3.resources.page_info.has_next);
    }

    /// #1079: a page whose `count` exactly fills `index.max_result_window`
    /// leaves no room for the over-fetched extra hit. The query builder must
    /// clamp `size` instead of asking Elasticsearch for `from + size =
    /// max_result_window + 1`, which it rejects.
    #[tokio::test]
    async fn es_integration_full_window_page_is_accepted() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::SearchQuery;

        let backend = create_backend_with("1ms", WriteRefreshPolicy::WaitFor).await;
        let tenant = create_tenant("full-window-page");
        create_cursor_paging_patients(&backend, &tenant, 3).await;

        let query = SearchQuery::new("Patient").with_count(10_000);
        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(result.resources.items.len(), 3);
        assert!(!result.resources.page_info.has_next);
        assert!(result.resources.page_info.next_cursor.is_none());
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

    // ========================================================================
    // Client-error classification (#1294)
    // ========================================================================

    /// An Observation whose code display `:text-advanced` can match.
    async fn seed_glucose_observation(backend: &ElasticsearchBackend, tenant: &TenantContext) {
        backend
            .create(
                tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "glucose",
                    "status": "final",
                    "code": { "coding": [{
                        "system": "http://loinc.org",
                        "code": "2339-0",
                        "display": "Glucose"
                    }]}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    fn text_advanced_query(value: &str) -> helios_persistence::types::SearchQuery {
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };
        SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::TextAdvanced),
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        })
    }

    /// Lucene syntax Elasticsearch cannot parse. `:text-advanced` hands the
    /// value to `query_string` verbatim, so this reaches the cluster and is
    /// answered with HTTP 400 `search_phase_execution_exception` /
    /// `query_shard_exception` — a bad request, not a cluster fault.
    const UNPARSEABLE_LUCENE: &str = "Glucose AND (";

    /// #1294: a query Elasticsearch rejects as malformed is the client's
    /// fault. It must fail once, promptly, as a search-parse error (REST 400)
    /// — not be retried with backoff and reported as an internal error.
    #[tokio::test]
    async fn es_integration_bad_query_is_not_retried_and_is_a_client_error() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::error::SearchError;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_glucose_observation(&backend, &tenant).await;

        // Positive control: the same parameter and modifier, well-formed.
        let found = backend
            .search(&tenant, &text_advanced_query("Glucose"))
            .await
            .unwrap();
        assert_eq!(found.resources.items.len(), 1, "positive control");

        let started = std::time::Instant::now();
        let err = backend
            .search(&tenant, &text_advanced_query(UNPARSEABLE_LUCENE))
            .await
            .unwrap_err();
        let elapsed = started.elapsed();

        let StorageError::Search(SearchError::QueryParseError { message }) = &err else {
            panic!("a malformed query must be a QueryParseError, got: {err:?}");
        };
        // The client-facing message is sanitized: no index names, no raw ES
        // exception payload.
        assert!(
            !message.contains("hfs_") && !message.contains("root_cause"),
            "message leaks Elasticsearch internals: {message}"
        );
        // Retrying sleeps 100ms + 200ms before giving up, so a retried query
        // cannot finish under 300ms; one round trip to a local container
        // takes a few milliseconds.
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "a permanent client error must not be retried, took {elapsed:?}"
        );
    }

    /// #1294: `search_count` shares the classification, so `_summary=count`
    /// with the same bad query is a client error too.
    #[tokio::test]
    async fn es_integration_bad_query_count_is_a_client_error() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::error::SearchError;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_glucose_observation(&backend, &tenant).await;

        let count = backend
            .search_count(&tenant, &text_advanced_query("Glucose"))
            .await
            .unwrap();
        assert_eq!(count, 1, "positive control");

        let err = backend
            .search_count(&tenant, &text_advanced_query(UNPARSEABLE_LUCENE))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::QueryParseError { .. })
            ),
            "a malformed count query must be a QueryParseError, got: {err:?}"
        );
    }

    /// The write paths against a real cluster (#1382; the failure paths are in
    /// `elasticsearch_storage_write_wiremock.rs`): what Elasticsearch really
    /// answers for an absent document or index is still `NotFound`, `update`
    /// keeps the contained documents in step, and `delete`/`purge_all` remove
    /// them and report real counts.
    #[tokio::test]
    async fn es_integration_writes_keep_contained_documents_in_step() {
        use helios_persistence::core::{PurgableStorage, SearchProvider};
        use helios_persistence::error::ResourceError;
        use helios_persistence::types::{
            ContainedMode, ContainedReturn, SearchParamType, SearchParameter, SearchQuery,
            SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("write-path-tenant");
        let containers_of = |org_name: &str| {
            let mut q = SearchQuery::new("Organization");
            q.contained = ContainedMode::On;
            q.contained_return = ContainedReturn::Container;
            q.parameters.push(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq(org_name)],
                chain: vec![],
                components: vec![],
            });
            q
        };
        let search_urls = |q: SearchQuery| {
            let backend = &backend;
            let tenant = &tenant;
            async move {
                backend
                    .refresh_index("write-path-tenant", "Organization")
                    .await
                    .ok();
                let found = backend.search(tenant, &q).await.unwrap();
                let mut urls: Vec<String> = found.resources.items.iter().map(|r| r.url()).collect();
                urls.sort();
                urls
            }
        };
        let is_not_found = |r: Result<(), StorageError>| {
            matches!(
                r,
                Err(StorageError::Resource(ResourceError::NotFound { .. }))
            )
        };
        let patient = |id: &str, org: &str| {
            json!({
                "resourceType": "Patient", "id": id,
                "contained": [{ "resourceType": "Organization", "id": "org", "name": org }],
                "managingOrganization": { "reference": "#org" }
            })
        };

        // No index yet: Elasticsearch's `index_not_found_exception`.
        assert!(is_not_found(backend.delete(&tenant, "Patient", "p1").await));

        let created = backend
            .create(
                &tenant,
                "Patient",
                patient("p1", "Acme"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                patient("p2", "Acme"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            search_urls(containers_of("Acme")).await,
            vec!["Patient/p1", "Patient/p2"],
            "positive control: contained documents are indexed and found"
        );

        // The index exists now: Elasticsearch's `"result": "not_found"`.
        assert!(is_not_found(
            backend.delete(&tenant, "Patient", "absent").await
        ));
        backend
            .purge(&tenant, "Patient", "absent")
            .await
            .expect("purging an absent document is idempotent");

        // `update` replaces the contained documents (it used to leave them).
        backend
            .update(&tenant, &created, patient("p1", "Globex"))
            .await
            .unwrap();
        assert_eq!(search_urls(containers_of("Acme")).await, vec!["Patient/p2"]);
        assert_eq!(
            search_urls(containers_of("Globex")).await,
            vec!["Patient/p1"]
        );

        // `delete` sweeps them.
        backend.delete(&tenant, "Patient", "p1").await.unwrap();
        assert!(search_urls(containers_of("Globex")).await.is_empty());

        // `purge_all` reports what it removed and sweeps the rest.
        assert_eq!(backend.purge_all(&tenant, "Patient").await.unwrap(), 1);
        assert!(search_urls(containers_of("Acme")).await.is_empty());
        assert_eq!(backend.purge_all(&tenant, "Patient").await.unwrap(), 0);
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
