//! SearchParameter Registry.
//!
//! The registry maintains an in-memory cache of all active SearchParameters,
//! indexed by both (resource_type, param_code) and canonical URL.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::types::{SearchParamType, SearchValue};

use super::errors::RegistryError;
use super::loader::SearchParameterLoader;

/// Status of a SearchParameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchParameterStatus {
    /// Active - can be used in searches.
    #[default]
    Active,
    /// Draft - informational, not yet active.
    Draft,
    /// Retired - disabled, not usable.
    Retired,
}

impl SearchParameterStatus {
    /// Parse from FHIR status string.
    pub fn from_fhir_status(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(SearchParameterStatus::Active),
            "draft" => Some(SearchParameterStatus::Draft),
            "retired" => Some(SearchParameterStatus::Retired),
            _ => None,
        }
    }

    /// Convert to FHIR status string.
    pub fn to_fhir_status(&self) -> &'static str {
        match self {
            SearchParameterStatus::Active => "active",
            SearchParameterStatus::Draft => "draft",
            SearchParameterStatus::Retired => "retired",
        }
    }

    /// Returns true if this status allows the parameter to be used in searches.
    pub fn is_usable(&self) -> bool {
        *self == SearchParameterStatus::Active
    }
}

/// Source of a SearchParameter definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchParameterSource {
    /// Built-in standard parameters (bundled at compile time).
    #[default]
    Embedded,
    /// POSTed SearchParameter resources (persisted in database).
    Stored,
    /// Runtime configuration file.
    Config,
}

/// Component of a composite search parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeComponentDef {
    /// Definition URL of the component parameter.
    pub definition: String,
    /// FHIRPath expression for extracting this component.
    pub expression: String,
}

/// Complete definition of a SearchParameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParameterDefinition {
    /// Canonical URL (unique identifier).
    pub url: String,

    /// Parameter code (the URL param name, e.g., "name", "identifier").
    pub code: String,

    /// Human-readable name.
    pub name: Option<String>,

    /// Description of the parameter.
    pub description: Option<String>,

    /// The parameter type.
    pub param_type: SearchParamType,

    /// FHIRPath expression for extracting values.
    pub expression: String,

    /// Resource types this parameter applies to.
    pub base: Vec<String>,

    /// Target resource types (for reference parameters).
    pub target: Option<Vec<String>>,

    /// Components (for composite parameters).
    pub component: Option<Vec<CompositeComponentDef>>,

    /// Current status.
    pub status: SearchParameterStatus,

    /// Source of this definition.
    pub source: SearchParameterSource,

    /// Supported modifiers.
    pub modifier: Option<Vec<String>>,

    /// Whether multiple values should use AND or OR logic.
    pub multiple_or: Option<bool>,
    /// Whether multiple parameters should use AND or OR logic.
    pub multiple_and: Option<bool>,

    /// Comparators supported (for number/date/quantity).
    pub comparator: Option<Vec<String>>,

    /// XPath expression (legacy, for reference).
    pub xpath: Option<String>,
}

impl SearchParameterDefinition {
    /// Creates a new SearchParameter definition.
    pub fn new(
        url: impl Into<String>,
        code: impl Into<String>,
        param_type: SearchParamType,
        expression: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            code: code.into(),
            name: None,
            description: None,
            param_type,
            expression: expression.into(),
            base: Vec::new(),
            target: None,
            component: None,
            status: SearchParameterStatus::Active,
            source: SearchParameterSource::Embedded,
            modifier: None,
            multiple_or: None,
            multiple_and: None,
            comparator: None,
            xpath: None,
        }
    }

    /// Sets the base resource types.
    pub fn with_base<I, S>(mut self, base: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base = base.into_iter().map(Into::into).collect();
        self
    }

    /// Sets target types for reference parameters.
    pub fn with_targets<I, S>(mut self, targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.target = Some(targets.into_iter().map(Into::into).collect());
        self
    }

    /// Sets the source.
    pub fn with_source(mut self, source: SearchParameterSource) -> Self {
        self.source = source;
        self
    }

    /// Sets the status.
    pub fn with_status(mut self, status: SearchParameterStatus) -> Self {
        self.status = status;
        self
    }

    /// Returns whether this is a composite parameter.
    pub fn is_composite(&self) -> bool {
        self.param_type == SearchParamType::Composite
            && self
                .component
                .as_ref()
                .map(|c| !c.is_empty())
                .unwrap_or(false)
    }

    /// Returns whether this parameter applies to the given resource type.
    pub fn applies_to(&self, resource_type: &str) -> bool {
        self.base
            .iter()
            .any(|b| b == resource_type || b == "Resource" || b == "DomainResource")
    }
}

