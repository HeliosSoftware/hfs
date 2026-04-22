//! Extract normalized validation models from FHIR `StructureDefinition` snapshots.
//!
//! This module converts raw FHIR definitions into generator-friendly
//! `TypeValidationModel`s used by `emit.rs`.
//!
//! Responsibilities:
//! - identify all generated Rust types implied by a single StructureDefinition
//!   snapshot, including nested backbone / element-derived types
//! - extract invariants declared on each generated type root
//! - extract direct child bindings from `ElementDefinition.binding`
//! - extract direct child field metadata used for recursive validation emission
//! - distinguish between:
//!   - `StructureKind`: the FHIR specification category (`resource`,
//!     `complex-type`, `primitive-type`, `logical`)
//!   - `ParentKind`: the inheritance/runtime family used by generated Rust code
//!     (`DomainResource`, `BackboneElement`, `Element`, etc.)
//!
//! This normalization step is intentionally separate from emission so code
//! generation can operate on a stable, FHIR-aware intermediate model.

use fhir_validation_types::{
    binding_target_kind_from_element_type_codes, normalize_fhir_element_type_code,
};
use serde_json::Value;
use std::collections::HashMap;

use crate::model::{
    BindingModel, BindingStrengthModel, BindingTargetKindModel, FieldModel, InvariantModel,
    ParentKind, SeverityModel, StructureKind, TypeValidationModel,
};
use crate::versions::FhirVersion;
use helios_fhir_gen::initial_fhir_model::{ElementDefinition, StructureDefinition};

/// Lookup index for resolving ancestor `StructureDefinition`s during inherited
/// invariant extraction.
#[derive(Debug, Clone)]
pub struct StructureDefinitionIndex<'a> {
    by_url: HashMap<String, &'a StructureDefinition>,
    by_type: HashMap<String, &'a StructureDefinition>,
}

/// Build a lookup index over the supplied StructureDefinitions.
///
/// The index supports resolution by canonical URL and by `type` name.
pub fn build_structure_definition_index<'a>(
    defs: &'a [StructureDefinition],
) -> StructureDefinitionIndex<'a> {
    let mut by_url = HashMap::new();
    let mut by_type = HashMap::new();

    for def in defs {
        let url = &def.url;
        by_url.insert(url.to_string(), def);
        let type_name = &def.r#type;
        by_type.insert(type_name.to_string(), def);
    }

    StructureDefinitionIndex { by_url, by_type }
}

/// Extract all validation models implied by a single StructureDefinition.
///
/// A single StructureDefinition can produce:
/// - one root validation model for the structure itself
/// - additional models for nested backbone/element-derived paths that become
///   generated Rust helper types
///
/// Returns `None` when the definition cannot be normalized into typed snapshot
/// elements.
///
/// This backward-compatible entry point extracts only direct invariants declared
/// on the current type/path. Use `extract_type_validation_models_with_index(...)`
/// to also merge inherited invariants from ancestor StructureDefinitions.
#[allow(dead_code)]
pub fn extract_type_validation_models(
    version: FhirVersion,
    def: &StructureDefinition,
) -> Option<Vec<TypeValidationModel>> {
    extract_type_validation_models_with_index(version, def, None)
}

/// Extract all validation models implied by a single StructureDefinition,
/// optionally merging inherited invariants through a StructureDefinition index.
pub fn extract_type_validation_models_with_index<'a>(
    version: FhirVersion,
    def: &'a StructureDefinition,
    index: Option<&StructureDefinitionIndex<'a>>,
) -> Option<Vec<TypeValidationModel>> {
    let def_json = serde_json::to_value(def).ok()?;
    let root_path = structure_root_path(def)?;
    let elements = snapshot_elements_typed(def)?;

    let type_paths = generated_type_paths(elements, root_path);
    let mut models = Vec::new();

    for path in type_paths {
        if let Some(model) = extract_type_validation_model_for_path(
            version, def, &def_json, elements, root_path, &path, index,
        ) {
            models.push(model);
        }
    }

    Some(models)
}

