//! # Parquet Schema Generation for SQL-on-FHIR
//!
//! This module provides functionality for converting SQL-on-FHIR ViewDefinition outputs
//! to Apache Parquet format with proper schema inference and data type mapping.
//!
//! ## Overview
//!
//! The parquet schema generator:
//! - Infers Arrow/Parquet schemas from FHIR data types
//! - Converts processed ViewDefinition rows to Arrow columnar format
//! - Handles collection columns as list types
//! - Maps FHIR primitive types to appropriate Arrow data types
//!
//! ## Type Mappings
//!
//! - FHIR String → Arrow UTF8
//! - FHIR Integer → Arrow Int32
//! - FHIR Decimal/Float → Arrow Float64
//! - FHIR Boolean → Arrow Boolean
//! - Collections → Arrow List types

use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, Int64Builder, ListBuilder,
    StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::sqlquery::ColumnFhirType;
use crate::sqlquery::engine::DeclaredColumn;
use crate::{ProcessedRow, SofError};

fn utf8_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)))
}

/// Arrow type for a ViewDefinition column with a declared FHIR type.
///
/// `boolean`→Boolean, `integer`/`positiveInt`/`unsignedInt`→Int32,
/// `integer64`→Int64, `decimal`→Float64, everything else→Utf8. A
/// `collection: true` column is `List<Utf8>` whatever its type.
pub fn arrow_type_for_declared(fhir_type: &ColumnFhirType, collection: bool) -> DataType {
    if collection {
        return utf8_list_type();
    }
    match fhir_type {
        ColumnFhirType::Boolean => DataType::Boolean,
        ColumnFhirType::Integer => DataType::Int32,
        ColumnFhirType::Integer64 => DataType::Int64,
        ColumnFhirType::Decimal => DataType::Float64,
        _ => DataType::Utf8,
    }
}

/// Number of leading rows sampled to infer the type of undeclared columns.
pub const SCHEMA_SAMPLE_ROWS: usize = 100;

/// Decides the Arrow schema for `columns`.
///
/// A column found (by name) in `declared` with a `type` or `collection: true`
/// gets its declared mapping; every other column is inferred from at most
/// [`SCHEMA_SAMPLE_ROWS`] rows of `sample`. Column order and names come from
/// `columns`. Shared by the buffered and the incremental writers so both agree.
pub fn resolve_arrow_schema(
    columns: &[String],
    declared: &[DeclaredColumn],
    sample: &[ProcessedRow],
) -> Result<Schema, SofError> {
    let sample = &sample[..sample.len().min(SCHEMA_SAMPLE_ROWS)];
    let fields = columns
        .iter()
        .enumerate()
        .map(|(col_idx, column_name)| {
            let decl = declared.iter().find(|d| &d.name == column_name);
            let data_type = match decl {
                Some(DeclaredColumn {
                    fhir_type: Some(t),
                    collection,
                    ..
                }) => arrow_type_for_declared(t, *collection),
                Some(DeclaredColumn {
                    collection: true, ..
                }) => utf8_list_type(),
                _ => {
                    let values: Vec<Option<Value>> = sample
                        .iter()
                        .map(|row| row.values.get(col_idx).cloned().flatten())
                        .collect();
                    infer_arrow_type(&values)
                }
            };
            Field::new(column_name, data_type, true)
        })
        .collect::<Vec<_>>();
    Ok(Schema::new(fields))
}

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

    // Collections are always `List<Utf8>`: the array builder stringifies
    // elements, so any other element type would not match the built arrays.
    if has_array {
        return utf8_list_type();
    }

    if has_object {
        return DataType::Utf8;
    }

    let most_common = type_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(type_name, _)| type_name);

    match most_common.as_deref() {
        Some("bool") => DataType::Boolean,
        Some("integer") => DataType::Int32,
        Some("decimal") => DataType::Float64,
        Some("string") => DataType::Utf8,
        _ => DataType::Utf8,
    }
}

/// Infers the schema purely from a sample of `rows` (no declared types).
pub fn create_arrow_schema(columns: &[String], rows: &[ProcessedRow]) -> Result<Schema, SofError> {
    resolve_arrow_schema(columns, &[], rows)
}

/// Parses a string cell the way a typed column expects (Postgres runners
/// return text for several columns).
fn parse_str_number(s: &str) -> Option<serde_json::Number> {
    s.trim().parse::<serde_json::Number>().ok()
}

