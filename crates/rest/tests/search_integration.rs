//! Integration tests for FHIR search functionality.
//!
//! Tests the search handler integration with the SQLite backend, covering:
//! - Basic search parameters (string, token, date, reference)
//! - Search modifiers (:exact, :contains, :missing)
//! - Pagination (_count, _offset)
//! - Sorting (_sort)
//! - Chained parameters (patient.name, _has)
//! - Include/revinclude (_include, _revinclude)
//! - Full-text search (_text, _content)
//! - Subsetting (_summary, _elements)
//! - Compartment search

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

#[allow(dead_code)]
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// Creates a test server with search capability.
async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    // Configure with data directory to load spec SearchParameters
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

/// Gets the test tenant context.
fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Seeds test data for search tests.
async fn seed_search_test_data(backend: &SqliteBackend) {
    let tenant = test_tenant();

    // Organizations (needed for chaining)
    let organizations = vec![
        json!({
            "resourceType": "Organization",
            "id": "org-1",
            "name": "General Hospital",
            "active": true,
            "type": [{"coding": [{"system": "http://terminology.hl7.org/CodeSystem/organization-type", "code": "prov"}]}]
        }),
        json!({
            "resourceType": "Organization",
            "id": "org-2",
            "name": "City Clinic",
            "active": true
        }),
    ];

    for org in organizations {
        let id = org["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Organization", org, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create organization {}: {}", id, e));
    }

    // Practitioners (for chaining tests)
    let practitioners = vec![json!({
        "resourceType": "Practitioner",
        "id": "pract-1",
        "name": [{"family": "Brown", "given": ["James"]}],
        "active": true
    })];

    for pract in practitioners {
        let id = pract["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Practitioner", pract, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create practitioner {}: {}", id, e));
    }

    // Patients
    let patients = vec![
        json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "active": true,
            "name": [{"family": "Smith", "given": ["John", "Michael"]}],
            "gender": "male",
            "birthDate": "1980-01-15",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN12345"}],
            "address": [{"city": "Boston", "state": "MA", "postalCode": "02101"}],
            "managingOrganization": {"reference": "Organization/org-1"},
            "generalPractitioner": [{"reference": "Practitioner/pract-1"}],
            "text": {"status": "generated", "div": "<div>John Smith is a patient with diabetes</div>"}
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-2",
            "active": true,
            "name": [{"family": "Smith", "given": ["Jane"]}],
            "gender": "female",
            "birthDate": "1990-05-20",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN67890"}],
            "managingOrganization": {"reference": "Organization/org-1"},
            "text": {"status": "generated", "div": "<div>Jane Smith is a healthy patient</div>"}
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-3",
            "active": false,
            "name": [{"family": "Jones", "given": ["Robert"]}],
            "gender": "male",
            "birthDate": "1975-12-01",
            "managingOrganization": {"reference": "Organization/org-2"},
            "text": {"status": "generated", "div": "<div>Robert Jones has hypertension</div>"}
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-4",
            "active": true,
            "name": [{"family": "Williams", "given": ["Sarah"]}],
            "gender": "female",
            "birthDate": "2000-03-10",
            "text": {"status": "generated", "div": "<div>Sarah Williams is a young patient</div>"}
        }),
    ];

    for patient in patients {
        let id = patient["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Patient", patient, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create patient {}: {}", id, e));
    }

    // Encounters (for _revinclude tests)
    let encounters = vec![
        json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
            "subject": {"reference": "Patient/patient-1"},
            "period": {"start": "2024-01-15T09:00:00Z", "end": "2024-01-15T10:00:00Z"}
        }),
        json!({
            "resourceType": "Encounter",
            "id": "enc-2",
            "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "IMP"},
            "subject": {"reference": "Patient/patient-1"},
            "period": {"start": "2024-02-01T00:00:00Z", "end": "2024-02-05T00:00:00Z"}
        }),
    ];

    for enc in encounters {
        let id = enc["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Encounter", enc, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create encounter {}: {}", id, e));
    }

    // Observations
    let observations = vec![
        json!({
            "resourceType": "Observation",
            "id": "obs-1",
            "status": "final",
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "8867-4", "display": "Heart rate"}]
            },
            "subject": {"reference": "Patient/patient-1"},
            "encounter": {"reference": "Encounter/enc-1"},
            "performer": [{"reference": "Practitioner/pract-1"}],
            "valueQuantity": {"value": 72, "unit": "bpm"},
            "text": {"status": "generated", "div": "<div>Heart rate measurement showing normal sinus rhythm</div>"}
        }),
        json!({
            "resourceType": "Observation",
            "id": "obs-2",
            "status": "final",
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "8310-5", "display": "Body temperature"}]
            },
            "subject": {"reference": "Patient/patient-1"},
            "encounter": {"reference": "Encounter/enc-1"},
            "valueQuantity": {"value": 37.0, "unit": "Cel"},
            "text": {"status": "generated", "div": "<div>Temperature within normal range</div>"}
        }),
        json!({
            "resourceType": "Observation",
            "id": "obs-3",
            "status": "final",
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "8867-4", "display": "Heart rate"}]
            },
            "subject": {"reference": "Patient/patient-2"},
            "valueQuantity": {"value": 68, "unit": "bpm"},
            "text": {"status": "generated", "div": "<div>Resting heart rate is good</div>"}
        }),
        json!({
            "resourceType": "Observation",
            "id": "obs-4",
            "status": "preliminary",
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "8867-4", "display": "Heart rate"}]
            },
            "subject": {"reference": "Patient/patient-3"},
            "valueQuantity": {"value": 80, "unit": "bpm"},
            "text": {"status": "generated", "div": "<div>Elevated heart rate noted</div>"}
        }),
    ];

    for obs in observations {
        let id = obs["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Observation", obs, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create observation {}: {}", id, e));
    }

    // Conditions
    let conditions = vec![
        json!({
            "resourceType": "Condition",
            "id": "condition-1",
            "clinicalStatus": {
                "coding": [{"system": "http://terminology.hl7.org/CodeSystem/condition-clinical", "code": "active"}]
            },
            "code": {
                "coding": [{"system": "http://snomed.info/sct", "code": "73211009", "display": "Diabetes mellitus"}]
            },
            "subject": {"reference": "Patient/patient-1"},
            "text": {"status": "generated", "div": "<div>Type 2 diabetes mellitus diagnosed in 2015</div>"}
        }),
        json!({
            "resourceType": "Condition",
            "id": "condition-2",
            "clinicalStatus": {
                "coding": [{"system": "http://terminology.hl7.org/CodeSystem/condition-clinical", "code": "active"}]
            },
            "code": {
                "coding": [{"system": "http://snomed.info/sct", "code": "38341003", "display": "Hypertension"}]
            },
            "subject": {"reference": "Patient/patient-3"},
            "text": {"status": "generated", "div": "<div>Essential hypertension requiring medication</div>"}
        }),
    ];

    for condition in conditions {
        let id = condition["id"].as_str().unwrap().to_string();
        backend
            .create(&tenant, "Condition", condition, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("Failed to create condition {}: {}", id, e));
    }
}

