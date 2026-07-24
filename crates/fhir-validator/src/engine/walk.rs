//! The cooperative validation walk.
//!
//! Port of the FHIR Schema validation algorithm (`docs/algorythm.md` and the
//! reference validator `fhirschema-js/src/index.js`), with the divergences
//! called out inline. The behavioral contract is the vendored conformance
//! suite; where the reference validator and this port differ, no fixture
//! distinguishes them and the difference is documented.
//!
//! # Deterministic error-emission order
//!
//! 1. Schema sets iterate in insertion order, ancestors first: a schema's
//!    `base` chain is added before the schema itself (pinned by fixture
//!    `4_required.json`: the base's `required` errors precede the profile's).
//! 2. Within one schema, keyword validators run in the canonical order
//!    `fixed, pattern, constraints, required, excluded, binding, elements,
//!    choices` (the reference validator follows JSON key declaration order;
//!    no fixture distinguishes).
//! 3. Data keys are walked in document order (`serde_json`'s
//!    `preserve_order`), array items in index order.
//! 4. Node-level keyword errors precede per-key errors; choice-conflict
//!    errors (`postValidate`) come last for each node.
//!
//! # Known divergences from the reference validator
//!
//! - Unresolvable schema references produce an `unknown-schema` error and
//!   validation continues; the reference validator throws.
//! - `excluded` is enforced (the reference implementation crashes on it).
//! - Dynamic `Resource` schemas are resolved per array item; the reference
//!   validator leaks them into the shared element set across siblings.
//! - A choice branch declared by several layers is counted once per node in
//!   the multiple-choices-present check (the reference validator counts it
//!   once per layer, which would false-positive on real profiles).
//! - The choice-group *declarer* schema joins a branch element's set under
//!   the key `{layer}.{group}` (the reference validator uses `{layer}.{key}`,
//!   which silently overwrites the branch schema and loses its constraints).

use super::errors::{self, ErrorKind, Severity, ValidationError};
use super::path::PathTracker;
use super::{SyncOutcome, UnknownProfilePolicy, ValidationOptions};
use crate::effects::Deferred;
use crate::resolver::SchemaResolver;
use crate::schema::FhirSchema;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// The special element type resolved dynamically from `data.resourceType`.
const RESOURCE_TYPE_NAME: &str = "Resource";

/// A cooperative schema set (the *schemata*): insertion-ordered, ancestors
/// first, deduplicated both by resolution key and by schema identity.
#[derive(Clone, Default)]
pub(super) struct SchemaSet {
    map: IndexMap<String, Arc<FhirSchema>>,
    /// Identity dedup (`Arc` pointer): the same schema reachable under
    /// several aliases (name and canonical URL) is added once.
    seen: HashSet<usize>,
}

impl SchemaSet {
    fn new() -> Self {
        Self::default()
    }

    fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &Arc<FhirSchema>)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub(super) fn schemas(&self) -> impl Iterator<Item = &Arc<FhirSchema>> {
        self.map.values()
    }
}

pub(super) struct WalkCtx<'a> {
    resolver: &'a dyn SchemaResolver,
    errors: Vec<ValidationError>,
    deferred: Vec<Deferred>,
    pub(super) path: PathTracker,
}

impl WalkCtx<'_> {
    pub(super) fn error(&mut self, kind: ErrorKind, message: String) {
        self.errors.push(ValidationError::new(
            kind,
            self.path.render_dotted(),
            message,
        ));
    }

    fn error_with_severity(&mut self, kind: ErrorKind, message: String, severity: Severity) {
        self.errors.push(
            ValidationError::new(kind, self.path.render_dotted(), message).with_severity(severity),
        );
    }
}

pub(super) fn validate(
    resolver: &dyn SchemaResolver,
    resource: &Value,
    opts: &ValidationOptions,
) -> SyncOutcome {
    let resource_type = resource
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let mut ctx = WalkCtx {
        resolver,
        errors: Vec::new(),
        deferred: Vec::new(),
        path: PathTracker::new(resource_type),
    };

    // Root schema-set: resourceType, then meta.profile claims, then
    // caller-supplied profiles (the draft-algorithm order).
    let mut set = SchemaSet::new();
    if !resource_type.is_empty() {
        match resolver.resolve(resource_type) {
            Some(schema) => add_schemas_to_set(&mut ctx, &mut set, schema, resource_type),
            None => ctx.error(
                ErrorKind::UnknownSchema,
                errors::msg_unknown_schema(resource_type),
            ),
        }
    }
    if opts.use_meta_profiles {
        for profile in meta_profiles(resource) {
            add_profile(&mut ctx, &mut set, &profile, opts.unknown_profile);
        }
    }
    for profile in &opts.profiles {
        add_profile(&mut ctx, &mut set, profile, opts.unknown_profile);
    }

    validate_node(&mut ctx, &set, resource);

    SyncOutcome {
        errors: ctx.errors,
        deferred: ctx.deferred,
    }
}