/// Build a normalized `TypeValidationModel` for one generated type path inside
/// a StructureDefinition snapshot.
///
/// This function determines:
/// - the Rust type name for the generated path
/// - whether the type should be emitted at all
/// - the path's `ParentKind` (inheritance/runtime behavior)
/// - the path's `StructureKind` (FHIR specification category)
/// - invariants declared exactly at this path
/// - direct child bindings and field metadata used for recursive emission
///
/// Root paths and nested paths are treated differently:
/// - the root path uses StructureDefinition metadata directly
/// - nested paths infer their classification from the typed element tree
fn extract_type_validation_model_for_path<'a>(
    version: FhirVersion,
    def: &'a StructureDefinition,
    def_json: &Value,
    elements: &[ElementDefinition],
    root_path: &str,
    path: &str,
    index: Option<&StructureDefinitionIndex<'a>>,
) -> Option<TypeValidationModel> {
    let rust_type = if path == root_path {
        root_rust_type_name(def_json, path)
    } else {
        rust_type_name(path)
    };

    if !should_emit_type_model(path, root_path, def_json, &rust_type) {
        return None;
    }

    let root_structure_kind = parse_structure_kind(def_json);
    let parent_kind = classify_parent_kind_for_path(path, root_structure_kind, def_json, elements);
    let structure_kind =
        classify_structure_kind_for_path(path, root_path, root_structure_kind, elements);

    let mut model = TypeValidationModel::new(rust_type, path.to_string(), parent_kind);
    model.structure_kind = structure_kind;
    model.structure_definition_url = json_str_field(def_json, "url").map(str::to_string);
    model.base_definition = json_str_field(def_json, "baseDefinition").map(str::to_string);

    let _ = (version, def);

    model.invariants =
        extract_invariants_with_inheritance(elements, path, root_path, def, parent_kind, index);
    model.bindings = extract_bindings_from_elements(elements, path);
    model.fields = extract_direct_fields_from_elements(elements, path);

    if let Some(base_name) = structure_base_name(def_json) {
        model
            .direct_supertypes
            .insert(model.rust_type.clone(), base_name.to_string());
    }

    Some(model)
}

/// Extract invariants declared exactly on the supplied generated type root path.
///
/// NOTE:
/// Some R4 invariants (notably dom-3) use older FHIRPath expressions with `as(...)`
/// over collections, which is not compatible with strict singleton semantics in
/// the FHIRPath spec. Newer specs (R5+) use `ofType(...)` instead.
///
/// We keep the original expression here, but this is the correct place to apply
/// normalization if needed (for example rewriting `.as(canonical)` to
/// `.ofType(canonical)`) so that validation stays compatible with official
/// HL7 invariants without changing FHIRPath engine semantics globally.
pub fn extract_invariants_from_elements(
    elements: &[ElementDefinition],
    root_path: &str,
) -> Vec<InvariantModel> {
    let mut out = Vec::new();

    for element in elements {
        let element_path = element.path.as_str();
        if element_path != root_path {
            continue;
        }

        let element_id = element
            .id
            .clone()
            .unwrap_or_else(|| element_path.to_string());

        let Some(constraints) = element.constraint.as_ref() else {
            continue;
        };

        for constraint in constraints {
            let Some(expression) = constraint.expression.as_deref() else {
                continue;
            };
            if expression.is_empty() {
                continue;
            }

            let normalized_expression = normalize_invariant_expression(&constraint.key, expression);

            out.push(InvariantModel {
                key: constraint.key.clone(),
                severity: parse_severity(Some(constraint.severity.as_str())),
                path: element_path.to_string(),
                expression: normalized_expression,
                human: constraint.human.clone(),
                source: constraint.source.clone(),
                element_id: element_id.clone(),
            });
        }
    }

    out
}

