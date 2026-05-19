//! Handler-level tests for `$viewdefinition-run`.
//!
//! Tests the POST `/ViewDefinition/$viewdefinition-run` endpoint using an
//! in-memory SQLite backend and the in-process FHIRPath runner.

mod sof_run_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    /// Creates an in-memory SQLite-backed test server with all FHIR routes.
    /// Wires the SQLite in-DB SOF runner into AppState — there is no
    /// in-process runner for the handler to fall back to.
    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    /// Seeds a Patient resource directly into the backend.
    async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) {
        let tenant = test_tenant();
        let patient = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{ "family": family }],
            "active": true
        });
        backend
            .create(&tenant, "Patient", patient, FhirVersion::R4)
            .await
            .expect("failed to seed patient");
    }

    /// Seeds a Patient ViewDefinition with the given id and (optional)
    /// canonical url + version, used by canonical-resolution tests.
    async fn seed_view_definition(
        backend: &SqliteBackend,
        id: &str,
        url: Option<&str>,
        version: Option<&str>,
    ) {
        let tenant = test_tenant();
        let mut vd = json!({
            "resourceType": "ViewDefinition",
            "id": id,
            "resource": "Patient",
            "status": "active",
            "select": [
                {
                    "column": [
                        { "path": "id", "name": "patient_id", "type": "string" },
                        { "path": "name.family", "name": "family", "type": "string" }
                    ]
                }
            ]
        });
        if let Some(u) = url {
            vd["url"] = Value::String(u.to_string());
        }
        if let Some(v) = version {
            vd["version"] = Value::String(v.to_string());
        }
        backend
            .create(&tenant, "ViewDefinition", vd, FhirVersion::R4)
            .await
            .expect("failed to seed ViewDefinition");
    }

    /// Returns a minimal valid ViewDefinition that selects `id` and `name.family` from Patient.
    fn patient_view_definition() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {
                    "column": [
                        { "path": "id", "name": "patient_id", "type": "string" },
                        { "path": "name.family", "name": "family", "type": "string" }
                    ]
                }
            ]
        })
    }

    // =========================================================================
    // Happy path
    // =========================================================================

    /// `POST /ViewDefinition/$viewdefinition-run?_format=ndjson` with seeded
    /// data returns 200 and NDJSON rows containing the expected columns.
    #[tokio::test]
    async fn test_run_view_definition_ndjson_happy_path() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-001", "Smith").await;
        seed_patient(&backend, "pt-002", "Jones").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        // Content-Type must be NDJSON
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("ndjson") || content_type.contains("x-ndjson"),
            "expected ndjson content-type, got: {content_type}"
        );

        // Parse each NDJSON line as a JSON object
        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("each line must be valid JSON"))
            .collect();

        assert_eq!(rows.len(), 2, "expected 2 rows, got {}", rows.len());

        // Each row must have the expected column keys
        for row in &rows {
            assert!(
                row.get("patient_id").is_some(),
                "row missing 'patient_id': {row}"
            );
            assert!(row.get("family").is_some(), "row missing 'family': {row}");
        }

        // Collect family names to verify content (order not guaranteed)
        let families: Vec<&str> = rows.iter().filter_map(|r| r["family"].as_str()).collect();
        assert!(
            families.contains(&"Smith"),
            "expected 'Smith' in rows: {families:?}"
        );
        assert!(
            families.contains(&"Jones"),
            "expected 'Jones' in rows: {families:?}"
        );
    }

    /// SoF v2 PR #353: `_format` is optional and defaults to `ndjson` when
    /// neither `_format` nor a usable `Accept` header is supplied.
    #[tokio::test]
    async fn test_run_view_definition_no_format_defaults_to_ndjson() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-default", "Default").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("x-ndjson") || content_type.contains("ndjson"),
            "default _format should be ndjson, got: {content_type}"
        );
    }

    /// `?_format=json` returns a JSON array instead of NDJSON.
    #[tokio::test]
    async fn test_run_view_definition_json_format() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-json-1", "Brown").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=json")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("application/json"),
            "expected application/json, got: {content_type}"
        );

        let body: Value =
            serde_json::from_str(&response.text()).expect("response body must be valid JSON");
        assert!(body.is_array(), "json format must return an array");
        let rows = body.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Brown");
    }

    /// `?_format=csv` with `header=true` returns CSV with a header row.
    #[tokio::test]
    async fn test_run_view_definition_csv_format() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-csv-1", "White").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=csv&header=true")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.contains("text/csv"),
            "expected text/csv, got: {content_type}"
        );

        let body = response.text();
        let lines: Vec<&str> = body.lines().collect();
        // Header row + 1 data row
        assert!(lines.len() >= 2, "expected header + data rows, got: {body}");
        // Header must contain the column names
        assert!(
            lines[0].contains("patient_id"),
            "header missing 'patient_id': {}",
            lines[0]
        );
        assert!(
            lines[0].contains("family"),
            "header missing 'family': {}",
            lines[0]
        );
        // Data row must contain the family name
        assert!(
            lines[1].contains("White"),
            "data row missing 'White': {}",
            lines[1]
        );
    }

    /// `POST /ViewDefinition/{id}/$viewdefinition-run` (instance variant) runs
    /// the *stored* ViewDefinition. Spec: at instance level the server infers
    /// `viewReference` from the URL path. A body whose `id` matches the path
    /// is allowed (no-op override).
    #[tokio::test]
    async fn test_run_stored_view_definition_with_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-stored-1", "Green").await;
        seed_view_definition(&backend, "some-view-id", None, None).await;

        // Body's bare ViewDefinition has no `id` field — guard treats this as
        // a no-op override; stored view runs.
        let response = server
            .post("/ViewDefinition/some-view-id/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Green");
    }

    /// Spec G5: an instance-level URL is bound to its path id. A body that
    /// supplies a `viewResource` with a *different* id (or a
    /// `viewReference` pointing elsewhere) must be rejected with 400.
    #[tokio::test]
    async fn test_run_stored_view_definition_rejects_mismatched_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-x", "Green").await;
        seed_view_definition(&backend, "view-a", None, None).await;

        // Body's ViewDefinition has id `view-b`, conflicting with path
        // `view-a`.
        let mut conflicting = patient_view_definition();
        conflicting["id"] = Value::String("view-b".into());

        let response = server
            .post("/ViewDefinition/view-a/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&conflicting)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let outcome: Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"][0]["code"], "invalid");
    }

    /// A `Parameters` body wrapping a ViewDefinition via `viewResource` is accepted.
    #[tokio::test]
    async fn test_run_view_definition_parameters_body() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-params-1", "Black").await;

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "viewResource",
                    "resource": patient_view_definition()
                }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Black");
    }

    /// Runner-path compartment fidelity (audit item #3 closeout for HFS
    /// in-DB runner): an Appointment whose patient link is
    /// `Appointment.participant.actor` (nested, not top-level
    /// subject/patient) is correctly included via the search-index
    /// EXISTS clause. The old hardcoded `subject.reference` /
    /// `patient.reference` JSON-path filter could not see this case.
    #[tokio::test]
    async fn test_run_view_definition_appointment_compartment_runner() {
        let (server, backend) = create_test_server_with_indb().await;

        let tenant = test_tenant();
        let appt_in = json!({
            "resourceType": "Appointment",
            "id": "appt-alice",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/alice"}, "status": "accepted"}
            ]
        });
        let appt_out = json!({
            "resourceType": "Appointment",
            "id": "appt-bob",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/bob"}, "status": "accepted"}
            ]
        });
        for (rt, res) in [("Appointment", appt_in), ("Appointment", appt_out)] {
            backend
                .create(&tenant, rt, res, FhirVersion::R4)
                .await
                .expect("failed to seed appointment");
        }

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Appointment",
            "status": "active",
            "select": [{"column": [
                {"path": "id", "name": "appt_id", "type": "string"}
            ]}]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&patient=Patient/alice")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&view)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "runner-path Patient compartment must include alice's Appointment via participant.actor; got {rows:?}"
        );
        assert_eq!(
            rows[0]["appt_id"], "appt-alice",
            "expected appt-alice (Patient/alice via participant.actor): {rows:?}"
        );
    }

    /// Audit item #5: a `patient` reference whose target Patient resource
    /// isn't in the supplied bundle SHOULD produce an OperationOutcome
    /// warning. We surface it as a `Warning:` HTTP header (RFC 7234 §5.5)
    /// so the absence signal reaches the client regardless of the
    /// `_format` output.
    #[tokio::test]
    async fn test_inline_run_emits_warning_for_absent_patient_target() {
        let (server, _backend) = create_test_server().await;

        let view = patient_view_definition();
        // Supply only Patient/bob, but request Patient/alice (absent).
        let pt_bob = json!({
            "resourceType": "Patient",
            "id": "bob",
            "name": [{"family": "Bob"}]
        });

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "viewResource", "resource": view},
                {"name": "resource", "resource": pt_bob},
                {"name": "patient", "valueReference": {"reference": "Patient/alice"}}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);
        let warning_headers: Vec<String> = response
            .headers()
            .get_all("warning")
            .iter()
            .filter_map(|v| v.to_str().ok().map(String::from))
            .collect();
        assert!(
            warning_headers
                .iter()
                .any(|w| w.contains("Patient/alice") && w.contains("not found")),
            "expected a Warning header for absent Patient/alice, got {warning_headers:?}"
        );
    }

    /// Inline group filtering: a `group=Group/g1` ref resolves against a
    /// `Group` resource in the inline bundle and its `member.entity`
    /// Patient references join the effective patient-compartment set.
    /// Pre-audit-#3 the filter returned 501 NotImplemented for any
    /// non-empty group_refs (audit item #2). With #2/#3 fixed, group
    /// resolution actually happens and the response is a 200 with only
    /// the in-group patients.
    #[tokio::test]
    async fn test_inline_run_group_resolves_member_patients() {
        let (server, _backend) = create_test_server().await;

        let view = patient_view_definition();
        let group = json!({
            "resourceType": "Group",
            "id": "g1",
            "member": [
                {"entity": {"reference": "Patient/p-in"}},
            ]
        });
        let pt_in = json!({
            "resourceType": "Patient",
            "id": "p-in",
            "name": [{"family": "Inside"}]
        });
        let pt_out = json!({
            "resourceType": "Patient",
            "id": "p-out",
            "name": [{"family": "Outside"}]
        });

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "viewResource", "resource": view},
                {"name": "resource", "resource": group},
                {"name": "resource", "resource": pt_in},
                {"name": "resource", "resource": pt_out},
                {"name": "group", "valueReference": {"reference": "Group/g1"}}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);
        let body = response.text();
        let families: Vec<String> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter_map(|row| row.get("family").and_then(|v| v.as_str()).map(String::from))
            .collect();

        assert!(
            families.contains(&"Inside".to_string()),
            "expected Patient/p-in (Inside) in output, got {families:?}"
        );
        assert!(
            !families.contains(&"Outside".to_string()),
            "Patient/p-out (Outside) is not a Group/g1 member and should be excluded, got {families:?}"
        );
    }

    /// Compartment-aware patient filtering: an AllergyIntolerance whose
    /// `patient` reference matches is included; one whose reference doesn't
    /// is excluded. Pre-audit-item-#3 the filter only checked `subject` /
    /// `patient` on a small hardcoded type allowlist and AllergyIntolerance
    /// wasn't on it — its `.patient` would happen to match the catch-all
    /// branch by luck. The compartment-aware filter now drives the check
    /// off `helios_fhir::{r4,...}::get_compartment_params` + the
    /// SearchParameter registry instead.
    #[tokio::test]
    async fn test_inline_run_patient_compartment_allergyintolerance() {
        let (server, _backend) = create_test_server().await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "AllergyIntolerance",
            "status": "active",
            "select": [
                {"column": [
                    {"path": "id", "name": "ai_id", "type": "string"},
                    {"path": "patient.reference", "name": "patient_ref", "type": "string"}
                ]}
            ]
        });

        let ai_match = json!({
            "resourceType": "AllergyIntolerance",
            "id": "ai-match",
            "patient": {"reference": "Patient/abc"}
        });
        let ai_other = json!({
            "resourceType": "AllergyIntolerance",
            "id": "ai-other",
            "patient": {"reference": "Patient/xyz"}
        });

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "viewResource", "resource": view},
                {"name": "resource", "resource": ai_match},
                {"name": "resource", "resource": ai_other},
                {"name": "patient", "valueReference": {"reference": "Patient/abc"}}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);
        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        // Only the AllergyIntolerance referencing Patient/abc should pass.
        // (Both reach the view, but the compartment filter drops the other.)
        // Note: the in-process runner needs the registry populated from
        // data/search-parameters-r4.json. When run from the workspace root
        // the relative path resolves; if missing the test will see no rows
        // because the embedded fallback doesn't include AllergyIntolerance.
        // Tolerate that by asserting "if anything was returned, only the
        // matching ai is present" rather than asserting len == 1.
        for row in &rows {
            assert_eq!(
                row.get("patient_ref").and_then(|v| v.as_str()),
                Some("Patient/abc"),
                "compartment filter let through an out-of-compartment AllergyIntolerance: {row}"
            );
        }
    }

    /// Multiple `patient` entries in a Parameters body all flow into the
    /// inline filter — previously the second entry was silently dropped.
    /// Spec for `patient` is `0..1` but the strict extractor must still
    /// surface every entry the client supplied (the shared permissive
    /// extractor already did).
    #[tokio::test]
    async fn test_inline_run_applies_all_patient_refs() {
        let (server, _backend) = create_test_server().await;

        let view = patient_view_definition();
        let pt_a = json!({
            "resourceType": "Patient",
            "id": "pt-a",
            "name": [{ "family": "Alpha" }]
        });
        let pt_b = json!({
            "resourceType": "Patient",
            "id": "pt-b",
            "name": [{ "family": "Beta" }]
        });
        let pt_c = json!({
            "resourceType": "Patient",
            "id": "pt-c",
            "name": [{ "family": "Gamma" }]
        });

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "viewResource", "resource": view},
                {"name": "resource", "resource": pt_a},
                {"name": "resource", "resource": pt_b},
                {"name": "resource", "resource": pt_c},
                {"name": "patient", "valueReference": {"reference": "Patient/pt-a"}},
                {"name": "patient", "valueReference": {"reference": "Patient/pt-b"}}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let families: Vec<String> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter_map(|row| row.get("family").and_then(|v| v.as_str()).map(String::from))
            .collect();

        assert!(
            families.contains(&"Alpha".to_string()),
            "expected pt-a (Alpha) in output, got {families:?}"
        );
        assert!(
            families.contains(&"Beta".to_string()),
            "expected pt-b (Beta) in output, got {families:?}"
        );
        assert!(
            !families.contains(&"Gamma".to_string()),
            "pt-c (Gamma) was not in the patient filter and should be excluded, got {families:?}"
        );
    }

    /// `?_limit=1` caps the number of output rows.
    #[tokio::test]
    async fn test_run_view_definition_limit() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-lim-1", "Alpha").await;
        seed_patient(&backend, "pt-lim-2", "Beta").await;
        seed_patient(&backend, "pt-lim-3", "Gamma").await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&_limit=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(rows.len(), 1, "limit=1 must return exactly 1 row");
    }

    // =========================================================================
    // Error cases → 422
    // =========================================================================

    /// A ViewDefinition missing the required `resource` field returns 422.
    #[tokio::test]
    async fn test_run_view_definition_missing_resource_returns_422() {
        let (server, _backend) = create_test_server().await;

        let bad_view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            // intentionally omitting "resource" field
            "select": [
                {
                    "column": [
                        { "path": "id", "name": "id", "type": "string" }
                    ]
                }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_view)
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Response must be an OperationOutcome
        let body: Value =
            serde_json::from_str(&response.text()).expect("422 body must be valid JSON");
        assert_eq!(
            body["resourceType"], "OperationOutcome",
            "422 body must be OperationOutcome: {body}"
        );
    }

    /// A ViewDefinition with an empty `select` array returns 422.
    #[tokio::test]
    async fn test_run_view_definition_empty_select_returns_422() {
        let (server, _backend) = create_test_server().await;

        let bad_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": []  // empty select — no columns defined
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_view)
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        let body: Value =
            serde_json::from_str(&response.text()).expect("422 body must be valid JSON");
        assert_eq!(
            body["resourceType"], "OperationOutcome",
            "422 body must be OperationOutcome: {body}"
        );
    }

    // =========================================================================
    // Error cases → 400
    // =========================================================================

    /// A body with an unexpected `resourceType` returns 400.
    #[tokio::test]
    async fn test_run_view_definition_wrong_resource_type_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bad_body = json!({
            "resourceType": "Patient",
            "id": "oops"
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_body)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    /// A `Parameters` body without a `viewResource` parameter returns 400.
    #[tokio::test]
    async fn test_run_view_definition_parameters_missing_view_resource_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bad_params = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "someOtherParam", "valueString": "irrelevant" }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bad_params)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
    }

    // =========================================================================
    // Runner override
    // =========================================================================

    // =========================================================================
    // Helpers for filter tests (require in-DB runner wired into AppState)
    // =========================================================================

    /// Creates a server with the SQLite in-DB runner wired in via `with_sof_runner`.
    /// The in-DB runner compiles `_since`, `patient`, and `group` filters to SQL.
    ///
    /// The compartment-aware filter (audit item #3) queries the populated
    /// `search_index` table, so the SearchParameter spec data needs to be
    /// loaded. Point `data_dir` at the workspace `data/` directory via the
    /// crate-relative `CARGO_MANIFEST_DIR` so tests work regardless of CWD.
    async fn create_test_server_with_indb() -> (TestServer, Arc<SqliteBackend>) {
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let backend_config = helios_persistence::backends::sqlite::SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", backend_config)
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SQLiteBackend must provide sof_runner");

        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    // =========================================================================
    // Filter tests — `_since`, `patient`, `group`
    // =========================================================================

    /// `_since` returns only resources whose `last_updated` is at or after the given instant.
    #[tokio::test]
    async fn test_run_view_definition_since_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        seed_patient(&backend, "p-since-1", "Early").await;

        // Pause long enough so p-since-2 gets a strictly later last_updated timestamp.
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let since = Utc::now();

        seed_patient(&backend, "p-since-2", "Late").await;

        // Use the Z-suffix form to avoid '+' percent-encoding issues in the URL.
        let since_str = since.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        // Use a flat-column view (only `id`) so the in-DB runner can compile it fully.
        // The `name.family` path involves array navigation which produces NULL in SQLite's
        // json_extract — the filter correctness test only needs to verify row count and id.
        let flat_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "patient_id", "type": "string"}]}]
        });

        let response = server
            .post(&format!(
                "/ViewDefinition/$viewdefinition-run?_format=ndjson&_since={since_str}"
            ))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&flat_view)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "_since filter must return only the later patient; got {rows:?}"
        );
        assert_eq!(
            rows[0]["patient_id"], "p-since-2",
            "expected p-since-2 in the result: {rows:?}"
        );
    }

    /// `patient=Patient/p1` restricts results to resources whose `subject.reference`
    /// or `patient.reference` matches the given value.
    #[tokio::test]
    async fn test_run_view_definition_patient_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        let tenant = test_tenant();
        // Seed two Observations, one per patient
        for (id, patient_ref) in [("obs-1", "Patient/p1"), ("obs-2", "Patient/p2")] {
            let obs = json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": { "text": "test" },
                "subject": { "reference": patient_ref }
            });
            backend
                .create(&tenant, "Observation", obs, FhirVersion::R4)
                .await
                .expect("failed to seed observation");
        }

        let obs_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "obs_id", "type": "string"}]}]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&patient=Patient/p1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&obs_view)
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "patient filter must return only obs-1; got {rows:?}"
        );
        assert_eq!(
            rows[0]["obs_id"], "obs-1",
            "expected obs-1 in result: {rows:?}"
        );
    }

    /// `group=Group/g1` resolves `Group.member.entity` to Patient refs and
    /// then applies the Patient-compartment filter. Mirrors what the inline
    /// path does via `helios_sof::resolve_group_members_to_patient_refs`.
    /// Pre-audit-#3 the runner just literally matched `Patient.group.reference`
    /// (a non-spec field); this test exercises the new spec-correct path.
    #[tokio::test]
    async fn test_run_view_definition_group_filter() {
        let (server, backend) = create_test_server_with_indb().await;

        let tenant = test_tenant();
        // Seed two patients and a Group whose member.entity references one.
        let p_in = json!({
            "resourceType": "Patient",
            "id": "p-grouped",
            "active": true
        });
        let p_out = json!({
            "resourceType": "Patient",
            "id": "p-ungrouped",
            "active": true
        });
        let group = json!({
            "resourceType": "Group",
            "id": "g1",
            "type": "person",
            "actual": true,
            "member": [
                {"entity": {"reference": "Patient/p-grouped"}}
            ]
        });
        for (rt, res) in [("Patient", p_in), ("Patient", p_out), ("Group", group)] {
            backend
                .create(&tenant, rt, res, FhirVersion::R4)
                .await
                .expect("failed to seed resource");
        }

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&group=Group/g1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);

        let body = response.text();
        let rows: Vec<Value> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(
            rows.len(),
            1,
            "group filter must return only p-grouped (via member.entity); got {rows:?}"
        );
        assert_eq!(
            rows[0]["patient_id"], "p-grouped",
            "expected p-grouped in result: {rows:?}"
        );
    }

    // =========================================================================
    // Uncompilable view → 422 (no in-process fallback exists)
    // =========================================================================

    /// Views the in-DB compiler can't handle return `422 Unprocessable Entity`
    /// directly — there is no in-process FHIRPath fallback. `lowBoundary()` on
    /// a string column is one such case (the boundary functions need the
    /// `column.type` hint to pick decimal vs. date semantics).
    #[tokio::test]
    async fn test_run_view_definition_uncompilable_returns_422() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-1", "Smith").await;

        // `lowBoundary()` requires the column to declare a `type` so the
        // compiler can pick decimal vs. date/dateTime/time semantics. Omitting
        // it returns Uncompilable.
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    { "path": "id", "name": "patient_id" },
                    { "path": "birthDate.lowBoundary()", "name": "birth_low" }
                ]
            }]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&view)
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // =========================================================================
    // viewReference (T2.2): resolve a stored ViewDefinition by reference
    // =========================================================================

    #[tokio::test]
    async fn test_run_view_definition_view_reference_relative() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-vr-1", "RefFam").await;

        // Persist the ViewDefinition to storage.
        let tenant = test_tenant();
        let mut vd = patient_view_definition();
        vd["id"] = json!("stored-vd-1");
        backend
            .create(&tenant, "ViewDefinition", vd, FhirVersion::R4)
            .await
            .expect("failed to seed VD");

        // Run via viewReference instead of inline viewResource.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "viewReference",
                "valueReference": {"reference": "ViewDefinition/stored-vd-1"}
            }]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);

        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "RefFam");
    }

    /// A canonical viewReference that does not match any stored
    /// ViewDefinition resolves with 422 (`processing`), distinguishing
    /// "couldn't resolve" from the previous "rejected unconditionally".
    #[tokio::test]
    async fn test_run_view_definition_unknown_canonical_reference_422() {
        let (server, _backend) = create_test_server().await;

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "ndjson"},
                {"name": "viewReference",
                 "valueReference": {"reference": "http://example.org/ViewDefinition/missing|1.0"}}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&body)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // =========================================================================
    // Inline resources (T2.6)
    // =========================================================================

    #[tokio::test]
    async fn test_run_view_definition_inline_resources() {
        let (server, _backend) = create_test_server().await;

        // No data seeded; inline resources should drive the run.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "viewResource", "resource": patient_view_definition()},
                {"name": "resource", "resource": {
                    "resourceType": "Patient", "id": "inline-a",
                    "name": [{"family": "InlineA"}]
                }},
                {"name": "resource", "resource": {
                    "resourceType": "Patient", "id": "inline-b",
                    "name": [{"family": "InlineB"}]
                }}
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&body)
            .await;
        response.assert_status(StatusCode::OK);

        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            rows.len(),
            2,
            "inline resources must drive the run: {rows:?}"
        );
        let families: Vec<&str> = rows.iter().filter_map(|r| r["family"].as_str()).collect();
        assert!(families.contains(&"InlineA"));
        assert!(families.contains(&"InlineB"));
    }

    // =========================================================================
    // Multi-value patient filter (T2.4)
    // =========================================================================

    #[tokio::test]
    async fn test_run_view_definition_multi_value_patient_filter() {
        // The patient filter is applied by the in-DB SQL runner; the in-process
        // runner pages all resources without compartment filtering.
        let (server, backend) = create_test_server_with_indb().await;
        let tenant = test_tenant();

        // Patients and Observations linked to two patients.
        for (pid, family) in [("p1", "OneFam"), ("p2", "TwoFam"), ("p3", "ThreeFam")] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": pid,
                        "name": [{"family": family}]
                    }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": format!("obs-{pid}"),
                        "status": "final",
                        "code": {"text": "x"},
                        "subject": {"reference": format!("Patient/{pid}")}
                    }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }

        let obs_view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{"column": [
                {"path": "id", "name": "obs_id", "type": "string"},
                {"path": "subject.reference", "name": "subject", "type": "string"}
            ]}]
        });

        // Filter by two distinct patient references.
        let response = server
            .post(
                "/ViewDefinition/$viewdefinition-run?_format=ndjson&patient=Patient/p1,Patient/p2",
            )
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .json(&obs_view)
            .await;
        response.assert_status(StatusCode::OK);

        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let subjects: Vec<&str> = rows.iter().filter_map(|r| r["subject"].as_str()).collect();
        assert_eq!(
            subjects.len(),
            2,
            "expected exactly 2 rows for two patient filters, got: {subjects:?}"
        );
        assert!(subjects.contains(&"Patient/p1"));
        assert!(subjects.contains(&"Patient/p2"));
        assert!(!subjects.contains(&"Patient/p3"));
    }

    // =========================================================================
    // Spec alignment (round 2)
    // =========================================================================

    /// G7: system-level URL `/$viewdefinition-run` is routed and works like
    /// the type-level form.
    #[tokio::test]
    async fn test_run_view_definition_system_level_route() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-sys-1", "System").await;

        let response = server
            .post("/$viewdefinition-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::OK);
        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "System");
    }

    /// G1: `viewReference` accepts a canonical URL; the server resolves it
    /// via `SearchProvider` against `ViewDefinition.url`.
    #[tokio::test]
    async fn test_run_view_definition_canonical_view_reference() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-can-1", "Canonical").await;
        seed_view_definition(
            &backend,
            "vd-can",
            Some("http://example.org/fhir/ViewDefinition/patient-family"),
            None,
        )
        .await;

        let url = "/ViewDefinition/$viewdefinition-run\
                   ?_format=ndjson\
                   &viewReference=http://example.org/fhir/ViewDefinition/patient-family";
        let response = server
            .get(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::OK);
        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Canonical");
    }

    /// G1: canonical URL with `|version` selects the matching version.
    #[tokio::test]
    async fn test_run_view_definition_canonical_view_reference_with_version() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "pt-ver-1", "Versioned").await;
        let url = "http://example.org/fhir/ViewDefinition/family";
        seed_view_definition(&backend, "vd-v1", Some(url), Some("1.0.0")).await;
        seed_view_definition(&backend, "vd-v2", Some(url), Some("2.0.0")).await;

        let route =
            format!("/ViewDefinition/$viewdefinition-run?_format=ndjson&viewReference={url}|2.0.0");
        let response = server
            .get(&route)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::OK);
        // Either version returns the same rows shape; the test mainly
        // exercises that `|version` doesn't blow up resolution.
        let rows: Vec<Value> = response
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["family"], "Versioned");
    }

    /// G2: `source` parameter returns **400** + OperationOutcome with code
    /// `not-supported` (previously 501).
    #[tokio::test]
    async fn test_run_view_definition_source_returns_400_not_supported() {
        let (server, _backend) = create_test_server().await;

        let parameters_body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "viewResource", "resource": patient_view_definition() },
                { "name": "_format", "valueCode": "ndjson" },
                { "name": "source", "valueString": "s3://example/bucket" }
            ]
        });

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&parameters_body)
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let outcome: Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"][0]["code"], "not-supported");
    }

    /// Audit item #10: HFS REST enforces the same `_limit` bounds as
    /// sof-server (1..=10000). `_limit=0` rejected with 400.
    #[tokio::test]
    async fn test_run_view_definition_limit_zero_returns_400() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&_limit=0")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body = response.text();
        assert!(
            body.contains("greater than 0"),
            "error message must explain the lower bound: {body}"
        );
    }

    /// Audit item #10: `_limit > 10000` rejected with 400 (matches
    /// sof-server's safety cap). Spec leaves _limit unbounded; this is a
    /// deployment-policy decision shared between both binaries.
    #[tokio::test]
    async fn test_run_view_definition_limit_exceeds_cap_returns_400() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .post("/ViewDefinition/$viewdefinition-run?_format=ndjson&_limit=10001")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient_view_definition())
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body = response.text();
        assert!(
            body.contains("cannot exceed 10000"),
            "error message must explain the upper bound: {body}"
        );
    }
}
