//! Query feature detection and analysis.
//!
//! This module provides query analysis to detect features that determine
//! which backends should handle different parts of a query.
//!
//! # Feature Detection Rules
//!
//! The analyzer detects features based on query characteristics:
//!
//! | Feature | Detection |
//! |---------|-----------|
//! | ChainedSearch | Parameters with non-empty `chain` field |
//! | ReverseChaining | `_has` parameter |
//! | FullTextSearch | `_text` or `_content` parameters |
//! | TerminologySearch | Modifiers `:above`, `:below`, `:in`, `:not-in` |
//! | Include | `_include` directives |
//! | Revinclude | `_revinclude` directives |

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::core::BackendCapability;
use crate::types::{IncludeType, SearchModifier, SearchParamType, SearchParameter, SearchQuery};

/// Features detected in a search query.
///
/// These features are used to route queries to appropriate backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryFeature {
    /// Basic search with simple parameters.
    BasicSearch,

    /// ID lookup (_id parameter).
    IdLookup,

    /// String parameter search.
    StringSearch,

    /// Token parameter search.
    TokenSearch,

    /// Date parameter search.
    DateSearch,

    /// Number parameter search.
    NumberSearch,

    /// Quantity parameter search.
    QuantitySearch,

    /// Reference parameter search.
    ReferenceSearch,

    /// URI parameter search.
    UriSearch,

    /// Composite parameter search.
    CompositeSearch,

    /// Chained parameter search (e.g., patient.name).
    ChainedSearch,

    /// Reverse chaining (_has parameter).
    ReverseChaining,

    /// Full-text search (_text, _content).
    FullTextSearch,

    /// Terminology expansion (:above, :below, :in, :not-in).
    TerminologySearch,

    /// _include directive.
    Include,

    /// _revinclude directive.
    Revinclude,

    /// Iterate include (_include:iterate).
    IterateInclude,

    /// Sorting (_sort parameter).
    Sorting,

    /// Cursor-based pagination.
    CursorPagination,

    /// Offset-based pagination.
    OffsetPagination,

    /// Total count requested.
    TotalCount,

    /// Summary mode requested.
    Summary,
}

impl QueryFeature {
    /// Returns the backend capability required for this feature.
    pub fn required_capability(&self) -> Option<BackendCapability> {
        match self {
            QueryFeature::BasicSearch
            | QueryFeature::IdLookup
            | QueryFeature::StringSearch
            | QueryFeature::TokenSearch
            | QueryFeature::ReferenceSearch
            | QueryFeature::UriSearch
            | QueryFeature::CompositeSearch => Some(BackendCapability::BasicSearch),

            QueryFeature::DateSearch => Some(BackendCapability::DateSearch),
            QueryFeature::NumberSearch | QueryFeature::QuantitySearch => {
                Some(BackendCapability::QuantitySearch)
            }

            QueryFeature::ChainedSearch => Some(BackendCapability::ChainedSearch),
            QueryFeature::ReverseChaining => Some(BackendCapability::ReverseChaining),
            QueryFeature::FullTextSearch => Some(BackendCapability::FullTextSearch),
            QueryFeature::TerminologySearch => Some(BackendCapability::TerminologySearch),

            QueryFeature::Include | QueryFeature::IterateInclude => {
                Some(BackendCapability::Include)
            }
            QueryFeature::Revinclude => Some(BackendCapability::Revinclude),

            QueryFeature::Sorting => Some(BackendCapability::Sorting),
            QueryFeature::CursorPagination => Some(BackendCapability::CursorPagination),
            QueryFeature::OffsetPagination => Some(BackendCapability::OffsetPagination),

            QueryFeature::TotalCount | QueryFeature::Summary => None,
        }
    }

    /// Returns true if this feature typically benefits from a specialized backend.
    pub fn prefers_specialized_backend(&self) -> bool {
        matches!(
            self,
            QueryFeature::ChainedSearch
                | QueryFeature::ReverseChaining
                | QueryFeature::FullTextSearch
                | QueryFeature::TerminologySearch
        )
    }
}

/// Terminology operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminologyOp {
    /// :above modifier - find codes above in hierarchy.
    Above,
    /// :below modifier - find codes below in hierarchy.
    Below,
    /// :in modifier - find codes in value set.
    In,
    /// :not-in modifier - find codes not in value set.
    NotIn,
}

