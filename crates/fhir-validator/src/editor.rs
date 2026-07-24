//! Editor projection: what a *user* may do at a point inside a resource.
//!
//! The engine answers "is this document legal?". An editor has to answer a
//! different question — *"here is a cursor; what am I allowed to put here?"* —
//! and that one has no document to check. It is a query against the schema
//! itself, and the engine's machinery for it ([`SchemaSet`](crate::engine)) is
//! private, so this module builds the answer from the public IR.
//!
//! It lives in this crate rather than in the web UI on purpose: the projection
//! is FHIR knowledge, not presentation, and the SearchParameter and Compartment
//! editors want the same thing.
//!
//! Three traps are handled here so no caller has to know about them:
//!
//! - **Choice groups occupy three keys.** `value[x]` appears in `elements` as a
//!   declarer (`value`, carrying `choices`) *plus* one concrete branch per type
//!   (`valueString`, each carrying `choice_of`). Offering the bare declarer as a
//!   field produces a document the engine rejects as an unknown element, so the
//!   declarer is offered as a **type pick** and the branches are suppressed.
//! - **Cardinality already spent is not tracked anywhere.** Nothing in the IR or
//!   the engine says "this element is 0..1 and already has a value". We diff the
//!   document against the schema to work it out.
//! - **Some pack schemas carry a literal `value[x]` element key** — a converter
//!   artifact. A caller enumerating keys would render a phantom field.
//!
//! Paths use the same dotted form the engine reports errors on
//! (`Patient.name.0.given`), so a [`ValidationError`](crate::ValidationError)
//! anchors onto an editor node by string equality — no FHIRPath parsing, and
//! none of the fuzzy path-matching that other editors resort to.

use std::collections::HashSet;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::resolver::SchemaResolver;
use crate::schema::{Binding, FhirSchema, kind};

/// One hop into a resource: a field name, or an index into a repeating element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Field(String),
    Index(usize),
}

/// A path from the root of a resource to a node.
pub type Path = Vec<Step>;