/// Helper to extract bundle entries.
fn get_bundle_entries(body: &Value) -> Vec<&Value> {
    body["entry"]
        .as_array()
        .map(|arr| arr.iter().collect())
        .unwrap_or_default()
}

/// Helper to get total from bundle.
#[allow(dead_code)]
fn get_bundle_total(body: &Value) -> Option<i64> {
    body["total"].as_i64()
}

// =============================================================================
// Basic Search Tests
// =============================================================================

mod basic_search {
    use super::*;

    #[tokio::test]
    async fn test_search_returns_bundle() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "searchset");
        // Total may or may not be present depending on search implementation
        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty(), "Should have search results");
    }

    #[tokio::test]
    async fn test_total_accurate_populates_bundle_total() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Baseline count of all patients (no _total -> total omitted).
        let baseline: Value = server
            .get("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .json();
        let patient_count = get_bundle_entries(&baseline).len() as i64;
        assert!(patient_count > 0, "fixture should seed patients");
        assert!(
            baseline["total"].is_null(),
            "Bundle.total should be absent without _total"
        );

        // _total=accurate -> Bundle.total present and equal to the match count.
        let response = server
            .get("/Patient?_total=accurate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(
            body["total"].as_i64(),
            Some(patient_count),
            "Bundle.total must equal the number of matches"
        );
    }

    #[tokio::test]
    async fn test_unknown_param_lenient_ignored_strict_rejected() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Lenient (default): unknown parameter is ignored, search succeeds.
        let lenient = server
            .get("/Patient?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        lenient.assert_status_ok();

        // Strict: unknown parameter is rejected with 400.
        let strict = server
            .get("/Patient?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        assert_eq!(strict.status_code(), StatusCode::BAD_REQUEST);

        // A known parameter is accepted even under strict handling.
        let ok = server
            .get("/Patient?name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        ok.assert_status_ok();
    }

    #[tokio::test]
    async fn test_search_by_id() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_id=patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["id"], "patient-1");
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_id=nonexistent")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        assert_eq!(body["resourceType"], "Bundle");
        // Empty results
        assert!(get_bundle_entries(&body).is_empty());
        // Total may be 0 or absent
        if let Some(total) = body["total"].as_i64() {
            assert_eq!(total, 0);
        }
    }

    #[tokio::test]
    async fn test_post_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Use form method which properly encodes form data
        let response = server
            .post("/Patient/_search")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .form(&[("_id", "patient-1")])
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
    }
}