/// `meta.profile` entries, in document order.
fn meta_profiles(resource: &Value) -> Vec<String> {
    resource
        .get("meta")
        .and_then(|m| m.get("profile"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn add_profile(
    ctx: &mut WalkCtx<'_>,
    set: &mut SchemaSet,
    profile: &str,
    policy: UnknownProfilePolicy,
) {
    match ctx.resolver.resolve(profile) {
        Some(schema) => add_schemas_to_set(ctx, set, schema, profile),
        None => match policy {
            UnknownProfilePolicy::Warn => ctx.error_with_severity(
                ErrorKind::UnknownProfile,
                errors::msg_unknown_profile(profile),
                Severity::Warning,
            ),
            UnknownProfilePolicy::Error => ctx.error(
                ErrorKind::UnknownProfile,
                errors::msg_unknown_profile(profile),
            ),
            UnknownProfilePolicy::Ignore => {}
        },
    }
}

/// Pull `schema` into `set` together with everything it references:
/// its `base` chain first (ancestors-first ordering), then its complex
/// `type` (skipped for primitive leaves), then any `elementReference`
/// target, then the schema itself.
pub(super) fn add_schemas_to_set(
    ctx: &mut WalkCtx<'_>,
    set: &mut SchemaSet,
    schema: Arc<FhirSchema>,
    key: &str,
) {
    let identity = Arc::as_ptr(&schema) as usize;
    if !set.seen.insert(identity) {
        return;
    }

    if let Some(base) = schema.base.clone()
        && !set.contains_key(&base)
    {
        match ctx.resolver.resolve(&base) {
            Some(base_schema) => add_schemas_to_set(ctx, set, base_schema, &base),
            None => ctx.error(ErrorKind::UnknownSchema, errors::msg_unknown_schema(&base)),
        }
    }

    if let Some(type_ref) = schema.type_.clone()
        && !schema.is_primitive()
        && !set.contains_key(&type_ref)
    {
        match ctx.resolver.resolve(&type_ref) {
            Some(type_schema) => add_schemas_to_set(ctx, set, type_schema, &type_ref),
            None => ctx.error(
                ErrorKind::UnknownSchema,
                errors::msg_unknown_schema(&type_ref),
            ),
        }
    }

    // `elementReference` pulls another element's schema into the set —
    // the mechanism behind recursive structures like Questionnaire.item.
    // Cycles terminate via the identity dedup above.
    if let Some(segments) = schema.element_reference.clone() {
        let ref_key = segments.join(".");
        match resolve_element_reference(ctx.resolver, &segments) {
            Some(target) => add_schemas_to_set(ctx, set, target, &ref_key),
            None => ctx.error(
                ErrorKind::UnknownSchema,
                errors::msg_unknown_schema(&ref_key),
            ),
        }
    }

    set.map.insert(key.to_string(), schema);
}

/// Navigate an element reference: `["Questionnaire", "elements", "item"]` —
/// a root schema name followed by alternating `"elements"` / element-name
/// segments.
fn resolve_element_reference(
    resolver: &dyn SchemaResolver,
    segments: &[String],
) -> Option<Arc<FhirSchema>> {
    let (root, rest) = segments.split_first()?;
    let mut current = resolver.resolve(root)?;
    let mut iter = rest.iter();
    while let Some(marker) = iter.next() {
        if marker != "elements" {
            return None;
        }
        let name = iter.next()?;
        let next = current.elements.as_ref()?.get(name)?.clone();
        current = next;
    }
    Some(current)
}

/// Validate one data node against a cooperative schema set.
pub(super) fn validate_node(ctx: &mut WalkCtx<'_>, set: &SchemaSet, data: &Value) {
    // Dynamic `Resource` resolution: an element typed `Resource` takes its
    // concrete schema from the nested `resourceType` at validation time
    // (Bundle.entry.resource, contained, Parameters.parameter.resource).
    let mut extended;
    let set = if data.is_object()
        && set
            .schemas()
            .any(|s| s.type_.as_deref() == Some(RESOURCE_TYPE_NAME))
    {
        extended = set.clone();
        if let Some(rt) = data.get("resourceType").and_then(Value::as_str) {
            match ctx.resolver.resolve(rt) {
                Some(schema) => add_schemas_to_set(ctx, &mut extended, schema, rt),
                None => ctx.error(ErrorKind::UnknownSchema, errors::msg_unknown_schema(rt)),
            }
        }
        // No resourceType on a Resource-typed node: nothing to resolve; the
        // static layers (e.g. the base `Resource` schema) still apply.
        &extended
    } else {
        set
    };

    eval_validators(ctx, set, data);

    let Some(obj) = data.as_object() else {
        return;
    };

    // Choice branches seen on this node, keyed by choice group, in first-
    // encounter order.
    let mut multi_choice: IndexMap<String, Vec<String>> = IndexMap::new();

    for (key, value) in obj {
        ctx.path.push_key(key);
        // `_foo` sidecars carry the Element part (id/extension) of a
        // primitive `foo` — they pair with the base element rather than
        // resolving as elements themselves.
        if let Some(base_key) = key.strip_prefix('_')
            && !base_key.is_empty()
        {
            handle_sidecar(ctx, set, key, base_key, value);
        } else {
            eval_element(ctx, set, &mut multi_choice, key, value);
        }
        ctx.path.pop();
    }

    // postValidate: more than one branch of the same choice group present.
    for (group, branches) in &multi_choice {
        if branches.len() > 1 {
            ctx.path.push_key(group);
            ctx.error(ErrorKind::Choice, errors::msg_choice(group, branches));
            ctx.path.pop();
        }
    }
}

/// Run every keyword validator of every schema in the set against the node.
fn eval_validators(ctx: &mut WalkCtx<'_>, set: &SchemaSet, data: &Value) {
    // Clone the Arcs up front: keyword validators need `&mut ctx` while the
    // set is borrowed, and Arc clones are cheap.
    let schemas: Vec<Arc<FhirSchema>> = set.schemas().cloned().collect();
    for schema in &schemas {
        if schema.is_primitive() {
            super::primitives::validate_primitive(ctx, schema, data);
        }
        if let Some(fixed) = &schema.fixed
            && data != fixed
        {
            ctx.error(ErrorKind::FixedValue, errors::msg_fixed_value(fixed, data));
        }
        if let Some(pattern) = &schema.pattern
            && !is_partial_match(data, pattern)
        {
            ctx.error(
                ErrorKind::PatternValue,
                errors::msg_pattern_value(pattern, data),
            );
        }
        if let Some(constraints) = &schema.constraints {
            let path = ctx.path.render_dotted();
            for (id, c) in constraints {
                ctx.deferred.push(Deferred::Constraint {
                    path: path.clone(),
                    id: id.clone(),
                    expression: c.expression.clone(),
                    human: c.human.clone(),
                    severity: c.severity.clone(),
                });
            }
        }
        if let Some(required) = &schema.required {
            validate_required(ctx, set, required, data);
        }
        if let Some(excluded) = &schema.excluded {
            validate_excluded(ctx, excluded, data);
        }
        if let Some(binding) = &schema.binding {
            ctx.deferred.push(Deferred::Binding {
                path: ctx.path.render_dotted(),
                binding: binding.clone(),
                value: data.clone(),
                type_hint: schema.type_.clone(),
            });
        }
        if schema.elements.is_some() && !data.is_object() {
            ctx.error(
                ErrorKind::Type,
                errors::msg_expected_object_comma(errors::json_type_name(data)),
            );
        }
        if let Some(choices) = &schema.choices {
            // Runs when a choice-group declarer joins a branch element's
            // set; the branch key is the last named path segment.
            if let Some(branch) = ctx.path.last_key()
                && !choices.iter().any(|c| c == branch)
            {
                let message = errors::msg_choice_excluded(branch);
                ctx.error(ErrorKind::ChoiceExcluded, message);
            }
        }
    }
}

fn validate_required(ctx: &mut WalkCtx<'_>, set: &SchemaSet, required: &[String], data: &Value) {
    let Some(obj) = data.as_object() else {
        ctx.error(
            ErrorKind::Type,
            errors::msg_expected_object_no_comma(errors::json_type_name(data)),
        );
        return;
    };
    for key in required {
        ctx.path.push_key(key);
        if !obj.contains_key(key) && !choice_branch_satisfies(set, key, obj) {
            ctx.error(ErrorKind::Required, errors::msg_required(key));
        }
        ctx.path.pop();
    }
}

/// Helios extension: a required key that is a choice group (`foo` for
/// `foo[x]`) is satisfied by any declared branch (`fooBoolean`, ...) being
/// present. The converter maps `min >= 1` on `foo[x]` to `required: ["foo"]`,
/// and the literal key never appears in data. No upstream fixture combines
/// `required` with choices, so conformance is unaffected.
fn choice_branch_satisfies(
    set: &SchemaSet,
    key: &str,
    obj: &serde_json::Map<String, Value>,
) -> bool {
    set.schemas().any(|schema| {
        schema
            .elements
            .as_ref()
            .and_then(|els| els.get(key))
            .and_then(|el| el.choices.as_ref())
            .is_some_and(|branches| branches.iter().any(|b| obj.contains_key(b)))
    })
}

fn validate_excluded(ctx: &mut WalkCtx<'_>, excluded: &[String], data: &Value) {
    let Some(obj) = data.as_object() else {
        ctx.error(
            ErrorKind::Type,
            errors::msg_expected_object_no_comma(errors::json_type_name(data)),
        );
        return;
    };
    for key in excluded {
        ctx.path.push_key(key);
        if obj.contains_key(key) {
            ctx.error(ErrorKind::Excluded, errors::msg_excluded(key));
        }
        ctx.path.pop();
    }
}

/// Validate one data key against the element schemas collected from every
/// layer of the set.
fn eval_element(
    ctx: &mut WalkCtx<'_>,
    set: &SchemaSet,
    multi_choice: &mut IndexMap<String, Vec<String>>,
    key: &str,
    value: &Value,
) {
    let mut elset = SchemaSet::new();
    collect_schemas_for_element(ctx, set, &mut elset, multi_choice, key);

    if elset.is_empty() {
        ctx.error(ErrorKind::UnknownElement, errors::msg_unknown_element(key));
        return;
    }

    // Array-ness is cooperative: an element is an array if ANY layer says so.
    let expect_array = elset.schemas().any(|s| s.array == Some(true));
    match (expect_array, value.is_array()) {
        (true, false) => ctx.error(ErrorKind::NotArray, errors::msg_not_array(key)),
        (false, true) => ctx.error(ErrorKind::NotSingular, errors::msg_not_singular(key)),
        _ => {}
    }
    // The walk continues after a shape mismatch, mirroring the reference
    // validator (the mismatched value still validates against the set).

    if let Some(items) = value.as_array() {
        // Whole-collection keywords, per schema: min then max.
        let schemas: Vec<Arc<FhirSchema>> = elset.schemas().cloned().collect();
        for schema in &schemas {
            if let Some(min) = schema.min
                && (items.len() as u64) < min
            {
                ctx.error(ErrorKind::Min, errors::msg_min(min, items.len()));
            }
            if let Some(max) = schema.max
                && (items.len() as u64) > max
            {
                ctx.error(ErrorKind::Max, errors::msg_max(max, items.len()));
            }
        }

        // Slicing (mark/sweep). Items validated cooperatively with their
        // matched slice schemas are consumed — the plain per-item pass below
        // skips them so base-set errors are not emitted twice.
        let consumed = super::slicing::validate_slices(ctx, &elset, items);

        for (index, item) in items.iter().enumerate() {
            if consumed.contains(&index) {
                continue;
            }
            ctx.path.push_index(index);
            validate_node(ctx, &elset, item);
            ctx.path.pop();
        }
    } else {
        validate_node(ctx, &elset, value);
    }
}

/// Validate a `_foo` primitive-extension sidecar.
///
/// The sidecar pairs with the base element `foo`: it is only legal when some
/// layer declares `foo` with a primitive type, it mirrors `foo`'s
/// array-vs-singular shape, and its content (per item for arrays, with
/// `null` gaps allowed) validates against the resolver's `Element` schema.
fn handle_sidecar(
    ctx: &mut WalkCtx<'_>,
    set: &SchemaSet,
    key: &str,
    base_key: &str,
    value: &Value,
) {
    // Collect the base element's sub-set; choice bookkeeping is discarded
    // (a sidecar is not a branch occurrence).
    let mut scratch: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut base_elset = SchemaSet::new();
    collect_schemas_for_element(ctx, set, &mut base_elset, &mut scratch, base_key);

    // Stray sidecar, or sidecar on a non-primitive element: unknown.
    if base_elset.is_empty() || !base_elset.schemas().any(|s| s.is_primitive()) {
        ctx.error(ErrorKind::UnknownElement, errors::msg_unknown_element(key));
        return;
    }

    // The sidecar mirrors the base element's shape.
    let expect_array = base_elset.schemas().any(|s| s.array == Some(true));
    match (expect_array, value.is_array()) {
        (true, false) => ctx.error(ErrorKind::NotArray, errors::msg_not_array(key)),
        (false, true) => ctx.error(ErrorKind::NotSingular, errors::msg_not_singular(key)),
        _ => {}
    }

    // Validate the Element part(s). Without a resolvable `Element` schema
    // (core packs always carry one) the content is left unchecked.
    let Some(element_schema) = ctx.resolver.resolve("Element") else {
        return;
    };
    let mut element_set = SchemaSet::new();
    add_schemas_to_set(ctx, &mut element_set, element_schema, "Element");

    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                if item.is_null() {
                    continue;
                }
                ctx.path.push_index(index);
                validate_node(ctx, &element_set, item);
                ctx.path.pop();
            }
        }
        Value::Null => {}
        other => validate_node(ctx, &element_set, other),
    }
}

