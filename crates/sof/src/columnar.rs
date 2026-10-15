//! Typed columnar batch builder.
//!
//! The in-DB runners know every output column's type up front. Instead of
//! building a `serde_json::Value` per cell and re-inferring types from the
//! first rows, they append database values straight to Arrow builders through
//! [`ColumnarBatchBuilder`] and receive finished [`RecordBatch`]es.
//!
//! The type mapping matches what [`parquet_schema`](crate::parquet_schema)
//! produces today:
//!
//! | [`ColumnKind`] | Arrow     |
//! |----------------|-----------|
//! | `Text`         | `Utf8`    |
//! | `Boolean`      | `Boolean` |
//! | `Integer`      | `Int32`   |
//! | `Decimal`      | `Float64` |
//! | `Deferred`     | inferred from the first flush |
//!
//! A value that does not fit its column (unparseable text, an integer outside
//! `i32`, a non-finite float) is written as null and logged once per column.
//! `Deferred` columns buffer JSON values and fix their Arrow type on the first
//! flush with the same inference [`create_arrow_schema`] uses (the first 100
//! values), so later batches keep that type.
//!
//! Columns the ViewDefinition declares a `type` or `collection: true` for can
//! be pinned to the row path's declared mapping with
//! [`ColumnarBatchBuilder::pin_declared_types`] (`integer64` is `Int64`, a
//! collection is `List<Utf8>`), so both paths emit the same schema.
//!
//! [`create_arrow_schema`]: crate::parquet_schema::create_arrow_schema

use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, StringBuilder};
use arrow::datatypes::{DataType, Schema};
use arrow::record_batch::RecordBatch;
use serde_json::{Number, Value};
use tracing::warn;

use crate::SofError;
use crate::parquet_schema::{
    build_array_from_values, build_named_array, infer_arrow_type, resolve_arrow_schema,
};
use crate::sqlquery::engine::DeclaredColumn;

/// Rows per batch used by the in-DB runners.
///
/// This is the minimum of the row-based estimator in the buffered formatters,
/// and it guarantees the first flush sees at least the 100 rows that type
/// inference samples.
pub const COLUMNAR_BATCH_ROWS: usize = 100_000;

/// Number of leading values of a `Deferred` column used to infer its type.
const INFER_SAMPLE: usize = 100;

/// The declared kind of one output column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// String-like column, written as `Utf8`.
    Text,
    /// `boolean` column, written as `Boolean`.
    Boolean,
    /// Integer column, written as `Int32`.
    Integer,
    /// Decimal column, written as `Float64`.
    Decimal,
    /// Collection, complex or untyped column: values are buffered as JSON and
    /// the Arrow type is inferred on the first flush.
    Deferred,
}

enum ColumnBuf {
    Text(StringBuilder),
    Boolean(BooleanBuilder),
    Integer(Int32Builder),
    Decimal(Float64Builder),
    Deferred {
        values: Vec<Option<Value>>,
        data_type: Option<DataType>,
        /// `true` when the type comes from the ViewDefinition (a declared
        /// `type` or `ansi/type` tag): a value that does not fit is an error
        /// naming the column. An inferred type is only a guess from earlier
        /// data, so a misfit becomes null there.
        declared: bool,
    },
}

impl ColumnBuf {
    fn new(kind: ColumnKind) -> Self {
        match kind {
            ColumnKind::Text => Self::Text(StringBuilder::new()),
            ColumnKind::Boolean => Self::Boolean(BooleanBuilder::new()),
            ColumnKind::Integer => Self::Integer(Int32Builder::new()),
            ColumnKind::Decimal => Self::Decimal(Float64Builder::new()),
            ColumnKind::Deferred => Self::Deferred {
                values: Vec::new(),
                data_type: None,
                declared: false,
            },
        }
    }
}