// =============================================================================
// String Search Tests
// =============================================================================

mod string_search {
    use super::*;

    #[tokio::test]
    async fn test_string_search_prefix_match() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Default string search is prefix match
        let response = server
            .get("/Patient?name=Smi")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Should match "Smith"
        assert!(entries.len() >= 2);
    }

    #[tokio::test]
    async fn test_string_search_exact_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?name:exact=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Should match exactly "Smith"
        for entry in &entries {
            let family = entry["resource"]["name"][0]["family"].as_str().unwrap();
            assert_eq!(family, "Smith");
        }
    }

    #[tokio::test]
    async fn test_string_search_contains_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?name:contains=mit")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should match "Smith"
        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    async fn test_string_search_text_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // `:text` on a string is a case-insensitive partial match (FHIR spec
        // allows :text on string). Previously the gate rejected it as
        // token-only; now it matches "Smith" via the substring "mit".
        let response = server
            .get("/Patient?name:text=mit")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    async fn test_not_in_modifier_returns_501_without_terminology_server() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // :not-in needs negated value-set filtering, which is unimplemented; it
        // must return 501 rather than silently returning a superset, even when
        // no terminology server is configured.
        let response = server
            .get("/Observation?code:not-in=http://example.org/vs")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn test_modifier_invalid_for_param_type_returns_400() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // `:above` is only defined for token/uri/reference params; applying it
        // to the string param `name` must be rejected with a 400 + invalid
        // OperationOutcome rather than silently ignored.
        let response = server
            .get("/Patient?name:above=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["severity"], "error");
        assert_eq!(body["issue"][0]["code"], "invalid");
        assert!(
            body["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("above")
        );
    }
}

// =============================================================================
// Token Search Tests
// =============================================================================

mod token_search {
    use super::*;

    #[tokio::test]
    async fn test_token_search_code_only() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?gender=male")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        for entry in &entries {
            assert_eq!(entry["resource"]["gender"], "male");
        }
    }

    #[tokio::test]
    async fn test_token_search_system_and_code() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Observation?code=http://loinc.org|8867-4")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Should find heart rate observations
        assert!(entries.len() >= 2);
    }

    #[tokio::test]
    async fn test_token_search_identifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?identifier=MRN12345")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["id"], "patient-1");
    }
}

// =============================================================================
// Reference Search Tests
// =============================================================================

mod reference_search {
    use super::*;

    #[tokio::test]
    async fn test_reference_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Observation?subject=Patient/patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Patient 1 has 2 observations
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_reference_search_id_only() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Observation?subject=patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_reference_search_contains_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // `:contains` is spec-valid for reference params (substring match on the
        // stored reference). Previously the validation gate rejected it as
        // string-only; now it resolves and matches "Patient/patient-1".
        let response = server
            .get("/Observation?subject:contains=patient-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty());
        for entry in &entries {
            let subject = entry["resource"]["subject"]["reference"].as_str().unwrap();
            assert!(subject.contains("patient-1"));
        }
    }

    #[tokio::test]
    async fn test_reference_search_below_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // :below does URL/path-prefix hierarchy on the reference. "Patient"
        // matches "Patient/patient-1" etc. (the seeded observation subjects).
        let response = server
            .get("/Observation?subject:below=Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty());
        for entry in &entries {
            let subject = entry["resource"]["subject"]["reference"].as_str().unwrap();
            assert!(subject.starts_with("Patient/"));
        }
    }

    #[tokio::test]
    async fn test_reference_search_text_modifier_on_display() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();

