//! Declared output schema of a SQL-on-FHIR view run.
//!
//! A [`ViewSchema`] lists a ViewDefinition's output columns in document order,
//! the order the `helios-sof` evaluator emits them in: for each `select`
//! entry its own `column`s, then its nested `select`s, then its `unionAll`
//! branches, recursively, keeping the first declaration of each name (the
//! branches of a `unionAll` fill the same columns). The walker here is the
//! single oracle every runner derives its schema from; it mirrors
//! `helios_sof::sqlquery::TableSchema::from_view_definition`.
//!
//! # Rows follow the schema
//!
//! Every runner's [`RunOutput::rows`](super::sof_runner::RunOutput::rows) are
//! normalized: each row is a JSON object whose keys are exactly the schema's
//! column names, in schema order, with an explicit `null` for an empty cell.
//! Each runner computes a [`ColumnPermutation`] from its compiled column list
//! once per run and [normalizes](ColumnPermutation::normalize) its row stream
//! with it at the runner boundary, so a compiled list in another order (as the
//! SQL and MongoDB compilers produced for a `select` after a `unionAll` before
//! they distributed `unionAll` in place) still yields document order. The
//! compiled statement, and so the row order, stays as compiled; only the keys
//! of each row move. The identity permutation passes the stream through
//! untouched, so a run pays a per-row key reorder only when its compiled order
//! differs.
//!
//! A columnar run
//! ([`SofRunner::run_view_columnar`](super::sof_runner::SofRunner::run_view_columnar))
//! follows the same rule: its record batches' columns are moved into schema
//! order with the same permutation ([`ColumnPermutation::normalize_batches`]),
//! which only re-points the column arrays.

use std::collections::HashSet;

use futures::StreamExt as _;
use serde_json::{Map, Value};

use helios_sof::arrow::record_batch::RecordBatch;

use super::sof_runner::{BatchStream, RowStream, SofError};
use crate::sof::decode::ColumnDecode;

/// One declared output column of a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewColumn {
    /// The column name, which is also the row key.
    pub name: String,
    /// The column's declared `type`, if the ViewDefinition gives one.
    pub declared_type: Option<String>,
    /// How the runner decodes the column's text values into JSON. The SQL
    /// runners take it from their compiler (which may infer a type the view
    /// does not declare); the other runners derive it from the declaration.
    pub decode: ColumnDecode,
    /// Whether the column is declared `collection: true`.
    pub collection: bool,
}

/// The ordered output columns of a view run, in ViewDefinition document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSchema {
    /// Columns in document order, one per distinct name.
    pub columns: Vec<ViewColumn>,
}

impl ViewSchema {
    /// Walks `view`'s `select` tree in document order (see the module docs).
    ///
    /// Each column's [`decode`](ViewColumn::decode) is derived from its
    /// declaration with [`ColumnDecode::from_declared`]; a runner with a
    /// compiler-inferred decode replaces it with [`Self::with_decodes`]. A
    /// column entry without a string `name` is skipped; validation is the
    /// runner's job, not the walker's.
    pub fn from_view_definition(view: &Value) -> Self {
        let mut columns = Vec::new();
        if let Some(selects) = view.get("select").and_then(Value::as_array) {
            for select in selects {
                collect_columns(select, &mut columns);
            }
        }
        let mut seen = HashSet::new();
        columns.retain(|column: &ViewColumn| seen.insert(column.name.clone()));
        Self { columns }
    }

    /// Replaces each column's decode with the one `names`/`decodes` (parallel
    /// lists, as a compiler produces them) give for the same name. Columns the
    /// lists do not name keep their declared decode.
    pub fn with_decodes(mut self, names: &[String], decodes: &[ColumnDecode]) -> Self {
        for column in &mut self.columns {
            if let Some(at) = names.iter().position(|name| *name == column.name)
                && let Some(decode) = decodes.get(at)
            {
                column.decode = *decode;
            }
        }
        self
    }

