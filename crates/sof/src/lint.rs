//! Structural + FHIRPath-syntax linting for ViewDefinition documents (#753
//! evaluation POC, matured into the single lint engine for `$sql-run`,
//! `sof-cli`, `pysof`, and the ViewDefinition editor by #821).
//!
//! [`lint_view_definition`] is the single source of truth for "is this JSON
//! document a well-formed ViewDefinition": it walks a raw [`serde_json::Value`]
//! (never a typed `helios_fhir` resource — a document being edited is often
//! not valid enough to deserialize) and returns every problem it finds,
//! located by [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901) JSON pointer.
//!
//! This is deliberately **structural and syntactic only** — the principle
//! #821 states is "the browser only knows syntax; the server knows FHIR", and
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
//! - Every `%name` reference inside a FHIRPath expression that parses
//!   successfully names something that actually exists: an entry in
//!   `constant[].name`, one of the FHIRPath environment variables the
//!   evaluator resolves (`%context`, `%resource`, `%rootResource`, `%ucum`,
//!   `%sct`, `%loinc`), or a SQL-on-FHIR environment variable this crate
//!   itself binds (`%rowIndex`) ([`DiagnosticCode::UndeclaredConstant`]).
//!   Locating the reference still doesn't evaluate the expression — it
//!   walks the parsed AST [`helios_fhirpath::external_constants`] returns.
//!
//! # Actionability and localization (#821)
//!
//! Every [`Diagnostic`] carries `args` — the values its English `message`
//! interpolates, as named strings — and `fixes`, structural edits (pointer-
//! addressed, never text-addressed: this module never sees source text)
//! believed to resolve it. `message` itself is always English and never
//! localized here; a caller that wants the diagnostic in another language
//! (the `/ui/sql/view-definitions/lint` handler, for one) renders its own
//! catalog from `code` + `args` instead of using `message` at all. See
//! [`Fix`] and the `args` doc on [`Diagnostic`] for the exact contract.
//!
//! [`node_keys`] exposes the same key model these checks are built on, so a
//! consumer that wants "what keys are valid here" (a completion endpoint,
//! for instance) doesn't have to duplicate it.
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
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Public types (RF1)
// ---------------------------------------------------------------------------

/// A location inside the **string value** a [`Diagnostic`] points at,
/// expressed in Unicode `char` offsets — never UTF-8 bytes — so a browser
/// counting Unicode code points (or anything else that is not counting raw
/// bytes) can index into the string directly. Only ever set for
/// [`DiagnosticCode::FhirPathSyntax`] and [`DiagnosticCode::UndeclaredConstant`]
/// — every other diagnostic already locates itself precisely enough with
/// `pointer` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// How serious a [`Diagnostic`] is. Nothing in this POC blocks Save — both
/// severities are informational, but `Warning` is reserved for a future
/// check that flags something suspicious rather than something the
/// ViewDefinition spec (or this module's own key model) outright forbids;
/// every check implemented today reports `Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

/// What kind of problem a [`Diagnostic`] reports. `#[non_exhaustive]`: this
/// is a POC rule set (see the module docs for what is deliberately out of
/// scope), and future work is expected to add codes, not just consumers
/// matching on the ones that exist today.
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
    /// A FHIRPath expression parses, but references `%name` for a `name`
    /// that is neither declared in `constant[]` nor a FHIRPath environment
    /// variable the evaluator resolves. See the module docs for the exact
    /// set of names this treats as declared.
    UndeclaredConstant,
}

/// A structural edit [`lint_view_definition`] believes would resolve (or at
/// least meaningfully address) the [`Diagnostic`] it is attached to,
/// expressed purely in terms of an [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)
/// JSON pointer — never a text position. This module never sees the
/// document's source text (a browser's CodeMirror instance does), so it
/// cannot offer a text edit; a pointer is the one location format both sides
/// agree on. `#[non_exhaustive]`: more fix shapes are expected as the lint
/// grows more rules with obvious one-click resolutions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Fix {
    /// Rename the object key at `pointer` to `to`, keeping its value.
    /// `pointer` names the *property*, not its parent object (e.g.
    /// `/select/0/columns`, for a `columns` key that should be `column`).
    RenameKey { pointer: String, to: String },
    /// Remove the object key at `pointer` entirely.
    RemoveKey { pointer: String },
    /// Replace the string value at `pointer` with `value`.
    SetString { pointer: String, value: String },
}

