//! `POST /ui/sql/view-definitions/complete` (#821): the ViewDefinition
//! editor's context-completion endpoint.
//!
//! The architecture this follows (see `/lint`, its sibling) is "the browser
//! knows syntax; the server knows FHIR": a CodeMirror completion source only
//! ever has to say *where the cursor is* — a JSON-pointer node for a
//! structural key, or a pointer plus a FHIRPath expression and a char offset
//! for a partial expression — and this handler answers *what fits there*.
//! Nothing here evaluates FHIRPath, touches storage, or resolves terminology;
//! every candidate list is a pure function of the request body and the
//! embedded FHIR Schema pack for the request's negotiated version.
//!
//! Two request shapes, tagged by `kind`:
//!
//! - `{"kind": "key", "pointer": "...", "present": [...]}` — completion at a
//!   ViewDefinition JSON object node. Candidates come straight from
//!   [`helios_sof::lint::node_keys`], the same key model `/lint`'s
//!   `unknown-key` check is built on, minus whatever the browser says is
//!   already present.
//! - `{"kind": "fhirpath", "pointer": "...", "document": {...}, "expression":
//!   "...", "cursor": N}` — completion inside a partial FHIRPath expression.
//!   `expression` is the field's full current text; `cursor` (a **char**
//!   offset — the browser's editor counts Unicode code points, not UTF-8
//!   bytes) marks where the caret sits, and any text after it is ignored, so
//!   the browser never has to split the string itself.
//!
//! The FHIRPath side runs a heuristic, not a parser: [`classify_cursor`]
//! tokenizes backward from the cursor to decide *what kind* of thing is being
//! typed (a `%constant`, a member after `.`, or a bare root-context token),
//! and — for a member or a `forEach`/`forEachOrNull`/`repeat` expression
//! upstream of the cursor — [`resolve_chain_type`] walks the dotted chain
//! segment by segment to guess the FHIR type it evaluates against. Both are
//! deliberately approximate (see their doc comments for exactly what they
//! do and don't handle); when the heuristic can't tell, the response omits
//! `element` candidates but always still offers `function` (and, in root
//! mode, `constant`/`variable`) — a plain "I don't know the type" degrades to
//! fewer suggestions, never a wrong one.

use axum::Json;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use helios_fhir_validator::SchemaResolver;
use helios_fhir_validator::editor::{self, ElementChild};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RequestVersion;

/// The largest serialized `document` this endpoint will still resolve a
/// FHIRPath completion against. Only `kind: "fhirpath"` bodies carry a whole
/// document; a ViewDefinition big enough to blow this is almost certainly a
/// mistaken paste, not a real editing session, and this endpoint has no
/// business holding it in memory just to answer "what goes here".
const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

/// FHIRPath functions [`resolve_segment`] treats as passing their input type
/// straight through, matched by bare name against a chain segment's
/// `name(args)` invocation. Every other function call (`resolve()`,
/// `select(...)`, ...) resolves to an unknown type.
const TYPE_PRESERVING_FUNCTIONS: &[&str] = &[
    "where",
    "first",
    "last",
    "tail",
    "skip",
    "take",
    "single",
    "exclude",
    "distinct",
    "union",
    "intersect",
    "trace",
];

// ---------------------------------------------------------------------------
// Wire contract
// ---------------------------------------------------------------------------

/// The request body, tagged by `kind`. An unrecognized `kind`, a body that
/// isn't JSON, or one missing a variant's required fields all fail to
/// deserialize the same way — surfaced by [`complete`] as one `400`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum CompleteRequest {
    /// Completion at a structural JSON node — see [`complete_key`].
    Key {
        pointer: String,
        /// Keys the document already has at this node; excluded from the
        /// candidate list. Absent is the same as empty (a freshly-added
        /// node has nothing present yet).
        #[serde(default)]
        present: Vec<String>,
    },
    /// Completion inside a partial FHIRPath expression — see
    /// [`complete_fhirpath`].
    Fhirpath {
        pointer: String,
        document: Value,
        expression: String,
        /// **Char** offset into `expression`, not a UTF-8 byte offset.
        cursor: usize,
    },
}

/// One completion candidate.
#[derive(Debug, Serialize)]
struct Item {
    label: String,
    kind: ItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ItemKind {
    Key,
    Element,
    Function,
    Constant,
    Variable,
}

/// The response body: `from` is the **char** offset into `expression` (`0`
/// for `kind: "key"`, which has no expression text) where the token being
/// typed starts — a browser splices its replacement in at `[from, cursor)`.
#[derive(Debug, Serialize)]
struct CompleteResponse {
    from: usize,
    items: Vec<Item>,
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message.into() })),
    )
        .into_response()
}

