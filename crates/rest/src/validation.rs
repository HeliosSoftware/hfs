//! Resource validation service: the REST layer's bridge to
//! `helios-fhir-validator`.
//!
//! Owns the per-version core validators (embedded schema packs), the
//! FHIRPath constraint evaluator, the optional terminology provider,
//! per-tenant profile registries fed from stored StructureDefinitions, and
//! the mapping from validator issues onto FHIR `OperationOutcome` issues.
//!
//! The `$validate` operation uses [`ValidationService::validate_resource`]
//! unconditionally; the write path (create/update/batch) goes through
//! [`ValidationService::check_write`], which is gated by
//! `HFS_VALIDATION_MODE` (`off` | `log` | `enforce`).

use crate::config::ValidationConfig;
use crate::error::RestError;
use crate::responses::operation_outcome::{
    Issue, IssueSeverity, IssueType, OperationOutcomeBuilder,
};
use async_trait::async_trait;
use dashmap::DashMap;
use helios_fhir::FhirVersion;
use helios_fhir_validator::fhirpath_effects::FhirPathConstraintEvaluator;
use helios_fhir_validator::{
    CodedValue, CompositeResolver, EffectHandlers, ErrorKind, PackageCache, PackageId, PackageRef,
    SchemaRegistry, SchemaResolver, Severity, TerminologyError, TerminologyProvider,
    UnknownProfilePolicy, ValidationError, ValidationOptions, Validator, dotted_to_fhirpath,
    materialize_package_layers_by_version, validate_questionnaire_response,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// How terminology bindings are checked.
#[derive(Clone)]
enum TerminologyMode {
    Off,
    /// Offline core ValueSets — selected per request FHIR version.
    Embedded,
    /// Remote `$validate-code` (version-agnostic).
    Remote(Arc<dyn TerminologyProvider>),
}

/// Per-tenant, per-version profile registries fed from stored
/// StructureDefinitions.
type TenantProfileMap = DashMap<(String, FhirVersion), Arc<RwLock<SchemaRegistry>>>;

/// Write-path behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationMode {
    /// Skip write-path validation entirely.
    #[default]
    Off,
    /// Validate, log issues, and proceed.
    Log,
    /// Reject invalid resources with `422 Unprocessable Entity`.
    Enforce,
}

/// The validation service. Cheap to construct (core registries load lazily,
/// once per process); hold one in `AppState` for the AST cache to pay off.
pub struct ValidationService {
    mode: ValidationMode,
    constraint_evaluator: Option<FhirPathConstraintEvaluator>,
    terminology: TerminologyMode,
    /// Constraint ids never evaluated (default: `dom-6`, the narrative
    /// warning that fires on almost every machine-generated resource).
    suppress_constraints: Vec<String>,
    /// Escalate terminology-service failures to errors.
    terminology_fail_closed: bool,
    /// Validate against `meta.profile` claims.
    use_meta_profiles: bool,
    unknown_profile: UnknownProfilePolicy,
    /// Opt-in warning enforcement for `extensible`-strength bindings.
    extensible_bindings: bool,
    /// Enforce `refers` target-type checks on Reference elements.
    enforce_refers: bool,
    /// Per-tenant profile overlays, fed from stored StructureDefinition
    /// writes. `None` disables the feature.
    tenant_profiles: Option<TenantProfileMap>,
    /// Server-wide IG/NPM package registry layers keyed by FHIR version
    /// (dependents before deps). Empty when packages are unset.
    package_layers: HashMap<FhirVersion, Vec<Arc<SchemaRegistry>>>,
    /// Optional lookup for Questionnaire resources (QR validation).
    questionnaire_lookup: Option<Arc<dyn QuestionnaireLookup>>,
}

/// Resolves a Questionnaire by canonical URL for QuestionnaireResponse checks.
pub trait QuestionnaireLookup: Send + Sync {
    /// Return the Questionnaire resource for `canonical`, if available.
    fn get_questionnaire(&self, canonical: &str) -> Option<Value>;
}

