//! JSON-based [`StructureDefinition`](https://hl7.org/fhir/structuredefinition.html) extraction.
//!
//! See [`super::extract`](crate::profile::extract) for differential vs snapshot behavior.

use crate::ValidationError;
use crate::issue_code::FHIR_JSON_VALUE;
use crate::profile::structure_definition_extract::StructureDefinitionExtractMessage as SdMsg;
use crate::profile::types::{
    ExtractedDiscriminatorType, ExtractedElementRule, ExtractedProfile,
    ExtractedSliceDiscriminator, ExtractedSlicing, ExtractedSlicingRules, ExtractedTypeConstraint,
    ExtractedValueConstraint,
};
use fhir_validation_types::{
    BindingDef, BindingStrength, BindingTargetKind, InvariantDef, Severity,
    StructureDefinitionKind, TypeDerivationRule, bindable_element_type_codes,
    binding_target_kind_from_element_type_codes, normalize_fhir_element_type_code,
};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Remove JSON `null` entries recursively (objects and arrays).
pub fn prune_json_nulls(value: Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Array(arr) => Value::Array(arr.into_iter().map(prune_json_nulls).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if v.is_null() {
                    continue;
                }
                out.insert(k, prune_json_nulls(v));
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// Extract [`ExtractedProfile`] from raw `StructureDefinition` JSON.
pub fn extract_structure_definition_profile_from_json(
    value: &Value,
) -> Result<ExtractedProfile, ValidationError> {
    let obj = value
        .as_object()
        .ok_or_else(|| ValidationError::from(SdMsg::JsonMustBeObject))?;

    // Typed `helios_fhir` resources serialized with `serde_json::to_value` may omit
    // `resourceType`; infer StructureDefinition when the JSON clearly describes one.
    let rt = match obj
        .get("resourceType")
        .or_else(|| obj.get("resource_type"))
        .and_then(|v| v.as_str())
    {
        Some(r) => r,
        None => {
            if obj.get("kind").is_some() && obj.get("differential").is_some() {
                "StructureDefinition"
            } else {
                return Err(ValidationError::from(SdMsg::MissingResourceType));
            }
        }
    };
    if rt != "StructureDefinition" {
        return Err(ValidationError::from(SdMsg::ExpectedResourceType {
            got: rt.to_string(),
        }));
    }

    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::from(SdMsg::UrlRequired))?
        .to_string();

    let kind_str = obj
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::from(SdMsg::KindRequired))?;
    let kind = StructureDefinitionKind::parse(kind_str).ok_or_else(|| {
        ValidationError::from(SdMsg::UnknownKind {
            value: kind_str.to_string(),
        })
    })?;

    let derivation_str = obj
        .get("derivation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::from(SdMsg::DerivationRequired))?;
    let derivation = TypeDerivationRule::parse(derivation_str).ok_or_else(|| {
        ValidationError::from(SdMsg::UnknownDerivation {
            value: derivation_str.to_string(),
        })
    })?;

    let resource_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::from(SdMsg::TypeRequired))?
        .to_string();

    let diff = obj
        .get("differential")
        .and_then(|d| d.get("element"))
        .and_then(|e| e.as_array())
        .ok_or_else(|| ValidationError::from(SdMsg::DifferentialElementMustBeArray))?;
    if diff.is_empty() {
        return Err(ValidationError::from(SdMsg::DifferentialElementNonEmpty));
    }

    let snapshot_map: HashMap<String, Value> = obj
        .get("snapshot")
        .and_then(|s| s.get("element"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let path = e.get("path")?.as_str()?;
                    Some((path.to_string(), e.clone()))
                })
                .collect()
        })
        .unwrap_or_default();

    let version = obj
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let base_definition = obj
        .get("baseDefinition")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut profile_invariants = Vec::new();
    let mut element_rules = Vec::new();

    for diff_el in diff {
        let diff_obj = diff_el
            .as_object()
            .ok_or_else(|| ValidationError::from(SdMsg::DifferentialElementEntryMustBeObject))?;
        let path = diff_obj
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ValidationError::from(SdMsg::DifferentialElementMissingPath))?;
        let id = diff_obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(path)
            .to_string();

        let resolved = snapshot_map
            .get(path)
            .cloned()
            .unwrap_or_else(|| diff_el.clone());

        if path == resource_type {
            if let Some(arr) = resolved.get("constraint").and_then(|c| c.as_array()) {
                for c in arr {
                    if let Some(inv) = parse_invariant(c, path)? {
                        profile_invariants.push(inv);
                    }
                }
            }
            continue;
        }

        element_rules.push(extract_element_rule(id, path.to_string(), &resolved)?);
    }

    Ok(ExtractedProfile {
        url,
        version,
        name,
        title,
        resource_type,
        base_definition,
        kind,
        derivation,
        invariants: profile_invariants,
        element_rules,
    })
}

