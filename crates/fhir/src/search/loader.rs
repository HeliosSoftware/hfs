//! SearchParameter Loader.
//!
//! Loads SearchParameter definitions from multiple sources:
//! - Embedded standard parameters (compiled into the binary)
//! - FHIR spec bundle files (search-parameters-*.json)
//! - Custom SearchParameter files in the data directory
//! - Stored SearchParameter resources (from database)
//! - Runtime configuration files

use std::path::Path;

use regex::Regex;
use serde_json::Value;

use crate::FhirVersion;

use super::errors::LoaderError;
use super::registry::{
    CompositeComponentDef, SearchParameterDefinition, SearchParameterSource, SearchParameterStatus,
};
use super::types::SearchParamType;

/// Transforms FHIRPath expressions to replace `as` operator/function with `ofType`.
///
/// Per FHIRPath spec, `as(type)` requires singleton input and throws an error for
/// collections with multiple items. However, many FHIR SearchParameter expressions
/// use `as` on paths that can return multiple values (e.g., `Observation.component.value`).
///
/// This function rewrites such expressions to use `ofType()` which properly filters
/// collections, making them compatible with strict FHIRPath evaluation.
///
/// Transformations:
/// - `(X as Type)` → `(X.ofType(Type))` (operator form)
/// - `X.as(Type)` → `X.ofType(Type)` (function form)
///
/// See: https://chat.fhir.org/#narrow/channel/179266-fhirpath/topic/FHIRPath.20Strictness.20in.20R4
pub(crate) fn transform_as_to_oftype(expression: &str) -> String {
    let operator_re = Regex::new(
        r"(\([^()]*\)|[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)"
    ).unwrap();

    let result = operator_re.replace_all(expression, |caps: &regex::Captures| {
        let path = &caps[1];
        let type_name = &caps[2];
        format!("{}.ofType({})", path, type_name)
    });

    let function_re = Regex::new(r"\.as\(([A-Za-z_][A-Za-z0-9_]*)\)").unwrap();
    let result = function_re.replace_all(&result, ".ofType($1)");

    result.into_owned()
}

/// Loader for SearchParameter definitions.
pub struct SearchParameterLoader {
    fhir_version: FhirVersion,
}

impl SearchParameterLoader {
    /// Creates a new loader for the specified FHIR version.
    pub fn new(fhir_version: FhirVersion) -> Self {
        Self { fhir_version }
    }

    /// Returns the FHIR version.
    pub fn version(&self) -> FhirVersion {
        self.fhir_version
    }