/// Update notification for registry changes.
#[derive(Debug, Clone)]
pub enum RegistryUpdate {
    /// A parameter was added.
    Added(String),
    /// A parameter was removed.
    Removed(String),
    /// A parameter's status changed.
    StatusChanged(String, SearchParameterStatus),
    /// Registry was bulk-reloaded.
    Reloaded,
}

/// In-memory registry of SearchParameter definitions.
///
/// Provides fast lookup by (resource_type, param_code) and by URL.
/// Notifies subscribers when parameters are added, removed, or changed.
pub struct SearchParameterRegistry {
    /// Parameters indexed by (resource_type, param_code).
    params_by_type: HashMap<String, HashMap<String, Arc<SearchParameterDefinition>>>,

    /// Parameters indexed by canonical URL.
    params_by_url: HashMap<String, Arc<SearchParameterDefinition>>,

    /// Notification channel for registry updates.
    update_tx: broadcast::Sender<RegistryUpdate>,
}

impl SearchParameterRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        let (update_tx, _) = broadcast::channel(64);
        Self {
            params_by_type: HashMap::new(),
            params_by_url: HashMap::new(),
            update_tx,
        }
    }

    /// Returns the number of registered parameters.
    pub fn len(&self) -> usize {
        self.params_by_url.len()
    }

    /// Returns true if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.params_by_url.is_empty()
    }

    /// Loads all parameters from a loader.
    pub async fn load_all(
        &mut self,
        loader: &SearchParameterLoader,
    ) -> Result<usize, super::errors::LoaderError> {
        let params = loader.load_embedded()?;
        let count = params.len();

        for param in params {
            // Skip duplicates silently during bulk load
            if !self.params_by_url.contains_key(&param.url) {
                self.register_internal(param);
            }
        }

        let _ = self.update_tx.send(RegistryUpdate::Reloaded);
        Ok(count)
    }

    /// Gets all active parameters for a resource type.
    pub fn get_active_params(&self, resource_type: &str) -> Vec<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .map(|params| {
                params
                    .values()
                    .filter(|p| p.status.is_usable())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets all parameters for a resource type (including inactive).
    pub fn get_all_params(&self, resource_type: &str) -> Vec<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .map(|params| params.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Gets a specific parameter by resource type and code.
    pub fn get_param(
        &self,
        resource_type: &str,
        code: &str,
    ) -> Option<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .and_then(|params| params.get(code))
            .cloned()
    }

    /// Gets a parameter by its canonical URL.
    pub fn get_by_url(&self, url: &str) -> Option<Arc<SearchParameterDefinition>> {
        self.params_by_url.get(url).cloned()
    }

    /// Registers a new parameter.
    pub fn register(&mut self, param: SearchParameterDefinition) -> Result<(), RegistryError> {
        if self.params_by_url.contains_key(&param.url) {
            return Err(RegistryError::DuplicateUrl { url: param.url });
        }

        let url = param.url.clone();
        self.register_internal(param);
        let _ = self.update_tx.send(RegistryUpdate::Added(url));

        Ok(())
    }

    /// Internal registration without duplicate checking.
    fn register_internal(&mut self, param: SearchParameterDefinition) {
        let param = Arc::new(param);

        // Index by URL
        self.params_by_url
            .insert(param.url.clone(), Arc::clone(&param));

        // Index by (resource_type, code) for each base type
        for base in &param.base {
            self.params_by_type
                .entry(base.clone())
                .or_default()
                .insert(param.code.clone(), Arc::clone(&param));
        }
    }

    /// Updates a parameter's status.
    pub fn update_status(
        &mut self,
        url: &str,
        status: SearchParameterStatus,
    ) -> Result<(), RegistryError> {
        // We need to create a new Arc with the updated status
        let old_param = self
            .params_by_url
            .get(url)
            .ok_or_else(|| RegistryError::NotFound {
                identifier: url.to_string(),
            })?;

        // Create updated definition
        let mut new_def = (**old_param).clone();
        new_def.status = status;
        let new_param = Arc::new(new_def);

        // Update URL index
        self.params_by_url
            .insert(url.to_string(), Arc::clone(&new_param));

        // Update type indexes
        for base in &new_param.base {
            if let Some(type_params) = self.params_by_type.get_mut(base) {
                type_params.insert(new_param.code.clone(), Arc::clone(&new_param));
            }
        }

        let _ = self
            .update_tx
            .send(RegistryUpdate::StatusChanged(url.to_string(), status));

        Ok(())
    }

    /// Removes a parameter from the registry.
    pub fn unregister(&mut self, url: &str) -> Result<(), RegistryError> {
        let param = self
            .params_by_url
            .remove(url)
            .ok_or_else(|| RegistryError::NotFound {
                identifier: url.to_string(),
            })?;

        // Remove from type indexes
        for base in &param.base {
            if let Some(type_params) = self.params_by_type.get_mut(base) {
                type_params.remove(&param.code);
                if type_params.is_empty() {
                    self.params_by_type.remove(base);
                }
            }
        }

        let _ = self
            .update_tx
            .send(RegistryUpdate::Removed(url.to_string()));

        Ok(())
    }

    /// Subscribes to registry updates.
    pub fn subscribe(&self) -> broadcast::Receiver<RegistryUpdate> {
        self.update_tx.subscribe()
    }

    /// Returns all resource types that have registered parameters.
    pub fn resource_types(&self) -> Vec<String> {
        self.params_by_type.keys().cloned().collect()
    }

    /// Returns all registered parameter URLs.
    pub fn all_urls(&self) -> Vec<String> {
        self.params_by_url.keys().cloned().collect()
    }
}

impl Default for SearchParameterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministically resolves a search parameter to its `SearchParamType`.
///
/// Resolution order:
/// 1. Registry lookup by `(resource_type, name)`.
/// 2. Registry lookup by `("Resource", name)` for global params (`_id`, `_lastUpdated`, etc.).
/// 3. Value-shape heuristic — only reached for params that aren't in the registry at all
///    (e.g., user-defined custom params not yet registered).
///
/// This is the single source of truth for type resolution. REST extractors and
/// storage backends both call this so they cannot disagree.
pub fn resolve_param_type(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    values: &[SearchValue],
) -> SearchParamType {
    if let Some(def) = registry.get_param(resource_type, name) {
        return def.param_type;
    }
    if let Some(def) = registry.get_param("Resource", name) {
        return def.param_type;
    }
    infer_param_type_from_value(values)
}

/// Resolves the allowed target resource types for a reference search parameter.
///
/// Returns the registry-declared targets (e.g., `["Patient", "Group"]` for
/// `Encounter.subject`). Returns an empty `Vec` when the parameter is unknown
/// or has no declared targets — callers should treat that as "don't filter by
/// target type."
pub fn resolve_param_targets(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
) -> Vec<String> {
    let lookup = registry
        .get_param(resource_type, name)
        .or_else(|| registry.get_param("Resource", name));
    lookup
        .and_then(|def| def.target.clone())
        .unwrap_or_default()
}

/// Last-resort value-shape heuristic for parameters not present in the registry.
///
/// Kept intentionally conservative — recognizes only the unambiguous shapes
/// (FHIR date, quantity with unit, token with system, reference) and otherwise
/// returns `String`.
fn infer_param_type_from_value(values: &[SearchValue]) -> SearchParamType {
    let Some(first) = values.first() else {
        return SearchParamType::String;
    };
    let value = &first.value;

    // FHIR date: YYYY or YYYY-MM-DD or full instant. Optional comparator prefix
    // (gt/lt/ge/le/sa/eb/ap/eq/ne) is stripped by SearchValue::parse before
    // we get here, so we only inspect the literal value.
    if value.len() >= 4 && value.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
        let rest = &value.as_bytes()[4..];
        if rest.is_empty() || rest[0] == b'-' || rest[0] == b'T' {
            return SearchParamType::Date;
        }
    }

    // Quantity: number|system|code (FHIR token-style separator).
    if value.contains('|') && value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return SearchParamType::Quantity;
    }

    // Reference: ResourceType/id form.
    if value.contains('/') && value.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return SearchParamType::Reference;
    }

    // Token: system|code.
    if value.contains('|') {
        return SearchParamType::Token;
    }

    SearchParamType::String
}

