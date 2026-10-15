//! Terminology dependencies of a SQLQuery / SQLView Library (SQL on FHIR
//! "Terminology in SQL", HL7/sql-on-fhir#407; HFS #1824).
//!
//! A Library may declare a **ValueSet** or a **ConceptMap** as a
//! `relatedArtifact[type=depends-on]` dependency. The runner exposes each one
//! under its `label` as an ordinary SQL relation with a fixed column set, so
//! code membership and code translation become plain joins:
//!
//! - value set relation: `system`, `version`, `code`, `display`, `inactive`
//!   (term-4), one row per distinct non-abstract `expansion.contains` entry
//!   at any depth (term-7, term-8);
//! - concept map relation: `source_system`, `source_version`, `source_code`,
//!   `source_display`, `target_system`, `target_version`, `target_code`,
//!   `target_display`, `relationship` (term-10), one row per
//!   `group.element.target` plus one per `noMap` element (term-13).
//!
//! Everything here is pure: no HTTP, no storage. The flatteners take the
//! artifact JSON and [`FlattenLimits`] and return rows plus the bytes those
//! rows retain; the caller (the REST layer's plan resolver, or the write-path
//! materializer) decides where the artifact came from and what a failure
//! means. Every cap is enforced **while traversing** (never after the whole
//! structure has been read), with an explicit stack and a fixed nesting
//! bound, so a hostile artifact is rejected before it is walked in full.
//!
//! [`expand_enumerated_compose`] is the no-terminology-server fallback: it
//! expands a `compose` that needs no terminology knowledge (enumerated
//! concepts only; see the function's eligibility rules) and refuses
//! everything else, so a local expansion never approximates membership.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::engine::{ColumnFhirType, ColumnSchema, TableSchema};

/// Columns of a value set relation, in order, as `(name, FHIR type code)`
/// (term-4, term-5).
pub const VALUE_SET_COLUMNS: [(&str, &str); 5] = [
    ("system", "uri"),
    ("version", "string"),
    ("code", "code"),
    ("display", "string"),
    ("inactive", "boolean"),
];

/// Columns of a concept map relation, in order, as `(name, FHIR type code)`
/// (term-10, term-11).
pub const CONCEPT_MAP_COLUMNS: [(&str, &str); 9] = [
    ("source_system", "uri"),
    ("source_version", "string"),
    ("source_code", "code"),
    ("source_display", "string"),
    ("target_system", "uri"),
    ("target_version", "string"),
    ("target_code", "code"),
    ("target_display", "string"),
    ("relationship", "code"),
];

/// Deepest `expansion.contains` nesting the flattener walks. Deeper
/// expansions are rejected as incomplete rather than recursed into.
pub const MAX_CONTAINS_DEPTH: usize = 32;

/// Visited-structure cap as a multiple of the member cap (D9).
pub const VISITED_PER_MEMBER: usize = 4;

/// Fixed per-row overhead charged by [`row_bytes`] on top of the string
/// bytes, covering the `serde_json::Value` object, its keys and the dedupe
/// set entry.
pub const ROW_OVERHEAD_BYTES: usize = 128;

/// Multiplier [`row_bytes`] applies to every string byte, a deliberately
/// conservative factor for the `String` allocations and the dedupe key copy.
pub const ROW_STRING_FACTOR: usize = 3;

fn schema_from(columns: &[(&str, &str)]) -> TableSchema {
    TableSchema {
        columns: columns
            .iter()
            .map(|(name, code)| ColumnSchema {
                name: (*name).to_string(),
                fhir_type: ColumnFhirType::from_code(code),
            })
            .collect(),
    }
}

/// The fixed schema of a value set relation.
pub fn value_set_schema() -> TableSchema {
    schema_from(&VALUE_SET_COLUMNS)
}

/// The fixed schema of a concept map relation.
pub fn concept_map_schema() -> TableSchema {
    schema_from(&CONCEPT_MAP_COLUMNS)
}

/// One row of a value set relation (term-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueSetRow {
    pub system: String,
    pub version: Option<String>,
    pub code: String,
    pub display: Option<String>,
    /// `None` when the expansion does not say; `Some(false)` when it says the
    /// code is active.
    pub inactive: Option<bool>,
}

impl ValueSetRow {
    /// The row as the engine's `INSERT` expects it: one JSON object keyed by
    /// column name.
    pub fn to_json(&self) -> Value {
        json!({
            "system": self.system,
            "version": self.version,
            "code": self.code,
            "display": self.display,
            "inactive": self.inactive,
        })
    }
}

/// One row of a concept map relation (term-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptMapRow {
    pub source_system: String,
    pub source_version: Option<String>,
    pub source_code: String,
    pub source_display: Option<String>,
    pub target_system: Option<String>,
    pub target_version: Option<String>,
    pub target_code: Option<String>,
    pub target_display: Option<String>,
    pub relationship: Option<String>,
}

impl ConceptMapRow {
    /// The row as the engine's `INSERT` expects it.
    pub fn to_json(&self) -> Value {
        json!({
            "source_system": self.source_system,
            "source_version": self.source_version,
            "source_code": self.source_code,
            "source_display": self.source_display,
            "target_system": self.target_system,
            "target_version": self.target_version,
            "target_code": self.target_code,
            "target_display": self.target_display,
            "relationship": self.relationship,
        })
    }
}

/// Caps applied while traversing, never after (D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlattenLimits {
    /// Most distinct rows a relation may have, checked at every insertion.
    pub max_members: usize,
    /// Most structures (`contains` entries, groups, elements, targets,
    /// includes, concepts) a flattener or eligibility check may touch,
    /// abstract or not. [`FlattenLimits::for_members`] sets it to
    /// [`VISITED_PER_MEMBER`] × `max_members`.
    pub max_visited: usize,
    /// Most bytes ([`row_bytes`], summed) a relation may retain.
    pub max_relation_bytes: usize,
}

impl FlattenLimits {
    /// Limits for `max_members` members and `max_relation_bytes` bytes, with
    /// the visited cap derived as [`VISITED_PER_MEMBER`] × `max_members`.
    pub fn for_members(max_members: usize, max_relation_bytes: usize) -> Self {
        Self {
            max_members,
            max_visited: max_members.saturating_mul(VISITED_PER_MEMBER),
            max_relation_bytes,
        }
    }
}

