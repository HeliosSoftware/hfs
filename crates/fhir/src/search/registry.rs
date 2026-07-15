//! In-memory registry of SearchParameter definitions.
//!
//! Lifted from `helios-persistence`. Locking (`Arc<RwLock<...>>`) and
//! change-notification (broadcast channel) stay in `helios-persistence`
//! where the runtime concerns live; this module owns the pure spec data.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::errors::{LoaderError, RegistryError};
use super::loader::SearchParameterLoader;
use super::types::SearchParamType;

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
///
/// The source also carries the **shadowing precedence** (see
/// [`precedence`](Self::precedence)): when two parameters from *different*
/// sources share a `(base, code)` slot, the higher-precedence source resolves
/// searches — deliberately, so an implementer can override a spec parameter's
/// expression by loading their own definition under a new canonical URL. Two
/// parameters from the *same* source sharing a slot have no precedence to
/// break the tie and are rejected as a conflict
/// ([`RegistryError::DuplicateCode`]).
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

impl SearchParameterSource {
    /// Shadowing precedence: `Embedded < Config < Stored`.
    ///
    /// Spec defaults lose to definitions the operator placed in the data
    /// directory, which in turn lose to definitions administered through the
    /// FHIR API. Resolution depends only on this ordering, never on load
    /// order.
    pub fn precedence(&self) -> u8 {
        match self {
            SearchParameterSource::Embedded => 0,
            SearchParameterSource::Config => 1,
            SearchParameterSource::Stored => 2,
        }
    }
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

/// In-memory registry of SearchParameter definitions.
///
/// Provides fast lookup by (resource_type, param_code) and by URL.
/// External locking (e.g. `Arc<RwLock<...>>`) is the caller's
/// responsibility — the registry itself is a plain struct.
///
/// Every registered parameter is kept in both indexes. A `(base, code)` slot
/// holds *all* the parameters claiming it, ordered by
/// [`SearchParameterSource::precedence`] (ties broken by registration order),
/// and the head of that list is what resolves searches. Registering,
/// unregistering, or re-statusing a *shadowed* parameter therefore never
/// changes which definition answers a query, and removing the winner
/// re-points the slot at the next candidate instead of leaving a hole (#239).
pub struct SearchParameterRegistry {
    /// Parameters indexed by (resource_type, param_code). Each slot holds the
    /// candidates in descending precedence order; the first entry wins.
    params_by_type: HashMap<String, HashMap<String, Vec<Arc<SearchParameterDefinition>>>>,

    /// Parameters indexed by canonical URL.
    params_by_url: HashMap<String, Arc<SearchParameterDefinition>>,
}

impl SearchParameterRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            params_by_type: HashMap::new(),
            params_by_url: HashMap::new(),
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
    pub async fn load_all(&mut self, loader: &SearchParameterLoader) -> Result<usize, LoaderError> {
        let params = loader.load_embedded()?;
        let count = params.len();

        for param in params {
            // Skip duplicates silently during bulk load
            if !self.params_by_url.contains_key(&param.url) {
                self.register_internal(param);
            }
        }

        Ok(count)
    }

