//! Element-scoped profile constraints: `cpb-12`-style FHIRPath on `CapabilityStatement.rest.resource`.

use crate::harness::{r5_evaluator_for, run_validate_profile};
use fhir_validation::profile::types::{ExtractedElementRule, ExtractedProfile};
use fhir_validation::{
    InvariantDef, Severity, StructureDefinitionKind, TypeDerivationRule, Validator,
};
use helios_fhir::FhirResource;
use helios_fhir::r5::{CapabilityStatement, Resource};
use serde_json::json;

fn cpb12_profile() -> ExtractedProfile {
    ExtractedProfile {
        url: "http://example.org/StructureDefinition/cpb12-test".to_string(),
        version: None,
        name: None,
        title: None,
        resource_type: "CapabilityStatement".to_string(),
        base_definition: None,
        snapshot_base_version: None,
        kind: StructureDefinitionKind::Resource,
        derivation: TypeDerivationRule::Constraint,
        invariants: vec![],
        element_rules: vec![ExtractedElementRule {
            id: "CapabilityStatement.rest.resource".to_string(),
            path: "CapabilityStatement.rest.resource".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: vec![InvariantDef {
                key: "cpb-12".to_string(),
                severity: Severity::Error,
                path: "CapabilityStatement.rest.resource".to_string(),
                expression: "searchParam.select(name).isDistinct()".to_string(),
                human: "Search parameter names must be unique in the context of a resource."
                    .to_string(),
            }],
            value_constraint: None,
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        }],
    }
}

fn capability_statement_duplicate_search_param_json() -> serde_json::Value {
    json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "date": "2020-01-01",
        "kind": "instance",
        "fhirVersion": "5.0.0",
        "format": ["json"],
        "rest": [{
            "mode": "server",
            "resource": [{
                "type": "Patient",
                "interaction": [{ "code": "read" }],
                "searchParam": [
                    {
                        "name": "_id",
                        "type": "token",
                        "definition": "http://hl7.org/fhir/SearchParameter/Resource-id"
                    },
                    {
                        "name": "_id",
                        "type": "token",
                        "definition": "http://hl7.org/fhir/SearchParameter/Resource-id"
                    }
                ]
            }]
        }]
    })
}

fn capability_statement_distinct_search_param_json() -> serde_json::Value {
    json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "date": "2020-01-01",
        "kind": "instance",
        "fhirVersion": "5.0.0",
        "format": ["json"],
        "rest": [{
            "mode": "server",
            "resource": [{
                "type": "Patient",
                "interaction": [{ "code": "read" }],
                "searchParam": [
                    {
                        "name": "_id",
                        "type": "token",
                        "definition": "http://hl7.org/fhir/SearchParameter/Resource-id"
                    },
                    {
                        "name": "_lastUpdated",
                        "type": "date",
                        "definition": "http://hl7.org/fhir/SearchParameter/Resource-lastUpdated"
                    }
                ]
            }]
        }]
    })
}

#[test]
fn cpb12_fails_when_search_param_names_not_distinct() {
    let json = capability_statement_duplicate_search_param_json();
    let cs: CapabilityStatement =
        serde_json::from_value(json.clone()).expect("CapabilityStatement");
    let fhir = FhirResource::R5(Box::new(Resource::CapabilityStatement(Box::new(cs))));
    let evaluator = r5_evaluator_for(&fhir);
    let profile = cpb12_profile();
    let issues = run_validate_profile(
        &Validator::default(),
        &json,
        "CapabilityStatement",
        &profile,
        &evaluator,
    );
    assert!(
        issues
            .iter()
            .any(|i| i.source_invariant_key.as_deref() == Some("cpb-12")),
        "expected cpb-12 invariant failure: {issues:#?}"
    );
}

#[test]
fn cpb12_passes_when_search_param_names_distinct() {
    let json = capability_statement_distinct_search_param_json();
    let cs: CapabilityStatement =
        serde_json::from_value(json.clone()).expect("CapabilityStatement");
    let fhir = FhirResource::R5(Box::new(Resource::CapabilityStatement(Box::new(cs))));
    let evaluator = r5_evaluator_for(&fhir);
    let profile = cpb12_profile();
    let issues = run_validate_profile(
        &Validator::default(),
        &json,
        "CapabilityStatement",
        &profile,
        &evaluator,
    );
    assert!(
        !issues
            .iter()
            .any(|i| i.source_invariant_key.as_deref() == Some("cpb-12")),
        "unexpected cpb-12 issue: {issues:#?}"
    );
}