impl std::fmt::Debug for SearchParameterRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchParameterRegistry")
            .field("params_count", &self.params_by_url.len())
            .field(
                "resource_types",
                &self.params_by_type.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_parameter_status() {
        assert!(SearchParameterStatus::Active.is_usable());
        assert!(!SearchParameterStatus::Draft.is_usable());
        assert!(!SearchParameterStatus::Retired.is_usable());

        assert_eq!(
            SearchParameterStatus::from_fhir_status("active"),
            Some(SearchParameterStatus::Active)
        );
        assert_eq!(SearchParameterStatus::Active.to_fhir_status(), "active");
    }

    #[test]
    fn test_search_parameter_definition() {
        let def = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Patient-name",
            "name",
            SearchParamType::String,
            "Patient.name",
        )
        .with_base(vec!["Patient"]);

        assert_eq!(def.code, "name");
        assert!(def.applies_to("Patient"));
        assert!(!def.applies_to("Observation"));
    }

    #[test]
    fn test_registry_operations() {
        let mut registry = SearchParameterRegistry::new();

        let def = SearchParameterDefinition::new(
            "http://example.org/sp/test",
            "test",
            SearchParamType::String,
            "Patient.test",
        )
        .with_base(vec!["Patient"]);

        // Register
        registry.register(def.clone()).unwrap();
        assert_eq!(registry.len(), 1);

        // Get by URL
        let found = registry.get_by_url("http://example.org/sp/test");
        assert!(found.is_some());

        // Get by type and code
        let found = registry.get_param("Patient", "test");
        assert!(found.is_some());
        assert_eq!(found.unwrap().code, "test");

        // Get active params
        let active = registry.get_active_params("Patient");
        assert_eq!(active.len(), 1);

        // Update status
        registry
            .update_status("http://example.org/sp/test", SearchParameterStatus::Retired)
            .unwrap();
        let active = registry.get_active_params("Patient");
        assert_eq!(active.len(), 0);

        // Unregister
        registry.unregister("http://example.org/sp/test").unwrap();
        assert_eq!(registry.len(), 0);
    }

    fn registry_with(defs: Vec<SearchParameterDefinition>) -> SearchParameterRegistry {
        let mut r = SearchParameterRegistry::new();
        for d in defs {
            r.register(d).unwrap();
        }
        r
    }

    #[test]
    fn resolve_param_type_hits_resource_specific_definition() {
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Goal-target-date",
                "target-date",
                SearchParamType::Date,
                "Goal.target.dueDate",
            )
            .with_base(vec!["Goal"]),
        ]);

        assert_eq!(
            resolve_param_type(&registry, "Goal", "target-date", &[]),
            SearchParamType::Date,
        );
    }

    #[test]
    fn resolve_param_type_falls_back_to_resource_base_for_global_params() {
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-lastUpdated",
                "_lastUpdated",
                SearchParamType::Date,
                "Resource.meta.lastUpdated",
            )
            .with_base(vec!["Resource"]),
        ]);

        assert_eq!(
            resolve_param_type(&registry, "Patient", "_lastUpdated", &[]),
            SearchParamType::Date,
        );
    }

    #[test]
    fn resolve_param_type_uses_value_heuristic_when_unregistered() {
        let registry = SearchParameterRegistry::new();
        let date_value = vec![SearchValue::eq("2020-01-15")];
        let ref_value = vec![SearchValue::eq("Patient/123")];
        let token_value = vec![SearchValue::eq("http://loinc.org|1234-5")];
        let plain = vec![SearchValue::eq("hello")];

        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &date_value),
            SearchParamType::Date,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &ref_value),
            SearchParamType::Reference,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &token_value),
            SearchParamType::Token,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &plain),
            SearchParamType::String,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &[]),
            SearchParamType::String,
        );
    }

    #[test]
    fn resolve_param_targets_returns_declared_targets() {
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Encounter-subject",
                "subject",
                SearchParamType::Reference,
                "Encounter.subject",
            )
            .with_base(vec!["Encounter"])
            .with_targets(vec!["Patient", "Group"]),
        ]);

        assert_eq!(
            resolve_param_targets(&registry, "Encounter", "subject"),
            vec!["Patient".to_string(), "Group".to_string()],
        );
        assert!(resolve_param_targets(&registry, "Encounter", "missing").is_empty());
    }

    #[test]
    fn test_duplicate_url_error() {
        let mut registry = SearchParameterRegistry::new();

        let def = SearchParameterDefinition::new(
            "http://example.org/sp/test",
            "test",
            SearchParamType::String,
            "Patient.test",
        )
        .with_base(vec!["Patient"]);

        registry.register(def.clone()).unwrap();

        let result = registry.register(def);
        assert!(matches!(result, Err(RegistryError::DuplicateUrl { .. })));
    }
}