impl Default for ValidationService {
    fn default() -> Self {
        Self {
            mode: ValidationMode::Off,
            constraint_evaluator: Some(FhirPathConstraintEvaluator::new()),
            terminology: TerminologyMode::Off,
            suppress_constraints: vec!["dom-6".to_string()],
            terminology_fail_closed: false,
            use_meta_profiles: true,
            unknown_profile: UnknownProfilePolicy::Warn,
            extensible_bindings: false,
            enforce_refers: false,
            tenant_profiles: Some(DashMap::new()),
            package_layers: HashMap::new(),
            questionnaire_lookup: None,
        }
    }
}

impl ValidationService {
    /// A service with the default posture: write path off, constraints on
    /// (dom-6 suppressed), meta.profile honored, unresolvable profiles
    /// warned, stored profiles on, no terminology provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a service from `HFS_VALIDATION_*` / `HFS_FHIR_*` configuration.
    /// `terminology_server` is `HFS_TERMINOLOGY_SERVER` (required for
    /// `terminology = remote`; the config validator enforces the pairing).
    ///
    /// Fails when `HFS_FHIR_PACKAGES` is set and package resolution or
    /// materialization fails — never boots with a silently empty overlay.
    pub fn from_config(
        config: &ValidationConfig,
        terminology_server: Option<&str>,
        version: helios_fhir::FhirVersion,
    ) -> Result<Self, String> {
        let mode = match config.mode.as_str() {
            "log" => ValidationMode::Log,
            "enforce" => ValidationMode::Enforce,
            _ => ValidationMode::Off,
        };
        let unknown_profile = match config.unknown_profile.as_str() {
            "error" => UnknownProfilePolicy::Error,
            "ignore" => UnknownProfilePolicy::Ignore,
            _ => UnknownProfilePolicy::Warn,
        };
        let terminology = match (config.terminology.as_str(), terminology_server) {
            ("remote", Some(base)) => {
                TerminologyMode::Remote(Arc::new(RemoteTerminologyProvider::new(
                    base.to_string(),
                    Duration::from_millis(config.terminology_timeout_ms),
                )))
            }
            // Offline required-binding checks against the FHIR core value
            // sets embedded in helios-fhir-validator (selected per request).
            ("embedded", _) => TerminologyMode::Embedded,
            _ => TerminologyMode::Off,
        };

        let package_layers = load_package_layers(config, version)?;

        Ok(Self {
            mode,
            constraint_evaluator: config.constraints.then(FhirPathConstraintEvaluator::new),
            terminology,
            suppress_constraints: config.suppress_constraints.clone(),
            terminology_fail_closed: config.terminology_fail == "closed",
            use_meta_profiles: config.meta_profiles,
            unknown_profile,
            extensible_bindings: false,
            enforce_refers: false,
            tenant_profiles: config.stored_profiles.then(DashMap::new),
            package_layers,
            questionnaire_lookup: None,
        })
    }

    /// Replace the terminology provider with a remote/custom one.
    pub fn with_terminology(mut self, provider: Arc<dyn TerminologyProvider>) -> Self {
        self.terminology = TerminologyMode::Remote(provider);
        self
    }

    /// Install a Questionnaire lookup used for QuestionnaireResponse checks.
    pub fn with_questionnaire_lookup(mut self, lookup: Arc<dyn QuestionnaireLookup>) -> Self {
        self.questionnaire_lookup = Some(lookup);
        self
    }

    /// The configured write-path mode.
    pub fn mode(&self) -> ValidationMode {
        self.mode
    }

