//! Integration tests for polyglot/multi-backend storage coordination.
//!
//! This module tests how multiple backends are coordinated for
//! complex queries that span different storage engines, using the
//! real composite module types.

use std::collections::HashSet;

use serde_json::json;

use helios_persistence::composite::{
    CompositeConfigBuilder, MergeStrategy, QueryAnalyzer, QueryFeature, QueryRouter, ResultMerger,
    detect_query_features, features_to_capabilities,
};
use helios_persistence::core::{BackendCapability, BackendKind};
use helios_persistence::types::{
    ChainedParameter, IncludeDirective, IncludeType, SearchModifier, SearchParamType,
    SearchParameter, SearchQuery, SearchValue,
};

// ============================================================================
// Query Feature Detection Integration Tests
// ============================================================================

/// Test detection of multiple features in a complex query.
#[test]
fn test_detect_multiple_features() {
    // Complex query with chained search + full-text + terminology + include
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
                has: None,
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
            search_param: "patient".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });

    let features = detect_query_features(&query);

    // Should detect all complex features
    assert!(features.contains(&QueryFeature::ChainedSearch));
    assert!(features.contains(&QueryFeature::FullTextSearch));
    assert!(features.contains(&QueryFeature::TerminologySearch));
    assert!(features.contains(&QueryFeature::Include));
}

/// Test that features are correctly mapped to capabilities.
#[test]
fn test_features_map_to_capabilities() {
    let features: HashSet<QueryFeature> = [
        QueryFeature::ChainedSearch,
        QueryFeature::FullTextSearch,
        QueryFeature::TerminologySearch,
        QueryFeature::Include,
    ]
    .into_iter()
    .collect();

    let required = features_to_capabilities(&features);

    assert!(required.contains(&BackendCapability::ChainedSearch));
    assert!(required.contains(&BackendCapability::FullTextSearch));
    assert!(required.contains(&BackendCapability::Include));
}

// ============================================================================
// Query Router Integration Tests
// ============================================================================

/// Test routing a polyglot query to multiple backends.
#[test]
fn test_polyglot_query_routing() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
        .graph_backend("neo4j", BackendKind::Neo4j)
        .terminology_backend("terminology", BackendKind::Postgres)
        .build()
        .unwrap();

    let router = QueryRouter::new(config);

    // Complex query that needs multiple backends
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
                has: None,
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
        });

    let routing = router.route(&query).unwrap();

    // Should route to multiple backends
    assert!(!routing.auxiliary_targets.is_empty());
}

/// Test that simple queries use only primary.
#[test]
fn test_simple_query_primary_only() {
    let config = CompositeConfigBuilder::new()
        .primary("primary", BackendKind::Sqlite)
        .search_backend("elasticsearch", BackendKind::Elasticsearch)
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

// ============================================================================
// Result Merging Tests
// ============================================================================

/// Test intersection merge strategy.
#[test]
fn test_result_merger_intersection() {
    let merger = ResultMerger::new();

    // Create mock results (just IDs for this test)
    let primary_ids: Vec<String> = vec!["1", "2", "3"].into_iter().map(String::from).collect();
    let graph_ids: Vec<String> = vec!["2", "3", "4"].into_iter().map(String::from).collect();
    let search_ids: Vec<String> = vec!["1", "3", "5"].into_iter().map(String::from).collect();

    // Intersection: only IDs found by all
    let merged = merger.merge_ids(
        vec![primary_ids, graph_ids, search_ids],
        MergeStrategy::Intersection,
    );

    assert_eq!(merged.len(), 1);
    assert!(merged.contains(&"3".to_string()));
}

/// Test union merge strategy.
#[test]
fn test_result_merger_union() {
    let merger = ResultMerger::new();

    let primary_ids: Vec<String> = vec!["1", "2"].into_iter().map(String::from).collect();
    let secondary_ids: Vec<String> = vec!["2", "3"].into_iter().map(String::from).collect();

    let merged = merger.merge_ids(vec![primary_ids, secondary_ids], MergeStrategy::Union);

    assert_eq!(merged.len(), 3);
    assert!(merged.contains(&"1".to_string()));
    assert!(merged.contains(&"2".to_string()));
    assert!(merged.contains(&"3".to_string()));
}

/// Test primary enriched merge strategy (falls back to intersection for ID-only merge).
#[test]
fn test_result_merger_primary_enriched() {
    let merger = ResultMerger::new();

    let primary_ids: Vec<String> = vec!["1", "2", "3"].into_iter().map(String::from).collect();
    let secondary_ids: Vec<String> = vec!["2", "3", "4", "5"]
        .into_iter()
        .map(String::from)
        .collect();

    // PrimaryEnriched falls back to intersection for ID-only merge
    let merged = merger.merge_ids(
        vec![primary_ids, secondary_ids],
        MergeStrategy::PrimaryEnriched,
    );

    // Should include intersection of IDs (2, 3)
    assert!(merged.contains(&"2".to_string()));
    assert!(merged.contains(&"3".to_string()));
}

// ============================================================================
// Query Analyzer Integration Tests
// ============================================================================

/// Test analyzer provides complete analysis.
#[test]
fn test_analyzer_complete_analysis() {
    let analyzer = QueryAnalyzer::new();

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
                has: None,
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
        });

    let analysis = analyzer.analyze(&query);

    // Should have features and capabilities
    assert!(!analysis.features.is_empty());
    assert!(!analysis.required_capabilities.is_empty());
    assert!(analysis.complexity_score > 0);
}