    /// The column names, in schema order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.columns.iter().map(|column| column.name.as_str())
    }

    /// Number of columns.
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// Whether the schema has no columns.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The permutation that maps rows whose keys come in `produced` order (a
    /// runner's compiled column list) into this schema's order.
    ///
    /// Computed once per run. `produced` must name exactly the schema's
    /// columns; anything else is a compiler/schema disagreement and an
    /// internal error, never a silent reorder. A name `produced` repeats (a
    /// view that names two columns alike, which a stored view can do because
    /// it is not re-linted) counts once, at its first position: the row
    /// builders keep one JSON key per name there, holding the last such
    /// column's value. Such a permutation reorders rows only; a columnar run
    /// must decline the view first (see [`apply_batch`](ColumnPermutation::apply_batch)).
    pub fn permutation_from(&self, produced: &[String]) -> Result<ColumnPermutation, SofError> {
        let schema: Vec<&str> = self.names().collect();
        let mut produced_set: HashSet<&str> = HashSet::with_capacity(produced.len());
        let distinct: Vec<&str> = produced
            .iter()
            .map(String::as_str)
            .filter(|name| produced_set.insert(*name))
            .collect();
        let collapsed = distinct.len() != produced.len();
        if distinct == schema {
            return Ok(ColumnPermutation {
                order: None,
                collapsed,
            });
        }
        let schema_set: HashSet<&str> = schema.iter().copied().collect();
        if distinct.len() != schema.len() || produced_set != schema_set {
            return Err(SofError::backend(format!(
                "runner columns {produced:?} do not match the declared view columns {schema:?}"
            )));
        }
        let order = schema
            .iter()
            .map(|name| {
                let at = distinct
                    .iter()
                    .position(|p| p == name)
                    .expect("the sets are equal");
                ((*name).to_string(), at)
            })
            .collect();
        Ok(ColumnPermutation {
            order: Some(order),
            collapsed,
        })
    }
}

/// Maps one run's rows from the runner's column order into [`ViewSchema`]
/// order. Built once per run by [`ViewSchema::permutation_from`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnPermutation {
    /// `None`: the runner already produces schema order. `Some(order)`: each
    /// schema column's name, in schema order, with its position in the
    /// runner's column order (distinct names, first occurrences).
    order: Option<Vec<(String, usize)>>,
    /// The runner's column list repeats a name, so its record batches would
    /// not line up with [`order`](Self::order): rows only.
    collapsed: bool,
}

impl ColumnPermutation {
    /// The permutation of a runner that already produces schema order.
    pub fn identity() -> Self {
        Self {
            order: None,
            collapsed: false,
        }
    }

    /// Whether rows pass through unchanged.
    pub fn is_identity(&self) -> bool {
        self.order.is_none()
    }

    /// Normalizes a run's row stream: every `Ok` row goes through
    /// [`apply`](Self::apply), and `Err` items keep their place. The identity
    /// permutation returns `rows` itself, without a wrapping stream.
    pub fn normalize(self, rows: RowStream) -> RowStream {
        if self.is_identity() {
            return rows;
        }
        Box::pin(rows.map(move |item| item.map(|row| self.apply(row))))
    }

    /// Normalizes a columnar run's batch stream: every `Ok` batch goes
    /// through [`apply_batch`](Self::apply_batch), and `Err` items keep their
    /// place. The identity permutation returns `batches` itself.
    pub fn normalize_batches(self, batches: BatchStream) -> BatchStream {
        if self.is_identity() && !self.collapsed {
            return batches;
        }
        Box::pin(batches.map(move |item| item.and_then(|batch| self.apply_batch(batch))))
    }

    /// Reorders one record batch's columns (in the runner's compiled column
    /// order) into schema order. Only the column arrays move; their data is
    /// shared, not copied. A batch whose width is not the runner's column
    /// count, or any batch of a runner whose column list repeats a name, is an
    /// internal error.
    pub fn apply_batch(&self, batch: RecordBatch) -> Result<RecordBatch, SofError> {
        if self.collapsed {
            return Err(SofError::backend(
                "a columnar run cannot carry two output columns with the same name",
            ));
        }
        let Some(order) = &self.order else {
            return Ok(batch);
        };
        let indices: Vec<usize> = order.iter().map(|(_, at)| *at).collect();
        batch.project(&indices).map_err(|e| {
            SofError::backend(format!(
                "failed to reorder the record batch into the view's column order: {e}"
            ))
        })
    }