/// Why an artifact's content cannot be turned into a relation. Every variant
/// is the client's problem (a 422 at run time, or a refused write at
/// materialization time), never an infrastructure fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminologyContentError {
    /// The ValueSet carries no `expansion`.
    NoExpansion,
    /// The expansion is paged, truncated, deeper than
    /// [`MAX_CONTAINS_DEPTH`], or has an entry without `system` or `code`
    /// (term-24).
    IncompleteExpansion { reason: String },
    /// The ConceptMap uses a construct the relation cannot represent
    /// (term-16) or has an unknown shape.
    UnsupportedConceptMap { reason: String },
    /// More distinct rows than the member cap.
    TooManyMembers { max: usize },
    /// More structures visited than the visited cap.
    TooManyEntries { max: usize },
    /// The relation or the artifact is bigger than the byte cap.
    TooManyBytes { max: usize },
    /// The compose is not enumerated-only, so it cannot be expanded without
    /// a terminology server.
    NotEnumerated { reason: String },
}

impl std::fmt::Display for TerminologyContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoExpansion => f.write_str(
                "the ValueSet has no expansion; store it with an expansion, re-save the \
                 Library so the server materializes one, or supply it in `context` with an \
                 expansion",
            ),
            Self::IncompleteExpansion { reason } => {
                write!(
                    f,
                    "the ValueSet expansion is not usable as a relation: {reason}"
                )
            }
            Self::UnsupportedConceptMap { reason } => {
                write!(
                    f,
                    "the ConceptMap cannot be represented as a relation: {reason}"
                )
            }
            Self::TooManyMembers { max } => write!(
                f,
                "the relation has more than {max} rows (HFS_SOF_TERMINOLOGY_MAX_EXPANSION_MEMBERS)"
            ),
            Self::TooManyEntries { max } => write!(
                f,
                "the artifact has more than {max} entries to walk \
                 ({VISITED_PER_MEMBER} x HFS_SOF_TERMINOLOGY_MAX_EXPANSION_MEMBERS)"
            ),
            Self::TooManyBytes { max } => write!(
                f,
                "the artifact or its relation exceeds {max} bytes \
                 (HFS_SOF_TERMINOLOGY_MAX_ARTIFACT_BYTES)"
            ),
            Self::NotEnumerated { reason } => write!(
                f,
                "the ValueSet compose cannot be expanded without a terminology server: {reason}"
            ),
        }
    }
}

impl std::error::Error for TerminologyContentError {}

/// The OperationOutcome issue code a content error maps to: `too-costly` for
/// a cap, `invalid` for everything else (the spec's error table).
impl TerminologyContentError {
    pub fn issue_code(&self) -> &'static str {
        match self {
            Self::TooManyMembers { .. }
            | Self::TooManyEntries { .. }
            | Self::TooManyBytes { .. } => "too-costly",
            _ => "invalid",
        }
    }
}

// ============================================================================
// Size accounting
// ============================================================================

/// The bytes one relation row is charged with: the UTF-8 bytes of every
/// string column × [`ROW_STRING_FACTOR`], plus [`ROW_OVERHEAD_BYTES`]. The one
/// row measure, used for the per-artifact `max_relation_bytes` cap and for the
/// per-job budget alike, so both bound the same quantity.
pub fn row_bytes(row: &Value) -> usize {
    let strings: usize = match row {
        Value::Object(map) => map.values().map(|v| v.as_str().map_or(0, str::len)).sum(),
        Value::String(s) => s.len(),
        _ => 0,
    };
    strings
        .saturating_mul(ROW_STRING_FACTOR)
        .saturating_add(ROW_OVERHEAD_BYTES)
}

/// A bounded, allocation-free estimate of a JSON value's serialized size:
/// stops and returns [`TerminologyContentError::TooManyBytes`] as soon as
/// `cap` is exceeded, so an oversized artifact costs at most `cap` bytes of
/// walking. Walks with an explicit stack; never serializes.
pub fn estimate_json_bytes(value: &Value, cap: usize) -> Result<usize, TerminologyContentError> {
    let mut total = 0usize;
    let mut stack: Vec<&Value> = vec![value];
    while let Some(v) = stack.pop() {
        let own = match v {
            Value::Null => 4,
            Value::Bool(_) => 5,
            Value::Number(_) => 20,
            Value::String(s) => s.len() + 2,
            Value::Array(items) => {
                stack.extend(items.iter());
                2 + items.len()
            }
            Value::Object(map) => {
                let mut keys = 2;
                for (k, child) in map {
                    keys += k.len() + 4;
                    stack.push(child);
                }
                keys
            }
        };
        total = total.saturating_add(own);
        if total > cap {
            return Err(TerminologyContentError::TooManyBytes { max: cap });
        }
    }
    Ok(total)
}

/// Counts every structure a traversal touches against `max_visited`.
struct Visits {
    visited: usize,
    max: usize,
}

impl Visits {
    fn new(max: usize) -> Self {
        Self { visited: 0, max }
    }

    fn visit(&mut self) -> Result<(), TerminologyContentError> {
        self.visited += 1;
        if self.visited > self.max {
            return Err(TerminologyContentError::TooManyEntries { max: self.max });
        }
        Ok(())
    }
}

/// Collects rows under the member and byte caps, deduplicating on `key`.
struct RowSink<K, R> {
    rows: Vec<R>,
    seen: HashSet<K>,
    bytes: usize,
    limits: FlattenLimits,
}

impl<K: std::hash::Hash + Eq, R> RowSink<K, R> {
    fn new(limits: FlattenLimits) -> Self {
        Self {
            rows: Vec::new(),
            seen: HashSet::new(),
            bytes: 0,
            limits,
        }
    }

    /// Inserts `row` unless `key` was seen; checks both caps **at insertion**.
    fn emit(
        &mut self,
        key: K,
        row: R,
        row_json: impl FnOnce(&R) -> Value,
    ) -> Result<(), TerminologyContentError> {
        if !self.seen.insert(key) {
            return Ok(());
        }
        self.rows.push(row);
        if self.rows.len() > self.limits.max_members {
            return Err(TerminologyContentError::TooManyMembers {
                max: self.limits.max_members,
            });
        }
        let last = self.rows.last().expect("just pushed");
        self.bytes = self.bytes.saturating_add(row_bytes(&row_json(last)));
        if self.bytes > self.limits.max_relation_bytes {
            return Err(TerminologyContentError::TooManyBytes {
                max: self.limits.max_relation_bytes,
            });
        }
        Ok(())
    }
}

fn non_empty_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn opt_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

// ============================================================================
// ValueSet expansion → rows
// ============================================================================

