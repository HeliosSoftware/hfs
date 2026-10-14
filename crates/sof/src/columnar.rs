//! Columnar batch builder for the in-DB runners.
//!
//! The in-DB runners know how each output column is decoded from the database
//! ([`ColumnKind`]). Instead of building a `serde_json::Value` per cell, they
//! append database values straight to Arrow builders through
//! [`ColumnarBatchBuilder`] and receive finished [`RecordBatch`]es.
//!
//! The result is identical to the row path: the schema is what
//! [`resolve_arrow_schema`](crate::parquet_schema::resolve_arrow_schema)
//! decides (declared types win; every other column is inferred from the first
//! [`SCHEMA_SAMPLE_ROWS`] values) and every array equals
//! `build_array_from_values` over the JSON values the row path would build.
//!
//! * A column whose target type is declared and compatible with its kind is
//!   *typed* from the start: the caller appends natively (`append_str`,
//!   `append_i64`, ...) and no JSON is built.
//! * Every other column is *buffered* as JSON for the first batch
//!   ([`needs_value`](ColumnarBatchBuilder::needs_value) is `true`; the caller
//!   supplies the cell through [`append_deferred`](ColumnarBatchBuilder::append_deferred)).
//!   The first flush fixes the schema; afterwards an undeclared column whose
//!   inferred type is compatible with its kind is promoted to a typed builder.
//!
//! Because the first flush samples the buffered rows, `batch_rows` should be at
//! least [`SCHEMA_SAMPLE_ROWS`] for the same inference as the row path.
//!
//! A value that does not fit its typed column (unparseable text, an integer
//! outside `i32`, a non-finite float, a JSON array) is written as null and
//! logged once per column.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use serde_json::{Number, Value};
use tracing::warn;

use crate::SofError;
use crate::parquet_schema::{
    SCHEMA_SAMPLE_ROWS, build_array_from_values, infer_arrow_type, parse_bool_text, parse_f64_text,
    parse_i64_text,
};

/// How the runner decodes one output column from the database.
///
/// This is the *decode category*; it does not decide the Arrow schema (see the
/// module docs), only which columns can use a typed builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// String-like column (database text).
    Text,
    /// Native boolean column.
    Boolean,
    /// Native integer column.
    Integer,
    /// Native floating point column.
    Decimal,
    /// Collection, complex or untyped column: always buffered as JSON.
    Deferred,
}

/// Whether a column of `kind` can feed a typed builder for `target`.
fn eligible(kind: ColumnKind, target: &DataType) -> bool {
    use ColumnKind::*;
    match target {
        DataType::Utf8 => kind == Text,
        DataType::Boolean => matches!(kind, Text | Boolean),
        DataType::Int32 | DataType::Int64 => matches!(kind, Text | Integer),
        DataType::Float64 => matches!(kind, Text | Decimal | Integer),
        _ => false,
    }
}

/// A typed Arrow builder. Every append returns `true` when the value fit and
/// `false` when it was written as null because it did not.
enum Typed {
    Utf8(StringBuilder),
    Bool(BooleanBuilder),
    I32(Int32Builder),
    I64(Int64Builder),
    F64(Float64Builder),
}

impl Typed {
    fn new(target: &DataType) -> Option<Self> {
        Some(match target {
            DataType::Utf8 => Self::Utf8(StringBuilder::new()),
            DataType::Boolean => Self::Bool(BooleanBuilder::new()),
            DataType::Int32 => Self::I32(Int32Builder::new()),
            DataType::Int64 => Self::I64(Int64Builder::new()),
            DataType::Float64 => Self::F64(Float64Builder::new()),
            _ => return None,
        })
    }

    fn append_null(&mut self) {
        match self {
            Self::Utf8(b) => b.append_null(),
            Self::Bool(b) => b.append_null(),
            Self::I32(b) => b.append_null(),
            Self::I64(b) => b.append_null(),
            Self::F64(b) => b.append_null(),
        }
    }

    /// Same as `build_array_from_values` over `Value::String(s)`.
    fn append_str(&mut self, s: &str) -> bool {
        match self {
            Self::Utf8(b) => {
                b.append_value(s);
                true
            }
            Self::Bool(b) => {
                let v = parse_bool_text(s);
                b.append_option(v);
                v.is_some()
            }
            Self::I32(b) => {
                let v = parse_i64_text(s).and_then(|i| i32::try_from(i).ok());
                b.append_option(v);
                v.is_some()
            }
            Self::I64(b) => {
                let v = parse_i64_text(s);
                b.append_option(v);
                v.is_some()
            }
            Self::F64(b) => {
                let v = parse_f64_text(s);
                b.append_option(v);
                v.is_some()
            }
        }
    }

