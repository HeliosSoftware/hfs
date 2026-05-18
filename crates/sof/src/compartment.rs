//! Compartment-aware membership checks for `$viewdefinition-run` filtering.
//!
//! Backs `filter_resources_by_patient_and_group` with a real
//! [`CompartmentDefinition`]-driven scan (audit item #3). The lookup
//! tables are compiled in via
//! [`helios_fhir::compartment_expressions`] — no runtime data-file
//! dependency, so the filter works identically whether the server is
//! invoked from the workspace root, from a Docker container, or from a
//! release tarball.
//!
//! For each resource and each requested patient reference, the algorithm is:
//!
//! 1. Look up the spec-defined `(search-param-name, FHIRPath-expression)`
//!    pairs that link the resource to the `Patient` compartment via
//!    `helios_fhir::compartment_expressions::{r4,r4b,r5,r6}::get_compartment_param_expressions`
//!    (joined at code-gen time from `CompartmentDefinition-patient.json`
//!    and `search-parameters.json`).
//! 2. Evaluate each FHIRPath expression against the resource JSON.
//! 3. Inspect the result for a `Reference` whose `reference` string
//!    matches any requested patient.
//!
//! [`CompartmentDefinition`]: https://hl7.org/fhir/compartmentdefinition.html

use helios_fhir::FhirVersion;
use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};
use serde_json::Value;
use std::collections::HashSet;

use crate::SofError;

/// Returns the spec-driven `(search-param-name, FHIRPath-expression)` pairs
/// linking `resource_type` to the named compartment, for the given FHIR
/// version. Wraps the version-specific code-generated lookup.
fn compartment_param_expressions(
    fhir_version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
) -> &'static [(&'static str, &'static str)] {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            helios_fhir::compartment_expressions::r4::get_compartment_param_expressions(
                compartment_type,
                resource_type,
            )
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            helios_fhir::compartment_expressions::r4b::get_compartment_param_expressions(
                compartment_type,
                resource_type,
            )
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            helios_fhir::compartment_expressions::r5::get_compartment_param_expressions(
                compartment_type,
                resource_type,
            )
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            helios_fhir::compartment_expressions::r6::get_compartment_param_expressions(
                compartment_type,
                resource_type,
            )
        }
        #[allow(unreachable_patterns)]
        _ => &[],
    }
}

/// Returns `true` if `resource` is in the Patient compartment of any of the
/// given `patient_refs`, using the FHIR `CompartmentDefinition-patient`
/// spec data joined with the corresponding SearchParameter FHIRPath
/// expressions at code-gen time.
///
/// `patient_refs` must already be canonicalised to `Patient/{id}` form (the
/// caller should run them through whatever normalisation it uses).
pub fn resource_in_patient_compartment(
    resource: &Value,
    patient_refs: &HashSet<String>,
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

    let expressions = compartment_param_expressions(fhir_version, "Patient", resource_type);
    if expressions.is_empty() {
        return Ok(false);
    }

    // Build the FHIRPath evaluation context once for this resource.
    let fhir_resource = crate::parse_json_to_fhir_resource_pub(resource.clone(), fhir_version)?;
    let context = EvaluationContext::new(vec![fhir_resource]);

    for (_name, expression) in expressions {
        let result = match evaluate_expression(expression, &context) {
            Ok(r) => r,
            // Don't fail the whole filter if a single search-param expression
            // doesn't compile against our FHIRPath dialect — skip and try the
            // next one. (FHIR spec expressions sometimes use constructs the
            // evaluator doesn't support yet.)
            Err(_) => continue,
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
    use serde_json::json;

    #[cfg(feature = "R4")]
    #[test]
    fn patient_compartment_includes_allergyintolerance_via_patient_ref() {
        // AllergyIntolerance.patient is a top-level reference — works with the
        // old hardcoded allowlist too, kept here as a regression baseline.
        let ai = json!({
            "resourceType": "AllergyIntolerance",
            "id": "ai-1",
            "patient": {"reference": "Patient/abc"},
        });

        let mut targets = HashSet::new();
        targets.insert("Patient/abc".to_string());

        assert!(resource_in_patient_compartment(&ai, &targets, FhirVersion::R4).unwrap());
    }

    /// Audit-item-#3 regression: Appointment links to Patient via
    /// `Appointment.participant.actor` (nested). The old hardcoded
    /// allowlist couldn't see this because it only checked top-level
    /// `.subject` / `.patient`. With the compiled-in expression table
    /// the FHIRPath drives the lookup correctly.
    #[cfg(feature = "R4")]
    #[test]
    fn patient_compartment_includes_appointment_via_nested_participant_actor() {
        let appt_alice = json!({
            "resourceType": "Appointment",
            "id": "appt-alice",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/alice"}, "status": "accepted"}
            ]
        });
        let appt_bob = json!({
            "resourceType": "Appointment",
            "id": "appt-bob",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/bob"}, "status": "accepted"}
            ]
        });

        let mut targets = HashSet::new();
        targets.insert("Patient/alice".to_string());

        assert!(
            resource_in_patient_compartment(&appt_alice, &targets, FhirVersion::R4).unwrap(),
            "Appointment for Patient/alice must be in alice's compartment via participant.actor"
        );
        assert!(
            !resource_in_patient_compartment(&appt_bob, &targets, FhirVersion::R4).unwrap(),
            "Appointment for Patient/bob must NOT be in alice's compartment"
        );
    }

    #[cfg(feature = "R4")]
    #[test]
    fn patient_resource_matches_only_its_own_id() {
        let patient = json!({"resourceType": "Patient", "id": "abc"});

        let mut matching = HashSet::new();
        matching.insert("Patient/abc".to_string());
        let mut nonmatching = HashSet::new();
        nonmatching.insert("Patient/xyz".to_string());

        assert!(resource_in_patient_compartment(&patient, &matching, FhirVersion::R4).unwrap());
        assert!(!resource_in_patient_compartment(&patient, &nonmatching, FhirVersion::R4).unwrap());
    }

    #[cfg(feature = "R4")]
    #[test]
    fn unrelated_resource_is_not_in_compartment() {
        // Library is not in the Patient compartment.
        let lib = json!({"resourceType": "Library", "id": "lib-1"});

        let mut targets = HashSet::new();
        targets.insert("Patient/abc".to_string());

        assert!(!resource_in_patient_compartment(&lib, &targets, FhirVersion::R4).unwrap());
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
