//! ElementDefinition-id → nested tree reconstruction.
//!
//! Each ElementDefinition is applied by navigating its `id` absolutely from
//! the root, segment by segment (`name` or `name:sliceName`), creating empty
//! intermediate nodes as needed — which is what makes sparse differentials
//! work without snapshot generation. Choice (`foo[x]`) expansion also lives
//! here: the `[x]` element becomes a `choices` declarer plus one `choiceOf`
//! branch element per type.

use super::{Ed, EdConstraint, EdDiscriminator};
use crate::schema::{Binding, Constraint, FhirSchema};
use indexmap::IndexMap;
use std::sync::Arc;

/// One `id` step: an element name plus an optional slice name.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Segment {
    pub name: String,
    pub slice: Option<String>,
}

/// Parse an ElementDefinition id into segments, dropping the root segment.
/// `"Patient.identifier:mrn.system"` → `[identifier:mrn, system]`.
pub(super) fn parse_segments(id: &str) -> Result<Vec<Segment>, String> {
    let mut parts = id.split('.');
    let root = parts.next().ok_or("empty id")?;
    if root.is_empty() {
        return Err("empty root segment".to_string());
    }
    let mut segments = Vec::new();
    for part in parts {
        if part.is_empty() {
            return Err("empty segment".to_string());
        }
        match part.split_once(':') {
            Some((name, slice)) if !name.is_empty() && !slice.is_empty() => {
                segments.push(Segment {
                    name: name.to_string(),
                    slice: Some(slice.to_string()),
                });
            }
            Some(_) => return Err(format!("malformed slice segment '{part}'")),
            None => segments.push(Segment {
                name: part.to_string(),
                slice: None,
            }),
        }
    }
    Ok(segments)
}

/// A node of the element tree under construction.
#[derive(Debug, Default)]
pub(super) struct Node {
    /// The element name this node sits under (used for extension-sugar
    /// detection on `extension`/`modifierExtension`).
    pub element_name: String,
    /// Accumulated element-level keywords.
    pub schema: FhirSchema,
    pub children: IndexMap<String, Node>,
    pub slices: IndexMap<String, SliceNode>,
    /// `type[].profile` URLs from the ElementDefinition (used when emitting
    /// `profile` slice matchers; not part of the finalized schema IR).
    pub type_profiles: Vec<String>,
    /// Slicing declaration from the sliced element's own ED.
    pub discriminators: Vec<EdDiscriminator>,
    pub slicing_rules: Option<String>,
    pub slicing_ordered: Option<bool>,
    /// `max: "0"` — the element is prohibited; recorded on the parent's
    /// `excluded` and the subtree is dropped.
    pub dead: bool,
}

