//! Regression tests for issue #1811: integer values outside the `Int32` range
//! must reach Parquet and Arrow IPC output unchanged, never wrapped or nulled
//! (a u64 above `i64::MAX` fits no Arrow integer type and is written as null).

use arrow::array::{Array, Int32Array, Int64Array};
use arrow::datatypes::DataType;
use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;
use helios_sof::{ProcessedResult, ProcessedRow, format_arrow_ipc, format_parquet};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};

fn result_with(column: &str, values: Vec<Option<Value>>) -> ProcessedResult {
    ProcessedResult {
        columns: vec![column.to_string()],
        rows: values
            .into_iter()
            .map(|v| ProcessedRow { values: vec![v] })
            .collect(),
    }
}

fn read_parquet(bytes: Vec<u8>) -> Vec<RecordBatch> {
    ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
        .expect("parquet metadata")
        .build()
        .expect("parquet reader")
        .map(|b| b.expect("record batch"))
        .collect()
}

fn read_arrow_ipc(bytes: Vec<u8>) -> Vec<RecordBatch> {
    StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .expect("arrow ipc reader")
        .map(|b| b.expect("record batch"))
        .collect()
}

fn int64_values(batches: &[RecordBatch]) -> Vec<Option<i64>> {
    batches
        .iter()
        .flat_map(|b| {
            let col = b
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64 column");
            (0..col.len())
                .map(|i| (!col.is_null(i)).then(|| col.value(i)))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn parquet_integer_column_with_values_outside_i32_is_written_as_int64() {
    let values = vec![
        Some(json!(3_000_000_000_i64)),
        Some(json!(-3_000_000_000_i64)),
        None,
        Some(json!(42)),
    ];
    let bytes = format_parquet(result_with("total", values), None).expect("parquet");

    let batches = read_parquet(bytes);
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(
        int64_values(&batches),
        vec![Some(3_000_000_000), Some(-3_000_000_000), None, Some(42)]
    );
}

#[test]
fn parquet_widens_when_the_large_value_is_past_the_inference_sample() {
    // Schema inference samples the first 100 rows; the width must still be
    // right for a value that only appears on row 101.
    let mut values: Vec<Option<Value>> = (0..100).map(|i| Some(json!(i))).collect();
    values.push(Some(json!(3_000_000_000_i64)));
    let bytes = format_parquet(result_with("n", values), None).expect("parquet");

    let batches = read_parquet(bytes);
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Int64);
    let read = int64_values(&batches);
    assert_eq!(read.len(), 101);
    assert_eq!(read[100], Some(3_000_000_000));
    assert_eq!(read[99], Some(99));
}

#[test]
fn parquet_integer_column_that_fits_i32_stays_int32() {
    let values = vec![Some(json!(i32::MAX)), Some(json!(i32::MIN)), Some(json!(0))];
    let bytes = format_parquet(result_with("n", values), None).expect("parquet");

    let batches = read_parquet(bytes);
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Int32);
    let col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(col.values(), &[i32::MAX, i32::MIN, 0]);
}

#[test]
fn parquet_integer_above_i64_max_is_null_not_wrapped() {
    // No Arrow integer type holds a u64 above i64::MAX; like any value that
    // does not fit its column type it is written as null.
    let values = vec![Some(json!(u64::MAX)), Some(json!(1))];
    let bytes = format_parquet(result_with("n", values), None).expect("parquet");

    let batches = read_parquet(bytes);
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(int64_values(&batches), vec![None, Some(1)]);
}

#[test]
fn arrow_ipc_integer_column_with_values_outside_i32_is_written_as_int64() {
    let values = vec![
        Some(json!(3_000_000_000_i64)),
        Some(json!(-3_000_000_000_i64)),
        None,
    ];
    let bytes = format_arrow_ipc(result_with("total", values)).expect("arrow ipc");

    let batches = read_arrow_ipc(bytes);
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(
        int64_values(&batches),
        vec![Some(3_000_000_000), Some(-3_000_000_000), None]
    );
}