/// Result of analyzing a search query.
#[derive(Debug, Clone)]
pub struct QueryAnalysis {
    /// All detected features.
    pub features: HashSet<QueryFeature>,

    /// Required backend capabilities.
    pub required_capabilities: HashSet<BackendCapability>,

    /// Estimated complexity score (1-10).
    /// Higher scores indicate more complex queries.
    pub complexity_score: u8,

    /// Parameters grouped by feature.
    pub feature_params: HashMap<QueryFeature, Vec<SearchParameter>>,

    /// Whether the query can be split across multiple backends.
    pub splittable: bool,

    /// Features that benefit from specialized backends.
    pub specialized_features: HashSet<QueryFeature>,

    /// Detected terminology operations.
    pub terminology_ops: Vec<(String, TerminologyOp)>,
}

impl QueryAnalysis {
    /// Creates an empty analysis.
    pub fn empty() -> Self {
        Self {
            features: HashSet::new(),
            required_capabilities: HashSet::new(),
            complexity_score: 1,
            feature_params: HashMap::new(),
            splittable: true,
            specialized_features: HashSet::new(),
            terminology_ops: Vec::new(),
        }
    }

    /// Returns true if any advanced features are detected.
    pub fn has_advanced_features(&self) -> bool {
        self.features
            .iter()
            .any(|f| f.prefers_specialized_backend())
    }

    /// Returns true if the query uses chained parameters.
    pub fn has_chaining(&self) -> bool {
        self.features.contains(&QueryFeature::ChainedSearch)
            || self.features.contains(&QueryFeature::ReverseChaining)
    }

    /// Returns true if the query uses full-text search.
    pub fn has_fulltext(&self) -> bool {
        self.features.contains(&QueryFeature::FullTextSearch)
    }

    /// Returns true if the query uses terminology operations.
    pub fn has_terminology(&self) -> bool {
        self.features.contains(&QueryFeature::TerminologySearch)
    }

    /// Returns true if the query uses includes.
    pub fn has_includes(&self) -> bool {
        self.features.contains(&QueryFeature::Include)
            || self.features.contains(&QueryFeature::Revinclude)
    }
}

/// Query analyzer that detects features in search queries.
#[derive(Debug, Clone, Default)]
pub struct QueryAnalyzer {
    /// Custom feature patterns (for extensibility).
    _custom_patterns: Vec<()>,
}

impl QueryAnalyzer {
    /// Creates a new analyzer with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyzes a query and returns detected features.
    pub fn analyze(&self, query: &SearchQuery) -> QueryAnalysis {
        let mut analysis = QueryAnalysis::empty();

        // Always add basic search if there are parameters
        if !query.parameters.is_empty() || !query.includes.is_empty() {
            analysis.features.insert(QueryFeature::BasicSearch);
        }

        // Analyze each parameter
        for param in &query.parameters {
            self.analyze_parameter(param, &mut analysis);
        }

        // Analyze reverse chains
        for reverse_chain in &query.reverse_chains {
            analysis.features.insert(QueryFeature::ReverseChaining);
            analysis
                .specialized_features
                .insert(QueryFeature::ReverseChaining);

            // Add to feature params with synthetic parameter
            // We don't populate chain since that's for forward chaining
            analysis
                .feature_params
                .entry(QueryFeature::ReverseChaining)
                .or_default()
                .push(SearchParameter {
                    name: format!(
                        "_has:{}:{}:{}",
                        reverse_chain.source_type,
                        reverse_chain.reference_param,
                        reverse_chain.terminal_param_as_written()
                    ),
                    param_type: SearchParamType::Special,
                    modifier: None,
                    values: reverse_chain.value.clone().into_iter().collect(),
                    chain: vec![],
                    components: vec![],
                });
        }

        // Analyze includes
        for include in &query.includes {
            match include.include_type {
                IncludeType::Include => {
                    if include.iterate {
                        analysis.features.insert(QueryFeature::IterateInclude);
                    } else {
                        analysis.features.insert(QueryFeature::Include);
                    }
                }
                IncludeType::Revinclude => {
                    analysis.features.insert(QueryFeature::Revinclude);
                }
            }
        }

        // Analyze sorting
        if !query.sort.is_empty() {
            analysis.features.insert(QueryFeature::Sorting);
        }

        // Analyze pagination
        if query.cursor.is_some() {
            analysis.features.insert(QueryFeature::CursorPagination);
        } else if query.offset.is_some() {
            analysis.features.insert(QueryFeature::OffsetPagination);
        }

        // Analyze total count
        if query.total.is_some() {
            analysis.features.insert(QueryFeature::TotalCount);
        }

        // Analyze summary
        if query.summary.is_some() {
            analysis.features.insert(QueryFeature::Summary);
        }

        // Calculate required capabilities
        for feature in &analysis.features {
            if let Some(cap) = feature.required_capability() {
                analysis.required_capabilities.insert(cap);
            }
        }

        // Calculate complexity score
        analysis.complexity_score = self.calculate_complexity(&analysis);

        // Determine if splittable
        analysis.splittable = self.is_splittable(&analysis);

        analysis
    }