    /// Validate a resource against the core pack for `version` (overlaid
    /// with package layers and the tenant's stored profiles) plus any extra
    /// profile canonicals. Structural issues first, then constraint issues,
    /// then binding issues.
    pub async fn validate_resource(
        &self,
        version: FhirVersion,
        resource: &Value,
        profiles: Vec<String>,
        tenant: Option<&str>,
    ) -> Vec<ValidationError> {
        let resolver = self.resolver_for(version, tenant);
        let validator = Validator::new(Arc::clone(&resolver));
        let opts = ValidationOptions {
            profiles,
            use_meta_profiles: self.use_meta_profiles,
            unknown_profile: self.unknown_profile,
            enforce_refers: self.enforce_refers,
        };
        let embedded = match &self.terminology {
            TerminologyMode::Embedded => Some(helios_fhir_validator::core_terminology(version)),
            _ => None,
        };
        let terminology: Option<&dyn TerminologyProvider> = match (&self.terminology, &embedded) {
            (TerminologyMode::Remote(provider), _) => Some(provider.as_ref()),
            (TerminologyMode::Embedded, Some(provider)) => Some(provider.as_ref()),
            _ => None,
        };
        let handlers = EffectHandlers {
            constraints: self
                .constraint_evaluator
                .as_ref()
                .map(|e| e as &dyn helios_fhir_validator::ConstraintEvaluator),
            terminology,
            suppress_constraints: &self.suppress_constraints,
            terminology_fail_closed: self.terminology_fail_closed,
            check_extensible_bindings: self.extensible_bindings,
        };
        let mut issues = validator
            .validate(resource, version, &opts, &handlers)
            .await;

        if resource.get("resourceType").and_then(Value::as_str) == Some("QuestionnaireResponse") {
            issues.extend(self.validate_qr(resource, terminology).await);
        }
        issues
    }

    async fn validate_qr(
        &self,
        qr: &Value,
        terminology: Option<&dyn TerminologyProvider>,
    ) -> Vec<ValidationError> {
        let Some(canonical) = qr.get("questionnaire").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(lookup) = &self.questionnaire_lookup else {
            return Vec::new();
        };
        let Some(questionnaire) = lookup.get_questionnaire(canonical) else {
            return vec![
                ValidationError::new(
                    ErrorKind::UnknownSchema,
                    "QuestionnaireResponse.questionnaire".into(),
                    format!(
                        "could not resolve Questionnaire '{canonical}' for response validation"
                    ),
                )
                .with_severity(Severity::Warning),
            ];
        };
        validate_questionnaire_response(qr, &questionnaire, terminology).await
    }

    /// Write-path gate. `Ok(())` = proceed with the write; `Err` = reject
    /// (enforce mode with error-severity issues → `422` carrying the full
    /// OperationOutcome).
    pub async fn check_write(
        &self,
        tenant: &str,
        version: FhirVersion,
        resource_type: &str,
        resource: &Value,
    ) -> Result<(), RestError> {
        if self.mode == ValidationMode::Off {
            return Ok(());
        }
        let issues = self
            .validate_resource(version, resource, Vec::new(), Some(tenant))
            .await;
        if issues.is_empty() {
            return Ok(());
        }
        match self.mode {
            ValidationMode::Off => Ok(()),
            ValidationMode::Log => {
                for issue in &issues {
                    warn!(
                        tenant = %tenant,
                        resource_type = %resource_type,
                        path = %issue.path,
                        kind = ?issue.kind,
                        "validation (log mode): {}",
                        issue.message
                    );
                }
                Ok(())
            }
            ValidationMode::Enforce => {
                if has_errors(&issues) {
                    debug!(
                        tenant = %tenant,
                        resource_type = %resource_type,
                        issues = issues.len(),
                        "rejecting write: validation failed"
                    );
                    Err(RestError::ValidationFailed {
                        outcome: validation_outcome(&issues),
                    })
                } else {
                    // Warnings only: proceed, but leave a trace.
                    for issue in &issues {
                        warn!(
                            tenant = %tenant,
                            resource_type = %resource_type,
                            path = %issue.path,
                            "validation warning on write: {}",
                            issue.message
                        );
                    }
                    Ok(())
                }
            }
        }
    }

