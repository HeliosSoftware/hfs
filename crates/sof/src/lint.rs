//! Structural + FHIRPath-syntax linting for ViewDefinition documents (#753
//! evaluation POC, ticket 03).
//!
//! [`lint_view_definition`] is the single source of truth for "is this JSON
//! document a well-formed ViewDefinition": it walks a raw [`serde_json::Value`]
//! (never a typed `helios_fhir` resource — a document being edited is often
//! not valid enough to deserialize) and returns every problem it finds,
//! located by [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901) JSON pointer.
//!
//! This is deliberately **structural and syntactic only** — the principle the
//! epic states is "the browser only knows syntax; the server knows FHIR", and
//! this module is the FHIR side of that split for the ViewDefinition shape
//! itself. It does not evaluate FHIRPath expressions, does not resolve
//! terminology, and does not touch storage: every check here is a pure
//! function of the document.
//!
//! # What this checks
//!
//! - The document is `{"resourceType": "ViewDefinition", ...}`
//!   ([`DiagnosticCode::NotAViewDefinition`]).
//! - Every key, at every node, is one this module's own key model (see
//!   [`Node::fields`]) allows for that node
//!   ([`DiagnosticCode::UnknownKey`]), has the JSON type the model expects
//!   ([`DiagnosticCode::WrongType`]), and — for required keys — is present
//!   ([`DiagnosticCode::MissingRequired`]) and non-empty
//!   ([`DiagnosticCode::EmptyRequired`]).
//! - A `select` produces some output
//!   ([`DiagnosticCode::SelectWithoutOutput`]) and carries at most one
//!   iteration directive ([`DiagnosticCode::MultipleIterationDirectives`]).
//! - Column names don't collide within one output row
//!   ([`DiagnosticCode::DuplicateColumnName`]).
//! - Every FHIRPath expression (`column[].path`, `where[].path`, `forEach`,
//!   `forEachOrNull`, each element of `repeat`) parses
//!   ([`DiagnosticCode::FhirPathSyntax`]), via [`helios_fhirpath`]'s parser —
//!   syntax only, never evaluated.
//!
//! [`DiagnosticCode::UndeclaredConstant`] is reserved for a rule ("`%x` used
//! without a matching `constant[].name`") this POC does not implement; see
//! its own docs for why.
//!
//! # Example
//!
//! ```
//! use helios_sof::lint::{lint_view_definition, DiagnosticCode, Severity};
//! use serde_json::json;
//!
//! let doc = json!({
//!     "resourceType": "ViewDefinition",
//!     "status": "active",
//!     "resource": "Patient",
//!     "select": [{
//!         "column": [{ "name": "id", "path": "getResourceKey(" }]
//!     }]
//! });
//!
//! let diagnostics = lint_view_definition(&doc);
//! assert_eq!(diagnostics.len(), 1);
//! assert_eq!(diagnostics[0].code, DiagnosticCode::FhirPathSyntax);
//! assert_eq!(diagnostics[0].severity, Severity::Error);
//! assert_eq!(diagnostics[0].pointer, "/select/0/column/0/path");
//! ```

use serde_json::Value;

// ---------------------------------------------------------------------------
// Public types (RF1)
// ---------------------------------------------------------------------------

/// A location inside the **string value** a [`Diagnostic`] points at,
/// expressed in Unicode `char` offsets — never UTF-8 bytes — so a browser
/// counting Unicode code points (or anything else that is not counting raw
/// bytes) can index into the string directly. Only ever set for
/// [`DiagnosticCode::FhirPathSyntax`] (and, if implemented,
/// [`DiagnosticCode::UndeclaredConstant`]) — every other diagnostic already
/// locates itself precisely enough with `pointer` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// How serious a [`Diagnostic`] is. Nothing in this POC blocks Save — both
/// severities are informational, but `Warning` is reserved for checks (like
/// [`DiagnosticCode::UndeclaredConstant`]) that flag something suspicious
/// rather than something the ViewDefinition spec outright forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

