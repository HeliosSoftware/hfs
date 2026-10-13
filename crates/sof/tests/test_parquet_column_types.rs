//! Parquet / Arrow IPC column types follow the SQL-on-FHIR 3.0.0-ballot default
//! mappings (`viewdef-6`/`viewdef-7`) and the `ansi/type` column tag.

#[cfg(feature = "R4")]
mod tests {
    use arrow::array::{
        Array, BinaryArray, BooleanArray, Date32Array, Int32Array, Int64Array, ListArray,
        StringArray, TimestampMicrosecondArray,
    };
    use arrow::datatypes::{DataType, TimeUnit};
    use arrow::ipc::reader::StreamReader;
    use arrow::record_batch::RecordBatch;
    use bytes::Bytes;
    use helios_sof::{ContentType, SofBundle, SofError, SofViewDefinition, run_view_definition};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use serde_json::{Value, json};
    use std::io::Cursor;

    fn run(view: Value, bundle: Value, ct: ContentType) -> Result<Vec<u8>, SofError> {
        let vd: helios_fhir::r4::ViewDefinition = serde_json::from_value(view).unwrap();
        let b: helios_fhir::r4::Bundle = serde_json::from_value(bundle).unwrap();
        run_view_definition(SofViewDefinition::R4(vd), SofBundle::R4(b), ct)
    }

