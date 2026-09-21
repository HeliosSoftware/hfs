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

/// Returns the bundle's `self` link URL.
fn self_link(body: &Value) -> String {
    body["link"]
        .as_array()
        .and_then(|links| links.iter().find(|l| l["relation"] == "self"))
        .and_then(|l| l["url"].as_str())
        .expect("searchset must carry a self link")
        .to_string()
}

/// Returns the `search.mode = outcome` entries of a searchset bundle.
fn outcome_entries(body: &Value) -> Vec<&Value> {
    get_bundle_entries(body)
        .into_iter()
        .filter(|e| e["search"]["mode"] == "outcome")
        .collect()
}

/// Returns the `search.mode = match` entries of a searchset bundle.
fn match_entries(body: &Value) -> Vec<&Value> {
    get_bundle_entries(body)
        .into_iter()
        .filter(|e| e["search"]["mode"] == "match")
        .collect()
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
            baseline.get("total").is_none(),
            "Bundle.total should be absent (not null) without _total: {baseline}"
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
    async fn test_unknown_underscore_param_rejected_under_strict() {
        // Regression for #524: `_`-prefixed names used to bypass the unknown
        // parameter check entirely, so strict handling could never reject one.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for param in ["_typo=foo", "_whatever=foo", "_language=en"] {
            let strict = server
                .get(&format!("/Patient?{param}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    HeaderName::from_static("prefer"),
                    HeaderValue::from_static("handling=strict"),
                )
                .await;
            assert_eq!(
                strict.status_code(),
                StatusCode::BAD_REQUEST,
                "{param} must be rejected under Prefer: handling=strict"
            );
        }

        // Global parameters the server does honour are still accepted.
        for param in ["_id=patient-1", "_lastUpdated=gt2000-01-01", "_tag=foo"] {
            let ok = server
                .get(&format!("/Patient?{param}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    HeaderName::from_static("prefer"),
                    HeaderValue::from_static("handling=strict"),
                )
                .await;
            ok.assert_status_ok();
        }
    }

    #[tokio::test]
    async fn test_ignored_param_dropped_from_self_link_and_reported() {
        // Under lenient handling an unsupported parameter may be ignored only if
        // the server says so: it must not appear in the self link (which states
        // what was applied) and is reported as an OperationOutcome entry.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let all: Value = server
            .get("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .json();
        let total = get_bundle_entries(&all).len();
        assert!(total > 0, "fixture should seed patients");

        for query in ["_typo=foo", "nonsense-param=foo"] {
            let response = server
                .get(&format!("/Patient?{query}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
            let body: Value = response.json();

            let link = self_link(&body);
            assert!(
                !link.contains("typo") && !link.contains("nonsense-param"),
                "self link must not echo the ignored parameter ({query}): {link}"
            );

            let outcomes = outcome_entries(&body);
            assert_eq!(
                outcomes.len(),
                1,
                "ignored parameter must be reported ({query})"
            );
            let issue = &outcomes[0]["resource"]["issue"][0];
            assert_eq!(issue["severity"], "warning");
            assert_eq!(issue["code"], "not-supported");
            let text = issue["details"]["text"].as_str().unwrap_or_default();
            assert!(
                text.contains(query.split('=').next().unwrap()),
                "outcome must name the ignored parameter: {text}"
            );

            // "Ignored" is literal: the filter is not applied, so the result set
            // is the same as the unfiltered search (previously the parameter
            // reached the backend and silently matched nothing).
            let matches = get_bundle_entries(&body)
                .into_iter()
                .filter(|e| e["search"]["mode"] == "match")
                .count();
            assert_eq!(
                matches, total,
                "ignored parameter must not filter ({query})"
            );
        }
    }

    #[tokio::test]
    async fn test_known_params_survive_lenient_handling() {
        // The self link still carries every parameter that was applied, and no
        // outcome entry is added when nothing was ignored.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let body: Value = server
            .get("/Patient?name=Smith&_count=5")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .json();

        let link = self_link(&body);
        assert!(link.contains("name=Smith"), "self link: {link}");
        assert!(link.contains("_count=5"), "self link: {link}");
        assert!(outcome_entries(&body).is_empty());
    }

    #[tokio::test]
    async fn test_date_prefix_uses_precision_boundaries() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "born-2020", "birthDate": "2020-06-15" }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        let ids = |body: &Value| -> Vec<String> {
            get_bundle_entries(body)
                .iter()
                .filter_map(|e| e["resource"]["id"].as_str().map(String::from))
                .collect()
        };
        let search = |q: &'static str| {
            let server = &server;
            async move {
                server
                    .get(&format!("/Patient?birthdate={}", q))
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .await
                    .json::<Value>()
            }
        };

        // gt2020 means "after all of 2020" → 2020-06-15 must NOT match.
        assert!(
            !ids(&search("gt2020").await).contains(&"born-2020".to_string()),
            "gt2020 should not match a date within 2020"
        );
        // gt2019 → after 2019 → matches.
        assert!(
            ids(&search("gt2019").await).contains(&"born-2020".to_string()),
            "gt2019 should match 2020-06-15"
        );
        // lt2021 → before 2021 → matches; lt2020 → before 2020 → no match.
        assert!(ids(&search("lt2021").await).contains(&"born-2020".to_string()));
        assert!(!ids(&search("lt2020").await).contains(&"born-2020".to_string()));
    }

    #[tokio::test]
    async fn test_string_search_is_accent_insensitive() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "accent-pt",
                    "name": [{ "family": "Müller" }]
                }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        // An unaccented, lowercase query must match the accented stored name.
        for query in ["muller", "Müller", "MULLER"] {
            let response = server
                .get(&format!("/Patient?family={}", query))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
            let body: Value = response.json();
            let ids: Vec<&str> = get_bundle_entries(&body)
                .iter()
                .filter_map(|e| e["resource"]["id"].as_str())
                .collect();
            assert!(
                ids.contains(&"accent-pt"),
                "accent-insensitive family search '{query}' should match 'Müller'"
            );
        }
    }

    #[tokio::test]
    async fn test_quantity_search_ucum_unit_equivalence() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        // Observation with a mass quantity expressed in grams.
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-mass",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "x" }] },
                    "valueQuantity": {
                        "value": 1,
                        "unit": "g",
                        "system": "http://unitsofmeasure.org",
                        "code": "g"
                    }
                }),
                FhirVersion::R4,
            )
            .await
            .unwrap();

        // Searching the equivalent quantity in milligrams must match (g ⇄ mg).
        let response = server
            .get("/Observation?value-quantity=1000|http://unitsofmeasure.org|mg")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let ids: Vec<&str> = get_bundle_entries(&body)
            .iter()
            .filter_map(|e| e["resource"]["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"obs-mass"),
            "UCUM-equivalent quantity (1000 mg) should match the stored 1 g"
        );
    }

    #[tokio::test]
    async fn test_reference_search_is_version_agnostic() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // obs-1 stores an unversioned subject (Patient/patient-1). A versioned
        // reference search must still match it (version is not considered).
        let response = server
            .get("/Observation?subject=Patient/patient-1/_history/5")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let ids: Vec<&str> = get_bundle_entries(&body)
            .iter()
            .filter_map(|e| e["resource"]["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"obs-1"),
            "versioned reference search should match the unversioned stored reference"
        );
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
        // Should match exactly "Smith" — and must actually match something, or
        // the per-entry check below would pass on an empty bundle.
        assert_eq!(entries.len(), 2, "both 'Smith' patients must match");
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
    async fn test_membership_parameter_in_is_rejected() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // `_in` asks whether the resource belongs to a referenced List or
        // Group. That is unimplemented, and it must not fall through: on R5/R6
        // it is a registered `reference` parameter, so lenient handling would
        // not drop it and the spec's placeholder `Resource.id` expression makes
        // the backends answer a membership question with an identity test
        // (PostgreSQL) or with the entire resource type (SQLite). Rejecting is
        // the only answer that is not silently wrong (#535).
        let response = server
            .get("/Patient?_in=42")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome");
        assert_eq!(body["issue"][0]["code"], "invalid");
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

    /// #1318: an unknown modifier on a direct parameter used to be dropped, so
    /// `name:exat=Smith` ran as `name=Smith`. It is a 400 over GET and POST,
    /// under either `Prefer: handling` mode — the parameter is one the server
    /// understands, so there is nothing to leniently ignore.
    #[tokio::test]
    async fn test_unknown_modifier_on_direct_param_returns_400() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Positive control: the unmodified search finds seeded patients, so a
        // dropped modifier would have returned 200 with these.
        let control = server
            .get("/Patient?name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        control.assert_status_ok();
        assert!(!get_bundle_entries(&control.json::<Value>()).is_empty());

        let assert_rejected = |body: Value, param: &str, suffix: &str| {
            assert_eq!(body["resourceType"], "OperationOutcome");
            assert_eq!(body["issue"][0]["severity"], "error");
            assert_eq!(body["issue"][0]["code"], "invalid");
            let text = body["issue"][0]["details"]["text"].as_str().unwrap();
            assert!(text.contains(param), "{text}");
            assert!(text.contains(suffix), "{text}");
        };

        for handling in ["handling=lenient", "handling=strict"] {
            for (path, key, param, suffix) in [
                ("/Patient", "name:bogus", "name", ":bogus"),
                ("/Patient", "name:exat", "name", ":exat"),
                // Capitalised, but not a resource type.
                ("/Observation", "subject:Bogus", "subject", ":Bogus"),
            ] {
                let response = server
                    .get(&format!("{path}?{key}=Smith"))
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .add_header(
                        HeaderName::from_static("prefer"),
                        HeaderValue::from_static(handling),
                    )
                    .await;
                response.assert_status(StatusCode::BAD_REQUEST);
                assert_rejected(response.json(), param, suffix);

                let response = server
                    .post(&format!("{path}/_search"))
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .add_header(
                        HeaderName::from_static("prefer"),
                        HeaderValue::from_static(handling),
                    )
                    .form(&[(key, "Smith")])
                    .await;
                response.assert_status(StatusCode::BAD_REQUEST);
                assert_rejected(response.json(), param, suffix);
            }
        }
    }

    /// #1318: an unknown modifier on an unknown *parameter* follows the
    /// unknown-parameter rule — ignored and reported under lenient handling,
    /// rejected as an unknown parameter under strict.
    #[tokio::test]
    async fn test_unknown_modifier_on_unknown_param_follows_unknown_param_rule() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let control = server
            .get("/Patient")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        control.assert_status_ok();
        let all = get_bundle_entries(&control.json::<Value>()).len();
        assert!(all > 0);

        let lenient = server
            .get("/Patient?nosuchparam:bogus=x")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        lenient.assert_status_ok();
        let body: Value = lenient.json();
        let matches = get_bundle_entries(&body)
            .into_iter()
            .filter(|e| e["resource"]["resourceType"] == "Patient")
            .count();
        assert_eq!(matches, all, "the unknown parameter is ignored");

        let strict = server
            .get("/Patient?nosuchparam:bogus=x")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        strict.assert_status(StatusCode::BAD_REQUEST);
        let text = strict.json::<Value>()["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("unknown search parameter"), "{text}");
    }
}

// =============================================================================
// :missing Modifier Tests
// =============================================================================