    /// Fold a stored StructureDefinition into the tenant's profile registry
    /// (called after successful StructureDefinition writes). Conversion
    /// failures are logged, never fatal — the write itself already
    /// succeeded.
    pub fn upsert_stored_profile(&self, tenant: &str, version: FhirVersion, sd: &Value) {
        let Some(registries) = &self.tenant_profiles else {
            return;
        };
        match helios_fhir_validator::converter::convert(sd) {
            Ok(conversion) => {
                for w in &conversion.warnings {
                    warn!(tenant = %tenant, "profile conversion warning: {w}");
                }
                let registry = registries
                    .entry((tenant.to_string(), version))
                    .or_insert_with(|| Arc::new(RwLock::new(SchemaRegistry::new())))
                    .clone();
                let inserted = registry
                    .write()
                    .expect("tenant profile registry lock")
                    .insert(conversion.schema);
                if inserted {
                    debug!(tenant = %tenant, url = ?sd.get("url"), "tenant profile registered");
                } else {
                    warn!(tenant = %tenant, "stored StructureDefinition has neither url nor name; not registered");
                }
            }
            Err(e) => {
                warn!(tenant = %tenant, "stored StructureDefinition failed to convert: {e}");
            }
        }
    }

    fn tenant_overlay(
        &self,
        tenant: Option<&str>,
        version: FhirVersion,
    ) -> Option<Arc<dyn SchemaResolver>> {
        let registries = self.tenant_profiles.as_ref()?;
        let tenant = tenant?;
        let registry = registries.get(&(tenant.to_string(), version))?.clone();
        Some(Arc::new(LockedRegistryResolver(registry)))
    }

    /// `CompositeResolver` layers: tenant overlay, package layers for
    /// `version` (dependents before deps), then the embedded core pack.
    fn resolver_for(&self, version: FhirVersion, tenant: Option<&str>) -> Arc<dyn SchemaResolver> {
        let core = helios_fhir_validator::packs::core_registry(version);
        let mut layers: Vec<Arc<dyn SchemaResolver>> = Vec::new();
        if let Some(overlay) = self.tenant_overlay(tenant, version) {
            layers.push(overlay);
        }
        if let Some(pkgs) = self.package_layers.get(&version) {
            for pkg in pkgs {
                layers.push(Arc::clone(pkg) as Arc<dyn SchemaResolver>);
            }
        }
        if layers.is_empty() {
            return core;
        }
        layers.push(core);
        Arc::new(CompositeResolver::new(layers))
    }
}

fn enabled_fhir_versions() -> Vec<FhirVersion> {
    let mut versions = Vec::new();
    #[cfg(feature = "R4")]
    versions.push(FhirVersion::R4);
    #[cfg(feature = "R4B")]
    versions.push(FhirVersion::R4B);
    #[cfg(feature = "R5")]
    versions.push(FhirVersion::R5);
    #[cfg(feature = "R6")]
    versions.push(FhirVersion::R6);
    versions
}

fn load_package_layers(
    config: &ValidationConfig,
    default_version: FhirVersion,
) -> Result<HashMap<FhirVersion, Vec<Arc<SchemaRegistry>>>, String> {
    if config.packages.is_empty() && config.package_sources.is_empty() {
        return Ok(HashMap::new());
    }
    let cache_root = config.package_cache.as_deref().ok_or_else(|| {
        "HFS_FHIR_PACKAGES / HFS_FHIR_PACKAGE_SOURCES require HFS_FHIR_PACKAGE_CACHE".to_string()
    })?;
    let cache = PackageCache::new(cache_root);

    let mut sourced: Vec<PackageId> = Vec::new();
    for raw in &config.package_sources {
        let id = seed_package_source(&cache, raw)
            .map_err(|e| format!("HFS_FHIR_PACKAGE_SOURCES entry '{raw}': {e}"))?;
        info!(package = %id, source = %raw, "seeded FHIR package into cache");
        sourced.push(id);
    }

    let roots = if config.packages.is_empty() {
        if sourced.is_empty() {
            return Ok(HashMap::new());
        }
        sourced
    } else {
        let mut roots = Vec::with_capacity(config.packages.len());
        for raw in &config.packages {
            roots.push(
                PackageRef::parse(raw)
                    .map_err(|e| format!("HFS_FHIR_PACKAGES entry '{raw}': {e}"))?,
            );
        }
        roots
    };

    let versions = enabled_fhir_versions();
    let by_version =
        materialize_package_layers_by_version(&cache, &roots, default_version, &versions)
            .map_err(|e| format!("FHIR package materialization failed: {e}"))?;

    for (version, layers) in &by_version {
        info!(
            fhir_version = %version.full_version(),
            layer_count = layers.len(),
            "loaded FHIR package schema layers"
        );
    }
    Ok(by_version)
}