/// One problem [`lint_view_definition`] found, located by
/// [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901) JSON pointer
/// (`""` is the document root; `~0`/`~1` escape `~`/`/` inside a key).
///
/// `message` is always English and never localized — `helios_sof` has no
/// locale of its own, and `$sql-run`, `sof-cli`, and `pysof` all surface it
/// verbatim. `args` carries the same information `message` interpolates, as
/// named strings a caller (the `/ui/sql/view-definitions/lint` handler, in
/// particular) can hand to its own catalog to render the message in the
/// user's language instead; it is `{}` when `message` has nothing to
/// interpolate. `fixes` are structural edits `lint_view_definition` believes
/// address this diagnostic — see [`Fix`] — and is `[]` when it has none to
/// offer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub pointer: String,
    pub message: String,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub span: Option<Span>,
    pub args: BTreeMap<String, String>,
    pub fixes: Vec<Fix>,
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
    /// favor of the struct over more general prose elsewhere.
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

// ---------------------------------------------------------------------------
// Key-model introspection (#821): what a completion endpoint needs to know
// ---------------------------------------------------------------------------

/// The FHIRPath function catalog and the FHIRPath environment variables,
/// re-exported for a completion endpoint's `function`/`variable` candidates.
///
/// `helios_fhirpath` is already this crate's own dependency (used above to
/// parse every expression this module lints); re-exporting these four items
/// here — rather than a caller like `helios-ui` taking a direct
/// `helios-fhirpath` dependency of its own just to read a static catalog —
/// keeps that catalog reachable through the one edge `helios-ui` already has
/// to this crate, the same way [`node_keys`] exposes the key model instead of
/// a caller duplicating it.
pub use helios_fhirpath::{
    FunctionCategory, FunctionInfo, builtin_functions, environment_variables,
};

/// The JSON shape [`node_keys`] reports for one key — [`Kind`] without the
/// nested [`Node`] a caller outside this module has no use for (and no way
/// to name, since `Node` itself is private).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    String,
    Number,
    Boolean,
    /// An array of strings (`repeat`).
    StringArray,
    /// A nested object, itself with its own keys.
    Object,
    /// An array of objects, each with its own keys.
    ObjectArray,
    /// Accepted, but not modeled further — see [`Kind::Any`].
    Other,
}

impl From<Kind> for KeyKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::String => KeyKind::String,
            Kind::Number => KeyKind::Number,
            Kind::Boolean => KeyKind::Boolean,
            Kind::StringArray => KeyKind::StringArray,
            Kind::Object(_) => KeyKind::Object,
            Kind::ObjectArray(_) => KeyKind::ObjectArray,
            Kind::Any => KeyKind::Other,
        }
    }
}

/// One key [`node_keys`] reports as valid at a node, in the order
/// [`Node::fields`] declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInfo {
    pub key: &'static str,
    pub required: bool,
    pub kind: KeyKind,
}

/// The keys this module's key model allows at the node `pointer` identifies,
/// in [`Node::fields`]'s own declaration order — the same model
/// [`lint_view_definition`]'s `unknown-key`/`missing-required`/`wrong-type`
/// checks are built on, exposed so a caller (a completion endpoint, in
/// particular) can answer "what keys are valid here" without duplicating it.
///
/// The node is resolved from `pointer` **alone** — no document is walked:
/// `""` is the document root; each `/`-separated segment that names an
/// object key steps into that key's own nested node (an
/// `Object`/`ObjectArray` field); a segment made entirely of ASCII digits is
/// skipped instead, since it names an array index, not a key — so
/// `/select/0/column/1` (one column object) and `/select/0/column` (the
/// array containing it) resolve to the same node. Returns `None` once a
/// segment names a key this module's model doesn't recognize, or one whose
/// value is a scalar or [`Kind::Any`] — neither has keys of its own to
/// report.
pub fn node_keys(pointer: &str) -> Option<Vec<KeyInfo>> {
    let mut node = Node::Root;
    for raw_segment in pointer.split('/').skip(1) {
        let segment = unescape_pointer_segment(raw_segment);
        if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let field = node.fields().iter().find(|f| f.key == segment.as_ref())?;
        node = match field.kind {
            Kind::Object(inner) | Kind::ObjectArray(inner) => inner,
            Kind::String | Kind::Number | Kind::Boolean | Kind::StringArray | Kind::Any => {
                return None;
            }
        };
    }
    Some(
        node.fields()
            .iter()
            .map(|f| KeyInfo {
                key: f.key,
                required: f.required,
                kind: f.kind.into(),
            })
            .collect(),
    )
}