/// What kind of problem a [`Diagnostic`] reports. `#[non_exhaustive]`: this
/// is a POC rule set (see the module docs for what is deliberately out of
/// scope), and the implementation issue this epic produces is expected to
/// add codes, not just consumers matching on the ones that exist today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// The document is not `{"resourceType": "ViewDefinition", ...}`. When
    /// this fires it is always the *only* diagnostic — nothing else about
    /// the document can be meaningfully checked.
    NotAViewDefinition,
    /// A key this module's model does not recognize for its node.
    UnknownKey,
    /// A key the model marks required is absent.
    MissingRequired,
    /// A key's value is not the JSON type the model expects for it.
    WrongType,
    /// A required string is empty or all whitespace, or a required array is
    /// empty.
    EmptyRequired,
    /// Two columns feeding the same output row share a `name`.
    DuplicateColumnName,
    /// A `select` sets more than one of `forEach`, `forEachOrNull`, `repeat`
    /// — the `sql-expressions` invariant `validate_select_with_context`
    /// already enforces at run time; this is its structural, all-errors
    /// counterpart.
    MultipleIterationDirectives,
    /// A `select` has none of `column`, `select`, `unionAll` — it can never
    /// produce a column.
    SelectWithoutOutput,
    /// A FHIRPath expression does not parse.
    ///
    /// `#[serde(rename)]` overrides the enum's own `kebab-case`: serde's
    /// auto-casing splits on every capital, which would turn `FhirPath`
    /// into `fhir-path` (two words) instead of the one word `fhirpath` the
    /// wire contract (RF1's own example, and the rest of this codebase's
    /// naming — `helios-fhirpath`, `helios_fhirpath`) uses everywhere else.
    #[serde(rename = "fhirpath-syntax")]
    FhirPathSyntax,
    /// **Not implemented in this POC** — see its own module-level note.
    UndeclaredConstant,
}

/// One problem [`lint_view_definition`] found, located by
/// [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901) JSON pointer
/// (`""` is the document root; `~0`/`~1` escape `~`/`/` inside a key).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub pointer: String,
    pub message: String,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub span: Option<Span>,
}

// ---------------------------------------------------------------------------
// The key model (RF2) — the single source of truth this module's UnknownKey/
// MissingRequired/WrongType checks drive off. Cross-checked in `tests` below
// against the generated `helios_fhir::r4` structs.
// ---------------------------------------------------------------------------

/// The JSON shape a modeled key's value must have.
#[derive(Clone, Copy)]
enum Kind {
    String,
    Number,
    Boolean,
    /// An array whose elements must each be a JSON string (`repeat`).
    StringArray,
    /// A nested object, itself checked against another node's model.
    Object(Node),
    /// An array whose elements must each be an object, checked against
    /// another node's model (`column`, `select`, `unionAll`, `constant`,
    /// `where`, `tag`).
    ObjectArray(Node),
    /// Accepted, but this POC's model does not police its shape further —
    /// every one of these is a real `ViewDefinition` field (see the
    /// `key_model_matches_generated_structs` test) whose own JSON type is
    /// either a full FHIR datatype (`Meta`, `Period`, `ContactDetail`, a
    /// `Reference`-bearing `Extension`, ...) or a `value[x]` choice —
    /// modeling those precisely is real scope this POC does not need to
    /// carry to prove the CM6 + server-lint architecture out.
    Any,
}

impl Kind {
    /// Whether this kind is a primitive (or array of primitives) — the FHIR
    /// primitive-extension convention (`"_status": {"extension": [...]}`)
    /// only applies to *primitive* values, never to objects or object
    /// arrays.
    fn is_primitive(self) -> bool {
        matches!(
            self,
            Kind::String | Kind::Number | Kind::Boolean | Kind::StringArray
        )
    }
}

/// One key this model allows on some [`Node`], and whether it is required.
struct Field {
    key: &'static str,
    required: bool,
    kind: Kind,
}

/// The node kinds a ViewDefinition document is built from. Each maps to one
/// generated `helios_fhir::r4` struct (`Node::fields` names it), which is
/// what the RF2 cross-check test in `tests` verifies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    /// `helios_fhir::r4::ViewDefinition`.
    Root,
    /// `helios_fhir::r4::ViewDefinitionSelect`.
    Select,
    /// `helios_fhir::r4::ViewDefinitionSelectColumn`.
    Column,
    /// `helios_fhir::r4::ViewDefinitionSelectColumnTag`.
    Tag,
    /// `helios_fhir::r4::ViewDefinitionWhere`.
    Where,
    /// `helios_fhir::r4::ViewDefinitionConstant`.
    Constant,
}

