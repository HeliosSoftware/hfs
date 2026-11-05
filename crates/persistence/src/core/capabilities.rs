//! Storage capabilities and capability statement generation.
//!
//! This module defines traits for runtime capability discovery and
//! generation of FHIR CapabilityStatement fragments.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{
    ChainingCapability, IncludeCapability, PaginationCapability, ResultModeCapability,
    SearchParamFullCapability, SearchParamType, SearchQuery, SpecialSearchParam,
};

/// Supported FHIR interactions for a resource type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interaction {
    /// read - Read the current state of the resource.
    Read,
    /// vread - Read a specific version.
    Vread,
    /// update - Update an existing resource.
    Update,
    /// patch - Partial update of a resource.
    Patch,
    /// delete - Delete a resource.
    Delete,
    /// history-instance - Retrieve history for a resource instance.
    HistoryInstance,
    /// history-type - Retrieve history for a resource type.
    HistoryType,
    /// create - Create a new resource.
    Create,
    /// search-type - Search for resources of a type.
    SearchType,
}

impl std::fmt::Display for Interaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Interaction::Read => write!(f, "read"),
            Interaction::Vread => write!(f, "vread"),
            Interaction::Update => write!(f, "update"),
            Interaction::Patch => write!(f, "patch"),
            Interaction::Delete => write!(f, "delete"),
            Interaction::HistoryInstance => write!(f, "history-instance"),
            Interaction::HistoryType => write!(f, "history-type"),
            Interaction::Create => write!(f, "create"),
            Interaction::SearchType => write!(f, "search-type"),
        }
    }
}

/// Supported system-level interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemInteraction {
    /// transaction - Process a transaction bundle.
    Transaction,
    /// batch - Process a batch bundle.
    Batch,
    /// history-system - Retrieve history for all resources.
    HistorySystem,
    /// search-system - Search across all resource types.
    SearchSystem,
}

impl std::fmt::Display for SystemInteraction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemInteraction::Transaction => write!(f, "transaction"),
            SystemInteraction::Batch => write!(f, "batch"),
            SystemInteraction::HistorySystem => write!(f, "history-system"),
            SystemInteraction::SearchSystem => write!(f, "search-system"),
        }
    }
}

/// Information about a supported search parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParamCapability {
    /// The parameter name.
    pub name: String,
    /// The parameter type.
    pub param_type: SearchParamType,
    /// Supported modifiers for this parameter.
    pub modifiers: Vec<String>,
    /// Whether chaining is supported for reference parameters.
    pub supports_chaining: bool,
    /// Documentation for this parameter.
    pub documentation: Option<String>,
}

impl SearchParamCapability {
    /// Creates a new search parameter capability.
    pub fn new(name: impl Into<String>, param_type: SearchParamType) -> Self {
        Self {
            name: name.into(),
            param_type,
            modifiers: Vec::new(),
            supports_chaining: false,
            documentation: None,
        }
    }

    /// Adds supported modifiers.
    pub fn with_modifiers(mut self, modifiers: Vec<&str>) -> Self {
        self.modifiers = modifiers.into_iter().map(String::from).collect();
        self
    }

    /// Enables chaining support.
    pub fn with_chaining(mut self) -> Self {
        self.supports_chaining = true;
        self
    }

    /// Adds documentation.
    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }
}

/// Capabilities for a specific resource type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceCapabilities {
    /// The resource type.
    pub resource_type: String,
    /// Supported interactions.
    pub interactions: HashSet<Interaction>,
    /// Supported search parameters.
    pub search_params: Vec<SearchParamCapability>,
    /// Whether _include is supported.
    pub supports_include: bool,
    /// Whether _revinclude is supported.
    pub supports_revinclude: bool,
    /// Supported _include targets.
    pub include_targets: Vec<String>,
    /// Supported _revinclude targets.
    pub revinclude_targets: Vec<String>,
    /// Whether conditional create is supported.
    pub conditional_create: bool,
    /// Whether conditional update is supported.
    pub conditional_update: bool,
    /// Whether conditional delete is supported.
    pub conditional_delete: bool,
    /// Whether conditional patch is supported. Not set by
    /// [`with_conditional_ops`](Self::with_conditional_ops) — MongoDB and S3
    /// lack it — and not rendered by
    /// [`StorageCapabilities::to_capability_rest`], which is version-unaware:
    /// `rest.resource.conditionalPatch` exists from FHIR R5 on only.
    #[serde(default)]
    pub conditional_patch: bool,
    /// Additional documentation.
    pub documentation: Option<String>,
}