    /// Gets all active parameters for a resource type.
    ///
    /// One parameter per `(base, code)` slot — the winning candidate — with
    /// unusable statuses filtered out.
    pub fn get_active_params(&self, resource_type: &str) -> Vec<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .map(|params| {
                params
                    .values()
                    .filter_map(|candidates| candidates.first())
                    .filter(|p| p.status.is_usable())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets all parameters for a resource type (including inactive).
    ///
    /// One parameter per `(base, code)` slot — shadowed candidates are not
    /// included; they remain reachable via [`get_by_url`](Self::get_by_url).
    pub fn get_all_params(&self, resource_type: &str) -> Vec<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .map(|params| {
                params
                    .values()
                    .filter_map(|candidates| candidates.first())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets a parameter by resource type and code.
    pub fn get_param(
        &self,
        resource_type: &str,
        code: &str,
    ) -> Option<Arc<SearchParameterDefinition>> {
        self.params_by_type
            .get(resource_type)
            .and_then(|params| params.get(code))
            .and_then(|candidates| candidates.first())
            .cloned()
    }

    /// Gets a parameter by its canonical URL.
    pub fn get_by_url(&self, url: &str) -> Option<Arc<SearchParameterDefinition>> {
        self.params_by_url.get(url).cloned()
    }

    /// Registers a new parameter.
    ///
    /// Fails with [`RegistryError::DuplicateUrl`] when the canonical URL is
    /// already registered, and with [`RegistryError::DuplicateCode`] when a
    /// parameter **from the same source** already claims one of the
    /// `(base, code)` slots — same-source collisions have no precedence to
    /// resolve them. A cross-source collision registers successfully as a
    /// shadow (resolved by [`SearchParameterSource::precedence`]) and logs a
    /// `WARN` naming both canonical URLs.
    pub fn register(&mut self, param: SearchParameterDefinition) -> Result<(), RegistryError> {
        if self.params_by_url.contains_key(&param.url) {
            return Err(RegistryError::DuplicateUrl { url: param.url });
        }
        // Validate every slot before mutating any, so a rejected registration
        // leaves the registry untouched.
        for base in &param.base {
            if let Some(existing) = self
                .params_by_type
                .get(base)
                .and_then(|params| params.get(&param.code))
                .and_then(|candidates| candidates.iter().find(|c| c.source == param.source))
            {
                return Err(RegistryError::DuplicateCode {
                    base: base.clone(),
                    code: param.code.clone(),
                    existing_url: existing.url.clone(),
                });
            }
        }
        self.register_internal(param);
        Ok(())
    }

    /// Internal registration without duplicate checking.
    ///
    /// Bulk-load path: same-source `(base, code)` collisions are tolerated
    /// here (first registration wins the tie) so that loading a malformed
    /// custom file degrades to a warning instead of aborting startup.
    fn register_internal(&mut self, param: SearchParameterDefinition) {
        let param = Arc::new(param);

        // Index by URL
        self.params_by_url
            .insert(param.url.clone(), Arc::clone(&param));

        // Index by (resource_type, code) for each base type
        for base in &param.base {
            let candidates = self
                .params_by_type
                .entry(base.clone())
                .or_default()
                .entry(param.code.clone())
                .or_default();

            // Descending precedence, stable within a precedence level, so the
            // winner (index 0) is independent of load order.
            let position = candidates
                .iter()
                .position(|c| c.source.precedence() < param.source.precedence())
                .unwrap_or(candidates.len());
            candidates.insert(position, Arc::clone(&param));

            if position == 0 {
                if let Some(shadowed) = candidates.get(1) {
                    tracing::warn!(
                        base = %base,
                        code = %param.code,
                        winner = %param.url,
                        shadowed = %shadowed.url,
                        "SearchParameter shadows an existing definition on (base, code); \
                         the higher-precedence source now resolves searches"
                    );
                }
            } else {
                tracing::warn!(
                    base = %base,
                    code = %param.code,
                    winner = %candidates[0].url,
                    shadowed = %param.url,
                    "SearchParameter registered shadowed on (base, code); \
                     an existing higher-precedence definition keeps resolving searches"
                );
            }
        }
    }

    /// Updates a parameter's status.
    ///
    /// Replaces the definition **in place** in every index: a shadowed
    /// parameter keeps its position behind the winner, so a status edit never
    /// changes which definition resolves searches.
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

        // Update type indexes: swap the same slot position by URL identity.
        for base in &new_param.base {
            if let Some(candidates) = self
                .params_by_type
                .get_mut(base)
                .and_then(|params| params.get_mut(&new_param.code))
                && let Some(slot) = candidates.iter_mut().find(|c| c.url == *url)
            {
                *slot = Arc::clone(&new_param);
            }
        }

        Ok(())
    }

    /// Removes a parameter from the registry.
    ///
    /// Only the candidate with this exact canonical URL is evicted from each
    /// `(base, code)` slot; removing a shadowed parameter leaves the winner
    /// untouched, and removing the winner promotes the next candidate.
    pub fn unregister(&mut self, url: &str) -> Result<(), RegistryError> {
        let param = self
            .params_by_url
            .remove(url)
            .ok_or_else(|| RegistryError::NotFound {
                identifier: url.to_string(),
            })?;

        // Remove from type indexes by URL identity.
        for base in &param.base {
            if let Some(type_params) = self.params_by_type.get_mut(base) {
                if let Some(candidates) = type_params.get_mut(&param.code) {
                    candidates.retain(|c| c.url != param.url);
                    if candidates.is_empty() {
                        type_params.remove(&param.code);
                    }
                }
                if type_params.is_empty() {
                    self.params_by_type.remove(base);
                }
            }
        }

        Ok(())
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
/// `values` are the raw search-value strings (without comparator prefixes,
/// which callers strip before calling). Persistence's
/// `SearchValue` wrapper exposes its inner string via `.value`.
pub fn resolve_param_type(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    values: &[&str],
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
fn infer_param_type_from_value(values: &[&str]) -> SearchParamType {
    let Some(value) = values.first() else {
        return SearchParamType::String;
    };

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

        registry.register(def.clone()).unwrap();
        assert_eq!(registry.len(), 1);

        let found = registry.get_by_url("http://example.org/sp/test");
        assert!(found.is_some());

        let found = registry.get_param("Patient", "test");
        assert!(found.is_some());
        assert_eq!(found.unwrap().code, "test");

        let active = registry.get_active_params("Patient");
        assert_eq!(active.len(), 1);

        registry
            .update_status("http://example.org/sp/test", SearchParameterStatus::Retired)
            .unwrap();
        let active = registry.get_active_params("Patient");
        assert_eq!(active.len(), 0);

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
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &["2020-01-15"]),
            SearchParamType::Date,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &["Patient/123"]),
            SearchParamType::Reference,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &["http://loinc.org|1234-5"]),
            SearchParamType::Token,
        );
        assert_eq!(
            resolve_param_type(&registry, "Custom", "x", &["hello"]),
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

    // ---- (base, code) shadowing (#239) ----------------------------------

    const SPEC_URL: &str = "http://hl7.org/fhir/SearchParameter/clinical-code";
    const CUSTOM_URL: &str = "http://acme.health/fhir/SearchParameter/observation-code-local";

    fn spec_code_param() -> SearchParameterDefinition {
        SearchParameterDefinition::new(SPEC_URL, "code", SearchParamType::Token, "Observation.code")
            .with_base(vec!["Observation"])
            .with_source(SearchParameterSource::Embedded)
    }

    fn custom_code_param() -> SearchParameterDefinition {
        SearchParameterDefinition::new(
            CUSTOM_URL,
            "code",
            SearchParamType::Token,
            "Observation.code.coding.where(system='http://acme.health/local')",
        )
        .with_base(vec!["Observation"])
        .with_source(SearchParameterSource::Config)
    }

    #[test]
    fn shadowing_resolves_by_source_precedence_not_load_order() {
        for reversed in [false, true] {
            let mut registry = SearchParameterRegistry::new();
            let (first, second) = if reversed {
                (custom_code_param(), spec_code_param())
            } else {
                (spec_code_param(), custom_code_param())
            };
            registry.register(first).unwrap();
            registry.register(second).unwrap();

            // Both registrations are visible and reachable by URL.
            assert_eq!(registry.len(), 2);
            assert!(registry.get_by_url(SPEC_URL).is_some());
            assert!(registry.get_by_url(CUSTOM_URL).is_some());

            // Config outranks Embedded regardless of which loaded first.
            let resolved = registry.get_param("Observation", "code").unwrap();
            assert_eq!(resolved.url, CUSTOM_URL, "reversed={reversed}");
            assert_eq!(registry.get_active_params("Observation").len(), 1);
        }
    }

    #[test]
    fn same_source_collision_is_rejected() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(custom_code_param()).unwrap();

        let rival = SearchParameterDefinition::new(
            "http://acme.health/fhir/SearchParameter/observation-code-other",
            "code",
            SearchParamType::Token,
            "Observation.code",
        )
        .with_base(vec!["Observation"])
        .with_source(SearchParameterSource::Config);

        let result = registry.register(rival);
        assert!(matches!(
            result,
            Err(RegistryError::DuplicateCode { ref existing_url, .. }) if existing_url == CUSTOM_URL
        ));
        // The losing registration left no trace.
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.get_param("Observation", "code").unwrap().url,
            CUSTOM_URL
        );
    }

    /// A rejected registration must not leave partial index entries behind
    /// when only one of several base types collides.
    #[test]
    fn rejected_registration_leaves_registry_untouched() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(custom_code_param()).unwrap();

        let partial_collision = SearchParameterDefinition::new(
            "http://acme.health/fhir/SearchParameter/multi-base-code",
            "code",
            SearchParamType::Token,
            "Observation.code",
        )
        .with_base(vec!["Patient", "Observation"])
        .with_source(SearchParameterSource::Config);

        assert!(registry.register(partial_collision).is_err());
        assert!(registry.get_param("Patient", "code").is_none());
        assert!(!registry.resource_types().contains(&"Patient".to_string()));
    }