impl Node {
    /// The keys this node accepts, in the order they appear on the
    /// generated struct.
    ///
    /// `id`/`extension`/`modifierExtension` are `ViewDefinition`'s own
    /// fields (inherited from the base `Resource`/`DomainResource` shape) —
    /// they are declared here, on [`Node::Root`], and nowhere else, because
    /// the generated backbone structs for `select`/`column`/`tag`/`where`/
    /// `constant` do not carry them (verified directly against
    /// `crates/fhir/src/r4.rs`: none of `ViewDefinitionSelect`,
    /// `ViewDefinitionSelectColumn`, `ViewDefinitionSelectColumnTag`,
    /// `ViewDefinitionWhere`, or `ViewDefinitionConstant` has an `id`,
    /// `extension`, or `modifierExtension` field). RF2's own principle —
    /// the model matches the generated structs — is what settles this in
    /// favor of the struct over the ticket's more general prose.
    fn fields(self) -> &'static [Field] {
        match self {
            Node::Root => &[
                Field {
                    key: "resourceType",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "id",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "meta",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "implicitRules",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "language",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "text",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "contained",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "extension",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "modifierExtension",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "url",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "identifier",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "version",
                    required: false,
                    kind: Kind::String,
                },
                // Choice type (value[x]-style): the JSON key carries the
                // chosen variant's suffix, never the bare "versionAlgorithm".
                Field {
                    key: "versionAlgorithmString",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "versionAlgorithmCoding",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "name",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "title",
                    required: false,
                    kind: Kind::String,
                },
                // RF3's prose lists `status` as required alongside `resource`/
                // `select`, matching the generated struct's non-Option `status:
                // Code` field - but the official SQL-on-FHIR conformance fixtures
                // this crate already vendors (crates/sof/tests/sql-on-fhir/tests/)
                // omit it in 33 of 133 non-error test views, and none of those are
                // `expectError` cases. helios-fhir's generated deserializer also
                // defaults a missing required scalar rather than rejecting it (see
                // the `key_model_matches_generated_structs` test), so nothing else
                // in this codebase treats `status` as load-bearing either. Modeled
                // as optional so this lint agrees with its own acceptance bar: the
                // existing suite's valid ViewDefinitions must lint clean.
                Field {
                    key: "status",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "experimental",
                    required: false,
                    kind: Kind::Boolean,
                },
                Field {
                    key: "date",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "publisher",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "contact",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "description",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "useContext",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "jurisdiction",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "purpose",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "copyright",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "copyrightLabel",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "approvalDate",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "lastReviewDate",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "effectivePeriod",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "topic",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "author",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "editor",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "reviewer",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "endorser",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "relatedArtifact",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "resource",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "profile",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "fhirVersion",
                    required: false,
                    kind: Kind::Any,
                },
                Field {
                    key: "constant",
                    required: false,
                    kind: Kind::ObjectArray(Node::Constant),
                },
                Field {
                    key: "select",
                    required: true,
                    kind: Kind::ObjectArray(Node::Select),
                },
                Field {
                    key: "where",
                    required: false,
                    kind: Kind::ObjectArray(Node::Where),
                },
            ],
            Node::Select => &[
                Field {
                    key: "column",
                    required: false,
                    kind: Kind::ObjectArray(Node::Column),
                },
                Field {
                    key: "select",
                    required: false,
                    kind: Kind::ObjectArray(Node::Select),
                },
                Field {
                    key: "forEach",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "forEachOrNull",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "repeat",
                    required: false,
                    kind: Kind::StringArray,
                },
                Field {
                    key: "unionAll",
                    required: false,
                    kind: Kind::ObjectArray(Node::Select),
                },
            ],
            Node::Column => &[
                Field {
                    key: "path",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "name",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "description",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "collection",
                    required: false,
                    kind: Kind::Boolean,
                },
                Field {
                    key: "type",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "tag",
                    required: false,
                    kind: Kind::ObjectArray(Node::Tag),
                },
            ],
            Node::Tag => &[
                Field {
                    key: "name",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "value",
                    required: true,
                    kind: Kind::String,
                },
            ],
            Node::Where => &[
                Field {
                    key: "path",
                    required: true,
                    kind: Kind::String,
                },
                Field {
                    key: "description",
                    required: false,
                    kind: Kind::String,
                },
            ],
            Node::Constant => &[
                Field {
                    key: "name",
                    required: true,
                    kind: Kind::String,
                },
                // value[x]: exactly one of 18 keys — handled specially in
                // `check_constant_value`, not through the generic
                // required-field loop (a choice type has no single "the
                // key", so `required` here would be meaningless).
                Field {
                    key: "valueBase64Binary",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueBoolean",
                    required: false,
                    kind: Kind::Boolean,
                },
                Field {
                    key: "valueCanonical",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueCode",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueDate",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueDateTime",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueDecimal",
                    required: false,
                    kind: Kind::Number,
                },
                Field {
                    key: "valueId",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueInstant",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueInteger",
                    required: false,
                    kind: Kind::Number,
                },
                Field {
                    key: "valueOid",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueString",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valuePositiveInt",
                    required: false,
                    kind: Kind::Number,
                },
                Field {
                    key: "valueTime",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueUnsignedInt",
                    required: false,
                    kind: Kind::Number,
                },
                Field {
                    key: "valueUri",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueUrl",
                    required: false,
                    kind: Kind::String,
                },
                Field {
                    key: "valueUuid",
                    required: false,
                    kind: Kind::String,
                },
            ],
        }
    }
}

