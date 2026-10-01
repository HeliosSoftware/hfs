//! The one patch applier.
//!
//! `PATCH [type]/[id]` (applied by the REST layer) and `PATCH [type]?criteria`
//! (applied by [`ConditionalStorage::conditional_patch`]) both go through
//! [`apply_patch`], so a patch document means the same thing on either
//! endpoint and on every backend (#1406). Before, REST, SQLite and PostgreSQL
//! each carried a private copy.
//!
//! # Formats
//!
//! * JSON Patch (RFC 6902) and JSON Merge Patch (RFC 7396) are applied with
//!   the `json-patch` crate.
//! * FHIRPath Patch locates editable JSON nodes while evaluating navigation
//!   against the stored resource's FHIR version. Computed FHIRPath results
//!   cannot be written back and are rejected.
//!
//! # Invariants
//!
//! A patch cannot change or remove `resourceType` or `id`: they are the
//! identity of the resource the request named. Backends re-assert both on
//! every `update`, so such a patch could not corrupt a row — it would be
//! silently undone and answered as a success, which is why it is refused
//! here instead.
//!
//! `meta.versionId` and `meta.lastUpdated` are not guarded. They are
//! server-assigned on every write, exactly as for a `PUT` whose body carries
//! them, so a patch naming them has no lasting effect.
//!
//! [`ConditionalStorage::conditional_patch`]: super::ConditionalStorage::conditional_patch

use base64::{Engine as _, engine::general_purpose::STANDARD};
use helios_fhir::FhirVersion;
use helios_fhirpath::evaluator::{EvaluationContext, evaluate};
use helios_fhirpath::parser::{Expression, Invocation, Literal, Term};
use helios_fhirpath_support::EvaluationResult;
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use thiserror::Error;

use super::PatchFormat;

/// Why a patch was not applied. Nothing has been written when one is returned.
///
/// The variants let the REST layer distinguish syntax, failed preconditions,
/// and operations that cannot apply to the current resource.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatchError {
    /// The patch document is not a document of its format — for JSON Patch,
    /// not an array of RFC 6902 operations.
    #[error("Invalid {format}: {message}")]
    MalformedDocument {
        /// The format, as a client would name it (`JSON Patch`).
        format: &'static str,
        /// The parser's complaint.
        message: String,
    },

    /// A JSON Patch `test` operation compared unequal (RFC 6902 §4.6). The
    /// document is well-formed and every path resolved; the resource is just
    /// not in the state the client expected.
    #[error("Failed to apply JSON Patch: {message}")]
    TestFailed {
        /// Which operation failed, and where.
        message: String,
    },

    /// A JSON Patch operation could not be carried out against this resource:
    /// a path that does not resolve, a `move` into its own source.
    #[error("Failed to apply JSON Patch: {message}")]
    OperationFailed {
        /// Which operation failed, and where.
        message: String,
    },

    /// The patch changes or removes an element that identifies the resource.
    #[error("Cannot change {element} via patch")]
    ImmutableElement {
        /// `resourceType` or `id`.
        element: &'static str,
    },

    /// The format is recognised but unavailable for this interaction/version.
    #[error("{format} is not implemented")]
    UnsupportedFormat {
        /// The format, as a client would name it (`FHIRPath Patch`).
        format: &'static str,
    },
}

/// The elements a patch may neither change nor remove.
const IMMUTABLE_ELEMENTS: [&str; 2] = ["resourceType", "id"];

/// Applies `patch` to `current` and returns the patched content.
///
/// `current` is the content of the resource as stored; it is not modified.
/// The result still has the `resourceType` and `id` of `current` (see the
/// module docs), and is otherwise unvalidated: the caller writes it with
/// [`ResourceStorage::update`](super::ResourceStorage::update) like any other
/// content.
pub fn apply_patch(current: &Value, patch: &PatchFormat) -> Result<Value, PatchError> {
    apply_patch_inner(current, patch, None)
}

/// Apply a patch using the FHIR version of the resource being patched.
/// FHIRPath Patch requires this entry point; the version selects choice-field
/// metadata and the semantics of predicates in navigation expressions.
pub fn apply_patch_for_version(
    current: &Value,
    patch: &PatchFormat,
    version: FhirVersion,
) -> Result<Value, PatchError> {
    apply_patch_inner(current, patch, Some(version))
}

fn apply_patch_inner(
    current: &Value,
    patch: &PatchFormat,
    version: Option<FhirVersion>,
) -> Result<Value, PatchError> {
    let mut patched = current.clone();

    match patch {
        PatchFormat::JsonPatch(operations) => {
            let operations: json_patch::Patch = serde_json::from_value(operations.clone())
                .map_err(|e| PatchError::MalformedDocument {
                    format: "JSON Patch",
                    message: e.to_string(),
                })?;

            json_patch::patch(&mut patched, &operations).map_err(|e| {
                let message = e.to_string();
                match e.kind {
                    json_patch::PatchErrorKind::TestFailed => PatchError::TestFailed { message },
                    _ => PatchError::OperationFailed { message },
                }
            })?;
        }
        PatchFormat::MergePatch(merge_doc) => json_patch::merge(&mut patched, merge_doc),
        PatchFormat::FhirPathPatch(parameters) => {
            let version = version.ok_or(PatchError::UnsupportedFormat {
                format: "FHIRPath Patch without a FHIR version",
            })?;
            apply_fhirpath_patch(&mut patched, parameters, version)?;
        }
    }

    for element in IMMUTABLE_ELEMENTS {
        // Compared as JSON: a resource stored without the element (nothing
        // requires `current` to carry its id) may not gain one either.
        if patched.get(element) != current.get(element) {
            return Err(PatchError::ImmutableElement { element });
        }
    }

    Ok(patched)
}