impl ResourceCapabilities {
    /// Creates capabilities for a resource type.
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            ..Default::default()
        }
    }

    /// Adds supported interactions.
    pub fn with_interactions(
        mut self,
        interactions: impl IntoIterator<Item = Interaction>,
    ) -> Self {
        self.interactions.extend(interactions);
        self
    }

    /// Adds all CRUD interactions.
    pub fn with_crud(mut self) -> Self {
        self.interactions.insert(Interaction::Read);
        self.interactions.insert(Interaction::Create);
        self.interactions.insert(Interaction::Update);
        self.interactions.insert(Interaction::Delete);
        self
    }

    /// Adds version support.
    pub fn with_versioning(mut self) -> Self {
        self.interactions.insert(Interaction::Vread);
        self.interactions.insert(Interaction::HistoryInstance);
        self
    }

    /// Adds search support.
    pub fn with_search(mut self, params: Vec<SearchParamCapability>) -> Self {
        self.interactions.insert(Interaction::SearchType);
        self.search_params = params;
        self
    }

    /// Enables _include support.
    pub fn with_include(mut self, targets: Vec<&str>) -> Self {
        self.supports_include = true;
        self.include_targets = targets.into_iter().map(String::from).collect();
        self
    }

    /// Enables _revinclude support.
    pub fn with_revinclude(mut self, targets: Vec<&str>) -> Self {
        self.supports_revinclude = true;
        self.revinclude_targets = targets.into_iter().map(String::from).collect();
        self
    }

    /// Enables conditional operations.
    pub fn with_conditional_ops(mut self) -> Self {
        self.conditional_create = true;
        self.conditional_update = true;
        self.conditional_delete = true;
        self
    }
}

/// Overall storage capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageCapabilities {
    /// Capabilities by resource type.
    pub resources: HashMap<String, ResourceCapabilities>,
    /// Supported system-level interactions.
    pub system_interactions: HashSet<SystemInteraction>,
    /// Whether system-level history is supported.
    pub supports_system_history: bool,
    /// Whether system-level search is supported.
    pub supports_system_search: bool,
    /// Supported sort parameters.
    pub supported_sorts: Vec<String>,
    /// Whether total counts are supported.
    pub supports_total: bool,
    /// Maximum page size.
    pub max_page_size: Option<u32>,
    /// Default page size.
    pub default_page_size: u32,
    /// Backend name.
    pub backend_name: String,
    /// Backend version.
    pub backend_version: Option<String>,
}

impl StorageCapabilities {
    /// Creates new storage capabilities.
    pub fn new(backend_name: impl Into<String>) -> Self {
        Self {
            backend_name: backend_name.into(),
            default_page_size: 20,
            ..Default::default()
        }
    }

    /// Adds resource capabilities.
    pub fn with_resource(mut self, caps: ResourceCapabilities) -> Self {
        self.resources.insert(caps.resource_type.clone(), caps);
        self
    }

    /// Adds system interactions.
    pub fn with_system_interactions(
        mut self,
        interactions: impl IntoIterator<Item = SystemInteraction>,
    ) -> Self {
        self.system_interactions.extend(interactions);
        self
    }

    /// Enables system history.
    pub fn with_system_history(mut self) -> Self {
        self.supports_system_history = true;
        self.system_interactions
            .insert(SystemInteraction::HistorySystem);
        self
    }

    /// Enables system search.
    pub fn with_system_search(mut self) -> Self {
        self.supports_system_search = true;
        self.system_interactions
            .insert(SystemInteraction::SearchSystem);
        self
    }

    /// Enables transaction support.
    pub fn with_transactions(mut self) -> Self {
        self.system_interactions
            .insert(SystemInteraction::Transaction);
        self.system_interactions.insert(SystemInteraction::Batch);
        self
    }

    /// Sets pagination limits.
    pub fn with_pagination(mut self, default: u32, max: Option<u32>) -> Self {
        self.default_page_size = default;
        self.max_page_size = max;
        self
    }

    /// Adds supported sort parameters.
    pub fn with_sorts(mut self, sorts: Vec<&str>) -> Self {
        self.supported_sorts = sorts.into_iter().map(String::from).collect();
        self
    }

    /// Enables total count support.
    pub fn with_total_support(mut self) -> Self {
        self.supports_total = true;
        self
    }