fn normalize_invariant_expression(key: &str, expression: &str) -> String {
    if key != "dom-3" {
        return expression.to_string();
    }

    expression
        .replace(".as(canonical)", ".ofType(canonical)")
        .replace(".as(uri)", ".ofType(uri)")
        .replace(".as(url)", ".ofType(url)")
}

/// Extract direct invariants for the current generated type path and merge
/// inherited invariants from ancestor StructureDefinitions when an index is
/// available.
fn extract_invariants_with_inheritance<'a>(
    elements: &[ElementDefinition],
    path: &str,
    root_path: &str,
    def: &'a StructureDefinition,
    parent_kind: ParentKind,
    index: Option<&StructureDefinitionIndex<'a>>,
) -> Vec<InvariantModel> {
    let mut out = extract_invariants_from_elements(elements, path);

    let Some(index) = index else {
        return out;
    };

    let inherited = if path == root_path {
        inherited_invariants_for_root(def, path, index)
    } else {
        inherited_invariants_for_nested_path(path, parent_kind, index)
    };

    for invariant in inherited {
        if !out.iter().any(|existing| existing.key == invariant.key) {
            out.push(invariant);
        }
    }

    out
}

/// Walk the `baseDefinition` chain for a root StructureDefinition and collect
/// rebased root-level invariants from each ancestor.
fn inherited_invariants_for_root<'a>(
    def: &'a StructureDefinition,
    rebased_path: &str,
    index: &StructureDefinitionIndex<'a>,
) -> Vec<InvariantModel> {
    let mut out = Vec::new();
    let mut current = def
        .base_definition
        .as_deref()
        .and_then(|url| index.by_url.get(url).copied());

    while let Some(sd) = current {
        out.extend(extract_root_invariants_from_structure_definition(
            sd,
            rebased_path,
        ));
        current = sd
            .base_definition
            .as_deref()
            .and_then(|url| index.by_url.get(url).copied());
    }

    out
}

/// Collect inherited invariants for a nested generated path by seeding the walk
/// from the corresponding core base type implied by `ParentKind`.
fn inherited_invariants_for_nested_path<'a>(
    rebased_path: &str,
    parent_kind: ParentKind,
    index: &StructureDefinitionIndex<'a>,
) -> Vec<InvariantModel> {
    let start_type = match parent_kind {
        ParentKind::BackboneElement => Some("BackboneElement"),
        ParentKind::Element => Some("Element"),
        ParentKind::DomainResource => Some("DomainResource"),
        ParentKind::Resource => Some("Resource"),
        _ => None,
    };

    let mut out = Vec::new();
    let mut current = start_type.and_then(|type_name| index.by_type.get(type_name).copied());

    while let Some(sd) = current {
        out.extend(extract_root_invariants_from_structure_definition(
            sd,
            rebased_path,
        ));
        current = sd
            .base_definition
            .as_deref()
            .and_then(|url| index.by_url.get(url).copied());
    }

    out
}

/// Extract root-level invariants from a StructureDefinition and rebase them to
/// the current generated type path.
fn extract_root_invariants_from_structure_definition(
    def: &StructureDefinition,
    rebased_path: &str,
) -> Vec<InvariantModel> {
    let Some(elements) = snapshot_elements_typed(def) else {
        return Vec::new();
    };

    let Some(root_element) = elements.first() else {
        return Vec::new();
    };

    let Some(constraints) = root_element.constraint.as_ref() else {
        return Vec::new();
    };

    let mut out = Vec::new();

    for constraint in constraints {
        let Some(expression) = constraint.expression.as_deref() else {
            continue;
        };
        if expression.is_empty() {
            continue;
        }

        let normalized_expression = normalize_invariant_expression(&constraint.key, expression);

        out.push(InvariantModel {
            key: constraint.key.clone(),
            severity: parse_severity(Some(constraint.severity.as_str())),
            path: rebased_path.to_string(),
            expression: normalized_expression,
            human: constraint.human.clone(),
            source: constraint.source.clone(),
            element_id: rebased_path.to_string(),
        });
    }

    out
}