/// `%name` references treated as declared for
/// [`DiagnosticCode::UndeclaredConstant`] beyond `constant[].name` and
/// [`helios_fhirpath::environment_variables`] — SQL-on-FHIR's own
/// environment variable(s), bound by `helios_sof` itself
/// (`extract_view_definition_constants` in `crates/sof/src/lib.rs`) rather
/// than resolved by `helios_fhirpath`'s evaluator, so `helios_fhirpath`
/// has no way to know about them.
///
/// - `rowIndex`: the 0-based position of the current element during
///   `forEach`/`forEachOrNull`/`repeat` iteration (0 outside one) — used by
///   several of this crate's own vendored SQL-on-FHIR conformance fixtures
///   (`tests/sql-on-fhir/tests/row_index.json`), which
///   `official_sql_on_fhir_fixtures_that_are_not_error_cases_lint_clean`
///   requires to keep linting clean of errors.
const SQL_ON_FHIR_ENVIRONMENT_VARIABLES: &[&str] = &["rowIndex"];

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
        diagnostics.push(not_a_view_definition(
            "a ViewDefinition document must be a JSON object".to_string(),
            "a non-object document".to_string(),
        ));
        return diagnostics;
    };

    let resource_type = root.get("resourceType").and_then(Value::as_str);
    if resource_type != Some("ViewDefinition") {
        let found = match root.get("resourceType") {
            None => "no `resourceType`".to_string(),
            Some(Value::String(s)) => format!("resourceType {s:?}"),
            Some(other) => format!("a non-string resourceType ({})", type_name(other)),
        };
        diagnostics.push(not_a_view_definition(
            format!("expected resourceType \"ViewDefinition\", found {found}"),
            found,
        ));
        return diagnostics;
    }

    validate_node(Node::Root, doc, "", &mut diagnostics);

    if let Some(constants) = root.get("constant").and_then(Value::as_array) {
        for (i, constant) in constants.iter().enumerate() {
            check_constant_value(constant, &format!("/constant/{i}"), &mut diagnostics);
        }
    }

    // Declared once for the whole document: every `constant[].name` whose
    // value is a string — a `name` of the wrong JSON type is already
    // reported by `validate_node`/`WrongType` and never makes something
    // "declared" here. Borrows straight from `doc`, so it stays valid for
    // every `check_expression` call below.
    let declared_constants: HashSet<&str> = root
        .get("constant")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();

    if let Some(selects) = root.get("select").and_then(Value::as_array) {
        // Every `column[].name` anywhere in the document, gathered up front
        // (#821 validation): a `duplicate-column-name` fix must never
        // suggest a name that collides with *any* column in the document,
        // not only the ones `check_duplicate_columns`' row-scoped walk has
        // already passed by the time it hits the duplicate — a later
        // sibling column, or one in a different select entirely, is just as
        // real a collision. `check_duplicate_columns` grows this same set
        // with every name it suggests, so two duplicates in one pass never
        // suggest each other's name either.
        let mut used_names: HashSet<String> = HashSet::new();
        for select in selects {
            collect_column_names(select, &mut used_names);
        }

        let mut column_scope = Vec::new();
        for (i, select) in selects.iter().enumerate() {
            let pointer = format!("/select/{i}");
            check_select_shape(select, &pointer, &mut diagnostics);
            column_scope = check_duplicate_columns(
                select,
                &pointer,
                column_scope,
                &mut used_names,
                &mut diagnostics,
            );
            check_fhirpath_in_select(select, &pointer, &declared_constants, &mut diagnostics);
        }
    }

    if let Some(wheres) = root.get("where").and_then(Value::as_array) {
        for (i, w) in wheres.iter().enumerate() {
            if let Some(path) = w.get("path").and_then(Value::as_str) {
                check_expression(
                    path,
                    &format!("/where/{i}/path"),
                    &declared_constants,
                    &mut diagnostics,
                );
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

// ---------------------------------------------------------------------------
// Pointer → FHIRPath expression (#821)
// ---------------------------------------------------------------------------

/// Renders an RFC 6901 JSON pointer as a dotted FHIRPath-style expression
/// rooted at `ViewDefinition`, e.g. `/select/0/column/1/path` becomes
/// `ViewDefinition.select[0].column[1].path`, and the document root (`""`)
/// becomes plain `ViewDefinition`.
///
/// This is the shape `$sql-run`'s `422` response puts in
/// `OperationOutcome.issue.expression` (`crate::error::ServerError`) so a
/// client can jump straight to the offending node without knowing JSON
/// Pointer syntax — and it's `pub` so the `/ui/sql/view-definitions/lint`
/// handler can reuse it for the same purpose in the editor.
///
/// Each `/`-separated pointer segment either names an array index (all
/// ASCII digits — a ViewDefinition document never has an object key that is
/// itself numeric, so this can't misfire) and becomes a `[N]` suffix on the
/// segment before it, or names an object key and becomes a `.key` suffix,
/// after undoing RFC 6901's own escaping (`~1` → `/`, then `~0` → `~`, in
/// that order — reversing the encoding, which escapes `~` before `/`).
pub fn pointer_to_fhirpath(pointer: &str) -> String {
    let mut expression = String::from("ViewDefinition");
    if pointer.is_empty() {
        return expression;
    }
    for segment in pointer.split('/').skip(1) {
        let key = segment.replace("~1", "/").replace("~0", "~");
        if key.as_bytes().iter().all(u8::is_ascii_digit) && !key.is_empty() {
            expression.push('[');
            expression.push_str(&key);
            expression.push(']');
        } else {
            expression.push('.');
            expression.push_str(&key);
        }
    }
    expression
}

// ---------------------------------------------------------------------------
// OperationOutcome shape (#821)
// ---------------------------------------------------------------------------

/// Stable `coding.system` for the `helios_sof::lint` diagnostic each
/// `OperationOutcome.issue.details.coding[0]` carries in
/// [`lint_operation_outcome`]. `coding.code` is the diagnostic's own
/// [`DiagnosticCode`] in the wire form [`lint_view_definition`] already
/// serializes it in — see [`diagnostic_coding_code`].
pub const LINT_DIAGNOSTIC_CODING_SYSTEM: &str =
    "http://heliossoftware.com/fhir/CodeSystem/view-definition-lint";

/// Builds a FHIR `OperationOutcome` from lint diagnostics: one `issue` per
/// **error**-severity diagnostic in `diagnostics` (any warning is silently
/// dropped — this renders the shape a caller uses to *reject* a request,
/// not to surface every diagnostic the lint found).
///
/// This is the single source of truth for turning `lint_view_definition`'s
/// output into an HTTP-facing `422` body, shared by every server that lints
/// an inline ViewDefinition before typed-parsing it: `sof-server`'s own
/// `$sql-run` handler (`crate::error::ServerError::InvalidViewDefinition`)
/// and HFS's `$sql-run` handler (`crates/rest/src/handlers/sof`).
///
/// # Example
///
/// ```
/// use helios_sof::lint::{lint_view_definition, lint_operation_outcome};
/// use serde_json::json;
///
/// let doc = json!({ "resourceType": "Patient" });
/// let outcome = lint_operation_outcome(&lint_view_definition(&doc));
/// assert_eq!(outcome["resourceType"], "OperationOutcome");
/// assert_eq!(outcome["issue"][0]["code"], "structure");
/// ```
pub fn lint_operation_outcome(diagnostics: &[Diagnostic]) -> Value {
    let issues: Vec<Value> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(diagnostic_issue)
        .collect();
    serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": issues,
    })
}

/// One `OperationOutcome.issue` for a single lint [`Diagnostic`], regardless
/// of its own severity — callers that only want error-severity issues
/// filter before mapping (see [`lint_operation_outcome`]).
fn diagnostic_issue(diagnostic: &Diagnostic) -> Value {
    serde_json::json!({
        "severity": diagnostic.severity,
        "code": issue_code(diagnostic.code),
        "diagnostics": diagnostic.message,
        "details": {
            "text": diagnostic.message,
            "coding": [{
                "system": LINT_DIAGNOSTIC_CODING_SYSTEM,
                "code": diagnostic_coding_code(diagnostic.code),
            }],
        },
        "expression": [pointer_to_fhirpath(&diagnostic.pointer)],
    })
}

/// FHIR `OperationOutcome.issue.code` for a lint diagnostic — the fixed
/// mapping: `structure` for shape/schema violations the document itself
/// gets wrong, `required` for a required key that's missing or empty,
/// `invalid` for everything else (semantic rules and the FHIRPath-syntax
/// check, neither of which is a schema violation).
///
/// Matched without a wildcard arm on purpose: adding a `DiagnosticCode`
/// variant to this module must fail this build until its issue code is
/// decided here too.
fn issue_code(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::NotAViewDefinition
        | DiagnosticCode::UnknownKey
        | DiagnosticCode::WrongType => "structure",
        DiagnosticCode::MissingRequired | DiagnosticCode::EmptyRequired => "required",
        DiagnosticCode::DuplicateColumnName
        | DiagnosticCode::MultipleIterationDirectives
        | DiagnosticCode::SelectWithoutOutput
        | DiagnosticCode::FhirPathSyntax
        | DiagnosticCode::UndeclaredConstant => "invalid",
    }
}

