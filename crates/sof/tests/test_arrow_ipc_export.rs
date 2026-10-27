//! Arrow IPC stream output (`ContentType::ArrowIpc`).
//!
//! The engine already builds Arrow RecordBatches internally for Parquet
//! output; these tests cover exposing them as the Arrow IPC stream format
//! (`application/vnd.apache.arrow.stream`) for live query results.

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use arrow::array::{Array, BooleanArray, ListArray, StringArray};
    use arrow::ipc::reader::StreamReader;
    use arrow::record_batch::RecordBatch;
    use helios_sof::{
        ChunkConfig, ContentType, SofBundle, SofViewDefinition, process_ndjson_chunked,
        run_view_definition,
    };
    use serde_json::json;

    fn patient_view() -> serde_json::Value {
        json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        { "name": "id", "path": "id" },
                        { "name": "gender", "path": "gender" },
                        { "name": "active", "path": "active" }
                    ]
                }
            ]
        })
    }

    fn patient_bundle() -> serde_json::Value {
        json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "gender": "male",
                        "active": true
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-2",
                        "gender": "female",
                        "active": false
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-3"
                    }
                }
            ]
        })
    }

    #[cfg(feature = "R4")]
    fn parse_r4(
        view: serde_json::Value,
        bundle: serde_json::Value,
    ) -> (SofViewDefinition, SofBundle) {
        let view_definition = serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view)
            .expect("Failed to parse ViewDefinition");
        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle)
            .expect("Failed to parse Bundle");
        (
            SofViewDefinition::R4(view_definition),
            SofBundle::R4(bundle),
        )
    }

    fn read_ipc_batches(bytes: &[u8]) -> Vec<RecordBatch> {
        let reader = StreamReader::try_new(Cursor::new(bytes), None)
            .expect("Output is not a valid Arrow IPC stream");
        reader
            .collect::<Result<Vec<_>, _>>()
            .expect("Failed to read Arrow IPC batches")
    }

    #[test]
    fn test_arrow_stream_mime_parses_to_arrow_ipc() {
        assert_eq!(
            ContentType::from_string("arrow").unwrap(),
            ContentType::ArrowIpc
        );
        assert_eq!(
            ContentType::from_string("application/vnd.apache.arrow.stream").unwrap(),
            ContentType::ArrowIpc
        );
        assert_eq!(
            ContentType::ArrowIpc.mime_type(),
            "application/vnd.apache.arrow.stream"
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_arrow_ipc_output_round_trips_values() {
        let (sof_view, sof_bundle) = parse_r4(patient_view(), patient_bundle());

        let bytes = run_view_definition(sof_view, sof_bundle, ContentType::ArrowIpc)
            .expect("Arrow IPC transformation failed");

        let batches = read_ipc_batches(&bytes);
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        let schema = batches[0].schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["id", "gender", "active"]);

        let first = &batches[0];
        let ids = first
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("id column should be Utf8");
        assert_eq!(ids.value(0), "patient-1");

        let genders = first
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("gender column should be Utf8");
        assert_eq!(genders.value(0), "male");
        assert_eq!(genders.value(1), "female");
        // patient-3 has no gender: nulls must survive the IPC crossing
        assert!(genders.is_null(2));

        let actives = first
            .column(2)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("active column should be Boolean");
        assert!(actives.value(0));
        assert!(!actives.value(1));
        assert!(actives.is_null(2));
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_arrow_ipc_matches_json_output_values() {
        let (sof_view, sof_bundle) = parse_r4(patient_view(), patient_bundle());
        let json_bytes = run_view_definition(sof_view, sof_bundle, ContentType::Json)
            .expect("JSON transformation failed");
        let json_rows: Vec<serde_json::Value> =
            serde_json::from_slice(&json_bytes).expect("Invalid JSON output");

        let (sof_view, sof_bundle) = parse_r4(patient_view(), patient_bundle());
        let bytes = run_view_definition(sof_view, sof_bundle, ContentType::ArrowIpc)
            .expect("Arrow IPC transformation failed");
        let batches = read_ipc_batches(&bytes);

        let ids = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for (i, row) in json_rows.iter().enumerate() {
            assert_eq!(ids.value(i), row["id"].as_str().unwrap());
        }
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_arrow_ipc_handles_multi_valued_columns() {
        let view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        { "name": "id", "path": "id" },
                        { "name": "given", "path": "name.given", "collection": true }
                    ]
                }
            ]
        });
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "name": [ { "given": ["Ada", "Byron"] } ]
                    }
                }
            ]
        });

        let (sof_view, sof_bundle) = parse_r4(view, bundle);
        let bytes = run_view_definition(sof_view, sof_bundle, ContentType::ArrowIpc)
            .expect("Arrow IPC transformation failed");
        let batches = read_ipc_batches(&bytes);

        let givens = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("collection column should be a List");
        let first = givens.value(0);
        let values = first
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("List items should be Utf8");
        assert_eq!(values.value(0), "Ada");
        assert_eq!(values.value(1), "Byron");
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_streaming_ndjson_rejects_arrow_ipc() {
        let view = serde_json::from_value::<helios_fhir::r4::ViewDefinition>(patient_view())
            .expect("Failed to parse ViewDefinition");
        let sof_view = SofViewDefinition::R4(view);

        let input = Cursor::new(b"{\"resourceType\":\"Patient\",\"id\":\"p1\"}\n".to_vec());
        let mut output: Vec<u8> = Vec::new();

        let result = process_ndjson_chunked(
            sof_view,
            input,
            &mut output,
            ContentType::ArrowIpc,
            ChunkConfig::default(),
        );

        match result {
            Err(helios_sof::SofError::UnsupportedContentType(msg)) => {
                assert!(msg.to_lowercase().contains("arrow"));
            }
            other => panic!("Expected UnsupportedContentType error, got: {:?}", other),
        }
    }
}