/// `POST /ui/sql/view-definitions/complete` (#821).
///
/// No tenant, no htmx swap, no locale — plain JSON in, JSON out, exactly like
/// `/lint`. The FHIR version comes from the same [`RequestVersion`] extractor
/// every page uses (`?version=`/cookie), never from the request body, since
/// nothing about "which schema pack" belongs in a per-node completion query.
pub(crate) async fn complete(rv: RequestVersion, body: Bytes) -> Response {
    let request: CompleteRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return bad_request(format!("invalid request: {error}")),
    };

    let response = match request {
        CompleteRequest::Key { pointer, present } => complete_key(&pointer, &present),
        CompleteRequest::Fhirpath {
            pointer,
            document,
            expression,
            cursor,
        } => {
            let document_size = serde_json::to_vec(&document).map(|bytes| bytes.len());
            if document_size.unwrap_or(0) > MAX_DOCUMENT_BYTES {
                return bad_request("document exceeds 1 MiB");
            }
            let registry = helios_fhir_validator::packs::core_registry(rv.0);
            complete_fhirpath(registry.as_ref(), &document, &pointer, &expression, cursor)
        }
    };

    // NF1: never log the document or the expression — only what shape of
    // request this was and how many candidates came back.
    tracing::debug!(
        item_count = response.items.len(),
        "computed ViewDefinition completion candidates"
    );

    Json(response).into_response()
}

// ---------------------------------------------------------------------------
// kind: "key"
// ---------------------------------------------------------------------------

/// Candidates for a structural JSON node: every key
/// [`helios_sof::lint::node_keys`] allows there, minus `present`, in the key
/// model's own declaration order. `pointer` naming a node the model doesn't
/// recognize (or a scalar/`Any` leaf, which has no keys of its own) answers
/// with no candidates rather than an error — an editor asking "what fits
/// here" about a node this module has no opinion on isn't wrong, it just
/// gets an empty answer.
fn complete_key(pointer: &str, present: &[String]) -> CompleteResponse {
    let items = helios_sof::lint::node_keys(pointer)
        .into_iter()
        .flatten()
        .filter(|key| !present.iter().any(|p| p == key.key))
        .map(|key| Item {
            label: key.key.to_string(),
            kind: ItemKind::Key,
            detail: Some(key_kind_detail(key.kind).to_string()),
            doc: None,
            required: Some(key.required),
        })
        .collect();
    CompleteResponse { from: 0, items }
}

fn key_kind_detail(kind: helios_sof::lint::KeyKind) -> &'static str {
    use helios_sof::lint::KeyKind;
    match kind {
        KeyKind::String => "string",
        KeyKind::Number => "number",
        KeyKind::Boolean => "boolean",
        KeyKind::StringArray => "string[]",
        KeyKind::Object => "object",
        KeyKind::ObjectArray => "object[]",
        KeyKind::Other => "other",
    }
}

// ---------------------------------------------------------------------------
// kind: "fhirpath" — tokenization and type resolution
// ---------------------------------------------------------------------------

/// What [`classify_cursor`] determined about the token at the cursor.
enum Mode {
    /// Start of expression, or right after `(`, `,`, an operator, or
    /// whitespace: offer everything reachable from the current context.
    Root,
    /// Right after a `.`: offer the resolved chain type's own children plus
    /// every function. `chain` is the (unparsed) text to its left.
    Member { chain: String },
    /// Right after `%` (or the token itself starts with `%`): offer only
    /// constants and environment variables.
    Constant,
}

/// Candidates inside a partial FHIRPath expression.
fn complete_fhirpath(
    resolver: &dyn SchemaResolver,
    document: &Value,
    pointer: &str,
    expression: &str,
    cursor: usize,
) -> CompleteResponse {
    let chars: Vec<char> = expression.chars().collect();
    let cursor = cursor.min(chars.len());

    // A cursor inside an unterminated string literal is never a FHIRPath
    // member, function, constant, or variable position.
    if inside_unclosed_string(&chars[..cursor]) {
        return CompleteResponse {
            from: cursor,
            items: Vec::new(),
        };
    }

    let (from, mode) = classify_cursor(&chars, cursor);

    let root_type = root_resource_type(resolver, document);
    let context_type = resolve_context_type(resolver, document, pointer, root_type.as_deref());

    let mut items = Vec::new();
    match mode {
        Mode::Constant => {
            items.extend(constant_items(document));
            items.extend(variable_items());
        }
        Mode::Member { chain } => {
            let resolved = resolve_chain_type(
                resolver,
                root_type.as_deref(),
                context_type.as_deref(),
                &chain,
            );
            items.extend(element_items(resolver, resolved.as_deref()));
            items.extend(function_items());
        }
        Mode::Root => {
            items.extend(element_items(resolver, context_type.as_deref()));
            items.extend(function_items());
            items.extend(constant_items(document));
            items.extend(variable_items());
        }
    }

    CompleteResponse { from, items }
}