/// Test complexity score reflects query complexity.
#[test]
fn test_complexity_score_reflects_complexity() {
    let analyzer = QueryAnalyzer::new();

    // Simple query
    let simple = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("123")],
        chain: vec![],
        components: vec![],
    });

    // Complex query
    let complex = SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("smith")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
                has: None,
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
        });

    let simple_analysis = analyzer.analyze(&simple);
    let complex_analysis = analyzer.analyze(&complex);

    assert!(complex_analysis.complexity_score > simple_analysis.complexity_score);
}

// ============================================================================
// Conceptual Tests (Design Verification)
// ============================================================================

/// Test merging results from multiple backends (conceptual).
#[test]
fn test_merge_results_conceptual() {
    // Simulate results from different backends
    let primary_results = ["Patient/1", "Patient/2", "Patient/3"];
    let graph_results = ["Patient/2", "Patient/3", "Patient/4"];
    let search_results = ["Patient/1", "Patient/3", "Patient/5"];

    // Intersection strategy: only resources found by all
    let merged_intersection: Vec<_> = primary_results
        .iter()
        .filter(|r| graph_results.contains(r) && search_results.contains(r))
        .collect();

    assert_eq!(merged_intersection, vec![&"Patient/3"]);
}

/// Test pagination coordination concept.
#[test]
fn test_pagination_coordination_conceptual() {
    // Backend A returns sorted IDs: [1, 3, 5, 7, 9]
    // Backend B returns sorted IDs: [2, 4, 6, 8, 10]
    let backend_a_ids: Vec<i32> = vec![1, 3, 5, 7, 9];
    let backend_b_ids: Vec<i32> = vec![2, 4, 6, 8, 10];

    // Merge and sort
    let mut all_ids: Vec<i32> = backend_a_ids
        .iter()
        .chain(backend_b_ids.iter())
        .cloned()
        .collect();
    all_ids.sort();

    // Page 1 (count=3)
    let page1: Vec<_> = all_ids.iter().take(3).collect();
    assert_eq!(page1, vec![&1, &2, &3]);

    // Page 2 (skip=3, count=3)
    let page2: Vec<_> = all_ids.iter().skip(3).take(3).collect();
    assert_eq!(page2, vec![&4, &5, &6]);
}

/// Test include resolution concept.
#[test]
fn test_include_resolution_conceptual() {
    // Scenario: Search returns IDs, then _include resolves references

    // Step 1: Get base resource IDs from search
    let _search_result_ids = ["Observation/1", "Observation/2"];

    // Step 2: Load full resources from primary
    let observations = [
        json!({
            "resourceType": "Observation",
            "id": "1",
            "subject": {"reference": "Patient/A"}
        }),
        json!({
            "resourceType": "Observation",
            "id": "2",
            "subject": {"reference": "Patient/B"}
        }),
    ];

    // Step 3: Extract references for _include
    let referenced_patients: Vec<_> = observations
        .iter()
        .filter_map(|o| o["subject"]["reference"].as_str())
        .collect();

    assert_eq!(referenced_patients, vec!["Patient/A", "Patient/B"]);
}

/// Test parallel query execution concept.
#[test]
fn test_parallel_execution_conceptual() {
    // Simulate parallel execution timing
    let primary_latency_ms = 50;
    let search_latency_ms = 30;
    let graph_latency_ms = 40;

    // Sequential: 50 + 30 + 40 = 120ms
    let sequential_total = primary_latency_ms + search_latency_ms + graph_latency_ms;

    // Parallel: max(50, 30, 40) = 50ms
    let parallel_total = *[primary_latency_ms, search_latency_ms, graph_latency_ms]
        .iter()
        .max()
        .unwrap();

    assert!(parallel_total < sequential_total);
    assert_eq!(parallel_total, 50);
}

/// Test eventual consistency model expectations.
#[test]
fn test_eventual_consistency_conceptual() {
    #[derive(Debug)]
    #[allow(dead_code)]
    struct ConsistencyExpectation {
        backend: &'static str,
        consistency: &'static str,
    }

    let expectations = [
        ConsistencyExpectation {
            backend: "Primary",
            consistency: "Strong",
        },
        ConsistencyExpectation {
            backend: "Search",
            consistency: "Eventual",
        },
        ConsistencyExpectation {
            backend: "Graph",
            consistency: "Eventual",
        },
    ];

    // Primary is always strongly consistent
    assert_eq!(expectations[0].consistency, "Strong");

    // Secondary backends are eventually consistent
    assert_eq!(expectations[1].consistency, "Eventual");
    assert_eq!(expectations[2].consistency, "Eventual");
}
