//! Terminology relations for SQL queries (SQL on FHIR "Terminology in SQL",
//! HL7/sql-on-fhir#407).
//!
//! A SQLQuery or SQLView Library may declare a `ValueSet` or `ConceptMap` as a
//! `relatedArtifact[type=depends-on]`. The runner exposes each one under its
//! `label` as a table with a fixed set of columns, so code membership and code
//! translation become ordinary SQL joins:
//!
//! - **Value set relation** ([`VALUE_SET_COLUMNS`]): one row per
//!   `expansion.contains` entry, at any depth, whose `abstract` is not `true`,
//!   unique on (`system`, `version`, `code`).
//! - **Concept map relation** ([`CONCEPT_MAP_COLUMNS`]): one row per
//!   `group.element.target`, plus one row per `group.element` with
//!   `noMap = true`. Maps that carry `dependsOn`, `product` or `valueSet`, or a
//!   group without a `source`, are rejected.
//!
//! This module only turns terminology JSON into rows. Fetching the expansion
//! or the map (from the terminology server) and persisting the rows are the
//! caller's concern.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::engine::{ColumnFhirType, ColumnSchema, TableSchema};

/// Columns of the value set relation, in table order, with their FHIR types.
pub const VALUE_SET_COLUMNS: [(&str, &str); 5] = [
    ("system", "uri"),
    ("version", "string"),
    ("code", "code"),
    ("display", "string"),
    ("inactive", "boolean"),
];

/// Columns of the concept map relation, in table order, with their FHIR types.
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

/// Which terminology artifact a relation was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminologyKind {
    ValueSet,
    ConceptMap,
}

impl TerminologyKind {
    /// The FHIR resource type, also the stable string a store persists.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ValueSet => "ValueSet",
            Self::ConceptMap => "ConceptMap",
        }
    }

    /// Parses [`Self::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ValueSet" => Some(Self::ValueSet),
            "ConceptMap" => Some(Self::ConceptMap),
            _ => None,
        }
    }

    /// The relation's column names and FHIR types, in table order.
    pub fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::ValueSet => &VALUE_SET_COLUMNS,
            Self::ConceptMap => &CONCEPT_MAP_COLUMNS,
        }
    }

    /// The relation's table schema.
    pub fn table_schema(self) -> TableSchema {
        TableSchema {
            columns: self
                .columns()
                .iter()
                .map(|(name, ty)| ColumnSchema {
                    name: (*name).to_string(),
                    fhir_type: ColumnFhirType::from_code(ty),
                    ansi: None,
                })
                .collect(),
        }
    }
}

/// A flattened terminology relation: rows are JSON objects keyed by the
/// column names of [`TerminologyKind::columns`], the shape
/// [`super::InMemorySqlEngine::insert_rows`] consumes.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminologyRelation {
    pub kind: TerminologyKind,
    pub rows: Vec<Value>,
}

/// A terminology artifact the relation cannot represent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TerminologyFlattenError {
    /// The artifact uses a feature the relation does not support (a concept
    /// map `dependsOn`, `product` or `valueSet`, or a group with no `source`).
    #[error("{0}")]
    Unsupported(String),
    /// The artifact is not shaped as expected (e.g. a ValueSet without an
    /// `expansion`).
    #[error("{0}")]
    Malformed(String),
}

/// Accumulates value set rows across one or more expansion pages, deduping on
/// (`system`, `version`, `code`) and keeping the first occurrence.
#[derive(Debug, Default)]
pub struct ValueSetRows {
    seen: HashSet<(String, Option<String>, String)>,
    rows: Vec<Value>,
    /// `contains` entries read, at every depth, abstract ones included.
    entries: usize,
}

