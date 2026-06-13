//! HTTP API handlers for the configuration advisor.
//!
//! This module provides request/response types and handler logic for
//! the advisor HTTP API endpoints.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::composite::{BackendRole, CompositeConfig};
use crate::core::{BackendCapability, BackendKind};

use super::analysis::{
    AnalysisResult, ConfigurationAnalyzer, ConfigurationIssue, GapImpact, IssueSeverity,
    Recommendation, RecommendationPriority,
};
use super::suggestions::{
    OptimizationSuggestion, SuggestionCategory, SuggestionEngine, SuggestionPriority,
    WorkloadPattern,
};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to analyze a configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AnalyzeRequest {
    /// Configuration to analyze.
    pub config: ConfigurationInput,
}

/// Response from configuration analysis.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeResponse {
    /// Whether the analysis was successful.
    pub success: bool,

    /// Whether the configuration is valid.
    pub is_valid: bool,

    /// Issues found.
    pub issues: Vec<IssueOutput>,

    /// Recommendations.
    pub recommendations: Vec<RecommendationOutput>,

    /// Capability coverage summary.
    pub capability_coverage: CapabilityCoverageOutput,

    /// Gap analysis summary.
    pub gap_summary: GapSummaryOutput,
}

/// Request to validate a configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidateRequest {
    /// Configuration to validate.
    pub config: ConfigurationInput,
}

/// Response from configuration validation.
#[derive(Debug, Clone, Serialize)]
pub struct ValidateResponse {
    /// Whether the configuration is valid.
    pub is_valid: bool,

    /// Validation errors.
    pub errors: Vec<String>,

    /// Validation warnings.
    pub warnings: Vec<String>,
}

/// Request for optimization suggestions.
#[derive(Debug, Clone, Deserialize)]
pub struct SuggestRequest {
    /// Current configuration.
    pub config: ConfigurationInput,

    /// Workload characteristics.
    pub workload: WorkloadInput,
}

/// Response with optimization suggestions.
#[derive(Debug, Clone, Serialize)]
pub struct SuggestResponse {
    /// Suggestions for improvement.
    pub suggestions: Vec<SuggestionOutput>,

    /// Summary of current configuration.
    pub current_summary: ConfigSummaryOutput,
}

/// Request to simulate query routing.
#[derive(Debug, Clone, Deserialize)]
pub struct SimulateRequest {
    /// Configuration to simulate with.
    pub config: ConfigurationInput,

    /// Query to simulate.
    pub query: QueryInput,
}

/// Response from query simulation.
#[derive(Debug, Clone, Serialize)]
pub struct SimulateResponse {
    /// Detected query features.
    pub features: Vec<String>,

    /// Query complexity score.
    pub complexity: u8,

    /// Routing decision.
    pub routing: RoutingOutput,

    /// Estimated cost.
    pub estimated_cost: f64,
}

// ============================================================================
// Input Types (for deserialization)
// ============================================================================

/// Configuration input for API requests.
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigurationInput {
    /// Backends in the configuration.
    pub backends: Vec<BackendInput>,

    /// Sync mode (optional).
    #[serde(default)]
    pub sync_mode: Option<String>,
}

/// Backend input for API requests.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendInput {
    /// Backend identifier.
    pub id: String,

    /// Backend role.
    pub role: String,

    /// Backend kind.
    pub kind: String,

    /// Whether the backend is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Custom capabilities (optional).
    #[serde(default)]
    pub capabilities: Vec<String>,

    /// Failover target (optional).
    #[serde(default)]
    pub failover_to: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Workload input for API requests.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkloadInput {
    /// Ratio of read operations.
    #[serde(default = "default_read_ratio")]
    pub read_ratio: f64,

    /// Ratio of write operations.
    #[serde(default = "default_write_ratio")]
    pub write_ratio: f64,

    /// Ratio of full-text search.
    #[serde(default)]
    pub fulltext_search_ratio: f64,

    /// Ratio of chained search.
    #[serde(default)]
    pub chained_search_ratio: f64,

    /// Ratio of terminology search.
    #[serde(default)]
    pub terminology_search_ratio: f64,

    /// Estimated data size in GB.
    #[serde(default = "default_data_size")]
    pub estimated_data_size_gb: f64,

    /// Queries per day.
    #[serde(default = "default_queries")]
    pub queries_per_day: u64,

    /// Peak concurrent users.
    #[serde(default = "default_users")]
    pub concurrent_users: u64,
}