/// Decode a FHIR Bundle PATCH entry's `resource` into the format understood by
/// the shared applier. FHIRPath Parameters works in all supported versions;
/// Binary containing JSON Patch is specified for R5 and R6 only. A Bundle has
/// no encoding for the direct endpoint's JSON Merge Patch media type.
pub fn decode_bundle_patch_resource(
    resource: &Value,
    version: FhirVersion,
) -> Result<PatchFormat, PatchError> {
    match resource.get("resourceType").and_then(Value::as_str) {
        Some("Parameters") => {
            parse_operations(resource)?;
            Ok(PatchFormat::FhirPathPatch(resource.clone()))
        }
        Some("Binary") if matches!(version.as_str(), "R5" | "R6") => {
            let content_type = resource
                .get("contentType")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("Binary.contentType is required"))?;
            if content_type.split(';').next().map(str::trim) != Some("application/json-patch+json")
            {
                return Err(malformed(
                    "Binary.contentType must be application/json-patch+json",
                ));
            }
            let encoded = resource
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("Binary.data is required"))?;
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|e| malformed(format!("Invalid Binary.data base64: {e}")))?;
            let document: Value = serde_json::from_slice(&bytes)
                .map_err(|e| malformed(format!("Invalid Binary JSON Patch: {e}")))?;
            let _: json_patch::Patch = serde_json::from_value(document.clone())
                .map_err(|e| malformed(format!("Invalid Binary JSON Patch: {e}")))?;
            Ok(PatchFormat::JsonPatch(document))
        }
        Some("Binary") => Err(PatchError::UnsupportedFormat {
            format: "Binary JSON Patch in R4/R4B Bundle",
        }),
        _ => Err(malformed(
            "Bundle PATCH resource must be Parameters or Binary",
        )),
    }
}

fn malformed(message: impl Into<String>) -> PatchError {
    PatchError::MalformedDocument {
        format: "FHIRPath Patch",
        message: message.into(),
    }
}

fn failed(message: impl Into<String>) -> PatchError {
    PatchError::OperationFailed {
        message: message.into(),
    }
}

#[derive(Debug)]
struct Operation {
    kind: String,
    path: String,
    name: Option<String>,
    value: Option<PartValue>,
    index: Option<usize>,
    source: Option<usize>,
    destination: Option<usize>,
}

#[derive(Debug)]
struct PartValue {
    value: Value,
    type_suffix: Option<String>,
    nested: Option<Vec<(String, PartValue)>>,
}

fn parse_operations(parameters: &Value) -> Result<Vec<Operation>, PatchError> {
    if parameters.get("resourceType").and_then(Value::as_str) != Some("Parameters") {
        return Err(malformed("FHIRPath patch must be a Parameters resource"));
    }
    let entries = parameters
        .get("parameter")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("Parameters.parameter must be an array"))?;
    if entries.is_empty() {
        return Err(malformed("Parameters.parameter must contain an operation"));
    }
    entries.iter().map(parse_operation).collect()
}

fn parse_operation(entry: &Value) -> Result<Operation, PatchError> {
    if entry.get("name").and_then(Value::as_str) != Some("operation") {
        return Err(malformed("Each parameter must be named operation"));
    }
    let parts = entry
        .get("part")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("operation.part must be an array"))?;
    let mut fields = std::collections::HashMap::new();
    for part in parts {
        let name = part
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("operation part needs a name"))?;
        if fields.insert(name, part).is_some() {
            return Err(malformed(format!("Duplicate operation part {name}")));
        }
    }
    let required_string = |name: &str, key: &str| -> Result<String, PatchError> {
        fields
            .get(name)
            .and_then(|part| part.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| malformed(format!("operation.{name}.{key} is required")))
    };
    let optional_index = |name: &str| -> Result<Option<usize>, PatchError> {
        fields
            .get(name)
            .map(|part| {
                part.get("valueInteger")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| {
                        malformed(format!("operation.{name}.valueInteger must be nonnegative"))
                    })
            })
            .transpose()
    };
    let kind = required_string("type", "valueCode")?;
    if !matches!(
        kind.as_str(),
        "add" | "insert" | "delete" | "replace" | "move"
    ) {
        return Err(malformed(format!(
            "Unknown FHIRPath Patch operation {kind}"
        )));
    }
    let path = required_string("path", "valueString")?;
    let name = fields
        .get("name")
        .map(|_| required_string("name", "valueString"))
        .transpose()?;
    let value = fields
        .get("value")
        .map(|part| parse_part_value(part))
        .transpose()?;
    let operation = Operation {
        kind,
        path,
        name,
        value,
        index: optional_index("index")?,
        source: optional_index("source")?,
        destination: optional_index("destination")?,
    };
    let valid = match operation.kind.as_str() {
        "add" => operation.name.is_some() && operation.value.is_some(),
        "insert" => operation.index.is_some() && operation.value.is_some(),
        "delete" => true,
        "replace" => operation.value.is_some(),
        "move" => operation.source.is_some() && operation.destination.is_some(),
        _ => unreachable!(),
    };
    if !valid {
        return Err(malformed(format!(
            "Missing parts for {} operation",
            operation.kind
        )));
    }
    let allowed: &[&str] = match operation.kind.as_str() {
        "add" => &["type", "path", "name", "value"],
        "insert" => &["type", "path", "value", "index"],
        "delete" => &["type", "path"],
        "replace" => &["type", "path", "value"],
        "move" => &["type", "path", "source", "destination"],
        _ => unreachable!(),
    };
    if let Some(unexpected) = fields.keys().find(|name| !allowed.contains(name)) {
        return Err(malformed(format!(
            "Unexpected {} operation part {unexpected}",
            operation.kind
        )));
    }
    Ok(operation)
}

fn parse_part_value(part: &Value) -> Result<PartValue, PatchError> {
    let object = part
        .as_object()
        .ok_or_else(|| malformed("value part must be an object"))?;
    let typed = object
        .iter()
        .filter(|(key, _)| {
            key.starts_with("value") && key.len() > 5 && key.as_bytes()[5].is_ascii_uppercase()
        })
        .collect::<Vec<_>>();
    let nested = object.get("part");
    if typed.len() + usize::from(nested.is_some()) != 1 {
        return Err(malformed(
            "value part needs exactly one value[x] or nested part",
        ));
    }
    if let Some((key, value)) = typed.first() {
        return Ok(PartValue {
            value: (*value).clone(),
            type_suffix: Some(key[5..].to_string()),
            nested: None,
        });
    }
    let children = nested
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("value.part must be an array"))?;
    let mut parsed_children = Vec::new();
    for child in children {
        let name = child
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("nested value part needs a name"))?;
        parsed_children.push((name.to_string(), parse_part_value(child)?));
    }
    Ok(PartValue {
        value: Value::Null,
        type_suffix: None,
        nested: Some(parsed_children),
    })
}

