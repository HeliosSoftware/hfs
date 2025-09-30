use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, ListBuilder, StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{ProcessedRow, SofError};

pub fn infer_arrow_type(values: &[Option<Value>]) -> DataType {
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut has_array = false;
    let mut has_object = false;
    let mut array_element_type = None;

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
            Value::Array(arr) => {
                has_array = true;
                if !arr.is_empty() && array_element_type.is_none() {
                    let element_values: Vec<Option<Value>> =
                        arr.iter().map(|v| Some(v.clone())).collect();
                    array_element_type = Some(infer_arrow_type(&element_values));
                }
            }
            Value::Object(_) => {
                has_object = true;
            }
            Value::Null => {}
        }
    }

    if has_array {
        if let Some(element_type) = array_element_type {
            return DataType::List(Arc::new(Field::new("item", element_type, true)));
        } else {
            return DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)));
        }
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

pub fn create_arrow_schema(columns: &[String], rows: &[ProcessedRow]) -> Result<Schema, SofError> {
    let sample_size = std::cmp::min(100, rows.len());
    let mut fields = Vec::new();

    for (col_idx, column_name) in columns.iter().enumerate() {
        let sample_values: Vec<Option<Value>> = rows
            .iter()
            .take(sample_size)
            .map(|row| row.values.get(col_idx).cloned().flatten())
            .collect();

        let data_type = infer_arrow_type(&sample_values);
        let field = Field::new(column_name, data_type, true);
        fields.push(field);
    }

    Ok(Schema::new(fields))
}

fn build_array_from_values(
    values: Vec<Option<Value>>,
    data_type: &DataType,
) -> Result<ArrayRef, SofError> {
    match data_type {
        DataType::Boolean => {
            let mut builder = BooleanBuilder::new();
            for value in values {
                match value {
                    Some(Value::Bool(b)) => builder.append_value(b),
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int32 => {
            let mut builder = Int32Builder::new();
            for value in values {
                match value {
                    Some(Value::Number(n)) if n.is_i64() => {
                        if let Some(i) = n.as_i64() {
                            builder.append_value(i as i32);
                        } else {
                            builder.append_null();
                        }
                    }
                    _ => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::new();
            for value in values {
                match value {
                    Some(Value::Number(n)) => {
                        if let Some(f) = n.as_f64() {
                            builder.append_value(f);
                        } else {
                            builder.append_null();
                        }
                    }
                    _ => builder.append_null(),
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
}