/// The kebab-case wire string [`DiagnosticCode`] already serializes as
/// (`fhirpath-syntax` for the FHIRPath-parser check; every other variant is
/// its own name) — read back through `serde_json` rather than
/// hand-duplicating the mapping, so `details.coding[0].code` can never drift
/// from what `lint_view_definition`'s own JSON output uses for the same
/// diagnostic.
fn diagnostic_coding_code(code: DiagnosticCode) -> String {
    match serde_json::to_value(code) {
        Ok(Value::String(code)) => code,
        _ => unreachable!("DiagnosticCode serializes to a JSON string"),
    }
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
// NotAViewDefinition / UnknownKey / MissingRequired / WrongType / EmptyRequired
// ---------------------------------------------------------------------------

/// The sole `not-a-view-definition` diagnostic [`lint_view_definition`]
/// returns when it fires — `found` names what the document actually was
/// (`"a non-object document"`, `` `resourceType "Patient"` ``, or a
/// non-string `resourceType`'s own JSON type), matching the wording already
/// folded into `message`.
fn not_a_view_definition(message: String, found: String) -> Diagnostic {
    let mut args = BTreeMap::new();
    args.insert("found".to_string(), found);
    Diagnostic {
        pointer: String::new(),
        message,
        severity: Severity::Error,
        code: DiagnosticCode::NotAViewDefinition,
        span: None,
        args,
        fixes: Vec::new(),
    }
}

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
            None => out.push(unknown_key(pointer, key, fields, obj)),
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
/// the value's own type is correct. A `WrongType` at this node stops here —
/// nothing inside a value of the wrong shape is inspected.
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
        out.push(empty_required(&pointer, f.key));
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
    let name = obj.get("name").and_then(Value::as_str);
    let present: Vec<&str> = CONSTANT_VALUE_KEYS
        .iter()
        .filter(|k| obj.contains_key(**k))
        .copied()
        .collect();
    match present.len() {
        0 => out.push(constant_value_diagnostic(
            pointer,
            DiagnosticCode::MissingRequired,
            "missing required key `value[x]`".to_string(),
            "missing",
            name,
        )),
        1 => {}
        _ => out.push(constant_value_diagnostic(
            pointer,
            DiagnosticCode::WrongType,
            format!(
                "a constant may set only one value[x] key, found {}: {}",
                present.len(),
                present.join(", ")
            ),
            "multiple",
            name,
        )),
    }
}