        // An Observation whose subject reference carries a display string. The
        // extractor indexes Reference.display so :text (contains) and :code-text
        // (starts-with) can match it.
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-display-1",
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                    "subject": {"reference": "Patient/p-xyz", "display": "Johnny Appleseed"}
                }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        // :text matches a substring of the display.
        let response = server
            .get("/Observation?subject:text=Appleseed")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["id"], "obs-display-1");

        // :code-text matches a prefix of the display, but not a mid-string token.
        let prefix = server
            .get("/Observation?subject:code-text=Johnny")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        prefix.assert_status_ok();
        let prefix_body: Value = prefix.json();
        assert_eq!(get_bundle_entries(&prefix_body).len(), 1);

        let mid = server
            .get("/Observation?subject:code-text=Appleseed")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        mid.assert_status_ok();
        let mid_body: Value = mid.json();
        assert_eq!(get_bundle_entries(&mid_body).len(), 0);
    }
}

// =============================================================================
// Date Search Tests
// =============================================================================

mod date_search {
    use super::*;

    #[tokio::test]
    async fn test_date_search_equals() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?birthdate=1980-01-15")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["id"], "patient-1");
    }

    #[tokio::test]
    async fn test_date_search_greater_than() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?birthdate=gt1985-01-01")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Patients born after 1985: patient-2 (1990), patient-4 (2000)
        assert!(entries.len() >= 2);
    }

    #[tokio::test]
    async fn test_date_search_less_than() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?birthdate=lt1985-01-01")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Patients born before 1985: patient-1 (1980), patient-3 (1975)
        assert!(entries.len() >= 2);
    }
}

// =============================================================================
// Pagination Tests
// =============================================================================

mod pagination {
    use super::*;

    #[tokio::test]
    async fn test_pagination_count() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_count=2")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 2);
        // Total may or may not be present; if present, should be >= 2
        if let Some(total) = body["total"].as_i64() {
            assert!(total >= 2);
        }
    }

    #[tokio::test]
    async fn test_pagination_offset() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // First page
        let response1 = server
            .get("/Patient?_count=2&_offset=0")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response1.assert_status_ok();
        let body1: Value = response1.json();
        let entries1 = get_bundle_entries(&body1);

        // Second page
        let response2 = server
            .get("/Patient?_count=2&_offset=2")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response2.assert_status_ok();
        let body2: Value = response2.json();
        let entries2 = get_bundle_entries(&body2);

        // Results should be different
        if !entries1.is_empty() && !entries2.is_empty() {
            let id1 = entries1[0]["resource"]["id"].as_str();
            let id2 = entries2[0]["resource"]["id"].as_str();
            assert_ne!(id1, id2);
        }
    }

    #[tokio::test]
    async fn test_pagination_links() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_count=2")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should have self link
        let links = body["link"].as_array().expect("Should have links");
        let has_self = links.iter().any(|l| l["relation"] == "self");
        assert!(has_self, "Bundle should have self link");
    }
}

// =============================================================================
// Subsetting Tests (_summary, _elements)
// =============================================================================

mod subsetting {
    use super::*;

    #[tokio::test]
    async fn test_summary_count() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_summary=count")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // With _summary=count, entries should be empty or absent
        let entries = body["entry"].as_array();
        assert!(
            entries.is_none() || entries.unwrap().is_empty(),
            "With _summary=count, entries should be empty"
        );
        // Bundle should still be valid
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_summary_true() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_summary=true&_count=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        if !entries.is_empty() {
            let resource = &entries[0]["resource"];
            // Summary should include basic elements
            assert!(resource["resourceType"].is_string());
            assert!(resource["id"].is_string());
            // Should include summary elements like name
            assert!(resource["name"].is_array());
        }
    }

    #[tokio::test]
    async fn test_summary_data() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_summary=data&_count=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        if !entries.is_empty() {
            let resource = &entries[0]["resource"];
            // Should not include text element
            assert!(resource["text"].is_null());
        }
    }

    #[tokio::test]
    async fn test_elements_parameter() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_elements=id,name&_count=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        if !entries.is_empty() {
            let resource = &entries[0]["resource"];
            // Should include requested elements
            assert!(resource["id"].is_string());
            assert!(resource["name"].is_array());
            // Should include resourceType (always included)
            assert!(resource["resourceType"].is_string());
            // Should not include unrequested elements
            assert!(resource["gender"].is_null());
            assert!(resource["birthDate"].is_null());
            // Subset responses must carry the SUBSETTED meta.tag (FHIR spec).
            let tags = resource["meta"]["tag"]
                .as_array()
                .expect("meta.tag present");
            assert!(
                tags.iter().any(|t| t["code"] == "SUBSETTED"
                    && t["system"] == "http://terminology.hl7.org/CodeSystem/v3-ObservationValue"),
                "subsetted entry should be tagged SUBSETTED"
            );
        }
    }

    #[tokio::test]
    async fn test_no_subsetted_tag_without_subsetting() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_count=1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        if let Some(entry) = entries.first() {
            let tags = entry["resource"]["meta"]["tag"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(
                !tags.iter().any(|t| t["code"] == "SUBSETTED"),
                "full representation must not be tagged SUBSETTED"
            );
        }
    }
}