fn materialize_value(
    part: PartValue,
    expected_type: Option<&str>,
    version: FhirVersion,
) -> Result<Value, PatchError> {
    let Some(children) = part.nested else {
        return Ok(part.value);
    };
    let mut object = Map::new();
    for (name, child) in children {
        let key = choice_key(&name, &child, expected_type, version)?;
        let child_type = expected_type
            .and_then(|parent| helios_fhir::get_field_type(version, parent, &key))
            .map(|(ty, _)| ty);
        let is_collection = expected_type
            .and_then(|parent| helios_fhir::get_field_type(version, parent, &key))
            .is_some_and(|(_, collection)| collection);
        let value = materialize_value(child, child_type, version)?;
        if is_collection {
            match object.entry(key) {
                serde_json::map::Entry::Vacant(entry) => {
                    entry.insert(Value::Array(vec![value]));
                }
                serde_json::map::Entry::Occupied(mut entry) => {
                    entry
                        .get_mut()
                        .as_array_mut()
                        .expect("collection array")
                        .push(value);
                }
            }
        } else if object.insert(key.clone(), value).is_some() {
            return Err(malformed(format!("Duplicate nested value part {key}")));
        }
    }
    Ok(Value::Object(object))
}

type JsonPath = Vec<PathSegment>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone)]
struct NodeRef {
    path: JsonPath,
    fhir_type: Option<String>,
}

#[derive(Debug, Default)]
struct Selection {
    nodes: Vec<NodeRef>,
    /// The actual JSON arrays traversed by the final member selection.
    arrays: Vec<JsonPath>,
}

fn apply_fhirpath_patch(
    resource: &mut Value,
    parameters: &Value,
    version: FhirVersion,
) -> Result<(), PatchError> {
    let identity = [
        resource.get("resourceType").cloned(),
        resource.get("id").cloned(),
    ];
    for operation in parse_operations(parameters)? {
        let expression = helios_fhirpath::parse_expression(&operation.path)
            .map_err(|e| malformed(format!("Invalid FHIRPath path: {e}")))?;
        let selected = locate(resource, &expression, version)?;
        match operation.kind.as_str() {
            "add" => {
                let node = exactly_one(selected.nodes, "add parent")?;
                let parent = get_mut(resource, &node.path)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| failed("add path must select an object"))?;
                let name = operation.name.as_deref().expect("parsed add name");
                let value = operation.value.expect("parsed add value");
                let key = choice_key(name, &value, node.fhir_type.as_deref(), version)?;
                let field = node.fhir_type.as_deref().and_then(|parent_type| {
                    helios_fhir::get_field_type(version, parent_type, &key)
                });
                let content = materialize_value(value, field.map(|(ty, _)| ty), version)?;
                match parent.get_mut(&key) {
                    Some(Value::Array(items)) => items.push(content),
                    Some(_) => return Err(failed(format!("add element {key} already exists"))),
                    None if field.is_some_and(|(_, repeating)| repeating) => {
                        parent.insert(key, Value::Array(vec![content]));
                    }
                    None => {
                        parent.insert(key, content);
                    }
                }
            }
            "insert" => {
                let array_path = exactly_one(selected.arrays, "insert collection")?;
                let field_type = field_type_for_path(resource, &array_path, version);
                let array = get_mut(resource, &array_path)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| failed("insert path must select an array"))?;
                let index = operation.index.expect("parsed insert index");
                if index > array.len() {
                    return Err(failed(format!(
                        "insert index {index} exceeds collection length {}",
                        array.len()
                    )));
                }
                let value = materialize_value(
                    operation.value.expect("parsed insert value"),
                    field_type.as_deref(),
                    version,
                )?;
                array.insert(index, value);
            }
            "delete" => {
                if selected.nodes.is_empty() {
                    continue;
                }
                let node = exactly_one(selected.nodes, "delete target")?;
                remove_at(resource, &node.path)?;
            }
            "replace" => {
                let node = exactly_one(selected.nodes, "replace target")?;
                let value = operation.value.expect("parsed replace value");
                if let Some(base) = final_member(&expression)
                    && let Some(PathSegment::Key(old_key)) = node.path.last()
                    && old_key != base
                    && let Some(parent_path) = node.path.get(..node.path.len() - 1)
                    && let Some(parent_type) = parent_type_for_path(resource, parent_path, version)
                {
                    let new_key = choice_key(base, &value, Some(&parent_type), version)?;
                    if new_key != *old_key {
                        let parent = get_mut(resource, parent_path)
                            .and_then(Value::as_object_mut)
                            .ok_or_else(|| failed("replace choice parent is not an object"))?;
                        if parent.contains_key(&new_key) {
                            return Err(failed(format!(
                                "replace choice element {new_key} already exists"
                            )));
                        }
                        let field_type =
                            helios_fhir::get_field_type(version, &parent_type, &new_key)
                                .map(|(ty, _)| ty);
                        let content = materialize_value(value, field_type, version)?;
                        parent.remove(old_key);
                        parent.insert(new_key, content);
                        continue;
                    }
                }
                let content = materialize_value(value, node.fhir_type.as_deref(), version)?;
                *get_mut(resource, &node.path)
                    .ok_or_else(|| failed("replace target disappeared"))? = content;
            }
            "move" => {
                let array_path = exactly_one(selected.arrays, "move collection")?;
                let array = get_mut(resource, &array_path)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| failed("move path must select an array"))?;
                let source = operation.source.expect("parsed move source");
                let destination = operation.destination.expect("parsed move destination");
                if source >= array.len() || destination >= array.len() {
                    return Err(failed(format!(
                        "move indexes {source}, {destination} exceed collection length {}",
                        array.len()
                    )));
                }
                let value = array.remove(source);
                array.insert(destination, value);
            }
            _ => unreachable!("operation kind validated by parser"),
        }
        for (element, original) in IMMUTABLE_ELEMENTS.into_iter().zip(identity.iter()) {
            if resource.get(element) != original.as_ref() {
                return Err(PatchError::ImmutableElement { element });
            }
        }
    }
    Ok(())
}

