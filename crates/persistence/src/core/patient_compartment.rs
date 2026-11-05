//! Patient-compartment membership decided from a resource's payload.
//!
//! Bulk export reads the primary store's payload rather than the search
//! index, so it keeps working when search is offloaded to a secondary backend
//! and the local index is empty. Compartment *search*, on the other hand,
//! decides membership from the index with the parameter set of the bundled
//! `CompartmentDefinition` (`helios_fhir::get_compartment_params`). This
//! matcher gives export the same parameter set, evaluated on the payload with
//! the search-parameter extractor, so `Patient/$export` and
//! `GET /Patient/{id}/*` agree on what belongs to a patient (#1122).

use std::collections::HashSet;
use std::sync::Arc;

use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::Value;

use crate::search::{
    IndexValue, SearchParameterDefinition, SearchParameterExtractor, SearchParameterRegistry,
};
use crate::types::strip_reference_version;

/// Membership test for one resource type in the Patient compartment.
pub struct PatientCompartmentMatcher {
    resource_type: String,
    params: Vec<Arc<SearchParameterDefinition>>,
    extractor: SearchParameterExtractor,
}

impl PatientCompartmentMatcher {
    /// Resolves the type's compartment parameters against `registry`.
    ///
    /// A parameter the registry does not know (or that is not a reference
    /// parameter) contributes nothing; a type with no resolvable parameter is
    /// [`is_empty`](Self::is_empty) and never a member.
    pub fn new(
        registry: Arc<RwLock<SearchParameterRegistry>>,
        version: FhirVersion,
        resource_type: &str,
    ) -> Self {
        let codes = helios_fhir::get_compartment_params(version, "Patient", resource_type);
        let params = {
            let registry = registry.read();
            codes
                .iter()
                .filter_map(|code| registry.get_param(resource_type, code))
                .filter(|param| param.param_type == crate::types::SearchParamType::Reference)
                .collect()
        };
        Self {
            resource_type: resource_type.to_string(),
            params,
            extractor: SearchParameterExtractor::new(registry),
        }
    }

    /// True when no compartment parameter resolved for the type.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// The parameter codes membership is decided on, in compartment order.
    pub fn param_codes(&self) -> impl Iterator<Item = &str> {
        self.params.iter().map(|param| param.code.as_str())
    }

    /// Whether `resource` references one of `patient_refs` (`Patient/{id}`)
    /// through any compartment parameter. A versioned reference
    /// (`Patient/{id}/_history/{vid}`) counts as its base.
    pub fn is_member(&self, resource: &Value, patient_refs: &HashSet<String>) -> bool {
        self.params.iter().any(|param| {
            self.extractor
                .extract_for_param(resource, param)
                .ok()
                .into_iter()
                .flatten()
                .any(|extracted| match &extracted.value {
                    IndexValue::Reference { reference, .. } => {
                        patient_refs.contains(strip_reference_version(reference))
                    }
                    _ => false,
                })
        })
    }