/// Appends database values to typed Arrow builders, one cell at a time.
///
/// Per row, call exactly one `append_*` method for each column index in
/// `0..columns`, then [`end_row`](Self::end_row). When `end_row` returns
/// `true` the batch is full: call [`flush`](Self::flush). Call `flush` once
/// more after the last row to emit the remainder.
pub struct ColumnarBatchBuilder {
    names: Vec<String>,
    kinds: Vec<ColumnKind>,
    cols: Vec<ColumnBuf>,
    warned: Vec<bool>,
    /// Columns whose type comes from the ViewDefinition (see
    /// [`pin_declared_types`](Self::pin_declared_types)); their schema field
    /// carries the declared marker, as on the row path.
    declared_cols: Vec<bool>,
    /// The first value that did not fit a declared column's type; reported by
    /// the next [`flush`](Self::flush) (a declared column never writes null for
    /// a misfit).
    strict_error: Option<String>,
    batch_rows: usize,
    rows: usize,
    schema: Option<Arc<Schema>>,
}

impl ColumnarBatchBuilder {
    /// Creates a builder. `columns` and `kinds` are parallel and keep the
    /// SELECT-list order; duplicate names are kept as separate columns.
    /// `batch_rows` is clamped to at least 1.
    pub fn new(
        columns: Vec<String>,
        kinds: Vec<ColumnKind>,
        batch_rows: usize,
    ) -> Result<Self, SofError> {
        if columns.len() != kinds.len() {
            return Err(SofError::ArrowConversionError(format!(
                "column count ({}) does not match kind count ({})",
                columns.len(),
                kinds.len()
            )));
        }
        let kinds_len = kinds.len();
        let cols = kinds.iter().map(|k| ColumnBuf::new(*k)).collect();
        let warned = vec![false; columns.len()];
        Ok(Self {
            names: columns,
            kinds,
            cols,
            warned,
            declared_cols: vec![false; kinds_len],
            strict_error: None,
            batch_rows: batch_rows.max(1),
            rows: 0,
            schema: None,
        })
    }

    /// Gives every column the ViewDefinition declares a `type` or
    /// `collection: true` for the Arrow type the row path uses
    /// ([`resolve_arrow_schema`]), so both paths produce the same schema.
    ///
    /// A column whose kind already maps to that type is left alone. Any other
    /// one (`integer64` text, a collection, a complex type) buffers its values
    /// like `Deferred` with the type fixed up front, so it is built exactly as
    /// the row path builds it, including for a first batch of nulls. Call
    /// before the first row.
    pub fn pin_declared_types(&mut self, declared: &[DeclaredColumn]) -> Result<(), SofError> {
        let resolved = resolve_arrow_schema(&self.names, declared, &[])?;
        for (i, name) in self.names.iter().enumerate() {
            let is_declared = declared
                .iter()
                .find(|d| &d.name == name)
                .is_some_and(|d| d.fhir_type.is_some() || d.ansi.is_some() || d.collection);
            if !is_declared {
                continue;
            }
            self.declared_cols[i] = true;
            let target = resolved.field(i).data_type();
            let natural = match self.kinds[i] {
                ColumnKind::Text => Some(DataType::Utf8),
                ColumnKind::Boolean => Some(DataType::Boolean),
                ColumnKind::Integer => Some(DataType::Int32),
                ColumnKind::Decimal => Some(DataType::Float64),
                ColumnKind::Deferred => None,
            };
            if natural.as_ref() != Some(target) {
                self.cols[i] = ColumnBuf::Deferred {
                    values: Vec::new(),
                    data_type: Some(target.clone()),
                    declared: true,
                };
            }
        }
        Ok(())
    }

    /// Number of columns.
    pub fn column_count(&self) -> usize {
        self.names.len()
    }

    /// Rows appended since the last flush.
    pub fn pending_rows(&self) -> usize {
        self.rows
    }

    /// The schema shared by every batch, once known: immediately when no
    /// column is `Deferred`, otherwise after the first non-empty flush.
    pub fn schema(&self) -> Option<Arc<Schema>> {
        self.schema.clone()
    }

    /// Appends SQL NULL to column `col`.
    pub fn append_null(&mut self, col: usize) {
        match &mut self.cols[col] {
            ColumnBuf::Text(b) => b.append_null(),
            ColumnBuf::Boolean(b) => b.append_null(),
            ColumnBuf::Integer(b) => b.append_null(),
            ColumnBuf::Decimal(b) => b.append_null(),
            ColumnBuf::Deferred { values, .. } => values.push(None),
        }
    }

