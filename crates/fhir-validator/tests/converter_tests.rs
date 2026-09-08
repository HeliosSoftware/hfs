//! Golden tests for the StructureDefinition → FhirSchema converter.
//!
//! Each fixture in `tests/fixtures/structuredefinitions/` is a trimmed but
//! shape-faithful StructureDefinition; the expected FhirSchema is asserted
//! with exact deep equality on the serialized form, so every mapping rule is
//! pinned: shape (base.max → array), min → parent required (choice base name
//! for `foo[x]`), max 0 → parent excluded, choice expansion, discriminator →
//! pattern match, extension slicing → extensions sugar, contentReference →
//! elementReference, targetProfile → refers, binding/constraint carrying
//! (ele-1/ext-1 dropped on non-root elements), and primitive regex
//! extraction.

use helios_fhir_validator::converter::convert;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn load_sd(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/structuredefinitions")
        .join(name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

fn convert_to_value(name: &str) -> Value {
    let sd = load_sd(name);
    let conversion = convert(&sd).unwrap_or_else(|e| panic!("{name}: conversion failed: {e}"));
    serde_json::to_value(&conversion.schema).unwrap()
}

#[test]
fn converts_snapshot_resource() {
    let actual = convert_to_value("mini-patient.json");
    let expected = json!({
        "url": "http://hl7.org/fhir/StructureDefinition/Patient",
        "name": "Patient",
        "base": "http://hl7.org/fhir/StructureDefinition/DomainResource",
        "kind": "resource",
        "derivation": "specialization",
        "type": "Patient",
        "constraints": {
            "dom-2": {
                "expression": "contained.contained.empty()",
                "severity": "error",
                "human": "If the resource is contained in another resource, it SHALL NOT contain nested Resources"
            },
            "dom-6": {
                "expression": "text.`div`.exists()",
                "severity": "warning",
                "human": "A resource should have narrative for robust management"
            }
        },
        "elements": {
            "resourceType": { "type": "code" },
            "gender": {
                "type": "code",
                "binding": {
                    "valueSet": "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1",
                    "strength": "required"
                }
            },
            "name": { "type": "HumanName", "array": true },
            "deceased": { "choices": ["deceasedBoolean", "deceasedDateTime"] },
            "deceasedBoolean": { "type": "boolean", "choiceOf": "deceased" },
            "deceasedDateTime": { "type": "dateTime", "choiceOf": "deceased" },
            "link": {
                "type": "BackboneElement",
                "array": true,
                "required": ["other", "type"],
                "constraints": {
                    "pat-1": {
                        "expression": "other.exists()",
                        "severity": "error",
                        "human": "Contact must have details"
                    }
                },
                "elements": {
                    "other": {
                        "type": "Reference",
                        "refers": [
                            "http://hl7.org/fhir/StructureDefinition/Patient",
                            "http://hl7.org/fhir/StructureDefinition/RelatedPerson"
                        ]
                    },
                    "type": {
                        "type": "code",
                        "binding": {
                            "valueSet": "http://hl7.org/fhir/ValueSet/link-type|4.0.1",
                            "strength": "required"
                        }
                    }
                }
            }
        }
    });
    assert_eq!(actual, expected);
}

#[test]
fn converts_differential_profile() {
    let actual = convert_to_value("mini-profile.json");
    let expected = json!({
        "url": "http://example.org/StructureDefinition/mini-patient-profile",
        "name": "MiniPatientProfile",
        "base": "http://hl7.org/fhir/StructureDefinition/Patient",
        "kind": "resource",
        "derivation": "constraint",
        "type": "Patient",
        "required": ["birthDate", "identifier"],
        "excluded": ["gender"],
        "extensions": {
            "race": {
                "url": "http://example.org/StructureDefinition/race",
                "min": 1,
                "max": 1
            }
        },
        "elements": {
            "identifier": {
                "array": true,
                "min": 1,
                "slicing": {
                    "slices": {
                        "mrn": {
                            "match": {
                                "type": "pattern",
                                "value": { "system": "http://example.org/mrn" }
                            },
                            "min": 1,
                            "max": 1,
                            "schema": {
                                "required": ["system"],
                                "elements": {
                                    "system": { "fixed": "http://example.org/mrn" }
                                }
                            }
                        }
                    },
                    "rules": "open"
                }
            },
            "maritalStatus": {
                "pattern": { "coding": [{ "system": "http://x" }] }
            }
        }
    });
    assert_eq!(actual, expected);
}

#[test]
fn converts_content_reference() {
    let actual = convert_to_value("mini-questionnaire.json");
    let expected = json!({
        "url": "http://hl7.org/fhir/StructureDefinition/Questionnaire",
        "name": "Questionnaire",
        "base": "http://hl7.org/fhir/StructureDefinition/DomainResource",
        "kind": "resource",
        "derivation": "specialization",
        "type": "Questionnaire",
        "elements": {
            "resourceType": { "type": "code" },
            "item": {
                "type": "BackboneElement",
                "array": true,
                "required": ["linkId"],
                "elements": {
                    "linkId": { "type": "string" },
                    "item": {
                        "array": true,
                        "elementReference": ["Questionnaire", "elements", "item"]
                    }
                }
            }
        }
    });
    assert_eq!(actual, expected);
}

#[test]
fn converts_primitive_with_regex() {
    let actual = convert_to_value("primitive-string.json");
    let expected = json!({
        "url": "http://hl7.org/fhir/StructureDefinition/string",
        "name": "string",
        "kind": "primitive-type",
        "derivation": "specialization",
        "type": "string",
        "regex": "[ \\r\\n\\t\\S]+"
    });
    assert_eq!(actual, expected);
}

#[test]
fn carries_informational_mirrors_and_short_labels() {
    let sd = json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/StructureDefinition/Informational",
        "name": "Informational",
        "kind": "resource",
        "derivation": "specialization",
        "type": "Informational",
        "snapshot": { "element": [
            { "path": "Informational", "min": 0, "max": "*" },
            {
                "path": "Informational.status",
                "min": 1, "max": "1",
                "type": [{ "code": "code" }],
                "mustSupport": true,
                "isSummary": true,
                "short": "Current lifecycle state"
            },
            {
                "path": "Informational.note",
                "min": 0, "max": "1",
                "type": [{ "code": "string" }],
                "short": "Free-text remark"
            }
        ]}
    });
    let conversion = convert(&sd).expect("conversion");
    let value = serde_json::to_value(&conversion.schema).unwrap();

    let status = &value["elements"]["status"];
    assert_eq!(status["mustSupport"], json!(true));
    assert_eq!(status["summary"], json!(true));
    assert_eq!(status["short"], json!("Current lifecycle state"));
    assert_eq!(status["modifier"], Value::Null, "unset flags stay absent");

    let note = &value["elements"]["note"];
    assert_eq!(note["short"], json!("Free-text remark"));
    assert_eq!(note["mustSupport"], Value::Null);
}