    /// Generates a FHIR CapabilityStatement rest resource for this storage.
    pub fn to_capability_rest(&self) -> Value {
        let mut resources = Vec::new();

        for caps in self.resources.values() {
            let mut resource = serde_json::json!({
                "type": caps.resource_type,
                "interaction": caps.interactions.iter().map(|i| {
                    serde_json::json!({"code": i.to_string()})
                }).collect::<Vec<_>>(),
            });

            if !caps.search_params.is_empty() {
                resource["searchParam"] = serde_json::json!(
                    caps.search_params
                        .iter()
                        .map(|sp| {
                            serde_json::json!({
                                "name": sp.name,
                                "type": sp.param_type.to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                );
            }

            if caps.conditional_create {
                resource["conditionalCreate"] = serde_json::json!(true);
            }
            if caps.conditional_update {
                resource["conditionalUpdate"] = serde_json::json!(true);
            }
            if caps.conditional_delete {
                resource["conditionalDelete"] = serde_json::json!("single");
            }

            resources.push(resource);
        }

        let mut rest = serde_json::json!({
            "mode": "server",
            "resource": resources,
        });

        if !self.system_interactions.is_empty() {
            rest["interaction"] = serde_json::json!(
                self.system_interactions
                    .iter()
                    .map(|i| { serde_json::json!({"code": i.to_string()}) })
                    .collect::<Vec<_>>()
            );
        }

        rest
    }
}

/// Trait for storage backends to declare their capabilities.
pub trait CapabilityProvider {
    /// Returns the capabilities of this storage backend.
    fn capabilities(&self) -> StorageCapabilities;

    /// Checks if a specific resource type interaction is supported.
    fn supports_interaction(&self, resource_type: &str, interaction: Interaction) -> bool {
        self.capabilities()
            .resources
            .get(resource_type)
            .map(|r| r.interactions.contains(&interaction))
            .unwrap_or(false)
    }

    /// Checks if a system interaction is supported.
    fn supports_system_interaction(&self, interaction: SystemInteraction) -> bool {
        self.capabilities()
            .system_interactions
            .contains(&interaction)
    }

    /// Gets the capabilities for a specific resource type.
    fn resource_capabilities(&self, resource_type: &str) -> Option<ResourceCapabilities> {
        self.capabilities().resources.get(resource_type).cloned()
    }
}

// ============================================================================
// Enhanced Search Capabilities
// ============================================================================

/// Comprehensive search capabilities for a resource type.
///
/// This provides detailed information about what search features are supported
/// for a specific resource type, including all parameters, modifiers, and
/// special capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceSearchCapabilities {
    /// The resource type these capabilities apply to.
    pub resource_type: String,

    /// Full capability information for each search parameter.
    pub search_params: Vec<SearchParamFullCapability>,

    /// Supported special parameters (_id, _lastUpdated, etc.).
    pub special_params: HashSet<SpecialSearchParam>,

    /// Include/revinclude capabilities.
    pub include_capabilities: HashSet<IncludeCapability>,

    /// Chaining capabilities.
    pub chaining_capabilities: HashSet<ChainingCapability>,

    /// Pagination capabilities.
    pub pagination_capabilities: HashSet<PaginationCapability>,

    /// Result mode capabilities (_summary, _elements, _total).
    pub result_mode_capabilities: HashSet<ResultModeCapability>,
}

impl ResourceSearchCapabilities {
    /// Creates new search capabilities for a resource type.
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            ..Default::default()
        }
    }

    /// Adds a search parameter capability.
    pub fn with_param(mut self, param: SearchParamFullCapability) -> Self {
        self.search_params.push(param);
        self
    }

    /// Adds multiple search parameter capabilities.
    pub fn with_param_list(mut self, params: Vec<SearchParamFullCapability>) -> Self {
        self.search_params.extend(params);
        self
    }

    /// Adds special parameter support.
    pub fn with_special_params<I>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = SpecialSearchParam>,
    {
        self.special_params.extend(params);
        self
    }

    /// Adds include capabilities.
    pub fn with_include_capabilities<I>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = IncludeCapability>,
    {
        self.include_capabilities.extend(caps);
        self
    }

    /// Adds chaining capabilities.
    pub fn with_chaining_capabilities<I>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = ChainingCapability>,
    {
        self.chaining_capabilities.extend(caps);
        self
    }

    /// Adds pagination capabilities.
    pub fn with_pagination_capabilities<I>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = PaginationCapability>,
    {
        self.pagination_capabilities.extend(caps);
        self
    }

    /// Adds result mode capabilities.
    pub fn with_result_mode_capabilities<I>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = ResultModeCapability>,
    {
        self.result_mode_capabilities.extend(caps);
        self
    }

    /// Returns the capability for a specific parameter by name.
    pub fn get_param(&self, name: &str) -> Option<&SearchParamFullCapability> {
        self.search_params.iter().find(|p| p.name == name)
    }

    /// Returns whether a special parameter is supported.
    pub fn supports_special(&self, param: SpecialSearchParam) -> bool {
        self.special_params.contains(&param)
    }

    /// Returns whether a specific include capability is supported.
    pub fn supports_include(&self, cap: IncludeCapability) -> bool {
        self.include_capabilities.contains(&cap)
    }

    /// Returns whether chaining is supported.
    pub fn supports_chaining(&self) -> bool {
        self.chaining_capabilities
            .contains(&ChainingCapability::ForwardChain)
    }

    /// Returns whether reverse chaining (_has) is supported.
    pub fn supports_reverse_chaining(&self) -> bool {
        self.chaining_capabilities
            .contains(&ChainingCapability::ReverseChain)
    }
}

/// Global search capabilities across all resource types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalSearchCapabilities {
    /// Common special parameters supported for all types.
    pub common_special_params: HashSet<SpecialSearchParam>,