    /// Returns the spec filename for the configured FHIR version.
    #[allow(unreachable_patterns)]
    pub fn spec_filename(&self) -> &'static str {
        match self.fhir_version {
            #[cfg(feature = "R4")]
            FhirVersion::R4 => "search-parameters-r4.json",
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => "search-parameters-r4b.json",
            #[cfg(feature = "R5")]
            FhirVersion::R5 => "search-parameters-r5.json",
            #[cfg(feature = "R6")]
            FhirVersion::R6 => "search-parameters-r6.json",
            _ => "search-parameters-r4.json",
        }
    }

    /// Loads embedded minimal fallback parameters for the FHIR version.
    ///
    /// This returns only the essential Resource-level search parameters that
    /// should always be available as a fallback. For full FHIR spec compliance,
    /// use `load_from_spec_file()` to load the complete parameter set.
    pub fn load_embedded(&self) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        Ok(self.get_minimal_fallback_parameters())
    }

    /// Loads SearchParameter resources from a FHIR spec bundle file.
    ///
    /// Expects files in the format `search-parameters-{version}.json` in the
    /// specified data directory, where version is r4, r4b, r5, or r6.
    pub fn load_from_spec_file(
        &self,
        data_dir: &Path,
    ) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        let path = data_dir.join(self.spec_filename());
        let content =
            std::fs::read_to_string(&path).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
        let json: Value =
            serde_json::from_str(&content).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let mut params = Vec::new();
        let mut errors = Vec::new();

        // Handle Bundle format (expected from FHIR spec files)
        if let Some(entries) = json.get("entry").and_then(|e| e.as_array()) {
            for entry in entries {
                if let Some(resource) = entry.get("resource") {
                    if resource.get("resourceType").and_then(|t| t.as_str())
                        == Some("SearchParameter")
                    {
                        match self.parse_resource(resource) {
                            Ok(mut param) => {
                                param.source = SearchParameterSource::Embedded;
                                // Treat draft params from spec files as active
                                // (the FHIR spec uses "draft" for most standard params)
                                if param.status == SearchParameterStatus::Draft {
                                    param.status = SearchParameterStatus::Active;
                                }
                                params.push(param);
                            }
                            Err(e) => {
                                // Log but continue - don't fail on individual params
                                errors.push(e);
                            }
                        }
                    }
                }
            }
        }

        if !errors.is_empty() {
            tracing::warn!(
                "Skipped {} invalid SearchParameters while loading spec file: {:?}",
                errors.len(),
                path
            );
        }

        tracing::info!(
            "Loaded {} SearchParameters from spec file: {:?}",
            params.len(),
            path
        );

        Ok(params)
    }

    /// Returns the raw SearchParameter resources from the FHIR spec bundle,
    /// as stored-shape JSON.
    ///
    /// This is the seeding companion to
    /// [`load_from_spec_file`](Self::load_from_spec_file): the registry wants
    /// parsed definitions, but seeding storage wants the original resource
    /// JSON (ids, metadata, and fields the definition model drops). Only
    /// entries that also parse as definitions are returned, so storage never
    /// holds a parameter the registry would reject.
    pub fn load_spec_resources(&self, data_dir: &Path) -> Result<Vec<Value>, LoaderError> {
        let path = data_dir.join(self.spec_filename());
        let content =
            std::fs::read_to_string(&path).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
        let json: Value =
            serde_json::from_str(&content).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let mut resources = Vec::new();
        if let Some(entries) = json.get("entry").and_then(|e| e.as_array()) {
            for entry in entries {
                if let Some(resource) = entry.get("resource")
                    && resource.get("resourceType").and_then(|t| t.as_str())
                        == Some("SearchParameter")
                    && self.parse_resource(resource).is_ok()
                {
                    resources.push(resource.clone());
                }
            }
        }
        Ok(resources)
    }

    /// Renders a definition as a minimal FHIR SearchParameter resource.
    ///
    /// Used to seed storage with the embedded fallback parameters, which are
    /// constructed programmatically and have no source JSON. The resource id
    /// is the last segment of the canonical URL (`.../Resource-id` → id
    /// `Resource-id`), matching the spec bundle's convention.
    pub fn definition_to_fhir_resource(def: &SearchParameterDefinition) -> Value {
        let id = def.url.rsplit('/').next().unwrap_or(&def.code);
        let mut resource = serde_json::json!({
            "resourceType": "SearchParameter",
            "id": id,
            "url": def.url,
            "name": def.name.clone().unwrap_or_else(|| def.code.clone()),
            "status": def.status.to_fhir_status(),
            "code": def.code,
            "base": def.base,
            "type": def.param_type.to_string(),
            "expression": def.expression,
        });
        let fields = resource.as_object_mut().expect("literal object");
        if let Some(description) = &def.description {
            fields.insert("description".into(), Value::String(description.clone()));
        }
        if let Some(target) = &def.target {
            fields.insert(
                "target".into(),
                Value::Array(target.iter().cloned().map(Value::String).collect()),
            );
        }
        resource
    }

    /// Loads custom SearchParameter files from the data directory.
    ///
    /// Scans the data directory for JSON files that are not the standard
    /// FHIR spec bundles (search-parameters-*.json). These files can contain:
    /// - A single SearchParameter resource
    /// - An array of SearchParameter resources
    /// - A Bundle containing SearchParameter resources
    ///
    /// This allows organizations to add custom SearchParameters by placing
    /// JSON files in the data directory.
    pub fn load_custom_from_directory(
        &self,
        data_dir: &Path,
    ) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        self.load_custom_from_directory_with_files(data_dir)
            .map(|(params, _)| params)
    }

    /// Loads custom SearchParameter files from the data directory.
    ///
    /// Returns both the loaded parameters and the list of filenames that were loaded.
    pub fn load_custom_from_directory_with_files(
        &self,
        data_dir: &Path,
    ) -> Result<(Vec<SearchParameterDefinition>, Vec<String>), LoaderError> {
        let mut params = Vec::new();
        let mut loaded_files = Vec::new();
        let mut errors = Vec::new();

        // List of spec files to skip (loaded separately)
        let spec_files = [
            "search-parameters-r4.json",
            "search-parameters-r4b.json",
            "search-parameters-r5.json",
            "search-parameters-r6.json",
        ];

        let entries = match std::fs::read_dir(data_dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::debug!(
                    "Could not read data directory {}: {}",
                    data_dir.display(),
                    e
                );
                return Ok((params, loaded_files));
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }

            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if spec_files.contains(&filename.as_str()) {
                continue;
            }

            if path.is_dir() {
                continue;
            }

            match self.load_custom_file(&path) {
                Ok(mut file_params) => {
                    if !file_params.is_empty() {
                        tracing::debug!(
                            "Loaded {} custom SearchParameters from {}",
                            file_params.len(),
                            filename
                        );
                        params.append(&mut file_params);
                        loaded_files.push(filename);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load custom SearchParameter file {:?}: {}",
                        path,
                        e
                    );
                    errors.push(e);
                }
            }
        }

        if !errors.is_empty() {
            tracing::warn!(
                "Encountered {} errors while loading custom SearchParameters",
                errors.len()
            );
        }

        Ok((params, loaded_files))
    }

    /// Loads SearchParameters from a single custom file.
    fn load_custom_file(&self, path: &Path) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        let content = std::fs::read_to_string(path).map_err(|e| LoaderError::ConfigLoadFailed {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;

        let json: Value =
            serde_json::from_str(&content).map_err(|e| LoaderError::ConfigLoadFailed {
                path: path.display().to_string(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let mut params = self.load_from_json(&json)?;

        for param in &mut params {
            param.source = SearchParameterSource::Config;
        }

        Ok(params)
    }

    /// Loads SearchParameter resources from a JSON bundle or array.
    pub fn load_from_json(
        &self,
        json: &Value,
    ) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        let mut params = Vec::new();

        if let Some(entries) = json.get("entry").and_then(|e| e.as_array()) {
            for entry in entries {
                if let Some(resource) = entry.get("resource") {
                    if resource.get("resourceType").and_then(|t| t.as_str())
                        == Some("SearchParameter")
                    {
                        params.push(self.parse_resource(resource)?);
                    }
                }
            }
        } else if let Some(array) = json.as_array() {
            for item in array {
                if item.get("resourceType").and_then(|t| t.as_str()) == Some("SearchParameter") {
                    params.push(self.parse_resource(item)?);
                }
            }
        } else if json.get("resourceType").and_then(|t| t.as_str()) == Some("SearchParameter") {
            params.push(self.parse_resource(json)?);
        }

        Ok(params)
    }

    /// Loads parameters from a configuration file.
    pub fn load_config(
        &self,
        config_path: &Path,
    ) -> Result<Vec<SearchParameterDefinition>, LoaderError> {
        let content =
            std::fs::read_to_string(config_path).map_err(|e| LoaderError::ConfigLoadFailed {
                path: config_path.display().to_string(),
                message: e.to_string(),
            })?;

        let json: Value =
            serde_json::from_str(&content).map_err(|e| LoaderError::ConfigLoadFailed {
                path: config_path.display().to_string(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let mut params = self.load_from_json(&json)?;

        for param in &mut params {
            param.source = SearchParameterSource::Config;
        }

        Ok(params)
    }

    /// Parses a SearchParameter FHIR resource into a definition.
    pub fn parse_resource(
        &self,
        resource: &Value,
    ) -> Result<SearchParameterDefinition, LoaderError> {
        let url = resource
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoaderError::MissingField {
                field: "url".to_string(),
                url: None,
            })?
            .to_string();

        let code = resource
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoaderError::MissingField {
                field: "code".to_string(),
                url: Some(url.clone()),
            })?
            .to_string();

        let type_str = resource
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoaderError::MissingField {
                field: "type".to_string(),
                url: Some(url.clone()),
            })?;

        let param_type =
            type_str
                .parse::<SearchParamType>()
                .map_err(|_| LoaderError::InvalidResource {
                    message: format!("Unknown search parameter type: {}", type_str),
                    url: Some(url.clone()),
                })?;

        let raw_expression = resource
            .get("expression")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let expression = if raw_expression.contains(" as ") || raw_expression.contains(".as(") {
            transform_as_to_oftype(raw_expression)
        } else {
            raw_expression.to_string()
        };

        if expression.is_empty() && param_type != SearchParamType::Composite {
            if !code.starts_with('_') {
                return Err(LoaderError::MissingField {
                    field: "expression".to_string(),
                    url: Some(url),
                });
            }
        }

        let base: Vec<String> = resource
            .get("base")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let target: Option<Vec<String>> =
            resource
                .get("target")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });

        let status = resource
            .get("status")
            .and_then(|v| v.as_str())
            .and_then(SearchParameterStatus::from_fhir_status)
            .unwrap_or(SearchParameterStatus::Active);

        let component = self.parse_components(resource)?;

        let modifier: Option<Vec<String>> = resource
            .get("modifier")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });

        let comparator: Option<Vec<String>> = resource
            .get("comparator")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });

        Ok(SearchParameterDefinition {
            url,
            code,
            name: resource
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from),
            description: resource
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            param_type,
            expression,
            base,
            target,
            component,
            status,
            source: SearchParameterSource::Stored,
            modifier,
            multiple_or: resource.get("multipleOr").and_then(|v| v.as_bool()),
            multiple_and: resource.get("multipleAnd").and_then(|v| v.as_bool()),
            comparator,
            xpath: resource
                .get("xpath")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }

    /// Parses composite components from a SearchParameter resource.
    fn parse_components(
        &self,
        resource: &Value,
    ) -> Result<Option<Vec<CompositeComponentDef>>, LoaderError> {
        let components = match resource.get("component").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return Ok(None),
        };

        let mut result = Vec::new();
        for comp in components {
            let definition = comp
                .get("definition")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LoaderError::InvalidResource {
                    message: "Composite component missing definition".to_string(),
                    url: resource
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })?
                .to_string();

            let expression = comp
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            result.push(CompositeComponentDef {
                definition,
                expression,
            });
        }

        Ok(if result.is_empty() {
            None
        } else {
            Some(result)
        })
    }

    /// Returns minimal fallback search parameters for the FHIR version.
    ///
    /// This provides only the essential Resource-level parameters that should
    /// always work, used when spec files are unavailable.
    #[allow(clippy::vec_init_then_push)]
    fn get_minimal_fallback_parameters(&self) -> Vec<SearchParameterDefinition> {
        let mut params = Vec::new();

        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-id",
                "_id",
                SearchParamType::Token,
                "id",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-lastUpdated",
                "_lastUpdated",
                SearchParamType::Date,
                "meta.lastUpdated",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-tag",
                "_tag",
                SearchParamType::Token,
                "meta.tag",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-profile",
                "_profile",
                SearchParamType::Uri,
                "meta.profile",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-security",
                "_security",
                SearchParamType::Token,
                "meta.security",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        // `_source` completes the meta-level set. Its absence here is what made
        // #523 a `_source`-only defect: the other four were shadowed by these
        // embedded definitions, whose expressions carry no `Resource.` prefix,
        // so only `_source` kept the spec file's unevaluable
        // `Resource.meta.source` and went unindexed. Without it, a registry
        // built from the fallbacks alone also types `_source` by value-shape
        // heuristic (String), which would look for the value in the wrong
        // index column.
        params.push(
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-source",
                "_source",
                SearchParamType::Uri,
                "meta.source",
            )
            .with_base(vec!["Resource"])
            .with_source(SearchParameterSource::Embedded),
        );

        // SQL-on-FHIR canonical resources: `url` and `version` are not in the
        // base FHIR R4/R4B search-parameters bundle for ViewDefinition (which
        // first appears in R5+; its full parameter set, including these two,
        // now ships in every `search-parameters-r*.json` file via the
        // SQL-on-FHIR IG's own bundle, #570). Library is a core resource in
        // every version, so its `url`/`version` parameters already ship in
        // every spec file too. The SoF `$viewdefinition-run`,
        // `$viewdefinition-export`, and `$sqlquery-run` operations resolve
        // `viewReference`/`queryReference` canonical URLs via SearchProvider;
        // without these embedded fallbacks the resolver returns zero matches
        // whenever the spec file cannot be loaded (many integration tests
        // build a backend with no `data_dir` configured for exactly this
        // reason — deferring canonical-reference resolution entirely to a
        // spec file on disk would break them) and the capability statement's
        // `supportsCanonicalReference: true` claim isn't truthful.
        //
        // ViewDefinition's fallback URLs deliberately match the SQL-on-FHIR
        // IG's own canonicals exactly (`.../uv/sql-on-fhir/SearchParameter/
        // ViewDefinition-{url,version}`), not a fabricated `hl7.org/fhir`
        // one: this loads first (`load_embedded` runs before
        // `load_from_spec_file` in every backend's bootstrap), so when the
        // spec file *is* available its `ViewDefinition-url`/`-version`
        // entries collide on the exact same canonical URL and are rejected as
        // an exact `DuplicateUrl` — a harmless no-op, since the fallback
        // already registered that identical `(code, type, expression, base)`
        // definition under that same URL. Reusing the old, non-IG canonical
        // here (as this loop did before #570) would instead win the
        // `(base, code)` slot under the *wrong* URL and force the spec file's
        // real entry into a same-source `DuplicateCode` rejection, leaving
        // the IG canonical completely unregistered rather than merely
        // shadowed.
        for rt in ["ViewDefinition", "Library"] {
            let (url_canonical, version_canonical) = if rt == "ViewDefinition" {
                (
                    "http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-url"
                        .to_string(),
                    "http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-version"
                        .to_string(),
                )
            } else {
                (
                    format!("http://hl7.org/fhir/SearchParameter/{rt}-url"),
                    format!("http://hl7.org/fhir/SearchParameter/{rt}-version"),
                )
            };
            params.push(
                SearchParameterDefinition::new(url_canonical, "url", SearchParamType::Uri, "url")
                    .with_base(vec![rt])
                    .with_source(SearchParameterSource::Embedded),
            );
            params.push(
                SearchParameterDefinition::new(
                    version_canonical,
                    "version",
                    SearchParamType::Token,
                    "version",
                )
                .with_base(vec![rt])
                .with_source(SearchParameterSource::Embedded),
            );
        }

        params
    }
}