/// The 18 `value[x]` keys on [`Node::Constant`] — see `check_constant_value`.
const CONSTANT_VALUE_KEYS: &[&str] = &[
    "valueBase64Binary",
    "valueBoolean",
    "valueCanonical",
    "valueCode",
    "valueDate",
    "valueDateTime",
    "valueDecimal",
    "valueId",
    "valueInstant",
    "valueInteger",
    "valueOid",
    "valueString",
    "valuePositiveInt",
    "valueTime",
    "valueUnsignedInt",
    "valueUri",
    "valueUrl",
    "valueUuid",
];

// ---------------------------------------------------------------------------
// Public entry point (RF1)
// ---------------------------------------------------------------------------

/// Lints `doc` as a ViewDefinition and returns every diagnostic found,
/// ordered by `pointer` in document order (array elements sort by their
/// numeric index, not lexicographically) and stable across runs.
///
/// Never panics — every branch below degrades to "not this shape, nothing
/// more to check here" rather than indexing or unwrapping into a `Value`
/// that turned out not to have the shape a rule expected.
///
/// See the module docs for what this checks and what it deliberately does
/// not.
pub fn lint_view_definition(doc: &Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let Some(root) = doc.as_object() else {
        diagnostics.push(Diagnostic {
            pointer: String::new(),
            message: "a ViewDefinition document must be a JSON object".to_string(),
            severity: Severity::Error,
            code: DiagnosticCode::NotAViewDefinition,
            span: None,
        });
        return diagnostics;
    };

    let resource_type = root.get("resourceType").and_then(Value::as_str);
    if resource_type != Some("ViewDefinition") {
        let found = match root.get("resourceType") {
            None => "no `resourceType`".to_string(),
            Some(Value::String(s)) => format!("resourceType {s:?}"),
            Some(other) => format!("a non-string resourceType ({})", type_name(other)),
        };
        diagnostics.push(Diagnostic {
            pointer: String::new(),
            message: format!("expected resourceType \"ViewDefinition\", found {found}"),
            severity: Severity::Error,
            code: DiagnosticCode::NotAViewDefinition,
            span: None,
        });
        return diagnostics;
    }

    validate_node(Node::Root, doc, "", &mut diagnostics);

    if let Some(constants) = root.get("constant").and_then(Value::as_array) {
        for (i, constant) in constants.iter().enumerate() {
            check_constant_value(constant, &format!("/constant/{i}"), &mut diagnostics);
        }
    }

    if let Some(selects) = root.get("select").and_then(Value::as_array) {
        let mut column_scope = Vec::new();
        for (i, select) in selects.iter().enumerate() {
            let pointer = format!("/select/{i}");
            check_select_shape(select, &pointer, &mut diagnostics);
            column_scope =
                check_duplicate_columns(select, &pointer, column_scope, &mut diagnostics);
            check_fhirpath_in_select(select, &pointer, &mut diagnostics);
        }
    }

    if let Some(wheres) = root.get("where").and_then(Value::as_array) {
        for (i, w) in wheres.iter().enumerate() {
            if let Some(path) = w.get("path").and_then(Value::as_str) {
                check_expression(path, &format!("/where/{i}/path"), &mut diagnostics);
            }
        }
    }

    // RF1: ordered by true document position, not by comparing pointer
    // text (which would sort `/select/*` before `/where/*` purely because
    // "select" < "where" lexicographically, regardless of which one the
    // source document actually declares first). `sort_by_cached_key`
    // computes each key once and is a stable sort, so diagnostics that
    // land on the exact same position (e.g. two MissingRequired on the
    // same container) keep their original relative order — deterministic
    // since every pass above walks the document in a fixed, repeatable
    // order.
    diagnostics.sort_by_cached_key(|d| {
        (
            document_position(doc, &d.pointer),
            d.span.map(|span| span.start).unwrap_or(0),
        )
    });
    diagnostics
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

// ---------------------------------------------------------------------------
// UnknownKey / MissingRequired / WrongType / EmptyRequired (RF3)
// ---------------------------------------------------------------------------

/// Checks every key on `value` (a node of kind `node`) against
/// [`Node::fields`], recursing into any `Object`/`ObjectArray` field whose
/// own value has the right JSON type. `value` is assumed to already be
/// known to be an object — call sites that hold an `Option<&Value>` check
/// that first (as part of their own `WrongType` handling), since "not an
/// object" is itself something the *caller* reports.
fn validate_node(node: Node, value: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    let Some(obj) = value.as_object() else {
        return;
    };
    let fields = node.fields();

    for (key, val) in obj {
        if let Some(base) = key.strip_prefix('_') {
            // FHIR's primitive-extension sibling (`"_status": {"id": ...,
            // "extension": [...]}`) is legal on any primitive field this
            // node has, regardless of whether the node itself carries its
            // own id/extension (see the `Node::fields` doc comment on why
            // that is not the same question).
            if fields
                .iter()
                .any(|f| f.key == base && f.kind.is_primitive())
            {
                continue;
            }
        }
        match fields.iter().find(|f| f.key == key.as_str()) {
            Some(f) => check_field_value(f, val, pointer, out),
            None => out.push(unknown_key(pointer, key)),
        }
    }

    for f in fields.iter().filter(|f| f.required) {
        if !obj.contains_key(f.key) {
            out.push(missing_required(pointer, f.key));
        }
    }
}

/// Type-checks one field's value and, for required fields, its emptiness;
/// recurses into `Object`/`ObjectArray`/`StringArray` element shapes when
/// the value's own type is correct. Per RF3, a `WrongType` at this node
/// stops here — nothing inside a value of the wrong shape is inspected.
fn check_field_value(f: &Field, val: &Value, parent_pointer: &str, out: &mut Vec<Diagnostic>) {
    let pointer = child_pointer(parent_pointer, f.key);
    let type_ok = match f.kind {
        Kind::Any => true,
        Kind::String => val.is_string(),
        Kind::Number => val.is_number(),
        Kind::Boolean => val.is_boolean(),
        Kind::StringArray | Kind::ObjectArray(_) => val.is_array(),
        Kind::Object(_) => val.is_object(),
    };
    if !type_ok {
        out.push(wrong_type(&pointer, f.kind, val));
        return;
    }

    if f.required && is_empty(f.kind, val) {
        out.push(empty_required(&pointer));
        return;
    }

    match f.kind {
        Kind::Any | Kind::String | Kind::Number | Kind::Boolean => {}
        Kind::StringArray => {
            for (i, item) in val.as_array().into_iter().flatten().enumerate() {
                if !item.is_string() {
                    out.push(wrong_type(&format!("{pointer}/{i}"), Kind::String, item));
                }
            }
        }
        Kind::Object(node) => validate_node(node, val, &pointer, out),
        Kind::ObjectArray(node) => {
            for (i, item) in val.as_array().into_iter().flatten().enumerate() {
                let item_pointer = format!("{pointer}/{i}");
                if item.is_object() {
                    validate_node(node, item, &item_pointer, out);
                } else {
                    out.push(wrong_type(&item_pointer, Kind::Object(node), item));
                }
            }
        }
    }
}

fn is_empty(kind: Kind, val: &Value) -> bool {
    match kind {
        Kind::String => val.as_str().is_some_and(|s| s.trim().is_empty()),
        Kind::StringArray | Kind::ObjectArray(_) => val.as_array().is_some_and(|a| a.is_empty()),
        Kind::Any | Kind::Number | Kind::Boolean | Kind::Object(_) => false,
    }
}

/// The `value[x]` choice on a `constant`: exactly one of the 18
/// `CONSTANT_VALUE_KEYS` must be present. `validate_node` already checked
/// each key's own JSON type (and flagged `UnknownKey` for anything else);
/// this only checks *how many* of the 18 are present, since that is a
/// cross-key rule the generic per-field loop cannot express.
fn check_constant_value(constant: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    let Some(obj) = constant.as_object() else {
        return;
    };
    let present: Vec<&str> = CONSTANT_VALUE_KEYS
        .iter()
        .filter(|k| obj.contains_key(**k))
        .copied()
        .collect();
    match present.len() {
        0 => out.push(missing_required(pointer, "value[x]")),
        1 => {}
        _ => out.push(Diagnostic {
            pointer: pointer.to_string(),
            message: format!(
                "a constant may set only one value[x] key, found {}: {}",
                present.len(),
                present.join(", ")
            ),
            severity: Severity::Error,
            code: DiagnosticCode::WrongType,
            span: None,
        }),
    }
}

fn unknown_key(pointer: &str, key: &str) -> Diagnostic {
    Diagnostic {
        pointer: child_pointer(pointer, key),
        message: format!("unknown key `{key}`"),
        severity: Severity::Error,
        code: DiagnosticCode::UnknownKey,
        span: None,
    }
}

fn missing_required(pointer: &str, key: &str) -> Diagnostic {
    Diagnostic {
        pointer: pointer.to_string(),
        message: format!("missing required key `{key}`"),
        severity: Severity::Error,
        code: DiagnosticCode::MissingRequired,
        span: None,
    }
}

fn empty_required(pointer: &str) -> Diagnostic {
    Diagnostic {
        pointer: pointer.to_string(),
        message: "required value must not be empty".to_string(),
        severity: Severity::Error,
        code: DiagnosticCode::EmptyRequired,
        span: None,
    }
}

fn wrong_type(pointer: &str, expected: Kind, found: &Value) -> Diagnostic {
    let expected = match expected {
        Kind::String => "a string",
        Kind::Number => "a number",
        Kind::Boolean => "a boolean",
        Kind::StringArray | Kind::ObjectArray(_) => "an array",
        Kind::Object(_) => "an object",
        Kind::Any => "any value",
    };
    Diagnostic {
        pointer: pointer.to_string(),
        message: format!("expected {expected}, found {}", type_name(found)),
        severity: Severity::Error,
        code: DiagnosticCode::WrongType,
        span: None,
    }
}

// ---------------------------------------------------------------------------
// SelectWithoutOutput / MultipleIterationDirectives (RF3)
// ---------------------------------------------------------------------------

/// A `select` "has" `column`/`select`/`unionAll` for the purposes of
/// [`DiagnosticCode::SelectWithoutOutput`] when the key is present *and* a
/// non-empty array — an empty array produces exactly as little output as an
/// absent key.
fn has_output_content(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key)
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
}