/// The diagnostic [`check_constant_value`] reports for a `constant`'s
/// `value[x]` choice, whichever of the two ways it went wrong: no `value[x]`
/// key present at all (`code: MissingRequired`, `variant: "missing"`) or
/// more than one present (`code: WrongType`, `variant: "multiple"`) —
/// carrying `args.variant` so a translated message can select the right
/// wording for either, and `args.name` when the constant itself names one,
/// so that wording can name it too. This is a different `args` shape than
/// the generic `missing-required`/`wrong-type` diagnostics
/// `missing_required`/`wrong_type` below build (`key` / `expected`+`found`)
/// — the value[x] choice is a cross-key rule, not "this one key has the
/// wrong shape", so `key`/`expected`/`found` would not describe it
/// accurately.
fn constant_value_diagnostic(
    pointer: &str,
    code: DiagnosticCode,
    message: String,
    variant: &'static str,
    name: Option<&str>,
) -> Diagnostic {
    let mut args = BTreeMap::new();
    args.insert("variant".to_string(), variant.to_string());
    if let Some(name) = name {
        args.insert("name".to_string(), name.to_string());
    }
    Diagnostic {
        pointer: pointer.to_string(),
        message,
        severity: Severity::Error,
        code,
        span: None,
        args,
        fixes: Vec::new(),
    }
}

