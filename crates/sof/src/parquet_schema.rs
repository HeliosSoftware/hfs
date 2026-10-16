//! # Parquet Schema Generation for SQL-on-FHIR
//!
//! This module converts SQL-on-FHIR ViewDefinition outputs to Arrow arrays (and
//! from there to Parquet / Arrow IPC), choosing each column's Arrow type.
//!
//! ## Type selection
//!
//! A column whose ViewDefinition declaration is known (see
//! [`create_arrow_schema_typed`]) gets the Arrow type of its resolved
//! ISO/IEC 9075 type ([`crate::column_types`]), following the SQL-on-FHIR
//! 3.0.0-ballot default mappings and the `ansi/type` column tag:
//!
//! | SQL type | Arrow type |
//! |---|---|
//! | `BOOLEAN` | `Boolean` |
//! | `INT` | `Int32` |
//! | `BIGINT` | `Int64` |
//! | `CHARACTER VARYING` | `Utf8` |
//! | `BINARY` | `Binary` (base64 decoded) |
//! | `TIMESTAMP WITH TIME ZONE` | `Timestamp(Microsecond, "UTC")` |
//! | `DATE` (tag only) | `Date32` |
//! | `TIME` (tag only) | `Time64(Microsecond)` |
//! | `TIMESTAMP` (tag only) | `Timestamp(Microsecond, None)` |
//! | `DOUBLE PRECISION` (tag only) | `Float64` |
//!
//! `collection: true` columns become `List<element type>`. A value that cannot
//! be represented in the column's type (an integer outside the `INT` range, a
//! partial date for `DATE`, invalid base64, ...) is an error naming the column;
//! it is never silently nulled.
//!
//! Columns without a declared type fall back to inspecting the values (see
//! [`infer_arrow_type`]) and the FHIRPath default mappings: a number with a
//! fractional part is a FHIRPath `Decimal` and so becomes `Utf8`, integers become
//! `Int32` (`Int64` if any value does not fit), strings `Utf8`.

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float64Builder, Int32Builder,
    Int64Builder, ListArray, StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow::buffer::{NullBuffer, OffsetBuffer};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use base64::Engine as _;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::column_types::{AnsiType, default_for_fhir_type};
use crate::sqlquery::ColumnFhirType;
use crate::sqlquery::engine::DeclaredColumn;
use crate::{ProcessedRow, SofError};

/// Infers an Arrow type from sampled values (the fallback for columns that do
/// not declare a `type`).
pub fn infer_arrow_type(values: &[Option<Value>]) -> DataType {
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut has_array = false;
    let mut has_object = false;

    for value in values.iter().flatten() {
        match value {
            Value::Bool(_) => {
                *type_counts.entry("bool".to_string()).or_insert(0) += 1;
            }
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    *type_counts.entry("integer".to_string()).or_insert(0) += 1;
                } else {
                    *type_counts.entry("decimal".to_string()).or_insert(0) += 1;
                }
            }
            Value::String(_) => {
                *type_counts.entry("string".to_string()).or_insert(0) += 1;
            }
            Value::Array(_) => {
                has_array = true;
            }
            Value::Object(_) => {
                has_object = true;
            }
            Value::Null => {}
        }
    }

    // Without a declaration the element type of a collection is not known, so
    // an inferred list is always `List<Utf8>`.
    if has_array {
        return utf8_list_type();
    }

    if has_object {
        return DataType::Utf8;
    }

    // A fractional number is a FHIRPath `Decimal`, which maps to character data
    // (the lexical form is preserved); integers in the same column are then
    // rendered as text too.
    if type_counts.contains_key("decimal") {
        return DataType::Utf8;
    }

    let most_common = type_counts
        .into_iter()
        .max_by_key(|(name, count)| (*count, name.clone()))
        .map(|(type_name, _)| type_name);

    match most_common.as_deref() {
        Some("bool") => DataType::Boolean,
        Some("integer") => DataType::Int32,
        Some("string") => DataType::Utf8,
        _ => DataType::Utf8,
    }
}

