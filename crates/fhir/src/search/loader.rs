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

        // SQL-on-FHIR canonical resources: `url` and `version` are not in the
        // base FHIR R4/R4B search-parameters bundle for ViewDefinition (which
        // first appears in R5+). The SoF `$viewdefinition-run`,
        // `$viewdefinition-export`, and `$sqlquery-run` operations resolve
        // `viewReference`/`queryReference` canonical URLs via SearchProvider;
        // without these embedded fallbacks the resolver returns zero matches
        // and the capability statement's `supportsCanonicalReference: true`
        // claim isn't truthful.
        for rt in ["ViewDefinition", "Library"] {
            params.push(
                SearchParameterDefinition::new(
                    format!("http://hl7.org/fhir/SearchParameter/{rt}-url"),
                    "url",
                    SearchParamType::Uri,
                    "url",
                )
                .with_base(vec![rt])
                .with_source(SearchParameterSource::Embedded),
            );
            params.push(
                SearchParameterDefinition::new(
                    format!("http://hl7.org/fhir/SearchParameter/{rt}-version"),
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

    #[test]
    fn test_load_embedded_minimal_fallback() {
        let loader = SearchParameterLoader::new(FhirVersion::default());
        let params = loader.load_embedded().unwrap();

        assert!(!params.is_empty());
        assert!(params.len() <= 9, "Minimal fallback should have ~9 params");

        let has_id = params.iter().any(|p| p.code == "_id");
        assert!(has_id, "Should have _id parameter");

        let has_last_updated = params.iter().any(|p| p.code == "_lastUpdated");
        assert!(has_last_updated, "Should have _lastUpdated parameter");

        let has_patient_specific = params
            .iter()
            .any(|p| p.code == "name" && p.base.contains(&"Patient".to_string()));
        assert!(
            !has_patient_specific,
            "Minimal fallback should not have Patient-specific params"
        );
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
}
