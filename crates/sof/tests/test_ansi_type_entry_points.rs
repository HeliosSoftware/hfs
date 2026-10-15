//! `ansi/type` column tag: every entry point must reject an invalid tag with an
//! error naming the column, and every valid tag must give its documented Arrow
//! type through both buffered columnar formats.

#[cfg(feature = "R4")]
mod tests {
    use arrow::array::{
        Array, BinaryArray, BooleanArray, Date32Array, Int32Array, Int64Array, StringArray,
        Time64MicrosecondArray, TimestampMicrosecondArray,
    };
    use arrow::datatypes::{DataType, TimeUnit};
    use arrow::ipc::reader::StreamReader;
    use arrow::record_batch::RecordBatch;
    use bytes::Bytes;
    use helios_sof::{
        ChunkConfig, ContentType, NdjsonChunkIterator, PreparedViewDefinition, ProcessedResult,
        SofBundle, SofError, SofViewDefinition, process_ndjson_chunked, run_view_definition,
        run_view_definition_record_batches,
    };
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use serde_json::{Value, json};
    use std::io::Cursor;

    fn vd(view: &Value) -> SofViewDefinition {
        let v: helios_fhir::r4::ViewDefinition = serde_json::from_value(view.clone()).unwrap();
        SofViewDefinition::R4(v)
    }