    /// Appends database text to column `col`.
    ///
    /// `Text` takes the string as is. `Boolean` accepts `true` / `false`.
    /// `Integer` and `Decimal` parse JSON number syntax. A `Deferred` column
    /// stores a JSON string; use [`append_deferred`](Self::append_deferred)
    /// to store a decoded value instead.
    pub fn append_str(&mut self, col: usize, s: &str) {
        let ok = match &mut self.cols[col] {
            ColumnBuf::Text(b) => {
                b.append_value(s);
                true
            }
            ColumnBuf::Boolean(b) => match s {
                "true" => {
                    b.append_value(true);
                    true
                }
                "false" => {
                    b.append_value(false);
                    true
                }
                _ => {
                    b.append_null();
                    false
                }
            },
            ColumnBuf::Integer(b) => {
                let parsed = parse_number(s)
                    .and_then(|n| n.as_i64())
                    .and_then(|i| i32::try_from(i).ok());
                b.append_option(parsed);
                parsed.is_some()
            }
            ColumnBuf::Decimal(b) => {
                let parsed = parse_number(s).and_then(|n| n.as_f64());
                b.append_option(parsed);
                parsed.is_some()
            }
            ColumnBuf::Deferred { values, .. } => {
                values.push(Some(Value::String(s.to_owned())));
                true
            }
        };
        if !ok {
            self.warn_once(col, "text");
        }
    }

    /// Appends a native integer to column `col`.
    ///
    /// `Text` writes its decimal text, `Boolean` accepts 0 and 1, `Integer`
    /// accepts the `i32` range, `Decimal` converts to `f64`.
    pub fn append_i64(&mut self, col: usize, n: i64) {
        let ok = match &mut self.cols[col] {
            ColumnBuf::Text(b) => {
                b.append_value(n.to_string());
                true
            }
            ColumnBuf::Boolean(b) => match n {
                0 | 1 => {
                    b.append_value(n == 1);
                    true
                }
                _ => {
                    b.append_null();
                    false
                }
            },
            ColumnBuf::Integer(b) => {
                let v = i32::try_from(n).ok();
                b.append_option(v);
                v.is_some()
            }
            ColumnBuf::Decimal(b) => {
                b.append_value(n as f64);
                true
            }
            ColumnBuf::Deferred { values, .. } => {
                values.push(Some(Value::from(n)));
                true
            }
        };
        if !ok {
            self.warn_once(col, "integer");
        }
    }

    /// Appends a native float to column `col`.
    ///
    /// `Text` writes the JSON number text, `Decimal` stores it. A non-finite
    /// value, or any float in a `Boolean` or `Integer` column, is written as
    /// null.
    pub fn append_f64(&mut self, col: usize, f: f64) {
        let ok = match &mut self.cols[col] {
            ColumnBuf::Text(b) => {
                let n = Number::from_f64(f);
                b.append_option(n.as_ref().map(Number::to_string));
                n.is_some()
            }
            ColumnBuf::Boolean(b) => {
                b.append_null();
                false
            }
            ColumnBuf::Integer(b) => {
                b.append_null();
                false
            }
            ColumnBuf::Decimal(b) => {
                let v = f.is_finite().then_some(f);
                b.append_option(v);
                v.is_some()
            }
            ColumnBuf::Deferred { values, .. } => {
                values.push(Number::from_f64(f).map(Value::Number));
                true
            }
        };
        if !ok {
            self.warn_once(col, "float");
        }
    }

    /// Appends a boolean to column `col`.
    ///
    /// `Text` writes `true` / `false`; `Integer` and `Decimal` write null.
    pub fn append_bool(&mut self, col: usize, v: bool) {
        let ok = match &mut self.cols[col] {
            ColumnBuf::Text(b) => {
                b.append_value(if v { "true" } else { "false" });
                true
            }
            ColumnBuf::Boolean(b) => {
                b.append_value(v);
                true
            }
            ColumnBuf::Integer(b) => {
                b.append_null();
                false
            }
            ColumnBuf::Decimal(b) => {
                b.append_null();
                false
            }
            ColumnBuf::Deferred { values, .. } => {
                values.push(Some(Value::Bool(v)));
                true
            }
        };
        if !ok {
            self.warn_once(col, "boolean");
        }
    }