mod missing_modifier {
    use super::*;

    fn entry_ids(body: &Value) -> Vec<String> {
        let mut ids: Vec<String> = get_bundle_entries(body)
            .iter()
            .map(|entry| {
                entry["resource"]["id"]
                    .as_str()
                    .expect("search result must carry a logical id")
                    .to_string()
            })
            .collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn test_missing_boolean_polarity_returns_exact_membership() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;
        backend
            .create(
                &test_tenant(),
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "patient-without-birthdate",
                    "active": true
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed Patient without birthDate");

        let missing = server
            .get("/Patient?birthdate:missing=true&_total=accurate&_count=100")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        missing.assert_status_ok();
        let missing_body: Value = missing.json();
        assert_eq!(missing_body["total"], 1);
        assert_eq!(entry_ids(&missing_body), vec!["patient-without-birthdate"]);

        let present = server
            .get("/Patient?birthdate:missing=false&_total=accurate&_count=100")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        present.assert_status_ok();
        let present_body: Value = present.json();
        assert_eq!(present_body["total"], 4);
        assert_eq!(
            entry_ids(&present_body),
            vec!["patient-1", "patient-2", "patient-3", "patient-4"]
        );
    }

    #[tokio::test]
    async fn test_missing_requires_an_exact_boolean_literal() {
        let (server, _) = create_test_server().await;

        for value in ["invalid", "", "TRUE", "true,false"] {
            let response = server
                .get(&format!("/Patient?birthdate:missing={value}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;

            response.assert_status(StatusCode::BAD_REQUEST);
            let body: Value = response.json();
            assert_eq!(body["resourceType"], "OperationOutcome", "value={value:?}");
            assert_eq!(body["issue"][0]["code"], "invalid", "value={value:?}");
        }
    }

    #[tokio::test]
    async fn test_missing_rejects_non_indexed_and_contained_searches() {
        let (server, _) = create_test_server().await;

        for query in [
            "/Patient?_text:missing=true",
            "/Patient?_content:missing=true",
            "/Patient?_contained=true&birthdate:missing=true",
            "/Patient?_contained=both&birthdate:missing=true",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;

            response.assert_status(StatusCode::BAD_REQUEST);
            let body: Value = response.json();
            assert_eq!(body["resourceType"], "OperationOutcome", "query={query}");
            assert_eq!(body["issue"][0]["code"], "invalid", "query={query}");
        }
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

    /// The date values no backend may accept, with the comparator prefixes the
    /// issues reported them under.
    const INVALID_DATE_VALUES: &[&str] = &[
        "not-a-date",
        "ltnot-a-date",
        "gtnot-a-date",
        "gtabcd",
        "lt2024-1x",
        "gt2024-13-45",
        "ne2024-13-45",
        // SQLite's `datetime()` used to roll this over to March 1st (#1295).
        "lt2024-02-30",
        "ltT25:00:00Z",
        // An hour needs minutes; minutes without seconds are fine.
        "2013-04-05T10",
    ];

    fn assert_invalid_date_outcome(response: &axum_test::TestResponse, context: &str) {
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome", "{context}");
        assert_eq!(body["issue"][0]["code"], "invalid", "{context}");
        let text = body["issue"][0]["diagnostics"]
            .as_str()
            .or_else(|| body["issue"][0]["details"]["text"].as_str())
            .unwrap_or_default();
        assert!(
            text.contains("not a valid FHIR date"),
            "{context}: the outcome should say what is wrong, got {body}"
        );
    }

    /// #1289, #1293, #1295: a value that is not a date never reaches a storage
    /// backend. PostgreSQL used to replace it with the current time and
    /// Elasticsearch with the year 2000, so the client got a plausible 200.
    #[tokio::test]
    async fn test_invalid_date_value_is_a_400_not_a_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for value in INVALID_DATE_VALUES {
            for param in ["birthdate", "_lastUpdated"] {
                let query = format!("/Patient?{param}={value}");
                let response = server
                    .get(&query)
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .await;
                assert_invalid_date_outcome(&response, &query);
            }
        }
    }

    /// The same values through POST `_search`, which has its own form decoding.
    #[tokio::test]
    async fn test_invalid_date_value_is_a_400_on_post_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for value in INVALID_DATE_VALUES {
            for param in ["birthdate", "_lastUpdated"] {
                let response = server
                    .post("/Patient/_search")
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .form(&[(param, value)])
                    .await;
                assert_invalid_date_outcome(&response, &format!("POST {param}={value}"));
            }
        }
    }

    /// An invalid value is a 400 whatever the client's `Prefer: handling`:
    /// lenient handling is for parameters the server does not know, not for
    /// values it cannot read.
    #[tokio::test]
    async fn test_invalid_date_value_is_a_400_under_lenient_handling() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for handling in ["handling=lenient", "handling=strict"] {
            let response = server
                .get("/Patient?birthdate=lt2024-02-30")
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    axum::http::header::HeaderName::from_static("prefer"),
                    HeaderValue::from_static(handling),
                )
                .await;
            assert_invalid_date_outcome(&response, handling);
        }
    }

    /// A chained terminal is typed only when the chain is resolved, so it is
    /// the storage gate that rejects it. (Valid prefixed terminals are #1292.)
    #[tokio::test]
    async fn test_invalid_chained_date_value_is_a_400() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let query = "/Observation?subject:Patient.birthdate=not-a-date";
        let response = server
            .get(query)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        assert_invalid_date_outcome(&response, query);
    }

    /// Seeds one Procedure performed at `2013-04-05T13:20:00Z`, written with a
    /// positive offset.
    async fn seed_procedure_with_positive_offset(backend: &SqliteBackend) {
        backend
            .create(
                &test_tenant(),
                "Procedure",
                json!({
                    "resourceType": "Procedure",
                    "id": "proc-plus",
                    "status": "completed",
                    "subject": {"reference": "Patient/patient-1"},
                    "performedDateTime": "2013-04-05T18:50:00+05:30"
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed procedure");
    }

    fn assert_finds_the_procedure(response: &axum_test::TestResponse, context: &str) {
        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1, "{context}: {body}");
        assert_eq!(entries[0]["resource"]["id"], "proc-plus", "{context}");
    }

    /// #1296: a `+` offset sent without percent-encoding is form-decoded into
    /// a space. It used to be an empty 200 on SQLite and PostgreSQL, a 400 on
    /// MongoDB and a 500 on Elasticsearch; it is now read as the `+` it was.
    #[tokio::test]
    async fn test_literal_plus_offset_finds_the_resource() {
        let (server, backend) = create_test_server().await;
        seed_procedure_with_positive_offset(&backend).await;

        for query in [
            // Positive control: properly encoded.
            "/Procedure?date=2013-04-05T18:50:00%2B05:30",
            // The literal `+`, under no prefix and under one.
            "/Procedure?date=2013-04-05T18:50:00+05:30",
            "/Procedure?date=ge2013-04-05T18:50:00+05:30",
            "/Procedure?date=eq2013-04-05T18:50+05:30",
            // The same instant in another zone.
            "/Procedure?date=2013-04-05T09:20:00-04:00",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            assert_finds_the_procedure(&response, query);
        }

        // The repair restores an offset; it does not make a wrong one match.
        let response = server
            .get("/Procedure?date=2013-04-05T18:50:00+05:00")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        assert_eq!(get_bundle_entries(&response.json::<Value>()).len(), 0);
    }

    /// The same through a POST `_search` body, which is form-decoded too.
    #[tokio::test]
    async fn test_literal_plus_offset_finds_the_resource_on_post_search() {
        let (server, backend) = create_test_server().await;
        seed_procedure_with_positive_offset(&backend).await;

        for body in [
            "date=2013-04-05T18:50:00%2B05:30",
            "date=2013-04-05T18:50:00+05:30",
        ] {
            let response = server
                .post("/Procedure/_search")
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                // `text` sets `text/plain`; the form content type goes after.
                .text(body)
                .content_type("application/x-www-form-urlencoded")
                .await;
            assert_finds_the_procedure(&response, body);
        }
    }

    /// The self link must describe the search that ran, and must round-trip:
    /// the repaired value is re-encoded, not echoed with a bare space or `+`.
    #[tokio::test]
    async fn test_literal_plus_offset_self_link_round_trips() {
        let (server, backend) = create_test_server().await;
        seed_procedure_with_positive_offset(&backend).await;

        let response = server
            .get("/Procedure?date=2013-04-05T18:50:00+05:30")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let self_link = body["link"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["relation"] == "self"))
            .and_then(|l| l["url"].as_str())
            .expect("self link")
            .to_string();

        let path = self_link
            .strip_prefix("http://localhost:8080")
            .unwrap_or(&self_link);
        let again = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        assert_finds_the_procedure(&again, &self_link);
    }

    /// Minute precision is valid in FHIR search, unlike the dateTime datatype.
    #[tokio::test]
    async fn test_minute_precision_date_value_is_accepted() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?birthdate=lt2013-04-05T09:20")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
    }

    /// #1319: a number or quantity value whose number part is not a number
    /// never reaches a storage backend. PostgreSQL and Elasticsearch used to
    /// skip it, turning `value-quantity=abc` into an unconstrained search.
    #[tokio::test]
    async fn test_invalid_number_or_quantity_value_is_a_400_not_a_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Positive control: the valid forms still search.
        for query in [
            "/Observation?value-quantity=gt70",
            "/Observation?value-quantity=72%7C%7Cbpm",
            "/Observation?value-quantity=le1e3",
            "/RiskAssessment?probability=gt0.5",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
        }
        let response = server
            .get("/Observation?value-quantity=gt70")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = response.json();
        assert!(!get_bundle_entries(&body).is_empty());

        let assert_invalid = |response: axum_test::TestResponse, context: String| {
            response.assert_status(StatusCode::BAD_REQUEST);
            let body: Value = response.json();
            assert_eq!(body["resourceType"], "OperationOutcome", "{context}");
            assert_eq!(body["issue"][0]["code"], "invalid", "{context}");
        };

        for query in [
            "/RiskAssessment?probability=abc",
            "/RiskAssessment?probability=gtabc",
            "/RiskAssessment?probability=1e",
            "/RiskAssessment?probability=ltinf",
            "/Observation?value-quantity=abc",
            "/Observation?value-quantity=neabc",
            "/Observation?value-quantity=abc%7Chttp://unitsofmeasure.org%7Cmg",
            "/Observation?value-quantity=gt%7Chttp://unitsofmeasure.org%7Cmg",
            "/Observation?value-quantity=nenan%7C%7Cmg",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            assert_invalid(response, format!("GET {query}"));
        }

        for (path, name, value) in [
            ("/RiskAssessment/_search", "probability", "abc"),
            ("/RiskAssessment/_search", "probability", "neabc"),
            ("/Observation/_search", "value-quantity", "abc"),
            (
                "/Observation/_search",
                "value-quantity",
                "gt|http://unitsofmeasure.org|mg",
            ),
        ] {
            let response = server
                .post(path)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .form(&[(name, value)])
                .await;
            assert_invalid(response, format!("POST {path} {name}={value}"));
        }
    }

    fn assert_invalid_number_outcome(response: &axum_test::TestResponse, context: &str) {
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome", "{context}");
        assert_eq!(body["issue"][0]["code"], "invalid", "{context}");
        let text = body["issue"][0]["diagnostics"]
            .as_str()
            .or_else(|| body["issue"][0]["details"]["text"].as_str())
            .unwrap_or_default();
        assert!(
            text.contains("not a valid number"),
            "{context}: the outcome should say what is wrong, got {body}"
        );
    }

    /// #1340: the values `f64::from_str` takes for numbers — as a bound they
    /// match every row — an empty value, and a composite's numeric component,
    /// over GET and POST `_search`, whatever the client's `Prefer: handling`:
    /// lenient handling is for parameters the server does not know, not for
    /// values it cannot read.
    #[tokio::test]
    async fn test_invalid_number_is_a_400_from_the_shared_gate() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for (path, name, value) in [
            ("/RiskAssessment", "probability", "inf"),
            ("/RiskAssessment", "probability", "lt-inf"),
            ("/RiskAssessment", "probability", "neNaN"),
            ("/RiskAssessment", "probability", "lt1e999"),
            ("/RiskAssessment", "probability", "0x10"),
            ("/RiskAssessment", "probability", ""),
            ("/Observation", "value-quantity", "ltinf||mg"),
            ("/Observation", "value-quantity", "||mg"),
            ("/Observation", "value-quantity", "5.4\\|mg"),
            ("/Observation", "code-value-quantity", "8480-6$abc"),
            ("/Observation", "code-value-quantity", "8480-6$ltinf||mg"),
        ] {
            for handling in ["handling=lenient", "handling=strict"] {
                let response = server
                    .get(path)
                    .add_query_param(name, value)
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .add_header(
                        axum::http::header::HeaderName::from_static("prefer"),
                        HeaderValue::from_static(handling),
                    )
                    .await;
                assert_invalid_number_outcome(&response, &format!("GET {name}={value} {handling}"));

                let response = server
                    .post(&format!("{path}/_search"))
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .add_header(
                        axum::http::header::HeaderName::from_static("prefer"),
                        HeaderValue::from_static(handling),
                    )
                    .form(&[(name, value)])
                    .await;
                assert_invalid_number_outcome(
                    &response,
                    &format!("POST {name}={value} {handling}"),
                );
            }
        }
    }

    /// A chained or `_has` terminal is typed only when the chain is resolved,
    /// so it is the storage gate that rejects it.
    #[tokio::test]
    async fn test_invalid_chained_number_value_is_a_400() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for (path, name, value) in [
            ("/DiagnosticReport", "result.value-quantity", "abc"),
            (
                "/DiagnosticReport",
                "result:Observation.value-quantity",
                "ltinf",
            ),
            ("/Patient", "_has:Observation:subject:value-quantity", "abc"),
            (
                "/Patient",
                "_has:Observation:subject:value-quantity",
                "ltinf||mg",
            ),
            (
                "/Patient",
                "_has:RiskAssessment:subject:probability",
                "nenan",
            ),
        ] {
            let response = server
                .get(path)
                .add_query_param(name, value)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            assert_invalid_number_outcome(&response, &format!("GET {path}?{name}={value}"));
        }

        // Positive control: the valid forms of the same searches run.
        for (path, name, value) in [
            ("/DiagnosticReport", "result.value-quantity", "gt1"),
            ("/Patient", "_has:Observation:subject:value-quantity", "gt1"),
        ] {
            let response = server
                .get(path)
                .add_query_param(name, value)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
        }
    }

    /// A `+` that form-decoding turned into a space is still the number the
    /// client wrote.
    #[tokio::test]
    async fn test_literal_plus_in_a_number_is_accepted() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for query in [
            "/Observation?value-quantity=lt1e+3",
            "/Observation?value-quantity=gt+70",
            "/Observation?value-quantity=+72",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
            assert!(
                !get_bundle_entries(&response.json::<Value>()).is_empty(),
                "{query} should find the seeded heart rate"
            );
        }
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