/// Extract direct child bindings declared under the supplied generated type root.
///
/// Only direct child elements are considered here so generated validation code
/// can apply bindings relative to the current focus type rather than scanning
/// the whole snapshot repeatedly.
///
/// Bindings are taken from `ElementDefinition.binding`, and only bindable target
/// types currently supported by the validator are retained.
pub fn extract_bindings_from_elements(
    elements: &[ElementDefinition],
    root_path: &str,
) -> Vec<BindingModel> {
    let mut out = Vec::new();

    for element in elements {
        let element_path = element.path.as_str();
        if !is_direct_child(root_path, element_path) {
            continue;
        }

        let Ok(element_json) = serde_json::to_value(element) else {
            continue;
        };

        let Some(binding_obj) = element_json.get("binding") else {
            continue;
        };

        let Some(value_set) = json_str_field(binding_obj, "valueSet") else {
            continue;
        };
        if value_set.is_empty() {
            continue;
        }

        let type_codes = element_type_codes(&element_json);
        let element_id = element
            .id
            .clone()
            .unwrap_or_else(|| element_path.to_string());
        let bindable_type_codes = type_codes
            .iter()
            .filter(|code| {
                matches!(
                    code.as_str(),
                    "code"
                        | "string"
                        | "uri"
                        | "Coding"
                        | "CodeableConcept"
                        | "CodeableReference"
                        | "Quantity"
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        let is_choice_binding = element_path.ends_with("[x]");

        out.push(BindingModel {
            path: element_path.to_string(),
            strength: parse_binding_strength(json_str_field(binding_obj, "strength")),
            value_set: value_set.to_string(),
            binding_name: extract_binding_name_from_extensions(binding_obj),
            description: json_str_field(binding_obj, "description").map(str::to_string),
            target_kind: binding_target_kind(&type_codes),
            element_id,
            element_path: element_path.to_string(),
            type_codes,
            bindable_type_codes,
            is_choice_binding,
        });
    }

    out
}

/// Extract direct child field metadata under the supplied generated type root.
///
/// This metadata is used by emitted validation code to:
/// - recurse into nested datatypes/backbone elements
/// - rebase instance paths correctly
/// - understand repeating vs scalar fields
/// - recognize choice elements and their generated Rust representation
pub fn extract_direct_fields_from_elements(
    elements: &[ElementDefinition],
    root_path: &str,
) -> Vec<FieldModel> {
    let mut out = Vec::new();

    for element in elements {
        let element_path = element.path.as_str();
        let is_direct = is_direct_child(root_path, element_path);
        if !is_direct {
            continue;
        }

        let Ok(element_json) = serde_json::to_value(element) else {
            continue;
        };

        let min = element.min.unwrap_or(0);
        let max = element.max.clone().unwrap_or_else(|| "0".to_string());
        let rust_field_name = direct_child_field_name(root_path, element_path);
        let type_codes = element_type_codes(&element_json);
        let element_id = element
            .id
            .clone()
            .unwrap_or_else(|| element_path.to_string());

        let fhir_field_name = direct_child_fhir_field_name(root_path, element_path);
        let is_choice = element_path.ends_with("[x]");
        let choice_base_name = if is_choice {
            Some(fhir_field_name.clone())
        } else {
            None
        };
        let choice_enum_name = choice_base_name.as_ref().map(|base| {
            format!(
                "{}{}",
                rust_type_name(root_path),
                capitalize_first_letter(base)
            )
        });

        out.push(FieldModel {
            element_id,
            fhir_path: element_path.to_string(),
            fhir_field_name: fhir_field_name.clone(),
            rust_field_name,
            type_codes,
            target_profiles: extract_target_profiles(&element_json),
            profiles: extract_profiles(&element_json),
            min,
            max: max.clone(),
            is_array: is_repeating_max(&max),
            is_choice,
            choice_base_name,
            is_required: min > 0,
            choice_enum_name,
        });
    }

    out
}

/// Convert FHIR constraint severity text into the generator's normalized severity model.
pub fn parse_severity(severity: Option<&str>) -> SeverityModel {
    match severity.unwrap_or("error") {
        "fatal" => SeverityModel::Fatal,
        "error" => SeverityModel::Error,
        "warning" => SeverityModel::Warning,
        "information" | "info" => SeverityModel::Information,
        _ => SeverityModel::Error,
    }
}

/// Convert FHIR binding strength text into the generator's normalized binding strength model.
pub fn parse_binding_strength(strength: Option<&str>) -> BindingStrengthModel {
    match strength.unwrap_or("example") {
        "required" => BindingStrengthModel::Required,
        "extensible" => BindingStrengthModel::Extensible,
        "preferred" => BindingStrengthModel::Preferred,
        "example" => BindingStrengthModel::Example,
        _ => BindingStrengthModel::Example,
    }
}

/// Determine which bindable runtime shape a binding applies to.
///
/// For single-type bindings, this returns the concrete target kind.
///
/// For choice fields (`[x]`) with more than one type code, this returns
/// `BindingTargetKindModel::Choice` when any variant is bindable at runtime.
/// The concrete runtime dispatch for choice fields is handled later during
/// code emission based on the actual selected variant.
///
/// Delegates to [`fhir_validation_types::binding_target_kind_from_element_type_codes`].
pub fn binding_target_kind(type_codes: &[String]) -> BindingTargetKindModel {
    binding_target_kind_from_element_type_codes(type_codes).into()
}

/// Classify the inheritance/runtime family for a generated type.
///
/// `ParentKind` is derived from base-definition semantics and is used by
/// emitted validation code for recursion and structural behavior.
///
/// This is intentionally different from `StructureKind`, which tracks the FHIR
/// specification category of the type.
pub fn classify_parent_kind(
    type_name: &str,
    structure_kind: StructureKind,
    def_json: &Value,
) -> ParentKind {
    match type_name {
        "Resource" => return ParentKind::Resource,
        "DomainResource" => return ParentKind::DomainResource,
        "Element" => return ParentKind::Element,
        "BackboneElement" => return ParentKind::BackboneElement,
        _ => {}
    }

    if let Some(base_name) = structure_base_name(def_json) {
        match base_name {
            "Resource" => return ParentKind::Resource,
            "DomainResource" => return ParentKind::DomainResource,
            "Element" => return ParentKind::Element,
            "BackboneElement" => return ParentKind::BackboneElement,
            _ => {}
        }
    }

    match structure_kind {
        StructureKind::PrimitiveType => ParentKind::Primitive,
        StructureKind::ComplexType => ParentKind::ComplexType,
        StructureKind::Resource => ParentKind::Resource,
        StructureKind::Logical | StructureKind::Unknown => ParentKind::Unknown,
    }
}

/// Classify the `ParentKind` for a specific generated path inside a snapshot.
///
/// Nested paths first inspect their typed element declarations (for example,
/// `BackboneElement` or `Element`) before falling back to root-level structure
/// metadata.
pub fn classify_parent_kind_for_path(
    path: &str,
    structure_kind: StructureKind,
    def_json: &Value,
    elements: &[ElementDefinition],
) -> ParentKind {
    if let Some(element) = find_element_by_path(elements, path) {
        let type_codes = element_type_codes_typed(element);

        if type_codes.iter().any(|code| code == "BackboneElement") {
            return ParentKind::BackboneElement;
        }
        if type_codes.iter().any(|code| code == "Element") {
            return ParentKind::Element;
        }
        if type_codes.iter().any(|code| code == "Resource") {
            return ParentKind::Resource;
        }
        if type_codes.iter().any(|code| code == "DomainResource") {
            return ParentKind::DomainResource;
        }
    }

    let type_name = rust_type_name(path);
    classify_parent_kind(&type_name, structure_kind, def_json)
}

/// Classify the FHIR specification category for a generated path.
///
/// `StructureKind` follows `StructureDefinition.kind` semantics:
/// `resource`, `complex-type`, `primitive-type`, or `logical`.
///
/// Nested generated helper types inside resources are treated as
/// `ComplexType` even when their parent resource root is a `Resource`.
/// Inheritance-specific distinctions such as `BackboneElement` are tracked
/// separately in `ParentKind`.
pub fn classify_structure_kind_for_path(
    path: &str,
    root_path: &str,
    root_structure_kind: StructureKind,
    elements: &[ElementDefinition],
) -> StructureKind {
    if path == root_path {
        return root_structure_kind;
    }

    if let Some(element) = find_element_by_path(elements, path) {
        let type_codes = element_type_codes_typed(element);

        if type_codes
            .iter()
            .any(|code| code == "BackboneElement" || code == "Element")
        {
            return StructureKind::ComplexType;
        }
        if type_codes
            .iter()
            .any(|code| code == "Resource" || code == "DomainResource")
        {
            return StructureKind::Resource;
        }
    }

    match root_structure_kind {
        StructureKind::PrimitiveType => StructureKind::PrimitiveType,
        StructureKind::Resource => StructureKind::ComplexType,
        StructureKind::ComplexType => StructureKind::ComplexType,
        StructureKind::Logical => StructureKind::Logical,
        StructureKind::Unknown => StructureKind::Unknown,
    }
}

fn snapshot_elements_typed(def: &StructureDefinition) -> Option<&[ElementDefinition]> {
    def.snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.element.as_ref())
        .map(|elements| elements.as_slice())
}

/// Determine all snapshot paths that become generated Rust validation types.
///
/// The root path is always included. Additional paths are included when the
/// element type indicates a generated nested structure such as
/// `BackboneElement` or `Element`.
fn generated_type_paths(elements: &[ElementDefinition], root_path: &str) -> Vec<String> {
    let mut paths = vec![root_path.to_string()];

    for element in elements {
        let path = element.path.as_str();
        if path == root_path {
            continue;
        }

        let type_codes = element_type_codes_typed(element);
        if type_codes
            .iter()
            .any(|code| code == "BackboneElement" || code == "Element")
        {
            paths.push(path.to_string());
        }
    }

    paths
}

fn find_element_by_path<'a>(
    elements: &'a [ElementDefinition],
    path: &str,
) -> Option<&'a ElementDefinition> {
    elements.iter().find(|element| element.path == path)
}