/// Renders a path in the engine's dotted form — `name.0.given` — which is also
/// what [`ValidationError::path`](crate::ValidationError) uses, so the two are
/// directly comparable.
pub fn path_to_string(path: &[Step]) -> String {
    path.iter()
        .map(|step| match step {
            Step::Field(name) => name.clone(),
            Step::Index(i) => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Parses the dotted form back. All-digit segments are indices; everything else
/// is a field name (FHIR element names never start with a digit).
pub fn path_from_string(text: &str) -> Path {
    text.split('.')
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment.parse::<usize>() {
            Ok(index) => Step::Index(index),
            Err(_) => Step::Field(segment.to_string()),
        })
        .collect()
}

/// What kind of affordance an addable element gets.
#[derive(Debug, Clone, PartialEq)]
pub enum AddableKind {
    /// Not present yet — offer it.
    Add,
    /// Present and repeating — offer another.
    AddAnother,
    /// A `value[x]`: the user picks a type first. Carries the concrete branch
    /// names (`valueString`, `valueAddress`, …).
    Choice(Vec<String>),
}

/// An element the user may add at a cursor.
#[derive(Debug, Clone)]
pub struct Addable {
    /// Element name, as it appears in the JSON.
    pub name: String,
    pub kind: AddableKind,
    /// The element's type (`HumanName`, `string`, `Extension`, …).
    pub type_: Option<String>,
    /// Whether the type is a primitive — decides leaf input vs. nested node.
    pub is_primitive: bool,
    /// Declared required on the parent.
    pub required: bool,
    /// Terminology binding, if the element is coded.
    pub binding: Option<Binding>,
    /// Allowed reference targets, for scoping a reference picker.
    pub refers: Option<Vec<String>>,
    /// `modifierExtension` — an unrecognised one means *do not process this
    /// resource*. It is a safety-relevant element and no editor surveyed
    /// surfaces it differently. This one will.
    pub is_modifier: bool,
}

/// An element that is *present* in the document at a cursor, paired with the
/// schema that governs it. Rendering walks these.
#[derive(Debug, Clone)]
pub struct Present {
    pub name: String,
    /// The element schema, if the schema knows this key. `None` means the
    /// document carries a key the schema does not describe — we still render it
    /// (never silently drop the user's data) and mark it unknown.
    pub schema: Option<Arc<FhirSchema>>,
    pub is_primitive: bool,
    pub is_array: bool,
    /// A primitive's sibling `_name` object (extensions on a primitive). Carried
    /// so it is never lost on round-trip.
    pub has_primitive_extension: bool,
}

// ---------------------------------------------------------------------------
// Schema navigation
// ---------------------------------------------------------------------------

/// Every element visible on a node: the base chain, the element's own inline
/// children (BackboneElements carry them), and the named type's children.
///
/// The core pack is snapshot-derived and therefore already flat, so this
/// usually returns on the first layer. It matters for tenant profiles, which
/// are differential and sparse.
fn merged_elements(
    resolver: &dyn SchemaResolver,
    schema: &FhirSchema,
    seen: &mut HashSet<String>,
) -> IndexMap<String, Arc<FhirSchema>> {
    let mut out = IndexMap::new();

    // Guard against a schema whose `type_` names itself (Patient.type_ ==
    // "Patient"), which would otherwise recurse forever.
    let identity = schema
        .url
        .clone()
        .or_else(|| schema.name.clone())
        .unwrap_or_default();
    if !identity.is_empty() && !seen.insert(identity) {
        return out;
    }

    if let Some(base) = &schema.base
        && let Some(base_schema) = resolver.resolve(base)
    {
        out.extend(merged_elements(resolver, &base_schema, seen));
    }

    // A named type (`name: { type: HumanName }`) contributes that type's
    // elements. Primitives contribute nothing structural.
    if let Some(type_name) = &schema.type_
        && let Some(type_schema) = resolver.resolve(type_name)
        && !type_schema.is_primitive()
    {
        out.extend(merged_elements(resolver, &type_schema, seen));
    }

    if let Some(elements) = &schema.elements {
        for (name, element) in elements {
            out.insert(name.clone(), Arc::clone(element));
        }
    }

    out
}

/// The names required on this node, gathered across the same layers.
fn merged_required(
    resolver: &dyn SchemaResolver,
    schema: &FhirSchema,
    seen: &mut HashSet<String>,
) -> HashSet<String> {
    let mut out = HashSet::new();

    let identity = schema
        .url
        .clone()
        .or_else(|| schema.name.clone())
        .unwrap_or_default();
    if !identity.is_empty() && !seen.insert(identity) {
        return out;
    }

    if let Some(base) = &schema.base
        && let Some(base_schema) = resolver.resolve(base)
    {
        out.extend(merged_required(resolver, &base_schema, seen));
    }
    if let Some(type_name) = &schema.type_
        && let Some(type_schema) = resolver.resolve(type_name)
        && !type_schema.is_primitive()
    {
        out.extend(merged_required(resolver, &type_schema, seen));
    }
    if let Some(required) = &schema.required {
        out.extend(required.iter().cloned());
    }
    out
}

/// The schema governing the node at `path`.
///
/// Indices do not change the schema — an item of a repeating element is
/// governed by the element itself.
pub fn schema_at(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    path: &[Step],
) -> Option<Arc<FhirSchema>> {
    let mut current = resolver.resolve(root_type)?;

    for step in path {
        let Step::Field(name) = step else { continue };
        let elements = merged_elements(resolver, &current, &mut HashSet::new());
        current = Arc::clone(elements.get(name)?);
    }
    Some(current)
}

/// Whether an element schema denotes a primitive value.
fn is_primitive(resolver: &dyn SchemaResolver, element: &FhirSchema) -> bool {
    if element.is_primitive() {
        return true;
    }
    match &element.type_ {
        Some(type_name) => resolver
            .resolve(type_name)
            .map(|schema| schema.is_primitive())
            .unwrap_or(false),
        // No type and no children: nothing to descend into.
        None => element.elements.is_none() && element.choices.is_none(),
    }
}

// ---------------------------------------------------------------------------
// Document navigation
// ---------------------------------------------------------------------------

/// The value at `path`, if the document has one.
pub fn node_at<'a>(document: &'a Value, path: &[Step]) -> Option<&'a Value> {
    let mut current = document;
    for step in path {
        current = match step {
            Step::Field(name) => current.get(name)?,
            Step::Index(index) => current.get(*index)?,
        };
    }
    Some(current)
}