/// Damerau-Levenshtein edit distance (optimal string alignment — a
/// transposition counts as one edit, but a substring is never transposed
/// more than once) between `a` and `b`, over `char`s. Used only for
/// `unknown-key` typo suggestions ([`suggest_key`]); no crate dependency
/// carries this, so it is implemented directly rather than adding one for a
/// handful of short-string comparisons.
fn damerau_levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (rows, cols) = (a.len() + 1, b.len() + 1);
    let mut d = vec![vec![0usize; cols]; rows];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..rows {
        for j in 1..cols {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[rows - 1][cols - 1]
}

/// Whether lowercase `a` and `b` are identical except that one has exactly
/// one trailing `s` the other doesn't (`column`/`columns`) — a suggestion
/// rule of its own because a short enough pair (e.g. a 2-character key) can
/// exceed the edit-distance threshold [`suggest_key`] otherwise applies
/// while still being an obvious singular/plural typo.
fn is_singular_plural_variant(a: &str, b: &str) -> bool {
    a.strip_suffix('s') == Some(b) || b.strip_suffix('s') == Some(a)
}

/// The best `unknown-key` typo suggestion for `key` among `fields`, or
/// `None` if nothing qualifies: a Damerau-Levenshtein distance of at most 2
/// (case-insensitive) or a singular/plural match
/// ([`is_singular_plural_variant`]), ties broken by the lower distance and
/// then by the field's position in `Node::fields` (declaration order). A key
/// already present on `existing` is never suggested — renaming to it would
/// just create a second problem.
fn suggest_key<'a>(
    key: &str,
    fields: &'a [Field],
    existing: &serde_json::Map<String, Value>,
) -> Option<&'a str> {
    let key_lower = key.to_ascii_lowercase();
    fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !existing.contains_key(f.key))
        .filter_map(|(index, f)| {
            let candidate_lower = f.key.to_ascii_lowercase();
            let distance = damerau_levenshtein_distance(&key_lower, &candidate_lower);
            let qualifies =
                distance <= 2 || is_singular_plural_variant(&key_lower, &candidate_lower);
            qualifies.then_some((distance, index, f.key))
        })
        .min_by_key(|&(distance, index, _)| (distance, index))
        .map(|(_, _, key)| key)
}

/// `key` is a key `fields` doesn't model, found on the object at `pointer`
/// (`present`, so a suggestion never proposes a key already there). Offers a
/// [`Fix::RenameKey`] to the best typo suggestion ([`suggest_key`]), if any,
/// followed always by a [`Fix::RemoveKey`] — dropping the unrecognized key
/// is always a valid way to resolve this, suggestion or not.
fn unknown_key(
    pointer: &str,
    key: &str,
    fields: &[Field],
    present: &serde_json::Map<String, Value>,
) -> Diagnostic {
    let key_pointer = child_pointer(pointer, key);
    let mut args = BTreeMap::new();
    args.insert("key".to_string(), key.to_string());
    let mut fixes = Vec::new();
    if let Some(suggestion) = suggest_key(key, fields, present) {
        args.insert("suggestion".to_string(), suggestion.to_string());
        fixes.push(Fix::RenameKey {
            pointer: key_pointer.clone(),
            to: suggestion.to_string(),
        });
    }
    fixes.push(Fix::RemoveKey {
        pointer: key_pointer.clone(),
    });
    Diagnostic {
        pointer: key_pointer,
        message: format!("unknown key `{key}`"),
        severity: Severity::Error,
        code: DiagnosticCode::UnknownKey,
        span: None,
        args,
        fixes,
    }
}

fn missing_required(pointer: &str, key: &str) -> Diagnostic {
    let mut args = BTreeMap::new();
    args.insert("key".to_string(), key.to_string());
    Diagnostic {
        pointer: pointer.to_string(),
        message: format!("missing required key `{key}`"),
        severity: Severity::Error,
        code: DiagnosticCode::MissingRequired,
        span: None,
        args,
        fixes: Vec::new(),
    }
}