impl ValueSetRows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds every row of one expanded ValueSet (one page of an `$expand`).
    /// Returns how many top-level `expansion.contains` entries the page had,
    /// which is what an `offset` pages over.
    pub fn add_page(&mut self, value_set: &Value) -> Result<usize, TerminologyFlattenError> {
        let expansion = value_set.get("expansion").ok_or_else(|| {
            TerminologyFlattenError::Malformed("the ValueSet has no expansion".to_string())
        })?;
        let top = expansion
            .get("contains")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        // Explicit stack: an expansion's nesting depth is the server's choice.
        let mut stack: Vec<&Value> = top.iter().rev().collect();
        while let Some(entry) = stack.pop() {
            self.entries += 1;
            if let Some(children) = entry.get("contains").and_then(Value::as_array) {
                stack.extend(children.iter().rev());
            }
            if entry.get("abstract").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let (Some(system), Some(code)) = (str_field(entry, "system"), str_field(entry, "code"))
            else {
                // `system` and `code` are never null in the relation; an entry
                // missing either is a grouping header, not a member.
                continue;
            };
            let version = str_field(entry, "version");
            let key = (
                system.to_string(),
                version.map(str::to_string),
                code.to_string(),
            );
            if !self.seen.insert(key) {
                continue;
            }
            let mut row = Map::new();
            row.insert("system".into(), Value::from(system));
            row.insert("version".into(), opt_str(version));
            row.insert("code".into(), Value::from(code));
            row.insert("display".into(), opt_str(str_field(entry, "display")));
            row.insert(
                "inactive".into(),
                entry
                    .get("inactive")
                    .and_then(Value::as_bool)
                    .map_or(Value::Null, Value::Bool),
            );
            self.rows.push(Value::Object(row));
        }
        Ok(top.len())
    }

    /// Rows collected so far.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `contains` entries read so far, at every depth.
    pub fn entries(&self) -> usize {
        self.entries
    }

    pub fn finish(self) -> TerminologyRelation {
        TerminologyRelation {
            kind: TerminologyKind::ValueSet,
            rows: self.rows,
        }
    }
}

/// Flattens a single expanded ValueSet into the value set relation.
pub fn value_set_relation(
    value_set: &Value,
) -> Result<TerminologyRelation, TerminologyFlattenError> {
    let mut rows = ValueSetRows::new();
    rows.add_page(value_set)?;
    Ok(rows.finish())
}

/// Flattens a ConceptMap (R4 or R5 shape) into the concept map relation.
///
/// R5 carries `group.source`/`group.target` as `url|version` canonicals and
/// `target.relationship`; R4 carries `source`/`sourceVersion`,
/// `target`/`targetVersion` and `target.equivalence`, which is mapped onto the
/// R5 relationship codes. An R4 target with `equivalence = unmatched` and no
/// code is R4's way of saying "no map", so it becomes a noMap row.
pub fn concept_map_relation(
    concept_map: &Value,
) -> Result<TerminologyRelation, TerminologyFlattenError> {
    let map_name = str_field(concept_map, "url").unwrap_or("<ConceptMap>");
    let mut rows = Vec::new();
    let groups = concept_map
        .get("group")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (gi, group) in groups.iter().enumerate() {
        let (source_system, source_version) = split_canonical(
            str_field(group, "source"),
            str_field(group, "sourceVersion"),
        );
        let Some(source_system) = source_system else {
            return Err(TerminologyFlattenError::Unsupported(format!(
                "ConceptMap '{map_name}' group[{gi}] has no source; groups without a source \
                 system are not supported"
            )));
        };
        let (target_system, target_version) = split_canonical(
            str_field(group, "target"),
            str_field(group, "targetVersion"),
        );
        let elements = group
            .get("element")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for (ei, element) in elements.iter().enumerate() {
            let at = format!("group[{gi}].element[{ei}]");
            if element.get("valueSet").is_some() {
                return Err(unsupported(map_name, &at, "valueSet"));
            }
            let base = |target_code: Option<&str>,
                        target_display: Option<&str>,
                        relationship: Option<&str>| {
                let mut row = Map::new();
                row.insert("source_system".into(), Value::from(source_system));
                row.insert("source_version".into(), opt_str(source_version));
                row.insert("source_code".into(), opt_str(str_field(element, "code")));
                row.insert(
                    "source_display".into(),
                    opt_str(str_field(element, "display")),
                );
                row.insert("target_system".into(), opt_str(target_system));
                row.insert("target_version".into(), opt_str(target_version));
                row.insert("target_code".into(), opt_str(target_code));
                row.insert("target_display".into(), opt_str(target_display));
                row.insert("relationship".into(), opt_str(relationship));
                Value::Object(row)
            };
            if element.get("noMap").and_then(Value::as_bool) == Some(true) {
                rows.push(base(None, None, None));
            }
            let targets = element
                .get("target")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for (ti, target) in targets.iter().enumerate() {
                let at = format!("{at}.target[{ti}]");
                for feature in ["dependsOn", "product", "valueSet"] {
                    if target.get(feature).is_some() {
                        return Err(unsupported(map_name, &at, feature));
                    }
                }
                let code = str_field(target, "code");
                let relationship = match str_field(target, "relationship") {
                    Some(r) => Some(r),
                    None => match str_field(target, "equivalence") {
                        Some("unmatched") if code.is_none() => {
                            rows.push(base(None, None, None));
                            continue;
                        }
                        Some(eq) => Some(r4_equivalence_to_relationship(eq)),
                        None => None,
                    },
                };
                rows.push(base(code, str_field(target, "display"), relationship));
            }
        }
    }
    Ok(TerminologyRelation {
        kind: TerminologyKind::ConceptMap,
        rows,
    })
}