fn exactly_one<T>(items: Vec<T>, description: &str) -> Result<T, PatchError> {
    if items.len() != 1 {
        return Err(failed(format!(
            "{description} must resolve to one editable node, found {}",
            items.len()
        )));
    }
    Ok(items.into_iter().next().expect("one item"))
}

fn choice_key(
    name: &str,
    value: &PartValue,
    parent_type: Option<&str>,
    version: FhirVersion,
) -> Result<String, PatchError> {
    let base = name.strip_suffix("[x]").unwrap_or(name);
    let Some(suffix) = value.type_suffix.as_deref() else {
        if name.ends_with("[x]") {
            return Err(malformed("choice element needs a typed value[x]"));
        }
        return Ok(base.to_string());
    };
    let candidate = format!("{base}{suffix}");
    if parent_type
        .and_then(|ty| helios_fhir::get_field_type(version, ty, &candidate))
        .is_some()
    {
        return Ok(candidate);
    }
    if name.ends_with("[x]") {
        return Err(failed(format!(
            "{candidate} is not a choice field of this element"
        )));
    }
    Ok(base.to_string())
}

fn locate(
    resource: &Value,
    expression: &Expression,
    version: FhirVersion,
) -> Result<Selection, PatchError> {
    if contains_resolve(expression) {
        return Err(failed("PATCH path cannot traverse resolve()"));
    }
    let root = Selection {
        nodes: vec![NodeRef {
            path: vec![],
            fhir_type: resource
                .get("resourceType")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }],
        arrays: vec![],
    };
    locate_from(resource, expression, version, root)
}

fn contains_resolve(expression: &Expression) -> bool {
    match expression {
        Expression::Term(Term::Invocation(Invocation::Function(name, args))) => {
            name == "resolve" || args.iter().any(contains_resolve)
        }
        Expression::Term(Term::Parenthesized(inner)) => contains_resolve(inner),
        Expression::Invocation(base, Invocation::Function(name, args)) => {
            name == "resolve" || contains_resolve(base) || args.iter().any(contains_resolve)
        }
        Expression::Invocation(base, _)
        | Expression::Polarity(_, base)
        | Expression::Type(base, _, _) => contains_resolve(base),
        Expression::Indexer(left, right)
        | Expression::Multiplicative(left, _, right)
        | Expression::Additive(left, _, right)
        | Expression::Union(left, right)
        | Expression::Inequality(left, _, right)
        | Expression::Equality(left, _, right)
        | Expression::Membership(left, _, right)
        | Expression::And(left, right)
        | Expression::Or(left, _, right)
        | Expression::Implies(left, right) => contains_resolve(left) || contains_resolve(right),
        Expression::Lambda(_, body) => contains_resolve(body),
        Expression::InstanceSelector(_, fields) => {
            fields.iter().any(|(_, value)| contains_resolve(value))
        }
        _ => false,
    }
}

fn locate_from(
    resource: &Value,
    expression: &Expression,
    version: FhirVersion,
    root: Selection,
) -> Result<Selection, PatchError> {
    match expression {
        Expression::Term(Term::Invocation(Invocation::Member(name))) => {
            if resource.get("resourceType").and_then(Value::as_str) == Some(name.as_str()) {
                Ok(root)
            } else {
                select_member(resource, root, name, version)
            }
        }
        Expression::Term(Term::Parenthesized(inner)) => locate_from(resource, inner, version, root),
        Expression::Invocation(base, Invocation::Member(name)) => {
            let parents = locate_from(resource, base, version, root)?;
            select_member(resource, parents, name, version)
        }
        Expression::Invocation(base, Invocation::Function(name, args)) => {
            let mut selected = locate_from(resource, base, version, root)?;
            match (name.as_str(), args.as_slice()) {
                ("where", [predicate]) => {
                    let mut kept = Vec::new();
                    for node in selected.nodes {
                        let value = get(resource, &node.path)
                            .ok_or_else(|| failed("where target disappeared"))?;
                        if predicate_true(value, predicate, version)? {
                            kept.push(node);
                        }
                    }
                    selected.nodes = kept;
                    selected.arrays.clear();
                    Ok(selected)
                }
                ("first", []) => {
                    selected.nodes.truncate(1);
                    selected.arrays.clear();
                    Ok(selected)
                }
                ("last", []) => {
                    selected.nodes = selected.nodes.pop().into_iter().collect();
                    selected.arrays.clear();
                    Ok(selected)
                }
                ("single", []) => {
                    if selected.nodes.len() > 1 {
                        return Err(failed("single() selected multiple nodes"));
                    }
                    selected.arrays.clear();
                    Ok(selected)
                }
                ("ofType", [type_expression]) => {
                    let type_name = match type_expression {
                        Expression::Term(Term::Invocation(Invocation::Member(name))) => name,
                        _ => return Err(failed("ofType() needs a named FHIR type")),
                    };
                    selected.nodes.retain(|node| {
                        node.fhir_type
                            .as_deref()
                            .is_some_and(|ty| ty.eq_ignore_ascii_case(type_name))
                    });
                    selected.arrays.clear();
                    Ok(selected)
                }
                _ => Err(failed(format!(
                    "FHIRPath function {name} does not preserve an editable location"
                ))),
            }
        }
        Expression::Indexer(base, index) => {
            let mut selected = locate_from(resource, base, version, root)?;
            let index = match index.as_ref() {
                Expression::Term(Term::Literal(Literal::Integer(n))) if *n >= 0 => *n as usize,
                _ => {
                    return Err(failed(
                        "PATCH path index must be a nonnegative integer literal",
                    ));
                }
            };
            selected.nodes = selected.nodes.into_iter().nth(index).into_iter().collect();
            selected.arrays.clear();
            Ok(selected)
        }
        _ => Err(failed(
            "FHIRPath expression does not identify editable resource nodes",
        )),
    }
}