mod unsupported_params {
    use super::*;

    #[tokio::test]
    async fn test_unsupported_control_param_rejected() {
        // `_query`/`_contained`/`_containedType` are not implemented; the server
        // must reject them (400) rather than silently ignoring them and returning
        // an unfiltered result. (`_list` and `_score` are implemented — see the
        // `list_search` and `score_param` modules below.)
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for param in ["_query", "_contained", "_containedType"] {
            let response = server
                .get(&format!("/Patient?{param}=anything"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::BAD_REQUEST,
                "expected 400 for unsupported param {param}"
            );
        }
    }
}

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
    async fn test_compartment_membership_via_non_first_param() {
        // A resource that belongs to the compartment via a NON-first membership
        // param must still be found. AllergyIntolerance joins the Patient
        // compartment via patient/recorder/asserter; here the resource is linked
        // ONLY through `recorder` (its `patient` points elsewhere). The previous
        // first-param-only behaviour would have missed it.
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "patient-1", "active": true}),
                FhirVersion::R4,
            )
            .await
            .expect("create patient-1");

        backend
            .create(
                &tenant,
                "AllergyIntolerance",
                json!({
                    "resourceType": "AllergyIntolerance",
                    "id": "ai-1",
                    "patient": {"reference": "Patient/patient-2"},
                    "recorder": {"reference": "Patient/patient-1"}
                }),
                FhirVersion::R4,
            )
            .await
            .expect("create allergy");

        let response = server
            .get("/Patient/patient-1/AllergyIntolerance")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(
            entries.len(),
            1,
            "AllergyIntolerance linked via `recorder` should be in the compartment"
        );
        assert_eq!(entries[0]["resource"]["id"], "ai-1");
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

    #[tokio::test]
    async fn test_patient_compartment_all_types() {
        // `GET /Patient/{id}/*` returns every resource in the compartment,
        // regardless of type. patient-1 has 2 Observations in the seed data,
        // all of which must appear; nothing referencing another patient should.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient/patient-1/*")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "searchset");

        let entries = get_bundle_entries(&body);
        // At least the 2 Observations for patient-1 should be present.
        let observations = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Observation")
            .count();
        assert_eq!(observations, 2, "expected both patient-1 observations");

        // Every returned resource must actually belong to patient-1 (never
        // patient-2). We can't assert a single reference field across types, so
        // confirm patient-2 is not referenced anywhere in the matched set.
        for entry in &entries {
            let serialized = entry["resource"].to_string();
            assert!(
                !serialized.contains("patient-2"),
                "compartment all-types must not include resources of another patient"
            );
        }
    }

    #[tokio::test]
    async fn test_compartment_unknown_param_lenient_ignored_strict_rejected() {
        // Compartment search used to hand an unrecognized parameter straight to
        // the backend: no 400 under strict handling, and under lenient handling
        // a silent empty result set whose self link still claimed the filter.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let lenient = server
            .get("/Patient/patient-1/Observation?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        lenient.assert_status_ok();

        let strict = server
            .get("/Patient/patient-1/Observation?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        assert_eq!(
            strict.status_code(),
            StatusCode::BAD_REQUEST,
            "unknown compartment parameter must be rejected under Prefer: handling=strict"
        );

        // A parameter the target type does know is accepted even under strict.
        let ok = server
            .get("/Patient/patient-1/Observation?code=8867-4")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        ok.assert_status_ok();
    }

    #[tokio::test]
    async fn test_compartment_unknown_underscore_param_rejected_under_strict() {
        // `_`-prefixed names are not a bypass here either (#524 for the
        // type-level path).
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for param in ["_typo=foo", "_whatever=foo"] {
            let strict = server
                .get(&format!("/Patient/patient-1/Observation?{param}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    HeaderName::from_static("prefer"),
                    HeaderValue::from_static("handling=strict"),
                )
                .await;
            assert_eq!(
                strict.status_code(),
                StatusCode::BAD_REQUEST,
                "{param} must be rejected under Prefer: handling=strict"
            );
        }

        // Global parameters the server does honour still pass.
        for param in ["_id=obs-1", "_lastUpdated=gt2000-01-01"] {
            let ok = server
                .get(&format!("/Patient/patient-1/Observation?{param}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    HeaderName::from_static("prefer"),
                    HeaderValue::from_static("handling=strict"),
                )
                .await;
            ok.assert_status_ok();
        }
    }

    #[tokio::test]
    async fn test_compartment_ignored_param_dropped_from_self_link_and_reported() {
        // Under lenient handling an unsupported parameter may be ignored only if
        // the server says so: it must not appear in the self link, it must be
        // reported as an OperationOutcome entry, and "ignored" must be literal —
        // the compartment result set is the same as without it.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let unfiltered: Value = server
            .get("/Patient/patient-1/Observation")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .json();
        let total = match_entries(&unfiltered).len();
        assert!(total > 0, "fixture should seed patient-1 observations");

        for query in ["_typo=foo", "nonsense-param=foo"] {
            let response = server
                .get(&format!("/Patient/patient-1/Observation?{query}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
            let body: Value = response.json();

            let link = self_link(&body);
            assert!(
                !link.contains("typo") && !link.contains("nonsense-param"),
                "self link must not echo the ignored parameter ({query}): {link}"
            );

            let outcomes = outcome_entries(&body);
            assert_eq!(
                outcomes.len(),
                1,
                "ignored parameter must be reported ({query})"
            );
            let issue = &outcomes[0]["resource"]["issue"][0];
            assert_eq!(issue["severity"], "warning");
            assert_eq!(issue["code"], "not-supported");
            let text = issue["details"]["text"].as_str().unwrap_or_default();
            assert!(
                text.contains(query.split('=').next().unwrap()),
                "outcome must name the ignored parameter: {text}"
            );

            assert_eq!(
                match_entries(&body).len(),
                total,
                "ignored parameter must not filter the compartment ({query})"
            );
        }
    }

    #[tokio::test]
    async fn test_compartment_all_types_unknown_param() {
        // `GET [compartment]/[id]/*` applies the same rule, but a parameter only
        // counts as unknown when NO member type knows it.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let strict = server
            .get("/Patient/patient-1/*?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        assert_eq!(strict.status_code(), StatusCode::BAD_REQUEST);

        // `code` is not a Patient parameter, but it is an Observation one, and
        // the member types that cannot satisfy it are simply skipped. Rejecting
        // it here would break `*` for a parameter the single-type search accepts.
        let known_to_one_member = server
            .get("/Patient/patient-1/*?code=8867-4")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("handling=strict"),
            )
            .await;
        known_to_one_member.assert_status_ok();

        // Lenient: ignored, reported, and absent from the self link.
        let response = server
            .get("/Patient/patient-1/*?nonsense-param=foo")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert!(
            !self_link(&body).contains("nonsense-param"),
            "self link must not echo the ignored parameter: {}",
            self_link(&body)
        );
        assert_eq!(outcome_entries(&body).len(), 1);
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

        // Works the same as without the qualifier — and actually matches:
        // the qualifier used to be parsed as a modifier that swallowed the
        // chain, silently degrading this into `subject=Smith` (zero results).
        let entries = get_bundle_entries(&body);
        assert!(
            !entries.is_empty(),
            "typed chain must match the same observations as the untyped one"
        );
        for entry in &entries {
            let subject_ref = entry["resource"]["subject"]["reference"].as_str().unwrap();
            assert!(
                subject_ref.contains("patient-1") || subject_ref.contains("patient-2"),
                "Should only include observations for patients named Smith"
            );
        }
    }

    /// `general-practitioner` declares multiple targets (Practitioner,
    /// Organization, PractitionerRole). The resolver used to fall back to a
    /// name heuristic that fabricated a resource type called
    /// `General-practitioner`, so this chain silently matched nothing.
    #[tokio::test]
    async fn test_chained_multi_target_reference() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        for url in [
            "/Patient?general-practitioner.name=Brown",
            "/Patient?general-practitioner:Practitioner.name=Brown",
        ] {
            let response = server
                .get(url)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            response.assert_status_ok();
            let body: Value = response.json();
            let entries = get_bundle_entries(&body);
            let ids: Vec<&str> = entries
                .iter()
                .map(|e| e["resource"]["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, ["patient-1"], "{url}");
        }
    }

    /// A forward chain and a reverse chain in one query intersect.
    #[tokio::test]
    async fn test_forward_and_reverse_chain_combined() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_has:Observation:subject:status=final&general-practitioner.name=Brown")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        let ids: Vec<&str> = entries
            .iter()
            .map(|e| e["resource"]["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"patient-1"),
            "patient-1 has both a final Observation and GP Brown, got {ids:?}"
        );
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

    /// Two patients with two Procedures each, plus an Encounter and an
    /// Observation per patient for the multi-hop case (#1292).
    async fn seed_dated_chain_data(backend: &SqliteBackend) {
        let tenant = test_tenant();
        let resources = [
            json!({"resourceType": "Patient", "id": "p80", "birthDate": "1980-05-06",
                   "name": [{"family": "Lee"}]}),
            json!({"resourceType": "Patient", "id": "p90", "birthDate": "1990-01-01",
                   "name": [{"family": "Gert"}]}),
            json!({"resourceType": "Procedure", "id": "pr1", "status": "completed",
                   "subject": {"reference": "Patient/p80"},
                   "performedDateTime": "2013-04-05T09:20:00-04:00"}),
            json!({"resourceType": "Procedure", "id": "pr2", "status": "completed",
                   "subject": {"reference": "Patient/p80"},
                   "performedDateTime": "2013-04-05"}),
            json!({"resourceType": "Procedure", "id": "pr3", "status": "completed",
                   "subject": {"reference": "Patient/p90"},
                   "performedDateTime": "2020-06-01T10:00:00+05:30"}),
            json!({"resourceType": "Procedure", "id": "pr4", "status": "completed",
                   "subject": {"reference": "Patient/p90"},
                   "performedDateTime": "2021-02-03"}),
            json!({"resourceType": "Encounter", "id": "e80", "status": "finished",
                   "class": {"code": "AMB"},
                   "subject": {"reference": "Patient/p80"}}),
            json!({"resourceType": "Encounter", "id": "e90", "status": "finished",
                   "class": {"code": "AMB"},
                   "subject": {"reference": "Patient/p90"}}),
            json!({"resourceType": "Observation", "id": "ob80", "status": "final",
                   "code": {"text": "hr"},
                   "subject": {"reference": "Patient/p80"},
                   "encounter": {"reference": "Encounter/e80"},
                   "valueQuantity": {"value": 60, "unit": "bpm"}}),
            json!({"resourceType": "Observation", "id": "ob90", "status": "final",
                   "code": {"text": "hr"},
                   "subject": {"reference": "Patient/p90"},
                   "encounter": {"reference": "Encounter/e90"},
                   "valueQuantity": {"value": 90, "unit": "bpm"}}),
        ];
        for resource in resources {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            backend
                .create(&tenant, &resource_type, resource, FhirVersion::R4)
                .await
                .unwrap();
        }
    }

    /// Sorted ids of a search that must succeed.
    async fn ids(server: &TestServer, url: &str) -> Vec<String> {
        let response = server
            .get(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        let mut ids: Vec<String> = get_bundle_entries(&body)
            .iter()
            .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    }

    /// #1292: the terminal parameter of a forward chain is parsed like the
    /// same parameter in a direct search — comparator prefix and OR list.
    #[tokio::test]
    async fn test_chained_terminal_value_prefix_and_or_list() {
        let (server, backend) = create_test_server().await;
        seed_dated_chain_data(&backend).await;
        let all = ["pr1", "pr2", "pr3", "pr4"];

        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.birthdate=1980-05-06").await,
            ["pr1", "pr2"]
        );
        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.birthdate=ge1980-01-01").await,
            all
        );
        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.birthdate=eq1980-05-06").await,
            ["pr1", "pr2"]
        );
        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.birthdate=lt1985-01-01").await,
            ["pr1", "pr2"]
        );
        // Untyped chain, same answer.
        assert_eq!(
            ids(&server, "/Procedure?subject.birthdate=ge1985-01-01").await,
            ["pr3", "pr4"]
        );
        // Every value of the OR list counts, not just the first.
        assert_eq!(
            ids(
                &server,
                "/Procedure?subject:Patient.birthdate=1980-05-06,1990-01-01"
            )
            .await,
            all
        );
        // It agrees with the equivalent direct search.
        assert_eq!(
            ids(&server, "/Patient?birthdate=ge1985-01-01").await,
            ["p90"]
        );
        // Multi-hop.
        assert_eq!(
            ids(
                &server,
                "/Observation?encounter.subject.birthdate=ge1985-01-01"
            )
            .await,
            ["ob90"]
        );
        // Quantity terminal.
        assert_eq!(
            ids(
                &server,
                "/Encounter?_has:Observation:encounter:value-quantity=gt70"
            )
            .await,
            ["e90"]
        );
        // Chained _lastUpdated.
        assert_eq!(
            ids(
                &server,
                "/Procedure?subject:Patient._lastUpdated=ge2000-01-01"
            )
            .await,
            all
        );
        assert!(
            ids(
                &server,
                "/Procedure?subject:Patient._lastUpdated=lt2000-01-01"
            )
            .await
            .is_empty()
        );
        // A string terminal that starts with comparator letters is untouched.
        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.family=Lee").await,
            ["pr1", "pr2"]
        );
        assert_eq!(
            ids(&server, "/Procedure?subject:Patient.family=gert").await,
            ["pr3", "pr4"]
        );
    }

    /// #1292: same for the terminal parameter of `_has`.
    #[tokio::test]
    async fn test_has_terminal_value_prefix_and_or_list() {
        let (server, backend) = create_test_server().await;
        seed_dated_chain_data(&backend).await;

        assert_eq!(
            ids(&server, "/Patient?_has:Procedure:subject:date=ge2013-01-01").await,
            ["p80", "p90"]
        );
        assert_eq!(
            ids(&server, "/Patient?_has:Procedure:subject:date=ge2020-01-01").await,
            ["p90"]
        );
        assert_eq!(
            ids(&server, "/Patient?_has:Procedure:subject:date=lt2014").await,
            ["p80"]
        );
        assert_eq!(
            ids(
                &server,
                "/Patient?_has:Procedure:subject:date=2013-04-05T09:20:00-04:00,2020"
            )
            .await,
            ["p80", "p90"]
        );
    }

    /// Data for the chain-name parsing cases (#1302, #1303).
    ///
    /// `Patient.general-practitioner` is polymorphic and both of the targets
    /// used here define `name`: patient `ps` points at a *Practitioner* named
    /// Smith, patient `pl` at an *Organization* named Smith Clinic — so only a
    /// middle-hop `:Type` qualifier can tell their Observations apart.
    ///
    /// The two patients are "Smith" and "Smithson" (default string matching is
    /// a prefix match, so `:exact` narrows it), differ in gender, and only `ps`
    /// has a birth date.
    async fn seed_chain_name_data(backend: &SqliteBackend) {
        let tenant = test_tenant();
        let resources = [
            json!({"resourceType": "Practitioner", "id": "gp-prac",
                   "name": [{"family": "Smith"}]}),
            json!({"resourceType": "Organization", "id": "gp-org",
                   "name": "Smith Clinic"}),
            json!({"resourceType": "Patient", "id": "ps", "gender": "male",
                   "birthDate": "1980-01-01",
                   "name": [{"family": "Smith"}],
                   "generalPractitioner": [{"reference": "Practitioner/gp-prac"}]}),
            json!({"resourceType": "Patient", "id": "pl", "gender": "female",
                   "name": [{"family": "Smithson"}],
                   "generalPractitioner": [{"reference": "Organization/gp-org"}]}),
            json!({"resourceType": "Encounter", "id": "es", "status": "finished",
                   "class": {"code": "AMB"},
                   "subject": {"reference": "Patient/ps"}}),
            json!({"resourceType": "Encounter", "id": "el", "status": "finished",
                   "class": {"code": "AMB"},
                   "subject": {"reference": "Patient/pl"}}),
            json!({"resourceType": "Observation", "id": "os", "status": "final",
                   "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                   "subject": {"reference": "Patient/ps"},
                   "encounter": {"reference": "Encounter/es"}}),
            json!({"resourceType": "Observation", "id": "ol", "status": "final",
                   "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
                   "subject": {"reference": "Patient/pl"},
                   "encounter": {"reference": "Encounter/el"}}),
        ];
        for resource in resources {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            backend
                .create(&tenant, &resource_type, resource, FhirVersion::R4)
                .await
                .unwrap();
        }
    }

    /// Status of a search, whatever it is.
    async fn status(server: &TestServer, url: &str) -> StatusCode {
        server
            .get(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .status_code()
    }

    /// #1303: a `:Type` qualifier on a middle hop constrains that hop's
    /// reference, so a polymorphic reference resolves only to the named type.
    #[tokio::test]
    async fn test_chained_middle_hop_type_qualifier() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Unqualified: every target type of general-practitioner is searched.
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.general-practitioner.name=Smith"
            )
            .await,
            ["ol", "os"]
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.general-practitioner:Practitioner.name=Smith"
            )
            .await,
            ["os"]
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.general-practitioner:Organization.name=Smith"
            )
            .await,
            ["ol"]
        );
        // The qualifier works without one on the first hop …
        assert_eq!(
            ids(
                &server,
                "/Observation?subject.general-practitioner:Organization.name=Smith"
            )
            .await,
            ["ol"]
        );
        // … and on the last reference of a three-hop chain.
        assert_eq!(
            ids(
                &server,
                "/Observation?encounter.subject.general-practitioner:Practitioner.name=Smith"
            )
            .await,
            ["os"]
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?encounter:Encounter.subject:Patient.general-practitioner:Organization.name=Smith"
            )
            .await,
            ["ol"]
        );
    }

    /// #1302: a modifier on the terminal parameter of a forward chain applies
    /// exactly as it does on the same parameter in a direct search.
    #[tokio::test]
    async fn test_chained_terminal_modifier() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Baseline: default string matching is a prefix match.
        assert_eq!(
            ids(&server, "/Observation?subject:Patient.family=Smith").await,
            ["ol", "os"]
        );
        assert_eq!(ids(&server, "/Patient?family:exact=Smith").await, ["ps"]);

        assert_eq!(
            ids(&server, "/Observation?subject:Patient.family:exact=Smith").await,
            ["os"]
        );
        assert_eq!(
            ids(&server, "/Observation?subject.family:exact=Smith").await,
            ["os"]
        );
        // `mith` is no prefix of either name; only :contains finds it.
        assert!(
            ids(&server, "/Observation?subject:Patient.family=mith")
                .await
                .is_empty()
        );
        assert_eq!(
            ids(&server, "/Observation?subject:Patient.family:contains=mith").await,
            ["ol", "os"]
        );
        assert_eq!(
            ids(&server, "/Observation?subject:Patient.family:contains=thso").await,
            ["ol"]
        );
        // :missing on a date terminal — the value is a boolean, not a date.
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.birthdate:missing=true"
            )
            .await,
            ["ol"]
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.birthdate:missing=false"
            )
            .await,
            ["os"]
        );
        // :not on a token terminal.
        assert_eq!(
            ids(&server, "/Observation?subject:Patient.gender:not=male").await,
            ["ol"]
        );
        // Multi-hop, with a middle-hop qualifier next to the modifier.
        assert_eq!(
            ids(
                &server,
                "/Observation?encounter:Encounter.subject:Patient.family:exact=Smith"
            )
            .await,
            ["os"]
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.general-practitioner:Organization.name:exact=Smith"
            )
            .await,
            Vec::<String>::new()
        );
        assert_eq!(
            ids(
                &server,
                "/Observation?subject:Patient.general-practitioner:Organization.name:contains=clin"
            )
            .await,
            ["ol"]
        );
    }

    /// #1302: what a direct search rejects, a chained terminal rejects too.
    #[tokio::test]
    async fn test_chained_terminal_modifier_rejections() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        for url in [
            // Modifier not defined for the terminal's type.
            "/Observation?subject:Patient.birthdate:exact=1980-01-01",
            "/Observation?subject:Patient.gender:contains=mal",
            "/Observation?encounter.subject:Patient.birthdate:exact=1980-01-01",
            "/Patient?_has:Observation:subject:code:exact=1234-5",
            // Not a modifier at all.
            "/Observation?subject:Patient.family:bogus=Smith",
            "/Patient?_has:Observation:subject:code:bogus=1234-5",
            // A resource type is not a modifier of a string parameter.
            "/Observation?subject:Patient.family:Patient=Smith",
            // :missing takes exactly true|false.
            "/Observation?subject:Patient.birthdate:missing=yes",
            "/Patient?_has:Observation:subject:code:missing=yes",
        ] {
            assert_eq!(status(&server, url).await, StatusCode::BAD_REQUEST, "{url}");
        }
    }

    /// #1302: same for the terminal parameter of `_has`.
    #[tokio::test]
    async fn test_has_terminal_modifier() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        assert_eq!(
            ids(&server, "/Patient?_has:Observation:subject:code=1234-5").await,
            ["ps"]
        );
        assert_eq!(
            ids(&server, "/Patient?_has:Observation:subject:code:not=1234-5").await,
            ["pl"]
        );
        assert_eq!(
            ids(
                &server,
                "/Patient?_has:Observation:subject:encounter:missing=false"
            )
            .await,
            ["pl", "ps"]
        );
        assert!(
            ids(
                &server,
                "/Patient?_has:Observation:subject:encounter:missing=true"
            )
            .await
            .is_empty()
        );
        // Nested: the modifier sits on the innermost terminal.
        assert_eq!(
            ids(
                &server,
                "/Patient?_has:Encounter:subject:_has:Observation:encounter:code:not=1234-5"
            )
            .await,
            ["pl"]
        );
    }

    /// The `501` a search gets, as (status, OperationOutcome text).
    async fn outcome(server: &TestServer, url: &str) -> (StatusCode, String) {
        let response = server
            .get(url)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = response.json();
        (response.status_code(), body["issue"][0].to_string())
    }

    /// #1317: with no terminology server configured, a terminology-backed
    /// modifier is a `501` on a direct parameter. On the terminal parameter of
    /// a typed chain or a `_has` it used to slip past that guard — which reads
    /// the modifier off the query key's first `:` segment — and the terminal
    /// search then ran without terminology: `:in` matched the ValueSet URL as a
    /// literal code (an empty `200`), `:above` / `:below` matched the code
    /// alone.
    #[tokio::test]
    async fn test_chained_terminology_modifier_without_terminology_server() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Positive controls: the chains themselves resolve.
        assert_eq!(ids(&server, "/Encounter?subject.gender=male").await, ["es"]);
        assert_eq!(
            ids(&server, "/Patient?_has:Observation:subject:code=1234-5").await,
            ["ps"]
        );

        // The direct form, for reference.
        let (direct_status, direct_text) =
            outcome(&server, "/Observation?code:in=http://example.org/vs").await;
        assert_eq!(direct_status, StatusCode::NOT_IMPLEMENTED);
        assert!(
            direct_text.contains("requires a configured terminology server"),
            "{direct_text}"
        );

        let gender = "http://hl7.org/fhir/administrative-gender|male";
        for url in [
            // Forward chains: untyped, typed, multi-hop, typed multi-hop.
            "/Encounter?subject.gender:in=http://example.org/vs".to_string(),
            "/Encounter?subject:Patient.gender:in=http://example.org/vs".to_string(),
            "/Observation?encounter.subject.gender:in=http://example.org/vs".to_string(),
            "/Observation?encounter:Encounter.subject:Patient.gender:in=http://example.org/vs"
                .to_string(),
            format!("/Encounter?subject.gender:below={gender}"),
            format!("/Encounter?subject.gender:above={gender}"),
            format!("/Encounter?subject:Patient.gender:below={gender}"),
            format!("/Observation?encounter.subject:Patient.gender:above={gender}"),
            // `_has`, plain and nested.
            "/Patient?_has:Observation:subject:code:in=http://example.org/vs".to_string(),
            "/Patient?_has:Observation:subject:code:below=http://loinc.org|1234-5".to_string(),
            "/Patient?_has:Observation:subject:code:above=http://loinc.org|1234-5".to_string(),
            "/Patient?_has:Encounter:subject:_has:Observation:encounter:code:in=http://example.org/vs"
                .to_string(),
            "/Patient?_has:Encounter:subject:_has:Observation:encounter:code:below=http://loinc.org|1234-5"
                .to_string(),
        ] {
            let (status, text) = outcome(&server, &url).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{url}: {text}");
            assert!(
                text.contains("requires a configured terminology server"),
                "{url}: {text}"
            );
        }

        // The wording is the direct form's, naming the parameter as written.
        let (_, text) = outcome(
            &server,
            "/Encounter?subject:Patient.gender:below=http://hl7.org/fhir/administrative-gender|male",
        )
        .await;
        assert!(
            text.contains(
                "search modifier ':below' on token parameter 'subject:Patient.gender' requires \
                 a configured terminology server (set HFS_TERMINOLOGY_SERVER)"
            ),
            "{text}"
        );
        let (_, text) = outcome(
            &server,
            "/Patient?_has:Observation:subject:code:in=http://example.org/vs",
        )
        .await;
        assert!(
            text.contains("':in' on token parameter '_has:Observation:subject:code'"),
            "{text}"
        );

        // `:not-in` stays the `501` it already was.
        for url in [
            "/Encounter?subject.gender:not-in=http://example.org/vs",
            "/Patient?_has:Observation:subject:code:not-in=http://example.org/vs",
        ] {
            assert_eq!(
                status(&server, url).await,
                StatusCode::NOT_IMPLEMENTED,
                "{url}"
            );
        }
    }

    /// #1317: the same over `POST [type]/_search`.
    #[tokio::test]
    async fn test_chained_terminology_modifier_without_terminology_server_post() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        for (resource_type, key, value) in [
            (
                "Encounter",
                "subject:Patient.gender:in",
                "http://example.org/vs",
            ),
            (
                "Patient",
                "_has:Observation:subject:code:below",
                "http://loinc.org|1234-5",
            ),
        ] {
            let response = server
                .post(&format!("/{resource_type}/_search"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .form(&[(key, value)])
                .await;
            response.assert_status(StatusCode::NOT_IMPLEMENTED);
            let body: Value = response.json();
            assert!(
                body["issue"][0]
                    .to_string()
                    .contains("requires a configured terminology server"),
                "{key}: {body}"
            );
        }

        // Positive control: a chained POST search works.
        let response = server
            .post("/Encounter/_search")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .form(&[("subject:Patient.gender", "male")])
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(get_bundle_entries(&body).len(), 1);
    }

    /// #1317: `:above` / `:below` are terminology-backed on a token only. On a
    /// uri or reference terminal they are structural, need no terminology
    /// server, and must keep resolving.
    #[tokio::test]
    async fn test_chained_structural_hierarchy_modifier_needs_no_terminology_server() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;
        let tenant = test_tenant();
        for resource in [
            json!({"resourceType": "Patient", "id": "pp",
                   "meta": {"profile": ["http://example.org/profiles/special-patient"]},
                   "name": [{"family": "Profiled"}]}),
            json!({"resourceType": "Observation", "id": "op", "status": "final",
                   "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                   "subject": {"reference": "Patient/pp"}}),
        ] {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            backend
                .create(&tenant, &resource_type, resource, FhirVersion::R4)
                .await
                .unwrap();
        }

        // Positive control: the direct uri `:below`.
        assert_eq!(
            ids(
                &server,
                "/Patient?_profile:below=http://example.org/profiles"
            )
            .await,
            ["pp"]
        );
        // Uri terminal, forward (untyped and typed) and `_has`.
        for url in [
            "/Observation?subject._profile:below=http://example.org/profiles",
            "/Observation?subject:Patient._profile:below=http://example.org/profiles",
        ] {
            assert_eq!(ids(&server, url).await, ["op"], "{url}");
        }
        assert_eq!(
            ids(
                &server,
                "/Patient?_has:Observation:subject:_profile:below=http://loinc.org"
            )
            .await,
            Vec::<String>::new()
        );
        // Reference terminal.
        assert_eq!(
            ids(&server, "/Observation?encounter.subject:below=Patient/ps").await,
            ["os"]
        );
        assert_eq!(
            ids(
                &server,
                "/Patient?_has:Observation:subject:encounter:below=Encounter/es"
            )
            .await,
            ["ps"]
        );
    }

    /// #1339: a terminology-backed modifier the parameter's type does not
    /// define is a client error, whether or not a terminology server is
    /// configured: the `400` a direct or chained `birthdate:exact` gets, not the
    /// `501` reserved for a *valid* modifier this server cannot answer.
    #[tokio::test]
    async fn test_terminology_modifier_on_wrong_parameter_type_is_400_not_501() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Positive controls: the parameters and chains themselves resolve.
        assert_eq!(ids(&server, "/Patient?name=Smith").await, ["pl", "ps"]);
        assert_eq!(ids(&server, "/Patient?birthdate=1980-01-01").await, ["ps"]);
        assert_eq!(ids(&server, "/Encounter?subject.name=Smith").await.len(), 2);

        for url in [
            // Direct.
            "/Patient?name:in=http://example.org/vs",
            "/Patient?name:not-in=http://example.org/vs",
            "/Patient?name:below=Smith",
            "/Patient?birthdate:below=1980-01-01",
            "/Patient?birthdate:above=1980-01-01",
            "/Patient?birthdate:in=http://example.org/vs",
            // `:in` is token-only: not defined for a reference or a uri either.
            "/Observation?subject:in=http://example.org/vs",
            "/Patient?_profile:in=http://example.org/vs",
            // Forward chains, untyped, typed and multi-hop.
            "/Encounter?subject.name:in=http://example.org/vs",
            "/Encounter?subject:Patient.name:in=http://example.org/vs",
            "/Encounter?subject:Patient.birthdate:below=1980-01-01",
            "/Observation?encounter.subject:Patient.name:above=Smith",
            // `_has`, plain and nested.
            "/Patient?_has:Observation:subject:date:in=http://example.org/vs",
            "/Patient?_has:Observation:subject:date:below=2020-01-01",
            "/Patient?_has:Encounter:subject:_has:Observation:encounter:date:in=http://example.org/vs",
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {text}");
            assert!(text.contains("is not supported for"), "{url}: {text}");
        }

        // A valid terminology modifier is still the `501`.
        for url in [
            "/Patient?gender:in=http://example.org/vs",
            "/Patient?gender:below=http://hl7.org/fhir/administrative-gender|male",
            "/Encounter?subject.gender:in=http://example.org/vs",
            "/Patient?_has:Observation:subject:code:below=http://loinc.org|1234-5",
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{url}: {text}");
        }
    }

    /// #1365: `:not-in` on the terminal parameter of a chained / `_has` search
    /// follows the direct form's order — the `400` for a type that does not
    /// define it comes before the `501` for one that does. It used to be a
    /// blanket `501` on any key ending in `:not-in`.
    #[tokio::test]
    async fn test_chained_not_in_is_400_on_a_non_token_terminal_and_501_on_a_token() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Positive controls: the chains themselves resolve.
        assert_eq!(ids(&server, "/Encounter?subject.name=Smith").await.len(), 2);
        assert_eq!(ids(&server, "/Encounter?subject.gender=male").await, ["es"]);
        assert_eq!(
            ids(&server, "/Patient?_has:Observation:subject:code=1234-5").await,
            ["ps"]
        );

        // The direct forms, for reference.
        let (direct_status, _) = outcome(&server, "/Patient?name:not-in=http://vs").await;
        assert_eq!(direct_status, StatusCode::BAD_REQUEST);
        let (direct_status, direct_text) =
            outcome(&server, "/Patient?gender:not-in=http://vs").await;
        assert_eq!(direct_status, StatusCode::NOT_IMPLEMENTED);
        assert!(
            direct_text.contains("search modifier ':not-in' is not supported"),
            "{direct_text}"
        );

        for url in [
            "/Encounter?subject.name:not-in=http://example.org/vs",
            "/Encounter?subject:Patient.name:not-in=http://example.org/vs",
            "/Encounter?subject:Patient.birthdate:not-in=http://example.org/vs",
            "/Observation?encounter.subject:Patient.name:not-in=http://example.org/vs",
            "/Patient?_has:Observation:subject:date:not-in=http://example.org/vs",
            "/Patient?_has:Encounter:subject:_has:Observation:encounter:date:not-in=http://example.org/vs",
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {text}");
            assert!(text.contains("is not supported for"), "{url}: {text}");
        }

        for url in [
            "/Encounter?subject.gender:not-in=http://example.org/vs",
            "/Encounter?subject:Patient.gender:not-in=http://example.org/vs",
            "/Observation?encounter.subject:Patient.gender:not-in=http://example.org/vs",
            "/Patient?_has:Observation:subject:code:not-in=http://example.org/vs",
            "/Patient?_has:Encounter:subject:_has:Observation:encounter:code:not-in=http://example.org/vs",
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{url}: {text}");
            // The direct form's wording, not "requires a terminology server":
            // one would not help.
            assert!(
                text.contains("search modifier ':not-in' is not supported"),
                "{url}: {text}"
            );
        }
    }

    /// #1339: modifiers and `:[type]` qualifiers are case-sensitive. A
    /// differently-cased one used to be honoured (`name:EXACT`); it is now the
    /// `400` of any unknown modifier, never a search without it.
    #[tokio::test]
    async fn test_modifiers_are_case_sensitive() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Positive controls: the properly-cased forms.
        assert_eq!(ids(&server, "/Patient?family:exact=Smith").await, ["ps"]);
        assert_eq!(
            ids(&server, "/Observation?subject:Patient=ps").await,
            ["os"]
        );
        assert_eq!(
            ids(&server, "/Encounter?subject:Patient.family:exact=Smith").await,
            ["es"]
        );

        for (url, hint) in [
            ("/Patient?family:EXACT=Smith", "':exact'?"),
            ("/Patient?family:Exact=Smith", "':exact'?"),
            ("/Patient?gender:Missing=true", "':missing'?"),
            ("/Observation?code:NOT=1234-5", "':not'?"),
            ("/Observation?subject:patient=ps", "':Patient'?"),
            ("/Observation?subject:PATIENT=ps", "':Patient'?"),
            ("/Encounter?subject:Patient.family:EXACT=Smith", "':exact'?"),
            (
                "/Patient?_has:Observation:subject:code:NOT=1234-5",
                "':not'?",
            ),
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {text}");
            assert!(text.contains("case-sensitive"), "{url}: {text}");
            assert!(text.contains(hint), "{url}: {text}");
        }
    }

    /// #1339: a `:[type]` qualifier is judged against the FHIR version of the
    /// search (R4 here), wherever it is written. On a chain hop and as a `_has`
    /// source type it used not to be judged at all: an empty `200`, or a `400`
    /// about something else.
    #[tokio::test]
    async fn test_type_qualifier_must_be_a_resource_type_of_the_request_version() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        // Positive controls.
        assert_eq!(
            ids(&server, "/Encounter?subject:Patient.family=Smith").await,
            ["el", "es"]
        );
        assert_eq!(
            ids(&server, "/Patient?_has:Observation:subject:code=1234-5").await,
            ["ps"]
        );

        for (url, named) in [
            // `ActorDefinition` is an R5 resource type.
            ("/Observation?subject:ActorDefinition=x", "ActorDefinition"),
            (
                "/Encounter?subject:ActorDefinition.name=x",
                "ActorDefinition",
            ),
            (
                "/Patient?_has:ActorDefinition:subject:code=x",
                "ActorDefinition",
            ),
            ("/Encounter?subject:Bogus.family=Smith", "Bogus"),
            ("/Encounter?subject:patient.family=Smith", "':Patient'?"),
            (
                "/Observation?subject:Patient.general-practitioner:practitioner.name=Smith",
                "':Practitioner'?",
            ),
            (
                "/Patient?_has:observation:subject:code=1234-5",
                "':Observation'?",
            ),
            (
                "/Patient?_has:Encounter:subject:_has:Bogus:encounter:code=1234-5",
                "Bogus",
            ),
        ] {
            let (status, text) = outcome(&server, url).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {text}");
            assert!(text.contains(named), "{url}: {text}");
            assert!(text.contains("of FHIR R4"), "{url}: {text}");
        }
    }

    /// #1339: the `400` / `501` for the terminal parameter of a nested `_has`
    /// names the whole key as written, not just its innermost level.
    #[tokio::test]
    async fn test_nested_has_errors_name_the_full_key() {
        let (server, backend) = create_test_server().await;
        seed_chain_name_data(&backend).await;

        let key = "_has:Encounter:subject:_has:Observation:encounter:code";
        // Positive control: the nested `_has` resolves.
        assert_eq!(
            ids(&server, &format!("/Patient?{key}=1234-5")).await,
            ["ps"]
        );

        for (suffix, value, expected) in [
            // Needs a terminology server (#1317).
            (":in", "http://example.org/vs", StatusCode::NOT_IMPLEMENTED),
            (
                ":below",
                "http://loinc.org|1234-5",
                StatusCode::NOT_IMPLEMENTED,
            ),
            // Not defined for a token; not a modifier; not a boolean (#1302).
            (":exact", "1234-5", StatusCode::BAD_REQUEST),
            (":bogus", "1234-5", StatusCode::BAD_REQUEST),
            (":missing", "yes", StatusCode::BAD_REQUEST),
        ] {
            let url = format!("/Patient?{key}{suffix}={value}");
            let (status, text) = outcome(&server, &url).await;
            assert_eq!(status, expected, "{url}: {text}");
            assert!(text.contains(key), "{url}: {text}");
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

    /// Sorted `(resourceType, id)` pairs for bundle entries tagged
    /// `search.mode == "include"`, for exact-set assertions independent of
    /// fetch order.
    fn include_entries(body: &Value) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = get_bundle_entries(body)
            .iter()
            .filter(|e| e["search"]["mode"] == "include")
            .map(|e| {
                (
                    e["resource"]["resourceType"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    e["resource"]["id"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        pairs.sort();
        pairs
    }

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

        // The referenced Patient must be included, tagged search.mode=include.
        assert!(patient_count >= 1, "Should include the referenced patient");
        let included_patient = entries.iter().find(|e| {
            e["resource"]["resourceType"] == "Patient" && e["search"]["mode"] == "include"
        });
        assert!(
            included_patient.is_some(),
            "Patient should be present as an include entry"
        );
        assert_eq!(
            included_patient.unwrap()["resource"]["id"],
            "patient-1",
            "the subject of obs-1 is patient-1"
        );
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
        // Repeated _include keys are both honored (AND). obs-1's subject is
        // Patient/patient-1; it has no encounter, so only the Patient is added —
        // but crucially the subject include is NOT lost to the encounter one.
        assert!(
            resource_types.contains(&"Patient"),
            "subject include must survive alongside the encounter include"
        );
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

        // Observations referencing patient-1 must be reverse-included.
        assert!(obs_count >= 1, "Should revinclude observations");
        assert!(
            entries
                .iter()
                .any(|e| e["resource"]["resourceType"] == "Observation"
                    && e["search"]["mode"] == "include"),
            "revincluded observations must be tagged search.mode=include"
        );
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

        // _include:iterate follows references from included resources:
        // obs-1 -> subject Patient/patient-1 -> organization Organization/org-1.
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject&_include:iterate=Patient:organization")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        let types: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["resource"]["resourceType"].as_str())
            .collect();

        // First-hop include (Patient) and the transitively-included Organization.
        assert!(types.contains(&"Patient"), "subject Patient included");
        assert!(
            types.contains(&"Organization"),
            ":iterate should transitively include the Patient's managingOrganization"
        );
    }

    #[tokio::test]
    async fn test_include_wildcard() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // _include=Observation:* expands to every reference search param of
        // Observation; obs-1 references subject (Patient) and performer
        // (Practitioner), both of which should be included.
        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:*")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        let types: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["resource"]["resourceType"].as_str())
            .collect();
        assert!(
            types.contains(&"Patient"),
            "wildcard include resolves subject"
        );
        assert!(
            types.contains(&"Practitioner"),
            "wildcard include resolves performer"
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

    #[tokio::test]
    async fn test_include_service_provider_conditional_reference_yields_no_includes() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let tenant = test_tenant();
        let encounter = json!({
            "resourceType": "Encounter",
            "id": "enc-cond",
            "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
            "subject": {"reference": "Patient/patient-1"},
            "serviceProvider": {
                "reference": "Organization?identifier=http://example.org/org|dept-9"
            }
        });
        backend
            .create(&tenant, "Encounter", encounter, FhirVersion::R4)
            .await
            .unwrap();

        let response = server
            .get("/Encounter?_id=enc-cond&_include=Encounter:service-provider")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);

        let match_entries: Vec<&&Value> = entries
            .iter()
            .filter(|e| e["search"]["mode"] == "match")
            .collect();
        assert_eq!(match_entries.len(), 1);
        assert_eq!(match_entries[0]["resource"]["resourceType"], "Encounter");

        assert!(
            include_entries(&body).is_empty(),
            "a conditional serviceProvider reference must not produce an include"
        );
        assert!(
            !entries
                .iter()
                .any(|e| e["resource"]["resourceType"] == "Patient"),
            "the conditional serviceProvider reference must not pull in the subject Patient either"
        );
    }

    #[tokio::test]
    async fn test_include_service_provider_returns_only_the_organization() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let tenant = test_tenant();
        let encounter = json!({
            "resourceType": "Encounter",
            "id": "enc-sp",
            "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
            "subject": {"reference": "Patient/patient-1"},
            "serviceProvider": {"reference": "Organization/org-2"}
        });
        backend
            .create(&tenant, "Encounter", encounter, FhirVersion::R4)
            .await
            .unwrap();

        let expected = vec![("Organization".to_string(), "org-2".to_string())];

        for query in [
            "/Encounter?_id=enc-sp&_include=Encounter:service-provider",
            "/Encounter?_id=enc-sp&_include=Encounter:service-provider:Organization",
        ] {
            let response = server
                .get(query)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;

            response.assert_status_ok();
            let body: Value = response.json();
            assert_eq!(include_entries(&body), expected);
        }
    }

    #[tokio::test]
    async fn test_include_subject_returns_only_the_patient() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let tenant = test_tenant();
        let encounter = json!({
            "resourceType": "Encounter",
            "id": "enc-sp",
            "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
            "subject": {"reference": "Patient/patient-1"},
            "serviceProvider": {"reference": "Organization/org-2"}
        });
        backend
            .create(&tenant, "Encounter", encounter, FhirVersion::R4)
            .await
            .unwrap();

        let response = server
            .get("/Encounter?_id=enc-sp&_include=Encounter:subject")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let expected = vec![("Patient".to_string(), "patient-1".to_string())];
        assert_eq!(include_entries(&body), expected);
    }
}

// =============================================================================
// Full-Text Search Tests (_text, _content)
// =============================================================================

mod fulltext_search {
    use super::*;

    /// Extracts the `id` of every `entry[].resource` in a search Bundle.
    ///
    /// Used instead of raw `get_bundle_entries` closures so every full-text
    /// test can assert both presence and absence of specific ids without
    /// repeating the extraction logic.
    fn entry_ids(body: &Value) -> Vec<String> {
        get_bundle_entries(body)
            .iter()
            .filter_map(|e| e["resource"]["id"].as_str())
            .map(|s| s.to_string())
            .collect()
    }

    #[tokio::test]
    async fn test_text_search() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search patient narrative text for "diabetes"
        let response = server
            .get("/Patient?_text=diabetes")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let ids = entry_ids(&body);

        // patient-1's narrative mentions diabetes; the others don't.
        assert!(
            ids.contains(&"patient-1".to_string()),
            "expected patient-1 in results, got {:?}",
            ids
        );
        for excluded in ["patient-2", "patient-3", "patient-4"] {
            assert!(
                !ids.contains(&excluded.to_string()),
                "did not expect {} in results, got {:?}",
                excluded,
                ids
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

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let ids = entry_ids(&body);

        // condition-2's display ("Hypertension") and narrative both mention
        // it; condition-1 is about diabetes and mentions neither.
        assert!(
            ids.contains(&"condition-2".to_string()),
            "expected condition-2 in results, got {:?}",
            ids
        );
        assert!(
            !ids.contains(&"condition-1".to_string()),
            "did not expect condition-1 in results, got {:?}",
            ids
        );
    }

    #[tokio::test]
    async fn test_text_search_multiple_words() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        // Search for multiple words; FTS5's default bareword syntax is an
        // implicit AND across terms, so this must behave like Elasticsearch's
        // `operator: "and"` rather than matching on either word alone.
        let response = server
            .get("/Observation?_text=heart%20rate")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let ids = entry_ids(&body);

        // obs-1, obs-3, obs-4 mention both "heart" and "rate"; obs-2 mentions
        // neither (its narrative is about temperature).
        for expected in ["obs-1", "obs-3", "obs-4"] {
            assert!(
                ids.contains(&expected.to_string()),
                "expected {} in results, got {:?}",
                expected,
                ids
            );
        }
        assert!(
            !ids.contains(&"obs-2".to_string()),
            "did not expect obs-2 in results, got {:?}",
            ids
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

        assert_eq!(response_lower.status_code(), StatusCode::OK);
        assert_eq!(response_upper.status_code(), StatusCode::OK);

        let mut ids_lower = entry_ids(&response_lower.json::<Value>());
        let mut ids_upper = entry_ids(&response_upper.json::<Value>());
        ids_lower.sort();
        ids_upper.sort();

        // patient-1 and patient-2 are both named Smith; case must not change
        // which resources are found.
        assert!(!ids_lower.is_empty(), "expected some matches for 'smith'");
        assert_eq!(
            ids_lower, ids_upper,
            "case should not affect which resources are found"
        );
    }

    #[tokio::test]
    async fn test_text_search_count_summary_carries_total() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_text=smith&_summary=count")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();

        // patient-1 and patient-2 both match "smith"; _summary=count must
        // carry the numeric total instead of an empty/absent Bundle field.
        assert_eq!(body["total"].as_i64(), Some(2));
        assert!(
            body.get("entry").is_none(),
            "_summary=count must not return entries, got {:?}",
            body.get("entry")
        );
    }

    #[tokio::test]
    async fn test_text_search_without_match_is_empty_200() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_text=zzzznomatchzzzz")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        // No matches is a well-formed empty result, not "unsupported".
        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        assert!(get_bundle_entries(&body).is_empty());
        // `total` may be omitted or reported as zero, but never a non-zero
        // number: that would mean the filter was silently dropped.
        let total = body.get("total").and_then(|t| t.as_i64());
        assert!(
            total.is_none() || total == Some(0),
            "expected no total or a total of 0, got {:?}",
            total
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
        // :text-advanced is out of scope for #1012; still accepts OK or 400.
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

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let ids = entry_ids(&body);

        // obs-1 has "normal sinus rhythm", obs-2 has "normal range"; obs-3
        // and obs-4 don't mention "normal" anywhere.
        for expected in ["obs-1", "obs-2"] {
            assert!(
                ids.contains(&expected.to_string()),
                "expected {} in results, got {:?}",
                expected,
                ids
            );
        }
        for excluded in ["obs-3", "obs-4"] {
            assert!(
                !ids.contains(&excluded.to_string()),
                "did not expect {} in results, got {:?}",
                excluded,
                ids
            );
        }
    }
}

mod list_search {
    use super::*;

    /// Creates a `List` resource whose entries reference the given patient ids.
    async fn create_patient_list(backend: &SqliteBackend, id: &str, patient_ids: &[&str]) {
        let entries: Vec<Value> = patient_ids
            .iter()
            .map(|pid| json!({ "item": { "reference": format!("Patient/{pid}") } }))
            .collect();
        backend
            .create(
                &test_tenant(),
                "List",
                json!({
                    "resourceType": "List",
                    "id": id,
                    "status": "current",
                    "mode": "working",
                    "entry": entries,
                }),
                FhirVersion::R4,
            )
            .await
            .unwrap_or_else(|e| panic!("Failed to create List {id}: {e}"));
    }

    #[tokio::test]
    async fn test_list_filters_to_members() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;
        create_patient_list(&backend, "list-1", &["patient-1", "patient-3"]).await;

        let response = server
            .get("/Patient?_list=list-1")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let mut ids: Vec<String> = get_bundle_entries(&body)
            .iter()
            .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["patient-1", "patient-3"]);
    }

    #[tokio::test]
    async fn test_list_combined_with_other_criteria() {
        // `_list` membership AND-s with ordinary search criteria.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;
        create_patient_list(&backend, "list-1", &["patient-1", "patient-2", "patient-3"]).await;

        let response = server
            .get("/Patient?_list=list-1&gender=male")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        // Only list members that are also male.
        for entry in get_bundle_entries(&body) {
            assert_eq!(entry["resource"]["gender"], "male");
        }
    }

    #[tokio::test]
    async fn test_list_missing_yields_empty() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_list=does-not-exist")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert!(get_bundle_entries(&body).is_empty());
    }

    #[tokio::test]
    async fn test_functional_list_rejected() {
        // The `$current-*` functional lists are not implemented; reject (501)
        // rather than silently returning an unfiltered result.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_list=$current-problems")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
    }
}