/// Gather the element sub-set for `key` across all layers: each layer's
/// `elements[key]` (plus its transitive `base`/`type`), and — when the key
/// is a choice branch — every layer's declarer schema for the choice group.
fn collect_schemas_for_element(
    ctx: &mut WalkCtx<'_>,
    set: &SchemaSet,
    elset: &mut SchemaSet,
    multi_choice: &mut IndexMap<String, Vec<String>>,
    key: &str,
) {
    let layers: Vec<(String, Arc<FhirSchema>)> = set
        .iter()
        .map(|(name, schema)| (name.to_string(), Arc::clone(schema)))
        .collect();

    // Every layer's `extensions` map compiles into a synthetic slicing on
    // the `extension` element, matched by `{url}` — authoring sugar rewritten
    // into the core slicing primitive at validation time.
    if key == "extension" {
        compile_extensions(ctx, &layers, elset);
    }

    let mut choice_group: Option<String> = None;
    for (layer_name, layer) in &layers {
        let Some(subschema) = layer.elements.as_ref().and_then(|els| els.get(key)) else {
            continue;
        };
        // Choice-group declarers are not element schemas for their own key:
        // a bare group key in data (e.g. `"choice": ...`) is unknown.
        if subschema.choices.is_none() {
            let subset_key = format!("{layer_name}.{key}");
            add_schemas_to_set(ctx, elset, Arc::clone(subschema), &subset_key);
        }
        if let Some(group) = &subschema.choice_of {
            choice_group = Some(group.clone());
            let entry = multi_choice.entry(group.clone()).or_default();
            // Count each branch once per node, even when several layers
            // declare it.
            if !entry.iter().any(|k| k == key) {
                entry.push(key.to_string());
            }
        }
    }

    // The key is a choice branch: every layer's declarer for the group joins
    // the branch's set, so `choices` narrowing applies cooperatively.
    if let Some(group) = choice_group {
        for (layer_name, layer) in &layers {
            if let Some(declarer) = layer.elements.as_ref().and_then(|els| els.get(&group)) {
                let subset_key = format!("{layer_name}.{group}");
                add_schemas_to_set(ctx, elset, Arc::clone(declarer), &subset_key);
            }
        }
    }
}

