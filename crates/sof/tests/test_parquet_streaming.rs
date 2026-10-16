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
    // `decimal` follows the SQL-on-FHIR default mapping: character data, so the
    // digits are kept (#569).
    assert_eq!(ty("d"), DataType::Utf8);
    assert_eq!(ty("b"), DataType::Boolean);
    assert!(matches!(ty("tags"), DataType::List(_)), "{:?}", ty("tags"));
}

/// What the maintainer asked for in review of #1989: `decimal` is `Utf8`,
/// `instant` is `Timestamp(µs, UTC)`, `base64Binary` is `Binary` and a
/// collection column is a typed list, through the incremental writers too.
#[test]
fn streaming_writers_follow_the_spec_type_table() {
    use arrow::array::{Array, BinaryArray, Int32Array, ListArray, StringArray};
    use arrow::datatypes::{DataType, TimeUnit};
    use helios_sof::sqlquery::engine::try_declared_columns;

    let view = json!({"select": [{"column": [
        {"name": "d", "path": "x", "type": "decimal"},
        {"name": "i", "path": "x", "type": "instant"},
        {"name": "b", "path": "x", "type": "base64Binary"},
        {"name": "ns", "path": "x", "type": "integer", "collection": true},
    ]}]});
    let declared = try_declared_columns(&view).unwrap();
    let r = ProcessedResult {
        columns: ["d", "i", "b", "ns"].map(String::from).to_vec(),
        rows: vec![ProcessedRow {
            values: vec![
                Some(json!(1.5)),
                Some(json!("2024-01-01T12:00:00+02:00")),
                Some(json!("aGVsbG8=")),
                Some(json!([1, 2, 3])),
            ],
        }],
    };
    let schema = Arc::new(resolve_arrow_schema(&r.columns, &declared, &r.rows).unwrap());
    assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        schema.field(1).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(schema.field(2).data_type(), &DataType::Binary);
    assert!(
        matches!(schema.field(3).data_type(), DataType::List(f) if f.data_type() == &DataType::Int32)
    );

    let mut w = ParquetStreamWriter::new(Vec::new(), schema, &ParquetOptions::default()).unwrap();
    w.write_rows(&r.rows).unwrap();
    let (_, batch, _) = read_parquet(w.finish().unwrap());

    let d = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(d.value(0), "1.5");
    let b = batch
        .column(2)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(b.value(0), b"hello");
    let ns = batch
        .column(3)
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap();
    let items = ns.value(0);
    let items = items.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(items.values(), &[1, 2, 3]);
    // 12:00 at +02:00 is 10:00 UTC.
    let micros = batch
        .column(1)
        .as_any()
        .downcast_ref::<arrow::array::TimestampMicrosecondArray>()
        .unwrap()
        .value(0);
    assert_eq!(micros, 1_704_103_200_000_000);
}

// ---------------------------------------------------------------------------
// Typed columns through the incremental writers (Parquet and Arrow IPC)
// ---------------------------------------------------------------------------

use arrow::array::{
    Array, BinaryArray, Date32Array, Int32Array, Int64Array, ListArray, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, TimeUnit};
use helios_sof::sqlquery::engine::try_declared_columns;

fn row(values: Vec<serde_json::Value>) -> ProcessedRow {
    ProcessedRow {
        values: values.into_iter().map(Some).collect(),
    }
}

/// Resolves the schema from `view`, then writes `rows` through BOTH incremental
/// writers and returns the two read-back batches (Parquet, Arrow IPC).
fn write_both(
    view: serde_json::Value,
    columns: &[&str],
    rows: Vec<ProcessedRow>,
) -> Result<[RecordBatch; 2], helios_sof::SofError> {
    let declared = try_declared_columns(&view).unwrap();
    let columns: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    let schema = Arc::new(resolve_arrow_schema(&columns, &declared, &rows).unwrap());

    let mut pw =
        ParquetStreamWriter::new(Vec::new(), schema.clone(), &ParquetOptions::default()).unwrap();
    pw.write_rows(&rows)?;
    let (_, parquet_batch, _) = read_parquet(pw.finish().unwrap());

    let mut iw = ArrowIpcStreamWriter::new(Vec::new(), schema).unwrap();
    iw.write_rows(&rows)?;
    let ipc = iw.finish().unwrap();
    let reader =
        arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(ipc), None).unwrap();
    let s = reader.schema();
    let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    Ok([parquet_batch, concat_batches(&s, &batches).unwrap()])
}

#[test]
fn incremental_writers_normalise_instant_fraction_and_offset() {
    let view = json!({"select": [{"column": [{"name": "i", "path": "x", "type": "instant"}]}]});
    let rows = vec![
        row(vec![json!("2024-01-01T12:00:00.123456+02:00")]),
        row(vec![json!("2024-01-01T12:00:00.5Z")]),
        row(vec![json!("2024-01-01T00:30:00-05:30")]),
    ];
    for batch in write_both(view, &["i"], rows).unwrap() {
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(a.value(0), 1_704_103_200_123_456); // 10:00:00.123456Z
        assert_eq!(a.value(1), 1_704_110_400_500_000); // 12:00:00.5Z
        assert_eq!(a.value(2), 1_704_088_800_000_000); // 06:00:00Z
    }
}