    /// Analyzes a single parameter.
    fn analyze_parameter(&self, param: &SearchParameter, analysis: &mut QueryAnalysis) {
        // Check for ID lookup
        if param.name == "_id" {
            analysis.features.insert(QueryFeature::IdLookup);
            return;
        }

        // Check for full-text search
        if param.name == "_text" || param.name == "_content" {
            analysis.features.insert(QueryFeature::FullTextSearch);
            analysis
                .specialized_features
                .insert(QueryFeature::FullTextSearch);
            analysis
                .feature_params
                .entry(QueryFeature::FullTextSearch)
                .or_default()
                .push(param.clone());
            return;
        }

        // Check for chained search
        if !param.chain.is_empty() {
            analysis.features.insert(QueryFeature::ChainedSearch);
            analysis
                .specialized_features
                .insert(QueryFeature::ChainedSearch);
            analysis
                .feature_params
                .entry(QueryFeature::ChainedSearch)
                .or_default()
                .push(param.clone());
        }

        // Check for terminology modifiers
        if let Some(ref modifier) = param.modifier {
            if let Some(term_op) = self.parse_terminology_modifier(modifier) {
                analysis.features.insert(QueryFeature::TerminologySearch);
                analysis
                    .specialized_features
                    .insert(QueryFeature::TerminologySearch);
                analysis.terminology_ops.push((param.name.clone(), term_op));
                analysis
                    .feature_params
                    .entry(QueryFeature::TerminologySearch)
                    .or_default()
                    .push(param.clone());
            }
        }

        // Check for text modifier (for token parameters)
        if let Some(SearchModifier::Text) = param.modifier {
            if param.param_type == SearchParamType::Token {
                analysis.features.insert(QueryFeature::FullTextSearch);
            }
        }

        // Detect parameter type features
        let type_feature = match param.param_type {
            SearchParamType::String => QueryFeature::StringSearch,
            SearchParamType::Token => QueryFeature::TokenSearch,
            SearchParamType::Date => QueryFeature::DateSearch,
            SearchParamType::Number => QueryFeature::NumberSearch,
            SearchParamType::Quantity => QueryFeature::QuantitySearch,
            SearchParamType::Reference => QueryFeature::ReferenceSearch,
            SearchParamType::Uri => QueryFeature::UriSearch,
            SearchParamType::Composite => QueryFeature::CompositeSearch,
            SearchParamType::Special => QueryFeature::BasicSearch,
        };
        analysis.features.insert(type_feature);

        // Add to feature params for basic types (if not already categorized)
        if param.chain.is_empty()
            && !analysis
                .specialized_features
                .contains(&QueryFeature::TerminologySearch)
        {
            analysis
                .feature_params
                .entry(QueryFeature::BasicSearch)
                .or_default()
                .push(param.clone());
        }
    }

    /// Parses a terminology modifier.
    fn parse_terminology_modifier(&self, modifier: &SearchModifier) -> Option<TerminologyOp> {
        match modifier {
            SearchModifier::Above => Some(TerminologyOp::Above),
            SearchModifier::Below => Some(TerminologyOp::Below),
            SearchModifier::In => Some(TerminologyOp::In),
            SearchModifier::NotIn => Some(TerminologyOp::NotIn),
            _ => None,
        }
    }