// ── issue #424: Element.id-derived ids are `string`, not `id` ──────────────
//
// The FHIRPath `System.String` code carries a `structuredefinition-fhir-type`
// extension that HL7 populates inconsistently (R4B stamps `id` on every `.id`,
// R5 on `ElementDefinition.id`). Taken literally, the `id` regex
// `[A-Za-z0-9\-\.]{1,64}` rejects element ids with `[x]`, `:`, or > 64 chars —
// i.e. StructureDefinitions the spec itself publishes. The converter now keys
// the value-domain type on the base element: `Element.id` → `string`, while
// `Resource.id` and `Extension.url` keep honoring the extension.

const FHIR_TYPE_EXT: &str = "http://hl7.org/fhir/StructureDefinition/structuredefinition-fhir-type";
const SYSTEM_STRING: &str = "http://hl7.org/fhirpath/System.String";

/// Converts a one-field complex-type SD and returns that field's schema `type`.
/// The field derives from `base_path` and its single `type[]` carries `code`
/// plus a `structuredefinition-fhir-type` extension of `ext_value`.
fn field_type(field: &str, base_path: &str, code: &str, ext_value: &str) -> Value {
    let sd = json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/StructureDefinition/T",
        "name": "T", "kind": "complex-type", "derivation": "specialization", "type": "T",
        "snapshot": { "element": [
            { "path": "T", "min": 0, "max": "*" },
            {
                "path": format!("T.{field}"),
                "min": 0, "max": "1",
                "base": { "path": base_path, "min": 0, "max": "1" },
                "type": [{
                    "extension": [{ "url": FHIR_TYPE_EXT, "valueUrl": ext_value }],
                    "code": code
                }]
            }
        ]}
    });
    let conversion = convert(&sd).expect("conversion");
    serde_json::to_value(&conversion.schema).unwrap()["elements"][field]["type"].clone()
}