fn check_select_shape(select: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    let Some(obj) = select.as_object() else {
        return;
    };

    if !has_output_content(obj, "column")
        && !has_output_content(obj, "select")
        && !has_output_content(obj, "unionAll")
    {
        out.push(Diagnostic {
            pointer: pointer.to_string(),
            message: "a select must have at least one of column, select, or unionAll".to_string(),
            severity: Severity::Error,
            code: DiagnosticCode::SelectWithoutOutput,
            span: None,
        });
    }

    // Presence-based, like the sql-expressions invariant this mirrors
    // (`validate_select_with_context` above): an empty `repeat: []` still
    // counts as "set", matching `forEach.exists()` in the FHIRPath
    // invariant, which is true for an empty collection too.
    let directive_count = ["forEach", "forEachOrNull", "repeat"]
        .iter()
        .filter(|k| obj.contains_key(**k))
        .count();
    if directive_count > 1 {
        out.push(Diagnostic {
            pointer: pointer.to_string(),
            message: "a select may set at most one of forEach, forEachOrNull, repeat".to_string(),
            severity: Severity::Error,
            code: DiagnosticCode::MultipleIterationDirectives,
            span: None,
        });
    }

    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for (i, child) in nested.iter().enumerate() {
            check_select_shape(child, &format!("{pointer}/select/{i}"), out);
        }
    }
    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for (i, branch) in branches.iter().enumerate() {
            check_select_shape(branch, &format!("{pointer}/unionAll/{i}"), out);
        }
    }
}