/// Widens `Int32` (or `List<Int32>`) to the 64-bit equivalent when any value
/// does not fit in 32 bits, so un-declared integer columns never truncate.
fn widen_for_overflow(data_type: DataType, values: impl Iterator<Item = Value>) -> DataType {
    fn needs_wide(v: &Value) -> bool {
        match v {
            Value::Number(n) => n.as_i64().is_none_or(|i| i32::try_from(i).is_err()),
            Value::Array(a) => a.iter().any(needs_wide),
            _ => false,
        }
    }
    match &data_type {
        DataType::Int32 => {
            if values.into_iter().any(|v| needs_wide(&v)) {
                DataType::Int64
            } else {
                data_type
            }
        }
        DataType::List(f) if f.data_type() == &DataType::Int32 => {
            if values.into_iter().any(|v| needs_wide(&v)) {
                DataType::List(Arc::new(Field::new("item", DataType::Int64, true)))
            } else {
                data_type
            }
        }
        _ => data_type,
    }
}

/// The Arrow type of a resolved SQL type.
pub fn arrow_type_for_ansi(ansi: AnsiType) -> DataType {
    match ansi {
        AnsiType::Boolean => DataType::Boolean,
        AnsiType::Int => DataType::Int32,
        AnsiType::BigInt => DataType::Int64,
        AnsiType::Varchar => DataType::Utf8,
        AnsiType::Binary => DataType::Binary,
        AnsiType::TimestampTz => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        AnsiType::Date => DataType::Date32,
        AnsiType::Time => DataType::Time64(TimeUnit::Microsecond),
        AnsiType::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
        AnsiType::Double => DataType::Float64,
    }
}

/// Number of leading rows sampled to infer the type of undeclared columns.
pub const SCHEMA_SAMPLE_ROWS: usize = 100;

/// Arrow field metadata key marking a column whose type comes from the
/// ViewDefinition (a declared `type`, an `ansi/type` tag or `collection:
/// true`). Array building is strict for such a column: a value that does not
/// fit the type is an error naming the column. For any other column the type
/// is only a guess from the data seen so far, so a misfit becomes null.
pub const DECLARED_FIELD_KEY: &str = "helios.sof.declared";

/// A `Field` for `name`, marked as declared when `declared` is true.
pub fn schema_field(name: &str, data_type: DataType, declared: bool) -> Field {
    let field = Field::new(name, data_type, true);
    if declared {
        field.with_metadata(HashMap::from([(
            DECLARED_FIELD_KEY.to_string(),
            "true".to_string(),
        )]))
    } else {
        field
    }
}

fn utf8_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)))
}

/// Arrow type for a ViewDefinition column with a declared FHIR `type` and no
/// `ansi/type` tag: the spec default for that type (see [`default_for_fhir_type`]).
/// A `collection: true` column is a `List` of that type.
pub fn arrow_type_for_declared(fhir_type: &ColumnFhirType, collection: bool) -> DataType {
    arrow_type_for_resolved(default_for_fhir_type(fhir_type.code()), collection)
}

/// Arrow type for an already resolved SQL type, wrapped in a `List` for
/// `collection: true` columns.
pub fn arrow_type_for_resolved(ansi: AnsiType, collection: bool) -> DataType {
    let element = arrow_type_for_ansi(ansi);
    if collection {
        DataType::List(Arc::new(Field::new("item", element, true)))
    } else {
        element
    }
}

/// Builds the Arrow schema purely by inspecting values; see
/// [`resolve_arrow_schema`] for the declaration-driven variant.
pub fn create_arrow_schema(columns: &[String], rows: &[ProcessedRow]) -> Result<Schema, SofError> {
    resolve_arrow_schema(columns, &[], rows)
}

