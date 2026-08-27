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
    /// Profiled extensions applicable here, offered by name above the
    /// ad-hoc URL entry.
    pub ext_options: Vec<ExtOption>,
    /// `mustSupport` in the governing schema — emphasised in the form.
    pub must_support: bool,
    /// The slice this array item matches, when the element is sliced.
    pub slice: String,
    /// The binding strength (`required`, `extensible`, …), for the chip.
    pub binding_strength: String,
    /// The bound value set's canonical URL, for the live `$expand` picker.
    pub binding_url: String,
    /// The `short` human label; the raw element name stays as the technical
    /// hint next to it.
    pub short: String,
}

/// A profiled extension offered at a node (#363).
pub struct ExtOption {
    pub url: String,
    pub name: String,
    pub short: String,
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
    pub must_support: bool,
    /// The `short` label, shown as the option's description.
    pub short: String,
    /// Adds into this named slice (seeded so the item matches).
    pub slice: String,
}

#[derive(Template)]
#[template(path = "pages/editor.html")]
pub struct EditorPage {
    pub status: crate::Status,
    pub i18n: I18n,
    pub active_page: &'static str,
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
    /// Dotted path of the node the last mutation created, so the client can
    /// put the caret straight into it after the swap (#547). Empty when the
    /// mutation created nothing.
    pub focus_path: String,
    /// Whether the root add-picker opens by itself — a document with no
    /// elements gives the user nothing else to act on (#547).
    pub auto_open_add: bool,
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
    #[serde(default)]
    pub slice: String,
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
        status: crate::current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "editor",
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
    rv: RequestVersion,
    Form(form): Form<EditorForm>,
) -> Response {
    let i18n = I18n::new(locale);
    // The sidebar's FHIR version picks the schema pack (#488): an R4B build
    // edits against R4B schemas — and offers R4B's extension catalogue.
    let registry = packs::core_registry(rv.0);

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
                focus_path: String::new(),
                auto_open_add: false,
            });
        }
    };

    let resource_type = document
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or("Patient")
        .to_string();

    let created = apply(&*registry, &resource_type, &mut document, &form);

    render(build_body(
        i18n,
        registry,
        rv.0,
        resource_type,
        document,
        None,
        created,
    ))
}

/// Applies one mutation to the document.
fn apply(
    resolver: &dyn helios_fhir_validator::SchemaResolver,
    resource_type: &str,
    document: &mut Value,
    form: &EditorForm,
) -> Option<editor::Path> {
    let path = editor::path_from_string(&form.path);

    match form.op.as_str() {
        "add" if !form.slice.is_empty() => {
            return editor::add_slice_element(
                resolver,
                resource_type,
                document,
                &path,
                &form.name,
                &form.slice,
            );
        }
        "add" => {
            return editor::add_element(resolver, resource_type, document, &path, &form.name);
        }
        "choose" => {
            return editor::choose_type(
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
            return editor::add_extension(
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
            editor::set_value(resolver, resource_type, document, &path, &form.value);
        }
        // No op: a plain re-render (the first load, or a mode switch).
        _ => {}
    }
    None
}

/// Validates, flattens, and packages the editor body.
fn build_body(
    i18n: I18n,
    registry: Arc<helios_fhir_validator::SchemaRegistry>,
    version: helios_fhir::FhirVersion,
    resource_type: String,
    document: Value,
    parse_error: Option<String>,
    created: Option<editor::Path>,
) -> EditorBody {
    // The cheap pass, on every mutation. Pure, no I/O — this is what makes
    // continuous validation affordable at all.
    let resolver: Arc<dyn helios_fhir_validator::SchemaResolver> = Arc::clone(&registry) as _;
    let validator = Validator::new(resolver);
    let SyncOutcome {
        mut errors,
        deferred,
    } = validator.validate_sync(&document, &ValidationOptions::default());
    // The editor's issue count blocks saving, so only error-severity issues
    // belong in it — warnings (e.g. extension context, #615) are $validate
    // guidance, not save blockers.
    errors.retain(|e| e.severity == helios_fhir_validator::Severity::Error);

    // Required-binding checks against the embedded core value sets (offline, no
    // terminology server), so an out-of-value-set code — e.g. gender
    // "masculino" — surfaces in the editor exactly as it does at `$validate`.
    errors.extend(
        helios_fhir_validator::core_terminology(version).required_binding_errors(&deferred),
    );

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
        &RowCtx {
            resolver: registry.as_ref(),
            registry: registry.as_ref(),
            resource_type: &resource_type,
            document: &document,
            errors: &by_path,
        },
        &[],
        0,
        &mut rows,
    );

    // Anything that did not land on a row still has to be seen.
    let claimed: std::collections::HashSet<&str> =
        rows.iter().map(|row| row.path.as_str()).collect();
    let mut orphan_errors: Vec<String> = by_path
        .iter()
        .filter(|(path, _)| !claimed.contains(path.as_str()) && !path.is_empty())
        .flat_map(|(path, messages)| {
            messages.iter().map(move |message| {
                // The validator's message usually opens with the element name
                // ("priority is required"); prefixing the path then reads
                // "priority: priority is required". Only prefix when the
                // message doesn't already carry the leaf name.
                let leaf = path.rsplit('.').next().unwrap_or(path);
                if message.starts_with(leaf) {
                    message.clone()
                } else {
                    format!("{path}: {message}")
                }
            })
        })
        .collect();
    orphan_errors.sort();
    orphan_errors.dedup();

    // A document with nothing beyond resourceType leaves the user nothing to
    // act on except adding elements — open the root picker for them (#547).
    let auto_open_add = document
        .as_object()
        .map(|o| o.keys().all(|k| k == "resourceType"))
        .unwrap_or(false);

    EditorBody {
        i18n,
        document: serde_json::to_string(&document).unwrap_or_default(),
        pretty: serde_json::to_string_pretty(&document).unwrap_or_default(),
        json_lines: crate::json_view::lines(&document),
        error_count: errors.len(),
        orphan_errors,
        rows,
        parse_error,
        focus_path: created
            .map(|path| editor::path_to_string(&path))
            .unwrap_or_default(),
        auto_open_add,
    }
}

/// What a row walk carries unchanged all the way down.
///
/// Bundled so [`build_rows`] takes the walk position (`path`, `depth`) and its
/// sink separately from its fixed inputs — the recursion threads five constant
/// arguments through every level otherwise.
struct RowCtx<'a> {
    resolver: &'a dyn helios_fhir_validator::SchemaResolver,
    registry: &'a helios_fhir_validator::SchemaRegistry,
    resource_type: &'a str,
    document: &'a Value,
    errors: &'a HashMap<String, Vec<String>>,
}

