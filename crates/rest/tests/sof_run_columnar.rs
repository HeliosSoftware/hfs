//! End-to-end tests for columnar `$sql-run` output (`_format=parquet` and
//! `_format=arrow`) served by the SQLite in-DB runner.
//!
//! The output is decoded and compared with `_format=json`. The all-null
//! `integer` column also pins the schema to the declared type, which only the
//! columnar path produces (the row path infers `Utf8` from the values).

mod sof_run_columnar_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use bytes::Bytes;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_sof::arrow::array::{Array, BooleanArray, Int32Array, RecordBatch, StringArray};
    use helios_sof::arrow::datatypes::DataType;
    use helios_sof::arrow::ipc::reader::StreamReader;
    use helios_sof::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use serde_json::{Value, json};
    use std::io::Cursor;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);
        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        (TestServer::new(app).expect("test server"), backend)
    }

    async fn seed_patients(backend: &SqliteBackend) {
        let tenant = TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        );
        for (id, family, active) in [
            ("p1", Some("Smith"), true),
            ("p2", Some("Jones"), false),
            ("p3", None, true),
        ] {
            let mut patient = json!({
                "resourceType": "Patient",
                "id": id,
                "active": active,
            });
            if let Some(f) = family {
                patient["name"] = json!([{ "family": f }]);
            }
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }
    }

    /// `multipleBirthInteger` is absent from every patient: the declared
    /// `integer` type is the only thing that can type the column.
    fn view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    { "path": "id", "name": "patient_id", "type": "string" },
                    { "path": "name.family", "name": "family", "type": "string" },
                    { "path": "active", "name": "active", "type": "boolean" },
                    { "path": "multipleBirthInteger", "name": "births", "type": "integer" }
                ]
            }]
        })
    }

    async fn run(server: &TestServer, format: &str) -> axum_test::TestResponse {
        server
            .post(&format!("/$sql-run?_format={format}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&view())
            .await
    }

    async fn json_rows(server: &TestServer) -> Vec<Value> {
        let response = run(server, "json").await;
        response.assert_status(StatusCode::OK);
        let mut rows: Vec<Value> = response.json();
        rows.sort_by_key(|r| r["patient_id"].as_str().unwrap_or_default().to_string());
        rows
    }

    fn read_arrow(bytes: &[u8]) -> Vec<RecordBatch> {
        StreamReader::try_new(Cursor::new(bytes.to_vec()), None)
            .expect("valid Arrow IPC stream")
            .map(|b| b.expect("readable batch"))
            .collect()
    }

    fn read_parquet(bytes: Vec<u8>) -> Vec<RecordBatch> {
        ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes))
            .expect("valid Parquet file")
            .build()
            .expect("reader")
            .map(|b| b.expect("readable batch"))
            .collect()
    }

    /// Checks schema and values of the decoded batches against the JSON rows.
    fn assert_matches_json(batches: &[RecordBatch], json_rows: &[Value]) {
        assert_eq!(batches.len(), 1, "three rows fit in a single batch");
        let batch = &batches[0];

        let schema = batch.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["patient_id", "family", "active", "births"]);
        let types: Vec<DataType> = schema
            .fields()
            .iter()
            .map(|f| f.data_type().clone())
            .collect();
        assert_eq!(
            types,
            [
                DataType::Utf8,
                DataType::Utf8,
                DataType::Boolean,
                DataType::Int32
            ],
            "declared column types, including the all-null integer column"
        );

        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let family = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let active = batch
            .column(2)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        let births = batch
            .column(3)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();

        assert_eq!(batch.num_rows(), json_rows.len());
        let mut order: Vec<usize> = (0..batch.num_rows()).collect();
        order.sort_by_key(|&i| ids.value(i).to_string());
        for (i, expected) in order.into_iter().zip(json_rows) {
            assert_eq!(Some(ids.value(i)), expected["patient_id"].as_str());
            let fam = (!family.is_null(i)).then(|| family.value(i));
            assert_eq!(fam, expected["family"].as_str());
            assert_eq!(Some(active.value(i)), expected["active"].as_bool());
            assert!(births.is_null(i));
            assert!(expected["births"].is_null());
        }
    }

    #[tokio::test]
    async fn parquet_output_matches_json_and_uses_declared_types() {
        let (server, backend) = create_test_server().await;
        seed_patients(&backend).await;
        let expected = json_rows(&server).await;
        assert_eq!(expected.len(), 3);

        let response = run(&server, "parquet").await;
        response.assert_status(StatusCode::OK);
        let body = response.as_bytes().to_vec();
        assert_eq!(&body[..4], b"PAR1");
        assert_matches_json(&read_parquet(body), &expected);
    }

    #[tokio::test]
    async fn arrow_output_matches_json_and_uses_declared_types() {
        let (server, backend) = create_test_server().await;
        seed_patients(&backend).await;
        let expected = json_rows(&server).await;

        let response = run(&server, "arrow").await;
        response.assert_status(StatusCode::OK);
        assert_matches_json(&read_arrow(response.as_bytes()), &expected);
    }

    /// A view that matches no rows still answers 200 with a valid, empty file.
    #[tokio::test]
    async fn empty_result_is_a_valid_empty_stream() {
        let (server, _backend) = create_test_server().await;

        let response = run(&server, "arrow").await;
        response.assert_status(StatusCode::OK);
        let batches = read_arrow(response.as_bytes());
        assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 0);

        let response = run(&server, "parquet").await;
        response.assert_status(StatusCode::OK);
        let batches = read_parquet(response.as_bytes().to_vec());
        assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 0);
    }
}
