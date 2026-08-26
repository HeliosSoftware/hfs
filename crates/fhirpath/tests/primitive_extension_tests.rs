//! Tests for FHIRPath access to id/extension on FHIR primitive values.
//!
//! In FHIR JSON, primitive values can carry id/extension metadata via the
//! parallel `_field` sibling. These tests verify that FHIRPath expressions
//! such as `Patient.active.id` and `Patient.birthDate.extension.where(...)`
//! reach that metadata through the new `PrimitiveElement` carrier.

use chumsky::Parser;
use helios_fhir::FhirResource;
use helios_fhir::r4::{self, Boolean, Date, Extension, ExtensionValue};
use helios_fhirpath::evaluator::{EvaluationContext, evaluate};
use helios_fhirpath::parser::parser;
use helios_fhirpath_support::EvaluationResult;

fn eval(input: &str, context: &EvaluationContext) -> EvaluationResult {
    let expr = parser().parse(input).into_result().unwrap_or_else(|e| {
        panic!("Parser error for input '{}': {:?}", input, e);
    });
    evaluate(&expr, context, None).unwrap_or_else(|e| panic!("Eval error for '{}': {:?}", input, e))
}

fn ctx_with_patient(patient: r4::Patient) -> EvaluationContext {
    let resources = vec![FhirResource::R4(Box::new(r4::Resource::Patient(Box::new(
        patient,
    ))))];
    EvaluationContext::new(resources)
}

#[test]
fn primitive_id_access_returns_id_when_present() {
    // Patient.active = true with id "abc"
    let active = Boolean {
        id: Some("abc".to_string()),
        extension: None,
        value: Some(true),
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        active: Some(active),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    let result = eval("Patient.active.id", &ctx);
    assert_eq!(
        result,
        EvaluationResult::fhir_string("abc".to_string(), "id")
    );
}

#[test]
fn primitive_id_access_empty_when_absent() {
    let active = Boolean {
        id: None,
        extension: None,
        value: Some(true),
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        active: Some(active),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    let result = eval("Patient.active.id", &ctx);
    assert_eq!(result, EvaluationResult::Empty);
}

#[test]
fn primitive_extension_access_returns_extensions() {
    // Patient.birthDate = "1980-01-01" with extension url=...
    let ext = Extension {
        url: "http://example.org/qualifier".to_string().into(),
        value: Some(ExtensionValue::Code(r4::Code {
            id: None,
            extension: None,
            value: Some("approximate".to_string()),
        })),
        ..Default::default()
    };
    let birth_date = Date {
        id: None,
        extension: Some(vec![ext]),
        value: None, // No value, just extension - simulates `_birthDate` only case
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        birth_date: Some(birth_date),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    // When value is None but extension exists, the result is an Object with extension
    let result = eval("Patient.birthDate.extension.exists()", &ctx);
    assert_eq!(result, EvaluationResult::boolean(true));
}

#[test]
fn primitive_extension_with_value_carries_metadata() {
    // birthDate has both value AND extension/id metadata
    let ext = Extension {
        url: "http://example.org/qualifier".to_string().into(),
        value: Some(ExtensionValue::Code(r4::Code {
            id: None,
            extension: None,
            value: Some("approximate".to_string()),
        })),
        ..Default::default()
    };
    let birth_date = Date {
        id: Some("bd-1".to_string()),
        extension: Some(vec![ext]),
        value: Some(helios_fhir::PrecisionDate::from_ymd(1980, 1, 1)),
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        birth_date: Some(birth_date),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    // The id is reachable
    let id_result = eval("Patient.birthDate.id", &ctx);
    assert_eq!(
        id_result,
        EvaluationResult::fhir_string("bd-1".to_string(), "id")
    );

    // The extension list is reachable
    let ext_count = eval("Patient.birthDate.extension.count()", &ctx);
    assert_eq!(ext_count, EvaluationResult::integer(1));

    // The extension's url is reachable
    let url = eval("Patient.birthDate.extension.url", &ctx);
    assert_eq!(
        url,
        EvaluationResult::fhir_string("http://example.org/qualifier".to_string(), "uri")
    );

    // where() filtering on extension url
    let filtered = eval(
        "Patient.birthDate.extension.where(url = 'http://example.org/qualifier').count()",
        &ctx,
    );
    assert_eq!(filtered, EvaluationResult::integer(1));
}

#[test]
fn primitive_without_metadata_returns_empty() {
    // birthDate has only value, no id or extension
    let birth_date = Date {
        id: None,
        extension: None,
        value: Some(helios_fhir::PrecisionDate::from_ymd(1980, 1, 1)),
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        birth_date: Some(birth_date),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    assert_eq!(eval("Patient.birthDate.id", &ctx), EvaluationResult::Empty);
    assert_eq!(
        eval("Patient.birthDate.extension", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Patient.birthDate.extension.exists()", &ctx),
        EvaluationResult::boolean(false)
    );
}

#[test]
fn empty_extension_list_returns_empty() {
    // birthDate has empty extension Vec
    let birth_date = Date {
        id: Some("bd-1".to_string()),
        extension: Some(vec![]),
        value: Some(helios_fhir::PrecisionDate::from_ymd(1980, 1, 1)),
    };
    let patient = r4::Patient {
        id: Some("p1".to_string().into()),
        birth_date: Some(birth_date),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    // Empty extension list -> Empty (since it has no items)
    assert_eq!(
        eval("Patient.birthDate.extension", &ctx),
        EvaluationResult::Empty
    );
    // But id is still reachable
    assert_eq!(
        eval("Patient.birthDate.id", &ctx),
        EvaluationResult::fhir_string("bd-1".to_string(), "id")
    );
}

#[test]
fn scalar_operations_treat_an_extension_only_primitive_as_empty() {
    let given = r4::String {
        id: Some("given-1".to_string()),
        extension: None,
        value: None,
    };
    let patient = r4::Patient {
        name: Some(vec![r4::HumanName {
            given: Some(vec![given]),
            ..Default::default()
        }]),
        ..Default::default()
    };
    let ctx = ctx_with_patient(patient);

    // The FHIR Element remains navigable even without a primitive value.
    assert_eq!(
        eval("Patient.name.given.first().exists()", &ctx),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        eval("Patient.name.given.first().hasValue()", &ctx),
        EvaluationResult::boolean(false)
    );

    // Its implicit System primitive value is empty, so scalar operations
    // propagate {} instead of raising a type error.
    for expression in [
        "Patient.name.given.first().length()",
        "Patient.name.given.first().toChars()",
        "Patient.name.given.first().substring(0, 1)",
        "Patient.name.given.first().upper()",
        "Patient.name.given.first() < 'x'",
        "Patient.name.given.first().exists() and Patient.name.given.first().length() = 1",
    ] {
        assert_eq!(
            eval(expression, &ctx),
            EvaluationResult::Empty,
            "{expression}"
        );
    }
}