#[test]
fn incremental_writers_handle_integer64_list_of_strings_and_date_tag() {
    let view = json!({"select": [{"column": [
        {"name": "big", "path": "x", "type": "integer64"},
        {"name": "ss", "path": "x", "type": "string", "collection": true},
        {"name": "d", "path": "x", "type": "date",
         "tag": [{"name": "ansi/type", "value": "DATE"}]},
    ]}]});
    let rows = vec![row(vec![
        json!(5_000_000_000_i64),
        json!(["a", "b"]),
        json!("1990-01-01"),
    ])];
    for batch in write_both(view, &["big", "ss", "d"], rows).unwrap() {
        let big = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(big.value(0), 5_000_000_000);
        assert!(matches!(
            batch.schema().field(1).data_type(),
            DataType::List(f) if f.data_type() == &DataType::Utf8
        ));
        let ss = batch
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let items = ss.value(0);
        let items = items.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!((items.value(0), items.value(1)), ("a", "b"));
        assert_eq!(batch.schema().field(2).data_type(), &DataType::Date32);
        let d = batch
            .column(2)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert_eq!(d.value(0), 7305);
    }
}

#[test]
fn incremental_writers_are_strict_for_declared_and_lenient_for_inferred() {
    // Declared integer: 3_000_000_000 does not fit Int32 -> error naming `n`.
    let declared_view =
        json!({"select": [{"column": [{"name": "n", "path": "x", "type": "integer"}]}]});
    let declared = try_declared_columns(&declared_view).unwrap();
    let rows = vec![row(vec![json!(3_000_000_000_i64)])];
    let columns = vec!["n".to_string()];
    let schema = Arc::new(resolve_arrow_schema(&columns, &declared, &rows).unwrap());
    let mut pw =
        ParquetStreamWriter::new(Vec::new(), schema.clone(), &ParquetOptions::default()).unwrap();
    let err = pw.write_rows(&rows).unwrap_err();
    assert!(err.to_string().contains("'n'"), "{err}");
    let mut iw = ArrowIpcStreamWriter::new(Vec::new(), schema).unwrap();
    let err = iw.write_rows(&rows).unwrap_err();
    assert!(err.to_string().contains("'n'"), "{err}");

    // Inferred (undeclared) Int32 column: a later misfit becomes null.
    let schema = Arc::new(
        resolve_arrow_schema(
            &columns,
            &[],
            &[row(vec![json!(1)])], // schema sampled from a small value only
        )
        .unwrap(),
    );
    assert_eq!(schema.field(0).data_type(), &DataType::Int32);
    let rows = vec![row(vec![json!(1)]), row(vec![json!(3_000_000_000_i64)])];
    let mut pw =
        ParquetStreamWriter::new(Vec::new(), schema.clone(), &ParquetOptions::default()).unwrap();
    pw.write_rows(&rows).unwrap();
    let (_, batch, _) = read_parquet(pw.finish().unwrap());
    let a = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(a.value(0), 1);
    assert!(a.is_null(1), "an inferred column writes a misfit as null");
    let mut iw = ArrowIpcStreamWriter::new(Vec::new(), schema).unwrap();
    iw.write_rows(&rows).unwrap();
}

/// Mirror of `streaming_writers_follow_the_spec_type_table` for Arrow IPC.
#[test]
fn arrow_ipc_stream_writer_follows_the_spec_type_table() {
    let view = json!({"select": [{"column": [
        {"name": "d", "path": "x", "type": "decimal"},
        {"name": "i", "path": "x", "type": "instant"},
        {"name": "b", "path": "x", "type": "base64Binary"},
        {"name": "ns", "path": "x", "type": "integer", "collection": true},
    ]}]});
    let rows = vec![row(vec![
        json!(1.5),
        json!("2024-01-01T12:00:00+02:00"),
        json!("aGVsbG8="),
        json!([1, 2, 3]),
    ])];
    let [_, batch] = write_both(view, &["d", "i", "b", "ns"], rows).unwrap();
    let schema = batch.schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        schema.field(1).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(schema.field(2).data_type(), &DataType::Binary);
    assert!(
        matches!(schema.field(3).data_type(), DataType::List(f) if f.data_type() == &DataType::Int32)
    );
    let d = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(d.value(0), "1.5");
    let i = batch
        .column(1)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .unwrap();
    assert_eq!(i.value(0), 1_704_103_200_000_000);
    let b = batch
        .column(2)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(b.value(0), b"hello");
    let ns = batch
        .column(3)
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap();
    let items = ns.value(0);
    let items = items.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(items.values(), &[1, 2, 3]);
}