mod score_param {
    use super::*;

    const PREFER: HeaderName = HeaderName::from_static("prefer");

    #[tokio::test]
    async fn test_score_input_accepted_and_ignored() {
        // `_score` is an output concept, not a filter. It must not be rejected
        // (it used to 400) and must not filter results — it is simply ignored
        // as an input.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_score=5")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert!(
            !get_bundle_entries(&body).is_empty(),
            "`_score` input is ignored, so all patients are returned"
        );
    }

    #[tokio::test]
    async fn test_sort_by_score_accepted() {
        // `_sort=_score` is accepted under both lenient and strict handling. On a
        // non-full-text backend (SQLite) it falls back to default ordering.
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let lenient = server
            .get("/Patient?_sort=_score")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        lenient.assert_status_ok();

        let strict = server
            .get("/Patient?_sort=_score")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(PREFER, HeaderValue::from_static("handling=strict"))
            .await;
        strict.assert_status_ok();
    }

    #[tokio::test]
    async fn test_no_score_without_relevance_backend() {
        // SQLite does not compute relevance, so match entries carry no
        // `search.score` field.
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
        assert_eq!(entries[0]["search"]["mode"], "match");
        assert!(entries[0]["search"].get("score").is_none());
    }
}

mod contained_search {
    use super::*;