fn default_read_ratio() -> f64 {
    0.8
}
fn default_write_ratio() -> f64 {
    0.2
}
fn default_data_size() -> f64 {
    10.0
}
fn default_queries() -> u64 {
    1000
}
fn default_users() -> u64 {
    10
}

/// Query input for simulation.
#[derive(Debug, Clone, Deserialize)]
pub struct QueryInput {
    /// Resource type.
    pub resource_type: String,

    /// Search parameters.
    #[serde(default)]
    pub parameters: Vec<ParameterInput>,
}

/// Parameter input for query simulation.
#[derive(Debug, Clone, Deserialize)]
pub struct ParameterInput {
    /// Parameter name.
    pub name: String,

    /// Parameter value.
    pub value: String,

    /// Parameter modifier (optional).
    #[serde(default)]
    pub modifier: Option<String>,
}

// ============================================================================
// Output Types (for serialization)
// ============================================================================

/// Issue output for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct IssueOutput {
    /// Issue severity.
    pub severity: String,

    /// Issue category.
    pub category: String,

    /// Issue message.
    pub message: String,

    /// Suggestion for fixing.
    pub suggestion: Option<String>,
}

/// Recommendation output for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationOutput {
    /// Priority level.
    pub priority: String,

    /// Recommendation title.
    pub title: String,

    /// Detailed description.
    pub description: String,

    /// Expected impact.
    pub impact: String,
}

/// Capability coverage output for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCoverageOutput {
    /// Covered capabilities.
    pub covered: Vec<String>,

    /// Missing capabilities.
    pub missing: Vec<String>,

    /// Coverage by backend.
    pub by_backend: HashMap<String, Vec<String>>,
}

/// Gap summary output for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct GapSummaryOutput {
    /// Completeness score (0-100).
    pub completeness_percent: u8,

    /// High impact gaps.
    pub high_impact_gaps: Vec<String>,

    /// Medium impact gaps.
    pub medium_impact_gaps: Vec<String>,
}

/// Suggestion output for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct SuggestionOutput {
    /// Priority level.
    pub priority: String,

    /// Suggestion category.
    pub category: String,

    /// Suggestion title.
    pub title: String,

    /// Detailed description.
    pub description: String,

    /// Estimated improvement.
    pub estimated_improvement: Option<String>,

    /// Implementation guidance.
    pub implementation: Option<String>,
}

/// Configuration summary output.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummaryOutput {
    /// Number of backends.
    pub backend_count: usize,

    /// Enabled backends.
    pub enabled_backends: Vec<String>,

    /// Primary backend.
    pub primary: Option<String>,

    /// Secondary backends.
    pub secondaries: Vec<String>,
}

/// Routing output for simulation responses.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingOutput {
    /// Primary target backend.
    pub primary_target: Option<String>,

    /// Auxiliary targets.
    pub auxiliary_targets: Vec<String>,

    /// Routing error (if any).
    pub error: Option<String>,
}

// ============================================================================
// Backend Info
// ============================================================================

/// Information about a backend type.
#[derive(Debug, Clone, Serialize)]
pub struct BackendInfo {
    /// Backend kind identifier.
    pub kind: String,

    /// Human-readable name.
    pub name: String,

    /// Description.
    pub description: String,

    /// Default capabilities.
    pub default_capabilities: Vec<String>,

    /// Recommended roles.
    pub recommended_roles: Vec<String>,

    /// Strengths.
    pub strengths: Vec<String>,

    /// Weaknesses.
    pub weaknesses: Vec<String>,
}