/// Flattens a ValueSet's `expansion` into relation rows (term-7, term-8,
/// term-24): one row per distinct `(system, version, code)` among the
/// non-abstract `contains` entries at any depth. Returns the rows and the
/// bytes they retain ([`row_bytes`] summed).
///
/// `total`, when present, is compared against the number of `contains`
/// entries at **all** depths, abstract ones included — what term-24 says,
/// and what lets an expansion whose `total` counts an abstract grouping
/// parent pass. An expansion carrying `offset` is a page, never a whole, and
/// is rejected.
pub fn flatten_value_set(
    value_set: &Value,
    limits: FlattenLimits,
) -> Result<(Vec<ValueSetRow>, usize), TerminologyContentError> {
    let Some(expansion) = value_set.get("expansion").filter(|e| e.is_object()) else {
        return Err(TerminologyContentError::NoExpansion);
    };
    if let Some(offset) = expansion.get("offset") {
        return Err(TerminologyContentError::IncompleteExpansion {
            reason: format!("expansion carries offset={offset}; a page is not a whole expansion"),
        });
    }

    let mut visits = Visits::new(limits.max_visited);
    let mut sink: RowSink<(String, Option<String>, String), ValueSetRow> = RowSink::new(limits);

    let empty = Vec::new();
    let top = expansion
        .get("contains")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    // (entries, path, depth)
    let mut stack: Vec<(&Vec<Value>, String, usize)> =
        vec![(top, "expansion.contains".to_string(), 0)];

    while let Some((entries, path, depth)) = stack.pop() {
        if depth > MAX_CONTAINS_DEPTH {
            return Err(TerminologyContentError::IncompleteExpansion {
                reason: format!("contains nesting deeper than {MAX_CONTAINS_DEPTH} at {path}"),
            });
        }
        for (i, entry) in entries.iter().enumerate() {
            visits.visit()?;
            let is_abstract = entry.get("abstract").and_then(Value::as_bool) == Some(true);
            if !is_abstract {
                let system = non_empty_str(entry, "system").ok_or_else(|| {
                    TerminologyContentError::IncompleteExpansion {
                        reason: format!("{path}[{i}] has no system"),
                    }
                })?;
                let code = non_empty_str(entry, "code").ok_or_else(|| {
                    TerminologyContentError::IncompleteExpansion {
                        reason: format!("{path}[{i}] has no code"),
                    }
                })?;
                let row = ValueSetRow {
                    system: system.to_string(),
                    version: opt_string(entry, "version"),
                    code: code.to_string(),
                    display: opt_string(entry, "display"),
                    inactive: entry.get("inactive").and_then(Value::as_bool),
                };
                let key = (row.system.clone(), row.version.clone(), row.code.clone());
                sink.emit(key, row, ValueSetRow::to_json)?;
            }
            if let Some(children) = entry.get("contains").and_then(Value::as_array)
                && !children.is_empty()
            {
                stack.push((children, format!("{path}[{i}].contains"), depth + 1));
            }
        }
    }

    if let Some(total) = expansion.get("total").and_then(Value::as_u64)
        && (total as usize) > visits.visited
    {
        return Err(TerminologyContentError::IncompleteExpansion {
            reason: format!(
                "expansion.total={total} but {} contains entries at all depths",
                visits.visited
            ),
        });
    }

    Ok((sink.rows, sink.bytes))
}

// ============================================================================
// ConceptMap → rows
// ============================================================================

/// R4 `target.equivalence` → R5 `relationship` (D5). `unmatched` is the R4
/// spelling of a no-map element and yields `None`; an unknown code is an
/// error.
fn map_equivalence(equivalence: &str) -> Result<Option<&'static str>, String> {
    Ok(Some(match equivalence {
        "relatedto" | "inexact" => "related-to",
        "equivalent" | "equal" => "equivalent",
        "wider" | "subsumes" => "source-is-narrower-than-target",
        "narrower" | "specializes" => "source-is-broader-than-target",
        "disjoint" => "not-related-to",
        "unmatched" => return Ok(None),
        other => return Err(format!("unknown equivalence '{other}'")),
    }))
}

/// Splits an R5 `url|version` canonical; an R4 explicit `*Version` element
/// fills the version when the canonical carries none.
fn split_scope(
    scope: Option<&str>,
    explicit_version: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(scope) = scope.filter(|s| !s.is_empty()) else {
        return (None, explicit_version.map(str::to_string));
    };
    let (system, version) = match scope.split_once('|') {
        Some((s, v)) => (s.to_string(), Some(v.to_string())),
        None => (scope.to_string(), None),
    };
    (
        Some(system),
        version.or_else(|| explicit_version.map(str::to_string)),
    )
}