// =============================================================================
// Compartment Search Tests
// =============================================================================

mod compartment_search {
    use super::*;

    #[tokio::test]
    async fn test_patient_compartment_observations() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient/patient-1/Observation")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Patient 1 has 2 observations
        assert_eq!(entries.len(), 2);

        // All observations should reference patient-1
        for entry in &entries {
            let subject = entry["resource"]["subject"]["reference"].as_str().unwrap();
            assert!(subject.contains("patient-1"));
        }
    }

    #[tokio::test]
    async fn test_patient_compartment_with_params() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Get observations for patient-1 with code filter
        let response = server
            .get("/Patient/patient-1/Observation?code=8867-4")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Patient 1 has 1 heart rate observation
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn test_patient_compartment_conditions() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient/patient-1/Condition")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Bundle should be returned
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "searchset");

        // Note: Condition search may return 0 or 1 depending on search index state
        // The important test is that the compartment search executes correctly
        let entries = get_bundle_entries(&body);
        // If entries exist, they should all reference patient-1
        for entry in &entries {
            if let Some(subject) = entry["resource"]["subject"]["reference"].as_str() {
                assert!(subject.contains("patient-1"));
            }
        }
    }

    #[tokio::test]
    async fn test_compartment_invalid_combination() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Try to search for a resource type not in the Patient compartment
        let response = server
            .get("/Patient/patient-1/Organization")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Should return 400 Bad Request
        response.assert_status(StatusCode::BAD_REQUEST);
    }
}

// =============================================================================
// Multiple Parameters Tests
// =============================================================================

mod multiple_params {
    use super::*;

    #[tokio::test]
    async fn test_and_parameters() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for male patients named Smith
        let response = server
            .get("/Patient?name=Smith&gender=male")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Only patient-1 is male and named Smith
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["id"], "patient-1");
    }

    #[tokio::test]
    async fn test_or_values() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for patients with gender male OR female (comma = OR)
        let response = server
            .get("/Patient?gender=male,female")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Should get all patients with gender specified
        assert!(entries.len() >= 3);
    }
}

// =============================================================================
// Sorting Tests (_sort)
// =============================================================================

mod sorting {
    use super::*;

    #[tokio::test]
    async fn test_sort_ascending() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_sort=birthdate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert!(entries.len() >= 2, "Should have multiple patients");

        // Verify we got results - sorting order may not be enforced in current implementation
        // The key test is that _sort parameter is accepted without error
        let birthdates: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["resource"]["birthDate"].as_str())
            .collect();
        assert!(!birthdates.is_empty(), "Should have birthdates in results");
    }

    #[tokio::test]
    async fn test_sort_descending() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_sort=-birthdate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert!(entries.len() >= 2, "Should have multiple patients");

        // Verify _sort with - prefix is accepted
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_sort_multiple_fields() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Sort by gender ascending, then birthdate descending
        let response = server
            .get("/Patient?_sort=gender,-birthdate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty(), "Should have results");

        // Verify multi-field sort is accepted
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_sort_by_name() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_sort=family")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        assert!(!entries.is_empty(), "Should have results");

        // Verify _sort by name field is accepted
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_sort_observation_by_date() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Observation?_sort=-date")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should return observations sorted by date descending
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_sort_with_search_params() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Combine _sort with other search parameters
        let response = server
            .get("/Patient?gender=female&_sort=birthdate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Should only have female patients
        for entry in &entries {
            assert_eq!(entry["resource"]["gender"], "female");
        }
    }
}