impl BackendInfo {
    /// Returns info for all supported backend types.
    pub fn all() -> Vec<BackendInfo> {
        vec![
            BackendInfo {
                kind: "Sqlite".to_string(),
                name: "SQLite".to_string(),
                description:
                    "Embedded SQL database, ideal for development and single-node deployments"
                        .to_string(),
                default_capabilities: vec![
                    "Create",
                    "Read",
                    "Update",
                    "Delete",
                    "Search",
                    "History",
                    "VersionRead",
                    "Transaction",
                    "BulkExport",
                    "BulkSubmitIngest",
                    "BulkSubmitRestWorker",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                recommended_roles: vec!["Primary"].into_iter().map(String::from).collect(),
                strengths: vec![
                    "Zero configuration",
                    "Embedded (no network)",
                    "ACID compliant",
                    "Great for development",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                weaknesses: vec![
                    "Single writer",
                    "Limited full-text search",
                    "Not suitable for high concurrency",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            },
            BackendInfo {
                kind: "Postgres".to_string(),
                name: "PostgreSQL".to_string(),
                description: "Robust relational database, ideal for production CRUD operations"
                    .to_string(),
                default_capabilities: vec![
                    "Create",
                    "Read",
                    "Update",
                    "Delete",
                    "Search",
                    "History",
                    "VersionRead",
                    "Transaction",
                    "ConditionalCreate",
                    "ConditionalUpdate",
                    "ConditionalDelete",
                    "BulkExport",
                    "BulkSubmitIngest",
                    "BulkSubmitRestWorker",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                recommended_roles: vec!["Primary"].into_iter().map(String::from).collect(),
                strengths: vec![
                    "ACID compliant",
                    "Concurrent writes",
                    "Advanced indexing",
                    "Production-ready",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                weaknesses: vec![
                    "Requires separate server",
                    "More complex setup",
                    "Limited full-text search",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            },
            BackendInfo {
                kind: "Elasticsearch".to_string(),
                name: "Elasticsearch".to_string(),
                description: "Distributed search engine, ideal for full-text and analytics queries"
                    .to_string(),
                default_capabilities: vec!["Read", "Search", "FullTextSearch"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                recommended_roles: vec!["Search"].into_iter().map(String::from).collect(),
                strengths: vec![
                    "Excellent full-text search",
                    "Fast analytics",
                    "Horizontal scaling",
                    "Rich query DSL",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                weaknesses: vec![
                    "Eventual consistency",
                    "Not suitable as primary store",
                    "Resource intensive",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            },
            BackendInfo {
                kind: "Neo4j".to_string(),
                name: "Neo4j".to_string(),
                description: "Graph database, ideal for relationship-heavy queries".to_string(),
                default_capabilities: vec!["Read", "Search", "ChainedSearch"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                recommended_roles: vec!["Graph"].into_iter().map(String::from).collect(),
                strengths: vec![
                    "Fast graph traversal",
                    "Natural relationship modeling",
                    "Excellent for chained searches",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                weaknesses: vec![
                    "Not suitable as primary store",
                    "Learning curve for Cypher",
                    "Can be expensive",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            },
            BackendInfo {
                kind: "S3".to_string(),
                name: "Amazon S3 / Object Storage".to_string(),
                description: "Object storage, ideal for archival and large data volumes"
                    .to_string(),
                default_capabilities: vec![
                    "Create",
                    "Read",
                    "Delete",
                    "BulkExport",
                    "BulkSubmitIngest",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                recommended_roles: vec!["Archive"].into_iter().map(String::from).collect(),
                strengths: vec![
                    "Very low storage cost",
                    "Virtually unlimited capacity",
                    "High durability",
                    "Good for compliance/archival",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                weaknesses: vec![
                    "No search capability",
                    "Higher latency",
                    "No updates (only replace)",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            },
        ]
    }

    /// Returns info for a specific backend type.
    pub fn get(kind: &str) -> Option<BackendInfo> {
        Self::all()
            .into_iter()
            .find(|b| b.kind.eq_ignore_ascii_case(kind))
    }
}

// ============================================================================
// Handler Functions
// ============================================================================

/// Handles the analyze endpoint.
pub fn handle_analyze(request: AnalyzeRequest) -> Result<AnalyzeResponse, String> {
    let config = convert_config(&request.config)?;
    let analyzer = ConfigurationAnalyzer::new();
    let result = analyzer.analyze(&config);

    Ok(AnalyzeResponse {
        success: true,
        is_valid: result.is_valid,
        issues: result.issues.iter().map(convert_issue).collect(),
        recommendations: result
            .recommendations
            .iter()
            .map(convert_recommendation)
            .collect(),
        capability_coverage: convert_capability_coverage(&result),
        gap_summary: convert_gap_summary(&result),
    })
}

/// Handles the validate endpoint.
pub fn handle_validate(request: ValidateRequest) -> Result<ValidateResponse, String> {
    let config = convert_config(&request.config)?;
    let analyzer = ConfigurationAnalyzer::new();
    let result = analyzer.validate(&config);

    Ok(ValidateResponse {
        is_valid: result.is_valid,
        errors: result.errors,
        warnings: result.warnings,
    })
}

/// Handles the suggest endpoint.
pub fn handle_suggest(request: SuggestRequest) -> Result<SuggestResponse, String> {
    let config = convert_config(&request.config)?;
    let workload = convert_workload(&request.workload);
    let engine = SuggestionEngine::new();
    let suggestions = engine.suggest(&config, &workload);

    Ok(SuggestResponse {
        suggestions: suggestions.iter().map(convert_suggestion).collect(),
        current_summary: create_config_summary(&config),
    })
}

/// Handles the simulate endpoint.
pub fn handle_simulate(request: SimulateRequest) -> Result<SimulateResponse, String> {
    let config = convert_config(&request.config)?;
    let query = convert_query(&request.query);
    let analyzer = ConfigurationAnalyzer::new();
    let simulation = analyzer.simulate_query(&query, &config);

    Ok(SimulateResponse {
        features: simulation
            .query_features
            .iter()
            .map(|f| format!("{:?}", f))
            .collect(),
        complexity: simulation.complexity_score,
        routing: RoutingOutput {
            primary_target: simulation.routing_decision,
            auxiliary_targets: simulation.auxiliary_targets,
            error: simulation.routing_error,
        },
        estimated_cost: simulation.estimated_cost,
    })
}

/// Handles the backends endpoint.
pub fn handle_backends() -> Vec<BackendInfo> {
    BackendInfo::all()
}

/// Handles the backend capabilities endpoint.
pub fn handle_backend_capabilities(kind: &str) -> Result<BackendInfo, String> {
    BackendInfo::get(kind).ok_or_else(|| format!("Unknown backend kind: {}", kind))
}

// ============================================================================
// Conversion Functions
// ============================================================================

fn convert_config(input: &ConfigurationInput) -> Result<CompositeConfig, String> {
    use crate::composite::{BackendEntry, CompositeConfigBuilder, SyncMode};

    let mut builder = CompositeConfigBuilder::new();

    for backend in &input.backends {
        let kind = parse_backend_kind(&backend.kind)?;
        let role = parse_backend_role(&backend.role)?;

        let mut entry = BackendEntry::new(&backend.id, role, kind);
        entry.enabled = backend.enabled;
        entry.failover_to = backend.failover_to.clone();

        if !backend.capabilities.is_empty() {
            entry.capabilities = backend
                .capabilities
                .iter()
                .filter_map(|c| parse_capability(c).ok())
                .collect();
        }

        builder = builder.with_backend(entry);
    }

    // Set sync mode
    if let Some(ref mode) = input.sync_mode {
        let sync_mode = match mode.to_lowercase().as_str() {
            "synchronous" | "sync" => SyncMode::Synchronous,
            "asynchronous" | "async" => SyncMode::Asynchronous,
            "hybrid" => SyncMode::Hybrid {
                sync_for_search: true,
            },
            _ => SyncMode::Asynchronous,
        };
        builder = builder.sync_mode(sync_mode);
    }

    builder
        .build()
        .map_err(|e| format!("Invalid configuration: {:?}", e))
}

fn convert_workload(input: &WorkloadInput) -> WorkloadPattern {
    WorkloadPattern {
        read_ratio: input.read_ratio,
        write_ratio: input.write_ratio,
        fulltext_search_ratio: input.fulltext_search_ratio,
        chained_search_ratio: input.chained_search_ratio,
        terminology_search_ratio: input.terminology_search_ratio,
        estimated_data_size_gb: input.estimated_data_size_gb,
        queries_per_day: input.queries_per_day,
        concurrent_users: input.concurrent_users,
        ..Default::default()
    }
}

fn convert_query(input: &QueryInput) -> crate::types::SearchQuery {
    use crate::types::{SearchModifier, SearchParameter, SearchQuery, SearchValue};

    let mut query = SearchQuery::new(&input.resource_type);

    for param in &input.parameters {
        let modifier = param
            .modifier
            .as_ref()
            .and_then(|m| match m.to_lowercase().as_str() {
                "exact" => Some(SearchModifier::Exact),
                "contains" => Some(SearchModifier::Contains),
                "text" => Some(SearchModifier::Text),
                "not" => Some(SearchModifier::Not),
                "missing" => Some(SearchModifier::Missing),
                "above" => Some(SearchModifier::Above),
                "below" => Some(SearchModifier::Below),
                "in" => Some(SearchModifier::In),
                "not-in" | "notin" => Some(SearchModifier::NotIn),
                "identifier" => Some(SearchModifier::Identifier),
                "oftype" | "of-type" => Some(SearchModifier::OfType),
                "iterate" => Some(SearchModifier::Iterate),
                _ => None,
            });

        let search_param = SearchParameter {
            name: param.name.clone(),
            values: vec![SearchValue::eq(&param.value)],
            modifier,
            ..Default::default()
        };
        query.parameters.push(search_param);
    }

    query
}

fn convert_issue(issue: &ConfigurationIssue) -> IssueOutput {
    IssueOutput {
        severity: match issue.severity {
            IssueSeverity::Error => "error".to_string(),
            IssueSeverity::Warning => "warning".to_string(),
            IssueSeverity::Info => "info".to_string(),
        },
        category: format!("{:?}", issue.category),
        message: issue.message.clone(),
        suggestion: issue.suggestion.clone(),
    }
}

fn convert_recommendation(rec: &Recommendation) -> RecommendationOutput {
    RecommendationOutput {
        priority: match rec.priority {
            RecommendationPriority::Critical => "critical".to_string(),
            RecommendationPriority::High => "high".to_string(),
            RecommendationPriority::Medium => "medium".to_string(),
            RecommendationPriority::Low => "low".to_string(),
        },
        title: rec.title.clone(),
        description: rec.description.clone(),
        impact: rec.impact.clone(),
    }
}

fn convert_suggestion(sug: &OptimizationSuggestion) -> SuggestionOutput {
    SuggestionOutput {
        priority: match sug.priority {
            SuggestionPriority::Critical => "critical".to_string(),
            SuggestionPriority::High => "high".to_string(),
            SuggestionPriority::Medium => "medium".to_string(),
            SuggestionPriority::Low => "low".to_string(),
        },
        category: match sug.category {
            SuggestionCategory::Performance => "performance".to_string(),
            SuggestionCategory::Scalability => "scalability".to_string(),
            SuggestionCategory::Cost => "cost".to_string(),
            SuggestionCategory::Feature => "feature".to_string(),
            SuggestionCategory::Reliability => "reliability".to_string(),
        },
        title: sug.title.clone(),
        description: sug.description.clone(),
        estimated_improvement: sug.estimated_improvement.clone(),
        implementation: sug.implementation.clone(),
    }
}

fn convert_capability_coverage(result: &AnalysisResult) -> CapabilityCoverageOutput {
    CapabilityCoverageOutput {
        covered: result
            .capability_coverage
            .covered
            .iter()
            .map(|c| format!("{:?}", c))
            .collect(),
        missing: result
            .capability_coverage
            .missing
            .iter()
            .map(|c| format!("{:?}", c))
            .collect(),
        by_backend: result
            .capability_coverage
            .capability_backends
            .iter()
            .map(|(cap, backends)| (format!("{:?}", cap), backends.clone()))
            .collect(),
    }
}

fn convert_gap_summary(result: &AnalysisResult) -> GapSummaryOutput {
    let high_impact: Vec<_> = result
        .gap_analysis
        .feature_gaps
        .iter()
        .filter(|g| g.impact == GapImpact::High)
        .map(|g| format!("{:?}", g.capability))
        .collect();

    let medium_impact: Vec<_> = result
        .gap_analysis
        .feature_gaps
        .iter()
        .filter(|g| g.impact == GapImpact::Medium)
        .map(|g| format!("{:?}", g.capability))
        .collect();

    GapSummaryOutput {
        completeness_percent: (result.gap_analysis.completeness_score * 100.0) as u8,
        high_impact_gaps: high_impact,
        medium_impact_gaps: medium_impact,
    }
}

fn create_config_summary(config: &CompositeConfig) -> ConfigSummaryOutput {
    let primary = config
        .backends
        .iter()
        .find(|b| b.role == BackendRole::Primary)
        .map(|b| b.id.clone());

    let secondaries: Vec<_> = config
        .backends
        .iter()
        .filter(|b| b.role != BackendRole::Primary && b.enabled)
        .map(|b| b.id.clone())
        .collect();

    let enabled: Vec<_> = config
        .backends
        .iter()
        .filter(|b| b.enabled)
        .map(|b| b.id.clone())
        .collect();

    ConfigSummaryOutput {
        backend_count: config.backends.len(),
        enabled_backends: enabled,
        primary,
        secondaries,
    }
}

fn parse_backend_kind(s: &str) -> Result<BackendKind, String> {
    match s.to_lowercase().as_str() {
        "sqlite" => Ok(BackendKind::Sqlite),
        "postgres" | "postgresql" => Ok(BackendKind::Postgres),
        "elasticsearch" | "es" => Ok(BackendKind::Elasticsearch),
        "neo4j" => Ok(BackendKind::Neo4j),
        "s3" | "objectstore" => Ok(BackendKind::S3),
        "mongodb" | "mongo" => Ok(BackendKind::MongoDB),
        "cassandra" => Ok(BackendKind::Cassandra),
        _ => Err(format!("Unknown backend kind: {}", s)),
    }
}

fn parse_backend_role(s: &str) -> Result<BackendRole, String> {
    match s.to_lowercase().as_str() {
        "primary" => Ok(BackendRole::Primary),
        "search" => Ok(BackendRole::Search),
        "graph" => Ok(BackendRole::Graph),
        "terminology" => Ok(BackendRole::Terminology),
        "archive" => Ok(BackendRole::Archive),
        _ => Err(format!("Unknown backend role: {}", s)),
    }
}

fn parse_capability(s: &str) -> Result<BackendCapability, String> {
    let normalized = s.to_lowercase().replace(['-', '_', ' '], "");
    match normalized.as_str() {
        "crud" | "create" | "read" | "update" | "delete" => Ok(BackendCapability::Crud),
        "basicsearch" | "search" => Ok(BackendCapability::BasicSearch),
        "versioning" | "versionread" => Ok(BackendCapability::Versioning),
        "instancehistory" | "history" => Ok(BackendCapability::InstanceHistory),
        "typehistory" => Ok(BackendCapability::TypeHistory),
        "systemhistory" => Ok(BackendCapability::SystemHistory),
        "transactions" | "transaction" => Ok(BackendCapability::Transactions),
        "chainedsearch" => Ok(BackendCapability::ChainedSearch),
        "reversechaining" => Ok(BackendCapability::ReverseChaining),
        "include" => Ok(BackendCapability::Include),
        "revinclude" => Ok(BackendCapability::Revinclude),
        "fulltextsearch" => Ok(BackendCapability::FullTextSearch),
        "terminologysearch" | "terminologyexpansion" => Ok(BackendCapability::TerminologySearch),
        "datesearch" => Ok(BackendCapability::DateSearch),
        "quantitysearch" => Ok(BackendCapability::QuantitySearch),
        "referencesearch" => Ok(BackendCapability::ReferenceSearch),
        "cursorpagination" => Ok(BackendCapability::CursorPagination),
        "offsetpagination" => Ok(BackendCapability::OffsetPagination),
        "sorting" => Ok(BackendCapability::Sorting),
        "bulkexport" => Ok(BackendCapability::BulkExport),
        "bulksubmitingest" | "bulkimport" => Ok(BackendCapability::BulkSubmitIngest),
        "bulksubmitrestworker" => Ok(BackendCapability::BulkSubmitRestWorker),
        "optimisticlocking" => Ok(BackendCapability::OptimisticLocking),
        _ => Err(format!("Unknown capability: {}", s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_backends() {
        let backends = handle_backends();
        assert!(!backends.is_empty());
        assert!(backends.iter().any(|b| b.kind == "Sqlite"));
    }

    #[test]
    fn test_handle_backend_capabilities() {
        let result = handle_backend_capabilities("Sqlite");
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.kind, "Sqlite");
    }

    #[test]
    fn test_parse_bulk_submit_capabilities() {
        assert_eq!(
            parse_capability("bulk-submit-ingest").unwrap(),
            BackendCapability::BulkSubmitIngest
        );
        assert_eq!(
            parse_capability("BulkSubmitRestWorker").unwrap(),
            BackendCapability::BulkSubmitRestWorker
        );
        assert_eq!(
            parse_capability("bulkimport").unwrap(),
            BackendCapability::BulkSubmitIngest
        );
    }

    #[test]
    fn test_handle_validate() {
        let request = ValidateRequest {
            config: ConfigurationInput {
                backends: vec![BackendInput {
                    id: "primary".to_string(),
                    role: "Primary".to_string(),
                    kind: "Sqlite".to_string(),
                    enabled: true,
                    capabilities: vec![],
                    failover_to: None,
                }],
                sync_mode: None,
            },
        };

        let result = handle_validate(request);
        assert!(result.is_ok());
        assert!(result.unwrap().is_valid);
    }

    #[test]
    fn test_handle_analyze() {
        let request = AnalyzeRequest {
            config: ConfigurationInput {
                backends: vec![BackendInput {
                    id: "primary".to_string(),
                    role: "Primary".to_string(),
                    kind: "Sqlite".to_string(),
                    enabled: true,
                    capabilities: vec![],
                    failover_to: None,
                }],
                sync_mode: None,
            },
        };

        let result = handle_analyze(request);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.success);
    }

    #[test]
    fn test_handle_suggest() {
        let request = SuggestRequest {
            config: ConfigurationInput {
                backends: vec![BackendInput {
                    id: "primary".to_string(),
                    role: "Primary".to_string(),
                    kind: "Sqlite".to_string(),
                    enabled: true,
                    capabilities: vec![],
                    failover_to: None,
                }],
                sync_mode: None,
            },
            workload: WorkloadInput {
                read_ratio: 0.8,
                write_ratio: 0.2,
                fulltext_search_ratio: 0.1,
                chained_search_ratio: 0.05,
                terminology_search_ratio: 0.02,
                estimated_data_size_gb: 10.0,
                queries_per_day: 1000,
                concurrent_users: 10,
            },
        };

        let result = handle_suggest(request);
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_simulate() {
        let request = SimulateRequest {
            config: ConfigurationInput {
                backends: vec![BackendInput {
                    id: "primary".to_string(),
                    role: "Primary".to_string(),
                    kind: "Sqlite".to_string(),
                    enabled: true,
                    capabilities: vec![],
                    failover_to: None,
                }],
                sync_mode: None,
            },
            query: QueryInput {
                resource_type: "Patient".to_string(),
                parameters: vec![ParameterInput {
                    name: "name".to_string(),
                    value: "Smith".to_string(),
                    modifier: None,
                }],
            },
        };

        let result = handle_simulate(request);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.estimated_cost > 0.0);
    }

    #[test]
    fn test_parse_backend_kind() {
        assert!(parse_backend_kind("Sqlite").is_ok());
        assert!(parse_backend_kind("SQLITE").is_ok());
        assert!(parse_backend_kind("postgres").is_ok());
        assert!(parse_backend_kind("unknown").is_err());
    }

    #[test]
    fn test_parse_backend_role() {
        assert!(parse_backend_role("Primary").is_ok());
        assert!(parse_backend_role("search").is_ok());
        assert!(parse_backend_role("unknown").is_err());
    }
}