    fn read_parquet(bytes: Vec<u8>) -> RecordBatch {
        let reader = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes))
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    fn read_ipc(bytes: Vec<u8>) -> RecordBatch {
        let reader = StreamReader::try_new(Cursor::new(bytes), None).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    fn bundle(resources: Vec<Value>) -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": resources.into_iter().map(|r| json!({"resource": r})).collect::<Vec<_>>()
        })
    }

    fn observation_view(columns: Value) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Observation",
            "select": [{"column": columns}]
        })
    }

    fn observations() -> Value {
        bundle(vec![
            json!({
                "resourceType": "Observation", "id": "o1", "status": "final",
                "code": {"text": "x"},
                "issued": "2020-01-01T10:00:00+02:00",
                "valueInteger": 2,
                "referenceRange": [{"low": {"value": 2.5}}],
            }),
            json!({
                "resourceType": "Observation", "id": "o2", "status": "final",
                "code": {"text": "x"},
                "issued": "2021-06-15T12:30:45.123Z",
                "valueInteger": 100,
                "referenceRange": [{"low": {"value": 100}}]
            }),
        ])
    }

    fn typed_columns() -> Value {
        json!([
            {"name": "id", "path": "id", "type": "id"},
            {"name": "dec", "path": "referenceRange.low.value", "type": "decimal"},
            {"name": "inst", "path": "issued", "type": "instant"},
            // 100 * 100_000_000 does not fit in 32 bits.
            {"name": "big", "path": "valueInteger * 100000000",
             "type": "integer64"},
            {"name": "small", "path": "valueInteger", "type": "integer"},
            {"name": "flag", "path": "valueInteger > 50", "type": "boolean"}
        ])
    }

    fn check_default_mappings(batch: &RecordBatch) {
        let schema = batch.schema();
        let ty = |n: &str| schema.field_with_name(n).unwrap().data_type().clone();
        assert_eq!(ty("id"), DataType::Utf8);
        assert_eq!(ty("dec"), DataType::Utf8, "decimal is character data");
        assert_eq!(
            ty("inst"),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(ty("big"), DataType::Int64);
        assert_eq!(ty("small"), DataType::Int32);
        assert_eq!(ty("flag"), DataType::Boolean);

        let col = |n: &str| batch.column(schema.index_of(n).unwrap()).clone();
        let dec = col("dec");
        let dec = dec.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(dec.value(0), "2.5");
        assert_eq!(dec.value(1), "100");

        let inst = col("inst");
        let inst = inst
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        // 2020-01-01T10:00:00+02:00 is normalised to 08:00:00Z.
        assert_eq!(inst.value(0), 1_577_865_600_000_000);
        // 2021-06-15T12:30:45.123Z
        assert_eq!(inst.value(1), 1_623_760_245_123_000);

        let big = col("big");
        let big = big.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(big.value(1), 10_000_000_000);

        let small = col("small");
        let small = small.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(small.value(1), 100);

        let flag = col("flag");
        let flag = flag.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert!(!flag.value(0));
        assert!(flag.value(1));
    }

    #[test]
    fn parquet_uses_the_spec_default_types() {
        let bytes = run(
            observation_view(typed_columns()),
            observations(),
            ContentType::Parquet,
        )
        .unwrap();
        check_default_mappings(&read_parquet(bytes));
    }

    #[test]
    fn arrow_ipc_uses_the_spec_default_types() {
        let bytes = run(
            observation_view(typed_columns()),
            observations(),
            ContentType::ArrowIpc,
        )
        .unwrap();
        check_default_mappings(&read_ipc(bytes));
    }

    #[test]
    fn integer64_beyond_i32_is_not_truncated_when_untyped() {
        let view = observation_view(json!([
            {"name": "big", "path": "valueInteger * 100000000"}
        ]));
        let batch = read_parquet(run(view, observations(), ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
        let big = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(big.value(1), 10_000_000_000);
    }

    #[test]
    fn untyped_fractional_number_is_character_data() {
        let view = observation_view(json!([{"name": "v", "path": "referenceRange.low.value"}]));
        let batch = read_parquet(run(view, observations(), ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
    }

    #[test]
    fn base64_binary_is_decoded_to_binary() {
        let view = json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Media",
            "select": [{"column": [
                {"name": "data", "path": "content.data", "type": "base64Binary"}
            ]}]
        });
        let b = bundle(vec![json!({
            "resourceType": "Media", "id": "m1", "status": "completed",
            "content": {"contentType": "text/plain", "data": "aGVsbG8="}
        })]);
        let batch = read_parquet(run(view, b, ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Binary);
        let data = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(data.value(0), b"hello");
    }

    #[test]
    fn collection_of_integers_is_a_list_of_int32() {
        let view = json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
            "select": [{"column": [
                {"name": "n", "path": "multipleBirthInteger", "type": "integer",
                 "collection": true}
            ]}]
        });
        let b = bundle(vec![
            json!({"resourceType": "Patient", "id": "p1", "multipleBirthInteger": 2}),
            json!({"resourceType": "Patient", "id": "p2"}),
        ]);
        for ct in [ContentType::Parquet, ContentType::ArrowIpc] {
            let bytes = run(view.clone(), b.clone(), ct).unwrap();
            let batch = if ct == ContentType::Parquet {
                read_parquet(bytes)
            } else {
                read_ipc(bytes)
            };
            let DataType::List(f) = batch.schema().field(0).data_type().clone() else {
                panic!("expected a list column");
            };
            assert_eq!(f.data_type(), &DataType::Int32);
            let list = batch
                .column(0)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();
            let first = list.value(0);
            assert_eq!(
                first
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .value(0),
                2
            );
            assert_eq!(list.value(1).len(), 0);
        }
    }

    fn patient_view_with_tag(path: &str, fhir_type: &str, tag: &str) -> Value {
        json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
            "select": [{"column": [{
                "name": "c", "path": path, "type": fhir_type,
                "tag": [{"name": "ansi/type", "value": tag}]
            }]}]
        })
    }

    fn patients(birth_date: &str) -> Value {
        bundle(vec![
            json!({"resourceType": "Patient", "id": "p1", "birthDate": birth_date,
                   "multipleBirthInteger": 3}),
        ])
    }

    #[test]
    fn ansi_type_date_tag_gives_a_native_date() {
        let view = patient_view_with_tag("birthDate", "date", "DATE");
        let batch = read_parquet(run(view, patients("1990-01-01"), ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Date32);
        let d = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        // 1990-01-01 is 7305 days after the epoch.
        assert_eq!(d.value(0), 7305);
    }

    #[test]
    fn ansi_type_varchar_tag_renders_an_integer_as_text() {
        let view = patient_view_with_tag("multipleBirthInteger", "integer", "varchar");
        let batch = read_parquet(run(view, patients("1990-01-01"), ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
        let s = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(s.value(0), "3");
    }

    #[test]
    fn ansi_type_double_tag_restores_a_float_column() {
        let view = observation_view(json!([{
            "name": "dec", "path": "referenceRange.low.value", "type": "decimal",
            "tag": [{"name": "ansi/type", "value": "DOUBLE PRECISION"}]
        }]));
        let batch = read_parquet(run(view, observations(), ContentType::Parquet).unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Float64);
    }

    #[test]
    fn unknown_or_incompatible_ansi_type_is_rejected() {
        for (path, ty, tag) in [
            ("birthDate", "date", "GEOMETRY"),
            ("multipleBirthInteger", "integer", "DATE"),
            ("birthDate", "date", "INT"),
        ] {
            let err = run(
                patient_view_with_tag(path, ty, tag),
                patients("1990-01-01"),
                ContentType::Parquet,
            )
            .unwrap_err();
            assert!(
                matches!(err, SofError::InvalidViewDefinition(_)),
                "{tag} on {ty}: {err:?}"
            );
        }
    }

    #[test]
    fn partial_date_does_not_fit_a_native_date_and_names_the_column() {
        let view = patient_view_with_tag("birthDate", "date", "DATE");
        let err = run(view, patients("1980"), ContentType::Parquet).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("'c'") && msg.contains("DATE"), "{msg}");
    }

    #[test]
    fn integer_outside_i32_range_is_an_error_not_a_wrap() {
        let view = observation_view(json!([{
            "name": "n", "path": "valueInteger * 100000000",
            "type": "integer"
        }]));
        let err = run(view, observations(), ContentType::Parquet).unwrap_err();
        assert!(err.to_string().contains("'n'"), "{err}");
    }
}