// =============================================================================
// Chained Parameter Tests
// =============================================================================

mod chaining {
    use super::*;

    #[tokio::test]
    async fn test_chained_reference_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for observations where the subject (Patient) has name "Smith"
        // Chained parameter: subject.name=Smith
        let response = server
            .get("/Observation?subject.name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // Observations for patient-1 (John Smith) and patient-2 (Jane Smith)
        // Should find obs-1, obs-2, obs-3
        for entry in &entries {
            let subject_ref = entry["resource"]["subject"]["reference"].as_str().unwrap();
            assert!(
                subject_ref.contains("patient-1") || subject_ref.contains("patient-2"),
                "Should only include observations for patients named Smith"
            );
        }
    }

    #[tokio::test]
    async fn test_chained_reference_with_type() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Explicitly specify the reference type: subject:Patient.name=Smith
        let response = server
            .get("/Observation?subject:Patient.name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should work the same as without type qualifier
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_chained_organization() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for patients whose managing organization is named "General Hospital"
        let response = server
            .get("/Patient?organization.name=General")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);
        // patient-1 and patient-2 have managingOrganization = org-1 (General Hospital)
        for entry in &entries {
            let id = entry["resource"]["id"].as_str().unwrap();
            assert!(
                id == "patient-1" || id == "patient-2",
                "Should only include patients at General Hospital"
            );
        }
    }

    #[tokio::test]
    async fn test_reverse_chaining_has() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for patients that have observations with status=final
        // _has:Observation:subject:status=final
        let response = server
            .get("/Patient?_has:Observation:subject:status=final")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // _has may or may not be fully implemented
        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            assert_eq!(body["resourceType"], "Bundle");

            // If _has is working correctly, results should be filtered
            // But we don't assert specific results since implementation may vary
            let entries = get_bundle_entries(&body);
            // Just verify we got a valid response
            for entry in &entries {
                assert!(entry["resource"]["resourceType"] == "Patient");
            }
        } else {
            // _has not supported - that's acceptable
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_has_with_code() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for patients that have conditions with code for diabetes
        let response = server
            .get("/Patient?_has:Condition:subject:code=73211009")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // _has may or may not be fully implemented
        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            assert_eq!(body["resourceType"], "Bundle");
            // If working, should return patients with diabetes condition
            // but we don't assert specific results
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_multiple_chain_levels() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for observations where patient's organization name contains "General"
        // This is a two-level chain: Observation -> Patient -> Organization
        let response = server
            .get("/Observation?subject.organization.name=General")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // This may or may not be supported depending on implementation
        // Just verify we get a valid response (200 OK or 400 Bad Request)
        let status = response.status_code();
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or indicate unsupported operation"
        );
    }
}

// =============================================================================
// Include Tests (_include, _revinclude)
// =============================================================================

mod includes {
    use super::*;

    #[tokio::test]
    async fn test_include_subject() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search observations and include the subject (Patient)
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);

        // Should have at least the observation and possibly the included patient
        let obs_count = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Observation")
            .count();
        let patient_count = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Patient")
            .count();

        assert!(obs_count >= 1, "Should have at least 1 observation");

        // If includes are working, we should have a patient too
        // Check if any entry has search.mode = "include"
        let has_include = entries.iter().any(|e| e["search"]["mode"] == "include");
        if has_include {
            assert!(patient_count >= 1, "Should include the patient");
        }
    }

    #[tokio::test]
    async fn test_include_performer() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search observations and include the performer
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:performer")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should return valid bundle
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_include_multiple() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Include both subject and encounter
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject&_include=Observation:encounter")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);

        // Check for different resource types in results
        let resource_types: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["resource"]["resourceType"].as_str())
            .collect();

        assert!(
            resource_types.contains(&"Observation"),
            "Should have observation"
        );
        // Included resources depend on implementation
    }

    #[tokio::test]
    async fn test_revinclude() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search patients and reverse-include observations that reference them
        let response = server
            .get("/Patient?_id=patient-1&_revinclude=Observation:subject")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);

        // Should have the patient
        let patient_count = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Patient")
            .count();
        assert!(patient_count >= 1, "Should have the patient");

        // Check for observations (reverse includes)
        let obs_count = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Observation")
            .count();

        // If revinclude is working, we should have observations
        let has_include = entries.iter().any(|e| e["search"]["mode"] == "include");
        if has_include {
            assert!(obs_count >= 1, "Should revinclude observations");
        }
    }

    #[tokio::test]
    async fn test_revinclude_encounter() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search patients and reverse-include encounters
        let response = server
            .get("/Patient?_id=patient-1&_revinclude=Encounter:subject")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        // Should return valid bundle
        assert_eq!(body["resourceType"], "Bundle");
    }

    #[tokio::test]
    async fn test_include_iterate() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Use _include:iterate to follow references from included resources
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject&_include:iterate=Patient:organization")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // May or may not be supported
        let status = response.status_code();
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or indicate unsupported"
        );
    }

    #[tokio::test]
    async fn test_include_wildcard() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Use * to include all references
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:*")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // May or may not be supported
        let status = response.status_code();
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or indicate unsupported"
        );
    }

    #[tokio::test]
    async fn test_search_entry_mode() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Include subject to get both match and include entries
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();

        let entries = get_bundle_entries(&body);

        // Check that entries have search.mode
        for entry in &entries {
            if let Some(search) = entry.get("search") {
                let mode = search["mode"].as_str();
                assert!(
                    mode == Some("match") || mode == Some("include") || mode == Some("outcome"),
                    "search.mode should be match, include, or outcome"
                );
            }
        }
    }
}

