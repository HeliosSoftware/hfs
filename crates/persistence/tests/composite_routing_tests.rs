//! Integration tests for query routing logic in composite storage.
//!
//! This module tests query decomposition and routing decisions
//! for directing queries to appropriate backends based on capabilities.

use std::collections::HashSet;

use helios_persistence::composite::{
    CompositeConfigBuilder, QueryAnalyzer, QueryFeature, QueryRouter, decompose_query,
    detect_query_features, features_to_capabilities,
};
use helios_persistence::core::{BackendCapability, BackendKind};
use helios_persistence::types::{
    ChainedParameter, IncludeDirective, IncludeType, ReverseChainedParameter, SearchModifier,
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue, SortDirective,
};

// ============================================================================
// Query Feature Detection Tests
// ============================================================================

/// Test detection of basic search features.
#[test]
fn test_detect_basic_search_features() {
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::BasicSearch));
    assert!(features.contains(&QueryFeature::StringSearch));
}

/// Test detection of date search features.
#[test]
fn test_detect_date_search_features() {
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "1990-01-01")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::DateSearch));
}

/// Test detection of token search features.
#[test]
fn test_detect_token_search_features() {
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::TokenSearch));
}

/// Test detection of reference search features.
#[test]
fn test_detect_reference_search_features() {
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Patient/123")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::ReferenceSearch));
}

/// Test detection of chained search features.
#[test]
fn test_detect_chained_search_features() {
    // Chained search is expressed via chain field on SearchParameter
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::ChainedSearch));
}

/// Test detection of reverse chained search (_has).
#[test]
fn test_detect_reverse_chained_features() {
    let mut query = SearchQuery::new("Patient");
    query.reverse_chains.push(ReverseChainedParameter::terminal(
        "Observation",
        "subject",
        "code",
        SearchValue::token(Some("http://loinc.org"), "8867-4"),
    ));

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::ReverseChaining));
}

/// Test detection of _include features.
#[test]
fn test_detect_include_features() {
    let query = SearchQuery::new("Observation").with_include(IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::Include));
}

/// Test detection of _revinclude features.
#[test]
fn test_detect_revinclude_features() {
    let query = SearchQuery::new("Patient").with_include(IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::Revinclude));
}

/// Test detection of full-text search (_text, _content).
#[test]
fn test_detect_fulltext_search_features() {
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("cardiac patient")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::FullTextSearch));
}

/// Test detection of terminology search (code:below).
#[test]
fn test_detect_terminology_search_features() {
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Below),
        values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
        chain: vec![],
        components: vec![],
    });

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::TerminologySearch));
}

/// Test detection of sorting features.
#[test]
fn test_detect_sorting_features() {
    let query = SearchQuery::new("Patient").with_sort(SortDirective::parse("-_lastUpdated"));

    let features = detect_query_features(&query);

    assert!(features.contains(&QueryFeature::Sorting));
}

// ============================================================================
// Query Routing Tests
// ============================================================================

/// Test routing a simple query to primary backend.
#[test]
fn test_route_simple_query_to_primary() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("patient-123")],
        chain: vec![],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    assert_eq!(routing.primary_target, "primary");
    assert!(routing.auxiliary_targets.is_empty());
}

/// Test routing a basic query in mongodb-elasticsearch mode.
#[test]
fn test_route_basic_query_mongodb_primary() {
    let config = CompositeConfigBuilder::new()
        .primary("mongodb", BackendKind::MongoDB)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(
            Some("http://hospital.org/mrn"),
            "MONGO-123",
        )],
        chain: vec![],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    assert_eq!(routing.primary_target, "mongodb");
    assert!(
        routing.auxiliary_targets.is_empty(),
        "Basic token search should remain on mongodb primary"
    );
}

/// Test routing full-text query to Elasticsearch in mongodb-elasticsearch mode.
#[test]
fn test_route_fulltext_query_mongodb_elasticsearch_split() {
    let config = CompositeConfigBuilder::new()
        .primary("mongodb", BackendKind::MongoDB)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("congestive heart failure")],
        chain: vec![],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    assert_eq!(routing.primary_target, "mongodb");
    assert!(
        routing
            .auxiliary_targets
            .values()
            .any(|backend_id| backend_id == "elasticsearch"),
        "Full-text search should route to Elasticsearch secondary"
    );
}

/// Test routing chained search to graph backend.
#[test]
fn test_route_chained_search_to_graph() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .graph_backend("neo4j", BackendKind::Neo4j)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    // Chained search expressed via chain field
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    // Chained searches benefit from graph backend
    assert!(
        routing.auxiliary_targets.values().any(|v| v == "neo4j")
            || routing.primary_target == "neo4j"
    );
}

