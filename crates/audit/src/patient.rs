//! Patient identity waterfall.
//!
//! Attempts to resolve a patient reference from the context of a FHIR
//! operation using a four-step waterfall:
//!
//! 1. Direct Patient resource access
//! 2. Compartment reference traversal (subject/patient fields in body)
//! 3. Search parameter extraction
//! 4. No patient (fallback)

/// Resolves patient identity from FHIR operation context.
pub struct PatientResolver;

impl PatientResolver {
    /// Attempt to resolve a patient reference from the operation context.
    ///
    /// Returns `Some("Patient/123")` if a patient can be identified, `None`
    /// otherwise.
    pub fn resolve(
        resource_type: &str,
        resource_id: Option<&str>,
        resource_body: Option<&serde_json::Value>,
        search_params: Option<&[(String, String)]>,
    ) -> Option<String> {
        // Step 1: Direct Patient resource access
        if resource_type == "Patient" {
            if let Some(id) = resource_id {
                return Some(format!("Patient/{id}"));
            }
        }

        // Step 2: Compartment — check subject/patient field in body
        if let Some(body) = resource_body {
            if let Some(ref_str) = extract_patient_reference(body) {
                return Some(ref_str);
            }
        }

        // Step 3: Search parameters — look for "patient" or "subject" param
        if let Some(params) = search_params {
            for (key, value) in params {
                if key == "patient" || key == "subject" {
                    let reference = if value.starts_with("Patient/") {
                        value.clone()
                    } else {
                        format!("Patient/{value}")
                    };
                    return Some(reference);
                }
            }
        }

        // Step 4: No patient found
        None
    }
}

/// Extract a Patient reference from a resource body's `subject` or `patient`
/// field.
fn extract_patient_reference(body: &serde_json::Value) -> Option<String> {
    for field in ["subject", "patient"] {
        if let Some(ref_val) = body
            .get(field)
            .and_then(|s| s.get("reference"))
            .and_then(|r| r.as_str())
        {
            if ref_val.starts_with("Patient/") {
                return Some(ref_val.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_patient_access() {
        let result = PatientResolver::resolve("Patient", Some("123"), None, None);
        assert_eq!(result.as_deref(), Some("Patient/123"));
    }

    #[test]
    fn test_patient_type_without_id() {
        // POST /Patient (create) — no id yet
        let result = PatientResolver::resolve("Patient", None, None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_compartment_subject_reference() {
        let body = serde_json::json!({
            "resourceType": "Observation",
            "subject": { "reference": "Patient/456" }
        });
        let result = PatientResolver::resolve("Observation", Some("obs-1"), Some(&body), None);
        assert_eq!(result.as_deref(), Some("Patient/456"));
    }

    #[test]
    fn test_compartment_patient_field() {
        let body = serde_json::json!({
            "resourceType": "Encounter",
            "patient": { "reference": "Patient/789" }
        });
        let result = PatientResolver::resolve("Encounter", Some("enc-1"), Some(&body), None);
        assert_eq!(result.as_deref(), Some("Patient/789"));
    }

    #[test]
    fn test_compartment_non_patient_reference_ignored() {
        let body = serde_json::json!({
            "resourceType": "Observation",
            "subject": { "reference": "Group/g1" }
        });
        let result = PatientResolver::resolve("Observation", Some("obs-1"), Some(&body), None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_search_param_patient() {
        let params = vec![("patient".to_string(), "999".to_string())];
        let result = PatientResolver::resolve("Observation", None, None, Some(&params));
        assert_eq!(result.as_deref(), Some("Patient/999"));
    }

    #[test]
    fn test_search_param_subject() {
        let params = vec![("subject".to_string(), "Patient/888".to_string())];
        let result = PatientResolver::resolve("Observation", None, None, Some(&params));
        assert_eq!(result.as_deref(), Some("Patient/888"));
    }

    #[test]
    fn test_no_patient_found() {
        let result = PatientResolver::resolve("Medication", Some("med-1"), None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_priority_patient_type_over_body() {
        // Patient resource access should take precedence over body fields
        let body = serde_json::json!({
            "resourceType": "Patient",
            "subject": { "reference": "Patient/other" }
        });
        let result = PatientResolver::resolve("Patient", Some("123"), Some(&body), None);
        assert_eq!(result.as_deref(), Some("Patient/123"));
    }

    #[test]
    fn test_priority_body_over_search_params() {
        let body = serde_json::json!({
            "resourceType": "Observation",
            "subject": { "reference": "Patient/from-body" }
        });
        let params = vec![("patient".to_string(), "from-param".to_string())];
        let result =
            PatientResolver::resolve("Observation", Some("obs-1"), Some(&body), Some(&params));
        assert_eq!(result.as_deref(), Some("Patient/from-body"));
    }
}
