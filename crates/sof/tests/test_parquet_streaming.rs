//! The incremental Parquet / Arrow IPC writers must produce the same schema and
//! rows as the buffered formatters, and must honour size-based row groups.

use std::sync::Arc;

use arrow::compute::concat_batches;
use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use helios_sof::columnar_stream::{ArrowIpcStreamWriter, ParquetStreamWriter};
use helios_sof::parquet_schema::resolve_arrow_schema;
use helios_sof::{
    ParquetOptions, ProcessedResult, ProcessedRow, format_arrow_ipc, format_parquet,
    format_parquet_multi_file,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::json;

fn result(n: usize) -> ProcessedResult {
    ProcessedResult {
        columns: vec![
            "id".to_string(),
            "age".to_string(),
            "score".to_string(),
            "active".to_string(),
            "tags".to_string(),
        ],
        rows: (0..n)
            .map(|i| ProcessedRow {
                values: vec![
                    Some(json!(format!("patient-{i}-{}", "x".repeat(150)))),
                    Some(json!(i as i64)),
                    Some(json!(i as f64 + 0.5)),
                    if i % 3 == 0 {
                        None
                    } else {
                        Some(json!(i % 2 == 0))
                    },
                    Some(json!(["a", "b"])),
                ],
            })
            .collect(),
    }
}

fn read_parquet(bytes: Vec<u8>) -> (Arc<Schema>, RecordBatch, usize) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes)).unwrap();
    let groups = builder.metadata().num_row_groups();
    let schema = builder.schema().clone();
    let batches: Vec<RecordBatch> = builder.build().unwrap().map(|b| b.unwrap()).collect();
    let merged = concat_batches(&schema, &batches).unwrap();
    (schema, merged, groups)
}

fn incremental_parquet(r: &ProcessedResult, opts: &ParquetOptions) -> Vec<u8> {
    let schema = Arc::new(resolve_arrow_schema(&r.columns, &[], &r.rows).unwrap());
    let mut w = ParquetStreamWriter::new(Vec::new(), schema, opts).unwrap();
    // Feed in uneven slices, as a row stream would.
    for chunk in r.rows.chunks(777) {
        w.write_rows(chunk).unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn incremental_parquet_matches_buffered_schema_and_rows() {
    let r = result(5_000);
    let opts = ParquetOptions::default();
    let (buf_schema, buf_rows, _) = read_parquet(format_parquet(r.clone(), Some(&opts)).unwrap());
    let (inc_schema, inc_rows, _) = read_parquet(incremental_parquet(&r, &opts));
    assert_eq!(buf_schema, inc_schema);
    assert_eq!(buf_rows.num_rows(), 5_000);
    assert_eq!(buf_rows, inc_rows);
}

#[test]
fn small_row_group_size_yields_several_row_groups() {
    let r = result(30_000);
    let opts = ParquetOptions {
        row_group_size_mb: 1,
        compression: "none".to_string(),
        ..ParquetOptions::default()
    };
    let (_, rows, groups) = read_parquet(incremental_parquet(&r, &opts));
    assert_eq!(rows.num_rows(), 30_000);
    assert!(groups > 1, "expected several row groups, got {groups}");
}

#[test]
fn empty_result_keeps_the_full_schema() {
    let r = result(0);
    let (schema, rows, _) = read_parquet(incremental_parquet(&r, &ParquetOptions::default()));
    assert_eq!(rows.num_rows(), 0);
    let names: Vec<_> = schema.fields().iter().map(|f| f.name().clone()).collect();
    assert_eq!(names, r.columns);

    let files = format_parquet_multi_file(r, None, usize::MAX).unwrap();
    assert_eq!(files.len(), 1);
    assert!(!files[0].is_empty());
}

#[test]
fn incremental_arrow_ipc_matches_buffered() {
    let r = result(3_000);
    let schema = Arc::new(resolve_arrow_schema(&r.columns, &[], &r.rows).unwrap());
    let mut w = ArrowIpcStreamWriter::new(Vec::new(), schema).unwrap();
    for chunk in r.rows.chunks(500) {
        w.write_rows(chunk).unwrap();
    }
    let incremental = w.finish().unwrap();
    let buffered = format_arrow_ipc(r).unwrap();

    let read = |bytes: Vec<u8>| {
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
        let schema = reader.schema();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        concat_batches(&schema, &batches).unwrap()
    };
    assert_eq!(read(buffered), read(incremental));
}

/// Declared column types must reach the Parquet schema through a typed R4
/// ViewDefinition (not just through raw JSON), even when the first rows give
/// inference nothing to go on.
#[test]
#[cfg(feature = "R4")]
fn typed_view_definition_fixes_the_parquet_schema() {
    use arrow::datatypes::DataType;
    use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

    let view: helios_fhir::r4::ViewDefinition = serde_json::from_value(json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Observation",
        "select": [{ "column": [
            { "name": "id", "path": "id", "type": "id" },
            { "name": "n", "path": "valueInteger", "type": "integer" },
            { "name": "d", "path": "valueQuantity.value", "type": "decimal" },
            { "name": "b", "path": "valueBoolean", "type": "boolean" },
            { "name": "tags", "path": "code.coding.code", "type": "code", "collection": true }
        ]}]
    }))
    .unwrap();
    // The first observation has no value at all: inference alone would give
    // every typed column Utf8.
    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            { "resource": { "resourceType": "Observation", "id": "o1", "status": "final",
                "code": { "coding": [{ "code": "a" }] } } },
            { "resource": { "resourceType": "Observation", "id": "o2", "status": "final",
                "code": { "coding": [{ "code": "a" }, { "code": "b" }] },
                "valueInteger": 7, "valueBoolean": true,
                "valueQuantity": { "value": 2.5 } } }
        ]
    }))
    .unwrap();

    let bytes = run_view_definition(
        SofViewDefinition::R4(view),
        SofBundle::R4(bundle),
        ContentType::Parquet,
    )
    .unwrap();
    let (schema, _, _) = read_parquet(bytes);
    let ty = |n: &str| schema.field_with_name(n).unwrap().data_type().clone();
    assert_eq!(ty("n"), DataType::Int32);
    assert_eq!(ty("d"), DataType::Float64);
    assert_eq!(ty("b"), DataType::Boolean);
    assert!(matches!(ty("tags"), DataType::List(_)), "{:?}", ty("tags"));
}