    /// The element paths under the resource root the compartment parameters
    /// read, for stores that filter on the payload with a path expression
    /// (dotted, e.g. `link.other` for `Patient.link.other`).
    ///
    /// Each union member of a parameter's FHIRPath expression that starts with
    /// the resource type is reduced to its leading element path; anything
    /// after a function call or a type operator is dropped, so the paths are a
    /// superset filter — a candidate found through them still has to pass
    /// [`is_member`](Self::is_member). Returns `None` when some parameter
    /// yields no usable path, in which case the caller cannot prefilter and
    /// must test every resource of the type.
    pub fn payload_paths(&self) -> Option<Vec<String>> {
        let mut paths = Vec::new();
        for param in &self.params {
            let mut found = false;
            for member in param.expression.split('|') {
                if let Some(path) = leading_element_path(member, &self.resource_type) {
                    found = true;
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
            if !found {
                return None;
            }
        }
        Some(paths)
    }
}

/// `Observation.subject.where(resolve() is Patient)` → `subject`;
/// `(Patient.link.other as Reference)` → `link.other`; a member rooted in
/// another type, or one with no element after the type, → `None`.
fn leading_element_path(member: &str, resource_type: &str) -> Option<String> {
    let member = member.trim().trim_start_matches('(');
    let rest = member.strip_prefix(resource_type)?.strip_prefix('.')?;
    let mut segments: Vec<&str> = Vec::new();
    for segment in rest.split('.') {
        let ident_end = segment
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(segment.len());
        let ident = &segment[..ident_end];
        // A segment that opens a call is a function, not an element: stop
        // before it. Any other cut (a space, a paren) ends the path after it.
        if segment[ident_end..].starts_with('(') {
            break;
        }
        if !ident.is_empty() {
            segments.push(ident);
        }
        if ident_end < segment.len() {
            break;
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchParameterLoader;
    use serde_json::json;

    /// The embedded fallback plus the spec file under the workspace `data/`
    /// directory, the same two sources a backend loads on startup.
    fn registry() -> Arc<RwLock<SearchParameterRegistry>> {
        let loader = SearchParameterLoader::new(FhirVersion::default());
        let mut registry = SearchParameterRegistry::new();
        for param in loader.load_embedded().unwrap() {
            let _ = registry.register(param);
        }
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        for param in loader.load_from_spec_file(&data_dir).unwrap() {
            let _ = registry.register(param);
        }
        Arc::new(RwLock::new(registry))
    }

    fn refs(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| format!("Patient/{id}")).collect()
    }

    #[test]
    fn a_type_outside_the_compartment_has_no_parameters() {
        let matcher =
            PatientCompartmentMatcher::new(registry(), FhirVersion::default(), "Organization");
        assert!(matcher.is_empty());
        assert!(!matcher.is_member(&json!({"resourceType": "Organization"}), &refs(&["p1"])));
    }

    #[test]
    fn membership_follows_every_compartment_parameter_not_only_subject() {
        let matcher = PatientCompartmentMatcher::new(
            registry(),
            FhirVersion::default(),
            "AllergyIntolerance",
        );
        let codes: Vec<&str> = matcher.param_codes().collect();
        assert_eq!(codes, ["patient", "recorder", "asserter"]);
        let recorded = json!({
            "resourceType": "AllergyIntolerance",
            "patient": {"reference": "Patient/other"},
            "recorder": {"reference": "Patient/p1"}
        });
        assert!(matcher.is_member(&recorded, &refs(&["p1"])));
        assert!(!matcher.is_member(&recorded, &refs(&["p2"])));
    }

    #[test]
    fn a_versioned_reference_counts_as_its_base() {
        let matcher =
            PatientCompartmentMatcher::new(registry(), FhirVersion::default(), "Observation");
        let observation = json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"text": "x"},
            "performer": [{"reference": "Patient/p1/_history/3"}]
        });
        assert!(matcher.is_member(&observation, &refs(&["p1"])));
    }

    #[test]
    fn a_patient_belongs_through_link() {
        let matcher = PatientCompartmentMatcher::new(registry(), FhirVersion::default(), "Patient");
        let linked = json!({
            "resourceType": "Patient",
            "id": "p9",
            "link": [{"other": {"reference": "Patient/p1"}, "type": "seealso"}]
        });
        assert!(matcher.is_member(&linked, &refs(&["p1"])));
        assert!(!matcher.is_member(
            &json!({"resourceType": "Patient", "id": "p1"}),
            &refs(&["p1"])
        ));
    }

    #[test]
    fn payload_paths_reduce_each_expression_to_its_element_path() {
        let matcher =
            PatientCompartmentMatcher::new(registry(), FhirVersion::default(), "Observation");
        let paths = matcher.payload_paths().unwrap();
        assert!(paths.contains(&"subject".to_string()), "{paths:?}");
        assert!(paths.contains(&"performer".to_string()), "{paths:?}");
        let patient = PatientCompartmentMatcher::new(registry(), FhirVersion::default(), "Patient");
        assert_eq!(patient.payload_paths().unwrap(), ["link.other"]);
    }

    #[test]
    fn leading_path_stops_at_functions_and_operators() {
        assert_eq!(
            leading_element_path(
                " Observation.subject.where(resolve() is Patient) ",
                "Observation"
            )
            .as_deref(),
            Some("subject")
        );
        assert_eq!(
            leading_element_path("(Patient.link.other as Reference)", "Patient").as_deref(),
            Some("link.other")
        );
        assert_eq!(
            leading_element_path("Condition.subject", "Observation"),
            None
        );
        assert_eq!(leading_element_path("Observation", "Observation"), None);
    }
}