fn node_at_mut<'a>(document: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    let mut current = document;
    for step in path {
        current = match step {
            Step::Field(name) => current.get_mut(name)?,
            Step::Index(index) => current.get_mut(*index)?,
        };
    }
    Some(current)
}

// ---------------------------------------------------------------------------
// The question the editor rests on
// ---------------------------------------------------------------------------

/// What may be added at `path`, in spec order.
///
/// Excludes what the document has already spent: a non-repeating element that
/// is set is not offered again, and a `value[x]` whose sibling arm is taken is
/// not offered again either.
pub fn addable(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &Value,
    path: &[Step],
) -> Vec<Addable> {
    let Some(schema) = schema_at(resolver, root_type, path) else {
        return Vec::new();
    };
    let elements = merged_elements(resolver, &schema, &mut HashSet::new());
    let required = merged_required(resolver, &schema, &mut HashSet::new());
    let excluded: HashSet<&str> = schema
        .excluded
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();

    let node = node_at(document, path);
    let present = |name: &str| {
        node.and_then(|value| value.get(name))
            .map(|value| !value.is_null())
            .unwrap_or(false)
    };

    let mut out = Vec::new();
    for (name, element) in &elements {
        // Converter artifact: a literal `value[x]` key that no document ever
        // carries. Rendering it would invent a field that cannot exist.
        if name.contains("[x]") || excluded.contains(name.as_str()) {
            continue;
        }
        // `resourceType` is the document's identity, not an element of it.
        // Offering it would let a user turn a Patient into an Observation by
        // typing.
        if name == "resourceType" && path.is_empty() {
            continue;
        }
        // A concrete choice branch is reached through its declarer, never on
        // its own.
        if element.choice_of.is_some() {
            continue;
        }

        if let Some(choices) = &element.choices {
            // One arm at most: if any is taken, the group is spent.
            if choices.iter().any(|arm| present(arm)) {
                continue;
            }
            out.push(Addable {
                name: name.clone(),
                kind: AddableKind::Choice(choices.clone()),
                type_: None,
                is_primitive: false,
                required: required.contains(name),
                binding: element.binding.clone(),
                refers: element.refers.clone(),
                is_modifier: false,
            });
            continue;
        }

        let repeats = element.array.unwrap_or(false);
        let taken = present(name);
        if taken && !repeats {
            continue;
        }

        out.push(Addable {
            name: name.clone(),
            kind: if taken {
                AddableKind::AddAnother
            } else {
                AddableKind::Add
            },
            type_: element.type_.clone(),
            is_primitive: is_primitive(resolver, element),
            required: required.contains(name),
            binding: element.binding.clone(),
            refers: element.refers.clone(),
            is_modifier: name == "modifierExtension" || element.modifier.unwrap_or(false),
        });
    }
    out
}