/// Compile `extensions` maps from every layer into one synthetic
/// slicing-bearing schema on the `extension` element. Each entry becomes a
/// slice matched by `{url: <ext.url>}` carrying the entry's `min`/`max`; its
/// slice schema is the entry (minus url/min/max) overlaid by the schema the
/// extension's url resolves to.
///
/// An unresolvable extension url is reported as `unknown-schema` and the
/// entry-only schema is used (the reference validator throws instead).
fn compile_extensions(
    ctx: &mut WalkCtx<'_>,
    layers: &[(String, Arc<FhirSchema>)],
    elset: &mut SchemaSet,
) {
    use crate::schema::{Match, Slice, Slicing};

    let mut slices: IndexMap<String, Slice> = IndexMap::new();
    for (_, layer) in layers {
        let Some(extensions) = &layer.extensions else {
            continue;
        };
        for (ext_name, entry) in extensions {
            let Some(url) = entry.url.clone() else {
                continue;
            };
            let resolved = ctx.resolver.resolve(&url);
            if resolved.is_none() {
                ctx.error(ErrorKind::UnknownSchema, errors::msg_unknown_schema(&url));
            }
            let merged = merge_extension_schema(entry, resolved.as_deref());
            slices.insert(
                ext_name.clone(),
                Slice {
                    match_: Some(Match {
                        type_: Some("pattern".to_string()),
                        value: Some(serde_json::json!({ "url": url })),
                        resolve_ref: None,
                    }),
                    min: entry.min,
                    max: entry.max,
                    order: None,
                    reslice: None,
                    slice_is_constraining: None,
                    schema: Some(Arc::new(merged)),
                },
            );
        }
    }

    if !slices.is_empty() {
        let synthetic = FhirSchema {
            slicing: Some(Slicing {
                slices,
                rules: None,
                ordered: None,
            }),
            ..Default::default()
        };
        add_schemas_to_set(ctx, elset, Arc::new(synthetic), "extension");
    }
}