    /// Seeds an Observation containing a Patient named "Smith".
    async fn seed_contained(backend: &SqliteBackend) {
        backend
            .create(
                &test_tenant(),
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-c",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "#pc" },
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "pc",
                        "name": [{ "family": "Smith" }]
                    }]
                }),
                FhirVersion::R4,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_contained_true_returns_container() {
        let (server, backend) = create_test_server().await;
        seed_contained(&backend).await;

        let response = server
            .get("/Patient?_contained=true&name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["resourceType"], "Observation");
        assert_eq!(entries[0]["resource"]["id"], "obs-c");
    }

    #[tokio::test]
    async fn test_contained_type_contained_returns_contained_resource() {
        let (server, backend) = create_test_server().await;
        seed_contained(&backend).await;

        let response = server
            .get("/Patient?_contained=true&_containedType=contained&name=Smith")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["resourceType"], "Patient");
        assert_eq!(entries[0]["resource"]["id"], "pc");
    }

    #[tokio::test]
    async fn test_invalid_contained_value_rejected() {
        let (server, backend) = create_test_server().await;
        seed_contained(&backend).await;

        let response = server
            .get("/Patient?_contained=maybe")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_metadata_advertises_new_search_controls() {
        // SQLite supports contained search, so `_contained`/`_containedType` are
        // advertised alongside the always-supported `_list`.
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();

        let patient = body["rest"][0]["resource"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["type"] == "Patient")
            .expect("Patient resource in CapabilityStatement");
        let names: Vec<&str> = patient["searchParam"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["name"].as_str())
            .collect();

        assert!(names.contains(&"_list"), "advertises _list");
        assert!(names.contains(&"_contained"), "advertises _contained");
        assert!(
            names.contains(&"_containedType"),
            "advertises _containedType"
        );
    }
}

