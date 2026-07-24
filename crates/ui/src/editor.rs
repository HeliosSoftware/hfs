//! Schema-driven resource editor (#264).
//!
//! The design in `docs/resource-editor-design.md`, made real. Two things about
//! it are unusual and both are deliberate.
//!
//! **The document is the state.** There is no client-side model and no session
//! on the server. Every structural mutation — add a node, remove one, pick a
//! `value[x]` type, attach an extension — posts the whole in-flight resource
//! back, the server applies the mutation, re-validates, re-renders, and htmx
//! swaps the result in. Resources are small and `validate_sync` is a pure
//! ~300 µs call, so this is affordable, and it buys three things a form
//! projection cannot: the editor's hard rules (cardinality, choice groups,
//! slicing) exist once, in Rust, where they are tested; validation errors
//! arrive already anchored to their node; and **whatever we do not render, we
//! also do not destroy** — the `_birthDate` sibling of an extended primitive
//! survives a round-trip here, and does not survive one in any editor surveyed
//! for #264.
//!
//! **Extensions are ordinary elements.** They are not a panel and not an escape
//! hatch. `extension` is an element of `HumanName` exactly as `family` is, so it
//! is offered wherever the schema allows it, at any depth, nested, with the
//! full `value[x]` type pick — which is the thing none of the surveyed editors
//! can do on a resource that carries no profile.

use askama::Template;
use axum::{
    Form,
    extract::{Query, State},
    response::Response,
};
use helios_fhir_validator::{
    Addable, AddableKind, Step, SyncOutcome, ValidationOptions, Validator, editor, packs,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, render};

/// One line of the editor tree. The tree is flattened here rather than rendered
/// recursively: Askama has no recursive includes, and a flat list with an
/// indent is what the markup wants anyway.
pub struct Row {
    /// Dotted path — the same form the validator reports errors on, so an issue
    /// anchors onto a row by string equality.
    pub path: String,
    pub indent: usize,
    /// Element name, or the index of a repeating item.
    pub label: String,
    /// `HumanName`, `string`, … — shown as a hint.
    pub type_label: String,
    /// Primitive value, for the input.
    pub value: String,
    pub is_primitive: bool,
    /// Rendered with the extension tint. Extensions are normal elements, just
    /// visibly different — the stance clinFHIR got right in 2016.
    pub is_extension: bool,
    /// An unrecognised modifier extension means *do not process this resource*.
    /// No editor surveyed says so. This one does.
    pub is_modifier: bool,
    /// The schema does not know this key. We still render it — never silently
    /// drop a user's data.
    pub is_unknown: bool,
    /// The primitive carries an `_name` sibling (extensions on a primitive).
    pub has_primitive_extension: bool,
    /// A required binding: the value must come from a value set.
    pub binding: Option<String>,
    /// Validation issues anchored exactly here.
    pub errors: Vec<String>,
    /// What may be added under this node.
    pub addable: Vec<AddOption>,
    /// The node accepts an extension, so the ad-hoc URL control is offered.
    /// Precomputed: Askama has no closures.
    pub accepts_extension: bool,
    pub can_remove: bool,
}

/// An element offered under a node.
pub struct AddOption {
    pub name: String,
    /// `add` | `another` | `choice`
    pub kind: &'static str,
    pub type_label: String,
    pub required: bool,
    /// Concrete arms, when this is a `value[x]`.
    pub arms: Vec<String>,
}

#[derive(Template)]
#[template(path = "pages/editor.html")]
pub struct EditorPage {
    pub status: crate::Status,
    pub i18n: I18n,
    pub active_page: &'static str,
    pub nl_enabled: bool,
    pub resource_type: String,
    pub resource_id: String,
}

#[derive(Template)]
#[template(path = "partials/editor-body.html")]
pub struct EditorBody {
    pub i18n: I18n,
    pub rows: Vec<Row>,
    /// The in-flight document. This *is* the editor's state: it rides in a
    /// hidden field and comes back with every mutation.
    pub document: String,
    /// Pretty JSON, for the raw-edit textarea.
    pub pretty: String,
    /// The foldable, line-numbered JSON view shown beside the guided form.
    pub json_lines: Vec<crate::json_view::JsonLine>,
    pub error_count: usize,
    /// Issues the validator reported against a path no row owns (an invariant
    /// on a backbone element, say). Surfaced rather than swallowed.
    pub orphan_errors: Vec<String>,
    pub parse_error: Option<String>,
}

#[derive(Deserialize)]
pub struct EditorQuery {
    #[serde(rename = "type")]
    pub resource_type: Option<String>,
    pub id: Option<String>,
}