type ConceptMapKey = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Flattens a ConceptMap into relation rows (term-13 to term-17, D5): one
/// row per `group.element.target`, plus one per element with `noMap = true`
/// (or an R4 `unmatched` target) that carries the group's target system and
/// version with null `target_code`, `target_display` and `relationship`,
/// accepting the R4/R4B group shape (`sourceVersion`, `targetVersion`,
/// `target.equivalence`) and the R5/R6 shape (`url|version` scopes,
/// `target.relationship`, `element.noMap`) alike, detected per group and per
/// target. Rejects, naming the path, every construct the relation cannot
/// represent: a group without `source`, an element or target with
/// `valueSet`, a target with `dependsOn` or `product` (term-16).
/// `group.unmapped` is ignored (term-17).
pub fn flatten_concept_map(
    concept_map: &Value,
    limits: FlattenLimits,
) -> Result<(Vec<ConceptMapRow>, usize), TerminologyContentError> {
    let unsupported = |reason: String| TerminologyContentError::UnsupportedConceptMap { reason };
    let mut visits = Visits::new(limits.max_visited);
    let mut sink: RowSink<ConceptMapKey, ConceptMapRow> = RowSink::new(limits);

    let empty = Vec::new();
    let groups = concept_map
        .get("group")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    for (gi, group) in groups.iter().enumerate() {
        visits.visit()?;
        let (source_system, source_version) = split_scope(
            group.get("source").and_then(Value::as_str),
            group.get("sourceVersion").and_then(Value::as_str),
        );
        let source_system =
            source_system.ok_or_else(|| unsupported(format!("group[{gi}] has no source")))?;
        let (target_system, target_version) = split_scope(
            group.get("target").and_then(Value::as_str),
            group.get("targetVersion").and_then(Value::as_str),
        );

        let elements = group
            .get("element")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        for (ei, element) in elements.iter().enumerate() {
            visits.visit()?;
            let epath = format!("group[{gi}].element[{ei}]");
            if element.get("valueSet").is_some() {
                return Err(unsupported(format!("{epath} maps a valueSet, not a code")));
            }
            let source_code = non_empty_str(element, "code")
                .ok_or_else(|| unsupported(format!("{epath} has no code")))?;
            let source_display = opt_string(element, "display");

            // A no-map row keeps the group's target system and version: the
            // assertion is that the code has no mapping *into that target*,
            // so only `target_code`, `target_display` and `relationship` are
            // null (the spec page's worked example).
            let no_map_row = |sink: &mut RowSink<ConceptMapKey, ConceptMapRow>| {
                let row = ConceptMapRow {
                    source_system: source_system.clone(),
                    source_version: source_version.clone(),
                    source_code: source_code.to_string(),
                    source_display: source_display.clone(),
                    target_system: target_system.clone(),
                    target_version: target_version.clone(),
                    target_code: None,
                    target_display: None,
                    relationship: None,
                };
                let key = (
                    row.source_system.clone(),
                    row.source_version.clone(),
                    row.source_code.clone(),
                    row.target_system.clone(),
                    row.target_version.clone(),
                    None,
                    None,
                );
                sink.emit(key, row, ConceptMapRow::to_json)
            };

            if element.get("noMap").and_then(Value::as_bool) == Some(true) {
                no_map_row(&mut sink)?;
            }

            let targets = element
                .get("target")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            for (ti, target) in targets.iter().enumerate() {
                visits.visit()?;
                let tpath = format!("{epath}.target[{ti}]");
                if target.get("valueSet").is_some() {
                    return Err(unsupported(format!(
                        "{tpath} maps to a valueSet, not a code"
                    )));
                }
                if target
                    .get("dependsOn")
                    .and_then(Value::as_array)
                    .is_some_and(|d| !d.is_empty())
                {
                    return Err(unsupported(format!("{tpath} has dependsOn")));
                }
                if target
                    .get("product")
                    .and_then(Value::as_array)
                    .is_some_and(|p| !p.is_empty())
                {
                    return Err(unsupported(format!("{tpath} has product")));
                }
                let relationship = match target.get("equivalence").and_then(Value::as_str) {
                    Some(equivalence) => match map_equivalence(equivalence) {
                        Ok(Some(rel)) => Some(rel.to_string()),
                        Ok(None) => {
                            // R4 `unmatched`: a no-map row, not a target row.
                            no_map_row(&mut sink)?;
                            continue;
                        }
                        Err(reason) => return Err(unsupported(format!("{tpath}: {reason}"))),
                    },
                    None => opt_string(target, "relationship"),
                };
                let target_code = non_empty_str(target, "code")
                    .ok_or_else(|| unsupported(format!("{tpath} has no code")))?;
                let row = ConceptMapRow {
                    source_system: source_system.clone(),
                    source_version: source_version.clone(),
                    source_code: source_code.to_string(),
                    source_display: source_display.clone(),
                    target_system: target_system.clone(),
                    target_version: target_version.clone(),
                    target_code: Some(target_code.to_string()),
                    target_display: opt_string(target, "display"),
                    relationship,
                };
                let key = (
                    row.source_system.clone(),
                    row.source_version.clone(),
                    row.source_code.clone(),
                    row.target_system.clone(),
                    row.target_version.clone(),
                    row.target_code.clone(),
                    row.relationship.clone(),
                );
                sink.emit(key, row, ConceptMapRow::to_json)?;
            }
        }
    }

    Ok((sink.rows, sink.bytes))
}

// ============================================================================
// Local expansion of an enumerated compose
// ============================================================================

/// Why a compose is not eligible for local expansion.
fn ineligible(reason: impl Into<String>) -> TerminologyContentError {
    TerminologyContentError::NotEnumerated {
        reason: reason.into(),
    }
}

/// Checks one `include` or `exclude` entry against the eligibility rules and
/// counts every structure it inspects. Returns the entry's `(system,
/// version)`.
fn check_compose_entry<'a>(
    entry: &'a Value,
    path: &str,
    visits: &mut Visits,
) -> Result<(&'a str, Option<&'a str>), TerminologyContentError> {
    visits.visit()?;
    let system = non_empty_str(entry, "system")
        .ok_or_else(|| ineligible(format!("{path} has no system")))?;
    if entry.get("filter").is_some() {
        return Err(ineligible(format!("{path} uses a filter")));
    }
    if entry.get("valueSet").is_some() {
        return Err(ineligible(format!("{path} includes another valueSet")));
    }
    let version = entry.get("version").and_then(Value::as_str);
    if version == Some("*") {
        return Err(ineligible(format!("{path} selects every version ('*')")));
    }
    let concepts = entry.get("concept").and_then(Value::as_array);
    match concepts {
        Some(concepts) if !concepts.is_empty() => {
            for (ci, concept) in concepts.iter().enumerate() {
                visits.visit()?;
                if non_empty_str(concept, "code").is_none() {
                    return Err(ineligible(format!("{path}.concept[{ci}] has no code")));
                }
            }
        }
        _ => {
            return Err(ineligible(format!(
                "{path} selects a whole code system (no enumerated concepts)"
            )));
        }
    }
    Ok((system, version))
}