    /// Same as `build_array_from_values` over a JSON integer.
    fn append_i64(&mut self, n: i64) -> bool {
        match self {
            Self::Utf8(b) => {
                b.append_value(n.to_string());
                true
            }
            Self::Bool(b) => {
                b.append_null();
                false
            }
            Self::I32(b) => {
                let v = i32::try_from(n).ok();
                b.append_option(v);
                v.is_some()
            }
            Self::I64(b) => {
                b.append_value(n);
                true
            }
            Self::F64(b) => {
                b.append_value(n as f64);
                true
            }
        }
    }

    /// Same as `build_array_from_values` over `Number::from_f64(f)`.
    fn append_f64(&mut self, f: f64) -> bool {
        match self {
            Self::Utf8(b) => {
                let n = Number::from_f64(f);
                b.append_option(n.as_ref().map(Number::to_string));
                n.is_some()
            }
            Self::F64(b) => {
                let v = f.is_finite().then_some(f);
                b.append_option(v);
                v.is_some()
            }
            Self::Bool(b) => {
                b.append_null();
                false
            }
            Self::I32(b) => {
                b.append_null();
                false
            }
            Self::I64(b) => {
                b.append_null();
                false
            }
        }
    }

    /// Same as `build_array_from_values` over `Value::Bool(v)`.
    fn append_bool(&mut self, v: bool) -> bool {
        match self {
            Self::Utf8(b) => {
                b.append_value(v.to_string());
                true
            }
            Self::Bool(b) => {
                b.append_value(v);
                true
            }
            Self::I32(b) => {
                b.append_null();
                false
            }
            Self::I64(b) => {
                b.append_null();
                false
            }
            Self::F64(b) => {
                b.append_null();
                false
            }
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Utf8(b) => Arc::new(b.finish()),
            Self::Bool(b) => Arc::new(b.finish()),
            Self::I32(b) => Arc::new(b.finish()),
            Self::I64(b) => Arc::new(b.finish()),
            Self::F64(b) => Arc::new(b.finish()),
        }
    }
}

enum Buf {
    Typed(Typed),
    Values(Vec<Option<Value>>),
}

struct Column {
    name: String,
    kind: ColumnKind,
    /// The fixed Arrow type: the declared one, else set at the first flush.
    target: Option<DataType>,
    buf: Buf,
    warned: bool,
}

impl Column {
    fn warn_once(&mut self, got: &str) {
        if !self.warned {
            self.warned = true;
            warn!(
                column = %self.name,
                kind = ?self.kind,
                target = ?self.target,
                got,
                "columnar output: value does not fit the column type; written as null"
            );
        }
    }
}

/// Appends database values to Arrow builders, one cell at a time.
///
/// Per row, call exactly one `append_*` method for each column index in
/// `0..column_count()`, then [`end_row`](Self::end_row). When `end_row`
/// returns `true` the batch is full: call [`flush`](Self::flush). Call `flush`
/// once more after the last row to emit the remainder.
pub struct ColumnarBatchBuilder {
    cols: Vec<Column>,
    batch_rows: usize,
    rows: usize,
    schema: Option<Arc<Schema>>,
}

impl ColumnarBatchBuilder {
    /// Creates a builder. `columns`, `kinds` and `declared` are parallel and
    /// keep the SELECT-list order (duplicate names stay separate columns).
    /// `declared[i]` is the Arrow type fixed by the declaration (see
    /// [`declared_arrow_types`](crate::parquet_schema::declared_arrow_types)),
    /// or `None` to infer it at the first flush. `batch_rows` is clamped to at
    /// least 1.
    pub fn new(
        columns: Vec<String>,
        kinds: Vec<ColumnKind>,
        declared: Vec<Option<DataType>>,
        batch_rows: usize,
    ) -> Result<Self, SofError> {
        if columns.len() != kinds.len() || columns.len() != declared.len() {
            return Err(SofError::ArrowConversionError(format!(
                "column count ({}) does not match kind count ({}) / declared count ({})",
                columns.len(),
                kinds.len(),
                declared.len()
            )));
        }
        let cols = columns
            .into_iter()
            .zip(kinds)
            .zip(declared)
            .map(|((name, kind), target)| {
                let typed = target
                    .as_ref()
                    .filter(|t| eligible(kind, t))
                    .and_then(Typed::new);
                Column {
                    name,
                    kind,
                    buf: match typed {
                        Some(t) => Buf::Typed(t),
                        None => Buf::Values(Vec::new()),
                    },
                    target,
                    warned: false,
                }
            })
            .collect();
        Ok(Self {
            cols,
            batch_rows: batch_rows.max(1),
            rows: 0,
            schema: None,
        })
    }