#[test]
fn element_id_is_string_despite_id_type_extension() {
    // The bug: R4B/R5 stamp `valueUrl: "id"` on Element.id. Must resolve to
    // `string` so the restrictive `id` regex is never applied to element ids.
    assert_eq!(
        field_type("id", "Element.id", SYSTEM_STRING, "id"),
        json!("string")
    );
}

#[test]
fn resource_id_keeps_id_type() {
    // Resource ids are the genuine constrained `id` token; the fix must NOT
    // relax them. A `Resource.id`-based field keeps honoring the extension.
    assert_eq!(
        field_type("id", "Resource.id", SYSTEM_STRING, "id"),
        json!("id")
    );
}

#[test]
fn extension_url_keeps_uri_type() {
    // Extension.url is `uri` in every version's fhir.schema.json; the fix must
    // not demote it to `string`.
    assert_eq!(
        field_type("url", "Extension.url", SYSTEM_STRING, "uri"),
        json!("uri")
    );
}

#[test]
fn element_id_override_is_independent_of_the_extension_value() {
    // Whatever the (inconsistent) extension claims, an Element.id-derived field
    // is `string`: `id` (the R4B/R5 bug) and `string` (R4/R6) both land there.
    assert_eq!(
        field_type("id", "Element.id", SYSTEM_STRING, "string"),
        json!("string")
    );
    assert_eq!(
        field_type("id", "Element.id", SYSTEM_STRING, "id"),
        json!("string")
    );
}

#[test]
fn multiple_types_without_choice_take_the_first_and_warn() {
    // A non-`[x]` element with more than one type is malformed but tolerated:
    // the converter warns and uses the first type. This also exercises the
    // `value_type_override` fall-through on the multi-type arm — the field is
    // not `Element.id`-derived, so the override yields None and the first
    // type's effective code (`string`) is used.
    let sd = json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/StructureDefinition/T",
        "name": "T", "kind": "complex-type", "derivation": "specialization", "type": "T",
        "snapshot": { "element": [
            { "path": "T", "min": 0, "max": "*" },
            {
                "path": "T.weird",
                "min": 0, "max": "1",
                "type": [{ "code": "string" }, { "code": "integer" }]
            }
        ]}
    });
    let conversion = convert(&sd).expect("conversion");
    let value = serde_json::to_value(&conversion.schema).unwrap();
    assert_eq!(value["elements"]["weird"]["type"], json!("string"));
    assert!(
        conversion
            .warnings
            .iter()
            .any(|w| w.contains("multiple types without [x]")),
        "expected a multiple-types warning, got: {:?}",
        conversion.warnings
    );
}

