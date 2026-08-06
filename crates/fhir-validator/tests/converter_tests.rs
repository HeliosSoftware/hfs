//! Golden tests for the StructureDefinition → FhirSchema converter.
//!
//! Each fixture in `tests/fixtures/structuredefinitions/` is a trimmed but
//! shape-faithful StructureDefinition; the expected FhirSchema is asserted
//! with exact deep equality on the serialized form, so every mapping rule is
//! pinned: shape (base.max → array), min → parent required (choice base name
//! for `foo[x]`), max 0 → parent excluded, choice expansion, discriminator →
//! pattern / type / profile / binding / exists / extension match, extension
//! slicing → extensions sugar, contentReference → elementReference,
//! targetProfile → refers,
//! binding/constraint carrying (ele-1/ext-1 dropped on non-root elements),
//! and primitive regex extraction.

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
fn converts_type_and_profile_slice_discriminators() {
    let actual = convert_to_value("slice-discriminators.json");
    let expected = json!({
        "url": "http://example.org/StructureDefinition/slice-discriminators",
        "name": "SliceDiscriminators",
        "base": "http://hl7.org/fhir/StructureDefinition/Bundle",
        "kind": "resource",
        "derivation": "constraint",
        "type": "Bundle",
        "elements": {
            "entry": {
                "slicing": {
                    "slices": {
                        "patient": {
                            "match": { "type": "type", "value": "Patient" },
                            "min": 1,
                            "max": 1,
                            "schema": {
                                "elements": {
                                    "resource": { "type": "Patient" }
                                },
                                "required": ["resource"]
                            }
                        },
                        "observation": {
                            "match": { "type": "type", "value": "Observation" },
                            "min": 0,
                            "max": 1,
                            "schema": {
                                "elements": {
                                    "resource": { "type": "Observation" }
                                }
                            }
                        }
                    },
                    "rules": "closed",
                    "ordered": false
                }
            },
            "identifier": {
                "slicing": {
                    "slices": {
                        "org": {
                            "match": {
                                "type": "profile",
                                "value": "http://example.org/StructureDefinition/org-ref"
                            },
                            "min": 0,
                            "max": 1,
                            "schema": {
                                "elements": {
                                    "assigner": {
                                        "type": "Reference",
                                        "refers": [
                                            "http://example.org/StructureDefinition/org-ref"
                                        ]
                                    }
                                }
                            }
                        }
                    },
                    "rules": "open"
                }
            }
        }
    });
    assert_eq!(actual, expected);
}

#[test]
fn converts_exists_and_extension_path_discriminators() {
    let actual = convert_to_value("exists-extension-discriminators.json");
    let expected = json!({
        "url": "http://example.org/StructureDefinition/exists-extension-discriminators",
        "name": "ExistsExtensionDiscriminators",
        "base": "http://hl7.org/fhir/StructureDefinition/Observation",
        "kind": "resource",
        "derivation": "constraint",
        "type": "Observation",
        "elements": {
            "component": {
                "slicing": {
                    "slices": {
                        "withValue": {
                            "match": {
                                "type": "exists",
                                "value": [{ "path": ["value"], "exists": true }]
                            },
                            "min": 1,
                            "schema": {
                                "required": ["value"],
                                "elements": {
                                    "value": { "choices": ["valueQuantity"] },
                                    "valueQuantity": { "type": "Quantity", "choiceOf": "value" }
                                }
                            }
                        },
                        "noValue": {
                            "match": {
                                "type": "exists",
                                "value": [{ "path": ["value"], "exists": false }]
                            },
                            "min": 0,
                            "schema": { "excluded": ["value"] }
                        }
                    },
                    "rules": "open"
                }
            },
            "identifier": {
                "slicing": {
                    "slices": {
                        "kindA": {
                            "match": {
                                "type": "extension",
                                "value": {
                                    "url": "http://example.org/ext-kind",
                                    "pattern": { "valueString": "A" }
                                }
                            },
                            "min": 0,
                            "max": 1,
                            "schema": {
                                "elements": {
                                    "extension": {
                                        "slicing": {
                                            "slices": {
                                                "kind": {
                                                    "match": {
                                                        "type": "pattern",
                                                        "value": { "url": "http://example.org/ext-kind" }
                                                    },
                                                    "min": 1,
                                                    "max": 1,
                                                    "schema": {
                                                        "elements": {
                                                            "url": { "fixed": "http://example.org/ext-kind" },
                                                            "value": { "choices": ["valueString"] },
                                                            "valueString": {
                                                                "type": "string",
                                                                "choiceOf": "value",
                                                                "fixed": "A"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            "rules": "open"
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "rules": "open"
                }
            }
        }
    });
    assert_eq!(actual, expected);
}

#[test]
fn converts_binding_slice_discriminator() {
    let actual = convert_to_value("binding-slice.json");
    let expected = json!({
        "url": "http://example.org/StructureDefinition/binding-slice",
        "name": "BindingSlice",
        "base": "http://hl7.org/fhir/StructureDefinition/Observation",
        "kind": "resource",
        "derivation": "constraint",
        "type": "Observation",
        "elements": {
            "category": {
                "array": true,
                "slicing": {
                    "slices": {
                        "laboratory": {
                            "match": { "type": "binding", "value": "laboratory" },
                            "min": 1,
                            "max": 1,
                            "schema": {
                                "binding": {
                                    "valueSet": "laboratory",
                                    "strength": "required"
                                }
                            }
                        }
                    },
                    "rules": "open"
                }
            }
        }
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