/// Decides the Arrow schema for `columns`.
///
/// A column found (by name) in `declared` with a resolved SQL type (the
/// declared `type`, overridden by an `ansi/type` tag) gets that type's Arrow
/// mapping; every other column is inferred from at most
/// [`SCHEMA_SAMPLE_ROWS`] rows of `rows` (widening `Int32` to `Int64` if any
/// row has a larger integer). Column order and names come from `columns`.
/// Shared by the buffered and the incremental writers so both agree.
pub fn resolve_arrow_schema(
    columns: &[String],
    declared: &[DeclaredColumn],
    rows: &[ProcessedRow],
) -> Result<Schema, SofError> {
    let sample_size = std::cmp::min(SCHEMA_SAMPLE_ROWS, rows.len());
    let mut fields = Vec::new();

    for (col_idx, column_name) in columns.iter().enumerate() {
        let decl = declared.iter().find(|d| &d.name == column_name);
        let is_declared = decl.is_some_and(|d| d.ansi.is_some() || d.collection);
        let data_type = match decl {
            Some(DeclaredColumn {
                ansi: Some(ansi),
                collection,
                ..
            }) => arrow_type_for_resolved(*ansi, *collection),
            Some(DeclaredColumn {
                collection: true, ..
            }) => utf8_list_type(),
            _ => {
                let sample_values: Vec<Option<Value>> = rows
                    .iter()
                    .take(sample_size)
                    .map(|row| row.values.get(col_idx).cloned().flatten())
                    .collect();
                let inferred = infer_arrow_type(&sample_values);
                widen_for_overflow(
                    inferred,
                    rows.iter()
                        .filter_map(|row| row.values.get(col_idx).cloned().flatten()),
                )
            }
        };
        fields.push(schema_field(column_name, data_type, is_declared));
    }

    Ok(Schema::new(fields))
}

fn conversion_error(column: &str, expected: &str, value: &Value) -> SofError {
    SofError::ParquetConversionError(format!(
        "column '{}': value {} cannot be represented as {}",
        column, value, expected
    ))
}

fn parse_instant(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_micros())
}

fn parse_naive_timestamp(s: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.naive_local().and_utc().timestamp_micros());
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|dt| dt.and_utc().timestamp_micros())
}

fn parse_date32(s: &str) -> Option<i32> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    i32::try_from(date.signed_duration_since(epoch).num_days()).ok()
}