    /// Symptom 2 (#239): unregistering the shadowed loser must not evict the
    /// winner's index entry.
    #[test]
    fn unregister_shadowed_param_keeps_winner_searchable() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(spec_code_param()).unwrap();
        registry.register(custom_code_param()).unwrap();

        registry.unregister(SPEC_URL).unwrap();

        assert_eq!(registry.len(), 1);
        let resolved = registry.get_param("Observation", "code").unwrap();
        assert_eq!(resolved.url, CUSTOM_URL);
        assert!(
            registry
                .resource_types()
                .contains(&"Observation".to_string())
        );
    }

    /// Removing the winner promotes the next candidate instead of leaving the
    /// slot empty.
    #[test]
    fn unregister_winner_promotes_next_candidate() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(spec_code_param()).unwrap();
        registry.register(custom_code_param()).unwrap();

        registry.unregister(CUSTOM_URL).unwrap();

        let resolved = registry.get_param("Observation", "code").unwrap();
        assert_eq!(resolved.url, SPEC_URL);
    }

    /// Symptom 3 (#239): a status edit on the shadowed loser must not flip
    /// which definition resolves searches.
    #[test]
    fn update_status_on_shadowed_does_not_change_resolution() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(spec_code_param()).unwrap();
        registry.register(custom_code_param()).unwrap();

        registry
            .update_status(SPEC_URL, SearchParameterStatus::Retired)
            .unwrap();

        let resolved = registry.get_param("Observation", "code").unwrap();
        assert_eq!(resolved.url, CUSTOM_URL);
        // The edit itself landed on the shadowed definition.
        assert_eq!(
            registry.get_by_url(SPEC_URL).unwrap().status,
            SearchParameterStatus::Retired
        );
    }

    #[test]
    fn update_status_still_applies_to_the_winner() {
        let mut registry = SearchParameterRegistry::new();
        registry.register(spec_code_param()).unwrap();
        registry.register(custom_code_param()).unwrap();

        registry
            .update_status(CUSTOM_URL, SearchParameterStatus::Retired)
            .unwrap();

        // The winner still owns the slot; it is just no longer active.
        let resolved = registry.get_param("Observation", "code").unwrap();
        assert_eq!(resolved.url, CUSTOM_URL);
        assert_eq!(resolved.status, SearchParameterStatus::Retired);
        assert!(registry.get_active_params("Observation").is_empty());
    }
}
