//! End-to-end coverage for #787's second failure: the US Core `asserted-date`
//! search parameter must actually filter, using the exact resource shapes
//! Inferno's US Core client tests load.
//!
//! `crates/rest/tests/transaction_bundle_search_parameter.rs` already pins that
//! a `SearchParameter` registered in a prior transaction Bundle takes effect
//! immediately, but it does so with a `string` parameter over a plain element
//! path (`Patient.name.where(use = 'nickname').given`). Nothing in the repo
//! exercised the shape that actually failed in CI:
//!
//! * a `date`-typed custom parameter,
//! * whose expression walks `extension.where(url = ...).value` — a polymorphic
//!   `value[x]` that has to resolve to `valueDateTime` during indexing,
//! * queried with a comparison prefix (`gt`),
//! * against a sibling resource that carries no such extension at all.
//!
//! That last point is the #787 symptom: HFS returned
//! `Condition/us-core-client-tests-condition-encounter-diagnosis`, which has no
//! `assertedDate` extension, so Inferno reported `Expected: gt2016-08-09...` /
//! `Found:` (empty). An ignored parameter and a correctly applied one are
//! indistinguishable unless the negative resource is present, so both
//! Conditions are loaded here and the unfiltered count is asserted first.
//!
//! Resources are copied verbatim (minus narrative) from the files
//! `crates/hfs/tests/inferno/install.sh` POSTs into the server under test:
//! `uscore_bundle_asserted-date.json` and
//! `uscore_bundle_patient_client_test.json`. As in `install.sh` — and for the
//! reason spelled out in `transaction_bundle_search_parameter.rs` — the
//! parameter and the resources it indexes arrive in two *separate* transaction
//! Bundles.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const TENANT: HeaderValue = HeaderValue::from_static("test-tenant");

const PATIENT_ID: &str = "us-core-client-tests-patient";
/// The Condition that carries `condition-assertedDate` (`2016-08-10`).
const ASSERTED_CONDITION_ID: &str = "us-core-client-tests-condition-problems-health-concerns";
/// The Condition with no `assertedDate` extension — the one #787 over-returned.
const UNASSERTED_CONDITION_ID: &str = "us-core-client-tests-condition-encounter-diagnosis";

/// `SearchParameter/us-core-condition-asserted-date`, verbatim from
/// `crates/hfs/tests/inferno/uscore_bundle_asserted-date.json` with only the
/// generated `text` narrative elided. The primitive-extension siblings
/// (`_multipleOr`, `_multipleAnd`, `_comparator`), the resource-level
/// `extension`, `xpathUsage` and `comparator` are kept deliberately: the point
/// is that the real published US Core resource parses and registers, not a
/// hand-minimised stand-in.
fn asserted_date_search_parameter() -> Value {
    json!({
        "resourceType": "SearchParameter",
        "id": "us-core-condition-asserted-date",
        "extension": [{
            "url": "http://hl7.org/fhir/StructureDefinition/structuredefinition-wg",
            "valueCode": "cgp"
        }],
        "url": "http://hl7.org/fhir/us/core/SearchParameter/us-core-condition-asserted-date",
        "version": "7.0.0",
        "name": "USCoreConditionAssertedDate",
        "status": "active",
        "date": "2023-04-13",
        "publisher": "HL7 International / Cross-Group Projects",
        "contact": [{
            "name": "HL7 International / Cross-Group Projects",
            "telecom": [
                { "system": "url", "value": "http://www.hl7.org/Special/committees/cgp" },
                { "system": "email", "value": "cgp@lists.HL7.org" }
            ]
        }],
        "description": "Returns conditions with an [assertedDate extension](http://hl7.org/fhir/StructureDefinition/condition-assertedDate) matching the specified date (dateTime).",
        "jurisdiction": [{
            "coding": [{ "system": "urn:iso:std:iso:3166", "code": "US" }]
        }],
        "code": "asserted-date",
        "base": ["Condition"],
        "type": "date",
        "expression": "Condition.extension.where(url = 'http://hl7.org/fhir/StructureDefinition/condition-assertedDate').value",
        "xpathUsage": "normal",
        "multipleOr": true,
        "_multipleOr": {
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-expectation",
                "valueCode": "MAY"
            }]
        },
        "multipleAnd": true,
        "_multipleAnd": {
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-expectation",
                "valueCode": "SHOULD"
            }]
        },
        "comparator": ["eq", "ne", "gt", "ge", "lt", "le", "sa", "eb", "ap"]
    })
}