fn parse_time_micros(s: &str) -> Option<i64> {
    let t = NaiveTime::parse_from_str(s, "%H:%M:%S%.f").ok()?;
    Some(i64::from(t.num_seconds_from_midnight()) * 1_000_000 + i64::from(t.nanosecond() / 1_000))
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Builds an array of `data_type` from raw cell values, without a column name
/// for error messages (used by the columnar accumulator).
///
/// Lenient: a value that does not fit `data_type` becomes null, because the
/// type of such a column was inferred from earlier data and cannot change
/// once the stream has started. Declared columns use [`build_named_array`],
/// which reports a misfit instead.
pub(crate) fn build_array_from_values(
    values: Vec<Option<Value>>,
    data_type: &DataType,
) -> Result<ArrayRef, SofError> {
    if let Ok(array) = build_named_array(&values, data_type, "column") {
        return Ok(array);
    }
    // Slow path, only after a misfit: null out the values that do not fit.
    let cleaned: Vec<Option<Value>> = values
        .into_iter()
        .map(|value| match value {
            Some(v) if !v.is_null() => {
                let one = [Some(v)];
                if build_named_array(&one, data_type, "column").is_ok() {
                    let [kept] = one;
                    kept
                } else {
                    None
                }
            }
            other => other,
        })
        .collect();
    build_named_array(&cleaned, data_type, "column")
}

/// Builds an array of `data_type` from raw cell values; a value that does not
/// fit is an error naming `column`.
pub(crate) fn build_named_array(
    values: &[Option<Value>],
    data_type: &DataType,
    column: &str,
) -> Result<ArrayRef, SofError> {
    // JSON null is the same as an absent value.
    let is_null = |v: &Option<Value>| matches!(v, None | Some(Value::Null));
    match data_type {
        DataType::Boolean => {
            let mut builder = BooleanBuilder::new();
            for value in values {
                match value {
                    Some(Value::Bool(b)) => builder.append_value(*b),
                    // PostgreSQL renders booleans as 't' / 'f'; SQLite stores
                    // them as the integers 0 / 1.
                    Some(Value::String(s))
                        if matches!(s.to_ascii_lowercase().as_str(), "true" | "t") =>
                    {
                        builder.append_value(true)
                    }
                    Some(Value::String(s))
                        if matches!(s.to_ascii_lowercase().as_str(), "false" | "f") =>
                    {
                        builder.append_value(false)
                    }
                    Some(Value::Number(n)) if n.as_i64() == Some(1) => builder.append_value(true),
                    Some(Value::Number(n)) if n.as_i64() == Some(0) => builder.append_value(false),
                    v if is_null(v) => builder.append_null(),
                    Some(other) => return Err(conversion_error(column, "BOOLEAN", other)),
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int32 => {
            let mut builder = Int32Builder::new();
            for value in values {
                match value {
                    v if is_null(v) => builder.append_null(),
                    Some(other) => match as_i64(other).and_then(|i| i32::try_from(i).ok()) {
                        Some(i) => builder.append_value(i),
                        None => return Err(conversion_error(column, "INT", other)),
                    },
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int64 => {
            let mut builder = Int64Builder::new();
            for value in values {
                match value {
                    v if is_null(v) => builder.append_null(),
                    Some(other) => match as_i64(other) {
                        Some(i) => builder.append_value(i),
                        None => return Err(conversion_error(column, "BIGINT", other)),
                    },
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::new();
            for value in values {
                match value {
                    Some(Value::Number(n)) => match n.as_f64() {
                        Some(f) => builder.append_value(f),
                        None => {
                            return Err(conversion_error(
                                column,
                                "DOUBLE PRECISION",
                                &Value::Number(n.clone()),
                            ));
                        }
                    },
                    Some(Value::String(s)) => match s.trim().parse::<f64>() {
                        Ok(f) => builder.append_value(f),
                        Err(_) => {
                            return Err(conversion_error(
                                column,
                                "DOUBLE PRECISION",
                                &Value::String(s.clone()),
                            ));
                        }
                    },
                    v if is_null(v) => builder.append_null(),
                    Some(other) => {
                        return Err(conversion_error(column, "DOUBLE PRECISION", other));
                    }
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Utf8 => {
            let mut builder = StringBuilder::new();
            for value in values {
                match value {
                    Some(Value::String(s)) => builder.append_value(s),
                    Some(Value::Number(n)) => builder.append_value(n.to_string()),
                    Some(Value::Bool(b)) => builder.append_value(b.to_string()),
                    Some(v @ (Value::Object(_) | Value::Array(_))) => {
                        builder.append_value(
                            serde_json::to_string(&v).unwrap_or_else(|_| "null".into()),
                        );
                    }
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Binary => {
            let mut builder = BinaryBuilder::new();
            for value in values {
                match value {
                    Some(Value::String(s)) => {
                        match base64::engine::general_purpose::STANDARD.decode(s.trim()) {
                            Ok(bytes) => builder.append_value(bytes),
                            Err(_) => {
                                return Err(conversion_error(
                                    column,
                                    "BINARY (invalid base64)",
                                    &Value::String(s.clone()),
                                ));
                            }
                        }
                    }
                    v if is_null(v) => builder.append_null(),
                    Some(other) => return Err(conversion_error(column, "BINARY", other)),
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let with_tz = tz.is_some();
            let expected = if with_tz {
                "TIMESTAMP WITH TIME ZONE"
            } else {
                "TIMESTAMP"
            };
            let mut builder = TimestampMicrosecondBuilder::new();
            for value in values {
                match value {
                    Some(Value::String(s)) => {
                        let micros = if with_tz {
                            parse_instant(s)
                        } else {
                            parse_naive_timestamp(s)
                        };
                        match micros {
                            Some(m) => builder.append_value(m),
                            None => {
                                return Err(conversion_error(
                                    column,
                                    expected,
                                    &Value::String(s.clone()),
                                ));
                            }
                        }
                    }
                    v if is_null(v) => builder.append_null(),
                    Some(other) => return Err(conversion_error(column, expected, other)),
                    None => unreachable!(),
                }
            }
            let array = builder.finish();
            Ok(Arc::new(match tz {
                Some(tz) => array.with_timezone(tz.clone()),
                None => array,
            }))
        }
        DataType::Date32 => {
            let mut builder = Date32Builder::new();
            for value in values {
                match value {
                    Some(Value::String(s)) => match parse_date32(s) {
                        Some(d) => builder.append_value(d),
                        None => {
                            return Err(conversion_error(
                                column,
                                "DATE",
                                &Value::String(s.clone()),
                            ));
                        }
                    },
                    v if is_null(v) => builder.append_null(),
                    Some(other) => return Err(conversion_error(column, "DATE", other)),
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Time64(TimeUnit::Microsecond) => {
            let mut builder = Time64MicrosecondBuilder::new();
            for value in values {
                match value {
                    Some(Value::String(s)) => match parse_time_micros(s) {
                        Some(t) => builder.append_value(t),
                        None => {
                            return Err(conversion_error(
                                column,
                                "TIME",
                                &Value::String(s.clone()),
                            ));
                        }
                    },
                    v if is_null(v) => builder.append_null(),
                    Some(other) => return Err(conversion_error(column, "TIME", other)),
                    None => unreachable!(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::List(field) => {
            // Flatten every list into one element vector, then build the
            // element array with the (recursive) scalar builder.
            let mut flat: Vec<Option<Value>> = Vec::new();
            let mut lengths: Vec<usize> = Vec::with_capacity(values.len());
            let mut validity: Vec<bool> = Vec::with_capacity(values.len());
            for value in values {
                match value {
                    Some(Value::Array(arr)) => {
                        lengths.push(arr.len());
                        validity.push(true);
                        flat.extend(arr.iter().cloned().map(Some));
                    }
                    None | Some(Value::Null) => {
                        lengths.push(0);
                        validity.push(false);
                    }
                    // A lone value in a collection column is a one-element list.
                    Some(single) => {
                        lengths.push(1);
                        validity.push(true);
                        flat.push(Some(single.clone()));
                    }
                }
            }
            let elements = build_named_array(&flat, field.data_type(), column)?;
            let list = ListArray::try_new(
                field.clone(),
                OffsetBuffer::from_lengths(lengths),
                elements,
                Some(NullBuffer::from(validity)),
            )
            .map_err(|e| {
                SofError::ParquetConversionError(format!(
                    "column '{}': failed to assemble list array: {}",
                    column, e
                ))
            })?;
            Ok(Arc::new(list))
        }
        _ => Err(SofError::ParquetConversionError(format!(
            "Unsupported data type for Parquet conversion: {:?}",
            data_type
        ))),
    }
}

pub fn process_to_arrow_arrays(
    schema: &Schema,
    _columns: &[String],
    rows: &[ProcessedRow],
) -> Result<Vec<ArrayRef>, SofError> {
    let mut arrays = Vec::new();

    for (col_idx, field) in schema.fields().iter().enumerate() {
        let values: Vec<Option<Value>> = rows
            .iter()
            .map(|row| row.values.get(col_idx).cloned().flatten())
            .collect();

        let array = if field.metadata().contains_key(DECLARED_FIELD_KEY) {
            build_named_array(&values, field.data_type(), field.name())?
        } else {
            build_array_from_values(values, field.data_type())?
        };
        arrays.push(array);
    }

    Ok(arrays)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;
    use serde_json::json;

    #[test]
    fn test_infer_boolean_type() {
        let values = vec![
            Some(json!(true)),
            Some(json!(false)),
            None,
            Some(json!(true)),
        ];
        assert_eq!(infer_arrow_type(&values), DataType::Boolean);
    }

    #[test]
    fn test_infer_integer_type() {
        let values = vec![Some(json!(42)), Some(json!(100)), None, Some(json!(-5))];
        assert_eq!(infer_arrow_type(&values), DataType::Int32);
    }

    #[test]
    fn test_infer_decimal_type() {
        let values = vec![
            Some(json!(std::f64::consts::PI)),
            Some(json!(std::f64::consts::E)),
            None,
            Some(json!(1.0)),
        ];
        // A FHIRPath Decimal maps to character data, not a float.
        assert_eq!(infer_arrow_type(&values), DataType::Utf8);
    }

    #[test]
    fn test_infer_string_type() {
        let values = vec![
            Some(json!("hello")),
            Some(json!("world")),
            None,
            Some(json!("test")),
        ];
        assert_eq!(infer_arrow_type(&values), DataType::Utf8);
    }

    #[test]
    fn test_infer_array_type() {
        let values = vec![Some(json!(["a", "b", "c"])), Some(json!(["d", "e"])), None];
        match infer_arrow_type(&values) {
            DataType::List(field) => {
                assert_eq!(field.name(), "item");
                assert_eq!(field.data_type(), &DataType::Utf8);
            }
            _ => panic!("Expected List type"),
        }
    }

    #[test]
    fn test_infer_object_type_as_string() {
        let values = vec![
            Some(json!({"key": "value"})),
            Some(json!({"foo": "bar"})),
            None,
        ];
        assert_eq!(infer_arrow_type(&values), DataType::Utf8);
    }

    #[test]
    fn test_mixed_types_favor_most_common() {
        let values = vec![
            Some(json!("string1")),
            Some(json!("string2")),
            Some(json!(42)),
            Some(json!("string3")),
        ];
        assert_eq!(infer_arrow_type(&values), DataType::Utf8);
    }

    #[test]
    fn test_create_schema_basic() {
        let columns = vec!["id".to_string(), "name".to_string(), "age".to_string()];
        let rows = vec![
            ProcessedRow {
                values: vec![Some(json!("123")), Some(json!("John Doe")), Some(json!(42))],
            },
            ProcessedRow {
                values: vec![
                    Some(json!("456")),
                    Some(json!("Jane Smith")),
                    Some(json!(35)),
                ],
            },
        ];

        let schema = create_arrow_schema(&columns, &rows).unwrap();
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(1).data_type(), &DataType::Utf8);
        assert_eq!(schema.field(2).name(), "age");
        assert_eq!(schema.field(2).data_type(), &DataType::Int32);
    }

    #[test]
    fn test_build_boolean_array() {
        let values = vec![
            Some(json!(true)),
            None,
            Some(json!(false)),
            Some(json!(true)),
        ];
        let array = build_named_array(&values, &DataType::Boolean, "c").unwrap();
        let bool_array = array
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();

        assert_eq!(array.len(), 4);
        assert!(bool_array.value(0));
        assert!(array.is_null(1));
        assert!(!bool_array.value(2));
        assert!(bool_array.value(3));
    }

    #[test]
    fn test_build_string_array_with_mixed_types() {
        let values = vec![
            Some(json!("text")),
            Some(json!(42)),
            Some(json!(true)),
            Some(json!({"key": "value"})),
            None,
        ];
        let array = build_named_array(&values, &DataType::Utf8, "c").unwrap();
        let string_array = array
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();

        assert_eq!(array.len(), 5);
        assert_eq!(string_array.value(0), "text");
        assert_eq!(string_array.value(1), "42");
        assert_eq!(string_array.value(2), "true");
        assert!(string_array.value(3).contains("\"key\""));
        assert!(array.is_null(4));
    }

    #[test]
    fn test_boolean_array_accepts_postgres_text_and_sqlite_integers() {
        let values = vec![
            Some(json!("t")),
            Some(json!("F")),
            Some(json!("TRUE")),
            Some(json!("maybe")),
            Some(json!(1)),
            Some(json!(0)),
        ];
        let array = build_array_from_values(values, &DataType::Boolean).unwrap();
        let b = array
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();
        assert!(b.value(0) && !b.value(1) && b.value(2));
        assert!(array.is_null(3));
        assert!(b.value(4) && !b.value(5));
        // A declared boolean column reports the misfit instead.
        let err = build_named_array(&[Some(json!("maybe"))], &DataType::Boolean, "flag")
            .unwrap_err()
            .to_string();
        assert!(err.contains("flag"), "{err}");
    }

    #[test]
    fn test_infer_integer_type_widens_to_int64_when_sample_exceeds_i32() {
        let values = vec![Some(json!(1)), Some(json!(3_000_000_000_i64)), None];
        let schema = create_arrow_schema(
            &["n".to_string()],
            &values
                .into_iter()
                .map(|v| ProcessedRow { values: vec![v] })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(schema.field(0).data_type(), &DataType::Int64);
    }

    #[test]
    fn test_create_schema_widens_integer_column_past_the_sample() {
        // The kind is inferred from the first 100 rows; the width must be
        // decided from every row, or row 101 wraps or fails at write time.
        let columns = vec!["n".to_string()];
        let mut rows: Vec<ProcessedRow> = (0..100)
            .map(|i| ProcessedRow {
                values: vec![Some(json!(i))],
            })
            .collect();
        rows.push(ProcessedRow {
            values: vec![Some(json!(3_000_000_000_i64))],
        });

        let schema = create_arrow_schema(&columns, &rows).unwrap();
        assert_eq!(schema.field(0).data_type(), &DataType::Int64);
    }

    #[test]
    fn test_build_int32_array_nulls_out_of_range_when_inferred() {
        let values = vec![
            Some(json!(1)),
            Some(json!(3_000_000_000_i64)),
            Some(json!(-3_000_000_000_i64)),
            Some(json!("42")),
        ];
        let array = build_array_from_values(values.clone(), &DataType::Int32).unwrap();
        let ints = array
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap();
        assert_eq!(ints.value(0), 1);
        assert!(array.is_null(1));
        assert!(array.is_null(2));
        assert_eq!(ints.value(3), 42);

        // A declared column reports the first misfit, naming the column.
        let err = build_named_array(&values, &DataType::Int32, "n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("column 'n'"), "{err}");
    }

    #[test]
    fn test_build_int64_array() {
        let values = vec![
            Some(json!(3_000_000_000_i64)),
            Some(json!(-3_000_000_000_i64)),
            None,
            Some(json!(i64::MAX)),
            Some(json!(i64::MIN)),
            Some(json!("not a number")),
            Some(json!(u64::MAX)),
        ];
        let array = build_array_from_values(values, &DataType::Int64).unwrap();
        let ints = array
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert_eq!(array.len(), 7);
        assert_eq!(ints.value(0), 3_000_000_000);
        assert_eq!(ints.value(1), -3_000_000_000);
        assert!(array.is_null(2));
        assert_eq!(ints.value(3), i64::MAX);
        assert_eq!(ints.value(4), i64::MIN);
        assert!(array.is_null(5));
        assert!(array.is_null(6));
    }

    #[test]
    fn test_resolve_schema_declared_inferred_and_collection() {
        use crate::sqlquery::engine::declared_columns;
        let view = json!({"select": [{"column": [
            {"name": "n", "type": "integer"},
            {"name": "big", "type": "integer64"},
            {"name": "d", "type": "decimal"},
            {"name": "b", "type": "boolean"},
            {"name": "tags", "type": "string", "collection": true},
            {"name": "untyped", "path": "x"},
        ]}]});
        let declared = declared_columns(&view);
        assert_eq!(declared.len(), 6);
        assert!(declared[5].fhir_type.is_none());
        let columns: Vec<String> = declared.iter().map(|d| d.name.clone()).collect();
        let rows = vec![ProcessedRow {
            values: vec![
                Some(json!(1)),
                Some(json!(1)),
                Some(json!(1)), // integer-looking value, declared decimal
                Some(json!(true)),
                Some(json!(["a"])),
                Some(json!(7)),
            ],
        }];
        let schema = resolve_arrow_schema(&columns, &declared, &rows).unwrap();
        let types: Vec<_> = schema
            .fields()
            .iter()
            .map(|f| f.data_type().clone())
            .collect();
        assert_eq!(types[0], DataType::Int32);
        assert_eq!(types[1], DataType::Int64);
        assert_eq!(types[2], DataType::Utf8); // decimal is character data
        assert_eq!(types[3], DataType::Boolean);
        assert_eq!(types[4], utf8_list_type());
        assert_eq!(types[5], DataType::Int32); // inferred
        // Only the declared columns are marked strict.
        let strict: Vec<_> = schema
            .fields()
            .iter()
            .map(|f| f.metadata().contains_key(DECLARED_FIELD_KEY))
            .collect();
        assert_eq!(strict, vec![true, true, true, true, true, false]);
    }

    #[test]
    fn test_inferred_lists_are_always_utf8() {
        let values = vec![Some(json!([1, 2, 3]))];
        assert_eq!(infer_arrow_type(&values), utf8_list_type());
    }

    #[test]
    fn test_inferred_column_misfit_is_null_but_declared_misfit_is_an_error() {
        let columns = vec!["n".to_string()];
        let rows = vec![
            ProcessedRow {
                values: vec![Some(json!(1))],
            },
            ProcessedRow {
                values: vec![Some(json!("x"))],
            },
        ];
        // Inferred Int32 (the sample is all one kind here).
        let schema = Schema::new(vec![Field::new("n", DataType::Int32, true)]);
        let arrays = process_to_arrow_arrays(&schema, &columns, &rows).unwrap();
        assert_eq!(arrays[0].null_count(), 1);
        // The same type, declared.
        let schema = Schema::new(vec![schema_field("n", DataType::Int32, true)]);
        let err = process_to_arrow_arrays(&schema, &columns, &rows).unwrap_err();
        assert!(err.to_string().contains("column 'n'"), "{err}");
    }
}
