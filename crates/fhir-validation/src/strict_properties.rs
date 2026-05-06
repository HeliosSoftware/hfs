//! Optional validation that JSON property names match an [`ExtractedProfile`](crate::profile::types::ExtractedProfile)
//! element tree (from `StructureDefinition.snapshot` / differential extraction).
//!
//! This addresses “no extra properties” at the JSON layer without changing serde’s
//! permissive deserialization in [`helios_fhir`](https://docs.rs/helios-fhir).
//!
//! # Limits
//!
//! - Requires a **complete** extracted profile for the resource type (typically the
//!   base `StructureDefinition` for that type).
//! - Choice elements (`[x]`) are accepted when the JSON uses a typed suffix
//!   (`valueQuantity`, `deceasedBoolean`, …) matching the stem before `[x]`.
//! - `extension` array entries use a conservative allowlist (`url`, `id`, `extension`,
//!   and any key starting with `value`).
//! - `contained` entries are validated when `resourceType` is present and a matching
//!   [`crate::profile::profile_registry::ProfileRegistry`] entry exists for the core
//!   canonical URL; otherwise only `resourceType` is required.

use crate::issue_code;
use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::types::ExtractedProfile;
use crate::validation_issue_detail::ValidationIssueDetailCode;
use crate::{Severity, ValidationIssue};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Canonical HL7 core profile URL for a resource type (e.g. `Patient` →
/// `http://hl7.org/fhir/StructureDefinition/Patient`).
pub fn hl7_core_structure_definition_url(resource_type: &str) -> String {
    format!("http://hl7.org/fhir/StructureDefinition/{resource_type}")
}

/// Look up the base `ExtractedProfile` for `resource_type` in `registry`.
///
/// Uses `ValidationConfig::base_structure_definition_url_overrides` on the runtime
/// [`Validator`](crate::Validator)
/// when present; otherwise the HL7 canonical URL from
/// [`hl7_core_structure_definition_url`]. Registry keys must match the URLs used when
/// loading `StructureDefinition`s (publisher URLs need an explicit override).
pub fn resolve_base_profile_in_registry<'a>(
    resource_type: &str,
    registry: &'a ProfileRegistry,
    url_overrides: &HashMap<String, String>,
) -> Option<&'a ExtractedProfile> {
    if let Some(u) = url_overrides.get(resource_type) {
        return registry.get(u.as_str());
    }
    let url = hl7_core_structure_definition_url(resource_type);
    registry.get(&url)
}

/// Build a map `logical_parent_path -> allowed direct child segment names` from
/// extracted element paths (e.g. `Patient.name.family` → `Patient.name` → `family`).
pub fn allowed_children_from_profile(
    profile: &ExtractedProfile,
) -> BTreeMap<String, BTreeSet<String>> {
    let rt = profile.resource_type.as_str();
    let mut m: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for rule in &profile.element_rules {
        let path = rule.path.trim();
        if path == rt {
            continue;
        }
        let Some(rest) = path.strip_prefix(rt) else {
            continue;
        };
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        if rest.is_empty() {
            continue;
        }
        let segments: Vec<&str> = rest.split('.').collect();
        for i in 0..segments.len() {
            let parent = if i == 0 {
                rt.to_string()
            } else {
                format!("{}.{}", rt, segments[..i].join("."))
            };
            let child = segments[i].to_string();
            m.entry(parent).or_default().insert(child);
        }
    }

    m
}

fn choice_stem(segment: &str) -> Option<&str> {
    segment.strip_suffix("[x]")
}

fn fhir_type_to_choice_suffix(code: &str) -> Option<String> {
    let mut chars = code.chars();
    let first = chars.next()?;
    if first.is_ascii_lowercase() {
        let mut out = String::new();
        out.push(first.to_ascii_uppercase());
        out.push_str(chars.as_str());
        Some(out)
    } else {
        Some(code.to_string())
    }
}

fn allowed_choice_keys_from_profile(
    profile: &ExtractedProfile,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rule in &profile.element_rules {
        let Some(stem) = rule.path.strip_suffix("[x]") else {
            continue;
        };
        if rule.type_constraints.is_empty() {
            continue;
        }
        let Some((parent, base)) = stem.rsplit_once('.') else {
            continue;
        };
        for tc in &rule.type_constraints {
            if let Some(suffix) = fhir_type_to_choice_suffix(tc.code.trim()) {
                out.entry(parent.to_string())
                    .or_default()
                    .insert(format!("{base}{suffix}"));
            }
        }
    }
    out
}

/// JSON keys present on any resource instance that are **not** listed as `Patient.*`
/// snapshot paths (the extractor does not emit a row for `resourceType`, etc.).
fn domain_resource_metadata_json_key(key: &str) -> bool {
    matches!(
        key,
        "resourceType"
            | "id"
            | "meta"
            | "implicitRules"
            | "language"
            | "text"
            | "contained"
            | "extension"
            | "modifierExtension"
    )
}