    /// Number of columns.
    pub fn column_count(&self) -> usize {
        self.cols.len()
    }

    /// Rows appended since the last flush.
    pub fn pending_rows(&self) -> usize {
        self.rows
    }

    /// The schema shared by every batch: `Some` after the first flush.
    pub fn schema(&self) -> Option<Arc<Schema>> {
        self.schema.clone()
    }

    /// `true` while column `col` is buffered as JSON: the caller must supply
    /// its cell as a `serde_json::Value` through
    /// [`append_deferred`](Self::append_deferred), built as the row path builds
    /// it. `false` when the column has a typed builder and the caller may use
    /// `append_str` / `append_i64` / `append_f64` / `append_bool`.
    pub fn needs_value(&self, col: usize) -> bool {
        matches!(self.cols[col].buf, Buf::Values(_))
    }

    /// Appends SQL NULL to column `col`.
    pub fn append_null(&mut self, col: usize) {
        match &mut self.cols[col].buf {
            Buf::Typed(t) => t.append_null(),
            Buf::Values(v) => v.push(None),
        }
    }

    /// Appends database text, equal to appending `Value::String(s)`.
    pub fn append_str(&mut self, col: usize, s: &str) {
        let c = &mut self.cols[col];
        let ok = match &mut c.buf {
            Buf::Typed(t) => t.append_str(s),
            Buf::Values(v) => {
                v.push(Some(Value::String(s.to_owned())));
                true
            }
        };
        if !ok {
            c.warn_once("text");
        }
    }

    /// Appends a native integer, equal to appending a JSON integer.
    pub fn append_i64(&mut self, col: usize, n: i64) {
        let c = &mut self.cols[col];
        let ok = match &mut c.buf {
            Buf::Typed(t) => t.append_i64(n),
            Buf::Values(v) => {
                v.push(Some(Value::from(n)));
                true
            }
        };
        if !ok {
            c.warn_once("integer");
        }
    }

    /// Appends a native float, equal to appending `Number::from_f64(f)` (a
    /// non-finite value is null).
    pub fn append_f64(&mut self, col: usize, f: f64) {
        let c = &mut self.cols[col];
        let ok = match &mut c.buf {
            Buf::Typed(t) => t.append_f64(f),
            Buf::Values(v) => {
                v.push(Number::from_f64(f).map(Value::Number));
                true
            }
        };
        if !ok {
            c.warn_once("float");
        }
    }

    /// Appends a native boolean, equal to appending `Value::Bool(v)`.
    pub fn append_bool(&mut self, col: usize, v: bool) {
        let c = &mut self.cols[col];
        let ok = match &mut c.buf {
            Buf::Typed(t) => t.append_bool(v),
            Buf::Values(values) => {
                values.push(Some(Value::Bool(v)));
                true
            }
        };
        if !ok {
            c.warn_once("boolean");
        }
    }