impl Default for SearchParameterLoader {
    fn default() -> Self {
        Self::new(FhirVersion::default_enabled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::registry::SearchParameterRegistry;

    #[test]
    fn test_load_embedded_minimal_fallback() {
        let loader = SearchParameterLoader::new(FhirVersion::default());
        let params = loader.load_embedded().unwrap();

        // Exact, not `<=`: an upper bound alone passes when fallbacks are
        // *lost*, which is the direction that breaks indexing. The six
        // `Resource`-level parameters below plus `url`/`version` for each of
        // `ViewDefinition` and `Library`.
        assert_eq!(
            params.len(),
            10,
            "Minimal fallback set changed; update this count and the names below: {:?}",
            params.iter().map(|p| &p.code).collect::<Vec<_>>()
        );

        // The whole `Resource`-level meta set, asserted by name. `_source` was
        // the one omission, and because these definitions shadow the spec
        // file's `Resource.`-prefixed expressions, its absence is what left
        // `_source` unindexed (#523). Losing any of them again is the same bug.
        for code in [
            "_id",
            "_lastUpdated",
            "_tag",
            "_profile",
            "_security",
            "_source",
        ] {
            // Exactly one definition per code, not merely at least one: a
            // feature-gated duplicate would shadow-conflict at registration and
            // `.find()` would happily accept it.
            let matches: Vec<_> = params.iter().filter(|p| p.code == code).collect();
            assert_eq!(
                matches.len(),
                1,
                "Should have exactly one {code} fallback, got {}",
                matches.len()
            );
            let def = matches[0];
            assert!(
                !def.expression.starts_with("Resource."),
                "{code} fallback expression must be resource-relative, got {:?}",
                def.expression
            );
        }

        let has_patient_specific = params
            .iter()
            .any(|p| p.code == "name" && p.base.contains(&"Patient".to_string()));
        assert!(
            !has_patient_specific,
            "Minimal fallback should not have Patient-specific params"
        );

        // Both Library and ViewDefinition keep a `url`/`version` fallback, so
        // canonical-reference resolution for `$sql-run`/`$sql-export` still
        // works when a backend has no `data_dir` configured (common in tests)
        // and the spec file never loads.
        for base in ["Library", "ViewDefinition"] {
            for code in ["url", "version"] {
                let def = params
                    .iter()
                    .find(|p| p.code == code && p.base.contains(&base.to_string()))
                    .unwrap_or_else(|| panic!("{base} should have a {code} fallback"));
                assert_eq!(def.expression, code, "{base} {code} fallback expression");
            }
        }

        // ViewDefinition's fallback URLs must match the SQL-on-FHIR IG's own
        // canonicals exactly — not a fabricated `hl7.org/fhir` one — so a
        // same-source collision with the spec file's real `ViewDefinition-url`
        // / `-version` entries (when the spec file does load) is an exact,
        // harmless `DuplicateUrl` rather than a `DuplicateCode` that would
        // leave the real IG canonical completely unregistered.
        for code in ["url", "version"] {
            let def = params
                .iter()
                .find(|p| p.code == code && p.base.contains(&"ViewDefinition".to_string()))
                .unwrap();
            assert_eq!(
                def.url,
                format!("http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-{code}")
            );
        }
    }

    #[test]
    fn test_parse_resource() {
        let loader = SearchParameterLoader::new(FhirVersion::default());

        let json = serde_json::json!({
            "resourceType": "SearchParameter",
            "url": "http://example.org/sp/test",
            "code": "test",
            "type": "string",
            "expression": "Patient.test",
            "base": ["Patient"],
            "status": "active"
        });

        let param = loader.parse_resource(&json).unwrap();

        assert_eq!(param.url, "http://example.org/sp/test");
        assert_eq!(param.code, "test");
        assert_eq!(param.param_type, SearchParamType::String);
        assert_eq!(param.expression, "Patient.test");
        assert!(param.base.contains(&"Patient".to_string()));
        assert_eq!(param.status, SearchParameterStatus::Active);
    }

    #[test]
    fn test_parse_resource_missing_field() {
        let loader = SearchParameterLoader::new(FhirVersion::default());

        let json = serde_json::json!({
            "resourceType": "SearchParameter",
            "code": "test",
            "type": "string"
        });

        let result = loader.parse_resource(&json);
        assert!(matches!(result, Err(LoaderError::MissingField { field, .. }) if field == "url"));
    }

    /// The workspace data directory holding `search-parameters-r4.json`.
    fn workspace_data_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    #[test]
    fn test_load_spec_resources_returns_stored_shape_json() {
        let loader = SearchParameterLoader::new(FhirVersion::R4);
        let resources = loader
            .load_spec_resources(&workspace_data_dir())
            .expect("load spec resources");

        assert!(
            resources.len() > 1300,
            "expected the full R4 spec set, got {}",
            resources.len()
        );
        // Every entry is a SearchParameter carrying its bundle id — that id is
        // what makes seeding idempotent (a duplicate `create` fails).
        for resource in &resources {
            assert_eq!(
                resource.get("resourceType").and_then(|t| t.as_str()),
                Some("SearchParameter")
            );
            assert!(
                resource.get("id").and_then(|id| id.as_str()).is_some(),
                "spec resource is missing an id: {resource}"
            );
        }
    }

    #[test]
    fn test_load_spec_resources_missing_dir_errors() {
        let loader = SearchParameterLoader::new(FhirVersion::R4);
        let result = loader.load_spec_resources(Path::new("/nonexistent/data/dir"));
        assert!(matches!(result, Err(LoaderError::ConfigLoadFailed { .. })));
    }

    #[test]
    fn test_definition_to_fhir_resource() {
        // A reference parameter with description and targets exercises both
        // optional-field branches.
        let mut def = SearchParameterDefinition::new(
            "http://example.org/fhir/SearchParameter/Patient-organization",
            "organization",
            SearchParamType::Reference,
            "Patient.managingOrganization",
        )
        .with_base(vec!["Patient"])
        .with_targets(vec!["Organization"]);
        def.description = Some("The organization managing the patient".to_string());

        let resource = SearchParameterLoader::definition_to_fhir_resource(&def);
        assert_eq!(
            resource.get("resourceType").and_then(|t| t.as_str()),
            Some("SearchParameter")
        );
        // Id is the last URL segment, matching the spec bundle convention.
        assert_eq!(
            resource.get("id").and_then(|i| i.as_str()),
            Some("Patient-organization")
        );
        assert_eq!(
            resource.get("code").and_then(|c| c.as_str()),
            Some("organization")
        );
        assert_eq!(
            resource.get("type").and_then(|t| t.as_str()),
            Some("reference")
        );
        assert_eq!(
            resource.get("description").and_then(|d| d.as_str()),
            Some("The organization managing the patient")
        );
        assert_eq!(
            resource.get("target").and_then(|t| t.as_array()),
            Some(&vec![Value::String("Organization".to_string())])
        );
        // The rendered resource round-trips back into a parseable definition.
        assert!(loader_can_parse(&resource));

        // A parameter with no description/target omits those fields entirely.
        let minimal = SearchParameterLoader::definition_to_fhir_resource(
            &SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Resource-id",
                "_id",
                SearchParamType::Token,
                "id",
            )
            .with_base(vec!["Resource"]),
        );
        assert_eq!(
            minimal.get("id").and_then(|i| i.as_str()),
            Some("Resource-id")
        );
        assert!(minimal.get("description").is_none());
        assert!(minimal.get("target").is_none());
        assert!(loader_can_parse(&minimal));
    }

    fn loader_can_parse(resource: &Value) -> bool {
        SearchParameterLoader::new(FhirVersion::R4)
            .parse_resource(resource)
            .is_ok()
    }

    #[test]
    fn test_transform_as_to_oftype() {
        assert_eq!(
            transform_as_to_oftype("Observation.value as CodeableConcept"),
            "Observation.value.ofType(CodeableConcept)"
        );

        assert_eq!(
            transform_as_to_oftype("(Observation.value as CodeableConcept)"),
            "(Observation.value.ofType(CodeableConcept))"
        );

        assert_eq!(
            transform_as_to_oftype(
                "(Observation.value as CodeableConcept) | (Observation.component.value as CodeableConcept)"
            ),
            "(Observation.value.ofType(CodeableConcept)) | (Observation.component.value.ofType(CodeableConcept))"
        );

        assert_eq!(
            transform_as_to_oftype("Patient.name.as(HumanName)"),
            "Patient.name.ofType(HumanName)"
        );

        assert_eq!(
            transform_as_to_oftype("Patient.name.family"),
            "Patient.name.family"
        );

        assert_eq!(
            transform_as_to_oftype("Observation.value.ofType(Quantity)"),
            "Observation.value.ofType(Quantity)"
        );
    }

    // ---- ViewDefinition SearchParameters survive the real loader/registry (#570) ----

    /// The 17 SQL-on-FHIR IG canonicals shipped in every
    /// `search-parameters-r*.json` file, keyed by their `code`.
    const VIEW_DEFINITION_PARAM_CODES: &[&str] = &[
        "context",
        "context-quantity",
        "context-type",
        "context-type-quantity",
        "context-type-value",
        "date",
        "description",
        "identifier",
        "jurisdiction",
        "name",
        "profile",
        "publisher",
        "resource",
        "status",
        "title",
        "url",
        "version",
    ];

    /// Builds a registry the same way a storage backend does at startup:
    /// embedded fallback first, then the version's spec file — see
    /// `SqliteBackend::new`'s two-step load.
    fn registry_with_spec_file(version: FhirVersion) -> SearchParameterRegistry {
        let loader = SearchParameterLoader::new(version);
        let mut registry = SearchParameterRegistry::new();
        for param in loader.load_embedded().unwrap() {
            // Ignore collisions the same way the backend does: this test
            // cares whether the *spec file's* ViewDefinition parameters
            // survive, not whether the minimal fallback set is collision-free.
            let _ = registry.register(param);
        }
        for param in loader
            .load_from_spec_file(&workspace_data_dir())
            .expect("spec file should load")
        {
            let _ = registry.register(param);
        }
        registry
    }

    /// Every ViewDefinition SearchParameter from the SQL-on-FHIR IG bundle
    /// must be registered, active, and reachable both by `(base, code)` and
    /// by its canonical URL — for every FHIR version the spec file ships for.
    fn assert_view_definition_params_registered(version: FhirVersion) {
        let registry = registry_with_spec_file(version);

        for code in VIEW_DEFINITION_PARAM_CODES {
            let by_code = registry
                .get_param("ViewDefinition", code)
                .unwrap_or_else(|| {
                    panic!("{version:?}: ViewDefinition-{code} not registered under its code")
                });
            assert!(
                by_code.status.is_usable(),
                "{version:?}: ViewDefinition-{code} is registered but not active"
            );
            assert_eq!(
                by_code.base,
                vec!["ViewDefinition".to_string()],
                "{version:?}: ViewDefinition-{code} has unexpected base"
            );

            let expected_url =
                format!("http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-{code}");
            let by_url = registry.get_by_url(&expected_url).unwrap_or_else(|| {
                panic!("{version:?}: {expected_url} not reachable via get_by_url")
            });
            // Same winning definition either way — not merely two different
            // definitions that both happen to exist.
            assert_eq!(
                by_code.url, by_url.url,
                "{version:?}: ViewDefinition-{code}"
            );
            assert_eq!(by_url.url, expected_url);
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn r4_registers_all_view_definition_search_params() {
        assert_view_definition_params_registered(FhirVersion::R4);
    }

    #[cfg(feature = "R4B")]
    #[test]
    fn r4b_registers_all_view_definition_search_params() {
        assert_view_definition_params_registered(FhirVersion::R4B);
    }

    #[cfg(feature = "R5")]
    #[test]
    fn r5_registers_all_view_definition_search_params() {
        assert_view_definition_params_registered(FhirVersion::R5);
    }

    #[cfg(feature = "R6")]
    #[test]
    fn r6_registers_all_view_definition_search_params() {
        assert_view_definition_params_registered(FhirVersion::R6);
    }

    /// The composite `context-type-quantity` / `context-type-value` params
    /// carry their declared components through the real loader, matching the
    /// upstream IG bundle's component wiring.
    #[cfg(feature = "R4")]
    #[test]
    fn view_definition_composite_params_carry_components() {
        let registry = registry_with_spec_file(FhirVersion::R4);

        let context_type_quantity = registry
            .get_param("ViewDefinition", "context-type-quantity")
            .expect("context-type-quantity registered");
        assert!(context_type_quantity.is_composite());
        let components = context_type_quantity.component.as_ref().unwrap();
        assert_eq!(components.len(), 2);
        assert!(components.iter().any(|c| c.definition
            == "http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-context-type"));
        assert!(components.iter().any(|c| c.definition
            == "http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/ViewDefinition-context-quantity"));
    }

    /// The `profile` parameter is a `reference` targeting `StructureDefinition`,
    /// carried through by the real loader — not just present, but typed right.
    #[cfg(feature = "R4")]
    #[test]
    fn view_definition_profile_param_targets_structure_definition() {
        let registry = registry_with_spec_file(FhirVersion::R4);
        let profile = registry
            .get_param("ViewDefinition", "profile")
            .expect("profile registered");
        assert_eq!(profile.param_type, SearchParamType::Reference);
        assert_eq!(
            profile.target.as_deref(),
            Some(["StructureDefinition".to_string()].as_slice())
        );
    }
}