/// `ordered: true` slicing carries each slice's declaration ordinal as
/// `order` — without it the engine's ordered check has nothing to compare and
/// silently passes (`engine/slicing.rs`).
#[test]
fn converts_ordered_slicing_with_slice_ordinals() {
    let actual = convert_to_value("mini-ordered-slicing.json");
    let expected = json!({
        "url": "http://example.org/StructureDefinition/mini-ordered-slicing",
        "name": "MiniOrderedSlicing",
        "base": "http://hl7.org/fhir/StructureDefinition/Patient",
        "kind": "resource",
        "derivation": "constraint",
        "type": "Patient",
        "required": ["telecom"],
        "elements": {
            "telecom": {
                "array": true,
                "min": 1,
                "slicing": {
                    "slices": {
                        "phone": {
                            "match": {
                                "type": "pattern",
                                "value": { "system": "phone" }
                            },
                            "min": 1,
                            "max": 1,
                            "order": 0,
                            "schema": {
                                "required": ["system"],
                                "elements": {
                                    "system": { "fixed": "phone" }
                                }
                            }
                        },
                        "email": {
                            "match": {
                                "type": "pattern",
                                "value": { "system": "email" }
                            },
                            "min": 0,
                            "max": 2,
                            "order": 1,
                            "schema": {
                                "required": ["system"],
                                "elements": {
                                    "system": { "fixed": "email" }
                                }
                            }
                        }
                    },
                    "rules": "closed",
                    "ordered": true
                }
            }
        }
    });
    assert_eq!(actual, expected);
}

/// End-to-end: the ordinals the converter emits are what makes the engine's
/// ordered-slicing check fire. Without `order` the check reads `None` for
/// every slice and passes silently, so this is the test that would have caught
/// the gap.
#[test]
fn converted_ordered_slicing_is_enforced_by_the_engine() {
    use helios_fhir_validator::{
        FhirSchema, SchemaRegistry, UnknownProfilePolicy, ValidationOptions, Validator,
    };
    use std::sync::Arc;

    const PROFILE_URL: &str = "http://example.org/StructureDefinition/mini-ordered-slicing";
    const PATIENT_URL: &str = "http://hl7.org/fhir/StructureDefinition/Patient";

    let profile = convert(&load_sd("mini-ordered-slicing.json"))
        .expect("fixture converts")
        .schema;

    // Just enough of the base layer for the profile to resolve and walk.
    let named = |v: Value| -> FhirSchema { serde_json::from_value(v).expect("schema parses") };
    let patient = named(json!({
        "kind": "resource", "type": "Patient",
        "elements": {
            "resourceType": { "type": "code" },
            "telecom": { "type": "ContactPoint", "array": true }
        }
    }));
    let mut registry = SchemaRegistry::new();
    registry.insert_named("Patient", patient.clone());
    registry.insert_named(PATIENT_URL, patient);
    registry.insert_named(
        "ContactPoint",
        named(json!({ "kind": "complex-type", "elements": { "system": { "type": "code" } } })),
    );
    registry.insert_named("code", named(json!({ "kind": "primitive-type" })));
    registry.insert_named(PROFILE_URL, profile);

    let validator = Validator::new(Arc::new(registry));
    let opts = ValidationOptions {
        profiles: vec![PROFILE_URL.to_string()],
        use_meta_profiles: true,
        unknown_profile: UnknownProfilePolicy::Error,
        ..Default::default()
    };
    let kinds = |data: &Value| -> Vec<String> {
        validator
            .validate_sync(data, &opts)
            .errors
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    };

    // phone (order 0) before email (order 1): clean.
    assert_eq!(
        kinds(&json!({
            "resourceType": "Patient",
            "telecom": [{ "system": "phone" }, { "system": "email" }]
        })),
        Vec::<String>::new(),
    );

    // email before phone violates the declared order.
    assert_eq!(
        kinds(&json!({
            "resourceType": "Patient",
            "telecom": [{ "system": "email" }, { "system": "phone" }]
        })),
        vec!["slice-order".to_string()],
    );
}