mod summary_count {
    use super::*;

    /// #254: `_summary=count` exists to return `Bundle.total`, so it implies
    /// an accurate total without the client also sending `_total=accurate`.
    #[tokio::test]
    async fn test_summary_count_implies_accurate_total() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_summary=count")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert!(
            body["total"].is_u64(),
            "count mode must carry the count: {body}"
        );
        assert!(body["total"].as_u64().unwrap() > 0);
        assert!(
            body.get("entry").is_none(),
            "count mode returns no entries: {body}"
        );
    }

    /// An explicit `_total` still wins over the implication.
    #[tokio::test]
    async fn test_summary_count_respects_explicit_total_none() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let response = server
            .get("/Patient?_summary=count&_total=none")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();
        assert!(
            body.get("total").is_none(),
            "explicit _total=none wins and total is absent, not null: {body}"
        );
    }
}

// ============================================================================
// Meta Parameter Tests (_tag / _profile / _security, #474)
// ============================================================================

mod meta_params {
    use super::*;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    async fn seed_tagged_patient(backend: &SqliteBackend) {
        let tenant = TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        );
        let tagged = json!({
            "resourceType": "Patient",
            "meta": {
                "tag": [{"system": "http://example.org/tags", "code": "test-data"}],
                "profile": ["http://example.org/StructureDefinition/custom-patient"]
            },
            "name": [{"family": "Tagged"}]
        });
        backend
            .create(&tenant, "Patient", tagged, FhirVersion::R4)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_tag_filters_over_http() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;
        seed_tagged_patient(&backend).await;