    /// Appends an already decoded JSON value, for `Deferred` columns.
    ///
    /// On a typed column this dispatches to the matching `append_*` method
    /// (arrays and objects become null there), so callers never need to
    /// branch on the column kind.
    pub fn append_deferred(&mut self, col: usize, value: Value) {
        if let ColumnBuf::Deferred { values, .. } = &mut self.cols[col] {
            values.push(match value {
                Value::Null => None,
                v => Some(v),
            });
            return;
        }
        match value {
            Value::Null => self.append_null(col),
            Value::Bool(b) => self.append_bool(col, b),
            Value::String(s) => self.append_str(col, &s),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.append_i64(col, i);
                } else if let Some(f) = n.as_f64() {
                    self.append_f64(col, f);
                } else {
                    self.append_null(col);
                }
            }
            Value::Array(_) | Value::Object(_) => {
                self.append_null(col);
                self.warn_once(col, "json");
            }
        }
    }

    /// Marks the current row complete. Returns `true` when the batch is full
    /// and should be flushed.
    pub fn end_row(&mut self) -> bool {
        self.rows += 1;
        self.rows >= self.batch_rows
    }

    /// Emits the pending rows as a [`RecordBatch`], or `None` when there are
    /// none. Builders are reset for the next batch.
    pub fn flush(&mut self) -> Result<Option<RecordBatch>, SofError> {
        if let Some(message) = self.strict_error.take() {
            return Err(SofError::ParquetConversionError(message));
        }
        if self.rows == 0 {
            return Ok(None);
        }
        let mut fields = Vec::with_capacity(self.cols.len());
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(self.cols.len());
        let declared_cols = &self.declared_cols;
        for (col_idx, (name, buf)) in self.names.iter().zip(self.cols.iter_mut()).enumerate() {
            let (data_type, array): (DataType, ArrayRef) = match buf {
                ColumnBuf::Text(b) => (DataType::Utf8, Arc::new(b.finish())),
                ColumnBuf::Boolean(b) => (DataType::Boolean, Arc::new(b.finish())),
                ColumnBuf::Integer(b) => (DataType::Int32, Arc::new(b.finish())),
                ColumnBuf::Decimal(b) => (DataType::Float64, Arc::new(b.finish())),
                ColumnBuf::Deferred {
                    values,
                    data_type,
                    declared,
                } => {
                    let values = std::mem::take(values);
                    let dt = data_type
                        .get_or_insert_with(|| {
                            infer_arrow_type(&values[..values.len().min(INFER_SAMPLE)])
                        })
                        .clone();
                    let array = if *declared {
                        build_named_array(&values, &dt, name)?
                    } else {
                        build_array_from_values(values, &dt)?
                    };
                    (dt, array)
                }
            };
            fields.push(crate::parquet_schema::schema_field(
                name,
                data_type,
                declared_cols[col_idx],
            ));
            arrays.push(array);
        }
        self.rows = 0;

        let schema = self
            .schema
            .get_or_insert_with(|| Arc::new(Schema::new(fields)))
            .clone();
        RecordBatch::try_new(schema, arrays)
            .map(Some)
            .map_err(|e| SofError::ArrowConversionError(e.to_string()))
    }

    fn warn_once(&mut self, col: usize, got: &str) {
        if self.declared_cols[col] {
            if self.strict_error.is_none() {
                self.strict_error = Some(format!(
                    "column '{}': a {got} value cannot be represented in its declared type {:?}",
                    self.names[col], self.kinds[col]
                ));
            }
            return;
        }
        if !self.warned[col] {
            self.warned[col] = true;
            warn!(
                column = %self.names[col],
                kind = ?self.kinds[col],
                got,
                "columnar output: value does not fit the declared column type; written as null"
            );
        }
    }
}