/// Walks the document, emitting one row per node, depth-first, in spec order.
fn build_rows(ctx: &RowCtx<'_>, path: &[Step], depth: usize, out: &mut Vec<Row>) {
    let RowCtx {
        resolver,
        registry,
        resource_type,
        document,
        errors,
    } = *ctx;
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
            build_rows(ctx, &item_path, depth, out);
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

    let schema = editor::schema_at_in(resolver, resource_type, Some(document), path);
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
        must_support: schema
            .as_ref()
            .and_then(|schema| schema.must_support)
            .unwrap_or(false),
        slice: match path.split_last() {
            Some((Step::Index(_), parent_path)) => {
                editor::slice_label(resolver, resource_type, document, parent_path, node)
                    .unwrap_or_default()
            }
            _ => String::new(),
        },
        binding_strength: schema
            .as_ref()
            .and_then(|schema| schema.binding.as_ref())
            .and_then(|binding| binding.strength.clone())
            .unwrap_or_default(),
        binding_url: schema
            .as_ref()
            .and_then(|schema| schema.binding.as_ref())
            .map(|binding| {
                binding
                    .value_set
                    .split('|')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_default(),
        short: schema
            .as_ref()
            .and_then(|schema| schema.short.clone())
            .unwrap_or_default(),
        errors: errors.get(&key).cloned().unwrap_or_default(),
        accepts_extension: !is_primitive && offered.iter().any(|option| option.name == "extension"),
        ext_options: if !is_primitive && offered.iter().any(|option| option.name == "extension") {
            let dotted = std::iter::once(resource_type.to_string())
                .chain(path.iter().filter_map(|step| match step {
                    Step::Field(name) => Some(name.clone()),
                    Step::Index(_) => None,
                }))
                .collect::<Vec<_>>()
                .join(".");
            let type_context = schema
                .as_ref()
                .and_then(|schema| schema.type_.clone())
                .unwrap_or_default();
            // The abstract bases come from the shared list so this stays one
            // statement of that set rather than a third hand-copy of it.
            let contexts: Vec<&str> = [resource_type, dotted.as_str(), type_context.as_str()]
                .into_iter()
                .chain(["Element"])
                .chain(helios_fhir::search::ABSTRACT_BASE_TYPES)
                .collect();
            registry
                .extensions_applicable(&contexts)
                .into_iter()
                .take(30)
                .map(|ext| ExtOption {
                    url: ext.url.clone().unwrap_or_default(),
                    name: ext.name.clone().unwrap_or_default(),
                    short: ext.short.clone().unwrap_or_default(),
                })
                .collect()
        } else {
            Vec::new()
        },
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
        build_rows(ctx, &child_path, depth + 1, out);
    }
}

/// Live `$expand` proxy for bound fields (#365): forwards to the configured
/// terminology server and returns a compact JSON code list for the picker.
/// Responds 204 when no terminology server is configured — the picker then
/// stays a plain input.
#[derive(Deserialize)]
pub struct ExpandQuery {
    pub url: String,
    #[serde(default)]
    pub filter: String,
}

pub async fn expand(State(state): State<WebState>, Query(query): Query<ExpandQuery>) -> Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let Some(base) = state.terminology.as_ref() else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let target = format!("{}/ValueSet/$expand", base.trim_end_matches('/'));
    let mut params: Vec<(&str, &str)> = vec![("url", query.url.as_str()), ("count", "25")];
    if !query.filter.is_empty() {
        params.push(("filter", query.filter.as_str()));
    }
    let response = match reqwest::Client::new()
        .get(&target)
        .query(&params)
        .header("Accept", "application/fhir+json")
        .timeout(std::time::Duration::from_millis(2500))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return StatusCode::NO_CONTENT.into_response(),
    };
    let Ok(body) = response.json::<Value>().await else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let codes: Vec<Value> = body["expansion"]["contains"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "code": item["code"].as_str().unwrap_or_default(),
                        "display": item["display"].as_str().unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    axum::Json(serde_json::json!({ "codes": codes })).into_response()
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
        must_support: addable.must_support,
        short: addable.short.unwrap_or_default(),
        slice: addable.slice.unwrap_or_default(),
    }
}