// ---------------------------------------------------------------------------
// DuplicateColumnName (RF3)
// ---------------------------------------------------------------------------

/// Walks one `select` (and everything reachable through nested `select[]`)
/// checking for column names already seen in `scope` — every
/// `(name, pointer)` from the *same output row*, threaded in by the caller.
///
/// Returns `scope` extended with every name this select and its non-
/// `unionAll` descendants added, so a caller iterating sibling selects (the
/// document root's own `select[]` array, or a `select`'s own nested
/// `select[]`) can thread duplicate detection across all of them in
/// document order. `unionAll` branches are checked as their own scope
/// (seeded with the caller's `scope`, since they share the containing row)
/// but their columns are never threaded back out — each branch is its own
/// set, and branches never see each other's names.
fn check_duplicate_columns(
    select: &Value,
    pointer: &str,
    mut scope: Vec<(String, String)>,
    out: &mut Vec<Diagnostic>,
) -> Vec<(String, String)> {
    let Some(obj) = select.as_object() else {
        return scope;
    };

    if let Some(columns) = obj.get("column").and_then(Value::as_array) {
        for (i, column) in columns.iter().enumerate() {
            let Some(name) = column.get("name").and_then(Value::as_str) else {
                continue;
            };
            let name_pointer = format!("{pointer}/column/{i}/name");
            if scope.iter().any(|(seen, _)| seen == name) {
                out.push(Diagnostic {
                    pointer: name_pointer.clone(),
                    message: format!("duplicate column name `{name}`"),
                    severity: Severity::Error,
                    code: DiagnosticCode::DuplicateColumnName,
                    span: None,
                });
            }
            scope.push((name.to_string(), name_pointer));
        }
    }

    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for (i, child) in nested.iter().enumerate() {
            scope = check_duplicate_columns(child, &format!("{pointer}/select/{i}"), scope, out);
        }
    }

    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for (i, branch) in branches.iter().enumerate() {
            check_duplicate_columns(
                branch,
                &format!("{pointer}/unionAll/{i}"),
                scope.clone(),
                out,
            );
        }
    }

    scope
}