    /// Common include capabilities for all types.
    pub common_include_capabilities: HashSet<IncludeCapability>,

    /// Common pagination capabilities.
    pub common_pagination_capabilities: HashSet<PaginationCapability>,

    /// Common result mode capabilities.
    pub common_result_mode_capabilities: HashSet<ResultModeCapability>,

    /// Maximum chain depth supported.
    pub max_chain_depth: Option<u8>,

    /// Whether system-level search is supported.
    pub supports_system_search: bool,

    /// Supported common sort parameters (_id, _lastUpdated).
    pub common_sort_params: Vec<String>,
}

impl GlobalSearchCapabilities {
    /// Creates new global search capabilities.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds common special parameters.
    pub fn with_special_params<I>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = SpecialSearchParam>,
    {
        self.common_special_params.extend(params);
        self
    }

    /// Adds common pagination capabilities.
    pub fn with_pagination<I>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = PaginationCapability>,
    {
        self.common_pagination_capabilities.extend(caps);
        self
    }

    /// Sets max chain depth.
    pub fn with_max_chain_depth(mut self, depth: u8) -> Self {
        self.max_chain_depth = Some(depth);
        self
    }

    /// Enables system search.
    pub fn with_system_search(mut self) -> Self {
        self.supports_system_search = true;
        self
    }
}

/// Error returned when a search query uses unsupported features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedSearchFeature {
    /// Type of unsupported feature.
    pub feature_type: UnsupportedFeatureType,
    /// Description of the unsupported feature.
    pub description: String,
    /// The parameter or feature that caused the error.
    pub parameter: Option<String>,
    /// Suggested alternative, if any.
    pub suggestion: Option<String>,
}

/// Types of unsupported search features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnsupportedFeatureType {
    /// Parameter not defined for this resource type.
    UnknownParameter,
    /// Modifier not supported for this parameter.
    UnsupportedModifier,
    /// Prefix not supported for this parameter type.
    UnsupportedPrefix,
    /// Chaining not supported.
    UnsupportedChaining,
    /// Reverse chaining (_has) not supported.
    UnsupportedReverseChaining,
    /// Include/revinclude not supported.
    UnsupportedInclude,
    /// Composite parameter not supported.
    UnsupportedComposite,
    /// Special parameter not supported.
    UnsupportedSpecialParameter,
    /// Result mode not supported.
    UnsupportedResultMode,
    /// Pagination mode not supported.
    UnsupportedPagination,
}

impl UnsupportedSearchFeature {
    /// Creates a new unsupported feature error.
    pub fn new(feature_type: UnsupportedFeatureType, description: impl Into<String>) -> Self {
        Self {
            feature_type,
            description: description.into(),
            parameter: None,
            suggestion: None,
        }
    }

    /// Sets the parameter that caused the error.
    pub fn with_parameter(mut self, param: impl Into<String>) -> Self {
        self.parameter = Some(param.into());
        self
    }