/// An integer cell: a JSON integer or a string holding one. Anything else
/// (floats, out-of-range u64, non-numeric text) is `None`, i.e. null.
fn cell_as_i64(value: Option<Value>) -> Option<i64> {
    match value {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => parse_str_number(&s).and_then(|n| n.as_i64()),
        _ => None,
    }
}

pub(crate) fn build_array_from_values(
    values: Vec<Option<Value>>,
    data_type: &DataType,
) -> Result<ArrayRef, SofError> {
    match data_type {
        DataType::Boolean => {
            let mut builder = BooleanBuilder::new();
            for value in values {
                match value {
                    Some(Value::Bool(b)) => builder.append_value(b),
                    Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                        // Postgres text renders booleans as `t` / `f`.
                        "true" | "t" => builder.append_value(true),
                        "false" | "f" => builder.append_value(false),
                        _ => builder.append_null(),
                    },
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int32 => {
            let mut builder = Int32Builder::new();
            for value in values {
                match cell_as_i64(value).and_then(|i| i32::try_from(i).ok()) {
                    Some(i) => builder.append_value(i),
                    None => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int64 => {
            let mut builder = Int64Builder::new();
            for value in values {
                match cell_as_i64(value) {
                    Some(i) => builder.append_value(i),
                    None => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::new();
            for value in values {
                let number = match value {
                    Some(Value::Number(n)) => Some(n),
                    Some(Value::String(s)) => parse_str_number(&s),
                    _ => None,
                };
                match number.and_then(|n| n.as_f64()) {
                    Some(f) => builder.append_value(f),
                    None => builder.append_null(),
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
                    Some(Value::Object(_)) | Some(Value::Array(_)) => {
                        builder.append_value(
                            serde_json::to_string(&value.unwrap())
                                .unwrap_or_else(|_| "null".to_string()),
                        );
                    }
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::List(field) => {
            let element_type = field.data_type();
            match element_type {
                DataType::Utf8 => {
                    let mut builder = ListBuilder::new(StringBuilder::new());
                    for value in values {
                        match value {
                            Some(Value::Array(arr)) => {
                                for elem in arr {
                                    match elem {
                                        Value::String(s) => builder.values().append_value(s),
                                        _ => builder.values().append_value(elem.to_string()),
                                    }
                                }
                                builder.append(true);
                            }
                            _ => builder.append(false),
                        }
                    }
                    Ok(Arc::new(builder.finish()))
                }
                _ => {
                    let mut string_builder = ListBuilder::new(StringBuilder::new());
                    for value in values {
                        match value {
                            Some(Value::Array(arr)) => {
                                for elem in arr {
                                    string_builder.values().append_value(elem.to_string());
                                }
                                string_builder.append(true);
                            }
                            _ => string_builder.append(false),
                        }
                    }
                    Ok(Arc::new(string_builder.finish()))
                }
            }
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

        let array = build_array_from_values(values, field.data_type())?;
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
        assert_eq!(infer_arrow_type(&values), DataType::Float64);
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
        let array = build_array_from_values(values, &DataType::Boolean).unwrap();
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
    fn test_boolean_array_accepts_postgres_text() {
        let values = vec![
            Some(json!("t")),
            Some(json!("F")),
            Some(json!("TRUE")),
            Some(json!("maybe")),
        ];
        let array = build_array_from_values(values, &DataType::Boolean).unwrap();
        let b = array
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();
        assert!(b.value(0) && !b.value(1) && b.value(2));
        assert!(array.is_null(3));
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
        let array = build_array_from_values(values, &DataType::Utf8).unwrap();
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
        assert_eq!(types[2], DataType::Float64);
        assert_eq!(types[3], DataType::Boolean);
        assert_eq!(types[4], utf8_list_type());
        assert_eq!(types[5], DataType::Int32); // inferred
    }

    #[test]
    fn test_inferred_lists_are_always_utf8() {
        let values = vec![Some(json!([1, 2, 3]))];
        assert_eq!(infer_arrow_type(&values), utf8_list_type());
    }

    #[test]
    fn test_typed_builders_accept_numeric_strings_and_null_overflow() {
        let ints = build_array_from_values(
            vec![
                Some(json!("42")),
                Some(json!(5_000_000_000i64)),
                Some(json!("x")),
            ],
            &DataType::Int32,
        )
        .unwrap();
        assert_eq!(ints.null_count(), 2);
        let longs =
            build_array_from_values(vec![Some(json!(5_000_000_000i64))], &DataType::Int64).unwrap();
        assert_eq!(longs.null_count(), 0);
    }
}