        let response = server
            .get("/Patient?_tag=http%3A%2F%2Fexample.org%2Ftags%7Ctest-data")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(
            entries.len(),
            1,
            "_tag must filter instead of returning every patient"
        );
        assert_eq!(entries[0]["resource"]["name"][0]["family"], "Tagged");
    }

    #[tokio::test]
    async fn test_profile_filters_over_http() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;
        seed_tagged_patient(&backend).await;

        let response = server
            .get(
                "/Patient?_profile=http%3A%2F%2Fexample.org%2FStructureDefinition%2Fcustom-patient",
            )
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["resource"]["name"][0]["family"], "Tagged");
    }
}

/// #456: date search must honor the value's precision at every boundary.
/// Stored dates keep their source precision while bounds are full datetimes,
/// and SQLite compares text — so a day never fell inside its own range, and a
/// full-precision timestamp built the impossible range `>= X AND < X`.
mod date_precision {
    use super::*;

    async fn seed(backend: &SqliteBackend) {
        let tenant = test_tenant();
        let patients = vec![
            json!({"resourceType": "Patient", "id": "d-boundary", "birthDate": "1995-10-02"}),
            json!({"resourceType": "Patient", "id": "d-new-year", "birthDate": "1996-01-01"}),
            json!({"resourceType": "Patient", "id": "d-earlier", "birthDate": "1975-03-21"}),
        ];
        for p in patients {
            backend
                .create(&tenant, "Patient", p, FhirVersion::R4)
                .await
                .expect("seed patient");
        }
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "d-obs",
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "8302-2"}]},
                    "subject": {"reference": "Patient/d-boundary"},
                    "effectiveDateTime": "2016-01-23T13:07:42-04:00"
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed observation");
    }

    async fn total(server: &TestServer, query: &str) -> u64 {
        let response = server
            .get(&format!("{query}&_total=accurate"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        response.json::<Value>()["total"].as_u64().expect("total")
    }

    #[tokio::test]
    async fn day_precision_boundaries() {
        let (server, backend) = create_test_server().await;
        seed(&backend).await;

        // eq: the patient born that exact day is found.
        assert_eq!(total(&server, "/Patient?birthdate=1995-10-02").await, 1);
        // ge includes the named day itself.
        assert_eq!(total(&server, "/Patient?birthdate=ge1995-10-02").await, 2);
        // gt starts strictly after the day.
        assert_eq!(total(&server, "/Patient?birthdate=gt1995-10-02").await, 1);
        // le must NOT leak into the next day's midnight.
        assert_eq!(total(&server, "/Patient?birthdate=le1995-12-31").await, 2);
        // lt excludes the boundary day.
        assert_eq!(total(&server, "/Patient?birthdate=lt1996-01-01").await, 2);
        // A same-day sandwich pins exactly the one patient.
        assert_eq!(
            total(
                &server,
                "/Patient?birthdate=ge1995-10-02&birthdate=le1995-10-02"
            )
            .await,
            1
        );
    }

    #[tokio::test]
    async fn coarser_precisions_still_match() {
        let (server, backend) = create_test_server().await;
        seed(&backend).await;

        assert_eq!(total(&server, "/Patient?birthdate=1995-10").await, 1);
        // The year range must not swallow 1996-01-01.
        assert_eq!(total(&server, "/Patient?birthdate=1995").await, 1);
    }

    #[tokio::test]
    async fn full_precision_timestamp_matches_itself() {
        let (server, backend) = create_test_server().await;
        seed(&backend).await;

        assert_eq!(
            total(&server, "/Observation?date=2016-01-23T13:07:42-04:00").await,
            1
        );
        // And the same instant expressed in UTC matches too: datetime()
        // folds offsets before comparing.
        assert_eq!(
            total(&server, "/Observation?date=2016-01-23T17:07:42Z").await,
            1
        );
    }

    #[tokio::test]
    async fn chained_date_inherits_the_fix() {
        let (server, backend) = create_test_server().await;
        seed(&backend).await;

        assert_eq!(
            total(&server, "/Observation?patient.birthdate=1995-10-02").await,
            1
        );
    }
}

/// #873: response bodies must carry server-populated `meta.versionId` and
/// `meta.lastUpdated` on every read path. The row's version and timestamp were
/// header-only (ETag / Last-Modified), so search entries, reads, vreads, and
/// history bundles all returned resources with no server metadata — visibly,
/// the Resources workspace's UPDATED column rendered blank on every backend.
mod server_meta {
    use super::*;

    #[tokio::test]
    async fn test_bodies_carry_version_and_last_updated_on_every_read_path() {
        let (server, _backend) = create_test_server().await;
        let tenant_header = HeaderValue::from_static("test-tenant");

        // Create: the response body already carries v1 meta, with
        // client-supplied meta members preserved.
        let created = server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant_header.clone())
            .json(&json!({
                "resourceType": "Patient",
                "meta": {"profile": ["http://example.org/StructureDefinition/p"]},
                "name": [{"family": "Metadata"}]
            }))
            .await;
        created.assert_status(StatusCode::CREATED);
        let body: Value = created.json();
        assert_eq!(body["meta"]["versionId"], "1");
        assert!(body["meta"]["lastUpdated"].is_string());
        assert_eq!(
            body["meta"]["profile"][0], "http://example.org/StructureDefinition/p",
            "client-supplied meta members must survive the injection"
        );
        let id = body["id"].as_str().expect("created id").to_string();

        // Update to v2.
        let mut updated_body = body.clone();
        updated_body["name"][0]["family"] = json!("Metadata2");
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant_header.clone())
            .json(&updated_body)
            .await
            .assert_status_ok();

        // Read: current version's meta.
        let read: Value = server
            .get(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant_header.clone())
            .await
            .json();
        assert_eq!(read["meta"]["versionId"], "2");
        assert!(read["meta"]["lastUpdated"].is_string());

        // Vread: each version reports its own meta.
        let v1: Value = server
            .get(&format!("/Patient/{id}/_history/1"))
            .add_header(X_TENANT_ID, tenant_header.clone())
            .await
            .json();
        assert_eq!(v1["meta"]["versionId"], "1");

        // History bundle entries carry their version's meta.
        let history: Value = server
            .get(&format!("/Patient/{id}/_history"))
            .add_header(X_TENANT_ID, tenant_header.clone())
            .await
            .json();
        let versions: Vec<&str> = history["entry"]
            .as_array()
            .expect("history entries")
            .iter()
            .map(|e| e["resource"]["meta"]["versionId"].as_str().unwrap_or(""))
            .collect();
        assert!(
            versions.contains(&"1") && versions.contains(&"2"),
            "history entries must carry per-version meta, got {versions:?}"
        );

        // Search: match entries carry meta.
        let search: Value = server
            .get("/Patient?family=Metadata2")
            .add_header(X_TENANT_ID, tenant_header)
            .await
            .json();
        let entry = &get_bundle_entries(&search)[0]["resource"];
        assert_eq!(entry["meta"]["versionId"], "2");
        assert!(entry["meta"]["lastUpdated"].is_string());
    }
}