fn parse_number(s: &str) -> Option<Number> {
    serde_json::from_str::<Number>(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, BooleanArray, Float64Array, Int32Array, ListArray, StringArray};
    use arrow::datatypes::Field;
    use serde_json::json;

    fn builder(kinds: &[ColumnKind], batch_rows: usize) -> ColumnarBatchBuilder {
        let names = (0..kinds.len()).map(|i| format!("c{i}")).collect();
        ColumnarBatchBuilder::new(names, kinds.to_vec(), batch_rows).unwrap()
    }

    fn declared(name: &str, fhir_type: Option<&str>, collection: bool) -> DeclaredColumn {
        DeclaredColumn {
            name: name.to_string(),
            fhir_type: fhir_type.map(crate::sqlquery::ColumnFhirType::from_code),
            collection,
            ansi: fhir_type.map(crate::column_types::default_for_fhir_type),
        }
    }

    #[test]
    fn pinned_columns_take_the_declared_row_path_types() {
        // c0 integer64 text, c1 collection, c2 complex, c3 integer, c4 undeclared.
        let mut b = builder(
            &[
                ColumnKind::Text,
                ColumnKind::Deferred,
                ColumnKind::Deferred,
                ColumnKind::Integer,
                ColumnKind::Deferred,
            ],
            10,
        );
        b.pin_declared_types(&[
            declared("c0", Some("integer64"), false),
            declared("c1", Some("string"), true),
            declared("c2", Some("CodeableConcept"), false),
            declared("c3", Some("integer"), false),
        ])
        .unwrap();
        // A first batch of nulls still gets the declared types.
        for i in 0..5 {
            b.append_null(i);
        }
        b.end_row();
        b.append_str(0, "9007199254740993");
        b.append_deferred(1, json!(["a", "b"]));
        b.append_deferred(2, json!({ "text": "x" }));
        b.append_i64(3, 7);
        b.append_i64(4, 1);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let types: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.data_type().clone())
            .collect();
        assert_eq!(
            types,
            vec![
                DataType::Int64,
                DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                DataType::Utf8,
                DataType::Int32,
                DataType::Int32,
            ]
        );
        let c0 = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert!(c0.is_null(0));
        assert_eq!(c0.value(1), 9_007_199_254_740_993);
        let c1 = batch
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        assert!(c1.is_null(0));
        assert_eq!(c1.value(1).len(), 2);
    }

    #[test]
    fn mismatched_lengths_are_rejected() {
        assert!(ColumnarBatchBuilder::new(vec!["a".into()], vec![], 10).is_err());
    }

    #[test]
    fn text_column_keeps_database_string() {
        let mut b = builder(&[ColumnKind::Text], 10);
        for s in ["44054006", "null", "true", ""] {
            b.append_str(0, s);
            b.end_row();
        }
        b.append_null(0);
        b.end_row();
        b.append_i64(0, 7);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(a.value(0), "44054006");
        assert_eq!(a.value(1), "null");
        assert_eq!(a.value(2), "true");
        assert_eq!(a.value(3), "");
        assert!(a.is_null(4));
        assert_eq!(a.value(5), "7");
    }

    #[test]
    fn boolean_column_accepts_text_and_zero_one() {
        let mut b = builder(&[ColumnKind::Boolean], 10);
        b.append_str(0, "true");
        b.end_row();
        b.append_str(0, "false");
        b.end_row();
        b.append_i64(0, 1);
        b.end_row();
        b.append_i64(0, 0);
        b.end_row();
        b.append_i64(0, 2);
        b.end_row();
        b.append_str(0, "yes");
        b.end_row();
        b.append_null(0);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        let got: Vec<Option<bool>> = a.iter().collect();
        assert_eq!(
            got,
            vec![
                Some(true),
                Some(false),
                Some(true),
                Some(false),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn integer_column_parses_and_rejects_out_of_range() {
        let mut b = builder(&[ColumnKind::Integer], 10);
        b.append_str(0, "42");
        b.end_row();
        b.append_str(0, "-7");
        b.end_row();
        b.append_str(0, "1.5");
        b.end_row();
        b.append_str(0, "abc");
        b.end_row();
        b.append_str(0, "NaN");
        b.end_row();
        b.append_i64(0, i64::from(i32::MAX) + 1);
        b.end_row();
        b.append_i64(0, 9);
        b.end_row();
        b.append_f64(0, 2.0);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        let got: Vec<Option<i32>> = a.iter().collect();
        assert_eq!(
            got,
            vec![Some(42), Some(-7), None, None, None, None, Some(9), None]
        );
    }

    #[test]
    fn decimal_column_parses_json_numbers_only() {
        let mut b = builder(&[ColumnKind::Decimal], 10);
        b.append_str(0, "1.25");
        b.end_row();
        b.append_str(0, "3");
        b.end_row();
        b.append_str(0, "NaN");
        b.end_row();
        b.append_str(0, "inf");
        b.end_row();
        b.append_i64(0, 5);
        b.end_row();
        b.append_f64(0, 0.5);
        b.end_row();
        b.append_f64(0, f64::NAN);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let got: Vec<Option<f64>> = a.iter().collect();
        assert_eq!(
            got,
            vec![
                Some(1.25),
                Some(3.0),
                None,
                None,
                Some(5.0),
                Some(0.5),
                None
            ]
        );
    }

    #[test]
    fn deferred_type_is_fixed_on_first_flush() {
        let mut b = builder(&[ColumnKind::Deferred], 2);
        b.append_deferred(0, json!(1));
        assert!(!b.end_row());
        b.append_deferred(0, json!(2));
        assert!(b.end_row());
        let first = b.flush().unwrap().unwrap();
        assert_eq!(first.schema().field(0).data_type(), &DataType::Int32);

        // A string now would vote Utf8 on its own, but the type is locked.
        b.append_deferred(0, json!("x"));
        b.end_row();
        b.append_deferred(0, json!(3));
        b.end_row();
        let second = b.flush().unwrap().unwrap();
        assert_eq!(second.schema(), first.schema());
        let a = second
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![None, Some(3)]);
    }

    #[test]
    fn deferred_column_serialises_objects_and_string_lists() {
        let mut b = builder(&[ColumnKind::Deferred, ColumnKind::Deferred], 10);
        b.append_deferred(0, json!({"a": 1}));
        b.append_deferred(1, json!(["x", "y"]));
        b.end_row();
        b.append_deferred(0, Value::Null);
        b.append_deferred(1, Value::Null);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let obj = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(obj.value(0), r#"{"a":1}"#);
        assert!(obj.is_null(1));
        let list = batch
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        assert!(list.is_valid(0));
        assert!(list.is_null(1));
        let inner = list.value(0);
        let inner = inner.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(inner.iter().collect::<Vec<_>>(), vec![Some("x"), Some("y")]);
    }

    /// Like the row path, a list of non-strings is `List<Utf8>` with each
    /// element stringified.
    #[test]
    fn deferred_non_string_list_is_a_string_list_as_in_the_row_path() {
        let mut b = builder(&[ColumnKind::Deferred], 10);
        b.append_deferred(0, json!([1, 2]));
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let list = batch
            .column(0)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let inner = list.value(0);
        let inner = inner.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(inner.iter().collect::<Vec<_>>(), vec![Some("1"), Some("2")]);
    }

    #[test]
    fn typed_column_dispatches_deferred_values() {
        let mut b = builder(&[ColumnKind::Integer, ColumnKind::Text], 10);
        b.append_deferred(0, json!(5));
        b.append_deferred(1, json!(true));
        b.end_row();
        b.append_deferred(0, json!([1]));
        b.append_deferred(1, Value::Null);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let i = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(i.iter().collect::<Vec<_>>(), vec![Some(5), None]);
        let t = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(t.iter().collect::<Vec<_>>(), vec![Some("true"), None]);
    }

    #[test]
    fn batches_split_at_batch_rows_and_share_a_schema() {
        let mut b = builder(&[ColumnKind::Text, ColumnKind::Integer], 3);
        let mut batches = Vec::new();
        for i in 0..7 {
            b.append_str(0, &i.to_string());
            b.append_i64(1, i);
            if b.end_row() {
                batches.push(b.flush().unwrap().unwrap());
            }
        }
        batches.extend(b.flush().unwrap());
        let sizes: Vec<usize> = batches.iter().map(|b| b.num_rows()).collect();
        assert_eq!(sizes, vec![3, 3, 1]);
        assert!(batches.windows(2).all(|w| w[0].schema() == w[1].schema()));
        assert_eq!(b.schema().unwrap(), batches[0].schema());
        assert!(b.flush().unwrap().is_none());
    }

    #[test]
    fn duplicate_column_names_stay_separate_columns() {
        let mut b = ColumnarBatchBuilder::new(
            vec!["x".into(), "x".into()],
            vec![ColumnKind::Text, ColumnKind::Integer],
            10,
        )
        .unwrap();
        b.append_str(0, "a");
        b.append_i64(1, 1);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        assert_eq!(batch.num_columns(), 2);
    }

    #[test]
    fn empty_builder_flushes_nothing() {
        let mut b = builder(&[ColumnKind::Text], 10);
        assert!(b.flush().unwrap().is_none());
        assert!(b.schema().is_none());
    }

    const ALL_KINDS: [ColumnKind; 5] = [
        ColumnKind::Text,
        ColumnKind::Boolean,
        ColumnKind::Integer,
        ColumnKind::Decimal,
        ColumnKind::Deferred,
    ];

    #[test]
    fn counters_track_columns_and_pending_rows() {
        let mut b = builder(&ALL_KINDS, 10);
        assert_eq!(b.column_count(), 5);
        assert_eq!(b.pending_rows(), 0);
        for c in 0..5 {
            b.append_null(c);
        }
        assert!(!b.end_row());
        assert_eq!(b.pending_rows(), 1);
    }

    #[test]
    fn every_value_kind_is_accepted_by_every_column_kind() {
        // One row per value kind, each appended to all five column kinds. A
        // value that does not fit a typed column becomes null; none may panic
        // and every row must flush.
        let mut b = builder(&ALL_KINDS, 100);
        for c in 0..5 {
            b.append_str(c, "7");
        }
        b.end_row();
        for c in 0..5 {
            b.append_i64(c, 7);
        }
        b.end_row();
        for c in 0..5 {
            b.append_f64(c, 1.5);
        }
        b.end_row();
        for c in 0..5 {
            b.append_f64(c, f64::NAN);
        }
        b.end_row();
        for c in 0..5 {
            b.append_bool(c, true);
        }
        b.end_row();
        for c in 0..5 {
            b.append_deferred(c, json!("x"));
        }
        b.end_row();
        for c in 0..5 {
            b.append_deferred(c, json!(3));
        }
        b.end_row();
        for c in 0..5 {
            b.append_deferred(c, json!(2.5));
        }
        b.end_row();
        for c in 0..5 {
            b.append_deferred(c, json!({"a": 1}));
        }
        b.end_row();
        for c in 0..5 {
            b.append_deferred(c, Value::Null);
        }
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 10);
        assert_eq!(batch.num_columns(), 5);
    }

    #[test]
    fn decimal_text_and_boolean_columns_map_numbers_and_bools() {
        let mut b = builder(&[ColumnKind::Decimal, ColumnKind::Boolean], 10);
        b.append_f64(0, 2.5);
        b.append_bool(1, true);
        b.end_row();
        b.append_i64(0, 3);
        b.append_i64(1, 0);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let d = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!((d.value(0), d.value(1)), (2.5, 3.0));
        let t = batch
            .column(1)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        assert_eq!((t.value(0), t.value(1)), (true, false));
    }

    #[test]
    fn declared_column_misfit_is_an_error_but_undeclared_is_null() {
        // Declared `integer` on an Integer-kind column: its natural type
        // already matches, but a value outside the i32 range is an error.
        let mut b = builder(&[ColumnKind::Integer, ColumnKind::Integer], 10);
        b.pin_declared_types(&[declared("c0", Some("integer"), false)])
            .unwrap();
        b.append_i64(0, 3_000_000_000);
        b.append_i64(1, 3_000_000_000);
        b.end_row();
        let err = b.flush().unwrap_err().to_string();
        assert!(err.contains("column 'c0'"), "{err}");

        // The undeclared column only warns and writes null.
        let mut b = builder(&[ColumnKind::Integer], 10);
        b.append_i64(0, 3_000_000_000);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        assert_eq!(batch.column(0).null_count(), 1);
    }
}