/// Seed one source into the cache: local path (via `ensure_from_path`) or
/// HTTP(S) `.tgz` URL (downloaded under `{cache}/.downloads/`).
fn seed_package_source(cache: &PackageCache, raw: &str) -> Result<PackageId, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let downloaded = download_package_tgz(cache.root(), trimmed)?;
        return cache
            .ensure_from_tgz(&downloaded)
            .map_err(|e| e.to_string());
    }
    let path = PathBuf::from(trimmed);
    cache.ensure_from_path(&path).map_err(|e| e.to_string())
}

fn download_package_tgz(cache_root: &Path, url: &str) -> Result<PathBuf, String> {
    let downloads = cache_root.join(".downloads");
    fs::create_dir_all(&downloads)
        .map_err(|e| format!("cannot create {}: {e}", downloads.display()))?;

    let name = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("package.tgz");
    let name = if name.ends_with(".tgz") || name.ends_with(".tar.gz") {
        name.to_string()
    } else {
        format!("{name}.tgz")
    };
    let dest = downloads.join(&name);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GET {url}: {e}"))?;
    let bytes = response.bytes().map_err(|e| format!("read {url}: {e}"))?;
    fs::write(&dest, &bytes).map_err(|e| format!("write {}: {e}", dest.display()))?;
    Ok(dest)
}

/// Resolver adapter over a shared, mutable registry.
struct LockedRegistryResolver(Arc<RwLock<SchemaRegistry>>);

impl SchemaResolver for LockedRegistryResolver {
    fn resolve(&self, reference: &str) -> Option<Arc<helios_fhir_validator::FhirSchema>> {
        self.0
            .read()
            .expect("tenant profile registry lock")
            .resolve(reference)
    }
}

// ---------------------------------------------------------------------
// Remote terminology provider
// ---------------------------------------------------------------------

/// `TerminologyProvider` backed by a FHIR terminology server's
/// `ValueSet/$validate-code`, with a small in-memory TTL cache (neither the
/// fhirpath nor the search terminology clients cache).
pub struct RemoteTerminologyProvider {
    base_url: String,
    client: reqwest::Client,
    /// `(valueSet, coded-token)` → validity, cached for [`CACHE_TTL`].
    cache: DashMap<String, (bool, Instant)>,
}

/// How long `$validate-code` verdicts are cached.
const CACHE_TTL: Duration = Duration::from_secs(300);

impl RemoteTerminologyProvider {
    /// `base_url` is the terminology server root (e.g. `http://hts:8090`).
    pub fn new(base_url: String, timeout: Duration) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("reqwest client builds"),
            cache: DashMap::new(),
        }
    }

    fn payload(value_set: &str, coded: &CodedValue) -> Value {
        let mut parameter = vec![json!({ "name": "url", "valueUri": value_set })];
        match coded {
            CodedValue::Code(code) => {
                parameter.push(json!({ "name": "inferSystem", "valueBoolean": true }));
                parameter.push(json!({ "name": "code", "valueCode": code }));
            }
            CodedValue::Coding(coding) => {
                parameter.push(json!({ "name": "coding", "valueCoding": coding }));
            }
            CodedValue::CodeableConcept(concept) => {
                parameter
                    .push(json!({ "name": "codeableConcept", "valueCodeableConcept": concept }));
            }
        }
        json!({ "resourceType": "Parameters", "parameter": parameter })
    }
}