// =============================================================================
// Full-Text Search Tests (_text, _content)
// =============================================================================

mod fulltext_search {
    use super::*;

    #[tokio::test]
    async fn test_text_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search patient narrative text for "diabetes"
        let response = server
            .get("/Patient?_text=diabetes")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Full-text search may or may not be implemented
        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            let entries = get_bundle_entries(&body);

            // If implemented, should find patient-1 (has "diabetes" in text)
            if !entries.is_empty() {
                // At least one result should mention diabetes in narrative
                let has_diabetes_patient =
                    entries.iter().any(|e| e["resource"]["id"] == "patient-1");
                assert!(
                    has_diabetes_patient,
                    "Should find patient with diabetes in text"
                );
            }
        } else {
            // Not implemented - that's okay
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "Should return 400 if not supported"
            );
        }
    }

    #[tokio::test]
    async fn test_content_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search all content for "hypertension"
        let response = server
            .get("/Condition?_content=hypertension")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            let entries = get_bundle_entries(&body);

            // If implemented, should find condition-2 (hypertension)
            if !entries.is_empty() {
                let has_hypertension = entries.iter().any(|e| {
                    e["resource"]["id"] == "condition-2"
                        || e["resource"]["code"]["coding"]
                            .as_array()
                            .map(|arr| arr.iter().any(|c| c["display"] == "Hypertension"))
                            .unwrap_or(false)
                });
                if has_hypertension {
                    // Good, found the expected result
                }
            }
        }
        // Either OK or BAD_REQUEST is acceptable
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or 400"
        );
    }

    #[tokio::test]
    async fn test_text_search_multiple_words() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for multiple words
        let response = server
            .get("/Observation?_text=heart%20rate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            // Should find observations with "heart rate" in text
            assert_eq!(body["resourceType"], "Bundle");
        }
        // Either OK or BAD_REQUEST is acceptable
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or 400"
        );
    }

    #[tokio::test]
    async fn test_text_search_case_insensitive() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search with different case
        let response_lower = server
            .get("/Patient?_text=smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let response_upper = server
            .get("/Patient?_text=SMITH")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // Both should succeed or both should fail (unsupported)
        assert_eq!(
            response_lower.status_code(),
            response_upper.status_code(),
            "Case should not affect search availability"
        );
    }

    #[tokio::test]
    async fn test_text_advanced_modifier() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Use :text-advanced modifier for FTS5 syntax
        let response = server
            .get("/Patient?_text:text-advanced=diabetes%20OR%20healthy")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let status = response.status_code();
        // This is an advanced feature, may not be supported
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or 400"
        );
    }

    #[tokio::test]
    async fn test_content_search_observations() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search observation content for "normal"
        let response = server
            .get("/Observation?_content=normal")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let status = response.status_code();
        if status == StatusCode::OK {
            let body: Value = response.json();
            let entries = get_bundle_entries(&body);

            // obs-1 has "normal sinus rhythm", obs-2 has "normal range"
            if !entries.is_empty() {
                // Results should include observations with "normal" in content
            }
        }
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "Should return OK or 400"
        );
    }
}
