//! View models for the SQL Queries and SQL Views workspaces (#649).
//!
//! Both pages edit `Library` resources — a SQLQuery carries
//! `type.coding` `sql-query`, a SQLView `sql-view` (both under the SQL on
//! FHIR `LibraryTypesCodes` system, current or pre-ballot) — and run them as
//! `$sql-run` subjects. The SQL itself travels base64-encoded in a
//! `content[]` attachment; these helpers decode it for its own editor pane
//! and re-embed it on save, so nobody edits base64 by hand.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

const LIBRARY_TYPES_SYSTEM: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
const LEGACY_LIBRARY_TYPES_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

/// Whether this Library's `type.coding` carries `code` under either published
/// `LibraryTypesCodes` system.
pub(crate) fn has_library_code(library: &Value, code: &str) -> bool {
    library
        .get("type")
        .and_then(|t| t.get("coding"))
        .and_then(Value::as_array)
        .is_some_and(|codings| {
            codings.iter().any(|c| {
                c.get("code").and_then(Value::as_str) == Some(code)
                    && matches!(
                        c.get("system").and_then(Value::as_str),
                        Some(LIBRARY_TYPES_SYSTEM) | Some(LEGACY_LIBRARY_TYPES_SYSTEM)
                    )
            })
        })
}

/// One rail entry: a stored Library of the page's kind.
pub(crate) struct LibSummary {
    pub id: String,
    pub name: String,
    pub status: String,
}