    /// Sets a suggestion for the user.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Creates an unknown parameter error.
    pub fn unknown_parameter(resource_type: &str, param: &str) -> Self {
        Self::new(
            UnsupportedFeatureType::UnknownParameter,
            format!(
                "Parameter '{}' is not defined for resource type '{}'",
                param, resource_type
            ),
        )
        .with_parameter(param)
    }

    /// Creates an unsupported modifier error.
    pub fn unsupported_modifier(param: &str, modifier: &str) -> Self {
        Self::new(
            UnsupportedFeatureType::UnsupportedModifier,
            format!(
                "Modifier '{}' is not supported for parameter '{}'",
                modifier, param
            ),
        )
        .with_parameter(format!("{}:{}", param, modifier))
    }

    /// Creates an unsupported prefix error.
    pub fn unsupported_prefix(param: &str, prefix: &str) -> Self {
        Self::new(
            UnsupportedFeatureType::UnsupportedPrefix,
            format!(
                "Prefix '{}' is not supported for parameter '{}'",
                prefix, param
            ),
        )
        .with_parameter(param)
    }
}

impl std::fmt::Display for UnsupportedSearchFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)?;
        if let Some(ref suggestion) = self.suggestion {
            write!(f, " ({})", suggestion)?;
        }
        Ok(())
    }
}

impl std::error::Error for UnsupportedSearchFeature {}

/// Trait for providing detailed search capabilities.
///
/// This trait extends basic capability reporting with detailed search
/// capability information and query validation.
pub trait SearchCapabilityProvider: Send + Sync {
    /// Returns detailed search capabilities for a resource type.
    ///
    /// Returns `None` if the resource type is not supported.
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities>;

    /// Returns global search capabilities that apply to all resource types.
    fn global_search_capabilities(&self) -> GlobalSearchCapabilities;