    /// Calculates a complexity score (1-10).
    fn calculate_complexity(&self, analysis: &QueryAnalysis) -> u8 {
        let mut score = 1u8;

        // Add for advanced features
        if analysis.has_chaining() {
            score = score.saturating_add(2);
        }
        if analysis.has_fulltext() {
            score = score.saturating_add(1);
        }
        if analysis.has_terminology() {
            score = score.saturating_add(2);
        }
        if analysis.has_includes() {
            score = score.saturating_add(1);
        }

        // Add for number of features
        let feature_count = analysis.features.len();
        if feature_count > 5 {
            score = score.saturating_add(1);
        }
        if feature_count > 8 {
            score = score.saturating_add(1);
        }

        // Add for reverse chaining depth
        if analysis.features.contains(&QueryFeature::ReverseChaining) {
            score = score.saturating_add(1);
        }

        // Cap at 10
        score.min(10)
    }

    /// Determines if the query can be split across backends.
    fn is_splittable(&self, _analysis: &QueryAnalysis) -> bool {
        // Queries are splittable unless they have tight coupling
        // between parameters that must be evaluated together

        // For now, most queries are splittable
        // This can be refined based on specific query patterns
        true
    }

    /// Returns features for a specific parameter.
    pub fn features_for_param(&self, param: &SearchParameter) -> HashSet<QueryFeature> {
        let mut analysis = QueryAnalysis::empty();
        self.analyze_parameter(param, &mut analysis);
        analysis.features
    }
}

/// Convert query features to backend capabilities.
pub fn features_to_capabilities(features: &HashSet<QueryFeature>) -> HashSet<BackendCapability> {
    features
        .iter()
        .filter_map(|f| f.required_capability())
        .collect()
}

/// Detects features from a query (convenience function).
pub fn detect_query_features(query: &SearchQuery) -> HashSet<QueryFeature> {
    QueryAnalyzer::new().analyze(query).features
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChainedParameter, IncludeDirective, SearchModifier, SearchValue, SortDirective,
    };

    #[test]
    fn test_detect_basic_search() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::string("Smith")],
            chain: vec![],
            components: vec![],
        });

        let features = detect_query_features(&query);
        assert!(features.contains(&QueryFeature::BasicSearch));
        assert!(features.contains(&QueryFeature::StringSearch));
    }

    #[test]
    fn test_detect_chained_search() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::string("Smith")],
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

    #[test]
    fn test_detect_fulltext_search() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::string("cardiac")],
            chain: vec![],
            components: vec![],
        });

        let features = detect_query_features(&query);
        assert!(features.contains(&QueryFeature::FullTextSearch));
    }

    #[test]
    fn test_detect_terminology_search() {
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

    #[test]
    fn test_detect_terminology_search_above() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Above),
            values: vec![SearchValue::token(Some("http://loinc.org"), "8867-4")],
            chain: vec![],
            components: vec![],
        });

        let features = detect_query_features(&query);
        assert!(features.contains(&QueryFeature::TerminologySearch));
    }

    #[test]
    fn test_detect_include() {
        let query = SearchQuery::new("Observation").with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "patient".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });

        let features = detect_query_features(&query);
        assert!(features.contains(&QueryFeature::Include));
    }

    #[test]
    fn test_detect_revinclude() {
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

    #[test]
    fn test_detect_sorting() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective::parse("-_lastUpdated"));

        let features = detect_query_features(&query);
        assert!(features.contains(&QueryFeature::Sorting));
    }

    #[test]
    fn test_complexity_score() {
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
        let simple_analysis = analyzer.analyze(&simple);
        assert!(simple_analysis.complexity_score <= 3);

        // Complex query with chaining and full-text
        let complex = SearchQuery::new("Observation")
            .with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::string("Smith")],
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
                values: vec![SearchValue::string("cardiac")],
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
        let complex_analysis = analyzer.analyze(&complex);
        assert!(
            complex_analysis.complexity_score >= 5,
            "Expected complexity >= 5, got {}",
            complex_analysis.complexity_score
        );
    }

    #[test]
    fn test_features_to_capabilities() {
        let features = HashSet::from([
            QueryFeature::BasicSearch,
            QueryFeature::ChainedSearch,
            QueryFeature::FullTextSearch,
        ]);

        let caps = features_to_capabilities(&features);
        assert!(caps.contains(&BackendCapability::BasicSearch));
        assert!(caps.contains(&BackendCapability::ChainedSearch));
        assert!(caps.contains(&BackendCapability::FullTextSearch));
    }
}