fn select_member(
    resource: &Value,
    parents: Selection,
    name: &str,
    version: FhirVersion,
) -> Result<Selection, PatchError> {
    let mut result = Selection::default();
    for parent in parents.nodes {
        let Some(object) = get(resource, &parent.path).and_then(Value::as_object) else {
            continue;
        };
        let keys: Vec<&str> = if object.contains_key(name) {
            vec![name]
        } else if let Some(parent_type) = parent.fhir_type.as_deref() {
            let mut matches = object
                .keys()
                .filter(|key| {
                    key.strip_prefix(name)
                        .and_then(|suffix| suffix.chars().next())
                        .is_some_and(char::is_uppercase)
                        && helios_fhir::get_field_type(version, parent_type, key).is_some()
                })
                .map(String::as_str)
                .collect::<Vec<_>>();
            matches.sort_unstable();
            matches
        } else {
            vec![]
        };
        if keys.len() > 1 {
            return Err(failed(format!(
                "Choice path {name} resolves to multiple fields"
            )));
        }
        for key in keys {
            let mut path = parent.path.clone();
            path.push(PathSegment::Key(key.to_string()));
            let child_type = parent
                .fhir_type
                .as_deref()
                .and_then(|ty| helios_fhir::get_field_type(version, ty, key))
                .map(|(ty, _)| ty.to_string());
            match object.get(key) {
                Some(Value::Array(items)) => {
                    result.arrays.push(path.clone());
                    for index in 0..items.len() {
                        let mut item_path = path.clone();
                        item_path.push(PathSegment::Index(index));
                        result.nodes.push(NodeRef {
                            path: item_path,
                            fhir_type: child_type.clone(),
                        });
                    }
                }
                Some(_) => result.nodes.push(NodeRef {
                    path,
                    fhir_type: child_type,
                }),
                None => unreachable!("key drawn from object"),
            }
        }
    }
    Ok(result)
}

fn final_member(expression: &Expression) -> Option<&str> {
    match expression {
        Expression::Term(Term::Invocation(Invocation::Member(name))) => Some(name),
        Expression::Invocation(_, Invocation::Member(name)) => Some(name),
        Expression::Term(Term::Parenthesized(inner)) => final_member(inner),
        _ => None,
    }
}

fn parent_type_for_path(
    resource: &Value,
    path: &[PathSegment],
    version: FhirVersion,
) -> Option<String> {
    let mut ty = resource.get("resourceType")?.as_str()?.to_string();
    for segment in path {
        if let PathSegment::Key(key) = segment {
            ty = helios_fhir::get_field_type(version, &ty, key)?
                .0
                .to_string();
        }
    }
    Some(ty)
}

fn field_type_for_path(
    resource: &Value,
    path: &[PathSegment],
    version: FhirVersion,
) -> Option<String> {
    let (last, parent_path) = path.split_last()?;
    let PathSegment::Key(key) = last else {
        return None;
    };
    let parent_type = parent_type_for_path(resource, parent_path, version)?;
    helios_fhir::get_field_type(version, &parent_type, key).map(|(ty, _)| ty.to_string())
}

fn get<'a>(value: &'a Value, path: &[PathSegment]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, segment| match segment {
            PathSegment::Key(key) => current.get(key),
            PathSegment::Index(index) => current.get(*index),
        })
}

fn get_mut<'a>(value: &'a mut Value, path: &[PathSegment]) -> Option<&'a mut Value> {
    path.iter()
        .try_fold(value, |current, segment| match segment {
            PathSegment::Key(key) => current.get_mut(key),
            PathSegment::Index(index) => current.get_mut(*index),
        })
}

fn remove_at(value: &mut Value, path: &[PathSegment]) -> Result<(), PatchError> {
    let (last, parent_path) = path
        .split_last()
        .ok_or_else(|| failed("Cannot delete the resource root"))?;
    let parent = get_mut(value, parent_path).ok_or_else(|| failed("delete parent disappeared"))?;
    match last {
        PathSegment::Key(key) => {
            parent
                .as_object_mut()
                .and_then(|object| object.remove(key))
                .ok_or_else(|| failed("delete target disappeared"))?;
        }
        PathSegment::Index(index) => {
            let items = parent
                .as_array_mut()
                .ok_or_else(|| failed("delete parent is not an array"))?;
            if *index >= items.len() {
                return Err(failed("delete index disappeared"));
            }
            items.remove(*index);
        }
    }
    Ok(())
}

fn predicate_true(
    value: &Value,
    predicate: &Expression,
    version: FhirVersion,
) -> Result<bool, PatchError> {
    let item = json_evaluation_result(value);
    let mut context = EvaluationContext::new_empty(version);
    context.set_this(item.clone());
    let result = evaluate(predicate, &context, Some(&item))
        .map_err(|e| failed(format!("Cannot evaluate where() predicate: {e}")))?;
    Ok(matches!(result, EvaluationResult::Boolean(true, _, _)))
}