    /// Validates a search query against the backend's capabilities.
    ///
    /// Returns `Ok(())` if the query is fully supported, or an error
    /// describing the first unsupported feature encountered.
    fn validate_search_query(&self, query: &SearchQuery) -> Result<(), UnsupportedSearchFeature> {
        let resource_type = &query.resource_type;
        let caps = self
            .resource_search_capabilities(resource_type)
            .ok_or_else(|| {
                UnsupportedSearchFeature::new(
                    UnsupportedFeatureType::UnknownParameter,
                    format!("Resource type '{}' is not supported", resource_type),
                )
            })?;

        // Validate each parameter
        for param in &query.parameters {
            // Check if parameter is defined
            let param_cap = caps.get_param(&param.name).ok_or_else(|| {
                UnsupportedSearchFeature::unknown_parameter(resource_type, &param.name)
            })?;

            // Check modifier if present
            if let Some(ref modifier) = param.modifier {
                let modifier_str = modifier.to_string();
                if !param_cap.supports_modifier(&modifier_str) {
                    return Err(UnsupportedSearchFeature::unsupported_modifier(
                        &param.name,
                        &modifier_str,
                    ));
                }
            }

            // Check prefixes
            for value in &param.values {
                let prefix_str = value.prefix.to_string();
                if !param_cap.supports_prefix(&prefix_str) {
                    return Err(UnsupportedSearchFeature::unsupported_prefix(
                        &param.name,
                        &prefix_str,
                    ));
                }
            }

            // Check chaining
            if !param.chain.is_empty() && !caps.supports_chaining() {
                return Err(UnsupportedSearchFeature::new(
                    UnsupportedFeatureType::UnsupportedChaining,
                    "Chained search parameters are not supported",
                ));
            }
        }

        // Check reverse chaining
        if !query.reverse_chains.is_empty() && !caps.supports_reverse_chaining() {
            return Err(UnsupportedSearchFeature::new(
                UnsupportedFeatureType::UnsupportedReverseChaining,
                "_has (reverse chaining) is not supported",
            ));
        }

        // Check includes
        for include in &query.includes {
            let include_cap = if include.include_type == crate::types::IncludeType::Include {
                if include.iterate {
                    IncludeCapability::IncludeIterate
                } else {
                    IncludeCapability::Include
                }
            } else if include.iterate {
                IncludeCapability::RevincludeIterate
            } else {
                IncludeCapability::Revinclude
            };

            if !caps.supports_include(include_cap) {
                return Err(UnsupportedSearchFeature::new(
                    UnsupportedFeatureType::UnsupportedInclude,
                    format!("{:?} is not supported", include_cap),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interaction_display() {
        assert_eq!(Interaction::Read.to_string(), "read");
        assert_eq!(Interaction::HistoryInstance.to_string(), "history-instance");
    }

    #[test]
    fn test_system_interaction_display() {
        assert_eq!(SystemInteraction::Transaction.to_string(), "transaction");
        assert_eq!(
            SystemInteraction::HistorySystem.to_string(),
            "history-system"
        );
    }

    #[test]
    fn test_search_param_capability() {
        let cap = SearchParamCapability::new("name", SearchParamType::String)
            .with_modifiers(vec!["exact", "contains"])
            .with_documentation("Search by patient name");

        assert_eq!(cap.name, "name");
        assert_eq!(cap.modifiers.len(), 2);
        assert!(cap.documentation.is_some());
    }

    #[test]
    fn test_resource_capabilities() {
        let caps = ResourceCapabilities::new("Patient")
            .with_crud()
            .with_versioning()
            .with_conditional_ops();

        assert!(caps.interactions.contains(&Interaction::Read));
        assert!(caps.interactions.contains(&Interaction::Create));
        assert!(caps.interactions.contains(&Interaction::Vread));
        assert!(caps.conditional_create);
    }

    #[test]
    fn test_storage_capabilities() {
        let patient_caps = ResourceCapabilities::new("Patient")
            .with_crud()
            .with_search(vec![
                SearchParamCapability::new("name", SearchParamType::String),
                SearchParamCapability::new("identifier", SearchParamType::Token),
            ]);

        let caps = StorageCapabilities::new("sqlite")
            .with_resource(patient_caps)
            .with_transactions()
            .with_pagination(20, Some(100));

        assert!(caps.resources.contains_key("Patient"));
        assert!(
            caps.system_interactions
                .contains(&SystemInteraction::Transaction)
        );
        assert_eq!(caps.default_page_size, 20);
        assert_eq!(caps.max_page_size, Some(100));
    }

    #[test]
    fn test_to_capability_rest() {
        let caps = StorageCapabilities::new("test")
            .with_resource(ResourceCapabilities::new("Patient").with_crud())
            .with_transactions();

        let rest = caps.to_capability_rest();
        assert_eq!(rest["mode"], "server");
        assert!(rest["resource"].is_array());
        assert!(rest["interaction"].is_array());
    }

    // =========================================================================
    // Enhanced Search Capabilities Tests
    // =========================================================================

    #[test]
    fn test_resource_search_capabilities() {
        let caps = ResourceSearchCapabilities::new("Patient")
            .with_param(SearchParamFullCapability::new(
                "name",
                SearchParamType::String,
            ))
            .with_special_params(vec![
                SpecialSearchParam::Id,
                SpecialSearchParam::LastUpdated,
            ])
            .with_include_capabilities(vec![IncludeCapability::Include]);

        assert_eq!(caps.resource_type, "Patient");
        assert!(caps.get_param("name").is_some());
        assert!(caps.supports_special(SpecialSearchParam::Id));
        assert!(caps.supports_include(IncludeCapability::Include));
    }

    #[test]
    fn test_global_search_capabilities() {
        let global = GlobalSearchCapabilities::new()
            .with_special_params(vec![SpecialSearchParam::Id])
            .with_max_chain_depth(3)
            .with_system_search();

        assert!(
            global
                .common_special_params
                .contains(&SpecialSearchParam::Id)
        );
        assert_eq!(global.max_chain_depth, Some(3));
        assert!(global.supports_system_search);
    }

    #[test]
    fn test_unsupported_search_feature() {
        let err = UnsupportedSearchFeature::unknown_parameter("Patient", "unknown");
        assert_eq!(err.feature_type, UnsupportedFeatureType::UnknownParameter);
        assert!(err.parameter.is_some());
        assert!(err.to_string().contains("unknown"));

        let err2 = UnsupportedSearchFeature::unsupported_modifier("name", "phonetic");
        assert_eq!(
            err2.feature_type,
            UnsupportedFeatureType::UnsupportedModifier
        );
    }

    #[test]
    fn test_search_capabilities_chaining() {
        let caps = ResourceSearchCapabilities::new("Observation")
            .with_chaining_capabilities(vec![ChainingCapability::ForwardChain]);

        assert!(caps.supports_chaining());
        assert!(!caps.supports_reverse_chaining());
    }
}