/// Maps an R4 `ConceptMapEquivalence` onto the R5 `ConceptMapRelationship`
/// the spec's `relationship` column carries. R4's codes read from the target's
/// side ("the target is wider"), R5's from the source's.
fn r4_equivalence_to_relationship(equivalence: &str) -> &'static str {
    match equivalence {
        "equivalent" | "equal" => "equivalent",
        "wider" | "subsumes" => "source-is-narrower-than-target",
        "narrower" | "specializes" => "source-is-broader-than-target",
        "unmatched" | "disjoint" => "not-related-to",
        // relatedto, inexact, and anything unrecognised.
        _ => "related-to",
    }
}

fn unsupported(map_name: &str, at: &str, feature: &str) -> TerminologyFlattenError {
    TerminologyFlattenError::Unsupported(format!(
        "ConceptMap '{map_name}' {at} uses '{feature}', which a SQL concept map relation \
         cannot represent"
    ))
}

/// Splits an R5 `url|version` canonical; an explicit R4 version field wins
/// over nothing but never over a pipe version.
fn split_canonical<'a>(
    canonical: Option<&'a str>,
    explicit_version: Option<&'a str>,
) -> (Option<&'a str>, Option<&'a str>) {
    match canonical {
        None => (None, explicit_version),
        Some(c) => match c.split_once('|') {
            Some((url, version)) => (Some(url), Some(version)),
            None => (Some(c), explicit_version),
        },
    }
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn opt_str(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn value_set_rows_walk_nested_skip_abstract_and_dedupe() {
        let vs = json!({
            "resourceType": "ValueSet",
            "expansion": {"contains": [
                {"system": "http://s", "code": "parent", "abstract": true, "contains": [
                    {"system": "http://s", "version": "1", "code": "a", "display": "A"},
                    {"system": "http://s", "code": "b", "inactive": true}
                ]},
                {"system": "http://s", "version": "1", "code": "a", "display": "dup"},
                {"code": "no-system"}
            ]}
        });
        let rel = value_set_relation(&vs).unwrap();
        assert_eq!(rel.kind, TerminologyKind::ValueSet);
        assert_eq!(
            rel.rows,
            vec![
                json!({"system": "http://s", "version": "1", "code": "a", "display": "A", "inactive": null}),
                json!({"system": "http://s", "version": null, "code": "b", "display": null, "inactive": true}),
            ]
        );
    }

    #[test]
    fn value_set_without_expansion_is_malformed() {
        let err = value_set_relation(&json!({"resourceType": "ValueSet"})).unwrap_err();
        assert!(matches!(err, TerminologyFlattenError::Malformed(_)));
    }

    #[test]
    fn value_set_pages_dedupe_across_pages() {
        let page = |codes: &[&str]| {
            json!({"expansion": {"contains": codes.iter()
                .map(|c| json!({"system": "http://s", "code": c})).collect::<Vec<_>>()}})
        };
        let mut rows = ValueSetRows::new();
        assert_eq!(rows.add_page(&page(&["a", "b"])).unwrap(), 2);
        assert_eq!(rows.add_page(&page(&["b", "c"])).unwrap(), 2);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn r5_concept_map_splits_versions_and_emits_no_map_rows() {
        let cm = json!({
            "resourceType": "ConceptMap",
            "url": "http://example.org/cm",
            "group": [{
                "source": "http://snomed.info/sct|2024",
                "target": "http://hl7.org/fhir/sid/icd-10",
                "element": [
                    {"code": "1", "display": "One", "target": [
                        {"code": "X1", "display": "Ex one", "relationship": "equivalent"}
                    ]},
                    {"code": "2", "noMap": true}
                ]
            }]
        });
        let rel = concept_map_relation(&cm).unwrap();
        assert_eq!(
            rel.rows,
            vec![
                json!({"source_system": "http://snomed.info/sct", "source_version": "2024",
                       "source_code": "1", "source_display": "One",
                       "target_system": "http://hl7.org/fhir/sid/icd-10", "target_version": null,
                       "target_code": "X1", "target_display": "Ex one", "relationship": "equivalent"}),
                json!({"source_system": "http://snomed.info/sct", "source_version": "2024",
                       "source_code": "2", "source_display": null,
                       "target_system": "http://hl7.org/fhir/sid/icd-10", "target_version": null,
                       "target_code": null, "target_display": null, "relationship": null}),
            ]
        );
    }

    #[test]
    fn r4_concept_map_maps_equivalence_and_unmatched() {
        let cm = json!({
            "resourceType": "ConceptMap",
            "group": [{
                "source": "http://a", "sourceVersion": "3", "target": "http://b",
                "element": [
                    {"code": "1", "target": [{"code": "T", "equivalence": "wider"}]},
                    {"code": "2", "target": [{"equivalence": "unmatched"}]}
                ]
            }]
        });
        let rel = concept_map_relation(&cm).unwrap();
        assert_eq!(
            rel.rows[0]["relationship"],
            "source-is-narrower-than-target"
        );
        assert_eq!(rel.rows[0]["source_version"], "3");
        assert_eq!(rel.rows[1]["target_code"], Value::Null);
        assert_eq!(rel.rows[1]["relationship"], Value::Null);
    }

    #[test]
    fn concept_map_rejects_unsupported_features() {
        for (cm, needle) in [
            (
                json!({"group": [{"element": [{"code": "1"}]}]}),
                "has no source",
            ),
            (
                json!({"group": [{"source": "http://a", "element": [{"code": "1",
                    "target": [{"code": "x", "dependsOn": [{}]}]}]}]}),
                "'dependsOn'",
            ),
            (
                json!({"group": [{"source": "http://a", "element": [{"code": "1",
                    "target": [{"code": "x", "product": [{}]}]}]}]}),
                "'product'",
            ),
            (
                json!({"group": [{"source": "http://a", "element": [{"code": "1",
                    "valueSet": "http://vs"}]}]}),
                "'valueSet'",
            ),
        ] {
            let err = concept_map_relation(&cm).unwrap_err();
            assert!(matches!(err, TerminologyFlattenError::Unsupported(_)));
            assert!(err.to_string().contains(needle), "{err}");
        }
    }

    #[test]
    fn kind_round_trips_and_has_spec_columns() {
        for kind in [TerminologyKind::ValueSet, TerminologyKind::ConceptMap] {
            assert_eq!(TerminologyKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(TerminologyKind::ValueSet.table_schema().columns.len(), 5);
        assert_eq!(TerminologyKind::ConceptMap.table_schema().columns.len(), 9);
    }
}
