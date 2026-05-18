//! Compartment-aware membership checks for `$viewdefinition-run` filtering.
//!
//! Backs `filter_resources_by_patient_and_group` with a real
//! [`CompartmentDefinition`]-driven scan instead of the hand-rolled
//! `(subject|patient)` allowlist the function used before
//! [audit item #3](../docs/spec-audit-viewdefinition-run.md).
//!
//! For each resource and each requested patient reference, the algorithm is:
//!
//! 1. Look up the search-parameter names that link the resource to the
//!    `Patient` compartment via `helios_fhir::{r4,r4b,r5,r6}::get_compartment_params`
//!    (code-generated from the spec `CompartmentDefinition-patient.json`).
//! 2. For each name, resolve the corresponding FHIRPath expression via the
//!    shared [`SearchParameterRegistry`].
//! 3. Evaluate the FHIRPath against the resource JSON and inspect the
//!    resulting `Reference` (or collection of References) for a match
//!    against any requested patient.
//!
//! [`CompartmentDefinition`]: https://hl7.org/fhir/compartmentdefinition.html

use helios_fhir::FhirVersion;
use helios_fhir::search::{SearchParameterLoader, SearchParameterRegistry};
use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use crate::SofError;

/// Lazily-loaded default registry per FHIR version, used when a caller
/// asks for the helios-sof default (via [`default_registry`]) rather than
/// supplying its own. Populated from
/// `{data_dir}/search-parameters-{version}.json` on first use; falls back
/// to the embedded minimal parameter set when the spec file isn't present.
#[cfg(feature = "R4")]
static DEFAULT_R4: OnceLock<Arc<SearchParameterRegistry>> = OnceLock::new();
#[cfg(feature = "R4B")]
static DEFAULT_R4B: OnceLock<Arc<SearchParameterRegistry>> = OnceLock::new();
#[cfg(feature = "R5")]
static DEFAULT_R5: OnceLock<Arc<SearchParameterRegistry>> = OnceLock::new();
#[cfg(feature = "R6")]
static DEFAULT_R6: OnceLock<Arc<SearchParameterRegistry>> = OnceLock::new();

/// Returns a process-wide default [`SearchParameterRegistry`] for the
/// given FHIR version.
///
/// The registry is loaded once from `{data_dir}/search-parameters-{ver}.json`
/// — `data_dir` defaults to the `HFS_DATA_DIR` env var (falling back to
/// `./data`) to match the HFS server's conventions. If the spec file is
/// missing or fails to parse, the registry is populated with the embedded
/// minimal parameter set — sufficient to compile but lacking the
/// resource-specific compartment search params, which means compartment
/// filtering on the inline FHIRPath path will fall back to "not in
/// compartment" for unrecognised resource types.
pub fn default_registry(fhir_version: FhirVersion) -> Arc<SearchParameterRegistry> {
    let slot = match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => &DEFAULT_R4,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => &DEFAULT_R4B,
        #[cfg(feature = "R5")]
        FhirVersion::R5 => &DEFAULT_R5,
        #[cfg(feature = "R6")]
        FhirVersion::R6 => &DEFAULT_R6,
        #[allow(unreachable_patterns)]
        _ => &DEFAULT_R4,
    };
    Arc::clone(slot.get_or_init(|| Arc::new(load_default_registry(fhir_version))))
}

fn data_dir_from_env() -> PathBuf {
    std::env::var("HFS_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./data"))
}

fn load_default_registry(fhir_version: FhirVersion) -> SearchParameterRegistry {
    let loader = SearchParameterLoader::new(fhir_version);
    let data_dir = data_dir_from_env();
    load_registry_from(&loader, &data_dir)
}

/// Builds a `SearchParameterRegistry` by reading
/// `{data_dir}/search-parameters-{version}.json` (per the loader's
/// `spec_filename`) and falling back to the embedded minimal parameter
/// set if the spec file is missing. Public so server bootstraps can
/// share the same loading policy.
pub fn load_registry_from(
    loader: &SearchParameterLoader,
    data_dir: &Path,
) -> SearchParameterRegistry {
    let mut registry = SearchParameterRegistry::new();

    match loader.load_from_spec_file(data_dir) {
        Ok(params) => {
            for p in params {
                let _ = registry.register(p);
            }
        }
        Err(e) => {
            tracing::warn!(
                "Falling back to embedded SearchParameter set (could not load spec file from {}: {})",
                data_dir.display(),
                e
            );
            if let Ok(params) = loader.load_embedded() {
                for p in params {
                    let _ = registry.register(p);
                }
            }
        }
    }

    registry
}