/// Bundle 1 — what `uscore_bundle_asserted-date.json` posts: the parameter, as
/// a `POST SearchParameter` entry of a transaction Bundle.
///
/// The companion `StructureDefinition/condition-assertedDate` entry of that
/// file is omitted: HFS indexes from the parameter's FHIRPath expression, so
/// the extension's definition plays no part in this search path.
fn register_asserted_date_bundle() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "resource": asserted_date_search_parameter(),
            "request": { "method": "POST", "url": "SearchParameter" }
        }]
    })
}

/// Bundle 2 — the two Conditions from `uscore_bundle_patient_client_test.json`,
/// plus the `Patient` they both reference so `Condition?patient=` has a real
/// target. Same `PUT <Type>/<id>` entries the Inferno bundle uses.
fn load_conditions_bundle() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": PATIENT_ID,
                    "name": [{ "family": "Shaw", "given": ["Amy", "V."] }],
                    "gender": "female",
                    "birthDate": "1987-02-20"
                },
                "request": { "method": "PUT", "url": format!("Patient/{PATIENT_ID}") }
            },
            {
                // Has the extension: valueDateTime 2016-08-10, so it is the one
                // and only match for `asserted-date=gt2016-08-09`.
                "resource": {
                    "resourceType": "Condition",
                    "id": ASSERTED_CONDITION_ID,
                    "meta": {
                        "profile": ["http://hl7.org/fhir/us/core/StructureDefinition/us-core-condition-problems-health-concerns"]
                    },
                    "extension": [{
                        "url": "http://hl7.org/fhir/StructureDefinition/condition-assertedDate",
                        "valueDateTime": "2016-08-10"
                    }],
                    "identifier": [{
                        "system": "https://github.com/inferno-framework/us-core-test-kit",
                        "value": "b9fbaa22-6097-11ed-9b6a-0242ac120002"
                    }],
                    "clinicalStatus": {
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                            "code": "resolved",
                            "display": "Resolved"
                        }],
                        "text": "Resolved"
                    },
                    "verificationStatus": {
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-ver-status",
                            "code": "confirmed",
                            "display": "Confirmed"
                        }],
                        "text": "Confirmed"
                    },
                    "category": [{
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-category",
                            "code": "problem-list-item",
                            "display": "Problem List Item"
                        }],
                        "text": "Problem List Item"
                    }],
                    "code": {
                        "coding": [{
                            "system": "http://snomed.info/sct",
                            "code": "445281000124101",
                            "display": "Nutrition impaired due to limited access to healthful foods (finding)"
                        }],
                        "text": "Nutrition impaired due to limited access to healthful foods (finding)"
                    },
                    "subject": { "reference": format!("Patient/{PATIENT_ID}") },
                    "abatementDateTime": "2016-08-10T07:15:07-08:00",
                    "recordedDate": "2016-08-10T07:15:07-08:00"
                },
                "request": { "method": "PUT", "url": format!("Condition/{ASSERTED_CONDITION_ID}") }
            },
            {
                // No `extension` at all. #787's symptom was this resource coming
                // back from an `asserted-date` search.
                "resource": {
                    "resourceType": "Condition",
                    "id": UNASSERTED_CONDITION_ID,
                    "meta": {
                        "profile": ["http://hl7.org/fhir/us/core/StructureDefinition/us-core-condition"]
                    },
                    "clinicalStatus": {
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                            "code": "resolved"
                        }]
                    },
                    "verificationStatus": {
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-ver-status",
                            "code": "confirmed"
                        }]
                    },
                    "category": [{
                        "coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/condition-category",
                            "code": "encounter-diagnosis",
                            "display": "Encounter Diagnosis"
                        }]
                    }],
                    "code": {
                        "coding": [{
                            "system": "http://snomed.info/sct",
                            "code": "233678006",
                            "display": "Childhood asthma"
                        }],
                        "text": "Childhood asthma"
                    },
                    "subject": { "reference": format!("Patient/{PATIENT_ID}") },
                    "onsetDateTime": "1944-09-11T19:33:18-04:00",
                    "abatementDateTime": "1959-12-22T18:33:18-05:00",
                    "recordedDate": "1944-09-11T19:33:18-04:00"
                },
                "request": { "method": "PUT", "url": format!("Condition/{UNASSERTED_CONDITION_ID}") }
            }
        ]
    })
}

/// Ids of the `Condition` entries of a searchset Bundle, in response order.
fn condition_ids(body: &Value) -> Vec<String> {
    body["entry"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|entry| entry["resource"]["resourceType"] == "Condition")
        .filter_map(|entry| entry["resource"]["id"].as_str().map(str::to_owned))
        .collect()
}

async fn search(server: &TestServer, query: &str) -> Value {
    let response = server.get(query).add_header(X_TENANT_ID, TENANT).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "search {query} should succeed, got {}",
        response.text()
    );
    response.json()
}