fn empty_required(pointer: &str, key: &str) -> Diagnostic {
    let mut args = BTreeMap::new();
    args.insert("key".to_string(), key.to_string());
    Diagnostic {
        pointer: pointer.to_string(),
        message: "required value must not be empty".to_string(),
        severity: Severity::Error,
        code: DiagnosticCode::EmptyRequired,
        span: None,
        args,
        fixes: Vec::new(),
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
    let found = type_name(found);
    let mut args = BTreeMap::new();
    args.insert("expected".to_string(), expected.to_string());
    args.insert("found".to_string(), found.to_string());
    Diagnostic {
        pointer: pointer.to_string(),
        message: format!("expected {expected}, found {found}"),
        severity: Severity::Error,
        code: DiagnosticCode::WrongType,
        span: None,
        args,
        fixes: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// SelectWithoutOutput / MultipleIterationDirectives
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
            args: BTreeMap::new(),
            fixes: Vec::new(),
        });
    }

    // Presence-based, like the sql-expressions invariant this mirrors
    // (`validate_select_with_context` above): an empty `repeat: []` still
    // counts as "set", matching `forEach.exists()` in the FHIRPath
    // invariant, which is true for an empty collection too.
    const ITERATION_DIRECTIVES: [&str; 3] = ["forEach", "forEachOrNull", "repeat"];
    let directive_count = ITERATION_DIRECTIVES
        .iter()
        .filter(|k| obj.contains_key(**k))
        .count();
    if directive_count > 1 {
        out.push(multiple_iteration_directives(
            pointer,
            obj,
            &ITERATION_DIRECTIVES,
        ));
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

/// The `multiple-iteration-directives` diagnostic for the `select` at
/// `pointer`: `args.keys` lists the directives `obj` actually sets, in
/// `directives`' order (`forEach`, `forEachOrNull`, `repeat`), joined with
/// `, `. `fixes` offers removing every one of them after the first, in that
/// same order, so applying all of a select's fixes in sequence leaves
/// exactly one directive behind — whichever the document declared first.
fn multiple_iteration_directives(
    pointer: &str,
    obj: &serde_json::Map<String, Value>,
    directives: &[&str],
) -> Diagnostic {
    let present: Vec<&str> = directives
        .iter()
        .copied()
        .filter(|key| obj.contains_key(*key))
        .collect();
    let mut args = BTreeMap::new();
    args.insert("keys".to_string(), present.join(", "));
    let fixes = present
        .iter()
        .skip(1)
        .map(|key| Fix::RemoveKey {
            pointer: child_pointer(pointer, key),
        })
        .collect();
    Diagnostic {
        pointer: pointer.to_string(),
        message: "a select may set at most one of forEach, forEachOrNull, repeat".to_string(),
        severity: Severity::Error,
        code: DiagnosticCode::MultipleIterationDirectives,
        span: None,
        args,
        fixes,
    }
}

// ---------------------------------------------------------------------------
// DuplicateColumnName
// ---------------------------------------------------------------------------

/// Every `column[].name` reachable from `select` — through nested `select[]`
/// and `unionAll[]` alike, regardless of row scoping — added to `out`.
/// [`lint_view_definition`] walks the whole document's `select[]` array with
/// this before checking anything, to seed [`check_duplicate_columns`]'
/// `used_names` with every column name that exists anywhere, not only the
/// ones a row-scoped walk happens to have already passed.
fn collect_column_names(select: &Value, out: &mut HashSet<String>) {
    let Some(obj) = select.as_object() else {
        return;
    };
    if let Some(columns) = obj.get("column").and_then(Value::as_array) {
        for column in columns {
            if let Some(name) = column.get("name").and_then(Value::as_str) {
                out.insert(name.to_string());
            }
        }
    }
    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for child in nested {
            collect_column_names(child, out);
        }
    }
    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for branch in branches {
            collect_column_names(branch, out);
        }
    }
}

/// The `duplicate-column-name` diagnostic for one repeated column `name` at
/// `name_pointer`. The one fix offered renames the duplicate to `name_2` —
/// or `name_3`, `name_4`, ... — whichever suffix is the first not already in
/// `used_names`: every column name in the *whole document* (seeded by
/// [`collect_column_names`], not just the ones a row-scoped walk has already
/// passed — a later sibling column, or one in an entirely different select,
/// is just as real a collision), plus every name a fix has already
/// suggested this pass. The chosen suffix is inserted back into
/// `used_names` before returning, so the next duplicate in the same pass
/// never suggests it either.
fn duplicate_column_name(
    name_pointer: &str,
    name: &str,
    used_names: &mut HashSet<String>,
) -> Diagnostic {
    let mut suffix = 2;
    let mut candidate = format!("{name}_{suffix}");
    while used_names.contains(&candidate) {
        suffix += 1;
        candidate = format!("{name}_{suffix}");
    }
    used_names.insert(candidate.clone());
    let mut args = BTreeMap::new();
    args.insert("name".to_string(), name.to_string());
    Diagnostic {
        pointer: name_pointer.to_string(),
        message: format!("duplicate column name `{name}`"),
        severity: Severity::Error,
        code: DiagnosticCode::DuplicateColumnName,
        span: None,
        args,
        fixes: vec![Fix::SetString {
            pointer: name_pointer.to_string(),
            value: candidate,
        }],
    }
}

/// Walks one `select` (and everything reachable through nested `select[]`)
/// checking for column names already seen in `scope` — every
/// `(name, pointer)` from the *same output row*, threaded in by the caller.
/// `used_names` is a separate, document-wide set (see
/// [`collect_column_names`]) threaded through purely so
/// [`duplicate_column_name`] can pick a fix value that collides with
/// nothing in the document — it plays no part in *detecting* a duplicate,
/// only in naming its fix.
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
    used_names: &mut HashSet<String>,
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
                out.push(duplicate_column_name(&name_pointer, name, used_names));
            }
            scope.push((name.to_string(), name_pointer));
        }
    }

    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for (i, child) in nested.iter().enumerate() {
            scope = check_duplicate_columns(
                child,
                &format!("{pointer}/select/{i}"),
                scope,
                used_names,
                out,
            );
        }
    }

    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for (i, branch) in branches.iter().enumerate() {
            check_duplicate_columns(
                branch,
                &format!("{pointer}/unionAll/{i}"),
                scope.clone(),
                used_names,
                out,
            );
        }
    }

    scope
}