/// Test routing full-text search to search backend.
#[test]
fn test_route_fulltext_to_search_backend() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("chronic heart failure")],
        chain: vec![],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    // Full-text searches should use search backend
    assert!(
        routing
            .auxiliary_targets
            .values()
            .any(|v| v == "elasticsearch")
            || routing.primary_target == "elasticsearch"
    );
}

/// Test routing terminology search to terminology service.
#[test]
fn test_route_terminology_to_terminology_service() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .terminology_backend("terminology", BackendKind::Postgres)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Below),
        values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
        chain: vec![],
        components: vec![],
    });

    let routing = router.route(&query).unwrap();

    // Terminology expansion should involve terminology service
    assert!(
        routing
            .auxiliary_targets
            .values()
            .any(|v| v == "terminology")
    );
}

/// Test routing complex query to multiple backends.
#[test]
fn test_route_complex_query_to_multiple_backends() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
        .graph_backend("neo4j", BackendKind::Neo4j)
        .terminology_backend("terminology", BackendKind::Postgres)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    // Complex query: chained search + full-text + terminology + _include
    let query = SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("smith")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
            }],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("cardiac")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });

    let routing = router.route(&query).unwrap();

    // Complex query should involve multiple backends
    assert!(!routing.auxiliary_targets.is_empty());
}

// ============================================================================
// Capability Matching Tests
// ============================================================================

/// Test matching query features to backend capabilities.
#[test]
fn test_match_features_to_capabilities() {
    let features: HashSet<QueryFeature> = [
        QueryFeature::BasicSearch,
        QueryFeature::StringSearch,
        QueryFeature::Sorting,
    ]
    .into_iter()
    .collect();

    let required_caps = features_to_capabilities(&features);

    assert!(required_caps.contains(&BackendCapability::BasicSearch));
    assert!(required_caps.contains(&BackendCapability::Sorting));
}

/// Test that graph-specific features require graph capabilities.
#[test]
fn test_graph_features_require_graph_capabilities() {
    let features: HashSet<QueryFeature> =
        [QueryFeature::ChainedSearch, QueryFeature::ReverseChaining]
            .into_iter()
            .collect();

    let required_caps = features_to_capabilities(&features);

    assert!(required_caps.contains(&BackendCapability::ChainedSearch));
    assert!(required_caps.contains(&BackendCapability::ReverseChaining));
}

/// Test that full-text features require search capabilities.
#[test]
fn test_fulltext_features_require_search_capabilities() {
    let features: HashSet<QueryFeature> = [QueryFeature::FullTextSearch].into_iter().collect();

    let required_caps = features_to_capabilities(&features);

    assert!(required_caps.contains(&BackendCapability::FullTextSearch));
}

// ============================================================================
// Query Decomposition Tests
// ============================================================================

/// Test decomposing a complex query into backend-specific parts.
#[test]
fn test_decompose_query() {
    let query = SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("cardiac")],
            chain: vec![],
            components: vec![],
        });

    let parts = decompose_query(&query);

    // Should have at least primary part
    assert!(!parts.is_empty());

    // Token search can go to primary (BasicSearch feature)
    let primary_part = parts
        .iter()
        .find(|p| p.feature == QueryFeature::BasicSearch);
    assert!(primary_part.is_some());

    // Full-text should have FullTextSearch feature
    let search_part = parts
        .iter()
        .find(|p| p.feature == QueryFeature::FullTextSearch);
    assert!(search_part.is_some());
}

/// Test that decomposition preserves all query parameters.
#[test]
fn test_decomposition_preserves_parameters() {
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
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "1990-01-01")],
            chain: vec![],
            components: vec![],
        });

    let parts = decompose_query(&query);

    // Count total parameters across all parts
    let total_params: usize = parts.iter().map(|p| p.parameters.len()).sum();

    // Should account for all original parameters (may be distributed)
    assert!(total_params >= 2);
}

// ============================================================================
// Query Analyzer Tests
// ============================================================================

/// Test QueryAnalyzer provides complete analysis.
#[test]
fn test_query_analyzer_full_analysis() {
    let analyzer = QueryAnalyzer::new();

    let query = SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
            chain: vec![],
            components: vec![],
        })
        .with_sort(SortDirective::parse("-date"));

    let analysis = analyzer.analyze(&query);

    assert!(!analysis.features.is_empty());
    assert!(!analysis.required_capabilities.is_empty());
    assert!(analysis.complexity_score >= 1);
}

/// Test complexity score increases with query complexity.
#[test]
fn test_complexity_increases_with_features() {
    let analyzer = QueryAnalyzer::new();

    // Simple query
    let simple = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![],
        components: vec![],
    });

    // Complex query with chaining and includes
    let complex = SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
            }],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("cardiac")],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });

    let simple_analysis = analyzer.analyze(&simple);
    let complex_analysis = analyzer.analyze(&complex);

    assert!(complex_analysis.complexity_score >= simple_analysis.complexity_score);
}