/// True if `json_key` is allowed given the SD child segments for `parent_logical`.
fn property_allowed(
    json_key: &str,
    allowed: &BTreeSet<String>,
    parent_logical: &str,
    allowed_choice_keys: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    if allowed.contains(json_key) {
        return true;
    }
    for seg in allowed {
        if let Some(stem) = choice_stem(seg) {
            if let Some(explicit) = allowed_choice_keys.get(parent_logical)
                && !explicit.is_empty()
            {
                if explicit.contains(json_key) {
                    return true;
                }
            } else if json_key.starts_with(stem) && json_key.len() > stem.len() {
                return true;
            }
        }
    }
    false
}

fn extension_object_keys_valid(map: &Map<String, Value>) -> bool {
    for key in map.keys() {
        let ok = matches!(key.as_str(), "url" | "id" | "extension") || key.starts_with("value");
        if !ok {
            return false;
        }
    }
    true
}

fn validate_object_keys(
    obj: &Map<String, Value>,
    logical_parent: &str,
    instance_prefix: &str,
    rt_root: &str,
    allowed: &BTreeMap<String, BTreeSet<String>>,
    allowed_choice_keys: &BTreeMap<String, BTreeSet<String>>,
    registry: Option<&ProfileRegistry>,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(set) = allowed.get(logical_parent) {
        for key in obj.keys() {
            if logical_parent == rt_root && domain_resource_metadata_json_key(key) {
                continue;
            }
            if property_allowed(key, set, logical_parent, allowed_choice_keys) {
                continue;
            }
            let inst = if instance_prefix.is_empty() {
                key.clone()
            } else {
                format!("{instance_prefix}.{key}")
            };
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some("Property is not defined for this element path".to_string()),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!(
                    "Unknown JSON property '{key}' at logical path '{logical_parent}' (instance '{inst}')."
                ),
                expression: None,
                fhir_path: logical_parent.to_string(),
                instance_path: Some(inst),
            });
        }
    }

    for (key, val) in obj.iter() {
        let next_instance = if instance_prefix.is_empty() {
            key.clone()
        } else {
            format!("{instance_prefix}.{key}")
        };

        if key == "extension" {
            if let Value::Array(items) = val {
                for (i, item) in items.iter().enumerate() {
                    if let Value::Object(em) = item {
                        if !extension_object_keys_valid(em) {
                            issues.push(ValidationIssue {
                                severity: Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some("Invalid Extension property set".to_string()),
                                expression_kind: None,
                                source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                                diagnostics: format!(
                                    "Extension at {next_instance}[{i}] has unknown or disallowed properties."
                                ),
                                expression: None,
                                fhir_path: format!("{logical_parent}.extension"),
                                instance_path: Some(format!("{next_instance}[{i}]")),
                            });
                        }
                        let ext_logical = format!("{logical_parent}.extension");
                        walk_value(
                            item,
                            ext_logical.as_str(),
                            &format!("{next_instance}[{i}]"),
                            rt_root,
                            allowed,
                            allowed_choice_keys,
                            registry,
                            issues,
                        );
                    }
                }
            }
            continue;
        }

        if key == "modifierExtension" {
            if let Value::Array(items) = val {
                for (i, item) in items.iter().enumerate() {
                    if let Value::Object(em) = item {
                        if !extension_object_keys_valid(em) {
                            issues.push(ValidationIssue {
                                severity: Severity::Error,
                                code: issue_code::STRUCTURE.to_string(),
                                summary: Some("Invalid Extension property set".to_string()),
                                expression_kind: None,
                                source_invariant_key: None,
                                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                                diagnostics: format!(
                                    "ModifierExtension at {next_instance}[{i}] has unknown properties."
                                ),
                                expression: None,
                                fhir_path: format!("{logical_parent}.modifierExtension"),
                                instance_path: Some(format!("{next_instance}[{i}]")),
                            });
                        }
                    }
                }
            }
            continue;
        }

        let next_logical = format!("{logical_parent}.{key}");
        if key == "contained" {
            if let Value::Array(items) = val {
                for (i, item) in items.iter().enumerate() {
                    let Value::Object(co) = item else {
                        continue;
                    };
                    let Some(Value::String(rt)) = co.get("resourceType") else {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            code: issue_code::STRUCTURE.to_string(),
                            summary: Some("Contained resource missing resourceType".to_string()),
                            expression_kind: None,
                            source_invariant_key: None,
                            detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                            diagnostics: format!(
                                "contained[{i}] at {next_instance} must declare resourceType for strict validation."
                            ),
                            expression: None,
                            fhir_path: logical_parent.to_string(),
                            instance_path: Some(format!("{next_instance}[{i}]")),
                        });
                        continue;
                    };
                    let url = hl7_core_structure_definition_url(rt);
                    if let Some(reg) = registry
                        && let Some(prof) = reg.get(&url)
                    {
                        walk_value(
                            item,
                            prof.resource_type.as_str(),
                            &format!("{next_instance}[{i}]"),
                            prof.resource_type.as_str(),
                            &allowed_children_from_profile(prof),
                            &allowed_choice_keys_from_profile(prof),
                            registry,
                            issues,
                        );
                    }
                }
            }
            continue;
        }

        walk_value(
            val,
            next_logical.as_str(),
            &next_instance,
            rt_root,
            allowed,
            allowed_choice_keys,
            registry,
            issues,
        );
    }
}