    fn bnd(bundle: &Value) -> SofBundle {
        let b: helios_fhir::r4::Bundle = serde_json::from_value(bundle.clone()).unwrap();
        SofBundle::R4(b)
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

    fn patient_bundle() -> Value {
        json!({"resourceType": "Bundle", "type": "collection", "entry": [
            {"resource": {"resourceType": "Patient", "id": "p1", "active": true,
                          "birthDate": "1990-01-01", "multipleBirthInteger": 3}}
        ]})
    }

    fn patient_ndjson() -> String {
        json!({"resourceType": "Patient", "id": "p1", "birthDate": "1990-01-01",
               "multipleBirthInteger": 3})
        .to_string()
    }

    /// A Patient view with a good column `ok` and a column `badcol` carrying
    /// the given `ansi/type` tag value.
    fn view_with_tag(path: &str, ty: &str, tag_value: Value) -> Value {
        json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
            "select": [{"column": [
                {"name": "ok", "path": "id"},
                {"name": "badcol", "path": path, "type": ty,
                 "tag": [{"name": "ansi/type", "value": tag_value}]}
            ]}]
        })
    }

    /// (path, declared type, tag value): unknown, incompatible, non-string.
    fn invalid_cases() -> Vec<(&'static str, &'static str, Value)> {
        vec![
            ("birthDate", "date", json!("GEOMETRY")),
            ("referenceRange.low.value", "decimal", json!("INT")),
            ("birthDate", "date", json!(42)),
        ]
    }

    /// Parses the view; a non-string tag value may already be refused by the
    /// typed ViewDefinition model, which is an acceptable rejection too (the
    /// error must still name the column or the offending value).
    fn try_vd(view: &Value) -> Result<SofViewDefinition, String> {
        serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view.clone())
            .map(SofViewDefinition::R4)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn invalid_tag_is_rejected_naming_the_column_for_every_content_type() {
        for (path, ty, tag) in invalid_cases() {
            let view = view_with_tag(path, ty, tag.clone());
            let Ok(_) = try_vd(&view) else {
                assert!(
                    tag.is_number(),
                    "only the non-string case may fail to parse"
                );
                continue;
            };
            for ct in [
                ContentType::Csv,
                ContentType::CsvWithHeader,
                ContentType::Json,
                ContentType::NdJson,
                ContentType::Parquet,
                ContentType::ArrowIpc,
            ] {
                let err = run_view_definition(vd(&view), bnd(&patient_bundle()), ct)
                    .expect_err(&format!("{ct:?} with tag {tag}"));
                assert!(
                    matches!(err, SofError::InvalidViewDefinition(_)),
                    "{ct:?} with tag {tag}: {err:?}"
                );
                assert!(
                    err.to_string().contains("badcol"),
                    "{ct:?} with tag {tag}: {err}"
                );
            }
        }
    }

    #[test]
    fn invalid_tag_is_rejected_by_record_batches() {
        for (path, ty, tag) in invalid_cases() {
            let view = view_with_tag(path, ty, tag.clone());
            if try_vd(&view).is_err() {
                continue;
            }
            let err = run_view_definition_record_batches(vd(&view), bnd(&patient_bundle()))
                .expect_err(&format!("tag {tag}"));
            assert!(
                matches!(err, SofError::InvalidViewDefinition(_))
                    && err.to_string().contains("badcol"),
                "tag {tag}: {err:?}"
            );
        }
    }

    #[test]
    fn invalid_tag_is_rejected_by_the_streaming_ndjson_path() {
        for (path, ty, tag) in invalid_cases() {
            let view = view_with_tag(path, ty, tag.clone());
            if try_vd(&view).is_err() {
                continue;
            }
            // PreparedViewDefinition::new validates.
            let err = PreparedViewDefinition::new(vd(&view))
                .err()
                .expect("prepare");
            assert!(err.to_string().contains("badcol"), "tag {tag}: {err}");

            // So does the iterator constructor...
            let err = NdjsonChunkIterator::new(
                vd(&view),
                Cursor::new(patient_ndjson()),
                ChunkConfig::default(),
            )
            .err()
            .expect("iterator");
            assert!(err.to_string().contains("badcol"), "tag {tag}: {err}");

            // ...and the chunked processor, for every streamable format.
            for ct in [
                ContentType::Csv,
                ContentType::CsvWithHeader,
                ContentType::Json,
                ContentType::NdJson,
            ] {
                let mut out = Vec::new();
                let err = process_ndjson_chunked(
                    vd(&view),
                    Cursor::new(patient_ndjson()),
                    &mut out,
                    ct,
                    ChunkConfig::default(),
                )
                .expect_err("chunked");
                assert!(
                    matches!(err, SofError::InvalidViewDefinition(_))
                        && err.to_string().contains("badcol"),
                    "{ct:?} tag {tag}: {err:?}"
                );
            }
        }
    }

    #[test]
    fn fhir_format_rejects_or_documents_invalid_tag() {
        // `_format=fhir` renders an already-computed result; the tag is
        // validated when the view is run, so a caller that runs first never
        // reaches the formatter. Pin that the run step is where it fails.
        let view = view_with_tag("birthDate", "date", json!("GEOMETRY"));
        let res = ProcessedResult {
            columns: vec!["ok".into(), "badcol".into()],
            rows: vec![],
        };
        match helios_sof::fhir_format::format_view_fhir_parameters(&res, &view) {
            Err(e) => assert!(e.to_string().contains("badcol"), "{e}"),
            Ok(_) => {
                // Formatter is type-agnostic: confirm the run rejects instead.
                let err = run_view_definition(vd(&view), bnd(&patient_bundle()), ContentType::Json)
                    .unwrap_err();
                assert!(err.to_string().contains("badcol"));
            }
        }
    }

    #[test]
    fn plural_tags_key_is_not_read() {
        // Intentional: the element is `tag` (singular). The spec's prose
        // example spells it `tags`, which is a typo, so a `tags` key is
        // ignored and the column keeps its default type even when the value
        // would otherwise be rejected.
        let view = json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
            "select": [{"column": [
                {"name": "b", "path": "birthDate", "type": "date",
                 "tags": [{"name": "ansi/type", "value": "GEOMETRY"}]}
            ]}]
        });
        // The typed model may drop the unknown key; go through it as the
        // production entry points do.
        let batch = read_parquet(
            run_view_definition(vd(&view), bnd(&patient_bundle()), ContentType::Parquet).unwrap(),
        );
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
        let s = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(s.value(0), "1990-01-01");
    }

    /// Runs a one-column Patient view with an `ansi/type` tag through both
    /// buffered columnar formats and returns the two batches.
    fn tagged(path: &str, ty: Option<&str>, tag: &str, bundle: &Value) -> [RecordBatch; 2] {
        let mut col = json!({"name": "c", "path": path,
                             "tag": [{"name": "ansi/type", "value": tag}]});
        if let Some(t) = ty {
            col["type"] = json!(t);
        }
        let view = json!({
            "resourceType": "ViewDefinition", "status": "active",
            "resource": bundle["entry"][0]["resource"]["resourceType"],
            "select": [{"column": [col]}]
        });
        let p = run_view_definition(vd(&view), bnd(bundle), ContentType::Parquet)
            .unwrap_or_else(|e| panic!("{tag}: {e}"));
        let i = run_view_definition(vd(&view), bnd(bundle), ContentType::ArrowIpc).unwrap();
        [read_parquet(p), read_ipc(i)]
    }

    fn one(resource: Value) -> Value {
        json!({"resourceType": "Bundle", "type": "collection",
               "entry": [{"resource": resource}]})
    }

    #[test]
    fn tag_time_gives_time64_microseconds() {
        let b = one(json!({"resourceType": "Patient", "id": "p1"}));
        for batch in tagged("'10:30:15'", Some("time"), "TIME", &b) {
            assert_eq!(
                batch.schema().field(0).data_type(),
                &DataType::Time64(TimeUnit::Microsecond)
            );
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<Time64MicrosecondArray>()
                .unwrap();
            assert_eq!(a.value(0), (10 * 3600 + 30 * 60 + 15) * 1_000_000);
        }
    }

    #[test]
    fn tag_timestamp_gives_naive_microseconds() {
        let b = one(
            json!({"resourceType": "Observation", "id": "o", "status": "final",
                           "code": {"text": "x"}, "effectiveDateTime": "2020-01-01T10:00:00"}),
        );
        for batch in tagged("effective", Some("dateTime"), "TIMESTAMP", &b) {
            assert_eq!(
                batch.schema().field(0).data_type(),
                &DataType::Timestamp(TimeUnit::Microsecond, None)
            );
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            assert_eq!(a.value(0), 1_577_872_800_000_000);
        }
    }

    #[test]
    fn tag_timestamp_with_time_zone_is_utc() {
        let b = one(
            json!({"resourceType": "Observation", "id": "o", "status": "final",
                           "code": {"text": "x"}, "issued": "2020-01-01T10:00:00+02:00"}),
        );
        for batch in tagged("issued", Some("instant"), "TIMESTAMP WITH TIME ZONE", &b) {
            assert_eq!(
                batch.schema().field(0).data_type(),
                &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
            );
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            assert_eq!(a.value(0), 1_577_865_600_000_000);
        }
    }

    #[test]
    fn tag_boolean_int_bigint_binary() {
        let p = one(
            json!({"resourceType": "Patient", "id": "p1", "active": true,
                           "multipleBirthInteger": 3}),
        );
        for batch in tagged("active", Some("boolean"), "BOOLEAN", &p) {
            assert_eq!(batch.schema().field(0).data_type(), &DataType::Boolean);
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            assert!(a.value(0));
        }
        for batch in tagged("multipleBirthInteger", Some("integer"), "INT", &p) {
            assert_eq!(batch.schema().field(0).data_type(), &DataType::Int32);
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap();
            assert_eq!(a.value(0), 3);
        }
        for batch in tagged("multipleBirthInteger", Some("integer"), "BIGINT", &p) {
            assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            assert_eq!(a.value(0), 3);
        }
        let m = one(
            json!({"resourceType": "Media", "id": "m1", "status": "completed",
                           "content": {"contentType": "text/plain", "data": "aGVsbG8="}}),
        );
        for batch in tagged("content.data", Some("base64Binary"), "BINARY", &m) {
            assert_eq!(batch.schema().field(0).data_type(), &DataType::Binary);
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            assert_eq!(a.value(0), b"hello");
        }
    }

    #[test]
    fn tag_without_declared_type_sets_the_arrow_type() {
        let p = one(json!({"resourceType": "Patient", "id": "p1", "birthDate": "1990-01-01"}));
        for batch in tagged("birthDate", None, "DATE", &p) {
            assert_eq!(batch.schema().field(0).data_type(), &DataType::Date32);
            let a = batch
                .column(0)
                .as_any()
                .downcast_ref::<Date32Array>()
                .unwrap();
            assert_eq!(a.value(0), 7305);
        }
    }

    #[test]
    fn invalid_base64_in_a_declared_column_names_the_column() {
        let view = json!({
            "resourceType": "ViewDefinition", "status": "active", "resource": "Media",
            "select": [{"column": [
                {"name": "data", "path": "content.data", "type": "base64Binary"}
            ]}]
        });
        let m = one(
            json!({"resourceType": "Media", "id": "m1", "status": "completed",
                           "content": {"contentType": "text/plain", "data": "@@not base64!!"}}),
        );
        for ct in [ContentType::Parquet, ContentType::ArrowIpc] {
            let err = run_view_definition(vd(&view), bnd(&m), ct).unwrap_err();
            assert!(err.to_string().contains("'data'"), "{ct:?}: {err}");
        }
    }
}