    /// Appends an already decoded JSON value to any column.
    ///
    /// A buffered column stores it. A typed column dispatches to the matching
    /// typed append; arrays and objects become null there (with a warning).
    pub fn append_deferred(&mut self, col: usize, value: Value) {
        if let Buf::Values(values) = &mut self.cols[col].buf {
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
                } else if matches!(self.cols[col].buf, Buf::Typed(Typed::Utf8(_))) {
                    // u64 above i64::MAX keeps its exact digits.
                    if let Buf::Typed(Typed::Utf8(b)) = &mut self.cols[col].buf {
                        b.append_value(n.to_string());
                    }
                } else if let Some(f) = n.as_f64() {
                    self.append_f64(col, f);
                } else {
                    self.append_null(col);
                }
            }
            Value::Array(_) | Value::Object(_) => {
                self.append_null(col);
                self.cols[col].warn_once("json");
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
    ///
    /// The first flush fixes the schema (inferring every undeclared column from
    /// its first [`SCHEMA_SAMPLE_ROWS`] values) and promotes eligible
    /// undeclared columns to typed builders for the following batches.
    pub fn flush(&mut self) -> Result<Option<RecordBatch>, SofError> {
        if self.rows == 0 {
            return Ok(None);
        }
        let mut fields = Vec::with_capacity(self.cols.len());
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(self.cols.len());
        for c in &mut self.cols {
            let array = match &mut c.buf {
                Buf::Typed(t) => t.finish(),
                Buf::Values(values) => {
                    let values = std::mem::take(values);
                    if c.target.is_none() {
                        let sample = values.len().min(SCHEMA_SAMPLE_ROWS);
                        c.target = Some(infer_arrow_type(&values[..sample]));
                    }
                    let target = c.target.as_ref().expect("target fixed above");
                    build_array_from_values(values, target)?
                }
            };
            fields.push(Field::new(&c.name, array.data_type().clone(), true));
            arrays.push(array);
        }
        self.rows = 0;

        // Later batches keep the types fixed now; promote what can go typed.
        for c in &mut self.cols {
            if let (Buf::Values(_), Some(t)) = (&c.buf, &c.target)
                && eligible(c.kind, t)
                && let Some(typed) = Typed::new(t)
            {
                c.buf = Buf::Typed(typed);
            }
        }

        let schema = self
            .schema
            .get_or_insert_with(|| Arc::new(Schema::new(fields)))
            .clone();
        RecordBatch::try_new(schema, arrays)
            .map(Some)
            .map_err(|e| SofError::ArrowConversionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessedRow;
    use crate::parquet_schema::{declared_arrow_types, resolve_arrow_schema};
    use crate::sqlquery::ColumnFhirType;
    use crate::sqlquery::engine::DeclaredColumn;
    use arrow::array::{Array, BooleanArray, Float64Array, Int32Array, ListArray, StringArray};
    use serde_json::json;

    const ALL_KINDS: [ColumnKind; 5] = [
        ColumnKind::Text,
        ColumnKind::Boolean,
        ColumnKind::Integer,
        ColumnKind::Decimal,
        ColumnKind::Deferred,
    ];

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("c{i}")).collect()
    }

    fn builder(
        kinds: &[ColumnKind],
        declared: &[Option<DataType>],
        batch_rows: usize,
    ) -> ColumnarBatchBuilder {
        ColumnarBatchBuilder::new(
            names(kinds.len()),
            kinds.to_vec(),
            declared.to_vec(),
            batch_rows,
        )
        .unwrap()
    }

    fn undeclared(kinds: &[ColumnKind], batch_rows: usize) -> ColumnarBatchBuilder {
        builder(kinds, &vec![None; kinds.len()], batch_rows)
    }

    fn target_types() -> Vec<DataType> {
        vec![
            DataType::Utf8,
            DataType::Boolean,
            DataType::Int32,
            DataType::Int64,
            DataType::Float64,
        ]
    }

    const TRICKY: &[&str] = &[
        "",
        " 5 ",
        "1e3",
        "007",
        "5.0",
        "-5",
        "true",
        "T",
        " False ",
        "f",
        "yes",
        "NaN",
        "inf",
        "9223372036854775808",
        "9223372036854775807",
        "2147483648",
        "2147483647",
        "-2147483649",
        "-0",
        "0.1",
        "1e999",
        "+5",
        "héllo ✓ 日本",
    ];

    fn assert_same(got: &ArrayRef, expected: &ArrayRef, what: &str) {
        assert_eq!(got.data_type(), expected.data_type(), "{what}: type");
        assert_eq!(got.to_data(), expected.to_data(), "{what}: values");
    }

    #[test]
    fn mismatched_lengths_are_rejected() {
        assert!(ColumnarBatchBuilder::new(vec!["a".into()], vec![], vec![None], 10).is_err());
        assert!(
            ColumnarBatchBuilder::new(vec!["a".into()], vec![ColumnKind::Text], vec![], 10)
                .is_err()
        );
    }

    #[test]
    fn declared_eligible_columns_are_typed_from_the_start() {
        let b = builder(
            &[ColumnKind::Text, ColumnKind::Integer, ColumnKind::Deferred],
            &[
                Some(DataType::Utf8),
                Some(DataType::Int64),
                Some(DataType::Utf8),
            ],
            10,
        );
        assert!(!b.needs_value(0));
        assert!(!b.needs_value(1));
        assert!(b.needs_value(2)); // Deferred kind is always buffered
        let b = builder(
            &[ColumnKind::Integer, ColumnKind::Text],
            &[Some(DataType::Utf8), None],
            10,
        );
        assert!(b.needs_value(0)); // Integer kind cannot feed Utf8
        assert!(b.needs_value(1)); // undeclared: buffered until the first flush
    }

    #[test]
    fn typed_text_matches_buffered_for_every_target() {
        for target in target_types() {
            let mut b = builder(&[ColumnKind::Text], &[Some(target.clone())], 1000);
            assert!(!b.needs_value(0), "{target:?}");
            for s in TRICKY {
                b.append_str(0, s);
                b.end_row();
            }
            let batch = b.flush().unwrap().unwrap();
            let expected = build_array_from_values(
                TRICKY
                    .iter()
                    .map(|s| Some(Value::String((*s).to_owned())))
                    .collect(),
                &target,
            )
            .unwrap();
            assert_same(batch.column(0), &expected, &format!("{target:?}"));
            assert_eq!(batch.schema().field(0).data_type(), &target);
        }
    }

    #[test]
    fn typed_native_values_match_buffered_for_every_target() {
        let values: Vec<Option<Value>> = vec![
            None,
            Some(json!(0)),
            Some(json!(-7)),
            Some(json!(2147483648i64)),
            Some(json!(i64::MAX)),
            Some(json!(u64::MAX)),
            Some(json!(1.5)),
            Some(json!(-0.0)),
            Some(json!(2.0)),
            Some(json!(true)),
            Some(json!(false)),
            Some(json!("12")),
            Some(json!("t")),
            Some(json!("x")),
            Some(json!("")),
        ];
        for target in target_types() {
            let mut specific = builder(&[ColumnKind::Text], &[Some(target.clone())], 1000);
            let mut deferred = builder(&[ColumnKind::Text], &[Some(target.clone())], 1000);
            for v in &values {
                match v {
                    None => specific.append_null(0),
                    Some(Value::Bool(x)) => specific.append_bool(0, *x),
                    Some(Value::String(s)) => specific.append_str(0, s),
                    Some(Value::Number(n)) => match (n.as_i64(), n.as_f64()) {
                        (Some(i), _) => specific.append_i64(0, i),
                        (None, Some(f)) => specific.append_f64(0, f),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }
                specific.end_row();
                deferred.append_deferred(0, v.clone().unwrap_or(Value::Null));
                deferred.end_row();
            }
            let expected = build_array_from_values(values.clone(), &target).unwrap();
            let a = specific.flush().unwrap().unwrap();
            let d = deferred.flush().unwrap().unwrap();
            // u64::MAX through the native f64 path loses digits for Utf8
            // only; the deferred path must be exact everywhere.
            assert_same(d.column(0), &expected, &format!("deferred {target:?}"));
            if target != DataType::Utf8 {
                assert_same(a.column(0), &expected, &format!("native {target:?}"));
            }
        }
    }

    #[test]
    fn non_finite_floats_are_null() {
        for target in [DataType::Utf8, DataType::Float64] {
            let mut b = builder(&[ColumnKind::Decimal], &[Some(target.clone())], 10);
            if target == DataType::Utf8 {
                b = builder(&[ColumnKind::Text], &[Some(target.clone())], 10);
            }
            b.append_f64(0, f64::NAN);
            b.end_row();
            b.append_f64(0, f64::INFINITY);
            b.end_row();
            b.append_f64(0, 0.5);
            b.end_row();
            let batch = b.flush().unwrap().unwrap();
            assert_eq!(batch.column(0).null_count(), 2, "{target:?}");
        }
    }

    #[test]
    fn kinds_feed_their_declared_targets() {
        let mut b = builder(
            &[
                ColumnKind::Boolean,
                ColumnKind::Integer,
                ColumnKind::Integer,
                ColumnKind::Decimal,
            ],
            &[
                Some(DataType::Boolean),
                Some(DataType::Int32),
                Some(DataType::Float64),
                Some(DataType::Float64),
            ],
            10,
        );
        for c in 0..4 {
            assert!(!b.needs_value(c));
        }
        b.append_bool(0, true);
        b.append_i64(1, 5);
        b.append_i64(2, 5);
        b.append_f64(3, 2.5);
        b.end_row();
        b.append_i64(0, 1); // an integer is not a boolean: null
        b.append_i64(1, i64::from(i32::MAX) + 1);
        b.append_null(2);
        b.append_i64(3, 3);
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let bools = batch
            .column(0)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        assert_eq!(bools.iter().collect::<Vec<_>>(), vec![Some(true), None]);
        let ints = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(ints.iter().collect::<Vec<_>>(), vec![Some(5), None]);
        let f = batch
            .column(2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(f.iter().collect::<Vec<_>>(), vec![Some(5.0), None]);
        let d = batch
            .column(3)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(d.iter().collect::<Vec<_>>(), vec![Some(2.5), Some(3.0)]);
    }

    #[test]
    fn schema_equals_resolve_arrow_schema() {
        let columns = names(7);
        let declared_cols = vec![
            DeclaredColumn {
                name: "c0".into(),
                fhir_type: Some(ColumnFhirType::Boolean),
                collection: false,
            },
            DeclaredColumn {
                name: "c1".into(),
                fhir_type: Some(ColumnFhirType::Integer64),
                collection: false,
            },
            DeclaredColumn {
                name: "c2".into(),
                fhir_type: Some(ColumnFhirType::Decimal),
                collection: false,
            },
            DeclaredColumn {
                name: "c3".into(),
                fhir_type: Some(ColumnFhirType::String("code".into())),
                collection: true,
            },
            DeclaredColumn {
                name: "c4".into(),
                fhir_type: None,
                collection: false,
            },
        ];
        let kinds = vec![
            ColumnKind::Boolean,
            ColumnKind::Integer,
            ColumnKind::Decimal,
            ColumnKind::Deferred,
            ColumnKind::Text,
            ColumnKind::Integer,
            ColumnKind::Deferred,
        ];
        // c4: strings; c5: ints; c6: arrays; first 100 votes decide, later
        // rows must not.
        let rows: Vec<ProcessedRow> = (0..130)
            .map(|i| ProcessedRow {
                values: vec![
                    Some(json!(true)),
                    Some(json!(i)),
                    Some(json!(i as f64 + 0.5)),
                    Some(json!(["a"])),
                    Some(json!(format!("s{i}"))),
                    if i < 100 {
                        Some(json!(i))
                    } else {
                        Some(json!("x"))
                    },
                    Some(json!(["x", "y"])),
                ],
            })
            .collect();
        let expected = resolve_arrow_schema(&columns, &declared_cols, &rows).unwrap();

        let declared = declared_arrow_types(&columns, &declared_cols);
        let mut b = ColumnarBatchBuilder::new(columns.clone(), kinds, declared, 130).unwrap();
        assert!(b.schema().is_none());
        for row in &rows {
            for (c, v) in row.values.iter().enumerate() {
                let v = v.clone().unwrap();
                if b.needs_value(c) {
                    b.append_deferred(c, v);
                } else {
                    match v {
                        Value::Bool(x) => b.append_bool(c, x),
                        Value::Number(n) => match n.as_i64() {
                            Some(i) => b.append_i64(c, i),
                            None => b.append_f64(c, n.as_f64().unwrap()),
                        },
                        other => b.append_deferred(c, other),
                    }
                }
            }
            b.end_row();
        }
        let batch = b.flush().unwrap().unwrap();
        assert_eq!(batch.schema().as_ref(), &expected);
        assert_eq!(b.schema().unwrap().as_ref(), &expected);

        // Arrays equal the row path's, too.
        let arrays =
            crate::parquet_schema::process_to_arrow_arrays(&expected, &columns, &rows).unwrap();
        for (i, a) in arrays.iter().enumerate() {
            assert_same(batch.column(i), a, &format!("column {i}"));
        }
    }

    #[test]
    fn promotion_after_first_flush_matches_a_buffered_build() {
        // (kind, first-batch value that makes inference pick the type, type)
        let cases = [
            (ColumnKind::Text, json!(5), DataType::Int32),
            (ColumnKind::Text, json!(1.5), DataType::Float64),
            (ColumnKind::Text, json!(true), DataType::Boolean),
            (ColumnKind::Text, json!("s"), DataType::Utf8),
            (ColumnKind::Integer, json!(5), DataType::Int32),
            (ColumnKind::Decimal, json!(1.5), DataType::Float64),
            (ColumnKind::Boolean, json!(true), DataType::Boolean),
        ];
        for (kind, first, target) in cases {
            let mut b = undeclared(&[kind], 2);
            assert!(b.needs_value(0));
            b.append_deferred(0, first.clone());
            b.end_row();
            b.append_deferred(0, first);
            assert!(b.end_row());
            let first_batch = b.flush().unwrap().unwrap();
            assert_eq!(first_batch.schema().field(0).data_type(), &target);
            assert!(!b.needs_value(0), "{kind:?}/{target:?} should be promoted");

            let mut feed: Vec<Option<Value>> = TRICKY
                .iter()
                .map(|s| Some(Value::String((*s).to_owned())))
                .collect();
            feed.push(None);
            feed.push(Some(json!(3)));
            feed.push(Some(json!(2.5)));
            feed.push(Some(json!(false)));
            for v in &feed {
                b.append_deferred(0, v.clone().unwrap_or(Value::Null));
                b.end_row();
                // end_row flags a full batch but flush is deferred below
            }
            let second = b.flush().unwrap().unwrap();
            assert_eq!(second.schema(), first_batch.schema());
            let expected = build_array_from_values(feed, &target).unwrap();
            assert_same(second.column(0), &expected, &format!("{kind:?}/{target:?}"));
        }
    }

    #[test]
    fn ineligible_pairs_stay_buffered_and_match_the_row_path() {
        // Integer kind inferred Utf8 (strings), Deferred kind inferred Int32,
        // Text kind inferred List<Utf8>.
        let mut b = undeclared(
            &[ColumnKind::Integer, ColumnKind::Deferred, ColumnKind::Text],
            2,
        );
        for _ in 0..2 {
            b.append_deferred(0, json!("a"));
            b.append_deferred(1, json!(1));
            b.append_deferred(2, json!(["x"]));
            b.end_row();
        }
        let first = b.flush().unwrap().unwrap();
        assert_eq!(first.schema().field(0).data_type(), &DataType::Utf8);
        assert_eq!(first.schema().field(1).data_type(), &DataType::Int32);
        assert!(matches!(
            first.schema().field(2).data_type(),
            DataType::List(_)
        ));
        for c in 0..3 {
            assert!(b.needs_value(c), "column {c}");
        }
        b.append_deferred(0, json!(9));
        b.append_deferred(1, json!("77"));
        b.append_deferred(2, json!(["p", 1]));
        b.end_row();
        let second = b.flush().unwrap().unwrap();
        assert_eq!(second.schema(), first.schema());
        let a = second
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(a.value(0), "9");
        let i = second
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(i.value(0), 77);
        let l = second
            .column(2)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let inner = l.value(0);
        let inner = inner.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(inner.iter().collect::<Vec<_>>(), vec![Some("p"), Some("1")]);
    }

    #[test]
    fn typed_column_turns_json_containers_into_null() {
        let mut b = builder(
            &[ColumnKind::Integer, ColumnKind::Text],
            &[Some(DataType::Int32), Some(DataType::Utf8)],
            10,
        );
        b.append_deferred(0, json!(5));
        b.append_deferred(1, json!(true));
        b.end_row();
        b.append_deferred(0, json!([1]));
        b.append_deferred(1, json!({"a": 1}));
        b.end_row();
        b.append_deferred(0, Value::Null);
        b.append_deferred(1, json!(u64::MAX));
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        let i = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(i.iter().collect::<Vec<_>>(), vec![Some(5), None, None]);
        let t = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            vec![Some("true"), None, Some("18446744073709551615")]
        );
    }

    #[test]
    fn inferred_type_is_fixed_on_first_flush() {
        let mut b = undeclared(&[ColumnKind::Deferred], 2);
        b.append_deferred(0, json!(1));
        assert!(!b.end_row());
        b.append_deferred(0, json!(2));
        assert!(b.end_row());
        let first = b.flush().unwrap().unwrap();
        assert_eq!(first.schema().field(0).data_type(), &DataType::Int32);
        // A string would vote Utf8 on its own, but the type is locked.
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
    fn inference_samples_only_the_first_hundred_values() {
        let mut b = undeclared(&[ColumnKind::Deferred], 1000);
        for _ in 0..100 {
            b.append_deferred(0, json!(1));
            b.end_row();
        }
        for _ in 0..200 {
            b.append_deferred(0, json!("s"));
            b.end_row();
        }
        let batch = b.flush().unwrap().unwrap();
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Int32);
    }

    #[test]
    fn deferred_column_serialises_objects_and_string_lists() {
        let mut b = undeclared(&[ColumnKind::Deferred, ColumnKind::Deferred], 10);
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
    }

    #[test]
    fn declared_collection_is_buffered_as_utf8_list() {
        let cols = names(1);
        let declared = declared_arrow_types(
            &cols,
            &[DeclaredColumn {
                name: "c0".into(),
                fhir_type: Some(ColumnFhirType::Integer),
                collection: true,
            }],
        );
        let mut b =
            ColumnarBatchBuilder::new(cols, vec![ColumnKind::Deferred], declared, 10).unwrap();
        assert!(b.needs_value(0));
        b.append_deferred(0, json!([1, 2]));
        b.end_row();
        let batch = b.flush().unwrap().unwrap();
        assert!(matches!(
            batch.schema().field(0).data_type(),
            DataType::List(_)
        ));
    }

    #[test]
    fn batches_split_at_batch_rows_and_share_a_schema() {
        let mut b = builder(
            &[ColumnKind::Text, ColumnKind::Integer],
            &[Some(DataType::Utf8), None],
            3,
        );
        let mut batches = Vec::new();
        for i in 0..7 {
            b.append_str(0, &i.to_string());
            if b.needs_value(1) {
                b.append_deferred(1, json!(i));
            } else {
                b.append_i64(1, i);
            }
            if b.end_row() {
                batches.push(b.flush().unwrap().unwrap());
            }
        }
        batches.extend(b.flush().unwrap());
        let sizes: Vec<usize> = batches.iter().map(|b| b.num_rows()).collect();
        assert_eq!(sizes, vec![3, 3, 1]);
        assert!(batches.windows(2).all(|w| w[0].schema() == w[1].schema()));
        assert_eq!(b.schema().unwrap(), batches[0].schema());
        assert_eq!(batches[0].schema().field(1).data_type(), &DataType::Int32);
        assert!(!b.needs_value(1));
        assert!(b.flush().unwrap().is_none());
    }

    #[test]
    fn duplicate_column_names_stay_separate_columns() {
        let mut b = ColumnarBatchBuilder::new(
            vec!["x".into(), "x".into()],
            vec![ColumnKind::Text, ColumnKind::Integer],
            vec![Some(DataType::Utf8), Some(DataType::Int32)],
            10,
        )
        .unwrap();
        b.append_str(0, "a");
        b.append_i64(1, 1);
        b.end_row();
        assert_eq!(b.flush().unwrap().unwrap().num_columns(), 2);
    }

    #[test]
    fn empty_builder_flushes_nothing() {
        let mut b = undeclared(&[ColumnKind::Text], 10);
        assert!(b.flush().unwrap().is_none());
        assert!(b.schema().is_none());
    }

    #[test]
    fn counters_track_columns_and_pending_rows() {
        let mut b = undeclared(&ALL_KINDS, 10);
        assert_eq!(b.column_count(), 5);
        assert_eq!(b.pending_rows(), 0);
        for c in 0..5 {
            b.append_null(c);
        }
        assert!(!b.end_row());
        assert_eq!(b.pending_rows(), 1);
        b.flush().unwrap();
        assert_eq!(b.pending_rows(), 0);
    }

    #[test]
    fn every_value_kind_is_accepted_by_every_column() {
        // Declared and undeclared builders over all kinds; no value may panic
        // and every row must flush.
        for declared in [
            vec![
                Some(DataType::Utf8),
                Some(DataType::Boolean),
                Some(DataType::Int32),
                Some(DataType::Float64),
                None,
            ],
            vec![None; 5],
        ] {
            let mut b = builder(&ALL_KINDS, &declared, 100);
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
            for v in [
                json!("x"),
                json!(3),
                json!(2.5),
                json!({"a": 1}),
                Value::Null,
            ] {
                for c in 0..5 {
                    b.append_deferred(c, v.clone());
                }
                b.end_row();
            }
            let batch = b.flush().unwrap().unwrap();
            assert_eq!(batch.num_rows(), 10);
            assert_eq!(batch.num_columns(), 5);
        }
    }
}