/// Shallow overlay merge for extension slice schemas, mirroring the
/// reference validator's `Object.assign(entry-minus-url/min/max, resolved)`:
/// fields present on the resolved extension schema win; entry fields fill
/// the gaps.
fn merge_extension_schema(entry: &FhirSchema, resolved: Option<&FhirSchema>) -> FhirSchema {
    let mut out = entry.clone();
    out.url = None;
    out.min = None;
    out.max = None;

    let Some(r) = resolved else {
        return out;
    };

    macro_rules! overlay {
        ($($field:ident),* $(,)?) => {
            $(if r.$field.is_some() {
                out.$field = r.$field.clone();
            })*
        };
    }
    overlay!(
        url,
        name,
        base,
        kind,
        derivation,
        type_,
        array,
        scalar,
        min,
        max,
        elements,
        required,
        excluded,
        element_reference,
        choices,
        choice_of,
        fixed,
        pattern,
        binding,
        constraints,
        refers,
        slicing,
        extensions,
        modifier,
        must_support,
        summary,
        regex,
    );
    out
}

/// Lodash-style `_.isMatch`: partial deep match. Every key present in
/// `pattern` must exist in `data` and match recursively; extra data keys are
/// permitted. Arrays match index-wise as a prefix; scalars by equality.
pub(crate) fn is_partial_match(data: &Value, pattern: &Value) -> bool {
    match pattern {
        Value::Object(pattern_map) => {
            let Some(data_map) = data.as_object() else {
                return false;
            };
            pattern_map
                .iter()
                .all(|(k, pv)| data_map.get(k).is_some_and(|dv| is_partial_match(dv, pv)))
        }
        Value::Array(pattern_items) => {
            let Some(data_items) = data.as_array() else {
                return false;
            };
            pattern_items.len() <= data_items.len()
                && pattern_items
                    .iter()
                    .zip(data_items.iter())
                    .all(|(pv, dv)| is_partial_match(dv, pv))
        }
        scalar => data == scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_match_semantics() {
        assert!(is_partial_match(
            &json!({"use": "home", "city": "X"}),
            &json!({"use": "home"})
        ));
        assert!(!is_partial_match(
            &json!({"use": "work"}),
            &json!({"use": "home"})
        ));
        assert!(!is_partial_match(&json!("home"), &json!({"use": "home"})));
        assert!(is_partial_match(
            &json!({"a": {"b": 1, "c": 2}}),
            &json!({"a": {"b": 1}})
        ));
        assert!(is_partial_match(&json!([1, 2, 3]), &json!([1, 2])));
        assert!(!is_partial_match(&json!([2, 1]), &json!([1])));
        assert!(is_partial_match(&json!(5), &json!(5)));
    }
}