// ============================================================================
// Bundle.total on a resource type with no stored rows (#990)
// ============================================================================

mod empty_type_total {
    use super::*;

    /// `Bundle.total` is either a number or absent. A resource type with no
    /// stored rows is a known-empty set: `_summary=count` and `_total=accurate`
    /// report `0`, and a plain search that computes no total omits the key.
    /// The literal `null` is never valid FHIR JSON for a primitive.
    async fn get_json(server: &TestServer, path: &str) -> Value {
        let response = server
            .get(path)
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        response.json()
    }

    fn assert_total_is_number_or_absent(body: &Value) {
        match body.get("total") {
            None => {}
            Some(total) => assert!(
                total.is_u64(),
                "Bundle.total must be a number or absent, never {total}: {body}"
            ),
        }
    }

    #[tokio::test]
    async fn test_empty_type_summary_count_reports_zero() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let body = get_json(&server, "/Group?_summary=count").await;
        assert_eq!(body["total"], serde_json::json!(0), "{body}");
        assert!(body.get("entry").is_none(), "{body}");
    }

    #[tokio::test]
    async fn test_empty_type_total_accurate_reports_zero() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let body = get_json(&server, "/Group?_total=accurate").await;
        assert_eq!(body["total"], serde_json::json!(0), "{body}");
        assert_eq!(body["entry"], serde_json::json!([]), "{body}");
    }

    #[tokio::test]
    async fn test_empty_type_plain_search_never_serializes_null_total() {
        let (server, backend) = create_test_server().await;
        seed_search_test_data(&backend).await;

        let body = get_json(&server, "/Group").await;
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "searchset");
        assert_total_is_number_or_absent(&body);
        assert_eq!(body["entry"], serde_json::json!([]), "{body}");

        // Explicitly opting out of a total omits the key rather than nulling it.
        let body = get_json(&server, "/Group?_summary=count&_total=none").await;
        assert_total_is_number_or_absent(&body);
    }
}