/// Summarizes fetched Libraries carrying `code` into rail entries, name-sorted.
pub(crate) fn summarize(resources: &[Value], code: &str) -> Vec<LibSummary> {
    let mut entries: Vec<LibSummary> = resources
        .iter()
        .filter(|l| has_library_code(l, code))
        .filter_map(|l| {
            let id = l.get("id")?.as_str()?.to_string();
            let name = l
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let status = extract_status(l);
            Some(LibSummary { id, name, status })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    entries
}

/// The resource's own `status`, verbatim, empty when absent — the same
/// extraction each rail entry above already applies, and the source of the
/// editor-first title row's own status chip (#839), which shows this text
/// as-is even when it names no FHIR publication-status code Helios
/// recognizes.
pub(crate) fn extract_status(library: &Value) -> String {
    library
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The decoded SQL of the first `application/sql` content attachment, empty
/// when there is none (or its data is not valid base64 UTF-8).
pub(crate) fn extract_sql(library: &Value) -> String {
    library
        .get("content")
        .and_then(Value::as_array)
        .and_then(|atts| {
            atts.iter().find(|a| {
                a.get("contentType")
                    .and_then(Value::as_str)
                    .is_some_and(|ct| ct.starts_with("application/sql"))
            })
        })
        .and_then(|a| a.get("data").and_then(Value::as_str))
        .and_then(|data| BASE64.decode(data).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// Embeds `sql` as the base64 `data` of the Library's first `application/sql`
/// attachment, appending one when none exists. Other attachments are left
/// alone.
pub(crate) fn embed_sql(library: &mut Value, sql: &str) {
    let encoded = Value::String(BASE64.encode(sql));
    let Some(map) = library.as_object_mut() else {
        return;
    };
    let content = map
        .entry("content")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(atts) = content.as_array_mut() else {
        return;
    };
    let existing = atts.iter_mut().find(|a| {
        a.get("contentType")
            .and_then(Value::as_str)
            .is_some_and(|ct| ct.starts_with("application/sql"))
    });
    match existing {
        Some(att) => {
            if let Some(att) = att.as_object_mut() {
                att.insert("data".to_string(), encoded);
            }
        }
        None => atts.push(serde_json::json!({
            "contentType": "application/sql",
            "data": encoded,
        })),
    }
}

/// The starter document's fixed `status` — its own constant so
/// [`starter_library`]'s literal and the "Create New" title row's status
/// chip (#839, [`crate::status_tag_class`]) can never drift apart.
pub(crate) const STARTER_STATUS: &str = "draft";

/// The starter document behind "Create New" for `code`, as a parsed value —
/// the Details panel's own first paint (`crate::render_lib_details_pane`)
/// builds directly off this rather than re-parsing [`starter_library`]'s
/// string. Carries no `content` (#840): the SQL pane owns the
/// `application/sql` attachment — [`STARTER_SQL`] is its own starter text —
/// and [`embed_sql`] adds the attachment on save/run, exactly as it would
/// for any other document that has none yet.
pub(crate) fn starter_library_value(code: &str) -> Value {
    serde_json::json!({
        "resourceType": "Library",
        "name": if code == "sql-view" { "new_sql_view" } else { "new_sql_query" },
        "status": STARTER_STATUS,
        "type": { "coding": [{ "system": LIBRARY_TYPES_SYSTEM, "code": code }] },
        "relatedArtifact": [
            { "type": "depends-on", "resource": "http://example.org/ViewDefinition/change-me", "label": "v" }
        ]
    })
}

/// [`starter_library_value`], pretty-printed — the SQL pane's own starter
/// text (`STARTER_SQL`'s counterpart in the JSON pane) before the Details
/// panel existed to build the guided form from the parsed value directly.
pub(crate) fn starter_library(code: &str) -> String {
    serde_json::to_string_pretty(&starter_library_value(code)).expect("static JSON serializes")
}

/// The starter SQL paired with [`starter_library`]'s `label`.
pub(crate) const STARTER_SQL: &str = "SELECT * FROM v";

/// One declared `Library.parameter[use=in]` entry (#837), read with the same
/// semantics `helios_sof::sqlquery::library::extract_parameters` applies:
/// only `use=in` entries carry SQL on FHIR's binding semantics, and `name`/
/// `type` are both required by the SQLQuery profile. Unlike that function —
/// which fails the whole Library on a malformed entry, appropriate for the
/// engine about to run it — this reader is forgiving: a Library the UI
/// cannot fully describe must still let every other stored subject render.
/// If the server itself rejects a malformed Library at kick-off, the job
/// fails with its own message, exactly as it does today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredParameter {
    /// The bare name, without its `:` SQL placeholder prefix.
    pub name: String,
    /// The FHIR type code from `Library.parameter.type` (`string`,
    /// `integer`, `date`, …).
    pub type_code: String,
    /// The declared `default[X]` value in plain-text form — `defaultString`
    /// verbatim, a number's `to_string()`, `true`/`false` for a boolean —
    /// or `None` when the entry carries no `default[X]` field, or that
    /// field's JSON shape has no plain-text representation. A parameter
    /// with a default is optional everywhere this type is consumed; one
    /// without is required.
    pub default: Option<String>,
}

/// Reads every `use=in` parameter declaration off a `sql-query` Library, in
/// document order. An entry missing `use=in`, `name`, or `type` is skipped
/// (logged at `debug`) rather than surfaced as a page-wide error — see
/// [`DeclaredParameter`]'s own docs for why.
pub(crate) fn parameters(library: &Value) -> Vec<DeclaredParameter> {
    let Some(entries) = library.get("parameter").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|p| p.get("use").and_then(Value::as_str) == Some("in"))
        .filter_map(|p| {
            let name = p.get("name").and_then(Value::as_str);
            let type_code = p.get("type").and_then(Value::as_str);
            let (Some(name), Some(type_code)) = (name, type_code) else {
                tracing::debug!(
                    entry = ?p,
                    "skipping Library.parameter entry missing name or type"
                );
                return None;
            };
            Some(DeclaredParameter {
                name: name.to_string(),
                type_code: type_code.to_string(),
                default: default_text(p),
            })
        })
        .collect()
}

/// The plain-text form of a `default[X]` field on one `parameter` entry, per
/// [`DeclaredParameter::default`]'s own rule. Any key starting with
/// `default` and carrying at least one more character (`defaultString`,
/// `defaultValueInteger`, …) counts, forward-compatible with the same
/// tolerant match `helios_sof::sqlquery::library::read_default` applies.
fn default_text(entry: &Value) -> Option<String> {
    entry.as_object()?.iter().find_map(|(key, value)| {
        let rest = key.strip_prefix("default")?;
        if rest.is_empty() {
            return None;
        }
        match value {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    })
}

/// A `:name` placeholder [`undeclared_placeholder_names`] found in the SQL
/// that no `use=in` entry in `declared` names (#841): a hint, not an error —
/// the Parameters card offers a one-click *Declare* for each one rather than
/// blocking the live run. Exact-case comparison throughout, matching both
/// SQLite's own named-parameter semantics and
/// [`helios_sof::sqlquery::scan::scan_placeholders`]'s own dedup rule.
/// Malformed SQL the scanner cannot tokenize at all (see
/// [`helios_sof::sqlquery::ScanError`]) yields no hints — the caller's own
/// `$sql-run` call (or its parse-failure notice) is what reports that
/// problem instead.
pub(crate) fn undeclared_placeholder_names(
    sql: &str,
    declared: &[DeclaredParameter],
) -> Vec<String> {
    let Ok(scanned) = helios_sof::sqlquery::scan_sql(sql) else {
        return Vec::new();
    };
    let declared_names: std::collections::HashSet<&str> =
        declared.iter().map(|p| p.name.as_str()).collect();
    scanned
        .placeholders
        .into_iter()
        .filter(|p| !declared_names.contains(p.name.as_str()))
        .map(|p| p.name)
        .collect()
}

/// The Parameters card's own signature (#841): every declared parameter's
/// `name:type` pair, in declaration order, then one `?name` per undeclared
/// placeholder hint (in the order [`undeclared_placeholder_names`] found
/// them) — comma-joined, empty when there is neither. Computed identically
/// by the card's own hidden `params_sig` field and every `/run` fragment, so
/// the two sides can compare it to decide whether the card needs to travel
/// back over `hx-swap-oob`: a hint appearing or resolving changes this
/// string (so the card re-renders), while typing a parameter *value* never
/// does (so it never loses focus mid-keystroke, #841/NF2).
pub(crate) fn params_signature(declared: &[DeclaredParameter], hints: &[String]) -> String {
    let mut parts: Vec<String> = declared
        .iter()
        .map(|p| format!("{}:{}", p.name, p.type_code))
        .collect();
    parts.extend(hints.iter().map(|name| format!("?{name}")));
    parts.join(",")
}

/// Whether `name` is a valid SQL on FHIR parameter name: the `sql-name`
/// invariant `^[A-Za-z][A-Za-z0-9_]*$` the SQLQuery profile already
/// requires of `Library.parameter.name` — the same shape a `:name`
/// placeholder [`helios_sof::sqlquery::scan_sql`] finds, so a declared name
/// this accepts is always one SQLite could actually bind against.
pub(crate) fn is_valid_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why [`add_parameter`] refused to add a declaration — one variant per
/// validation rule in #841's own order (name shape, then uniqueness, then a
/// bindable type), each translated by the caller into the matching
/// `lib-params-add-*` catalog message rather than carrying English text
/// itself (this module never localizes, matching [`parameters`]'s own
/// split).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AddParameterError {
    /// `name` does not match [`is_valid_parameter_name`] (including empty).
    InvalidName,
    /// A `use=in` entry named `.0` is already declared.
    DuplicateName(String),
    /// `type_code` is not one of `helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES`.
    UnknownType,
}

/// Appends `{"name": name, "use": "in", "type": type_code}` to `document`'s
/// `parameter[]` (creating the array when the document carries none yet),
/// after validating `name`/`type_code` against #841's own rules — see
/// [`AddParameterError`] for each one and the order they are checked in.
/// `document` must already be a JSON object (every caller has parsed it as
/// one by the time it reaches here, having already confirmed
/// `resourceType: "Library"`); a `document` that somehow is not one is left
/// untouched and reported the same as an invalid name, since there is no
/// declaration this function could sensibly have added to it.
///
/// The rest of `document` is never reordered or reformatted beyond the
/// array append itself — an existing `parameter[]` entry, `use=out` or
/// otherwise, keeps its exact position and shape.
pub(crate) fn add_parameter(
    document: &mut Value,
    name: &str,
    type_code: &str,
) -> Result<(), AddParameterError> {
    if !is_valid_parameter_name(name) {
        return Err(AddParameterError::InvalidName);
    }
    if parameters(document).iter().any(|p| p.name == name) {
        return Err(AddParameterError::DuplicateName(name.to_string()));
    }
    if !helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES.contains(&type_code) {
        return Err(AddParameterError::UnknownType);
    }
    let Some(map) = document.as_object_mut() else {
        return Err(AddParameterError::InvalidName);
    };
    let entry = map
        .entry("parameter")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    // The `or_insert_with`/reset above guarantee an array here.
    if let Some(arr) = entry.as_array_mut() {
        arr.push(serde_json::json!({ "name": name, "use": "in", "type": type_code }));
    }
    Ok(())
}

/// Returns a copy of `library` without any `content[]` attachment whose
/// `contentType` starts with `application/sql` (#840) — the document
/// Details edits, since the SQL attachment lives in its own card. `content`
/// is dropped entirely when stripping it empties the array; a `library`
/// whose `content` is missing or not an array comes back unchanged. Other
/// attachments (CQL, plain text, …) keep their order and content.
///
/// Paired with [`extract_sql`]/[`embed_sql`] at save/run time: for a Library
/// with a single `application/sql` attachment,
/// `embed_sql(strip_sql_attachment(lib), extract_sql(lib))` reconstructs
/// `lib` (see the invariant test below) — stripping and re-embedding is a
/// round trip except that a re-embedded attachment always lands last, which
/// only matters when other attachments preceded it.
///
/// The Details panel's own document, both on the page's first paint
/// (`crate::shape_lib`, `crate::render_lib_details_pane`) and in the
/// `POST /ui/sql/queries`/`/ui/sql/views` Save error re-render.
pub(crate) fn strip_sql_attachment(library: &Value) -> Value {
    let mut out = library.clone();
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    let Some(atts) = map.get("content").and_then(Value::as_array) else {
        return out;
    };
    let kept: Vec<Value> = atts
        .iter()
        .filter(|attachment| {
            !attachment
                .get("contentType")
                .and_then(Value::as_str)
                .is_some_and(|ct| ct.starts_with("application/sql"))
        })
        .cloned()
        .collect();
    if kept.is_empty() {
        map.remove("content");
    } else {
        map.insert("content".to_string(), Value::Array(kept));
    }
    out
}

// ---------------------------------------------------------------------
// Tables panel (#842): a SQL Query/SQL View's declared table dependencies
// (`relatedArtifact[type=depends-on]`, *Reads from*) and the reverse —
// which other Libraries and SQL Export jobs depend on *this* one (*Used
// by*). Resolving a dependency's `resource` string against storage needs
// I/O (`ConformanceSource::read_resource`/`search_page`), so that half lives
// in `crate::lib`'s own async handlers; everything here is the pure part —
// reading the declarations, matching a reference against an
// already-fetched candidate, and the two mutations *Add table*/*Remove*
// apply to the document.
// ---------------------------------------------------------------------

/// One `relatedArtifact[type=depends-on]` entry, read tolerantly (unlike
/// `helios_sof::sqlquery::library::parse_sqlquery_library`, which rejects a
/// whole Library over one malformed entry — appropriate for the engine
/// about to run it, not for a page that must still render every other row):
/// `label`/`resource` missing or not a string reads as `""`, the same
/// empty-means-absent convention [`extract_status`] already uses in this
/// module. An entry whose own `type` is not `"depends-on"` is skipped
/// entirely, never represented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableDependency {
    /// `relatedArtifact.label`, verbatim; `""` when absent — the *Reads
    /// from* row's own cue to show the "(no label)" placeholder instead
    /// ([`crate::table_row_view`]).
    pub label: String,
    /// `relatedArtifact.resource`, verbatim; `""` when absent, which never
    /// resolves to anything ([`dependency_lookup`] finds neither a `Type/id`
    /// prefix nor a non-empty canonical URL in it).
    pub resource: String,
}

/// Reads every `depends-on` entry off `document.relatedArtifact`, in
/// document order — the *Reads from* card's own row order (#842).
/// `document.relatedArtifact` missing or not an array yields no rows, the
/// same "nothing declared yet" the starter document (before *Add table*)
/// and a hand-emptied array both read as.
pub(crate) fn table_dependencies(document: &Value) -> Vec<TableDependency> {
    let Some(entries) = document.get("relatedArtifact").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("depends-on"))
        .map(|entry| TableDependency {
            label: entry
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            resource: entry
                .get("resource")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect()
}

/// Whether `label` already names a declared dependency, ignoring case
/// (#842's own alias-uniqueness rule — `V` collides with `v`).
pub(crate) fn label_declared(document: &Value, label: &str) -> bool {
    table_dependencies(document)
        .iter()
        .any(|dep| dep.label.eq_ignore_ascii_case(label))
}

/// The *Reads from* card's own signature (#842): every declared
/// dependency's `label=resource` pair, in declaration order, comma-joined —
/// empty when there are none. Computed identically by the card's own hidden
/// `tables_sig` field and every `/run` fragment (mirroring
/// [`params_signature`]'s own role for the Parameters card), so the two
/// sides can compare it to decide whether the card needs to travel back
/// over `hx-swap-oob`: editing Details (adding/removing/relabeling a
/// dependency) changes this string, so the card re-resolves; nothing else
/// on the page does. #842/04 extends this with the unknown-table lint's own
/// findings — this function's own contract does not change, only what its
/// caller feeds it.
pub(crate) fn tables_signature(deps: &[TableDependency]) -> String {
    deps.iter()
        .map(|dep| format!("{}={}", dep.label, dep.resource))
        .collect::<Vec<_>>()
        .join(",")
}

/// Every table the SQL reads that no declared dependency's own `label`
/// names, case-insensitively (#842/04's own unknown-table lint), in
/// first-occurrence-in-SQL order — [`helios_sof::sqlquery::undeclared_
/// tables`]'s own findings, owned rather than borrowed so the caller can
/// hold onto them past the scan's own temporary [`helios_sof::sqlquery::
/// ScanResult`]. Malformed SQL the scanner cannot tokenize at all yields no
/// findings, mirroring [`undeclared_placeholder_names`]'s own contract: the
/// caller's own `$sql-run` call (or its own parse-failure notice) is what
/// reports that problem instead — this lint only ever *prevents* a call
/// that would otherwise happen, never replaces one that would fail anyway.
pub(crate) fn unknown_tables(
    sql: &str,
    deps: &[TableDependency],
) -> Vec<helios_sof::sqlquery::TableRef> {
    let Ok(scanned) = helios_sof::sqlquery::scan_sql(sql) else {
        return Vec::new();
    };
    let declared_labels: Vec<String> = deps.iter().map(|d| d.label.clone()).collect();
    helios_sof::sqlquery::undeclared_tables(&scanned, &declared_labels)
        .into_iter()
        .cloned()
        .collect()
}

/// Extends [`tables_signature`]'s own string with one `?name` per unknown
/// table the SQL reads (#842/04), mirroring [`params_signature`]'s
/// identical `?name` hint suffix for undeclared placeholders.
/// [`tables_signature`]'s own contract does not change — every caller that
/// used to compare its bare result now compares this one instead, so an
/// unknown table appearing or resolving changes the signature exactly like
/// a declared dependency being added or removed already does (#842/NF2).
pub(crate) fn tables_signature_with_unknown(
    deps: &[TableDependency],
    unknown_names: &[String],
) -> String {
    let mut signature = tables_signature(deps);
    for name in unknown_names {
        if !signature.is_empty() {
            signature.push(',');
        }
        signature.push('?');
        signature.push_str(name);
    }
    signature
}

/// Splits `reference` into its canonical URL and an optional pinned
/// version — `|version` only. The SQL on FHIR spec narrative's own
/// `@version` form (`crates/rest/.../references.rs` accepts both) is never
/// written by this UI's own [`add_table`] and is out of scope for #842's own
/// "imitates, does not replace" resolution — a document that used it would
/// simply not resolve here, exactly as an unversioned typo would not.
fn split_canonical_version(reference: &str) -> (&str, Option<&str>) {
    match reference.split_once('|') {
        Some((canonical, version)) => (canonical, Some(version)),
        None => (reference, None),
    }
}

/// How a `relatedArtifact.resource` (or an *Add table* combobox/textarea
/// submission, which shares the same two shapes) names its target —
/// [`dependency_lookup`]'s own pure result, before any I/O resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DependencyLookup {
    /// A relative `{resource_type}/{id}` reference, `resource_type` one of
    /// `"ViewDefinition"`/`"Library"` — the two types #842's own resolution
    /// ever reads (mirrors `resolve_resource_canonical_or_relative`'s own
    /// dual-type lookup server-side, minus the types this UI never needs to
    /// try, since a table dependency can only ever be one of these two).
    TypeId {
        resource_type: &'static str,
        id: String,
    },
    /// Anything else: an absolute canonical URL, optionally `|version`.
    Canonical {
        canonical: String,
        version: Option<String>,
    },
}

/// Classifies `reference` the same way `crates/rest/.../graph.rs`'s own
/// `StorageArtifactFetcher::fetch` does (#842's own resolution imitates,
/// never replaces, the server's own): a `ViewDefinition/{id}` or
/// `Library/{id}` prefix names a relative
/// reference (only the id up to the next `/`, matching
/// `resolve_resource_canonical_or_relative`'s own `rest.split('/').next()`);
/// anything else is an absolute canonical URL, optionally `|version`.
pub(crate) fn dependency_lookup(reference: &str) -> DependencyLookup {
    for resource_type in ["ViewDefinition", "Library"] {
        if let Some(rest) = reference
            .strip_prefix(resource_type)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            let id = rest.split('/').next().unwrap_or("");
            if !id.is_empty() {
                return DependencyLookup::TypeId {
                    resource_type,
                    id: id.to_string(),
                };
            }
        }
    }
    let (canonical, version) = split_canonical_version(reference);
    DependencyLookup::Canonical {
        canonical: canonical.to_string(),
        version: version.map(str::to_string),
    }
}

/// Whether `reference` — a `relatedArtifact.resource` string, in either
/// shape [`dependency_lookup`] recognizes — identifies `artifact`, an
/// already-fetched resource assumed to be of `expected_type`. Mirrors (does
/// not call — the two crates do not depend on each other)
/// `crates/rest/.../references.rs`'s `canonical_matches`, extended with the
/// `Type/id` form that function's own caller
/// (`resolve_resource_canonical_or_relative`) checks first: the relative
/// form matches only `artifact`'s own `id`; the canonical form matches
/// `artifact.url`, and — only when `reference` itself pins one — also
/// requires `artifact.version` to equal it. `artifact` missing the field
/// either form needs never matches, rather than panicking or guessing.
///
/// Used in both directions #842 needs: resolving a dependency's own target
/// (`reference` from *this* document, `artifact` a candidate
/// ViewDefinition/Library) and finding *Used by* peers (`reference` from
/// some *other* Library's own dependency list, `artifact` the selected
/// Library itself).
pub(crate) fn matches_reference(reference: &str, expected_type: &str, artifact: &Value) -> bool {
    let prefix = format!("{expected_type}/");
    if let Some(id) = reference.strip_prefix(prefix.as_str()) {
        return artifact.get("id").and_then(Value::as_str) == Some(id);
    }
    let Some(url) = artifact.get("url").and_then(Value::as_str) else {
        return false;
    };
    let (canonical, version) = split_canonical_version(reference);
    if url != canonical {
        return false;
    }
    match version {
        Some(pinned) => artifact.get("version").and_then(Value::as_str) == Some(pinned),
        None => true,
    }
}

/// What a *Reads from* dependency resolved to (#842) — [`TableRow`]'s
/// own target half, built by `crate::classify_table_artifact` once I/O has
/// fetched a candidate (or found none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TableTarget {
    /// Resolved to a stored ViewDefinition.
    ViewDefinition { id: String, name: String },
    /// Resolved to a stored `sql-view` Library.
    SqlView { id: String, name: String },
    /// Neither storage nor the in-memory Library list has anything
    /// answering to the dependency's own `resource` (the starter
    /// document's own `change-me` canonical, for instance).
    NotFound,
    /// Resolved to something storage holds, but not a ViewDefinition or a
    /// `sql-view` Library — a `sql-query` Library, most commonly, which
    /// `$sql-run`'s own graph walk would reject the same way.
    NotATable,
}

