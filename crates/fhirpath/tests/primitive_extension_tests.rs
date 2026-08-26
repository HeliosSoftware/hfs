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

// ---------------------------------------------------------------------------
// Value-consuming operations on primitives that carry extensions but no value
// (fhir-test-cases `primitivesWithoutValue`): the element is present as a node
// (`exists()` is true) but has no system value, so anything that consumes the
// value receives empty input and returns empty, and comparing it with anything
// is comparing with empty.
// ---------------------------------------------------------------------------

fn syllable_count_extension() -> Extension {
    Extension {
        url: "https://example.org/syllable-count".to_string().into(),
        value: Some(ExtensionValue::Code(r4::Code {
            id: None,
            extension: None,
            value: Some("five".to_string()),
        })),
        ..Default::default()
    }
}

/// Mirrors the r5 `patient-name-extensions.json` fixture: `given[0]` carries
/// only an extension (no value), `given[1]` is "James".
fn patient_with_valueless_given() -> EvaluationContext {
    let name = r4::HumanName {
        given: Some(vec![
            r4::String {
                id: None,
                extension: Some(vec![syllable_count_extension()]),
                value: None,
            },
            r4::String {
                id: None,
                extension: None,
                value: Some("James".to_string()),
            },
        ]),
        ..Default::default()
    };
    ctx_with_patient(r4::Patient {
        id: Some("p1".to_string().into()),
        name: Some(vec![name]),
        ..Default::default()
    })
}

fn assert_string_result(expr: &str, ctx: &EvaluationContext, expected: &str) {
    match eval(expr, ctx) {
        EvaluationResult::String(s, _, _) => assert_eq!(s, expected, "for {expr}"),
        other => panic!("expected string {expected:?} for {expr}, got {other:?}"),
    }
}

#[test]
fn valueless_primitive_string_functions_return_empty() {
    let ctx = patient_with_valueless_given();
    for expr in [
        "Patient.name.given.first().length()",
        "Patient.name.given.first().upper()",
        "Patient.name.given.first().lower()",
        "Patient.name.given.first().toChars()",
        "Patient.name.given.first().substring(0, 1)",
        "Patient.name.given.first().trim()",
        "Patient.name.given.first().contains('a')",
        "Patient.name.given.first().indexOf('a')",
        "Patient.name.given.first().startsWith('a')",
        "Patient.name.given.first().replace('a', 'b')",
        "Patient.name.given.first().matches('a')",
        "Patient.name.given.first().split(',')",
    ] {
        assert_eq!(
            eval(expr, &ctx),
            EvaluationResult::Empty,
            "expected empty for {expr}"
        );
    }
}

#[test]
fn valueless_primitive_as_string_function_argument_is_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("'abc'.startsWith(Patient.name.given.first())", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.indexOf(Patient.name.given.first())", &ctx),
        EvaluationResult::Empty
    );
}

#[test]
fn valueless_primitive_conversions_return_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first().toString()", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Patient.name.given.first().toInteger()", &ctx),
        EvaluationResult::Empty
    );
}

#[test]
fn comparison_with_valueless_primitive_is_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first() < 'x'", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Patient.name.given.first() >= 'x'", &ctx),
        EvaluationResult::Empty
    );
}

#[test]
fn invariant_pattern_with_valueless_primitive_is_empty() {
    // The pattern used by invariants that check the length of a name part:
    // exists() is true, but length() = 1 is empty, so the conjunction is empty.
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval(
            "Patient.name.given.first().exists() and Patient.name.given.first().length() = 1",
            &ctx
        ),
        EvaluationResult::Empty
    );
}

#[test]
fn equality_with_valueless_primitive_is_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first() = 'x'", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Patient.name.given.first() != 'x'", &ctx),
        EvaluationResult::Empty
    );
    // Both operands value-less: still comparing empty with empty.
    assert_eq!(
        eval(
            "Patient.name.given.first() = Patient.name.given.first()",
            &ctx
        ),
        EvaluationResult::Empty
    );
}

#[test]
fn equivalence_with_valueless_primitive_uses_empty_semantics() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first() ~ 'x'", &ctx),
        EvaluationResult::boolean(false)
    );
    assert_eq!(
        eval("Patient.name.given.first() !~ 'x'", &ctx),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        eval("Patient.name.given.first() ~ {}", &ctx),
        EvaluationResult::boolean(true)
    );
}

#[test]
fn membership_with_valueless_primitive_is_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first() in ('Peter' | 'James')", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval(
            "('Peter' | 'James') contains Patient.name.given.first()",
            &ctx
        ),
        EvaluationResult::Empty
    );
}

#[test]
fn arithmetic_with_valueless_primitive_propagates_empty() {
    let ctx = patient_with_valueless_given();
    assert_eq!(
        eval("Patient.name.given.first() + 'x'", &ctx),
        EvaluationResult::Empty
    );
    // `&` treats an empty operand as '' per the spec.
    assert_string_result("'Mr ' & Patient.name.given.first()", &ctx, "Mr ");
}

#[test]
fn join_skips_valueless_items() {
    let ctx = patient_with_valueless_given();
    assert_string_result("Patient.name.given.join(',')", &ctx, "James");
}

#[test]
fn valueless_decimal_primitive_has_no_value() {
    // Decimal primitives have a distinct runtime representation
    // (DecimalElement); an extension-only decimal must behave like any other
    // value-less primitive.
    let quantity = r4::Quantity {
        value: Some(r4::Decimal {
            id: None,
            extension: Some(vec![syllable_count_extension()]),
            value: None,
        }),
        ..Default::default()
    };
    let reference_range = r4::ObservationReferenceRange {
        low: Some(quantity),
        ..Default::default()
    };
    let observation = r4::Observation {
        reference_range: Some(vec![reference_range]),
        ..Default::default()
    };
    let resources = vec![FhirResource::R4(Box::new(r4::Resource::Observation(
        Box::new(observation),
    )))];
    let ctx = EvaluationContext::new(resources);

    assert_eq!(
        eval("Observation.referenceRange.low.value.exists()", &ctx),
        EvaluationResult::boolean(true)
    );
    assert_eq!(
        eval("Observation.referenceRange.low.value.hasValue()", &ctx),
        EvaluationResult::boolean(false)
    );
    assert_eq!(
        eval("Observation.referenceRange.low.value.abs()", &ctx),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("Observation.referenceRange.low.value + 1", &ctx),
        EvaluationResult::Empty
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
