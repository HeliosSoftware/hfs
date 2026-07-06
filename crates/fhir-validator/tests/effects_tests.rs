//! Deferred-effects execution: stub-driven tests pinning constraint and
//! binding issue shapes, severity handling, suppression, dedup, and
//! fail-open/fail-closed terminology behavior.

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_fhir_validator::{
    CodedValue, ConstraintEvaluator, ConstraintOutcome, DeferredConstraint, EffectHandlers,
    FhirSchema, SchemaRegistry, Severity, TerminologyError, TerminologyProvider,
    ValidationOptions, Validator,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn registry() -> SchemaRegistry {
    let mut reg = SchemaRegistry::new();
    reg.insert_named(
        "code",
        serde_json::from_value::<FhirSchema>(json!({ "kind": "primitive-type", "type": "code" }))
            .unwrap(),
    );
    reg.insert_named(
        "string",
        serde_json::from_value::<FhirSchema>(json!({ "kind": "primitive-type", "type": "string" }))
            .unwrap(),
    );
    reg.insert_named(
        "Patient",
        serde_json::from_value::<FhirSchema>(json!({
            "constraints": {
                "pat-x": {
                    "expression": "name.exists()",
                    "severity": "error",
                    "human": "must have a name"
                },
                "pat-w": {
                    "expression": "telecom.exists()",
                    "severity": "warning",
                    "human": "should have telecom"
                },
                "pat-g": {
                    "expression": "language.exists()",
                    "severity": "guideline"
                }
            },
            "elements": {
                "resourceType": { "type": "string" },
                "name": { "type": "string" },
                "telecom": { "type": "string" },
                "gender": {
                    "type": "code",
                    "binding": {
                        "strength": "required",
                        "valueSet": "http://hl7.org/fhir/ValueSet/administrative-gender"
                    }
                },
                "maritalStatus": {
                    "type": "CodeableConcept",
                    "binding": {
                        "strength": "extensible",
                        "valueSet": "http://hl7.org/fhir/ValueSet/marital-status"
                    }
                }
            }
        }))
        .unwrap(),
    );
    reg.insert_named(
        "CodeableConcept",
        serde_json::from_value::<FhirSchema>(json!({
            "elements": { "text": { "type": "string" } }
        }))
        .unwrap(),
    );
    reg
}

/// Forces every selected constraint to the same outcome, recording calls.
struct ScriptedConstraints {
    outcome: fn(&DeferredConstraint<'_>) -> ConstraintOutcome,
    seen: Mutex<Vec<String>>,
}

impl ConstraintEvaluator for ScriptedConstraints {
    fn evaluate_all(
        &self,
        _resource: &Value,
        _version: FhirVersion,
        constraints: &[DeferredConstraint<'_>],
    ) -> Vec<ConstraintOutcome> {
        let mut seen = self.seen.lock().unwrap();
        constraints
            .iter()
            .map(|c| {
                seen.push(c.id.to_string());
                (self.outcome)(c)
            })
            .collect()
    }
}

/// Allow-list terminology stub.
struct AllowList(&'static [&'static str]);

#[async_trait]
impl TerminologyProvider for AllowList {
    async fn validate_code(
        &self,
        _value_set: &str,
        coded: &CodedValue,
    ) -> Result<bool, TerminologyError> {
        let code = match coded {
            CodedValue::Code(c) => c.clone(),
            CodedValue::Coding(v) => v["code"].as_str().unwrap_or_default().to_string(),
            CodedValue::CodeableConcept(v) => {
                v["coding"][0]["code"].as_str().unwrap_or_default().to_string()
            }
        };
        Ok(self.0.contains(&code.as_str()))
    }
}

/// Always-erroring terminology stub.
struct Down;

#[async_trait]
impl TerminologyProvider for Down {
    async fn validate_code(
        &self,
        _value_set: &str,
        _coded: &CodedValue,
    ) -> Result<bool, TerminologyError> {
        Err(TerminologyError("connection refused".to_string()))
    }
}

fn validator() -> Validator {
    Validator::new(Arc::new(registry()))
}

#[tokio::test]
async fn constraint_failure_shapes_and_severities() {
    let evaluator = ScriptedConstraints {
        outcome: |_| ConstraintOutcome::Failed,
        seen: Mutex::new(Vec::new()),
    };
    let handlers =
        EffectHandlers { constraints: Some(&evaluator), ..Default::default() };
    let resource = json!({ "resourceType": "Patient" });
    let errors = validator()
        .validate(&resource, FhirVersion::R4, &ValidationOptions::default(), &handlers)
        .await;

    // guideline (pat-g) is never evaluated.
    assert_eq!(evaluator.seen.lock().unwrap().as_slice(), &["pat-x", "pat-w"]);

    let as_json = serde_json::to_value(&errors).unwrap();
    assert_eq!(
        as_json,
        json!([
            {
                "type": "fhirpath-constraint",
                "path": "Patient",
                "message": "FHIRPath constraint pat-x error: must have a name",
                "constraint": "pat-x"
            },
            {
                "type": "fhirpath-constraint",
                "path": "Patient",
                "message": "FHIRPath constraint pat-w error: should have telecom",
                "constraint": "pat-w"
            }
        ])
    );
    assert_eq!(errors[0].severity, Severity::Error);
    assert_eq!(errors[1].severity, Severity::Warning);
}

#[tokio::test]
async fn constraints_suppressed_and_not_evaluable() {
    let evaluator = ScriptedConstraints {
        outcome: |_| ConstraintOutcome::NotEvaluable("parse error: boom".to_string()),
        seen: Mutex::new(Vec::new()),
    };
    let suppress = vec!["pat-w".to_string()];
    let handlers = EffectHandlers {
        constraints: Some(&evaluator),
        suppress_constraints: &suppress,
        ..Default::default()
    };
    let resource = json!({ "resourceType": "Patient" });
    let errors = validator()
        .validate(&resource, FhirVersion::R4, &ValidationOptions::default(), &handlers)
        .await;

    assert_eq!(evaluator.seen.lock().unwrap().as_slice(), &["pat-x"]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, Severity::Warning, "not-evaluable is a warning");
    assert_eq!(
        errors[0].message,
        "FHIRPath constraint pat-x could not be evaluated: parse error: boom"
    );
}

#[tokio::test]
async fn required_binding_pass_and_fail() {
    let allow = AllowList(&["male", "female"]);
    let handlers = EffectHandlers { terminology: Some(&allow), ..Default::default() };

    let ok = json!({ "resourceType": "Patient", "gender": "male" });
    let errors = validator()
        .validate(&ok, FhirVersion::R4, &ValidationOptions::default(), &handlers)
        .await;
    assert_eq!(errors, vec![]);

    let bad = json!({ "resourceType": "Patient", "gender": "zzz" });
    let errors = validator()
        .validate(&bad, FhirVersion::R4, &ValidationOptions::default(), &handlers)
        .await;
    let as_json = serde_json::to_value(&errors).unwrap();
    assert_eq!(
        as_json,
        json!([{
            "type": "terminology-binding",
            "path": "Patient.gender",
            "message": "Provided coded value 'zzz' does not pass validation against the following valueset: 'http://hl7.org/fhir/ValueSet/administrative-gender'",
            "binding": {
                "valueSet": "http://hl7.org/fhir/ValueSet/administrative-gender",
                "strength": "required"
            }
        }])
    );
}

#[tokio::test]
async fn non_required_bindings_are_not_checked() {
    let allow = AllowList(&[]); // everything would fail if checked
    let handlers = EffectHandlers { terminology: Some(&allow), ..Default::default() };
    let resource = json!({
        "resourceType": "Patient",
        "maritalStatus": { "text": "married" }
    });
    let errors = validator()
        .validate(&resource, FhirVersion::R4, &ValidationOptions::default(), &handlers)
        .await;
    assert_eq!(errors, vec![], "extensible bindings must not be enforced");
}

#[tokio::test]
async fn terminology_outage_fail_open_and_closed() {
    let down = Down;
    let resource = json!({ "resourceType": "Patient", "gender": "male" });

    let open = EffectHandlers { terminology: Some(&down), ..Default::default() };
    let errors = validator()
        .validate(&resource, FhirVersion::R4, &ValidationOptions::default(), &open)
        .await;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, Severity::Warning, "fail-open surfaces a warning");
    assert!(errors[0].message.contains("could not be performed"));

    let closed = EffectHandlers {
        terminology: Some(&down),
        terminology_fail_closed: true,
        ..Default::default()
    };
    let errors = validator()
        .validate(&resource, FhirVersion::R4, &ValidationOptions::default(), &closed)
        .await;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].severity, Severity::Error, "fail-closed escalates to an error");
}

#[tokio::test]
async fn no_handlers_means_structural_only() {
    let resource = json!({ "resourceType": "Patient", "gender": "zzz", "bogus": 1 });
    let errors = validator()
        .validate(
            &resource,
            FhirVersion::R4,
            &ValidationOptions::default(),
            &EffectHandlers::default(),
        )
        .await;
    // Only the structural issue; deferred constraint/binding work is inert
    // without handlers.
    let as_json = serde_json::to_value(&errors).unwrap();
    assert_eq!(
        as_json,
        json!([{
            "type": "unknown-element",
            "path": "Patient.bogus",
            "message": "bogus is unknown"
        }])
    );
}