// ---------------------------------------------------------------------------
// FhirPathSyntax and UndeclaredConstant
// ---------------------------------------------------------------------------

fn check_fhirpath_in_select(
    select: &Value,
    pointer: &str,
    declared_constants: &HashSet<&str>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(obj) = select.as_object() else {
        return;
    };

    if let Some(columns) = obj.get("column").and_then(Value::as_array) {
        for (i, column) in columns.iter().enumerate() {
            if let Some(path) = column.get("path").and_then(Value::as_str) {
                check_expression(
                    path,
                    &format!("{pointer}/column/{i}/path"),
                    declared_constants,
                    out,
                );
            }
        }
    }
    for key in ["forEach", "forEachOrNull"] {
        if let Some(expr) = obj.get(key).and_then(Value::as_str) {
            check_expression(expr, &format!("{pointer}/{key}"), declared_constants, out);
        }
    }
    if let Some(items) = obj.get("repeat").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            if let Some(expr) = item.as_str() {
                check_expression(
                    expr,
                    &format!("{pointer}/repeat/{i}"),
                    declared_constants,
                    out,
                );
            }
        }
    }
    if let Some(nested) = obj.get("select").and_then(Value::as_array) {
        for (i, child) in nested.iter().enumerate() {
            check_fhirpath_in_select(
                child,
                &format!("{pointer}/select/{i}"),
                declared_constants,
                out,
            );
        }
    }
    if let Some(branches) = obj.get("unionAll").and_then(Value::as_array) {
        for (i, branch) in branches.iter().enumerate() {
            check_fhirpath_in_select(
                branch,
                &format!("{pointer}/unionAll/{i}"),
                declared_constants,
                out,
            );
        }
    }
}

/// Parses one FHIRPath expression string, and:
/// - on a syntax error, pushes a single [`DiagnosticCode::FhirPathSyntax`]
///   diagnostic — the first error `helios_fhirpath::parse_expression_spanned`
///   reports, which is the one closest to where the parser actually gave up;
/// - on success, pushes one [`DiagnosticCode::UndeclaredConstant`] per
///   `%name` reference the expression contains that isn't in
///   `declared_constants` or a FHIRPath environment variable. A non-parsing
///   expression is never checked for undeclared constants — `FhirPathSyntax`
///   alone already reports it.
fn check_expression(
    expression: &str,
    pointer: &str,
    declared_constants: &HashSet<&str>,
    out: &mut Vec<Diagnostic>,
) {
    if expression.trim().is_empty() {
        let message = "empty expression".to_string();
        let mut args = BTreeMap::new();
        args.insert("detail".to_string(), message.clone());
        out.push(Diagnostic {
            pointer: pointer.to_string(),
            message,
            severity: Severity::Error,
            code: DiagnosticCode::FhirPathSyntax,
            span: Some(Span { start: 0, end: 0 }),
            args,
            fixes: Vec::new(),
        });
        return;
    }
    match helios_fhirpath::parse_expression_spanned(expression) {
        Err(errors) => {
            if let Some(first) = errors.into_iter().next() {
                let mut args = BTreeMap::new();
                args.insert("detail".to_string(), first.message.clone());
                out.push(Diagnostic {
                    pointer: pointer.to_string(),
                    message: first.message,
                    severity: Severity::Error,
                    code: DiagnosticCode::FhirPathSyntax,
                    span: Some(Span {
                        start: first.span.0,
                        end: first.span.1,
                    }),
                    args,
                    fixes: Vec::new(),
                });
            }
        }
        Ok(parsed) => {
            for constant_ref in helios_fhirpath::external_constants(&parsed, expression) {
                if declared_constants.contains(constant_ref.name.as_str())
                    || helios_fhirpath::is_environment_variable(&constant_ref.name)
                    || SQL_ON_FHIR_ENVIRONMENT_VARIABLES.contains(&constant_ref.name.as_str())
                {
                    continue;
                }
                let (start, end) =
                    helios_fhirpath::expr_span_to_char_offsets(expression, &constant_ref.span);
                let mut args = BTreeMap::new();
                args.insert("name".to_string(), constant_ref.name.clone());
                out.push(Diagnostic {
                    pointer: pointer.to_string(),
                    message: format!("undeclared constant `%{}`", constant_ref.name),
                    severity: Severity::Error,
                    code: DiagnosticCode::UndeclaredConstant,
                    span: Some(Span { start, end }),
                    args,
                    fixes: Vec::new(),
                });
            }
        }
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