/// Returns the spec-driven list of search-parameter names that link
/// `resource_type` to the named compartment, for the given FHIR version.
///
/// Wraps the version-specific code-generated `get_compartment_params` so the
/// caller doesn't have to feature-gate or match on `FhirVersion`.
fn compartment_param_names(
    fhir_version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
) -> &'static [&'static str] {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::get_compartment_params(compartment_type, resource_type),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            helios_fhir::r4b::get_compartment_params(compartment_type, resource_type)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::get_compartment_params(compartment_type, resource_type),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::get_compartment_params(compartment_type, resource_type),
        #[allow(unreachable_patterns)]
        _ => &[],
    }
}

/// Returns `true` if `resource` is in the Patient compartment of any of the
/// given `patient_refs`, using the FHIR `CompartmentDefinition-patient`
/// spec data via the search-parameter registry.
///
/// `patient_refs` must already be canonicalised to `Patient/{id}` form (the
/// caller should run them through whatever normalisation it uses).
pub fn resource_in_patient_compartment(
    resource: &Value,
    patient_refs: &HashSet<String>,
    registry: &SearchParameterRegistry,
    fhir_version: FhirVersion,
) -> Result<bool, SofError> {
    let Some(resource_type) = resource.get("resourceType").and_then(|v| v.as_str()) else {
        return Ok(false);
    };

    // The Patient resource itself: in its own compartment iff its id matches.
    if resource_type == "Patient" {
        return Ok(resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(|id| patient_refs.contains(&format!("Patient/{}", id)))
            .unwrap_or(false));
    }

    let param_names = compartment_param_names(fhir_version, "Patient", resource_type);
    if param_names.is_empty() {
        return Ok(false);
    }

    // Build the FHIRPath evaluation context once for this resource.
    let fhir_resource = crate::parse_json_to_fhir_resource_pub(resource.clone(), fhir_version)?;
    let context = EvaluationContext::new(vec![fhir_resource]);

    for name in param_names {
        // Resolve the search param's FHIRPath expression. Skip unknown params
        // silently — the spec data may name params we don't have a
        // SearchParameter resource for in this version's bundle.
        let Some(def) = registry.get_param(resource_type, name) else {
            continue;
        };
        let expression = def.expression.trim();
        if expression.is_empty() {
            continue;
        }

        let result = match evaluate_expression(expression, &context) {
            Ok(r) => r,
            Err(_) => continue, // Don't fail the whole filter on one bad expression.
        };

        if any_reference_matches(&result, patient_refs) {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Walks an `EvaluationResult` looking for any FHIR `Reference` whose
/// `reference` string matches any entry in `targets`.
fn any_reference_matches(result: &EvaluationResult, targets: &HashSet<String>) -> bool {
    match result {
        EvaluationResult::Empty => false,
        EvaluationResult::Collection { items, .. } => {
            items.iter().any(|it| any_reference_matches(it, targets))
        }
        EvaluationResult::Object { map, .. } => {
            if let Some(reference) = map.get("reference") {
                if let Some(s) = extract_string(reference) {
                    if targets.contains(s) {
                        return true;
                    }
                }
            }
            false
        }
        EvaluationResult::String(s, _, _) => targets.contains(s.as_str()),
        _ => false,
    }
}

/// Extracts the inner string from `EvaluationResult::String` (the FHIR-id /
/// uri / canonical types). Returns `None` for any other variant.
fn extract_string(result: &EvaluationResult) -> Option<&str> {
    if let EvaluationResult::String(s, _, _) = result {
        Some(s.as_str())
    } else {
        None
    }
}

/// Resolves a set of group references to their member patient references.
///
/// Each group_ref must resolve to a Group resource in `inline_resources`.
/// Returns the union of `member.entity` Patient references across all
/// resolved groups. Unknown groups are silently skipped (the spec's SHOULD
/// for emitting an OperationOutcome is audit item #5 — separate fix).
pub fn resolve_group_members_to_patient_refs(
    group_refs: &[String],
    inline_resources: &[Value],
) -> HashSet<String> {
    let mut wanted: HashSet<String> = group_refs.iter().cloned().collect();
    let mut patient_refs = HashSet::new();

    for resource in inline_resources {
        if resource.get("resourceType").and_then(|v| v.as_str()) != Some("Group") {
            continue;
        }
        let Some(id) = resource.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let group_key_with_prefix = format!("Group/{}", id);
        if !wanted.contains(&group_key_with_prefix) && !wanted.contains(id) {
            continue;
        }
        wanted.remove(&group_key_with_prefix);
        wanted.remove(id);

        if let Some(members) = resource.get("member").and_then(|v| v.as_array()) {
            for member in members {
                if let Some(entity_ref) = member
                    .get("entity")
                    .and_then(|e| e.get("reference"))
                    .and_then(|r| r.as_str())
                {
                    if entity_ref.starts_with("Patient/") {
                        patient_refs.insert(entity_ref.to_string());
                    }
                }
            }
        }
    }

    patient_refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhir::search::{SearchParamType, SearchParameterDefinition};
    use serde_json::json;

    fn registry_with(defs: Vec<SearchParameterDefinition>) -> SearchParameterRegistry {
        let mut r = SearchParameterRegistry::new();
        for d in defs {
            r.register(d).unwrap();
        }
        r
    }

    #[cfg(feature = "R4")]
    #[test]
    fn patient_compartment_includes_allergyintolerance_via_patient_ref() {
        // AllergyIntolerance has Patient-compartment param names ["patient", "recorder", "asserter"]
        // in R4. We register `AllergyIntolerance.patient` to drive the lookup.
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/AllergyIntolerance-patient",
                "patient",
                SearchParamType::Reference,
                "AllergyIntolerance.patient",
            )
            .with_base(vec!["AllergyIntolerance"]),
        ]);

        let ai = json!({
            "resourceType": "AllergyIntolerance",
            "id": "ai-1",
            "patient": {"reference": "Patient/abc"},
        });

        let mut targets = HashSet::new();
        targets.insert("Patient/abc".to_string());

        assert!(
            resource_in_patient_compartment(&ai, &targets, &registry, FhirVersion::R4).unwrap()
        );
    }

    #[cfg(feature = "R4")]
    #[test]
    fn patient_resource_matches_only_its_own_id() {
        let registry = SearchParameterRegistry::new();
        let patient = json!({"resourceType": "Patient", "id": "abc"});

        let mut matching = HashSet::new();
        matching.insert("Patient/abc".to_string());
        let mut nonmatching = HashSet::new();
        nonmatching.insert("Patient/xyz".to_string());

        assert!(
            resource_in_patient_compartment(&patient, &matching, &registry, FhirVersion::R4)
                .unwrap()
        );
        assert!(
            !resource_in_patient_compartment(&patient, &nonmatching, &registry, FhirVersion::R4)
                .unwrap()
        );
    }

    #[cfg(feature = "R4")]
    #[test]
    fn unrelated_resource_is_not_in_compartment() {
        let registry = SearchParameterRegistry::new();
        // Library is not in the Patient compartment.
        let lib = json!({"resourceType": "Library", "id": "lib-1"});

        let mut targets = HashSet::new();
        targets.insert("Patient/abc".to_string());

        assert!(
            !resource_in_patient_compartment(&lib, &targets, &registry, FhirVersion::R4).unwrap()
        );
    }

    #[test]
    fn group_members_resolve_to_patient_refs() {
        let group = json!({
            "resourceType": "Group",
            "id": "g1",
            "member": [
                {"entity": {"reference": "Patient/a"}},
                {"entity": {"reference": "Patient/b"}},
                {"entity": {"reference": "Practitioner/p1"}},
            ]
        });

        let resolved = resolve_group_members_to_patient_refs(
            &["Group/g1".to_string()],
            std::slice::from_ref(&group),
        );
        assert!(resolved.contains("Patient/a"));
        assert!(resolved.contains("Patient/b"));
        assert!(!resolved.contains("Practitioner/p1"));
    }

    #[test]
    fn group_accepts_bare_id_and_typed_ref() {
        let group = json!({
            "resourceType": "Group",
            "id": "g2",
            "member": [{"entity": {"reference": "Patient/a"}}]
        });

        let typed = resolve_group_members_to_patient_refs(
            &["Group/g2".to_string()],
            std::slice::from_ref(&group),
        );
        assert!(typed.contains("Patient/a"));

        let bare = resolve_group_members_to_patient_refs(&["g2".to_string()], &[group]);
        assert!(bare.contains("Patient/a"));
    }
}