/// A mutation, plus the document it applies to.
#[derive(Deserialize)]
pub struct EditorForm {
    /// The whole in-flight resource.
    pub doc: String,
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arm: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub modifier: String,
}

/// The editor shell. The resource itself is fetched by the browser from the
/// ordinary FHIR API and posted straight back to [`render_body`] — the UI crate
/// never touches storage, and the read path stays the one we already trust.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<EditorQuery>,
) -> Response {
    render(EditorPage {
        status: crate::current_status(state.version, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "editor",
        nl_enabled: state.nl.enabled,
        resource_type: query.resource_type.unwrap_or_else(|| "Patient".to_string()),
        resource_id: query.id.unwrap_or_default(),
    })
}

/// Applies a mutation and re-renders. One endpoint for every structural edit:
/// the operation and the whole document arrive together, and the whole editor
/// body goes back.
pub async fn render_body(
    State(_state): State<WebState>,
    locale: RequestLocale,
    Form(form): Form<EditorForm>,
) -> Response {
    let i18n = I18n::new(locale);
    let registry = packs::core_registry(helios_fhir::FhirVersion::default());

    let mut document: Value = match serde_json::from_str(&form.doc) {
        Ok(value) => value,
        Err(error) => {
            // A malformed document is the source view's fault, and the user is
            // mid-keystroke. Say what is wrong and keep their text.
            return render(EditorBody {
                i18n,
                rows: Vec::new(),
                document: form.doc.clone(),
                pretty: form.doc,
                json_lines: Vec::new(),
                error_count: 0,
                orphan_errors: Vec::new(),
                parse_error: Some(error.to_string()),
            });
        }
    };

    let resource_type = document
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or("Patient")
        .to_string();

    apply(&*registry, &resource_type, &mut document, &form);

    render(build_body(i18n, registry, resource_type, document, None))
}

/// Applies one mutation to the document.
fn apply(
    resolver: &dyn helios_fhir_validator::SchemaResolver,
    resource_type: &str,
    document: &mut Value,
    form: &EditorForm,
) {
    let path = editor::path_from_string(&form.path);

    match form.op.as_str() {
        "add" => {
            editor::add_element(resolver, resource_type, document, &path, &form.name);
        }
        "choose" => {
            editor::choose_type(
                resolver,
                resource_type,
                document,
                &path,
                &form.name,
                &form.arm,
            );
        }
        "extension" => {
            let url = if form.url.trim().is_empty() {
                "http://example.org/fhir/StructureDefinition/my-extension"
            } else {
                form.url.trim()
            };
            editor::add_extension(
                resolver,
                resource_type,
                document,
                &path,
                url,
                form.modifier == "true",
            );
        }
        "remove" => {
            editor::remove_at(document, &path);
        }
        "set" => {
            editor::set_value(document, &path, &form.value);
        }
        // No op: a plain re-render (the first load, or a mode switch).
        _ => {}
    }
}

/// Validates, flattens, and packages the editor body.
fn build_body(
    i18n: I18n,
    registry: Arc<helios_fhir_validator::SchemaRegistry>,
    resource_type: String,
    document: Value,
    parse_error: Option<String>,
) -> EditorBody {
    // The cheap pass, on every mutation. Pure, no I/O — this is what makes
    // continuous validation affordable at all.
    let resolver: Arc<dyn helios_fhir_validator::SchemaResolver> = Arc::clone(&registry) as _;
    let validator = Validator::new(resolver);
    let SyncOutcome { errors, .. } =
        validator.validate_sync(&document, &ValidationOptions::default());

    // Anchor each issue to its node. The validator reports `Patient.name.0.given`
    // and our rows are keyed on `name.0.given`, so this is string equality —
    // none of the fuzzy FHIRPath matching other editors resort to.
    let mut by_path: HashMap<String, Vec<String>> = HashMap::new();
    for error in &errors {
        let path = error
            .path
            .strip_prefix(&format!("{resource_type}."))
            .unwrap_or(&error.path)
            .to_string();
        by_path.entry(path).or_default().push(error.message.clone());
    }

    let mut rows = Vec::new();
    build_rows(
        registry.as_ref(),
        &resource_type,
        &document,
        &[],
        0,
        &by_path,
        &mut rows,
    );

    // Anything that did not land on a row still has to be seen.
    let claimed: std::collections::HashSet<&str> =
        rows.iter().map(|row| row.path.as_str()).collect();
    let orphan_errors = by_path
        .iter()
        .filter(|(path, _)| !claimed.contains(path.as_str()) && !path.is_empty())
        .flat_map(|(path, messages)| {
            messages
                .iter()
                .map(move |message| format!("{path}: {message}"))
        })
        .collect();

    EditorBody {
        i18n,
        document: serde_json::to_string(&document).unwrap_or_default(),
        pretty: serde_json::to_string_pretty(&document).unwrap_or_default(),
        json_lines: crate::json_view::lines(&document),
        error_count: errors.len(),
        orphan_errors,
        rows,
        parse_error,
    }
}

/// Walks the document, emitting one row per node, depth-first, in spec order.
fn build_rows(
    resolver: &dyn helios_fhir_validator::SchemaResolver,
    resource_type: &str,
    document: &Value,
    path: &[Step],
    depth: usize,
    errors: &HashMap<String, Vec<String>>,
    out: &mut Vec<Row>,
) {
    let key = editor::path_to_string(path);
    let node = match editor::node_at(document, path) {
        Some(node) => node,
        None => return,
    };

    // A repeating element: one row per item, so each can be removed and edited
    // independently.
    if let Some(items) = node.as_array() {
        for index in 0..items.len() {
            let mut item_path = path.to_vec();
            item_path.push(Step::Index(index));
            build_rows(
                resolver,
                resource_type,
                document,
                &item_path,
                depth,
                errors,
                out,
            );
        }
        return;
    }

    let children = editor::present_children(resolver, resource_type, document, path);
    let offered = editor::addable(resolver, resource_type, document, path);

    let last = path.last();
    let label = match last {
        Some(Step::Field(name)) => name.clone(),
        Some(Step::Index(index)) => format!("[{index}]"),
        None => resource_type.to_string(),
    };
    let name_of_node = path.iter().rev().find_map(|step| match step {
        Step::Field(name) => Some(name.as_str()),
        Step::Index(_) => None,
    });

    let schema = editor::schema_at(resolver, resource_type, path);
    let is_primitive = node.is_string() || node.is_boolean() || node.is_number();

    // An extended primitive lives in two JSON keys — `birthDate` and
    // `_birthDate`. The sibling is not a field of its own, so it gets a marker
    // on the primitive that owns it, and it rides along untouched in the
    // document. Every editor surveyed for #264 either drops it or refuses to
    // show it.
    let has_primitive_extension = match (path.split_last(), &node) {
        (Some((Step::Field(name), parent_path)), _) if is_primitive => {
            editor::node_at(document, parent_path)
                .and_then(Value::as_object)
                .map(|object| object.contains_key(&format!("_{name}")))
                .unwrap_or(false)
        }
        _ => false,
    };

    out.push(Row {
        path: key.clone(),
        indent: depth * 18,
        label,
        type_label: schema
            .as_ref()
            .and_then(|schema| schema.type_.clone())
            .unwrap_or_default(),
        value: match node {
            Value::String(text) => text.clone(),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        },
        is_primitive,
        is_extension: matches!(name_of_node, Some("extension") | Some("modifierExtension")),
        is_modifier: matches!(name_of_node, Some("modifierExtension")),
        is_unknown: !path.is_empty() && schema.is_none(),
        has_primitive_extension,
        binding: schema.as_ref().and_then(|schema| {
            schema.binding.as_ref().and_then(|binding| {
                (binding.strength.as_deref() == Some("required")).then(|| {
                    binding
                        .value_set
                        .split('/')
                        .next_back()
                        .unwrap_or_default()
                        // The value set carries a `|4.0.1` version suffix.
                        .split('|')
                        .next()
                        .unwrap_or_default()
                        .to_string()
                })
            })
        }),
        errors: errors.get(&key).cloned().unwrap_or_default(),
        accepts_extension: !is_primitive && offered.iter().any(|option| option.name == "extension"),
        addable: if is_primitive {
            Vec::new()
        } else {
            offered.into_iter().map(to_option).collect()
        },
        can_remove: !path.is_empty(),
    });

    if is_primitive {
        return;
    }

    for child in children {
        let mut child_path = path.to_vec();
        child_path.push(Step::Field(child.name.clone()));
        build_rows(
            resolver,
            resource_type,
            document,
            &child_path,
            depth + 1,
            errors,
            out,
        );
    }
}

fn to_option(addable: Addable) -> AddOption {
    let (kind, arms) = match addable.kind {
        AddableKind::Add => ("add", Vec::new()),
        AddableKind::AddAnother => ("another", Vec::new()),
        AddableKind::Choice(arms) => ("choice", arms),
    };
    AddOption {
        name: addable.name,
        kind,
        type_label: addable.type_.unwrap_or_default(),
        required: addable.required,
        arms,
    }
}