fn walk_value(
    val: &Value,
    logical_parent: &str,
    instance_prefix: &str,
    rt_root: &str,
    allowed: &BTreeMap<String, BTreeSet<String>>,
    allowed_choice_keys: &BTreeMap<String, BTreeSet<String>>,
    registry: Option<&ProfileRegistry>,
    issues: &mut Vec<ValidationIssue>,
) {
    match val {
        Value::Object(map) => {
            validate_object_keys(
                map,
                logical_parent,
                instance_prefix,
                rt_root,
                allowed,
                allowed_choice_keys,
                registry,
                issues,
            );
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_value(
                    item,
                    logical_parent,
                    &format!("{instance_prefix}[{i}]"),
                    rt_root,
                    allowed,
                    allowed_choice_keys,
                    registry,
                    issues,
                );
            }
        }
        _ => {}
    }
}

/// Validate JSON instance keys against an extracted `StructureDefinition` profile.
///
/// `root` must be a JSON object with `resourceType` matching `profile.resource_type`
/// (or validation returns issues).
pub fn validate_json_against_extracted_profile(
    root: &Value,
    profile: &ExtractedProfile,
    registry: Option<&ProfileRegistry>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let Value::Object(obj) = root else {
        return vec![ValidationIssue {
            severity: Severity::Error,
            code: issue_code::STRUCTURE.to_string(),
            summary: Some("Resource root must be a JSON object".to_string()),
            expression_kind: None,
            source_invariant_key: None,
            detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
            diagnostics: "Strict JSON property validation expects an object at the root.".into(),
            expression: None,
            fhir_path: profile.resource_type.clone(),
            instance_path: Some(String::new()),
        }];
    };

    let rt_expected = profile.resource_type.as_str();
    match obj.get("resourceType") {
        Some(Value::String(s)) if s == rt_expected => {}
        Some(Value::String(s)) => {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some("resourceType does not match strict profile type".to_string()),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: format!("resourceType is '{s}' but profile is for '{rt_expected}'."),
                expression: None,
                fhir_path: rt_expected.to_string(),
                instance_path: Some("resourceType".into()),
            });
        }
        _ => {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                code: issue_code::STRUCTURE.to_string(),
                summary: Some("Missing resourceType".to_string()),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::StructureInvalid),
                diagnostics: "Strict JSON property validation requires a string resourceType."
                    .into(),
                expression: None,
                fhir_path: rt_expected.to_string(),
                instance_path: Some("resourceType".into()),
            });
        }
    }

    let allowed = allowed_children_from_profile(profile);
    let allowed_choice_keys = allowed_choice_keys_from_profile(profile);
    validate_object_keys(
        obj,
        rt_expected,
        "",
        rt_expected,
        &allowed,
        &allowed_choice_keys,
        registry,
        &mut issues,
    );

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::types::{ExtractedElementRule, ExtractedProfile, ExtractedTypeConstraint};
    use serde_json::json;

    fn path_rule(path: &str) -> ExtractedElementRule {
        ExtractedElementRule {
            id: path.to_string(),
            path: path.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_top_level_property_is_error() {
        let profile = ExtractedProfile {
            resource_type: "Patient".into(),
            element_rules: vec![
                path_rule("Patient.id"),
                path_rule("Patient.meta"),
                path_rule("Patient.identifier"),
            ],
            ..Default::default()
        };
        let bad = json!({
            "resourceType": "Patient",
            "id": "x",
            "notARealField": 1
        });
        let issues = validate_json_against_extracted_profile(&bad, &profile, None);
        assert!(
            issues
                .iter()
                .any(|i| i.diagnostics.contains("notARealField")),
            "{issues:?}"
        );
    }

    #[test]
    fn invalid_choice_variant_key_is_rejected() {
        let profile = ExtractedProfile {
            resource_type: "Patient".into(),
            element_rules: vec![ExtractedElementRule {
                id: "Patient.multipleBirth[x]".into(),
                path: "Patient.multipleBirth[x]".into(),
                type_constraints: vec![
                    ExtractedTypeConstraint {
                        code: "boolean".into(),
                        ..Default::default()
                    },
                    ExtractedTypeConstraint {
                        code: "integer".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let bad = json!({
            "resourceType": "Patient",
            "multipleBirthString": true
        });
        let issues = validate_json_against_extracted_profile(&bad, &profile, None);
        assert!(
            issues
                .iter()
                .any(|i| i.diagnostics.contains("multipleBirthString")),
            "{issues:?}"
        );
    }
}