/// Classifies an already-fetched `artifact` of `resource_type` into a
/// [`TableTarget`] — the last step of resolving one dependency, once I/O
/// has found (or failed to find) a candidate. `artifact`'s own `name`
/// falls back to its `id` when absent, matching every other rail/list
/// summary in this module ([`summarize`], `sql_views::summarize`).
pub(crate) fn classify_table_artifact(resource_type: &str, artifact: &Value) -> TableTarget {
    let id = artifact
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = artifact
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    match resource_type {
        "ViewDefinition" => TableTarget::ViewDefinition { id, name },
        "Library" if has_library_code(artifact, "sql-view") => TableTarget::SqlView { id, name },
        _ => TableTarget::NotATable,
    }
}

/// One *Reads from* row (#842): the declared alias and raw
/// `resource` string, plus how it resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableRow {
    pub label: String,
    pub resource: String,
    pub target: TableTarget,
}

/// One Library (of either kind) whose own `relatedArtifact[depends-on]`
/// names the selected artifact (#842) — a *Used by* artifact row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryArtifactKind {
    SqlQuery,
    SqlView,
}

/// One *Used by* artifact row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsedByArtifact {
    pub id: String,
    pub name: String,
    pub kind: LibraryArtifactKind,
}

/// Every Library in `libraries` (any kind, as already fetched for the rail)
/// whose own `relatedArtifact[depends-on].resource` names `target`'s own
/// identity — `target`'s `id`, `url`, or `url|version` — sorted by name
/// (#842). `target` itself is never included, matched by `id` when it
/// has one; a `target` with no `id` at all (`?lib=new`) can still be found
/// by `url` if it carries one, and otherwise matches nothing, which
/// is exactly right — nothing has been saved yet for another artifact to
/// depend on.
pub(crate) fn used_by_artifacts(libraries: &[Value], target: &Value) -> Vec<UsedByArtifact> {
    let target_id = target.get("id").and_then(Value::as_str);
    let mut out: Vec<UsedByArtifact> = libraries
        .iter()
        .filter(|lib| target_id.is_none() || lib.get("id").and_then(Value::as_str) != target_id)
        .filter_map(|lib| {
            let kind = if has_library_code(lib, "sql-query") {
                LibraryArtifactKind::SqlQuery
            } else if has_library_code(lib, "sql-view") {
                LibraryArtifactKind::SqlView
            } else {
                // Some other Library entirely — not a SQL on FHIR artifact
                // #842's own Used by card has a chip for.
                return None;
            };
            table_dependencies(lib)
                .iter()
                .any(|dep| matches_reference(&dep.resource, "Library", target))
                .then(|| {
                    let id = lib
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = lib
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    UsedByArtifact { id, name, kind }
                })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

/// One `$sql-export` job whose subjects include the selected Library
/// (#842) — a *Used by* export row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsedByExport {
    pub job_id: String,
    pub label: String,
}

/// Every job in `jobs` (id, record) with some `subjects[].reference` equal
/// to `Library/{library_id}` — exact match, never a canonical lookup, since
/// a job's own subject reference is always written in that shape at
/// kick-off (#842). `jobs` is assumed already sorted most-recently-
/// started first (`crate::sql_export::jobs_for_used_by`'s own contract), a
/// sort this function preserves rather than repeats. `library_id` empty
/// (`?lib=new`, or an unsaved document) yields no rows — a job can only
/// ever reference a saved Library's own id.
pub(crate) fn used_by_exports(
    jobs: &[(String, crate::sql_export::ExportJob)],
    library_id: &str,
) -> Vec<UsedByExport> {
    if library_id.is_empty() {
        return Vec::new();
    }
    let target = format!("Library/{library_id}");
    jobs.iter()
        .filter(|(_, job)| job.subjects.iter().any(|s| s.reference == target))
        .map(|(job_id, job)| UsedByExport {
            job_id: job_id.clone(),
            label: if job.name.is_empty() {
                job_id.clone()
            } else {
                job.name.clone()
            },
        })
        .collect()
}

/// Appends `{"type": "depends-on", "label": label, "resource": resource}` to
/// `document`'s `relatedArtifact[]` (creating the array when the document
/// carries none yet, matching [`add_parameter`]'s own shape) — the last
/// step of *Add table* (#842), after the caller has already resolved
/// and validated `label`/`resource` (this function trusts both; see
/// `crate::apply_add_table` for the checks). The rest of `document` is
/// never reordered or reformatted beyond the array append itself.
pub(crate) fn add_table(document: &mut Value, label: &str, resource: &str) {
    let Some(map) = document.as_object_mut() else {
        return;
    };
    let entry = map
        .entry("relatedArtifact")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    if let Some(arr) = entry.as_array_mut() {
        arr.push(serde_json::json!({
            "type": "depends-on", "label": label, "resource": resource,
        }));
    }
}

/// Removes the first `depends-on` entry of `document.relatedArtifact` whose
/// `label` exactly equals `label` (#842) — case-sensitive, unlike
/// [`label_declared`]'s own uniqueness check: *Remove* targets one row by
/// its own exact rendered alias, never a case-insensitive family of them.
/// A `label` that matches no entry (already removed, or never existed)
/// leaves `document` untouched — not an error, matching the spec's own
/// idempotence rule.
pub(crate) fn remove_table(document: &mut Value, label: &str) {
    let Some(entries) = document
        .get_mut("relatedArtifact")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    if let Some(pos) = entries.iter().position(|entry| {
        entry.get("type").and_then(Value::as_str) == Some("depends-on")
            && entry
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                == label
    }) {
        entries.remove(pos);
    }
}

// ---------------------------------------------------------------------
// Columns card (#842/04): the last good run's own column list — name,
// type, and origin — the *Reads from* resolution's own byproduct once a
// dependency resolves to a `ViewDefinition`. Building this needs no I/O of
// its own: the caller (`crate::resolve_view_definition_dependencies`)
// already resolved every dependency for the Tables panel; this module only
// reasons over the result.
// ---------------------------------------------------------------------

/// One *Columns* row's own answer to "what does this query produce, and
/// from where" (#842/04) — [`analyze_columns`]'s own per-column result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnInfo {
    /// The result column's own name, in the query's own output order.
    pub name: String,
    /// The FHIR type code to display: the resolved dependency's own
    /// declared column type when [`Self::origin`] is `Some`, otherwise
    /// inferred from the row values themselves
    /// ([`infer_column_type_from_rows`]); `None` when neither source has
    /// an answer (the card's own "—" placeholder).
    pub type_code: Option<String>,
    /// `Some((label, column_name))` only when this column's own name
    /// exists in exactly one resolved ViewDefinition dependency's own
    /// schema — `column_name` always equals [`Self::name`], carried
    /// alongside it only so the view layer never has to re-borrow `name`
    /// to render `"{label}.{column}"`. Ambiguous (present in two or more
    /// dependencies) or absent from all of them alike renders `None` — the
    /// card cannot tell "no origin" from "too many" apart, and #842/04
    /// does not try to.
    pub origin: Option<(String, String)>,
}

/// Builds one [`ColumnInfo`] per entry of `columns` (the last good run's
/// own result columns, in order) — the Columns card's own analysis
/// (#842/04). `view_definitions` is every dependency already resolved to a
/// `ViewDefinition` (label, resolved document), from `crate::
/// resolve_view_definition_dependencies` — SQL View dependencies never
/// reach here at all (#842's own "no origin from a SQL View" rule is
/// simply that this list never contains one). `rows` are the raw JSON
/// objects `$sql-run` returned, before [`crate::sql_views::build_table`]
/// stringifies them — needed so a column with no origin can still infer a
/// type from the values' own JSON shape rather than their rendered text.
/// The declared dependencies among `deps` whose own `label` the SQL
/// actually reads as a table, case-insensitively (#842/04) — the Columns
/// card's own origin lookup only ever considers dependencies the *current*
/// query uses, not every dependency the document happens to declare: a
/// Library can keep an old, no-longer-queried label around (most commonly
/// right after *Add table* resolves a table that used to be unknown under
/// a *different* label, which still points at the very same
/// ViewDefinition) — letting a stale declaration collide with a fresh one
/// over the same target would manufacture an "ambiguous" origin
/// ([`analyze_columns`]'s own `None`) that has nothing to do with what
/// this run's own columns actually came from. Malformed SQL the scanner
/// cannot tokenize at all yields no dependencies — the same "skip, don't
/// guess" contract [`unknown_tables`] already keeps.
pub(crate) fn dependencies_used_by_sql(
    sql: &str,
    deps: &[TableDependency],
) -> Vec<TableDependency> {
    let Ok(scanned) = helios_sof::sqlquery::scan_sql(sql) else {
        return Vec::new();
    };
    let used: std::collections::HashSet<String> = scanned
        .tables
        .iter()
        .map(|t| t.name.to_lowercase())
        .collect();
    deps.iter()
        .filter(|d| used.contains(&d.label.to_lowercase()))
        .cloned()
        .collect()
}

pub(crate) fn analyze_columns(
    columns: &[String],
    rows: &[Value],
    view_definitions: &[(String, Value)],
) -> Vec<ColumnInfo> {
    let schemas: Vec<(&str, helios_sof::sqlquery::TableSchema)> = view_definitions
        .iter()
        .map(|(label, vd)| {
            (
                label.as_str(),
                helios_sof::sqlquery::TableSchema::from_view_definition(vd),
            )
        })
        .collect();

    columns
        .iter()
        .map(|name| {
            let mut matches = schemas.iter().filter_map(|(label, schema)| {
                schema
                    .columns
                    .iter()
                    .find(|c| c.name == *name)
                    .map(|c| (*label, c.fhir_type.code()))
            });
            let first = matches.next();
            let single = first.filter(|_| matches.next().is_none());
            match single {
                Some((label, type_code)) => ColumnInfo {
                    name: name.clone(),
                    type_code: Some(type_code.to_string()),
                    origin: Some((label.to_string(), name.clone())),
                },
                None => ColumnInfo {
                    name: name.clone(),
                    type_code: infer_column_type_from_rows(name, rows).map(str::to_string),
                    origin: None,
                },
            }
        })
        .collect()
}

/// Infers `column`'s own display type from the raw JSON values `rows`
/// carry for it (#842/04's own "type without an origin" rule): `boolean`
/// when every non-null value is a JSON boolean, `integer` when every one is
/// a whole JSON number, `decimal` when every one is a JSON number but at
/// least one is not whole, `string` when every one is a JSON string *or*
/// the values mix more than one of these kinds — `None` (the card's own
/// "—") when every row is null or absent for this column, or there are no
/// rows at all. A JSON array/object value (never produced by `$sql-run`'s
/// own scalar columns today) counts as a "mix" the same as a string would,
/// rather than panicking or being silently skipped.
fn infer_column_type_from_rows(column: &str, rows: &[Value]) -> Option<&'static str> {
    let mut saw_bool = false;
    let mut saw_string_or_other = false;
    let mut saw_number = false;
    let mut all_numbers_whole = true;
    let mut any_non_null = false;

    for row in rows {
        match row.get(column) {
            None | Some(Value::Null) => continue,
            Some(Value::Bool(_)) => {
                any_non_null = true;
                saw_bool = true;
            }
            Some(Value::Number(n)) => {
                any_non_null = true;
                saw_number = true;
                if n.as_i64().is_none() && n.as_u64().is_none() {
                    all_numbers_whole = false;
                }
            }
            Some(_) => {
                any_non_null = true;
                saw_string_or_other = true;
            }
        }
    }

    if !any_non_null {
        return None;
    }
    let kinds_seen = [saw_bool, saw_number, saw_string_or_other]
        .into_iter()
        .filter(|seen| *seen)
        .count();
    if kinds_seen > 1 {
        return Some("string");
    }
    if saw_bool {
        return Some("boolean");
    }
    if saw_number {
        return Some(if all_numbers_whole {
            "integer"
        } else {
            "decimal"
        });
    }
    Some("string")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn library(code: &str, system: &str) -> Value {
        json!({
            "resourceType": "Library", "id": "l1", "name": "q",
            "status": "active",
            "type": {"coding": [{"system": system, "code": code}]},
        })
    }

    #[test]
    fn kind_matching_accepts_both_published_systems_and_nothing_else() {
        assert!(has_library_code(
            &library("sql-query", LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
        assert!(has_library_code(
            &library("sql-query", LEGACY_LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
        assert!(!has_library_code(
            &library("sql-query", "http://elsewhere"),
            "sql-query"
        ));
        assert!(!has_library_code(
            &library("sql-view", LIBRARY_TYPES_SYSTEM),
            "sql-query"
        ));
    }

    #[test]
    fn summaries_keep_only_the_requested_kind() {
        let libs = vec![
            library("sql-query", LIBRARY_TYPES_SYSTEM),
            library("sql-view", LIBRARY_TYPES_SYSTEM),
        ];
        assert_eq!(summarize(&libs, "sql-query").len(), 1);
        assert_eq!(summarize(&libs, "sql-view").len(), 1);
    }

    #[test]
    fn sql_roundtrips_through_the_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(extract_sql(&lib), "");
        embed_sql(&mut lib, "SELECT 1");
        assert_eq!(extract_sql(&lib), "SELECT 1");
        // Re-embedding replaces, not appends.
        embed_sql(&mut lib, "SELECT 2");
        assert_eq!(extract_sql(&lib), "SELECT 2");
        assert_eq!(lib["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn starters_parse_and_carry_their_coding() {
        for code in ["sql-query", "sql-view"] {
            let lib: Value = serde_json::from_str(&starter_library(code)).unwrap();
            assert!(has_library_code(&lib, code));
            assert_eq!(extract_sql(&lib), "");
            // #840: no `content` at all — the SQL card owns that branch and
            // embeds it on save/run, so the starter carries nothing for
            // Details (#840's guided form) to show or hide in the meantime.
            assert!(lib.get("content").is_none());
            // #839: the starter's own status never drifts from the constant
            // the title row's status chip reads on `?lib=new`.
            assert_eq!(lib["status"].as_str(), Some(STARTER_STATUS));
        }
    }

    #[test]
    fn strip_sql_attachment_drops_the_key_when_sql_was_the_only_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        embed_sql(&mut lib, "SELECT 1");
        let stripped = strip_sql_attachment(&lib);
        assert!(stripped.get("content").is_none());
        // Nothing else in the document moved.
        assert_eq!(stripped["name"], lib["name"]);
    }

    #[test]
    fn strip_sql_attachment_keeps_other_attachments_in_order() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        lib["content"] = json!([
            { "contentType": "text/cql", "data": "cql-data" },
            { "contentType": "application/sql", "data": "sql-data" },
            { "contentType": "text/plain", "data": "plain-data" },
        ]);
        let stripped = strip_sql_attachment(&lib);
        let content = stripped["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["contentType"], "text/cql");
        assert_eq!(content[1]["contentType"], "text/plain");
    }

    #[test]
    fn strip_sql_attachment_passes_through_a_missing_or_non_array_content() {
        let no_content = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(strip_sql_attachment(&no_content), no_content);

        let mut non_array_content = library("sql-query", LIBRARY_TYPES_SYSTEM);
        non_array_content["content"] = json!("not-an-array");
        assert_eq!(strip_sql_attachment(&non_array_content), non_array_content);
    }

    /// #840's own round-trip invariant: for a Library with a single SQL
    /// attachment, stripping it out and re-embedding the SQL it carried
    /// reconstructs the original document — the attachment only moves when
    /// other attachments already surrounded it (untested here, since there
    /// are none), never when it was alone.
    #[test]
    fn strip_then_embed_reconstructs_a_library_with_only_a_sql_attachment() {
        let mut lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        embed_sql(&mut lib, "SELECT 1 FROM t");

        let mut reconstructed = strip_sql_attachment(&lib);
        embed_sql(&mut reconstructed, &extract_sql(&lib));

        assert_eq!(reconstructed, lib);
    }

    #[test]
    fn status_is_extracted_verbatim_and_empty_when_absent() {
        let lib = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(extract_status(&lib), "active");
        assert_eq!(extract_status(&json!({"resourceType": "Library"})), "");
    }

    // -----------------------------------------------------------------
    // parameters() (#837)
    // -----------------------------------------------------------------

    #[test]
    fn parameters_ignores_out_entries_and_entries_missing_a_type() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "ward", "use": "in", "type": "string"},
                {"name": "result", "use": "out", "type": "string"},
                {"name": "untyped", "use": "in"},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "ward");
        assert_eq!(params[0].type_code, "string");
        assert_eq!(params[0].default, None);
    }

    #[test]
    fn parameters_ignores_entries_missing_a_name() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [{"use": "in", "type": "string"}],
        });
        assert!(parameters(&lib).is_empty());
    }

    #[test]
    fn parameters_reads_a_numeric_default_as_its_text_form() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "days", "use": "in", "type": "integer", "defaultInteger": 30},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params[0].default.as_deref(), Some("30"));
    }

    #[test]
    fn parameters_reads_string_and_boolean_defaults_and_falls_back_to_none_otherwise() {
        let lib = json!({
            "resourceType": "Library",
            "parameter": [
                {"name": "a", "use": "in", "type": "string", "defaultString": "west"},
                {"name": "b", "use": "in", "type": "boolean", "defaultBoolean": true},
                {"name": "c", "use": "in", "type": "string", "defaultCodeableConcept": {"text": "x"}},
            ],
        });
        let params = parameters(&lib);
        assert_eq!(params[0].default.as_deref(), Some("west"));
        assert_eq!(params[1].default.as_deref(), Some("true"));
        assert_eq!(params[2].default, None);
    }

    #[test]
    fn parameters_matches_parse_sqlquery_library_on_a_valid_library() {
        let sql = base64::engine::general_purpose::STANDARD.encode("SELECT * FROM v");
        let lib = json!({
            "resourceType": "Library",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-query"}]},
            "content": [{"contentType": "application/sql", "data": sql}],
            "parameter": [
                {"name": "ward", "use": "in", "type": "string"},
                {"name": "days", "use": "in", "type": "integer", "defaultInteger": 30},
                {"name": "ignored", "use": "out", "type": "string"},
            ],
        });
        let ours: Vec<(String, String, bool)> = parameters(&lib)
            .into_iter()
            .map(|p| (p.name, p.type_code, p.default.is_some()))
            .collect();
        let theirs: Vec<(String, String, bool)> =
            helios_sof::sqlquery::parse_sqlquery_library(&lib)
                .expect("valid Library parses")
                .parameters
                .into_iter()
                .map(|p| (p.name, p.type_code, p.has_default))
                .collect();
        assert_eq!(ours, theirs);
    }

    // -----------------------------------------------------------------
    // undeclared_placeholder_names() / params_signature() (#841)
    // -----------------------------------------------------------------

    fn param(name: &str, type_code: &str) -> DeclaredParameter {
        DeclaredParameter {
            name: name.into(),
            type_code: type_code.into(),
            default: None,
        }
    }

    #[test]
    fn undeclared_placeholders_excludes_declared_names_case_sensitively() {
        let declared = vec![param("ward", "string")];
        let hints =
            undeclared_placeholder_names("SELECT * WHERE a = :ward AND b = :Ward", &declared);
        // `:ward` is declared (exact case); `:Ward` is not — SQLite named
        // parameters are case-sensitive, and so is this comparison.
        assert_eq!(hints, vec!["Ward".to_string()]);
    }

    #[test]
    fn undeclared_placeholders_is_empty_when_every_placeholder_is_declared() {
        let declared = vec![param("ward", "string"), param("days", "integer")];
        let hints =
            undeclared_placeholder_names("SELECT * WHERE a = :ward AND b = :days", &declared);
        assert!(hints.is_empty());
    }

    #[test]
    fn undeclared_placeholders_reports_nothing_for_sql_the_scanner_cannot_tokenize() {
        // An unterminated string literal — `scan_sql` errors, and this
        // helper's contract is to yield no hints rather than propagate that.
        assert!(undeclared_placeholder_names("SELECT 'unterminated", &[]).is_empty());
    }

    #[test]
    fn signature_joins_declared_pairs_then_hints_and_is_empty_with_neither() {
        assert_eq!(params_signature(&[], &[]), "");
        assert_eq!(
            params_signature(&[param("ward", "string"), param("days", "integer")], &[]),
            "ward:string,days:integer"
        );
        assert_eq!(
            params_signature(&[param("ward", "string")], &["extra".to_string()]),
            "ward:string,?extra"
        );
        assert_eq!(params_signature(&[], &["extra".to_string()]), "?extra");
    }

    // -----------------------------------------------------------------
    // is_valid_parameter_name() / add_parameter() (#841)
    // -----------------------------------------------------------------

    #[test]
    fn parameter_name_validation_matches_the_sql_name_invariant() {
        assert!(is_valid_parameter_name("ward"));
        assert!(is_valid_parameter_name("Ward_2"));
        assert!(!is_valid_parameter_name(""));
        assert!(!is_valid_parameter_name("2ward"));
        assert!(!is_valid_parameter_name("ward-2"));
        assert!(!is_valid_parameter_name("ward name"));
        assert!(!is_valid_parameter_name("wärd"));
    }

    #[test]
    fn add_parameter_appends_a_use_in_entry_to_an_existing_or_missing_array() {
        let mut lib = json!({"resourceType": "Library"});
        add_parameter(&mut lib, "ward", "string").expect("valid declaration");
        assert_eq!(
            lib["parameter"],
            json!([{"name": "ward", "use": "in", "type": "string"}])
        );

        add_parameter(&mut lib, "days", "integer").expect("a second declaration");
        let params = lib["parameter"].as_array().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[1]["name"], "days");
    }

    #[test]
    fn add_parameter_keeps_existing_entries_untouched_including_use_out() {
        let mut lib = json!({
            "resourceType": "Library",
            "parameter": [{"name": "result", "use": "out", "type": "string"}],
        });
        add_parameter(&mut lib, "ward", "string").expect("valid declaration");
        let params = lib["parameter"].as_array().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(
            params[0],
            json!({"name": "result", "use": "out", "type": "string"})
        );
        assert_eq!(
            params[1],
            json!({"name": "ward", "use": "in", "type": "string"})
        );
    }

    #[test]
    fn add_parameter_rejects_an_invalid_name_without_mutating_the_document() {
        let mut lib = json!({"resourceType": "Library"});
        let err = add_parameter(&mut lib, "2ward", "string").unwrap_err();
        assert_eq!(err, AddParameterError::InvalidName);
        assert!(lib.get("parameter").is_none());
    }

    #[test]
    fn add_parameter_rejects_a_duplicate_use_in_name() {
        let mut lib = json!({
            "resourceType": "Library",
            "parameter": [{"name": "ward", "use": "in", "type": "string"}],
        });
        let err = add_parameter(&mut lib, "ward", "integer").unwrap_err();
        assert_eq!(err, AddParameterError::DuplicateName("ward".to_string()));
        assert_eq!(lib["parameter"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn add_parameter_allows_reusing_a_name_only_declared_as_use_out() {
        let mut lib = json!({
            "resourceType": "Library",
            "parameter": [{"name": "result", "use": "out", "type": "string"}],
        });
        // `parameters()` only reads `use=in`, so a `use=out` entry of the
        // same name is not a duplicate as far as #841's own gate is
        // concerned — the two live in disjoint namespaces in SQL on FHIR.
        add_parameter(&mut lib, "result", "string").expect("not a use=in duplicate");
        assert_eq!(lib["parameter"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn add_parameter_rejects_an_unbindable_type() {
        let mut lib = json!({"resourceType": "Library"});
        let err = add_parameter(&mut lib, "ward", "Quantity").unwrap_err();
        assert_eq!(err, AddParameterError::UnknownType);
        assert!(lib.get("parameter").is_none());
    }

    // -----------------------------------------------------------------
    // Tables panel (#842): table_dependencies() / tables_signature() /
    // dependency_lookup() / matches_reference() / used_by_*() /
    // add_table() / remove_table()
    // -----------------------------------------------------------------

    fn depends_on(label: &str, resource: &str) -> Value {
        json!({"type": "depends-on", "label": label, "resource": resource})
    }

    #[test]
    fn table_dependencies_reads_depends_on_entries_in_order_and_skips_other_types() {
        let lib = json!({
            "resourceType": "Library",
            "relatedArtifact": [
                depends_on("v", "http://example.org/ViewDefinition/v"),
                {"type": "composed-of", "resource": "http://example.org/ignored"},
                depends_on("w", "Library/w1"),
            ],
        });
        let deps = table_dependencies(&lib);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].label, "v");
        assert_eq!(deps[0].resource, "http://example.org/ViewDefinition/v");
        assert_eq!(deps[1].label, "w");
        assert_eq!(deps[1].resource, "Library/w1");
    }

    #[test]
    fn table_dependencies_defaults_a_missing_label_or_resource_to_empty() {
        let lib = json!({
            "relatedArtifact": [{"type": "depends-on", "resource": "Library/x"}],
        });
        let deps = table_dependencies(&lib);
        assert_eq!(deps[0].label, "");
        assert_eq!(deps[0].resource, "Library/x");
    }

    #[test]
    fn table_dependencies_is_empty_without_a_relatedartifact_array() {
        assert!(table_dependencies(&json!({"resourceType": "Library"})).is_empty());
    }

    #[test]
    fn label_declared_matches_without_distinguishing_case() {
        let lib = json!({"relatedArtifact": [depends_on("v", "Library/v1")]});
        assert!(label_declared(&lib, "v"));
        assert!(label_declared(&lib, "V"));
        assert!(!label_declared(&lib, "w"));
    }

    #[test]
    fn tables_signature_joins_label_resource_pairs_and_is_empty_with_none() {
        assert_eq!(tables_signature(&[]), "");
        let deps = vec![
            TableDependency {
                label: "v".to_string(),
                resource: "Library/v1".to_string(),
            },
            TableDependency {
                label: "w".to_string(),
                resource: "http://example.org/ViewDefinition/w".to_string(),
            },
        ];
        assert_eq!(
            tables_signature(&deps),
            "v=Library/v1,w=http://example.org/ViewDefinition/w"
        );
    }

    #[test]
    fn dependency_lookup_recognizes_type_id_prefixes() {
        assert_eq!(
            dependency_lookup("ViewDefinition/vd-1"),
            DependencyLookup::TypeId {
                resource_type: "ViewDefinition",
                id: "vd-1".to_string(),
            }
        );
        assert_eq!(
            dependency_lookup("Library/lib-1"),
            DependencyLookup::TypeId {
                resource_type: "Library",
                id: "lib-1".to_string(),
            }
        );
        // Only the first path segment counts as the id, mirroring the
        // server's own `rest.split('/').next()`.
        assert_eq!(
            dependency_lookup("Library/lib-1/_history/2"),
            DependencyLookup::TypeId {
                resource_type: "Library",
                id: "lib-1".to_string(),
            }
        );
    }

    #[test]
    fn dependency_lookup_falls_back_to_canonical_with_optional_version() {
        assert_eq!(
            dependency_lookup("http://example.org/ViewDefinition/v"),
            DependencyLookup::Canonical {
                canonical: "http://example.org/ViewDefinition/v".to_string(),
                version: None,
            }
        );
        assert_eq!(
            dependency_lookup("http://example.org/ViewDefinition/v|1.0.0"),
            DependencyLookup::Canonical {
                canonical: "http://example.org/ViewDefinition/v".to_string(),
                version: Some("1.0.0".to_string()),
            }
        );
        // Neither recognized prefix — an empty resource, or an id-less
        // `Type/` — is still (harmlessly) a canonical lookup.
        assert_eq!(
            dependency_lookup(""),
            DependencyLookup::Canonical {
                canonical: "".to_string(),
                version: None,
            }
        );
    }

    #[test]
    fn matches_reference_type_id_only_matches_the_artifacts_own_id() {
        let vd = json!({"resourceType": "ViewDefinition", "id": "vd-1"});
        assert!(matches_reference(
            "ViewDefinition/vd-1",
            "ViewDefinition",
            &vd
        ));
        assert!(!matches_reference(
            "ViewDefinition/vd-2",
            "ViewDefinition",
            &vd
        ));
        // The wrong expected_type prefix never matches even the right id.
        assert!(!matches_reference("Library/vd-1", "ViewDefinition", &vd));
    }

    #[test]
    fn matches_reference_canonical_compares_url_and_only_checks_version_when_pinned() {
        let unversioned = json!({"resourceType": "ViewDefinition", "url": "http://example.org/vd"});
        assert!(matches_reference(
            "http://example.org/vd",
            "ViewDefinition",
            &unversioned
        ));
        // The reference pins a version the artifact does not carry at all —
        // no match, exactly as `canonical_matches` requires server-side.
        assert!(!matches_reference(
            "http://example.org/vd|1.0.0",
            "ViewDefinition",
            &unversioned
        ));

        let versioned = json!({
            "resourceType": "ViewDefinition",
            "url": "http://example.org/vd",
            "version": "1.0.0",
        });
        assert!(matches_reference(
            "http://example.org/vd|1.0.0",
            "ViewDefinition",
            &versioned
        ));
        assert!(!matches_reference(
            "http://example.org/vd|2.0.0",
            "ViewDefinition",
            &versioned
        ));

        assert!(!matches_reference(
            "http://example.org/other",
            "ViewDefinition",
            &unversioned
        ));
        assert!(!matches_reference(
            "http://example.org/vd",
            "ViewDefinition",
            &json!({"resourceType": "ViewDefinition"})
        ));
    }

    #[test]
    fn classify_table_artifact_matches_view_definitions_and_sql_view_libraries_only() {
        let vd = json!({"id": "vd-1", "name": "patients_flat"});
        assert_eq!(
            classify_table_artifact("ViewDefinition", &vd),
            TableTarget::ViewDefinition {
                id: "vd-1".to_string(),
                name: "patients_flat".to_string(),
            }
        );

        let sql_view = library("sql-view", LIBRARY_TYPES_SYSTEM);
        assert_eq!(
            classify_table_artifact("Library", &sql_view),
            TableTarget::SqlView {
                id: "l1".to_string(),
                name: "q".to_string(),
            }
        );

        let sql_query = library("sql-query", LIBRARY_TYPES_SYSTEM);
        assert_eq!(
            classify_table_artifact("Library", &sql_query),
            TableTarget::NotATable
        );
    }

    #[test]
    fn used_by_artifacts_matches_by_identity_excludes_self_and_sorts_by_name() {
        let target = json!({
            "resourceType": "Library", "id": "v1", "name": "patients_flat",
            "url": "http://example.org/Library/patients_flat",
        });
        let dependent_view = json!({
            "resourceType": "Library", "id": "q2", "name": "z_query",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-query"}]},
            "relatedArtifact": [depends_on("v", "Library/v1")],
        });
        let dependent_by_url = json!({
            "resourceType": "Library", "id": "w3", "name": "a_view",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-view"}]},
            "relatedArtifact": [depends_on("v", "http://example.org/Library/patients_flat")],
        });
        let unrelated = json!({
            "resourceType": "Library", "id": "q4", "name": "unrelated",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-query"}]},
            "relatedArtifact": [depends_on("x", "Library/other")],
        });
        let libraries = vec![
            target.clone(),
            dependent_view.clone(),
            dependent_by_url.clone(),
            unrelated,
        ];
        let found = used_by_artifacts(&libraries, &target);
        assert_eq!(found.len(), 2);
        // Sorted by name: "a_view" before "z_query".
        assert_eq!(found[0].id, "w3");
        assert_eq!(found[0].kind, LibraryArtifactKind::SqlView);
        assert_eq!(found[1].id, "q2");
        assert_eq!(found[1].kind, LibraryArtifactKind::SqlQuery);
    }

    #[test]
    fn used_by_artifacts_finds_nothing_for_an_unsaved_document_with_no_url() {
        let starter = starter_library_value("sql-query");
        let other = json!({
            "resourceType": "Library", "id": "q1", "name": "q",
            "type": {"coding": [{"system": LIBRARY_TYPES_SYSTEM, "code": "sql-query"}]},
            "relatedArtifact": [depends_on("v", "Library/anything")],
        });
        assert!(used_by_artifacts(&[other], &starter).is_empty());
    }

    #[test]
    fn used_by_exports_matches_the_exact_library_reference_and_keeps_caller_order() {
        use crate::sql_export::{ExportJob, JobSubject};
        let job_a = ExportJob {
            name: "Nightly extract".to_string(),
            subjects: vec![JobSubject {
                name: "patients_flat".to_string(),
                reference: "Library/v1".to_string(),
                kind: "sql-view".to_string(),
                parameters: Vec::new(),
            }],
            ..Default::default()
        };
        let job_b = ExportJob {
            // No name — falls back to the job id.
            subjects: vec![JobSubject {
                name: "other".to_string(),
                reference: "Library/v2".to_string(),
                kind: "sql-view".to_string(),
                parameters: Vec::new(),
            }],
            ..Default::default()
        };
        let jobs = vec![("job-a".to_string(), job_a), ("job-b".to_string(), job_b)];
        let found = used_by_exports(&jobs, "v1");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].job_id, "job-a");
        assert_eq!(found[0].label, "Nightly extract");

        let found_b = used_by_exports(&jobs, "v2");
        assert_eq!(found_b[0].label, "job-b");

        assert!(used_by_exports(&jobs, "").is_empty());
    }

    #[test]
    fn add_table_appends_a_depends_on_entry_to_an_existing_or_missing_array() {
        let mut lib = json!({"resourceType": "Library"});
        add_table(&mut lib, "v", "http://example.org/ViewDefinition/v");
        assert_eq!(
            lib["relatedArtifact"],
            json!([depends_on("v", "http://example.org/ViewDefinition/v")])
        );

        add_table(&mut lib, "w", "Library/w1");
        let deps = lib["relatedArtifact"].as_array().unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[1]["label"], "w");
    }

    #[test]
    fn remove_table_removes_the_first_matching_label_only() {
        let mut lib = json!({
            "relatedArtifact": [
                depends_on("v", "Library/v1"),
                depends_on("w", "Library/w1"),
            ],
        });
        remove_table(&mut lib, "v");
        let deps = lib["relatedArtifact"].as_array().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0]["label"], "w");
    }

    #[test]
    fn remove_table_is_a_no_op_when_the_label_does_not_exist() {
        let mut lib = json!({"relatedArtifact": [depends_on("v", "Library/v1")]});
        remove_table(&mut lib, "missing");
        assert_eq!(lib["relatedArtifact"].as_array().unwrap().len(), 1);

        let mut no_deps = json!({"resourceType": "Library"});
        remove_table(&mut no_deps, "v");
        assert_eq!(no_deps, json!({"resourceType": "Library"}));
    }

    // -----------------------------------------------------------------
    // unknown_tables() / tables_signature_with_unknown() (#842/04)
    // -----------------------------------------------------------------

    #[test]
    fn unknown_tables_finds_a_table_no_declared_label_names() {
        let deps = vec![TableDependency {
            label: "v".to_string(),
            resource: "Library/v1".to_string(),
        }];
        let found = unknown_tables("SELECT * FROM v JOIN vv ON v.id = vv.id", &deps);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "vv");
    }

    #[test]
    fn unknown_tables_matches_declared_labels_case_insensitively() {
        let deps = vec![TableDependency {
            label: "V".to_string(),
            resource: "Library/v1".to_string(),
        }];
        assert!(unknown_tables("SELECT * FROM v", &deps).is_empty());
    }

    #[test]
    fn unknown_tables_reports_nothing_for_sql_the_scanner_cannot_tokenize() {
        assert!(unknown_tables("SELECT 'unterminated", &[]).is_empty());
    }

    #[test]
    fn tables_signature_with_unknown_appends_one_hint_per_name() {
        let deps = vec![TableDependency {
            label: "v".to_string(),
            resource: "Library/v1".to_string(),
        }];
        assert_eq!(
            tables_signature_with_unknown(&deps, &["vv".to_string()]),
            "v=Library/v1,?vv"
        );
        assert_eq!(
            tables_signature_with_unknown(&[], &["vv".to_string(), "ww".to_string()]),
            "?vv,?ww"
        );
        assert_eq!(tables_signature_with_unknown(&deps, &[]), "v=Library/v1");
        assert_eq!(tables_signature_with_unknown(&[], &[]), "");
    }

    #[test]
    fn dependencies_used_by_sql_keeps_only_labels_the_sql_actually_reads() {
        let deps = vec![
            TableDependency {
                label: "v".to_string(),
                resource: "Library/v1".to_string(),
            },
            TableDependency {
                label: "vv".to_string(),
                resource: "Library/v1".to_string(),
            },
        ];
        // Only "vv" is read; "v" is a stale declaration left over after
        // *Add table* resolved a formerly unknown spelling (#842/04).
        let used = dependencies_used_by_sql("SELECT * FROM vv", &deps);
        assert_eq!(used, vec![deps[1].clone()]);
    }

    #[test]
    fn dependencies_used_by_sql_matches_case_insensitively_and_reports_nothing_unparseable() {
        let deps = vec![TableDependency {
            label: "V".to_string(),
            resource: "Library/v1".to_string(),
        }];
        assert_eq!(
            dependencies_used_by_sql("SELECT * FROM v", &deps),
            deps.clone()
        );
        assert!(dependencies_used_by_sql("SELECT 'unterminated", &deps).is_empty());
    }

    // -----------------------------------------------------------------
    // analyze_columns() / infer_column_type_from_rows() (#842/04)
    // -----------------------------------------------------------------

    fn view_definition_with_columns(cols: &[(&str, &str)]) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "select": [{
                "column": cols.iter().map(|(name, type_code)| json!({
                    "name": name, "type": type_code,
                })).collect::<Vec<_>>(),
            }],
        })
    }

    #[test]
    fn analyze_columns_prefers_the_single_resolved_origin_and_its_type() {
        let vd = view_definition_with_columns(&[("id", "string"), ("family", "string")]);
        let rows = vec![json!({"id": "p1", "family": "Garcia", "n": 3, "ok": true})];
        let columns = vec![
            "id".to_string(),
            "family".to_string(),
            "n".to_string(),
            "ok".to_string(),
        ];
        let info = analyze_columns(&columns, &rows, &[("v".to_string(), vd)]);
        assert_eq!(
            info[0],
            ColumnInfo {
                name: "id".to_string(),
                type_code: Some("string".to_string()),
                origin: Some(("v".to_string(), "id".to_string())),
            }
        );
        assert_eq!(
            info[1].origin,
            Some(("v".to_string(), "family".to_string()))
        );
        // No dependency declares "n"/"ok" — type falls back to the row
        // values, and there is no origin to show.
        assert_eq!(
            info[2],
            ColumnInfo {
                name: "n".to_string(),
                type_code: Some("integer".to_string()),
                origin: None,
            }
        );
        assert_eq!(
            info[3],
            ColumnInfo {
                name: "ok".to_string(),
                type_code: Some("boolean".to_string()),
                origin: None,
            }
        );
    }

    #[test]
    fn analyze_columns_treats_a_column_declared_in_two_dependencies_as_having_no_origin() {
        let a = view_definition_with_columns(&[("id", "string")]);
        let b = view_definition_with_columns(&[("id", "string")]);
        let rows = vec![json!({"id": "p1"})];
        let info = analyze_columns(
            &["id".to_string()],
            &rows,
            &[("a".to_string(), a), ("b".to_string(), b)],
        );
        assert_eq!(info[0].origin, None);
        // Falls back to the row values: a single string value.
        assert_eq!(info[0].type_code.as_deref(), Some("string"));
    }

    #[test]
    fn infer_column_type_from_rows_covers_every_kind_and_the_empty_case() {
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": true}), json!({"x": false})]),
            Some("boolean")
        );
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": 1}), json!({"x": 2})]),
            Some("integer")
        );
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": 1}), json!({"x": 2.5})]),
            Some("decimal")
        );
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": "a"}), json!({"x": "b"})]),
            Some("string")
        );
        // A mix of kinds falls back to string.
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": 1}), json!({"x": "a"})]),
            Some("string")
        );
        // All null, missing, or no rows at all: no answer.
        assert_eq!(
            infer_column_type_from_rows("x", &[json!({"x": null}), json!({})]),
            None
        );
        assert_eq!(infer_column_type_from_rows("x", &[]), None);
    }
}