#[async_trait]
impl TerminologyProvider for RemoteTerminologyProvider {
    async fn validate_code(
        &self,
        value_set: &str,
        coded: &CodedValue,
    ) -> Result<bool, TerminologyError> {
        let key = format!("{value_set}|{coded:?}");
        if let Some(entry) = self.cache.get(&key) {
            let (verdict, at) = *entry;
            if at.elapsed() < CACHE_TTL {
                return Ok(verdict);
            }
        }

        let url = format!("{}/ValueSet/$validate-code", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&Self::payload(value_set, coded))
            .send()
            .await
            .map_err(|e| TerminologyError(format!("request to {url} failed: {e}")))?;
        if !response.status().is_success() {
            return Err(TerminologyError(format!(
                "{url} returned {}",
                response.status()
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| TerminologyError(format!("invalid $validate-code response: {e}")))?;
        let verdict = body
            .get("parameter")
            .and_then(Value::as_array)
            .and_then(|params| {
                params
                    .iter()
                    .find(|p| p.get("name").and_then(Value::as_str) == Some("result"))
            })
            .and_then(|p| p.get("valueBoolean"))
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                TerminologyError("no boolean 'result' parameter in response".to_string())
            })?;

        self.cache.insert(key, (verdict, Instant::now()));
        Ok(verdict)
    }
}

// ---------------------------------------------------------------------
// OperationOutcome mapping
// ---------------------------------------------------------------------

/// Map one validator issue onto an OperationOutcome issue.
pub fn to_outcome_issue(error: &ValidationError) -> Issue {
    let code = match error.kind {
        ErrorKind::Required => IssueType::Required,
        ErrorKind::FixedValue
        | ErrorKind::PatternValue
        | ErrorKind::PrimitiveValue
        | ErrorKind::MaxLength
        | ErrorKind::MinValue
        | ErrorKind::MaxValue => IssueType::Value,
        ErrorKind::FhirpathConstraint => IssueType::Invariant,
        ErrorKind::TerminologyBinding | ErrorKind::Questionnaire => IssueType::CodeInvalid,
        ErrorKind::UnknownSchema | ErrorKind::UnknownProfile => IssueType::NotSupported,
        // Everything structural: unknown-element, shape, cardinality,
        // slicing, choices, wrong container type, reference targets.
        ErrorKind::UnknownElement
        | ErrorKind::NotArray
        | ErrorKind::NotSingular
        | ErrorKind::Type
        | ErrorKind::Excluded
        | ErrorKind::Min
        | ErrorKind::Max
        | ErrorKind::SliceCardinality
        | ErrorKind::SliceUnmatched
        | ErrorKind::SliceOrder
        | ErrorKind::Choice
        | ErrorKind::ChoiceExcluded
        | ErrorKind::ReferenceTarget => IssueType::Structure,
    };
    let severity = match error.severity {
        Severity::Error => IssueSeverity::Error,
        Severity::Warning => IssueSeverity::Warning,
    };
    Issue::new(severity, code, error.message.clone())
        .with_expression(dotted_to_fhirpath(&error.path))
}

/// Build the `$validate` OperationOutcome: the mapped issues, or the
/// canonical all-clear information issue when there are none.
pub fn validation_outcome(errors: &[ValidationError]) -> Value {
    let mut builder = OperationOutcomeBuilder::new();
    if errors.is_empty() {
        builder = builder.information(IssueType::Informational, "Validation successful");
    } else {
        for error in errors {
            builder = builder.add_issue(to_outcome_issue(error));
        }
    }
    builder.build()
}

/// Whether any issue is error severity (drives `$validate` reporting and
/// enforce-mode rejection).
pub fn has_errors(errors: &[ValidationError]) -> bool {
    errors.iter().any(|e| e.severity == Severity::Error)
}