fn json_evaluation_result(value: &Value) -> EvaluationResult {
    match value {
        Value::Null => EvaluationResult::Empty,
        Value::Bool(value) => EvaluationResult::boolean(*value),
        Value::Number(number) => number
            .as_i64()
            .map(EvaluationResult::integer)
            .or_else(|| {
                Decimal::from_str_exact(&number.to_string())
                    .ok()
                    .map(EvaluationResult::decimal)
            })
            .unwrap_or_else(|| EvaluationResult::string(number.to_string())),
        Value::String(value) => EvaluationResult::string(value.clone()),
        Value::Array(values) => {
            EvaluationResult::collection(values.iter().map(json_evaluation_result).collect())
        }
        Value::Object(values) => EvaluationResult::Object {
            map: values
                .iter()
                .map(|(key, value)| (key.clone(), json_evaluation_result(value)))
                .collect(),
            type_info: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patient() -> Value {
        json!({
            "resourceType": "Patient",
            "id": "p1",
            "active": false,
            "name": [{"family": "Before"}]
        })
    }

    #[test]
    fn json_patch_is_applied_and_the_input_left_alone() {
        let current = patient();
        let patched = apply_patch(
            &current,
            &PatchFormat::JsonPatch(json!([
                {"op": "test", "path": "/active", "value": false},
                {"op": "replace", "path": "/active", "value": true},
                {"op": "replace", "path": "/name/0/family", "value": "After"}
            ])),
        )
        .unwrap();

        assert_eq!(patched["active"], json!(true));
        assert_eq!(patched["name"][0]["family"], json!("After"));
        assert_eq!(current, patient());
    }

    #[test]
    fn merge_patch_sets_and_removes() {
        let patched = apply_patch(
            &patient(),
            &PatchFormat::MergePatch(json!({"active": true, "name": null, "gender": "other"})),
        )
        .unwrap();

        assert_eq!(
            patched,
            json!({"resourceType": "Patient", "id": "p1", "active": true, "gender": "other"})
        );
    }

    #[test]
    fn a_document_that_is_not_a_json_patch_is_malformed() {
        for document in [
            json!({"not": "a patch"}),
            json!([{"op": "frobnicate", "path": "/active"}]),
            json!([{"op": "replace", "path": "no-leading-slash", "value": 1}]),
        ] {
            let err = apply_patch(&patient(), &PatchFormat::JsonPatch(document.clone()));
            assert!(
                matches!(
                    err,
                    Err(PatchError::MalformedDocument {
                        format: "JSON Patch",
                        ..
                    })
                ),
                "{document}: {err:?}"
            );
        }
    }

    #[test]
    fn a_failed_test_is_told_apart_from_an_operation_that_cannot_apply() {
        let failed_test = apply_patch(
            &patient(),
            &PatchFormat::JsonPatch(json!([{"op": "test", "path": "/active", "value": true}])),
        );
        assert!(
            matches!(failed_test, Err(PatchError::TestFailed { .. })),
            "{failed_test:?}"
        );

        let bad_path = apply_patch(
            &patient(),
            &PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/nope/deeper", "value": 1}])),
        );
        assert!(
            matches!(bad_path, Err(PatchError::OperationFailed { .. })),
            "{bad_path:?}"
        );
    }

    #[test]
    fn resource_type_and_id_can_be_neither_changed_nor_removed() {
        let refused = [
            (
                "resourceType",
                PatchFormat::JsonPatch(
                    json!([{"op": "replace", "path": "/resourceType", "value": "Person"}]),
                ),
            ),
            (
                "resourceType",
                PatchFormat::JsonPatch(json!([{"op": "remove", "path": "/resourceType"}])),
            ),
            (
                "resourceType",
                PatchFormat::MergePatch(json!({"resourceType": "Person"})),
            ),
            (
                "id",
                PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/id", "value": "p2"}])),
            ),
            (
                "id",
                PatchFormat::JsonPatch(json!([{"op": "move", "from": "/id", "path": "/was"}])),
            ),
            ("id", PatchFormat::MergePatch(json!({"id": null}))),
            ("id", PatchFormat::MergePatch(json!({"id": "p2"}))),
        ];
        for (element, patch) in refused {
            assert_eq!(
                apply_patch(&patient(), &patch),
                Err(PatchError::ImmutableElement { element }),
                "{patch:?}"
            );
        }

        // Naming them without changing them is not a change.
        for patch in [
            PatchFormat::MergePatch(json!({"resourceType": "Patient", "id": "p1", "active": true})),
            PatchFormat::JsonPatch(json!([
                {"op": "test", "path": "/id", "value": "p1"},
                {"op": "replace", "path": "/resourceType", "value": "Patient"},
                {"op": "replace", "path": "/active", "value": true}
            ])),
        ] {
            let patched = apply_patch(&patient(), &patch).unwrap();
            assert_eq!(patched["active"], json!(true), "{patch:?}");
        }
    }

    #[test]
    fn fhirpath_patch_needs_the_stored_resource_version() {
        let parameters = operations(vec![operation(
            "replace",
            "Patient.active",
            vec![json!({"name":"value","valueBoolean":true})],
        )]);
        assert!(matches!(
            apply_patch(&patient(), &PatchFormat::FhirPathPatch(parameters)),
            Err(PatchError::UnsupportedFormat { .. })
        ));
    }

    fn operation(kind: &str, path: &str, extra: Vec<Value>) -> Value {
        let mut parts = vec![
            json!({"name":"type","valueCode":kind}),
            json!({"name":"path","valueString":path}),
        ];
        parts.extend(extra);
        json!({"name":"operation","part":parts})
    }

    fn operations(operations: Vec<Value>) -> Value {
        json!({"resourceType":"Parameters","parameter":operations})
    }

    #[cfg(feature = "R4")]
    #[test]
    fn fhirpath_operations_apply_in_order_to_real_nodes() {
        let patch = operations(vec![
            operation(
                "add",
                "Patient",
                vec![
                    json!({"name":"name","valueString":"name"}),
                    json!({"name":"value","valueHumanName":{"family":"Second"}}),
                ],
            ),
            operation(
                "insert",
                "Patient.name",
                vec![
                    json!({"name":"index","valueInteger":1}),
                    json!({"name":"value","valueHumanName":{"family":"Middle"}}),
                ],
            ),
            operation(
                "move",
                "Patient.name",
                vec![
                    json!({"name":"source","valueInteger":2}),
                    json!({"name":"destination","valueInteger":0}),
                ],
            ),
            operation(
                "replace",
                "Patient.name[1].family",
                vec![json!({"name":"value","valueString":"Updated"})],
            ),
            operation("delete", "Patient.name[2]", vec![]),
            operation(
                "add",
                "Patient",
                vec![
                    json!({"name":"name","valueString":"birthDate"}),
                    json!({"name":"value","valueDate":"2000-01-01"}),
                ],
            ),
        ]);
        let result = apply_patch_for_version(
            &patient(),
            &PatchFormat::FhirPathPatch(patch),
            FhirVersion::R4,
        )
        .unwrap();
        assert_eq!(
            result["name"],
            json!([
                {"family":"Second"}, {"family":"Updated"}
            ])
        );
        assert_eq!(result["birthDate"], "2000-01-01");
    }

    #[cfg(feature = "R4")]
    #[test]
    fn where_selects_a_source_node_and_computed_or_ambiguous_paths_fail() {
        let input = json!({
            "resourceType":"Patient", "id":"p1",
            "name":[
                {"use":"usual","family":"Same"},
                {"use":"official","family":"Same"}
            ]
        });
        let replace = operations(vec![operation(
            "replace",
            "Patient.name.where(use = 'official').family",
            vec![json!({"name":"value","valueString":"Changed"})],
        )]);
        let result = apply_patch_for_version(
            &input,
            &PatchFormat::FhirPathPatch(replace),
            FhirVersion::R4,
        )
        .unwrap();
        assert_eq!(result["name"][0]["family"], "Same");
        assert_eq!(result["name"][1]["family"], "Changed");

        for path in [
            "Patient.name.family",
            "Patient.name.family.length()",
            "Patient.name.resolve()",
        ] {
            let patch = operations(vec![operation(
                "replace",
                path,
                vec![json!({"name":"value","valueString":"Bad"})],
            )]);
            assert!(matches!(
                apply_patch_for_version(
                    &input,
                    &PatchFormat::FhirPathPatch(patch),
                    FhirVersion::R4
                ),
                Err(PatchError::OperationFailed { .. })
            ));
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn nested_parts_choice_elements_and_zero_match_delete() {
        let input = json!({
            "resourceType":"Observation", "id":"o1",
            "status":"final", "code":{"text":"test"},
            "valueQuantity":{"value":1}
        });
        let patch = operations(vec![
            operation(
                "replace",
                "Observation.value",
                vec![json!({"name":"value","valueString":"new value"})],
            ),
            operation(
                "add",
                "Observation",
                vec![
                    json!({"name":"name","valueString":"component"}),
                    json!({"name":"value","part":[
                        {"name":"code","valueCodeableConcept":{"text":"component"}},
                        {"name":"value","valueString":"nested choice"}
                    ]}),
                ],
            ),
            operation("delete", "Observation.note", vec![]),
        ]);
        let result =
            apply_patch_for_version(&input, &PatchFormat::FhirPathPatch(patch), FhirVersion::R4)
                .unwrap();
        assert_eq!(result["valueString"], "new value");
        assert!(result.get("valueQuantity").is_none());
        assert_eq!(result["component"][0]["valueString"], "nested choice");
    }

    #[cfg(feature = "R4")]
    #[test]
    fn of_type_keeps_the_choice_elements_source_location() {
        let input = json!({
            "resourceType":"Observation", "id":"o1",
            "valueQuantity":{"value":1,"unit":"mg"}
        });
        let patch = operations(vec![operation(
            "replace",
            "Observation.value.ofType(Quantity).value",
            vec![json!({"name":"value","valueDecimal":2.5})],
        )]);
        let result =
            apply_patch_for_version(&input, &PatchFormat::FhirPathPatch(patch), FhirVersion::R4)
                .unwrap();
        assert_eq!(result["valueQuantity"]["value"], 2.5);
    }

    #[cfg(feature = "R4")]
    #[test]
    fn fhirpath_patch_rejects_identity_changes_and_malformed_operations() {
        let identity = operations(vec![operation(
            "replace",
            "Patient.id",
            vec![json!({"name":"value","valueId":"other"})],
        )]);
        assert_eq!(
            apply_patch_for_version(
                &patient(),
                &PatchFormat::FhirPathPatch(identity),
                FhirVersion::R4
            ),
            Err(PatchError::ImmutableElement { element: "id" })
        );
        let bad = operations(vec![operation("frobnicate", "Patient.active", vec![])]);
        assert!(matches!(
            decode_bundle_patch_resource(&bad, FhirVersion::R4),
            Err(PatchError::MalformedDocument { .. })
        ));
    }

    #[cfg(feature = "R4")]
    #[test]
    fn fhirpath_patch_rejects_malformed_operation_parts() {
        let malformed_documents = [
            json!({"resourceType":"Patient","parameter":[]}),
            operations(vec![]),
            operations(vec![json!({"name":"wrong","part":[]})]),
            operations(vec![json!({"name":"operation","part":[
                {"name":"type","valueCode":"delete"},
                {"name":"type","valueCode":"delete"},
                {"name":"path","valueString":"Patient.active"}
            ]})]),
            operations(vec![operation(
                "insert",
                "Patient.name",
                vec![
                    json!({"name":"index","valueInteger":-1}),
                    json!({"name":"value","valueHumanName":{"family":"New"}}),
                ],
            )]),
            operations(vec![operation("replace", "Patient.active", vec![])]),
            operations(vec![operation(
                "delete",
                "Patient.active",
                vec![json!({"name":"value","valueBoolean":true})],
            )]),
            operations(vec![operation(
                "replace",
                "Patient.active",
                vec![json!({"name":"value"})],
            )]),
        ];
        for document in malformed_documents {
            let result = decode_bundle_patch_resource(&document, FhirVersion::R4);
            assert!(
                matches!(result, Err(PatchError::MalformedDocument { .. })),
                "{document}: {result:?}"
            );
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn nested_fhirpath_values_keep_repeating_children_and_refuse_duplicate_scalar_children() {
        let value = json!({"name":"value","part":[
            {"name":"given","valueString":"Ada"},
            {"name":"given","valueString":"Grace"},
            {"name":"family","valueString":"Lovelace"}
        ]});
        let add_name = |value| {
            operations(vec![operation(
                "add",
                "Patient",
                vec![json!({"name":"name","valueString":"name"}), value],
            )])
        };
        let result = apply_patch_for_version(
            &patient(),
            &PatchFormat::FhirPathPatch(add_name(value)),
            FhirVersion::R4,
        )
        .unwrap();
        assert_eq!(result["name"][1]["given"], json!(["Ada", "Grace"]));
        assert_eq!(result["name"][1]["family"], "Lovelace");

        let duplicate = json!({"name":"value","part":[
            {"name":"family","valueString":"One"},
            {"name":"family","valueString":"Two"}
        ]});
        assert!(matches!(
            apply_patch_for_version(
                &patient(),
                &PatchFormat::FhirPathPatch(add_name(duplicate)),
                FhirVersion::R4
            ),
            Err(PatchError::MalformedDocument { .. })
        ));
    }

    #[cfg(feature = "R4")]
    #[test]
    fn fhirpath_patch_checks_collection_bounds_and_scalar_additions() {
        for patch in [
            operation(
                "add",
                "Patient",
                vec![
                    json!({"name":"name","valueString":"active"}),
                    json!({"name":"value","valueBoolean":true}),
                ],
            ),
            operation(
                "insert",
                "Patient.name",
                vec![
                    json!({"name":"index","valueInteger":2}),
                    json!({"name":"value","valueHumanName":{"family":"New"}}),
                ],
            ),
            operation(
                "move",
                "Patient.name",
                vec![
                    json!({"name":"source","valueInteger":1}),
                    json!({"name":"destination","valueInteger":0}),
                ],
            ),
        ] {
            let result = apply_patch_for_version(
                &patient(),
                &PatchFormat::FhirPathPatch(operations(vec![patch])),
                FhirVersion::R4,
            );
            assert!(
                matches!(result, Err(PatchError::OperationFailed { .. })),
                "{result:?}"
            );
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn fhirpath_selectors_address_one_editable_node() {
        let mut input = patient();
        input["name"] = json!([{"family":"First"},{"family":"Last"}]);
        for (path, expected) in [
            ("(Patient.name).first().family", json!(["Changed", "Last"])),
            ("Patient.name.last().family", json!(["First", "Changed"])),
        ] {
            let patch = operations(vec![operation(
                "replace",
                path,
                vec![json!({"name":"value","valueString":"Changed"})],
            )]);
            let result = apply_patch_for_version(
                &input,
                &PatchFormat::FhirPathPatch(patch),
                FhirVersion::R4,
            )
            .unwrap();
            assert_eq!(
                json!([result["name"][0]["family"], result["name"][1]["family"]]),
                expected
            );
        }

        for path in [
            "Patient.name.single().family",
            "Patient.name[-1].family",
            "Patient.name.where(given.resolve().exists()).family",
        ] {
            let patch = operations(vec![operation(
                "replace",
                path,
                vec![json!({"name":"value","valueString":"Changed"})],
            )]);
            let result = apply_patch_for_version(
                &input,
                &PatchFormat::FhirPathPatch(patch),
                FhirVersion::R4,
            );
            assert!(
                matches!(result, Err(PatchError::OperationFailed { .. })),
                "{path}: {result:?}"
            );
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn choice_addition_needs_a_supported_typed_value() {
        let observation =
            json!({"resourceType":"Observation","id":"o1","status":"final","code":{"text":"test"}});
        let add_choice = |value| {
            let patch = operations(vec![operation(
                "add",
                "Observation",
                vec![json!({"name":"name","valueString":"value[x]"}), value],
            )]);
            apply_patch_for_version(
                &observation,
                &PatchFormat::FhirPathPatch(patch),
                FhirVersion::R4,
            )
        };
        assert!(matches!(
            add_choice(json!({"name":"value","part":[{"name":"text","valueString":"nested"}]})),
            Err(PatchError::MalformedDocument { .. })
        ));
        assert!(matches!(
            add_choice(json!({"name":"value","valueHumanName":{"family":"Wrong"}})),
            Err(PatchError::OperationFailed { .. })
        ));
    }

    #[cfg(feature = "R4")]
    #[test]
    fn bundle_decoder_accepts_parameters_and_refuses_r4_binary() {
        let parameters = operations(vec![operation("delete", "Patient.active", vec![])]);
        assert!(matches!(
            decode_bundle_patch_resource(&parameters, FhirVersion::R4),
            Ok(PatchFormat::FhirPathPatch(_))
        ));
        let binary = json!({
            "resourceType":"Binary", "contentType":"application/json-patch+json",
            "data":"W3sib3AiOiJyZXBsYWNlIiwicGF0aCI6Ii9hY3RpdmUiLCJ2YWx1ZSI6dHJ1ZX1d"
        });
        assert!(matches!(
            decode_bundle_patch_resource(&binary, FhirVersion::R4),
            Err(PatchError::UnsupportedFormat { .. })
        ));
    }

    #[cfg(feature = "R5")]
    #[test]
    fn bundle_decoder_accepts_r5_binary_json_patch() {
        let binary = json!({
            "resourceType":"Binary", "contentType":"application/json-patch+json",
            "data":"W3sib3AiOiJyZXBsYWNlIiwicGF0aCI6Ii9hY3RpdmUiLCJ2YWx1ZSI6dHJ1ZX1d"
        });
        let format = decode_bundle_patch_resource(&binary, FhirVersion::R5).unwrap();
        let result = apply_patch_for_version(&patient(), &format, FhirVersion::R5).unwrap();
        assert_eq!(result["active"], true);
    }

    #[cfg(feature = "R4B")]
    #[test]
    fn r4b_bundle_decoder_accepts_parameters_but_not_binary() {
        let parameters = operations(vec![operation("delete", "Patient.active", vec![])]);
        assert!(matches!(
            decode_bundle_patch_resource(&parameters, FhirVersion::R4B),
            Ok(PatchFormat::FhirPathPatch(_))
        ));
        let binary = json!({
            "resourceType":"Binary", "contentType":"application/json-patch+json",
            "data":"W3sib3AiOiJyZXBsYWNlIiwicGF0aCI6Ii9hY3RpdmUiLCJ2YWx1ZSI6dHJ1ZX1d"
        });
        assert!(matches!(
            decode_bundle_patch_resource(&binary, FhirVersion::R4B),
            Err(PatchError::UnsupportedFormat { .. })
        ));
    }

    #[cfg(feature = "R6")]
    #[test]
    fn r6_bundle_decoder_accepts_parameters_and_binary() {
        let parameters = operations(vec![operation(
            "replace",
            "Patient.active",
            vec![json!({"name":"value","valueBoolean":true})],
        )]);
        let fhirpath = decode_bundle_patch_resource(&parameters, FhirVersion::R6).unwrap();
        let result = apply_patch_for_version(&patient(), &fhirpath, FhirVersion::R6).unwrap();
        assert_eq!(result["active"], true);

        let binary = json!({
            "resourceType":"Binary", "contentType":"application/json-patch+json",
            "data":"W3sib3AiOiJyZXBsYWNlIiwicGF0aCI6Ii9hY3RpdmUiLCJ2YWx1ZSI6dHJ1ZX1d"
        });
        let format = decode_bundle_patch_resource(&binary, FhirVersion::R6).unwrap();
        let result = apply_patch_for_version(&patient(), &format, FhirVersion::R6).unwrap();
        assert_eq!(result["active"], true);
    }
}