fn structure_root_path(def: &StructureDefinition) -> Option<&str> {
    snapshot_elements_typed(def)?
        .first()
        .map(|e| e.path.as_str())
}

/// Parse raw `StructureDefinition.kind` into the normalized generator enum.
fn parse_structure_kind(def_json: &Value) -> StructureKind {
    match json_str_field(def_json, "kind") {
        Some("primitive-type") => StructureKind::PrimitiveType,
        Some("complex-type") => StructureKind::ComplexType,
        Some("resource") => StructureKind::Resource,
        Some("logical") => StructureKind::Logical,
        _ => StructureKind::Unknown,
    }
}

fn structure_base_name(def_json: &Value) -> Option<&str> {
    let base = json_str_field(def_json, "baseDefinition")?;
    base.rsplit('/').next()
}

fn json_str_field<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn extract_binding_name_from_extensions(binding_obj: &Value) -> Option<String> {
    let extensions = binding_obj.get("extension")?.as_array()?;

    for ext in extensions {
        let url = json_str_field(ext, "url")?;
        if url == "http://hl7.org/fhir/StructureDefinition/elementdefinition-bindingName" {
            if let Some(name) = json_str_field(ext, "valueString") {
                return Some(name.to_string());
            }
        }
    }

    None
}

fn element_type_codes(element: &Value) -> Vec<String> {
    element
        .get("type")
        .and_then(Value::as_array)
        .map(|types| {
            types
                .iter()
                .filter_map(|t| json_str_field(t, "code"))
                .map(normalize_fhir_element_type_code)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn element_type_codes_typed(element: &ElementDefinition) -> Vec<String> {
    let Ok(element_json) = serde_json::to_value(element) else {
        return Vec::new();
    };

    element_type_codes(&element_json)
}

fn extract_target_profiles(element: &Value) -> Vec<String> {
    element
        .get("type")
        .and_then(Value::as_array)
        .map(|types| {
            types
                .iter()
                .flat_map(|t| {
                    t.get("targetProfile")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flat_map(|profiles| profiles.iter())
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn extract_profiles(element: &Value) -> Vec<String> {
    element
        .get("type")
        .and_then(Value::as_array)
        .map(|types| {
            types
                .iter()
                .flat_map(|t| {
                    t.get("profile")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flat_map(|profiles| profiles.iter())
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn is_direct_child(parent: &str, child: &str) -> bool {
    if !child.starts_with(parent) {
        return false;
    }

    let rest = &child[parent.len()..];
    rest.starts_with('.') && !rest[1..].contains('.')
}

fn direct_child_field_name(parent: &str, child: &str) -> String {
    if !is_direct_child(parent, child) {
        return child.to_string();
    }

    helios_fhir_gen::make_rust_safe(child[parent.len() + 1..].trim_end_matches("[x]"))
}

fn direct_child_fhir_field_name(parent: &str, child: &str) -> String {
    if !is_direct_child(parent, child) {
        return child.to_string();
    }

    child[parent.len() + 1..]
        .trim_end_matches("[x]")
        .to_string()
}

fn rust_type_name(path: &str) -> String {
    let mut out = String::new();

    for segment in path.split('.') {
        let segment = segment.trim_end_matches("[x]");
        if segment.is_empty() {
            continue;
        }
        out.push_str(&capitalize_first_letter(segment));
    }

    if out.is_empty() {
        path.to_string()
    } else {
        out
    }
}

fn root_rust_type_name(def_json: &Value, root_path: &str) -> String {
    if let Some(name) = json_str_field(def_json, "name") {
        if !name.is_empty() {
            return capitalize_first_letter(name);
        }
    }

    if let Some(id) = json_str_field(def_json, "id") {
        if !id.is_empty() {
            return capitalize_first_letter(id);
        }
    }

    rust_type_name(root_path)
}

/// Decide whether a generated validation model should be emitted for this path.
///
/// Rules:
/// - nested generated paths are always eligible
/// - primitive root definitions are skipped
/// - abstract root definitions are skipped
/// - known infrastructure / interface / base-only root types are skipped
///
/// This prevents the generator from emitting validation impls for abstract or
/// non-concrete artifacts such as `Base`, `DataType`, or `CanonicalResource`.
fn should_emit_type_model(path: &str, root_path: &str, def_json: &Value, rust_type: &str) -> bool {
    if path != root_path {
        return true;
    }

    if json_str_field(def_json, "kind") == Some("primitive-type") {
        return false;
    }

    if def_json
        .get("abstract")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }

    !matches!(
        rust_type,
        "Element"
            | "BackboneElement"
            | "BackboneType"
            | "Base"
            | "DataType"
            | "PrimitiveType"
            | "Resource"
            | "DomainResource"
            | "CanonicalResource"
            | "MetadataResource"
            | "MoneyQuantity"
            | "SimpleQuantity"
    )
}

fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn is_repeating_max(max: &str) -> bool {
    if max == "*" {
        return true;
    }

    max.parse::<u32>().map(|n| n > 1).unwrap_or(false)
}