/// The elements *present* on the node at `path`, in spec order, with any keys
/// the schema does not know appended.
///
/// Unknown keys are rendered rather than dropped. An editor that silently eats
/// what it cannot model is a data-loss bug, and every tool surveyed for #264
/// has one.
pub fn present_children(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &Value,
    path: &[Step],
) -> Vec<Present> {
    let Some(node) = node_at(document, path) else {
        return Vec::new();
    };
    let Some(object) = node.as_object() else {
        return Vec::new();
    };

    let elements = schema_at(resolver, root_type, path)
        .map(|schema| merged_elements(resolver, &schema, &mut HashSet::new()))
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut rendered: HashSet<&str> = HashSet::new();

    // Spec order first.
    for (name, element) in &elements {
        if name.contains("[x]") || element.choices.is_some() {
            continue;
        }
        // The document's identity, not one of its elements.
        if name == "resourceType" && path.is_empty() {
            continue;
        }
        if !object.contains_key(name) {
            continue;
        }
        rendered.insert(name.as_str());
        out.push(Present {
            name: name.clone(),
            is_primitive: is_primitive(resolver, element),
            is_array: object[name].is_array(),
            has_primitive_extension: object.contains_key(&format!("_{name}")),
            schema: Some(Arc::clone(element)),
        });
    }

    // Anything the schema did not describe. `_name` siblings are carried by
    // their primitive, not rendered on their own.
    for (name, value) in object {
        if rendered.contains(name.as_str())
            || name == "resourceType"
            || name.starts_with('_')
            || elements.contains_key(name)
        {
            continue;
        }
        out.push(Present {
            name: name.clone(),
            schema: None,
            is_primitive: !value.is_object() && !value.is_array(),
            is_array: value.is_array(),
            has_primitive_extension: false,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Mutation
// ---------------------------------------------------------------------------

/// An empty value of the right JSON shape for an element: an object for a
/// complex type, an empty string for a primitive, and an array wrapper when the
/// element repeats.
pub fn seed_value(resolver: &dyn SchemaResolver, element: &FhirSchema) -> Value {
    if is_primitive(resolver, element) {
        // `fixed` and `pattern` are free correctness: prefill them rather than
        // asking a human to type `http://loinc.org`.
        if let Some(fixed) = &element.fixed {
            return fixed.clone();
        }
        if let Some(pattern) = &element.pattern {
            return pattern.clone();
        }
        return Value::String(String::new());
    }
    if let Some(pattern) = &element.pattern {
        return pattern.clone();
    }
    Value::Object(Map::new())
}

/// Adds `name` under `path`. Repeating elements append; scalar elements are
/// created. Returns the path of the node that was created, so the caller can
/// focus it.
pub fn add_element(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &mut Value,
    path: &[Step],
    name: &str,
) -> Option<Path> {
    let schema = schema_at(resolver, root_type, path)?;
    let elements = merged_elements(resolver, &schema, &mut HashSet::new());
    let element = elements.get(name)?;
    let repeats = element.array.unwrap_or(false);
    let seed = seed_value(resolver, element);

    let node = node_at_mut(document, path)?;
    let object = node.as_object_mut()?;

    let mut created = path.to_vec();
    created.push(Step::Field(name.to_string()));

    if repeats {
        let array = object
            .entry(name.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let items = array.as_array_mut()?;
        items.push(seed);
        created.push(Step::Index(items.len() - 1));
    } else {
        object.insert(name.to_string(), seed);
    }
    Some(created)
}

/// Removes the node at `path`. Removing an array item collapses the array when
/// it empties, so the document never carries `"name": []`, which is invalid
/// FHIR.
pub fn remove_at(document: &mut Value, path: &[Step]) -> bool {
    let Some((last, parent_path)) = path.split_last() else {
        return false;
    };
    let Some(parent) = node_at_mut(document, parent_path) else {
        return false;
    };

    match last {
        Step::Field(name) => {
            let Some(object) = parent.as_object_mut() else {
                return false;
            };
            // A primitive's extension sibling goes with it.
            object.remove(&format!("_{name}"));
            object.remove(name).is_some()
        }
        Step::Index(index) => {
            let Some(array) = parent.as_array_mut() else {
                return false;
            };
            if *index >= array.len() {
                return false;
            }
            array.remove(*index);
            if array.is_empty() {
                // Drop the now-empty array from its parent.
                if let Some((name, grandparent_path)) = parent_path.split_last()
                    && let Step::Field(name) = name
                    && let Some(grandparent) = node_at_mut(document, grandparent_path)
                    && let Some(object) = grandparent.as_object_mut()
                {
                    object.remove(name);
                }
            }
            true
        }
    }
}

/// Sets a primitive value at `path`. An empty string removes the element rather
/// than storing `""`, which is not a valid FHIR primitive.
pub fn set_value(document: &mut Value, path: &[Step], raw: &str) -> bool {
    if raw.is_empty() {
        return remove_at(document, path);
    }
    let Some(node) = node_at_mut(document, path) else {
        return false;
    };
    // Keep JSON types honest: booleans and numbers are not strings in FHIR.
    *node = match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => match raw.parse::<i64>() {
            Ok(number) if !raw.starts_with('0') || raw == "0" => Value::from(number),
            _ => Value::String(raw.to_string()),
        },
    };
    true
}

/// Picks a concrete arm of a `value[x]`: creates `valueString` and drops any
/// sibling arm that was set.
pub fn choose_type(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &mut Value,
    path: &[Step],
    declarer: &str,
    arm: &str,
) -> Option<Path> {
    let schema = schema_at(resolver, root_type, path)?;
    let elements = merged_elements(resolver, &schema, &mut HashSet::new());
    let choices = elements.get(declarer)?.choices.clone()?;
    if !choices.contains(&arm.to_string()) {
        return None;
    }

    if let Some(node) = node_at_mut(document, path)
        && let Some(object) = node.as_object_mut()
    {
        for other in &choices {
            object.remove(other);
        }
    }
    add_element(resolver, root_type, document, path, arm)
}

/// The `Extension` element schema, for the ad-hoc extension path: every node
/// that accepts an extension accepts *any* extension, and the base `Extension`
/// type carries `url`, the recursive `extension`, and every `value[x]` arm.
/// This is what lets us offer extension editing on resources that carry no
/// profile at all — the thing the surveyed editors cannot do.
pub fn extension_value_arms(resolver: &dyn SchemaResolver) -> Vec<String> {
    let Some(extension) = resolver.resolve("Extension") else {
        return Vec::new();
    };
    extension
        .elements
        .iter()
        .flatten()
        .find(|(_, element)| element.choices.is_some())
        .and_then(|(_, element)| element.choices.clone())
        .unwrap_or_default()
}

/// Adds an extension with the given URL at `path`, returning the path of the
/// new extension node.
pub fn add_extension(
    resolver: &dyn SchemaResolver,
    root_type: &str,
    document: &mut Value,
    path: &[Step],
    url: &str,
    modifier: bool,
) -> Option<Path> {
    let name = if modifier {
        "modifierExtension"
    } else {
        "extension"
    };
    let created = add_element(resolver, root_type, document, path, name)?;
    if let Some(node) = node_at_mut(document, &created)
        && let Some(object) = node.as_object_mut()
    {
        object.insert("url".to_string(), Value::String(url.to_string()));
    }
    Some(created)
}

/// Whether a schema is a resource (as opposed to a datatype) — the editor only
/// opens resources.
pub fn is_resource(schema: &FhirSchema) -> bool {
    schema.kind.as_deref() == Some(kind::RESOURCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::core_registry;
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn registry() -> Arc<crate::SchemaRegistry> {
        core_registry(FhirVersion::R4)
    }

    fn donald() -> Value {
        json!({
            "resourceType": "Patient",
            "id": "donald-duck",
            "name": [{
                "family": "Duck",
                "given": ["Donald"],
                "extension": [{
                    "url": "http://hl7.org/fhir/StructureDefinition/humanname-own-name",
                    "valueString": "Donald"
                }]
            }],
            "gender": "male",
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/patient-birthPlace",
                "valueAddress": { "city": "Duckburg" }
            }]
        })
    }

    fn names(addables: &[Addable]) -> Vec<&str> {
        addables.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn offers_the_elements_of_a_patient_in_spec_order() {
        let registry = registry();
        let list = addable(registry.as_ref(), "Patient", &donald(), &[]);

        assert!(names(&list).contains(&"identifier"));
        assert!(names(&list).contains(&"birthDate"));
        // Inherited from DomainResource / Resource, and present without any
        // base-chain walking because the core pack is snapshot-derived.
        assert!(names(&list).contains(&"meta"));
        assert!(names(&list).contains(&"text"));
    }

    #[test]
    fn cardinality_already_spent_is_not_offered_again() {
        let registry = registry();
        let list = addable(registry.as_ref(), "Patient", &donald(), &[]);

        // gender is set and does not repeat.
        assert!(!names(&list).contains(&"gender"));
        // name is set but repeats — offered as "add another".
        let name = list
            .iter()
            .find(|a| a.name == "name")
            .expect("name offered");
        assert_eq!(name.kind, AddableKind::AddAnother);
    }

    #[test]
    fn a_choice_group_is_offered_as_a_type_pick_and_its_branches_are_hidden() {
        let registry = registry();
        let list = addable(registry.as_ref(), "Patient", &donald(), &[]);

        let deceased = list
            .iter()
            .find(|a| a.name == "deceased")
            .expect("the value[x] declarer is offered");
        match &deceased.kind {
            AddableKind::Choice(arms) => {
                assert!(arms.contains(&"deceasedBoolean".to_string()));
                assert!(arms.contains(&"deceasedDateTime".to_string()));
            }
            other => panic!("expected a choice, got {other:?}"),
        }
        // The concrete branches are never offered as fields of their own.
        assert!(!names(&list).contains(&"deceasedBoolean"));
    }

    #[test]
    fn taking_one_arm_spends_the_whole_choice_group() {
        let registry = registry();
        let mut patient = donald();
        patient["deceasedBoolean"] = json!(false);

        let list = addable(registry.as_ref(), "Patient", &patient, &[]);
        assert!(!names(&list).contains(&"deceased"));
    }

    /// The case Steve called out as the hard one: an extension hanging off a
    /// name. It needs no special support — `extension` is an ordinary element
    /// of `HumanName`.
    #[test]
    fn an_extension_can_be_added_to_a_name() {
        let registry = registry();
        let path = path_from_string("name.0");
        let list = addable(registry.as_ref(), "Patient", &donald(), &path);

        assert!(names(&list).contains(&"extension"));
        assert!(names(&list).contains(&"prefix"));
        assert!(names(&list).contains(&"period"));
    }

    /// And extensions nest, because `Extension.extension` is recursive.
    #[test]
    fn extensions_nest() {
        let registry = registry();
        let path = path_from_string("extension.0");
        let list = addable(registry.as_ref(), "Patient", &donald(), &path);

        assert!(names(&list).contains(&"extension"));
        // url is set and does not repeat.
        assert!(!names(&list).contains(&"url"));
        // The value[x] is spent by valueAddress.
        assert!(!names(&list).contains(&"value"));
    }

    #[test]
    fn a_fresh_extension_offers_every_value_type() {
        let registry = registry();
        let mut patient = donald();
        let path = add_extension(
            registry.as_ref(),
            "Patient",
            &mut patient,
            &[],
            "http://example.org/my-extension",
            false,
        )
        .expect("extension added");

        let list = addable(registry.as_ref(), "Patient", &patient, &path);
        let value = list
            .iter()
            .find(|a| a.name == "value")
            .expect("value[x] offered");
        match &value.kind {
            AddableKind::Choice(arms) => {
                assert!(arms.len() > 40, "every value[x] arm, got {}", arms.len());
                assert!(arms.contains(&"valueAddress".to_string()));
            }
            other => panic!("expected a choice, got {other:?}"),
        }
        // The url we asked for is already on it.
        assert_eq!(
            node_at(&patient, &path).and_then(|node| node.get("url")),
            Some(&json!("http://example.org/my-extension"))
        );
    }

    #[test]
    fn adding_a_repeating_element_appends_and_returns_its_path() {
        let registry = registry();
        let mut patient = donald();
        let path = add_element(registry.as_ref(), "Patient", &mut patient, &[], "name")
            .expect("name added");

        assert_eq!(path_to_string(&path), "name.1");
        assert_eq!(patient["name"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn choosing_a_type_drops_the_sibling_arm() {
        let registry = registry();
        let mut patient = donald();
        patient["deceasedBoolean"] = json!(true);

        choose_type(
            registry.as_ref(),
            "Patient",
            &mut patient,
            &[],
            "deceased",
            "deceasedDateTime",
        )
        .expect("type chosen");

        assert!(patient.get("deceasedBoolean").is_none());
        assert!(patient.get("deceasedDateTime").is_some());
    }

    #[test]
    fn removing_the_last_item_drops_the_empty_array() {
        let mut patient = donald();
        assert!(remove_at(&mut patient, &path_from_string("name.0")));
        // Not `"name": []`, which is invalid FHIR.
        assert!(patient.get("name").is_none());
    }

    /// The resource type is the document's identity, not an element of it —
    /// offering it as a field would let a user turn a Patient into an
    /// Observation by typing over it.
    #[test]
    fn resource_type_is_not_an_editable_element() {
        let registry = registry();
        let list = addable(registry.as_ref(), "Patient", &donald(), &[]);
        assert!(!names(&list).contains(&"resourceType"));

        let children = present_children(registry.as_ref(), "Patient", &donald(), &[]);
        assert!(!children.iter().any(|child| child.name == "resourceType"));
    }

    #[test]
    fn a_key_the_schema_does_not_know_is_still_rendered() {
        let registry = registry();
        let mut patient = donald();
        patient["somethingWeird"] = json!("keep me");

        let children = present_children(registry.as_ref(), "Patient", &patient, &[]);
        let weird = children
            .iter()
            .find(|c| c.name == "somethingWeird")
            .expect("unknown keys are rendered, never dropped");
        assert!(weird.schema.is_none());
    }

    /// The data-loss bug every surveyed editor has: an extended primitive lives
    /// in two keys (`birthDate` + `_birthDate`). We must notice.
    #[test]
    fn a_primitive_extension_sibling_is_noticed_and_travels_with_its_primitive() {
        let registry = registry();
        let mut patient = donald();
        patient["birthDate"] = json!("1934-06-09");
        patient["_birthDate"] = json!({
            "extension": [{ "url": "http://example.org/precision", "valueCode": "day" }]
        });

        let children = present_children(registry.as_ref(), "Patient", &patient, &[]);
        let birth_date = children
            .iter()
            .find(|c| c.name == "birthDate")
            .expect("birthDate present");
        assert!(birth_date.has_primitive_extension);
        // And it is not rendered as a field in its own right.
        assert!(!children.iter().any(|c| c.name == "_birthDate"));

        // Removing the primitive takes its extension sibling with it, rather
        // than leaving an orphan.
        remove_at(&mut patient, &path_from_string("birthDate"));
        assert!(patient.get("_birthDate").is_none());
    }

    #[test]
    fn modifier_extensions_are_flagged_as_such() {
        let registry = registry();
        let list = addable(registry.as_ref(), "Patient", &donald(), &[]);
        let modifier = list
            .iter()
            .find(|a| a.name == "modifierExtension")
            .expect("offered");
        assert!(
            modifier.is_modifier,
            "an unrecognised modifierExtension means 'do not process this \
             resource' — the editor has to say so"
        );
    }

    #[test]
    fn bindings_and_reference_targets_reach_the_caller() {
        let registry = registry();
        let mut patient = donald();
        patient.as_object_mut().unwrap().remove("gender");

        let list = addable(registry.as_ref(), "Patient", &patient, &[]);
        let gender = list.iter().find(|a| a.name == "gender").expect("offered");
        let binding = gender.binding.as_ref().expect("gender is coded");
        assert!(binding.value_set.contains("administrative-gender"));
        assert_eq!(binding.strength.as_deref(), Some("required"));

        let org = list
            .iter()
            .find(|a| a.name == "managingOrganization")
            .expect("offered");
        assert!(
            org.refers
                .as_ref()
                .expect("a Reference declares its targets")
                .iter()
                .any(|target| target.contains("Organization"))
        );
    }

    #[test]
    fn paths_round_trip_and_match_the_engines_error_format() {
        let path = path_from_string("name.0.given.1");
        assert_eq!(
            path,
            vec![
                Step::Field("name".into()),
                Step::Index(0),
                Step::Field("given".into()),
                Step::Index(1),
            ]
        );
        assert_eq!(path_to_string(&path), "name.0.given.1");
    }

    #[test]
    fn setting_a_value_keeps_json_types_honest() {
        let mut patient = donald();
        add_element(registry().as_ref(), "Patient", &mut patient, &[], "active").unwrap();

        set_value(&mut patient, &path_from_string("active"), "true");
        assert_eq!(patient["active"], json!(true));

        // Clearing a field removes it rather than storing "".
        set_value(&mut patient, &path_from_string("active"), "");
        assert!(patient.get("active").is_none());
    }
}