/// Expands a ValueSet locally, without a terminology server, when — and only
/// when — its `compose` needs no terminology knowledge (D3, D7):
///
/// 1. `compose.include` is a non-empty array;
/// 2. `compose.inactive` is absent (either value needs activity knowledge);
/// 3. `compose.lockedDate` is absent;
/// 4. every `include` and `exclude` has a non-empty `system`, a non-empty
///    enumerated `concept[]`, no `filter`, no `valueSet`, and no
///    `version: "*"`;
/// 5. every concept has a non-empty `code`;
/// 6. for each code system, every include and exclude naming it carries the
///    same `version` (all absent, or all the same), so an exclusion is an
///    exact key match.
///
/// Returns `Ok(None)` when ineligible (the caller reports why through
/// [`TerminologyContentError::NotEnumerated`] via [`local_expansion`]), or
/// `Ok(Some(value_set))`: the input with an `expansion` added — flat, with
/// `identifier`, `timestamp: now`, `total`, one `version` parameter per
/// distinct versioned include, and no `abstract` or `inactive` flags, since
/// a local expansion asserts membership exactly as enumerated and nothing
/// more. Counts every structure it inspects against `max_visited` and the
/// members against `max_members` at insertion.
pub fn expand_enumerated_compose(
    value_set: &Value,
    now: DateTime<Utc>,
    identifier: &str,
    limits: FlattenLimits,
) -> Result<Option<Value>, TerminologyContentError> {
    match local_expansion(value_set, now, identifier, limits) {
        Ok(expanded) => Ok(Some(expanded)),
        Err(TerminologyContentError::NotEnumerated { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// [`expand_enumerated_compose`] with the ineligibility reason surfaced as
/// [`TerminologyContentError::NotEnumerated`] instead of `Ok(None)`.
pub fn local_expansion(
    value_set: &Value,
    now: DateTime<Utc>,
    identifier: &str,
    limits: FlattenLimits,
) -> Result<Value, TerminologyContentError> {
    let compose = value_set
        .get("compose")
        .filter(|c| c.is_object())
        .ok_or_else(|| ineligible("the ValueSet has no compose"))?;
    if compose.get("inactive").is_some() {
        return Err(ineligible(
            "compose.inactive needs code system activity knowledge",
        ));
    }
    if compose.get("lockedDate").is_some() {
        return Err(ineligible("compose.lockedDate pins code system versions"));
    }
    let includes = compose
        .get("include")
        .and_then(Value::as_array)
        .filter(|i| !i.is_empty())
        .ok_or_else(|| ineligible("compose.include is empty"))?;
    let empty = Vec::new();
    let excludes = compose
        .get("exclude")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let mut visits = Visits::new(limits.max_visited);
    // Rule 6: one version per system across includes and excludes.
    let mut version_by_system: std::collections::HashMap<&str, Option<&str>> =
        std::collections::HashMap::new();
    let mut entries: Vec<(&Value, &str, Option<&str>)> = Vec::new();
    for (list, name) in [(includes, "include"), (excludes, "exclude")] {
        for (i, entry) in list.iter().enumerate() {
            let path = format!("compose.{name}[{i}]");
            let (system, version) = check_compose_entry(entry, &path, &mut visits)?;
            match version_by_system.get(system) {
                Some(seen) if *seen != version => {
                    return Err(ineligible(format!(
                        "{path} names {system} with version {} while another entry names it \
                         with version {}; mixed versions need a code system to compare",
                        version.unwrap_or("<none>"),
                        seen.unwrap_or("<none>")
                    )));
                }
                Some(_) => {}
                None => {
                    version_by_system.insert(system, version);
                }
            }
            entries.push((entry, system, version));
        }
    }

    // Ordered set keyed by (system, version, code).
    let mut members: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<(String, Option<String>, String), usize> =
        std::collections::HashMap::new();
    let mut removed: HashSet<usize> = HashSet::new();
    let mut parameters: Vec<Value> = Vec::new();
    let mut seen_versions: HashSet<(String, String)> = HashSet::new();

    let include_count = includes.len();
    for (n, (entry, system, version)) in entries.iter().enumerate() {
        visits.visit()?;
        let is_include = n < include_count;
        if is_include
            && let Some(v) = version
            && seen_versions.insert(((*system).to_string(), (*v).to_string()))
        {
            parameters.push(json!({"name": "version", "valueUri": format!("{system}|{v}")}));
        }
        let concepts = entry
            .get("concept")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        for concept in concepts {
            visits.visit()?;
            let code = non_empty_str(concept, "code").expect("checked by eligibility");
            let key = (
                (*system).to_string(),
                version.map(str::to_string),
                code.to_string(),
            );
            if is_include {
                if index.contains_key(&key) {
                    continue;
                }
                let mut member = Map::new();
                member.insert("system".into(), json!(system));
                if let Some(v) = version {
                    member.insert("version".into(), json!(v));
                }
                member.insert("code".into(), json!(code));
                if let Some(display) = non_empty_str(concept, "display") {
                    member.insert("display".into(), json!(display));
                }
                index.insert(key, members.len());
                members.push(Value::Object(member));
                if members.len() - removed.len() > limits.max_members {
                    return Err(TerminologyContentError::TooManyMembers {
                        max: limits.max_members,
                    });
                }
            } else if let Some(&pos) = index.get(&key) {
                removed.insert(pos);
            }
        }
    }

    let contains: Vec<Value> = members
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, m)| m)
        .collect();

    let mut expansion = Map::new();
    expansion.insert("identifier".into(), json!(identifier));
    expansion.insert("timestamp".into(), json!(now.to_rfc3339()));
    expansion.insert("total".into(), json!(contains.len()));
    if !parameters.is_empty() {
        expansion.insert("parameter".into(), Value::Array(parameters));
    }
    expansion.insert("contains".into(), Value::Array(contains));

    let mut expanded = value_set.clone();
    if let Value::Object(map) = &mut expanded {
        map.insert("expansion".into(), Value::Object(expansion));
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> FlattenLimits {
        FlattenLimits::for_members(1_000, 1 << 20)
    }

    /// The spec page's example ValueSet: `total: 3` counts one abstract
    /// grouping parent plus two leaf members.
    fn spec_value_set() -> Value {
        json!({
            "resourceType": "ValueSet",
            "url": "http://example.org/ValueSet/cardiovascular-disease",
            "version": "2026",
            "status": "active",
            "expansion": {
                "identifier": "urn:uuid:3c5a4d2e",
                "timestamp": "2026-01-15T10:00:00Z",
                "total": 3,
                "contains": [{
                    "system": "http://snomed.info/sct",
                    "abstract": true,
                    "display": "Cardiovascular disease (grouper)",
                    "contains": [
                        {"system": "http://snomed.info/sct", "version": "http://snomed.info/sct/900000000000207008/version/20250101",
                         "code": "22298006", "display": "Myocardial infarction"},
                        {"system": "http://hl7.org/fhir/sid/icd-10", "code": "I21", "display": "Acute myocardial infarction"}
                    ]
                }]
            }
        })
    }

    #[test]
    fn schemas_carry_the_fixed_columns_in_order() {
        let vs = value_set_schema();
        let names: Vec<&str> = vs.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["system", "version", "code", "display", "inactive"]);
        assert_eq!(
            vs.columns[0].fhir_type,
            ColumnFhirType::String("uri".into())
        );
        assert_eq!(
            vs.columns[2].fhir_type,
            ColumnFhirType::String("code".into())
        );
        assert_eq!(vs.columns[4].fhir_type, ColumnFhirType::Boolean);
        let cm = concept_map_schema();
        assert_eq!(cm.columns.len(), 9);
        assert_eq!(cm.columns[8].name, "relationship");
        assert_eq!(
            cm.columns[8].fhir_type,
            ColumnFhirType::String("code".into())
        );
    }

    #[test]
    fn spec_value_set_yields_two_rows_and_total_counts_the_abstract_parent() {
        let (rows, bytes) = flatten_value_set(&spec_value_set(), limits()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].code, "22298006");
        assert_eq!(
            rows[0].version.as_deref(),
            Some("http://snomed.info/sct/900000000000207008/version/20250101")
        );
        assert_eq!(rows[1].system, "http://hl7.org/fhir/sid/icd-10");
        assert_eq!(rows[1].version, None);
        assert_eq!(rows[1].inactive, None);
        assert!(bytes >= 2 * ROW_OVERHEAD_BYTES, "{bytes}");
    }

    #[test]
    fn duplicate_triple_under_two_parents_is_one_row() {
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "abstract": true, "contains": [{"system": "s", "code": "a"}]},
            {"system": "s", "abstract": true, "contains": [{"system": "s", "code": "a", "display": "again"}]},
            {"system": "s", "code": "b"}
        ]}});
        let (rows, _) = flatten_value_set(&vs, limits()).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn inactive_is_preserved_true_false_and_absent() {
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "code": "a", "inactive": true},
            {"system": "s", "code": "b", "inactive": false},
            {"system": "s", "code": "c"}
        ]}});
        let (rows, _) = flatten_value_set(&vs, limits()).unwrap();
        assert_eq!(rows[0].inactive, Some(true));
        assert_eq!(rows[1].inactive, Some(false));
        assert_eq!(rows[2].inactive, None);
        assert_eq!(rows[0].to_json()["inactive"], json!(true));
        assert_eq!(rows[2].to_json()["inactive"], Value::Null);
    }

    #[test]
    fn two_null_versions_are_equal_but_a_version_distinguishes() {
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "code": "a"},
            {"system": "s", "code": "a"},
            {"system": "s", "code": "a", "version": "1"}
        ]}});
        let (rows, _) = flatten_value_set(&vs, limits()).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn offset_is_a_page_and_is_rejected() {
        let vs = json!({"expansion": {"offset": 0, "contains": [{"system": "s", "code": "a"}]}});
        match flatten_value_set(&vs, limits()) {
            Err(TerminologyContentError::IncompleteExpansion { reason }) => {
                assert!(reason.contains("offset=0"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn total_above_entries_at_all_depths_is_incomplete_and_absent_total_is_fine() {
        let mut vs = spec_value_set();
        vs["expansion"]["total"] = json!(4);
        match flatten_value_set(&vs, limits()) {
            Err(TerminologyContentError::IncompleteExpansion { reason }) => {
                assert!(
                    reason.contains("total=4") && reason.contains("3 contains"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }
        vs["expansion"].as_object_mut().unwrap().remove("total");
        assert_eq!(flatten_value_set(&vs, limits()).unwrap().0.len(), 2);
    }

    #[test]
    fn non_abstract_entry_without_code_or_system_names_its_path() {
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "abstract": true, "contains": [{"system": "s"}]}
        ]}});
        match flatten_value_set(&vs, limits()) {
            Err(TerminologyContentError::IncompleteExpansion { reason }) => {
                assert_eq!(reason, "expansion.contains[0].contains[0] has no code")
            }
            other => panic!("{other:?}"),
        }
        let vs = json!({"expansion": {"contains": [{"code": "a"}]}});
        match flatten_value_set(&vs, limits()) {
            Err(TerminologyContentError::IncompleteExpansion { reason }) => {
                assert_eq!(reason, "expansion.contains[0] has no system")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn abstract_entry_with_a_code_contributes_no_row() {
        let vs = json!({"expansion": {"contains": [{"system": "s", "code": "grouper", "abstract": true}]}});
        assert!(flatten_value_set(&vs, limits()).unwrap().0.is_empty());
    }

    #[test]
    fn empty_contains_is_zero_rows_and_no_expansion_is_an_error() {
        assert!(
            flatten_value_set(&json!({"expansion": {"contains": []}}), limits())
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            flatten_value_set(&json!({"compose": {}}), limits()),
            Err(TerminologyContentError::NoExpansion)
        );
    }

    #[test]
    fn member_cap_stops_before_the_fourth_entry_is_visited() {
        // The 4th entry is malformed; a cap of 2 must fire on the 3rd member
        // before the 4th is ever inspected.
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "code": "a"}, {"system": "s", "code": "b"},
            {"system": "s", "code": "c"}, {"code": "no-system"}
        ]}});
        assert_eq!(
            flatten_value_set(&vs, FlattenLimits::for_members(2, 1 << 20)),
            Err(TerminologyContentError::TooManyMembers { max: 2 })
        );
    }

    #[test]
    fn visited_cap_rejects_abstract_only_expansions() {
        let entries: Vec<Value> = (0..1_000)
            .map(|_| json!({"system": "s", "abstract": true}))
            .collect();
        let vs = json!({"expansion": {"contains": entries}});
        let limits = FlattenLimits::for_members(2, 1 << 20);
        assert_eq!(
            flatten_value_set(&vs, limits),
            Err(TerminologyContentError::TooManyEntries { max: 8 })
        );
    }

    #[test]
    fn byte_cap_is_checked_at_insertion() {
        let vs = json!({"expansion": {"contains": [
            {"system": "s", "code": "a", "display": "x".repeat(100)},
            {"system": "s", "code": "b", "display": "y".repeat(100)}
        ]}});
        let limits = FlattenLimits {
            max_members: 10,
            max_visited: 40,
            max_relation_bytes: 500,
        };
        assert_eq!(
            flatten_value_set(&vs, limits),
            Err(TerminologyContentError::TooManyBytes { max: 500 })
        );
    }

    #[test]
    fn nesting_deeper_than_the_bound_is_rejected() {
        let mut node = json!({"system": "s", "code": "leaf"});
        for _ in 0..=MAX_CONTAINS_DEPTH {
            node = json!({"system": "s", "abstract": true, "contains": [node]});
        }
        let vs = json!({"expansion": {"contains": [node]}});
        match flatten_value_set(&vs, FlattenLimits::for_members(1_000, 1 << 20)) {
            Err(TerminologyContentError::IncompleteExpansion { reason }) => {
                assert!(reason.contains("deeper than 32"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// The spec page's example ConceptMap (R5 shape).
    fn spec_concept_map() -> Value {
        json!({
            "resourceType": "ConceptMap",
            "url": "http://example.org/ConceptMap/sct-to-icd10",
            "version": "2026",
            "status": "active",
            "group": [{
                "source": "http://snomed.info/sct|http://snomed.info/sct/900000000000207008/version/20250101",
                "target": "http://hl7.org/fhir/sid/icd-10|2024",
                "element": [
                    {"code": "22298006", "display": "Myocardial infarction",
                     "target": [{"code": "I21", "display": "Acute myocardial infarction", "relationship": "equivalent"}]},
                    {"code": "73211009", "display": "Diabetes mellitus",
                     "target": [{"code": "E14", "display": "Unspecified diabetes mellitus", "relationship": "source-is-broader-than-target"}]},
                    {"code": "102499006", "display": "Example concept with no mapping", "noMap": true}
                ]
            }]
        })
    }

    #[test]
    fn spec_concept_map_yields_the_three_rows_of_the_spec_table() {
        let (rows, _) = flatten_concept_map(&spec_concept_map(), limits()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].source_system, "http://snomed.info/sct");
        assert_eq!(
            rows[0].source_version.as_deref(),
            Some("http://snomed.info/sct/900000000000207008/version/20250101")
        );
        assert_eq!(
            rows[0].target_system.as_deref(),
            Some("http://hl7.org/fhir/sid/icd-10")
        );
        assert_eq!(rows[0].target_version.as_deref(), Some("2024"));
        assert_eq!(rows[0].target_code.as_deref(), Some("I21"));
        assert_eq!(rows[0].relationship.as_deref(), Some("equivalent"));
        assert_eq!(
            rows[1].relationship.as_deref(),
            Some("source-is-broader-than-target")
        );
        assert_eq!(rows[2].source_code, "102499006");
        assert_eq!(
            rows[2].target_system.as_deref(),
            Some("http://hl7.org/fhir/sid/icd-10"),
            "a noMap row keeps the group's target system"
        );
        assert_eq!(rows[2].target_version.as_deref(), Some("2024"));
        assert_eq!(rows[2].target_code, None);
        assert_eq!(rows[2].target_display, None);
        assert_eq!(rows[2].relationship, None);
    }

    #[test]
    fn two_groups_and_seven_tuple_dedupe() {
        let cm = json!({"group": [
            {"source": "a", "target": "b", "element": [{"code": "1", "target": [{"code": "x", "relationship": "equivalent"}]}]},
            {"source": "a", "target": "b", "element": [{"code": "1", "target": [{"code": "x", "relationship": "equivalent", "display": "dup"}]}]},
            {"source": "c", "element": [{"code": "2", "noMap": true}, {"code": "2", "noMap": true}]}
        ]});
        let (rows, _) = flatten_concept_map(&cm, limits()).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn term_16_rejections_name_the_path() {
        let cases = [
            (
                json!({"group": [{"element": []}]}),
                "group[0] has no source",
            ),
            (
                json!({"group": [{"source": "a", "element": [{"valueSet": "vs"}]}]}),
                "group[0].element[0] maps a valueSet",
            ),
            (
                json!({"group": [{"source": "a", "element": [{"code": "1", "target": [{"valueSet": "vs"}]}]}]}),
                "group[0].element[0].target[0] maps to a valueSet",
            ),
            (
                json!({"group": [{"source": "a", "element": [{"code": "1", "target": [{"code": "x", "relationship": "equivalent", "dependsOn": [{"attribute": "p", "value": "v"}]}]}]}]}),
                "target[0] has dependsOn",
            ),
            (
                json!({"group": [{"source": "a", "element": [{"code": "1", "target": [{"code": "x", "relationship": "equivalent", "product": [{"attribute": "p", "value": "v"}]}]}]}]}),
                "target[0] has product",
            ),
        ];
        for (cm, expected) in cases {
            match flatten_concept_map(&cm, limits()) {
                Err(TerminologyContentError::UnsupportedConceptMap { reason }) => {
                    assert!(reason.contains(expected), "{reason} vs {expected}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn unmapped_is_ignored_and_an_empty_map_is_zero_rows() {
        let cm = json!({"group": [{"source": "a", "unmapped": {"mode": "fixed", "code": "x"}, "element": []}]});
        assert!(flatten_concept_map(&cm, limits()).unwrap().0.is_empty());
        assert!(
            flatten_concept_map(&json!({"resourceType": "ConceptMap"}), limits())
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn member_cap_fires_before_the_malformed_last_target_is_read() {
        let mut targets: Vec<Value> = (0..4)
            .map(|i| json!({"code": format!("t{i}"), "relationship": "equivalent"}))
            .collect();
        targets.push(json!({"relationship": "equivalent"})); // no code
        let cm = json!({"group": [{"source": "a", "element": [{"code": "1", "target": targets}]}]});
        assert_eq!(
            flatten_concept_map(&cm, FlattenLimits::for_members(2, 1 << 20)),
            Err(TerminologyContentError::TooManyMembers { max: 2 })
        );
    }

    #[test]
    fn r4_versions_and_every_equivalence_code_are_mapped() {
        let codes = [
            ("relatedto", Some("related-to")),
            ("equivalent", Some("equivalent")),
            ("equal", Some("equivalent")),
            ("wider", Some("source-is-narrower-than-target")),
            ("subsumes", Some("source-is-narrower-than-target")),
            ("narrower", Some("source-is-broader-than-target")),
            ("specializes", Some("source-is-broader-than-target")),
            ("inexact", Some("related-to")),
            ("disjoint", Some("not-related-to")),
        ];
        for (r4, r5) in codes {
            let cm = json!({"group": [{
                "source": "http://snomed.info/sct", "sourceVersion": "v1",
                "target": "http://loinc.org", "targetVersion": "2.77",
                "element": [{"code": "1", "target": [{"code": "x", "equivalence": r4}]}]
            }]});
            let (rows, _) = flatten_concept_map(&cm, limits()).unwrap();
            assert_eq!(rows.len(), 1, "{r4}");
            assert_eq!(rows[0].source_version.as_deref(), Some("v1"));
            assert_eq!(rows[0].target_version.as_deref(), Some("2.77"));
            assert_eq!(rows[0].relationship.as_deref(), r5, "{r4}");
        }
    }

    #[test]
    fn r4_unmatched_is_a_no_map_row_and_unknown_equivalence_is_rejected() {
        let cm = json!({"group": [{"source": "a", "target": "b",
            "element": [{"code": "1", "target": [{"equivalence": "unmatched"}]}]}]});
        let (rows, _) = flatten_concept_map(&cm, limits()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_code, None);
        assert_eq!(rows[0].relationship, None);
        let cm = json!({"group": [{"source": "a",
            "element": [{"code": "1", "target": [{"code": "x", "equivalence": "sideways"}]}]}]});
        match flatten_concept_map(&cm, limits()) {
            Err(TerminologyContentError::UnsupportedConceptMap { reason }) => {
                assert!(
                    reason.contains("unknown equivalence 'sideways'"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn r4_depends_on_and_product_are_rejected_too() {
        let cm = json!({"group": [{"source": "a",
            "element": [{"code": "1", "target": [{"code": "x", "equivalence": "equal",
                "dependsOn": [{"property": "p", "value": "v"}]}]}]}]});
        assert!(matches!(
            flatten_concept_map(&cm, limits()),
            Err(TerminologyContentError::UnsupportedConceptMap { .. })
        ));
    }

    // ---- local expansion -------------------------------------------------

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-14T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn enumerated(include: Value, exclude: Option<Value>) -> Value {
        let mut compose = json!({"include": include});
        if let Some(exclude) = exclude {
            compose["exclude"] = exclude;
        }
        json!({"resourceType": "ValueSet", "url": "http://example.org/vs", "compose": compose})
    }

    #[test]
    fn enumerated_compose_expands_with_identifier_timestamp_total_and_parameters() {
        let vs = enumerated(
            json!([{"system": "http://snomed.info/sct", "version": "2025",
                    "concept": [{"code": "a", "display": "A"}, {"code": "b"}, {"code": "a"}]},
                   {"system": "http://loinc.org", "concept": [{"code": "c"}]}]),
            None,
        );
        let expanded = expand_enumerated_compose(&vs, now(), "urn:uuid:test", limits())
            .unwrap()
            .expect("eligible");
        let expansion = &expanded["expansion"];
        assert_eq!(expansion["identifier"], "urn:uuid:test");
        assert_eq!(expansion["timestamp"], "2026-10-14T12:00:00+00:00");
        assert_eq!(expansion["total"], 3);
        assert_eq!(
            expansion["parameter"],
            json!([{"name": "version", "valueUri": "http://snomed.info/sct|2025"}])
        );
        let contains = expansion["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 3);
        assert_eq!(
            contains[0],
            json!({"system": "http://snomed.info/sct", "version": "2025", "code": "a", "display": "A"})
        );
        assert!(contains[2].get("version").is_none());
        for c in contains {
            assert!(c.get("inactive").is_none() && c.get("abstract").is_none());
        }
        // The original compose is kept.
        assert_eq!(expanded["compose"], vs["compose"]);
        // And the result flattens.
        assert_eq!(flatten_value_set(&expanded, limits()).unwrap().0.len(), 3);
    }

    #[test]
    fn excludes_with_agreeing_versions_remove_exact_keys() {
        let vs = enumerated(
            json!([{"system": "s", "version": "1", "concept": [{"code": "a"}, {"code": "b"}]}]),
            Some(json!([{"system": "s", "version": "1", "concept": [{"code": "a"}]}])),
        );
        let expanded = expand_enumerated_compose(&vs, now(), "id", limits())
            .unwrap()
            .unwrap();
        assert_eq!(
            expanded["expansion"]["contains"],
            json!([{"system": "s", "version": "1", "code": "b"}])
        );
        assert_eq!(expanded["expansion"]["total"], 1);
    }

    #[test]
    fn every_ineligible_shape_is_refused_not_approximated() {
        let cases: Vec<(&str, Value)> = vec![
            ("no compose", json!({"resourceType": "ValueSet"})),
            ("empty include", enumerated(json!([]), None)),
            ("inactive true", {
                let mut v = enumerated(json!([{"system": "s", "concept": [{"code": "a"}]}]), None);
                v["compose"]["inactive"] = json!(true);
                v
            }),
            ("inactive false", {
                let mut v = enumerated(json!([{"system": "s", "concept": [{"code": "a"}]}]), None);
                v["compose"]["inactive"] = json!(false);
                v
            }),
            ("lockedDate", {
                let mut v = enumerated(json!([{"system": "s", "concept": [{"code": "a"}]}]), None);
                v["compose"]["lockedDate"] = json!("2026-01-01");
                v
            }),
            (
                "version *",
                enumerated(
                    json!([{"system": "s", "version": "*", "concept": [{"code": "a"}]}]),
                    None,
                ),
            ),
            ("whole system", enumerated(json!([{"system": "s"}]), None)),
            (
                "filter",
                enumerated(
                    json!([{"system": "s", "filter": [{"property": "concept", "op": "is-a", "value": "x"}]}]),
                    None,
                ),
            ),
            (
                "valueSet",
                enumerated(json!([{"valueSet": ["http://example.org/other"]}]), None),
            ),
            (
                "no system",
                enumerated(json!([{"concept": [{"code": "a"}]}]), None),
            ),
            (
                "concept without code",
                enumerated(
                    json!([{"system": "s", "concept": [{"display": "x"}]}]),
                    None,
                ),
            ),
            (
                "exclude with filter",
                enumerated(
                    json!([{"system": "s", "concept": [{"code": "a"}]}]),
                    Some(
                        json!([{"system": "s", "filter": [{"property": "p", "op": "=", "value": "v"}]}]),
                    ),
                ),
            ),
            (
                "version disagreement",
                enumerated(
                    json!([{"system": "s", "version": "1", "concept": [{"code": "a"}]},
                       {"system": "s", "concept": [{"code": "b"}]}]),
                    None,
                ),
            ),
            (
                "versioned exclude against unversioned include",
                enumerated(
                    json!([{"system": "s", "concept": [{"code": "a"}]}]),
                    Some(json!([{"system": "s", "version": "1", "concept": [{"code": "a"}]}])),
                ),
            ),
        ];
        for (name, vs) in cases {
            assert_eq!(
                expand_enumerated_compose(&vs, now(), "id", limits()).unwrap(),
                None,
                "{name} must be ineligible"
            );
            assert!(
                matches!(
                    local_expansion(&vs, now(), "id", limits()),
                    Err(TerminologyContentError::NotEnumerated { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn member_cap_and_visited_cap_apply_to_local_expansion() {
        let concepts: Vec<Value> = (0..5).map(|i| json!({"code": format!("c{i}")})).collect();
        let vs = enumerated(json!([{"system": "s", "concept": concepts}]), None);
        assert_eq!(
            expand_enumerated_compose(&vs, now(), "id", FlattenLimits::for_members(3, 1 << 20)),
            Err(TerminologyContentError::TooManyMembers { max: 3 })
        );
        // Visited cap fires inside the eligibility check, before any member
        // is produced.
        let concepts: Vec<Value> = (0..100).map(|i| json!({"code": format!("c{i}")})).collect();
        let vs = enumerated(json!([{"system": "s", "concept": concepts}]), None);
        assert_eq!(
            expand_enumerated_compose(
                &vs,
                now(),
                "id",
                FlattenLimits {
                    max_members: 1_000,
                    max_visited: 10,
                    max_relation_bytes: 1 << 20
                }
            ),
            Err(TerminologyContentError::TooManyEntries { max: 10 })
        );
    }

    // ---- size accounting -------------------------------------------------

    #[test]
    fn row_bytes_is_strings_times_factor_plus_overhead() {
        let row = json!({"system": "abcd", "code": "ef", "version": null, "inactive": true});
        assert_eq!(row_bytes(&row), 6 * ROW_STRING_FACTOR + ROW_OVERHEAD_BYTES);
        assert_eq!(row_bytes(&json!(null)), ROW_OVERHEAD_BYTES);
    }

    #[test]
    fn estimate_json_bytes_stops_at_the_cap() {
        let big = json!({"contains": (0..1000).map(|i| json!({"code": i.to_string()})).collect::<Vec<_>>()});
        assert_eq!(
            estimate_json_bytes(&big, 100),
            Err(TerminologyContentError::TooManyBytes { max: 100 })
        );
        let small = json!({"a": "b"});
        let n = estimate_json_bytes(&small, 1 << 20).unwrap();
        assert!((5..100).contains(&n), "{n}");
    }

    #[test]
    fn issue_codes_follow_the_spec_table() {
        assert_eq!(TerminologyContentError::NoExpansion.issue_code(), "invalid");
        assert_eq!(
            TerminologyContentError::TooManyMembers { max: 1 }.issue_code(),
            "too-costly"
        );
        assert_eq!(
            TerminologyContentError::TooManyBytes { max: 1 }.issue_code(),
            "too-costly"
        );
    }
}