fn extract_element_rule(
    id: String,
    path: String,
    resolved: &Value,
) -> Result<ExtractedElementRule, ValidationError> {
    let obj = resolved
        .as_object()
        .ok_or_else(|| ValidationError::from(SdMsg::ElementMustBeObject))?;

    Ok(ExtractedElementRule {
        id,
        path: path.clone(),
        min: obj.get("min").and_then(|v| v.as_u64()).map(|v| v as u32),
        max: obj
            .get("max")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        binding: extract_binding(path.clone(), resolved)?,
        constraints: extract_constraints(obj, &path)?,
        value_constraint: extract_value_constraint(resolved),
        type_constraints: extract_type_constraints(obj)?,
        slicing: extract_slicing(obj)?,
        slice_name: obj
            .get("sliceName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        max_length: obj
            .get("maxLength")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32),
        min_value: extract_min_value(obj),
        max_value: extract_max_value(obj),
        must_support: obj.get("mustSupport").and_then(|v| v.as_bool()),
        is_modifier: obj.get("isModifier").and_then(|v| v.as_bool()),
        is_modifier_reason: obj
            .get("isModifierReason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_binding_strength(s: &str) -> Option<BindingStrength> {
    match s {
        "required" => Some(BindingStrength::Required),
        "extensible" => Some(BindingStrength::Extensible),
        "preferred" => Some(BindingStrength::Preferred),
        "example" => Some(BindingStrength::Example),
        _ => None,
    }
}

fn parse_invariant_severity(s: &str) -> Option<Severity> {
    match s {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "information" => Some(Severity::Information),
        _ => None,
    }
}

fn parse_invariant(c: &Value, path: &str) -> Result<Option<InvariantDef>, ValidationError> {
    let Some(o) = c.as_object() else {
        return Ok(None);
    };
    let Some(key) = o.get("key").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let Some(expression) = o.get("expression").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let human = o
        .get("human")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let severity = o
        .get("severity")
        .and_then(|v| v.as_str())
        .and_then(parse_invariant_severity)
        .unwrap_or(Severity::Error);
    Ok(Some(InvariantDef {
        key: key.to_string(),
        severity,
        path: path.to_string(),
        expression: expression.to_string(),
        human,
    }))
}

fn extract_constraints(
    obj: &Map<String, Value>,
    path: &str,
) -> Result<Vec<InvariantDef>, ValidationError> {
    let Some(arr) = obj.get("constraint").and_then(|c| c.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for c in arr {
        if let Some(inv) = parse_invariant(c, path)? {
            out.push(inv);
        }
    }
    Ok(out)
}

fn extract_binding_name(b: &Map<String, Value>) -> Option<String> {
    let ext = b.get("extension")?.as_array()?;
    for e in ext {
        let o = e.as_object()?;
        if o.get("url")?.as_str()?
            == "http://hl7.org/fhir/StructureDefinition/elementdefinition-bindingName"
        {
            return o
                .get("valueString")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

fn extract_value_set(b: &Map<String, Value>) -> Result<String, ValidationError> {
    if let Some(v) = b.get("valueSet") {
        if let Some(s) = v.as_str() {
            return Ok(s.to_string());
        }
        if let Some(o) = v.as_object() {
            if let Some(s) = o.get(FHIR_JSON_VALUE).and_then(|x| x.as_str()) {
                return Ok(s.to_string());
            }
        }
    }
    if let Some(v) = b.get("valueSetCanonical").and_then(|v| v.as_str()) {
        return Ok(v.to_string());
    }
    if let Some(v) = b.get("valueSetUri").and_then(|v| v.as_str()) {
        return Ok(v.to_string());
    }
    Err(ValidationError::from(SdMsg::BindingMissingValueSet))
}

fn extract_type_codes(obj: &Map<String, Value>) -> Vec<String> {
    obj.get("type")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_object())
                .filter_map(|t| t.get("code").and_then(|v| v.as_str()))
                .map(normalize_fhir_element_type_code)
                .collect()
        })
        .unwrap_or_default()
}

fn extract_binding(path: String, resolved: &Value) -> Result<Option<BindingDef>, ValidationError> {
    let Some(obj) = resolved.as_object() else {
        return Ok(None);
    };
    let Some(bind) = obj.get("binding") else {
        return Ok(None);
    };
    let b = bind
        .as_object()
        .ok_or_else(|| ValidationError::from(SdMsg::BindingMustBeObject))?;
    let strength_str = b
        .get("strength")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::from(SdMsg::BindingStrengthRequired))?;
    let strength = parse_binding_strength(strength_str).ok_or_else(|| {
        ValidationError::from(SdMsg::UnknownBindingStrength {
            value: strength_str.to_string(),
        })
    })?;

    let value_set = extract_value_set(b)?;

    let binding_name = extract_binding_name(b);

    let type_codes = extract_type_codes(obj);
    let target_kind = if type_codes.is_empty() {
        BindingTargetKind::Code
    } else {
        binding_target_kind_from_element_type_codes(&type_codes)
    };

    let choice_type_codes = if target_kind == BindingTargetKind::Choice {
        let bindable = bindable_element_type_codes(&type_codes);
        if bindable.is_empty() {
            None
        } else {
            Some(bindable)
        }
    } else {
        None
    };

    Ok(Some(BindingDef {
        path,
        strength,
        value_set,
        binding_name,
        target_kind,
        choice_type_codes,
    }))
}

fn extract_type_constraints(
    obj: &Map<String, Value>,
) -> Result<Vec<ExtractedTypeConstraint>, ValidationError> {
    let Some(arr) = obj.get("type").and_then(|t| t.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for t in arr {
        let Some(tc) = t.as_object() else {
            continue;
        };
        let Some(code_raw) = tc.get("code").and_then(|v| v.as_str()) else {
            continue;
        };
        let code = normalize_fhir_element_type_code(code_raw);
        let profiles: Vec<String> = tc
            .get("profile")
            .map(|v| {
                if let Some(s) = v.as_str() {
                    vec![s.to_string()]
                } else if let Some(a) = v.as_array() {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        let target_profiles: Vec<String> = tc
            .get("targetProfile")
            .map(|v| {
                if let Some(s) = v.as_str() {
                    vec![s.to_string()]
                } else if let Some(a) = v.as_array() {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        let aggregation: Vec<String> = tc
            .get("aggregation")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let versioning = tc
            .get("versioning")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push(ExtractedTypeConstraint {
            code,
            profiles,
            target_profiles,
            aggregation,
            versioning,
        });
    }
    Ok(out)
}

fn extract_slicing(obj: &Map<String, Value>) -> Result<Option<ExtractedSlicing>, ValidationError> {
    let Some(s) = obj.get("slicing") else {
        return Ok(None);
    };
    let s = s
        .as_object()
        .ok_or_else(|| ValidationError::from(SdMsg::SlicingMustBeObject))?;
    let discriminators = s
        .get("discriminator")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    let o = d.as_object()?;
                    let path = o.get("path")?.as_str()?.to_string();
                    let t = o.get("type")?.as_str()?;
                    let discriminator_type = match t {
                        "value" => ExtractedDiscriminatorType::Value,
                        "exists" => ExtractedDiscriminatorType::Exists,
                        "pattern" => ExtractedDiscriminatorType::Pattern,
                        "type" => ExtractedDiscriminatorType::Type,
                        "profile" => ExtractedDiscriminatorType::Profile,
                        "position" => ExtractedDiscriminatorType::Position,
                        _ => return None,
                    };
                    Some(ExtractedSliceDiscriminator {
                        discriminator_type,
                        path,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let ordered = s.get("ordered").and_then(|v| v.as_bool()).unwrap_or(false);

    let rules_str = s.get("rules").and_then(|v| v.as_str()).unwrap_or("open");
    let rules = match rules_str {
        "closed" => ExtractedSlicingRules::Closed,
        "open" => ExtractedSlicingRules::Open,
        "openAtEnd" => ExtractedSlicingRules::OpenAtEnd,
        _ => ExtractedSlicingRules::Open,
    };

    Ok(Some(ExtractedSlicing {
        discriminators,
        ordered,
        rules,
    }))
}

fn extract_value_constraint(resolved: &Value) -> Option<ExtractedValueConstraint> {
    let obj = resolved.as_object()?;
    let mut fixed = None;
    let mut pattern = None;
    for (k, v) in obj {
        if k.starts_with("fixed") {
            fixed = Some(v.clone());
        } else if k.starts_with("pattern") {
            pattern = Some(v.clone());
        }
    }
    if let Some(v) = fixed {
        return Some(ExtractedValueConstraint::Fixed(v));
    }
    if let Some(v) = pattern {
        return Some(ExtractedValueConstraint::Pattern(v));
    }
    None
}

fn extract_min_value(obj: &Map<String, Value>) -> Option<Value> {
    for (k, v) in obj {
        if k.starts_with("minValue") {
            let mut m = Map::new();
            m.insert(k.clone(), v.clone());
            return Some(Value::Object(m));
        }
    }
    None
}

fn extract_max_value(obj: &Map<String, Value>) -> Option<Value> {
    for (k, v) in obj {
        if k.starts_with("maxValue") {
            let mut m = Map::new();
            m.insert(k.clone(), v.clone());
            return Some(Value::Object(m));
        }
    }
    None
}