impl Node {
    pub(super) fn new(element_name: String) -> Self {
        Self {
            element_name,
            ..Default::default()
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct SliceNode {
    pub node: Node,
    pub min: Option<u64>,
    pub max: Option<u64>,
    /// `type[0] == Extension` with a profile: compiles to the parent-level
    /// `extensions` sugar instead of raw slicing.
    pub extension_profile: Option<String>,
    /// Parent slice name when `sliceName` uses `parent/child` reslice form.
    pub reslice: Option<String>,
    /// Further constrains a same-named slice from a parent schema.
    pub slice_is_constraining: Option<bool>,
}

/// Apply one ElementDefinition at its navigated position.
pub(super) fn apply(root: &mut Node, segments: &[Segment], ed: &Ed, warnings: &mut Vec<String>) {
    // Navigate to the parent of the final segment.
    let mut node: &mut Node = root;
    for seg in &segments[..segments.len() - 1] {
        if node.dead {
            return;
        }
        node = descend(node, seg);
    }
    if node.dead {
        return;
    }
    let last = &segments[segments.len() - 1];

    // Explicit choice-branch constraint: `value[x]:valueQuantity`.
    if let (true, Some(slice)) = (last.name.ends_with("[x]"), &last.slice) {
        let branch = node
            .children
            .entry(slice.clone())
            .or_insert_with(|| Node::new(slice.clone()));
        apply_element_content(branch, ed, warnings);
        return;
    }

    // Slice definition: `identifier:mrn` or reslice `identifier:mrn/secondary`.
    if let Some(slice_name) = &last.slice {
        let element = node
            .children
            .entry(last.name.clone())
            .or_insert_with(|| Node::new(last.name.clone()));
        let (store_name, reslice_of) = match slice_name.rsplit_once('/') {
            Some((parent, child)) if !parent.is_empty() && !child.is_empty() => {
                (slice_name.clone(), Some(parent.to_string()))
            }
            _ => (slice_name.clone(), None),
        };
        let slice = element.slices.entry(store_name).or_default();
        slice.min = ed.min;
        slice.max = parse_numeric_max(ed.max.as_deref());
        slice.reslice = reslice_of;
        slice.slice_is_constraining = ed.slice_is_constraining;
        if matches!(
            element.element_name.as_str(),
            "extension" | "modifierExtension"
        ) && let Some(first) = ed.types.first()
            && first.code == "Extension"
            && let Some(profile) = first.profile.first()
        {
            slice.extension_profile = Some(profile.clone());
        }
        // Value-level keywords on the slice ED itself apply to the slice
        // schema (shape/cardinality were consumed above as slice bounds).
        apply_value_keywords(&mut slice.node.schema, ed, warnings);
        return;
    }

    // Choice element: `deceased[x]` — declarer + one branch per type.
    if let Some(base_name) = last.name.strip_suffix("[x]") {
        apply_choice(node, base_name, ed, warnings);
        return;
    }

    // Plain element.
    let name = last.name.clone();
    if ed.max.as_deref() == Some("0") {
        push_unique(&mut node.schema.excluded, &name);
        let element = node
            .children
            .entry(name.clone())
            .or_insert_with(|| Node::new(name.clone()));
        element.dead = true;
        return;
    }
    if ed.min.unwrap_or(0) >= 1 {
        push_unique(&mut node.schema.required, &name);
    }
    let element = node
        .children
        .entry(name.clone())
        .or_insert_with(|| Node::new(name));
    if let Some(slicing) = &ed.slicing {
        element.discriminators = slicing.discriminator.clone();
        element.slicing_rules = slicing.rules.clone();
        element.slicing_ordered = slicing.ordered;
    }
    apply_shape(element, ed);
    apply_element_content(element, ed, warnings);
}

fn descend<'a>(node: &'a mut Node, seg: &Segment) -> &'a mut Node {
    let child = node
        .children
        .entry(seg.name.clone())
        .or_insert_with(|| Node::new(seg.name.clone()));
    match &seg.slice {
        Some(slice) => &mut child.slices.entry(slice.clone()).or_default().node,
        None => child,
    }
}

/// Array-ness comes from `base.max` when present (profiles constrain the
/// count, not the shape), else the ED's own `max`/`min`.
fn apply_shape(element: &mut Node, ed: &Ed) {
    let shape_max = ed
        .base
        .as_ref()
        .and_then(|b| b.max.as_deref())
        .or(ed.max.as_deref());
    let is_array = match shape_max {
        Some("*") => true,
        Some(n) => n.parse::<u64>().map(|n| n > 1).unwrap_or(false),
        None => ed.min.unwrap_or(0) > 1,
    };
    if is_array {
        element.schema.array = Some(true);
        if let Some(min) = ed.min
            && min > 0
        {
            element.schema.min = Some(min);
        }
        if let Some(max) = parse_numeric_max(ed.max.as_deref()) {
            element.schema.max = Some(max);
        }
    }
}

/// Type reference, contentReference, and the value-level keywords.
fn apply_element_content(element: &mut Node, ed: &Ed, warnings: &mut Vec<String>) {
    match ed.types.len() {
        0 => {
            if let Some(reference) = &ed.content_reference {
                match parse_content_reference(reference) {
                    Some(segments) => element.schema.element_reference = Some(segments),
                    None => warnings.push(format!(
                        "{}: unparseable contentReference '{reference}'",
                        ed.path
                    )),
                }
            }
        }
        1 => {
            let t = &ed.types[0];
            element.schema.type_ = Some(t.effective_code());
            if !t.profile.is_empty() {
                element.type_profiles = t.profile.clone();
            }
            if !t.target_profile.is_empty() {
                element.schema.refers = Some(t.target_profile.clone());
            }
        }
        _ => {
            // Multiple types without `[x]` should not occur; take the first.
            warnings.push(format!(
                "{}: multiple types without [x]; using the first",
                ed.path
            ));
            element.schema.type_ = Some(ed.types[0].effective_code());
            if !ed.types[0].profile.is_empty() {
                element.type_profiles = ed.types[0].profile.clone();
            }
        }
    }
    apply_value_keywords(&mut element.schema, ed, warnings);
}

/// Binding, constraints, fixed/pattern/maxLength/minValue/maxValue — keywords
/// that apply to a value wherever the ED lands (plain element, choice branch,
/// or slice schema).
fn apply_value_keywords(schema: &mut FhirSchema, ed: &Ed, _warnings: &mut Vec<String>) {
    if let Some(binding) = &ed.binding
        && let Some(value_set) = &binding.value_set
    {
        schema.binding = Some(Binding {
            value_set: value_set.clone(),
            strength: binding.strength.clone(),
        });
    }
    apply_constraints(schema, &ed.constraint, false);
    if let Some(max_length) = ed.max_length {
        schema.max_length = Some(max_length);
    }
    for (key, value) in &ed.rest {
        if let Some(rest) = key.strip_prefix("fixed")
            && !rest.is_empty()
            && !key.starts_with('_')
        {
            schema.fixed = Some(value.clone());
        } else if let Some(rest) = key.strip_prefix("pattern")
            && !rest.is_empty()
            && !key.starts_with('_')
        {
            schema.pattern = Some(value.clone());
        } else if let Some(rest) = key.strip_prefix("minValue")
            && !rest.is_empty()
            && !key.starts_with('_')
        {
            schema.min_value = Some(value.clone());
        } else if let Some(rest) = key.strip_prefix("maxValue")
            && !rest.is_empty()
            && !key.starts_with('_')
        {
            schema.max_value = Some(value.clone());
        }
    }
}

/// Carry FHIRPath constraints. On non-root elements the ubiquitous inherited
/// invariants (`ele-1` on every element, `ext-1` on every extension) are
/// dropped — they are enforced once via the `Element`/`Extension` type
/// schemas that join every relevant cooperative set, and carrying them
/// per-element would bloat packs and the deferred-effects list enormously.
pub(super) fn apply_constraints(schema: &mut FhirSchema, constraints: &[EdConstraint], root: bool) {
    for c in constraints {
        if !root && matches!(c.key.as_str(), "ele-1" | "ext-1") {
            continue;
        }
        let Some(expression) = &c.expression else {
            continue; // xpath-only constraint
        };
        schema
            .constraints
            .get_or_insert_with(IndexMap::new)
            .entry(c.key.clone())
            .or_insert_with(|| Constraint {
                expression: expression.clone(),
                severity: c.severity.clone(),
                human: c.human.clone(),
            });
    }
}

/// Expand `foo[x]`: parent gets a `choices` declarer plus one branch element
/// per type, each carrying the value-level keywords of the `[x]` ED.
fn apply_choice(parent: &mut Node, base_name: &str, ed: &Ed, warnings: &mut Vec<String>) {
    if ed.max.as_deref() == Some("0") {
        push_unique(&mut parent.schema.excluded, base_name);
        return;
    }
    if ed.min.unwrap_or(0) >= 1 {
        push_unique(&mut parent.schema.required, base_name);
    }

    let mut branch_names = Vec::new();
    for t in &ed.types {
        let code = t.effective_code();
        let branch_name = format!("{base_name}{}", capitalize(&code));
        branch_names.push(branch_name.clone());

        let branch = parent
            .children
            .entry(branch_name.clone())
            .or_insert_with(|| Node::new(branch_name));
        branch.schema.type_ = Some(code);
        branch.schema.choice_of = Some(base_name.to_string());
        if !t.target_profile.is_empty() {
            branch.schema.refers = Some(t.target_profile.clone());
        }
        apply_value_keywords(&mut branch.schema, ed, warnings);
    }

    let declarer = parent
        .children
        .entry(base_name.to_string())
        .or_insert_with(|| Node::new(base_name.to_string()));
    match &mut declarer.schema.choices {
        Some(existing) => {
            for name in branch_names {
                if !existing.contains(&name) {
                    existing.push(name);
                }
            }
        }
        None => declarer.schema.choices = Some(branch_names),
    }
}

/// Recursively turn a tree node into a schema, pruning empty nodes.
pub(super) fn finalize(node: Node, warnings: &mut Vec<String>) -> Option<FhirSchema> {
    let Node {
        element_name: _,
        mut schema,
        type_profiles: _,
        children,
        slices,
        discriminators,
        slicing_rules,
        slicing_ordered,
        dead,
    } = node;

    if dead {
        return None;
    }

    // Slices on this node (non-sugar) become a slicing definition.
    if !slices.is_empty() {
        let sugar: Vec<&String> = slices
            .iter()
            .filter(|(_, s)| s.extension_profile.is_some())
            .map(|(n, _)| n)
            .collect();
        debug_assert!(
            sugar.is_empty(),
            "extension-sugar slices are lifted by the parent"
        );
        let slicing = super::slicing::build_slicing(
            slices,
            &discriminators,
            slicing_rules,
            slicing_ordered,
            warnings,
        );
        if let Some(slicing) = slicing {
            schema.slicing = Some(slicing);
        }
    }

    // Children: lift extension-sugar slices into our `extensions` map, then
    // finalize the rest into `elements`.
    let mut elements: IndexMap<String, Arc<FhirSchema>> = IndexMap::new();
    for (name, mut child) in children {
        // Extension sugar: slices on an extension child with a type profile
        // become entries in *this* schema's `extensions` map.
        if matches!(
            child.element_name.as_str(),
            "extension" | "modifierExtension"
        ) {
            let mut remaining: IndexMap<String, SliceNode> = IndexMap::new();
            for (slice_name, slice) in std::mem::take(&mut child.slices) {
                match &slice.extension_profile {
                    Some(profile) => {
                        let mut entry = finalize(slice.node, warnings).unwrap_or_default();
                        entry.url = Some(profile.clone());
                        entry.min = slice.min;
                        entry.max = slice.max;
                        schema
                            .extensions
                            .get_or_insert_with(IndexMap::new)
                            .insert(slice_name, Arc::new(entry));
                    }
                    None => {
                        remaining.insert(slice_name, slice);
                    }
                }
            }
            child.slices = remaining;
        }
        if let Some(child_schema) = finalize(child, warnings) {
            elements.insert(name, Arc::new(child_schema));
        }
    }
    if !elements.is_empty() {
        schema.elements = Some(elements);
    }

    if schema == FhirSchema::default() {
        None
    } else {
        Some(schema)
    }
}

fn parse_numeric_max(max: Option<&str>) -> Option<u64> {
    match max {
        Some("*") | None => None,
        Some(n) => n.parse().ok(),
    }
}

fn parse_content_reference(reference: &str) -> Option<Vec<String>> {
    let path = reference.strip_prefix('#')?;
    let mut parts = path.split('.');
    let root = parts.next()?;
    if root.is_empty() {
        return None;
    }
    let mut segments = vec![root.to_string()];
    for part in parts {
        if part.is_empty() {
            return None;
        }
        segments.push("elements".to_string());
        segments.push(part.to_string());
    }
    if segments.len() == 1 {
        return None; // reference to a whole root — not an element reference
    }
    Some(segments)
}

fn push_unique(list: &mut Option<Vec<String>>, value: &str) {
    let list = list.get_or_insert_with(Vec::new);
    if !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