/// Posts both Bundles, then walks the assertions #787 needed.
async fn assert_asserted_date_filters(server: &TestServer) {
    let register = server
        .post("/")
        .add_header(X_TENANT_ID, TENANT)
        .json(&register_asserted_date_bundle())
        .await;
    register.assert_status_ok();

    let load = server
        .post("/")
        .add_header(X_TENANT_ID, TENANT)
        .json(&load_conditions_bundle())
        .await;
    load.assert_status_ok();

    // Control: without `asserted-date`, both Conditions are in scope. Without
    // this the filtering assertions below could pass on an empty index.
    let mut unfiltered =
        condition_ids(&search(server, &format!("/Condition?patient={PATIENT_ID}")).await);
    unfiltered.sort();
    assert_eq!(
        unfiltered,
        vec![
            UNASSERTED_CONDITION_ID.to_string(),
            ASSERTED_CONDITION_ID.to_string()
        ],
        "both Inferno Conditions must be searchable before `asserted-date` narrows them"
    );

    // The #787 query. `+00:00` is percent-encoded: an unescaped `+` in a query
    // string decodes to a space and would never parse as a date.
    let filtered_query =
        format!("/Condition?patient={PATIENT_ID}&asserted-date=gt2016-08-09T00:00:00%2B00:00");
    let filtered = condition_ids(&search(server, &filtered_query).await);
    assert_eq!(
        filtered,
        vec![ASSERTED_CONDITION_ID.to_string()],
        "`asserted-date=gt2016-08-09` must return only the Condition carrying the \
         condition-assertedDate extension; returning {UNASSERTED_CONDITION_ID} (which has \
         no extension at all) is #787's symptom of the parameter being dropped"
    );

    // Under strict handling an unrecognised parameter is a 400, so a 200 here
    // proves the parameter was applied rather than silently ignored.
    let strict = server
        .get(&filtered_query)
        .add_header(X_TENANT_ID, TENANT)
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("handling=strict"),
        )
        .await;
    assert_eq!(
        strict.status_code(),
        StatusCode::OK,
        "strict handling must recognise the registered `asserted-date` parameter, got {}",
        strict.text()
    );
    assert_eq!(
        condition_ids(&strict.json()),
        vec![ASSERTED_CONDITION_ID.to_string()],
        "strict handling must return the same single match as lenient"
    );

    // The comparison is real in both directions: the extension's 2016-08-10 is
    // not before 2016-08-09, so this filters everything out.
    let empty = condition_ids(&search(server, "/Condition?asserted-date=lt2016-08-09").await);
    assert!(
        empty.is_empty(),
        "`asserted-date=lt2016-08-09` must exclude the 2016-08-10 assertion too, got {empty:?}"
    );

    // Indexed on its own, not merely as a co-filter alongside `patient`.
    let standalone = condition_ids(&search(server, "/Condition?asserted-date=eq2016-08-10").await);
    assert_eq!(
        standalone,
        vec![ASSERTED_CONDITION_ID.to_string()],
        "the extension's valueDateTime must be indexed under the custom date parameter"
    );
}

mod sqlite_tests {
    use super::*;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_rest::ServerConfig;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    async fn server() -> TestServer {
        let backend = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir()),
                ..Default::default()
            },
        )
        .expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::new(backend), config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    #[tokio::test]
    async fn sqlite_us_core_asserted_date_filters_by_extension_value() {
        let server = server().await;
        assert_asserted_date_filters(&server).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres_tests {
    use super::*;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_rest::ServerConfig;
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn server() -> TestServer {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        // Pin the major version, for the reason documented in
        // `transaction_bundle_search_parameter.rs`: the module default is the
        // EOL postgres:11, which rejects the `plan_cache_mode` startup option
        // the backend sends.
        let container = Postgres::default()
            .with_tag("16-alpine")
            .with_label("github.run_id", &run_id)
            .start()
            .await
            .expect("failed to start PostgreSQL container");

        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to get host port");
        let host = container
            .get_host()
            .await
            .expect("failed to get host")
            .to_string();

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = PostgresConfig {
            host,
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        };

        let backend = PostgresBackend::new(config)
            .await
            .expect("create PostgreSQL backend");
        backend.init_schema().await.expect("init schema");
        // Leaked for the lifetime of the test process so the container outlives
        // the pool; each test owns its own container.
        std::mem::forget(container);

        let server_config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::new(backend), server_config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    #[tokio::test]
    async fn postgres_us_core_asserted_date_filters_by_extension_value() {
        let server = server().await;
        assert_asserted_date_filters(&server).await;
    }
}