// ---------------------------------------------------------------------------
// FhirPathSyntax (RF3 + RF4)
// ---------------------------------------------------------------------------

fn check_fhirpath_in_select(select: &Value, pointer: &str, out: &mut Vec<Diagnostic>) {
    let Some(obj) = select.as_object() else {
        return;
    };

    if let Some(columns) = obj.get("column").and_then(Value::as_array) {
        for (i, column) in columns.iter().enumerate() {
            if let Some(path) = column.get("path").and_then(Value::as_str) {
                check_expression(path, &format!("{pointer}/column/{i}/path"), out);
            }
        }
    }
    for key in ["forEach", "forEachOrNull"] {
        if let Some(expr) = obj.get(key).and_then(Value::as_str) {
            check_expression(expr, &format!("{pointer}/{key}"), out);
        }
    }
    if let Some(items) = obj.get("repeat").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            if let Some(expr) = item.as_str() {
                check_expression(expr, &format!("{pointer}/repeat/{i}"), out);
            }
        }
    }
    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for (i, child) in nested.iter().enumerate() {
            check_fhirpath_in_select(child, &format!("{pointer}/select/{i}"), out);
        }
    }
    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for (i, branch) in branches.iter().enumerate() {
            check_fhirpath_in_select(branch, &format!("{pointer}/unionAll/{i}"), out);
        }
    }
}