/// Whether `prefix` (every char of `expression` before the cursor) leaves an
/// FHIRPath `'...'` string literal open — a `\`-escaped `'` inside one
/// doesn't close it.
fn inside_unclosed_string(prefix: &[char]) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for &c in prefix {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '\'' {
                in_string = false;
            }
        } else if c == '\'' {
            in_string = true;
        }
    }
    in_string
}

/// The tokenizer: scans `chars[..cursor]` right to left to find the token
/// being typed and classify the position it starts at.
///
/// The token is the longest trailing run of `[A-Za-z0-9_]`, then trimmed on
/// the left of any leading digits (a FHIRPath identifier can't start with
/// one — a bare digit run is a numeric literal, not a partial identifier, so
/// it contributes an empty token instead). `from` is that (possibly empty)
/// token's start, in char offset — except in constant mode, where it backs
/// up one more char to point at the `%` itself.
fn classify_cursor(chars: &[char], cursor: usize) -> (usize, Mode) {
    let mut token_start = cursor;
    while token_start > 0
        && (chars[token_start - 1].is_ascii_alphanumeric() || chars[token_start - 1] == '_')
    {
        token_start -= 1;
    }
    let mut ident_start = token_start;
    while ident_start < cursor && chars[ident_start].is_ascii_digit() {
        ident_start += 1;
    }

    if ident_start > 0 && chars[ident_start - 1] == '%' {
        return (ident_start - 1, Mode::Constant);
    }
    if ident_start > 0 && chars[ident_start - 1] == '.' {
        let chain: String = chars[..ident_start - 1].iter().collect();
        return (ident_start, Mode::Member { chain });
    }
    (ident_start, Mode::Root)
}

// ---------------------------------------------------------------------------
// Type resolution
// ---------------------------------------------------------------------------

/// `document.resource`, if it's a string the request's version registry
/// resolves — unknown (`None`) otherwise, so a document with no `resource`
/// yet (or a typo'd one) degrades to no `element` candidates instead of an
/// error.
fn root_resource_type(resolver: &dyn SchemaResolver, document: &Value) -> Option<String> {
    let name = document.get("resource")?.as_str()?;
    resolver.resolve(name)?;
    Some(name.to_string())
}

/// The type `%context` resolves to at `pointer` — the root type, narrowed by
/// each ancestor `select`'s own `forEach`/`forEachOrNull`/(first element of)
/// `repeat`, from the outermost `select` in, each resolved with
/// [`resolve_chain_type`] from the context the previous one established.
///
/// When `pointer` names a `select`'s own iteration expression (its
/// `forEach`, `forEachOrNull`, or a `repeat` element), that `select` is
/// excluded from its own context chain — an iteration expression is
/// evaluated in the context *above* it, not one it hasn't finished defining.
fn resolve_context_type(
    resolver: &dyn SchemaResolver,
    document: &Value,
    pointer: &str,
    root_type: Option<&str>,
) -> Option<String> {
    let mut ancestors = select_ancestor_pointers(pointer);
    if let Some(last) = ancestors.last() {
        let is_own_iteration_expression = pointer == format!("{last}/forEach")
            || pointer == format!("{last}/forEachOrNull")
            || pointer.starts_with(&format!("{last}/repeat/"));
        if is_own_iteration_expression {
            ancestors.pop();
        }
    }

    let mut context = root_type.map(str::to_string);
    for select_pointer in ancestors {
        let Some(expression) = select_iteration_expression(document, &select_pointer) else {
            continue;
        };
        context = resolve_chain_type(resolver, root_type, context.as_deref(), expression);
    }
    context
}