    /// Reorders one row's keys into schema order. A declared column the row
    /// lacks becomes an explicit JSON `null`; keys outside the schema keep
    /// their relative order after the declared ones. The identity permutation
    /// and non-object rows pass through unchanged.
    pub fn apply(&self, row: Value) -> Value {
        let Some(order) = &self.order else {
            return row;
        };
        match row {
            Value::Object(map) => {
                let mut entries: Vec<Option<(String, Value)>> = map.into_iter().map(Some).collect();
                let mut out = Map::with_capacity(order.len().max(entries.len()));
                for (name, at) in order {
                    let slot = match entries.get(*at) {
                        Some(Some((key, _))) if key == name => Some(*at),
                        _ => entries
                            .iter()
                            .position(|e| e.as_ref().is_some_and(|(key, _)| key == name)),
                    };
                    let value = slot
                        .and_then(|i| entries[i].take())
                        .map_or(Value::Null, |(_, value)| value);
                    out.insert(name.clone(), value);
                }
                out.extend(entries.into_iter().flatten());
                Value::Object(out)
            }
            other => other,
        }
    }
}

fn collect_columns(select: &Value, out: &mut Vec<ViewColumn>) {
    if let Some(columns) = select.get("column").and_then(Value::as_array) {
        for column in columns {
            let Some(name) = column.get("name").and_then(Value::as_str) else {
                continue;
            };
            let declared_type = column
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string);
            let collection = column
                .get("collection")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            out.push(ViewColumn {
                name: name.to_string(),
                decode: ColumnDecode::from_declared(declared_type.as_deref(), collection),
                declared_type,
                collection,
            });
        }
    }
    for key in ["select", "unionAll"] {
        if let Some(nested) = select.get(key).and_then(Value::as_array) {
            for select in nested {
                collect_columns(select, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A view whose `unionAll` comes before a sibling select: document order
    /// lists `active` last, a compiler that projected sibling columns first
    /// would list it second.
    fn union_first_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"name": "id", "path": "getResourceKey()"}]},
                {"unionAll": [
                    {"forEach": "name", "column": [
                        {"name": "family", "path": "family", "type": "string"},
                        {"name": "given", "path": "given", "collection": true}
                    ]},
                    {"forEach": "contact.name", "column": [
                        {"name": "family", "path": "family", "type": "code"},
                        {"name": "given", "path": "given", "collection": true}
                    ]}
                ]},
                {"column": [{"name": "active", "path": "active", "type": "boolean"}]}
            ]
        })
    }

    fn names(schema: &ViewSchema) -> Vec<&str> {
        schema.names().collect()
    }

    #[test]
    fn walker_follows_document_order_and_keeps_first_declarations() {
        let schema = ViewSchema::from_view_definition(&union_first_view());
        assert_eq!(names(&schema), ["id", "family", "given", "active"]);
        assert_eq!(
            schema.columns[1],
            ViewColumn {
                name: "family".into(),
                declared_type: Some("string".into()),
                decode: ColumnDecode::Text,
                collection: false,
            },
            "the first unionAll branch declares the column"
        );
        assert!(schema.columns[2].collection);
        assert_eq!(schema.columns[2].decode, ColumnDecode::Json);
        assert_eq!(schema.columns[0].declared_type, None);
        assert_eq!(schema.columns[0].decode, ColumnDecode::Auto);
        assert_eq!(schema.columns[3].decode, ColumnDecode::Boolean);
    }

    #[test]
    fn walker_visits_columns_then_nested_selects_then_union_branches() {
        let view = json!({
            "resource": "Patient",
            "select": [{
                "unionAll": [{"column": [{"name": "u", "path": "id"}]}],
                "select": [{"column": [{"name": "nested", "path": "id"}]}],
                "column": [{"name": "own", "path": "id"}, {"path": "unnamed"}]
            }]
        });
        let schema = ViewSchema::from_view_definition(&view);
        assert_eq!(names(&schema), ["own", "nested", "u"]);
        assert_eq!(schema.len(), 3);
        assert!(ViewSchema::from_view_definition(&json!({"resource": "Patient"})).is_empty());
    }

    #[test]
    fn compiled_decodes_replace_declared_ones_by_name() {
        let schema = ViewSchema::from_view_definition(&union_first_view()).with_decodes(
            &["active".into(), "id".into()],
            &[ColumnDecode::Boolean, ColumnDecode::Text],
        );
        assert_eq!(schema.columns[0].decode, ColumnDecode::Text);
        assert_eq!(
            schema.columns[1].decode,
            ColumnDecode::Text,
            "unnamed: kept"
        );
        assert_eq!(schema.columns[3].decode, ColumnDecode::Boolean);
    }

    #[test]
    fn the_same_order_is_the_identity_and_rows_pass_through() {
        let schema = ViewSchema::from_view_definition(&union_first_view());
        let produced: Vec<String> = schema.names().map(str::to_string).collect();
        let permutation = schema.permutation_from(&produced).expect("same columns");
        assert!(permutation.is_identity());
        assert_eq!(permutation, ColumnPermutation::identity());
        let row = json!({"id": "p1", "family": "F", "given": ["G"], "active": true});
        assert_eq!(permutation.apply(row.clone()), row);
    }

    #[test]
    fn another_order_is_permuted_into_document_order_once_per_row() {
        let schema = ViewSchema::from_view_definition(&union_first_view());
        let produced: Vec<String> = ["id", "active", "family", "given"]
            .map(str::to_string)
            .to_vec();
        let permutation = schema.permutation_from(&produced).expect("same columns");
        assert!(!permutation.is_identity());
        let row = json!({"id": "p1", "active": true, "family": null, "given": ["G"]});
        let normalized = permutation.apply(row);
        let keys: Vec<&String> = normalized.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["id", "family", "given", "active"]);
        assert_eq!(
            normalized,
            json!({"id": "p1", "family": null, "given": ["G"], "active": true})
        );

        // A row that lacks a declared key gets an explicit null; a key
        // outside the schema stays, after the declared ones.
        let partial = permutation.apply(json!({"given": [], "id": "p2", "extra": 1}));
        let keys: Vec<&String> = partial.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["id", "family", "given", "active", "extra"]);
        assert_eq!(partial["family"], Value::Null);
        assert_eq!(partial["active"], Value::Null);
        assert_eq!(permutation.apply(json!("not a row")), json!("not a row"));
    }

    #[tokio::test]
    async fn normalize_reorders_rows_and_keeps_errors_in_place() {
        let schema = ViewSchema::from_view_definition(&union_first_view());
        let produced: Vec<String> = ["id", "active", "family", "given"]
            .map(str::to_string)
            .to_vec();
        let items = vec![
            Ok(json!({"id": "p1", "active": true, "family": "F", "given": []})),
            Err(SofError::backend("mid-stream")),
            Ok(json!({"id": "p2", "active": null, "family": null, "given": ["G"]})),
        ];
        let rows: RowStream = Box::pin(futures::stream::iter(items));
        let out: Vec<_> = schema
            .permutation_from(&produced)
            .expect("same columns")
            .normalize(rows)
            .collect()
            .await;
        assert_eq!(out.len(), 3);
        for at in [0, 2] {
            let row = out[at].as_ref().expect("row");
            let keys: Vec<&String> = row.as_object().expect("object").keys().collect();
            assert_eq!(keys, ["id", "family", "given", "active"], "row {at}");
        }
        assert!(
            matches!(&out[1], Err(SofError::Backend { message, .. }) if message == "mid-stream")
        );

        // The identity permutation hands the rows over unchanged, key order
        // included.
        let row = json!({"id": "p3", "family": "F", "given": [], "active": false});
        let rows: RowStream = Box::pin(futures::stream::iter(vec![Ok(row.clone())]));
        let out: Vec<_> = ColumnPermutation::identity()
            .normalize(rows)
            .collect()
            .await;
        let got = out[0].as_ref().expect("row");
        assert_eq!(got, &row);
        assert!(
            got.as_object()
                .unwrap()
                .keys()
                .eq(row.as_object().unwrap().keys())
        );
    }

    #[test]
    fn a_column_list_that_disagrees_with_the_schema_is_an_error() {
        let schema = ViewSchema::from_view_definition(&union_first_view());
        for produced in [
            vec!["id", "family", "given"],
            vec!["id", "family", "given", "active", "extra"],
            vec!["id", "family", "given", "other"],
            vec!["id", "id", "family", "given"],
        ] {
            let produced: Vec<String> = produced.into_iter().map(str::to_string).collect();
            match schema.permutation_from(&produced) {
                Err(SofError::Backend { message, .. }) => {
                    assert!(message.contains("do not match"), "{message}")
                }
                other => panic!("{produced:?}: expected a Backend error, got {other:?}"),
            }
        }
    }

    /// A view that names two columns alike: the runner's column list repeats
    /// the name, its rows carry one key for it, and the permutation counts it
    /// once at its first position. Rows only: batches are refused.
    #[tokio::test]
    async fn a_repeated_column_name_counts_once_for_rows_and_refuses_batches() {
        let view = json!({"resource": "Patient", "select": [{"column": [
            {"name": "x", "path": "id"},
            {"name": "x", "path": "gender"},
            {"name": "dob", "path": "birthDate"}
        ]}]});
        let schema = ViewSchema::from_view_definition(&view);
        assert_eq!(names(&schema), ["x", "dob"]);

        let in_order: Vec<String> = ["x", "x", "dob"].map(str::to_string).to_vec();
        let permutation = schema.permutation_from(&in_order).expect("repeated name");
        assert!(permutation.is_identity());
        let row = json!({"x": "male", "dob": "1990-01-01"});
        assert_eq!(permutation.apply(row.clone()), row);

        let reordered: Vec<String> = ["dob", "x", "x"].map(str::to_string).to_vec();
        let permutation = schema.permutation_from(&reordered).expect("repeated name");
        assert!(!permutation.is_identity());
        let normalized = permutation.apply(json!({"dob": "1990-01-01", "x": "male"}));
        let keys: Vec<&String> = normalized.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["x", "dob"]);
        assert_eq!(normalized, json!({"x": "male", "dob": "1990-01-01"}));

        for produced in [in_order, reordered] {
            let permutation = schema.permutation_from(&produced).expect("repeated name");
            let batches: BatchStream =
                Box::pin(futures::stream::iter(vec![Ok(RecordBatch::new_empty(
                    std::sync::Arc::new(helios_sof::arrow::datatypes::Schema::empty()),
                ))]));
            let out: Vec<_> = permutation.normalize_batches(batches).collect().await;
            assert!(
                matches!(&out[..], [Err(SofError::Backend { message, .. })]
                    if message.contains("same name")),
                "{produced:?}: {out:?}"
            );
        }
    }

    /// The walker is the oracle for the evaluator's own column list.
    #[cfg(feature = "R4")]
    #[test]
    fn walker_matches_the_evaluator_column_order() {
        let views = [
            union_first_view(),
            json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [
                {"forEach": "name", "column": [{"name": "family", "path": "family"}],
                 "select": [{"column": [{"name": "use", "path": "use"}]}]},
                {"column": [{"name": "id", "path": "id"}]}
            ]}),
            json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [
                {"unionAll": [
                    {"column": [{"name": "a", "path": "id"}]},
                    {"column": [{"name": "a", "path": "gender"}]}
                ], "column": [{"name": "b", "path": "birthDate"}]}
            ]}),
        ];
        for view in views {
            let schema = ViewSchema::from_view_definition(&view);
            let parsed = helios_sof::parse_view_definition_for_version(
                view.clone(),
                helios_fhir::FhirVersion::R4,
            )
            .expect("valid view");
            let prepared = helios_sof::PreparedViewDefinition::new(parsed).expect("prepared");
            assert_eq!(
                schema.names().collect::<Vec<_>>(),
                prepared
                    .columns()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                "{view}"
            );
        }
    }
}