/// Parses one FHIRPath expression string and, on failure, pushes a single
/// [`DiagnosticCode::FhirPathSyntax`] diagnostic — the first error
/// `helios_fhirpath::parse_expression_diagnostics` reports, which is the one
/// closest to where the parser actually gave up.
fn check_expression(expression: &str, pointer: &str, out: &mut Vec<Diagnostic>) {
    if expression.trim().is_empty() {
        out.push(Diagnostic {
            pointer: pointer.to_string(),
            message: "empty expression".to_string(),
            severity: Severity::Error,
            code: DiagnosticCode::FhirPathSyntax,
            span: Some(Span { start: 0, end: 0 }),
        });
        return;
    }
    if let Err(errors) = helios_fhirpath::parse_expression_diagnostics(expression)
        && let Some(first) = errors.into_iter().next()
    {
        out.push(Diagnostic {
            pointer: pointer.to_string(),
            message: first.message,
            severity: Severity::Error,
            code: DiagnosticCode::FhirPathSyntax,
            span: Some(Span {
                start: first.span.0,
                end: first.span.1,
            }),
        });
    }
}

// ---------------------------------------------------------------------------
// JSON pointers (RFC 6901) and diagnostic ordering (RF1)
// ---------------------------------------------------------------------------

/// Appends `key`, RFC 6901-escaped, to `pointer`.
fn child_pointer(pointer: &str, key: &str) -> String {
    if key.contains('~') || key.contains('/') {
        // `~` must be escaped before `/` — escaping `/` first would turn the
        // `~1` it produces right back into something the `~`-escape step
        // would mangle a second time.
        format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"))
    } else {
        format!("{pointer}/{key}")
    }
}

/// Reverses [`child_pointer`]'s RFC 6901 escaping for one segment. `~1` must
/// be restored to `/` before `~0` is restored to `~` — the reverse of the
/// escaping order — or a literal `~` immediately followed by a literal `/`
/// would round-trip incorrectly.
fn unescape_pointer_segment(segment: &str) -> std::borrow::Cow<'_, str> {
    if segment.contains('~') {
        std::borrow::Cow::Owned(segment.replace("~1", "/").replace("~0", "~"))
    } else {
        std::borrow::Cow::Borrowed(segment)
    }
}

/// Computes a comparable "document position" for `pointer`, without this
/// module ever tracking byte offsets itself: at each object level, a
/// segment's ordinal is its index among ALL of that object's own keys (not
/// just the ones this module's model recognizes) in the order they were
/// declared in the source document — guaranteed to be the true declaration
/// order by `serde_json`'s `preserve_order` feature, enabled on this
/// crate's own `serde_json` dependency (`Cargo.toml`) specifically for
/// this; at each array level, a segment's ordinal is simply its own numeric
/// index.
///
/// Comparing two such paths lexicographically (`Vec<usize>`'s derived
/// `Ord`) recovers true document order: a shared prefix means "the same
/// container", and — because a shorter sequence that is a prefix of a
/// longer one sorts first — a diagnostic on a container itself (e.g.
/// `MissingRequired`, whose pointer is the container, not one of its keys)
/// always sorts before anything reported inside one of that container's own
/// children, exactly matching where the container's own opening `{`/`[`
/// sits relative to its contents in the source text.
///
/// A pointer built from data that is not actually reachable this way (not
/// expected for anything this module itself constructs, but this must
/// never panic regardless) stops at the first segment that does not
/// resolve, returning whatever prefix of the position was found — still a
/// valid, monotonic position, just less precise.
fn document_position(root: &Value, pointer: &str) -> Vec<usize> {
    let mut position = Vec::new();
    let mut node = root;
    for raw_segment in pointer.split('/').skip(1) {
        let segment = unescape_pointer_segment(raw_segment);
        match node {
            Value::Object(map) => match map.keys().position(|k| k == segment.as_ref()) {
                Some(index) => {
                    position.push(index);
                    node = map.get(segment.as_ref()).expect("just located by key");
                }
                None => break,
            },
            Value::Array(items) => match segment.parse::<usize>() {
                Ok(index) if index < items.len() => {
                    position.push(index);
                    node = &items[index];
                }
                _ => break,
            },
            _ => break,
        }
    }
    position
}

#[cfg(test)]
mod tests;