/// Every ancestor `select` array element's own JSON pointer along the path
/// to `pointer`, outermost first — e.g. `/select/0/select/1/column/0/path`
/// yields `["/select/0", "/select/0/select/1"]`. Matches a `select` segment
/// immediately followed by a numeric index anywhere in the pointer, so a
/// `select` nested under `unionAll` is found the same way as one nested
/// directly under another `select`.
fn select_ancestor_pointers(pointer: &str) -> Vec<String> {
    let segments: Vec<&str> = pointer.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    let mut prefix = String::new();
    let mut i = 0;
    while i < segments.len() {
        let is_index = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if segments[i] == "select" && segments.get(i + 1).is_some_and(|s| is_index(s)) {
            prefix.push_str("/select/");
            prefix.push_str(segments[i + 1]);
            out.push(prefix.clone());
            i += 2;
        } else {
            prefix.push('/');
            prefix.push_str(segments[i]);
            i += 1;
        }
    }
    out
}

/// The FHIRPath expression driving one `select`'s iteration — its `forEach`,
/// else `forEachOrNull`, else the first element of `repeat` — read straight
/// off `document` at `select_pointer`. `None` when the select structurally
/// has none of the three (already reported elsewhere by
/// [`helios_sof::lint::lint_view_definition`]).
fn select_iteration_expression<'a>(document: &'a Value, select_pointer: &str) -> Option<&'a str> {
    document
        .pointer(&format!("{select_pointer}/forEach"))
        .and_then(Value::as_str)
        .or_else(|| {
            document
                .pointer(&format!("{select_pointer}/forEachOrNull"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            document
                .pointer(&format!("{select_pointer}/repeat/0"))
                .and_then(Value::as_str)
        })
}

/// The chain-resolution heuristic: splits `chain` into top-level
/// (paren/bracket/string-respecting) `.`-separated segments and walks them
/// left to right, updating a running type starting from `context_type` —
/// never actually evaluating anything.
fn resolve_chain_type(
    resolver: &dyn SchemaResolver,
    root_type: Option<&str>,
    context_type: Option<&str>,
    chain: &str,
) -> Option<String> {
    let mut current = context_type.map(str::to_string);
    for (index, segment) in split_top_level_segments(chain).iter().enumerate() {
        current = resolve_segment(
            resolver,
            root_type,
            context_type,
            current.as_deref(),
            segment,
            index == 0,
        );
    }
    current
}

/// One segment of a dotted chain, resolved against `current` (the type
/// carried in from the segment before it):
///
/// - `%resource`/`%rootResource` → `root_type`; `%context` → `context_type`;
///   any other `%name` → unknown (a declared constant's type isn't tracked
///   here — only its presence, by [`constant_items`]).
/// - The very first segment, spelling out `root_type` or `context_type`
///   verbatim (`Patient.name` starting with the literal `Patient`) → passes
///   `current` through unchanged rather than looking it up as a member.
/// - `ofType(T)`/`as(T)` → `T`, verbatim. `extension(...)` → `Extension`.
///   [`TYPE_PRESERVING_FUNCTIONS`] → `current`, unchanged. Any other
///   `name(args)` call (`resolve()`, `select(...)`, ...) → unknown.
/// - A bare identifier, optionally followed by one or more `[...]`
///   indexers (which never change the type) → that name's child type on
///   `current`, via [`child_type`].
/// - Anything else (an operator, a literal, `%const`) → unknown.
fn resolve_segment(
    resolver: &dyn SchemaResolver,
    root_type: Option<&str>,
    context_type: Option<&str>,
    current: Option<&str>,
    segment: &str,
    is_first: bool,
) -> Option<String> {
    let stripped = strip_trailing_indexers(segment.trim());

    if let Some(open) = stripped.find('(') {
        if !stripped.ends_with(')') {
            return None;
        }
        let name = stripped[..open].trim();
        let args = stripped[open + 1..stripped.len() - 1].trim();
        return match name {
            "ofType" | "as" if !args.is_empty() => Some(args.to_string()),
            "extension" => Some("Extension".to_string()),
            name if TYPE_PRESERVING_FUNCTIONS.contains(&name) => current.map(str::to_string),
            _ => None,
        };
    }

    if let Some(name) = stripped.strip_prefix('%') {
        return match name {
            "resource" | "rootResource" => root_type.map(str::to_string),
            "context" => context_type.map(str::to_string),
            _ => None,
        };
    }

    if stripped.is_empty() {
        return current.map(str::to_string);
    }
    if is_first && (Some(stripped) == root_type || Some(stripped) == context_type) {
        return current.map(str::to_string);
    }
    if is_identifier(stripped) {
        return child_type(resolver, current, stripped);
    }
    None
}

/// Strips zero or more trailing `[...]` indexers — `d[0][1]` → `d` — since an
/// indexer never changes a segment's type.
fn strip_trailing_indexers(segment: &str) -> &str {
    let mut s = segment;
    while s.ends_with(']') {
        match s.rfind('[') {
            Some(open) => s = &s[..open],
            None => break,
        }
    }
    s
}

/// A FHIRPath bare identifier: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `field`'s type as a child of `current`, via [`element_children`] — unknown
/// when `current` itself is unknown, `field` isn't one of `current`'s
/// children, or it names a choice group (an ambiguous type, unless the next
/// chain segment narrows it with `ofType`/`as` — handled by
/// [`resolve_segment`] processing that segment next, not by this lookup
/// peeking ahead).
fn child_type(resolver: &dyn SchemaResolver, current: Option<&str>, field: &str) -> Option<String> {
    let type_name = current?;
    let children = editor::element_children(resolver, type_name, &[])?;
    let child = children.into_iter().find(|c| c.name == field)?;
    match child.types.as_slice() {
        [single] => Some(single.clone()),
        _ => None,
    }
}

/// Splits `chain` into `.`-separated segments at depth 0 — inside `(...)`,
/// `[...]`, or a `'...'` string literal, a `.` doesn't split.
fn split_top_level_segments(chain: &str) -> Vec<String> {
    let chars: Vec<char> = chain.chars().collect();
    let mut segments = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0usize;

    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '\'' {
                in_string = false;
            }
            continue;
        }
        match c {
            '\'' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '.' if depth == 0 => {
                segments.push(chars[start..i].iter().collect::<String>());
                start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(chars[start..].iter().collect::<String>());

    segments
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Candidate lists
// ---------------------------------------------------------------------------

/// `element` candidates: `type_name`'s own children, via
/// [`helios_fhir_validator::editor::element_children`]. Empty for an unknown
/// type (`None`) or one the resolver doesn't recognize.
fn element_items(resolver: &dyn SchemaResolver, type_name: Option<&str>) -> Vec<Item> {
    let Some(type_name) = type_name else {
        return Vec::new();
    };
    editor::element_children(resolver, type_name, &[])
        .into_iter()
        .flatten()
        .map(element_item)
        .collect()
}

fn element_item(child: ElementChild) -> Item {
    let mut detail = child.types.join(" | ");
    if child.is_collection {
        detail.push_str("[]");
    }
    Item {
        label: child.name,
        kind: ItemKind::Element,
        detail: Some(detail),
        doc: child.short,
        required: None,
    }
}

/// `function` candidates: the full [`helios_sof::lint::builtin_functions`]
/// catalog — prefix filtering against what the user has typed so far is the
/// browser's job, not this endpoint's.
fn function_items() -> Vec<Item> {
    helios_sof::lint::builtin_functions()
        .iter()
        .map(|info| Item {
            label: info.name.to_string(),
            kind: ItemKind::Function,
            detail: Some(info.signature.to_string()),
            doc: None,
            required: None,
        })
        .collect()
}

/// `constant` candidates: one per `document.constant[]` entry with a string
/// `name`, in document order. `detail` is the primitive type name derived
/// from whichever `value[x]` key is present (`valueString` → `string`), or
/// the literal `"unknown"` when none is.
fn constant_items(document: &Value) -> Vec<Item> {
    document
        .get("constant")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|constant| {
            let name = constant.get("name")?.as_str()?;
            Some(Item {
                label: format!("%{name}"),
                kind: ItemKind::Constant,
                detail: Some(constant_value_type(constant)),
                doc: None,
                required: None,
            })
        })
        .collect()
}

fn constant_value_type(constant: &Value) -> String {
    constant
        .as_object()
        .and_then(|fields| {
            fields.keys().find_map(|key| {
                let rest = key.strip_prefix("value")?;
                let mut chars = rest.chars();
                let first = chars.next().filter(char::is_ascii_uppercase)?;
                Some(first.to_ascii_lowercase().to_string() + chars.as_str())
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// `variable` candidates: [`helios_sof::lint::environment_variables`] — the
/// FHIRPath environment variables the evaluator resolves. SQL-on-FHIR's own
/// `%rowIndex` is deliberately not included here — `variable` is scoped to
/// exactly this catalog.
fn variable_items() -> Vec<Item> {
    helios_sof::lint::environment_variables()
        .iter()
        .map(|name| Item {
            label: format!("%{name}"),
            kind: ItemKind::Variable,
            detail: None,
            doc: None,
            required: None,
        })
        .collect()
}
