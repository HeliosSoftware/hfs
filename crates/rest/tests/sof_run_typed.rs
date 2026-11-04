//! End-to-end tests for typed `$sql-run` output (#569): the spec column type
//! table, the `ansi/type` tag, strict declared columns and lenient inferred
//! ones, served by the SQLite in-DB runner (streaming Parquet / Arrow IPC).

mod sof_run_typed_tests {
    use arrow::array::{Array, RecordBatch};
    use arrow::datatypes::{DataType, TimeUnit};
    use arrow::ipc::reader::StreamReader;
    use arrow::util::display::array_value_to_string;
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use bytes::Bytes;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
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
        let runner = backend.sof_runner().expect("in-DB SOF runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        (TestServer::new(app).expect("test server"), backend)
    }

    async fn seed_observation(backend: &SqliteBackend, id: &str, resource: Value) {
        let tenant = TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        );
        let mut obs = json!({
            "resourceType": "Observation",
            "id": id,
            "status": "final",
            "code": {"text": "t"}
        });
        for (k, v) in resource.as_object().unwrap() {
            obs[k] = v.clone();
        }
        backend
            .create(&tenant, "Observation", obs, FhirVersion::R4)
            .await
            .expect("seed observation");
    }

    /// One fully populated observation.
    async fn seed_typed_observation(backend: &SqliteBackend) {
        seed_observation(
            backend,
            "o1",
            json!({
                "effectiveInstant": "2024-03-05T10:20:30+02:00",
                "valueAttachment": {
                    "data": "AQID",
                    "size": 5000000000i64,
                    "creation": "2024-03-05",
                    "title": "10:20:30",
                    "contentType": "2024-03-05T10:20:30"
                },
                "component": [
                    {"code": {"text": "a"}, "valueInteger": 7},
                    {"code": {"text": "b"}, "valueInteger": 8}
                ]
            }),
        )
        .await;
    }

    fn col(path: &str, name: &str, ty: &str) -> Value {
        json!({"path": path, "name": name, "type": ty})
    }

    fn tagged(path: &str, name: &str, ty: &str, tag: &str) -> Value {
        json!({"path": path, "name": name, "type": ty,
               "tag": [{"name": "ansi/type", "value": tag}]})
    }

    fn view(columns: Vec<Value>) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": columns}]
        })
    }

    async fn run(server: &TestServer, format: &str, view: &Value) -> axum_test::TestResponse {
        server
            .post(&format!("/$sql-run?_format={format}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(view)
            .await
    }

    fn decode(format: &str, bytes: Vec<u8>) -> Vec<RecordBatch> {
        if format == "arrow" {
            StreamReader::try_new(Cursor::new(bytes), None)
                .expect("valid Arrow IPC stream")
                .map(|b| b.expect("readable batch"))
                .collect()
        } else {
            ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes))
                .expect("valid Parquet file")
                .build()
                .expect("reader")
                .map(|b| b.expect("readable batch"))
                .collect()
        }
    }

    fn cell(batch: &RecordBatch, name: &str, row: usize) -> String {
        let column = batch.column_by_name(name).expect("column present");
        if column.is_null(row) {
            "NULL".to_string()
        } else if let Some(ts) = column
            .as_any()
            .downcast_ref::<arrow::array::TimestampMicrosecondArray>()
        {
            // The display helper cannot render the "UTC" zone name here.
            ts.value(row).to_string()
        } else {
            array_value_to_string(column, row).unwrap()
        }
    }

    fn typed_columns() -> Vec<Value> {
        vec![
            col("id", "obs_id", "string"),
            col("component.first().valueInteger", "small", "integer"),
            col("valueAttachment.size", "big", "integer64"),
            col("component.first().valueInteger", "dec", "decimal"),
            col("effectiveInstant", "inst", "instant"),
            col("valueAttachment.data", "bin", "base64Binary"),
            col("valueAttachment.creation", "day_default", "date"),
            tagged("valueAttachment.creation", "day_tag", "date", "DATE"),
            tagged("valueAttachment.title", "time_tag", "time", "TIME"),
            tagged(
                "valueAttachment.contentType",
                "ts_tag",
                "dateTime",
                "TIMESTAMP",
            ),
            json!({"path": "component.valueInteger", "name": "ints",
                   "type": "integer", "collection": true}),
        ]
    }

    async fn assert_typed_values(format: &str) {
        let (server, backend) = create_test_server().await;
        seed_typed_observation(&backend).await;
        let response = run(&server, format, &view(typed_columns())).await;
        response.assert_status_ok();
        let batches = decode(format, response.as_bytes().to_vec());
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        let schema = batch.schema();
        let ty = |n: &str| schema.field_with_name(n).unwrap().data_type().clone();
        assert_eq!(ty("obs_id"), DataType::Utf8);
        assert_eq!(ty("small"), DataType::Int32);
        assert_eq!(ty("big"), DataType::Int64);
        assert_eq!(ty("dec"), DataType::Utf8, "decimal is text");
        assert_eq!(
            ty("inst"),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(ty("bin"), DataType::Binary);
        assert_eq!(ty("day_default"), DataType::Utf8, "date defaults to text");
        assert_eq!(ty("day_tag"), DataType::Date32);
        assert_eq!(ty("time_tag"), DataType::Time64(TimeUnit::Microsecond));
        assert_eq!(
            ty("ts_tag"),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        match ty("ints") {
            DataType::List(f) => assert_eq!(f.data_type(), &DataType::Int32),
            other => panic!("expected List<Int32>, got {other:?}"),
        }

        assert_eq!(cell(batch, "obs_id", 0), "o1");
        assert_eq!(cell(batch, "small", 0), "7");
        assert_eq!(cell(batch, "big", 0), "5000000000");
        assert_eq!(cell(batch, "dec", 0), "7.0"); // SQLite REAL, rendered as text
        // 10:20:30+02:00 is 08:20:30 UTC (epoch micros), normalised.
        assert_eq!(cell(batch, "inst", 0), "1709626830000000");
        assert_eq!(cell(batch, "bin", 0), "010203");
        assert_eq!(cell(batch, "day_default", 0), "2024-03-05");
        assert_eq!(cell(batch, "day_tag", 0), "2024-03-05");
        assert_eq!(cell(batch, "time_tag", 0), "10:20:30");
        // No zone: the wall-clock value is kept as is (10:20:30).
        assert_eq!(cell(batch, "ts_tag", 0), "1709634030000000");
        assert_eq!(cell(batch, "ints", 0), "[7, 8]");
    }

    #[tokio::test]
    async fn parquet_values_follow_the_spec_type_table() {
        assert_typed_values("parquet").await;
    }

    #[tokio::test]
    async fn arrow_values_follow_the_spec_type_table() {
        assert_typed_values("arrow").await;
    }

    /// Decimals keep their lexical form as text.
    #[tokio::test]
    async fn decimal_text_keeps_the_lexical_form() {
        let (server, backend) = create_test_server().await;
        seed_observation(&backend, "o1", json!({"valueQuantity": {"value": 1.50}})).await;
        let v = view(vec![col("valueQuantity.value", "dec", "decimal")]);
        for format in ["parquet", "arrow"] {
            let response = run(&server, format, &v).await;
            response.assert_status_ok();
            let batches = decode(format, response.as_bytes().to_vec());
            let text = cell(&batches[0], "dec", 0);
            assert!(text == "1.5" || text == "1.50", "{format}: {text}");
        }
    }

    // ----- invalid tags -----------------------------------------------------

    const FORMATS: [&str; 6] = ["csv", "json", "ndjson", "parquet", "arrow", "fhir"];

    async fn assert_rejected_naming(server: &TestServer, v: &Value, column: &str) {
        for format in FORMATS {
            let response = run(server, format, v).await;
            assert!(
                response.status_code().is_client_error(),
                "{format}: {} {}",
                response.status_code(),
                response.text()
            );
            assert!(
                response.text().contains(column),
                "{format}: body must name '{column}': {}",
                response.text()
            );
        }
    }

    #[tokio::test]
    async fn unknown_tag_is_rejected_for_every_format_naming_the_column() {
        let (server, backend) = create_test_server().await;
        seed_typed_observation(&backend).await;
        let v = view(vec![
            col("id", "obs_id", "string"),
            tagged("id", "weird", "string", "NOT A TYPE"),
        ]);
        assert_rejected_naming(&server, &v, "weird").await;
    }

    #[tokio::test]
    async fn incompatible_tag_is_rejected_for_every_format_naming_the_column() {
        let (server, backend) = create_test_server().await;
        seed_typed_observation(&backend).await;
        let v = view(vec![
            col("id", "obs_id", "string"),
            tagged(
                "component.first().valueInteger",
                "dec_int",
                "decimal",
                "INT",
            ),
        ]);
        assert_rejected_naming(&server, &v, "dec_int").await;
    }

    #[tokio::test]
    async fn non_string_tag_value_is_rejected_naming_the_column() {
        let (server, backend) = create_test_server().await;
        seed_typed_observation(&backend).await;
        let v = view(vec![json!({
            "path": "id", "name": "numtag", "type": "string",
            "tag": [{"name": "ansi/type", "value": 5}]
        })]);
        // The lint rejects it before the column resolver, naming the tag path.
        assert_rejected_naming(&server, &v, "column[0].tag[0].value").await;
    }

    #[tokio::test]
    async fn invalid_tag_is_rejected_even_when_no_rows_match() {
        let (server, _backend) = create_test_server().await;
        let v = view(vec![tagged("id", "weird", "string", "NOT A TYPE")]);
        assert_rejected_naming(&server, &v, "weird").await;
    }

    // ----- strict declared / lenient inferred -------------------------------

    /// A declared misfit surfaces while the first batch is built, before any
    /// header is sent, so it is a 422 OperationOutcome naming the column (the
    /// caller's data/view mismatch, not an internal fault).
    async fn assert_declared_misfit(format: &str, v: &Value, column: &str) {
        let (server, backend) = create_test_server().await;
        seed_typed_observation(&backend).await;
        assert_misfit_422(&server, format, v, column).await;
    }

    async fn assert_misfit_422(server: &TestServer, format: &str, v: &Value, column: &str) {
        let response = run(server, format, v).await;
        assert_eq!(response.status_code(), 422, "{format}: {}", response.text());
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome", "{format}");
        assert!(
            response.text().contains(column),
            "{format}: body must name '{column}': {}",
            response.text()
        );
    }

    /// A declared `integer` value that does not fit in Int32 is an error on
    /// the in-DB columnar path too (it used to be written as NULL).
    #[tokio::test]
    async fn declared_integer_over_a_value_beyond_int32_is_an_error() {
        let v = view(vec![
            col("id", "obs_id", "string"),
            col("valueAttachment.size", "too_big", "integer"),
        ]);
        for format in ["parquet", "arrow"] {
            assert_declared_misfit(format, &v, "too_big").await;
        }
    }

    #[tokio::test]
    async fn declared_date_tag_over_a_partial_date_is_an_error() {
        let (server, backend) = create_test_server().await;
        seed_observation(
            &backend,
            "o1",
            json!({"valueAttachment": {"creation": "1980"}}),
        )
        .await;
        let v = view(vec![tagged(
            "valueAttachment.creation",
            "partial_day",
            "date",
            "DATE",
        )]);
        for format in ["parquet", "arrow"] {
            assert_misfit_422(&server, format, &v, "partial_day").await;
        }
    }
}
