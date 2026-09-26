//! Empty search values (#1380), end to end through the REST layer.
//!
//! `GET /Patient?family=Zzz,` used to return every Patient with a family name:
//! the empty alternative reached the backend as the value `""`, and a string
//! search is a prefix match. The rule now:
//!
//! - a value with an empty OR-alternative (`family=Zzz,`, `family=,Zzz`,
//!   `family=a,,b`, `family=,`) is a `400` naming the parameter, for every
//!   parameter type, modifier and entry path, whatever `Prefer: handling` says;
//! - a parameter with no value at all (`family=`) is ignored, as FHIR has it
//!   ("Empty parameters are not an error - they are just ignored by the
//!   server"), and is absent from the self link;
//! - criteria that guard a write never ignore one: a conditional reference
//!   with an empty value does not resolve.
//!
//! The persistence-level table lives in
//! `crates/persistence/tests/search/empty_value_suite.rs` and runs on all four
//! backends; this file pins the behaviour over HTTP on SQLite.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use serde_json::{Value, json};

use helios_rest::{ServerConfig, create_app_with_config};

/// An in-memory HFS with the spec search parameters loaded — the embedded
/// fallback set knows neither `family` nor `gender`, so nothing would index —
/// seeded with four Patients (`p-zzz`: family Zzz, female, identifier 111, a
/// general practitioner; `p-abc`: family "Abc, Jr", male; `p-comma`: family
/// `Comma, J`; `p-bare`: nothing), the Practitioner, and an Observation about
/// each of the first two.
async fn seeded_server() -> TestServer {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");
    let server =
        TestServer::new(create_app_with_config(backend, ServerConfig::for_testing())).unwrap();

    for resource in [
        json!({"resourceType": "Practitioner", "id": "doc", "name": [{"family": "Doc"}]}),
        json!({"resourceType": "Patient", "id": "p-zzz",
               "identifier": [{"system": "http://example.org/mrn", "value": "111"}],
               "name": [{"family": "Zzz"}], "gender": "female",
               "generalPractitioner": [{"reference": "Practitioner/doc"}]}),
        json!({"resourceType": "Patient", "id": "p-abc",
               "name": [{"family": "Abc, Jr"}], "gender": "male"}),
        json!({"resourceType": "Patient", "id": "p-comma", "name": [{"family": "Comma, J"}]}),
        json!({"resourceType": "Patient", "id": "p-bare"}),
        json!({"resourceType": "Observation", "id": "o-a", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "8480-6"}],
                        "text": "alpha"},
               "subject": {"reference": "Patient/p-zzz"},
               "valueQuantity": {"value": 5.4, "unit": "mg",
                                 "system": "http://unitsofmeasure.org", "code": "mg"}}),
        json!({"resourceType": "Observation", "id": "o-b", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
               "subject": {"reference": "Patient/p-abc"}}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server.put(&path).json(&resource).await;
        assert!(response.status_code().is_success(), "seeding {path}");
    }
    server
}

fn prefer(handling: &'static str) -> (HeaderName, HeaderValue) {
    (
        HeaderName::from_static("prefer"),
        HeaderValue::from_static(handling),
    )
}

/// The sorted ids of a searchset's matches.
fn match_ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["resource"]["resourceType"] != "OperationOutcome")
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn self_link(body: &Value) -> String {
    body["link"]
        .as_array()
        .and_then(|links| links.iter().find(|l| l["relation"] == "self"))
        .and_then(|l| l["url"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// A `400` whose OperationOutcome names `param` and says the value is empty.
fn assert_empty_value_outcome(response: &TestResponse, param: &str, context: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "{context}: {}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome", "{context}");
    assert_eq!(body["issue"][0]["severity"], "error", "{context}");
    let text = body["issue"][0]["diagnostics"]
        .as_str()
        .or_else(|| body["issue"][0]["details"]["text"].as_str())
        .unwrap_or_default();
    assert!(
        text.contains(&format!("'{param}'")) && text.contains("empty alternative"),
        "{context}: the outcome should name '{param}' and say what is wrong, got {body}"
    );
}

/// Runs `path?name=value` over GET and POST `_search`, under both handling
/// preferences.
async fn each_way(
    server: &TestServer,
    path: &str,
    name: &str,
    value: &str,
) -> Vec<(String, TestResponse)> {
    let mut out = Vec::new();
    for handling in ["handling=lenient", "handling=strict"] {
        let (header, header_value) = prefer(handling);
        out.push((
            format!("GET {path}?{name}={value} ({handling})"),
            server
                .get(path)
                .add_query_param(name, value)
                .add_header(header.clone(), header_value.clone())
                .await,
        ));
        out.push((
            format!("POST {path}/_search {name}={value} ({handling})"),
            server
                .post(&format!("{path}/_search"))
                .add_header(header, header_value)
                .form(&[(name, value)])
                .await,
        ));
    }
    out
}

#[tokio::test]
async fn an_empty_alternative_is_a_400_naming_the_parameter() {
    let server = seeded_server().await;

    // Positive controls: the parameters index, and an OR-list works.
    for (path, name, value, expected) in [
        ("/Patient", "family", "Zzz", &["p-zzz"][..]),
        ("/Patient", "family", "Zzz,Abc", &["p-abc", "p-zzz"]),
        ("/Patient", "gender", "female", &["p-zzz"]),
        (
            "/Patient",
            "general-practitioner",
            "Practitioner/doc",
            &["p-zzz"],
        ),
        ("/Observation", "subject.family", "Zzz", &["o-a"]),
        (
            "/Patient",
            "_has:Observation:subject:code",
            "8480-6",
            &["p-zzz"],
        ),
        (
            "/Observation",
            "code-value-quantity",
            "8480-6$5.4",
            &["o-a"],
        ),
    ] {
        for (context, response) in each_way(&server, path, name, value).await {
            assert_eq!(response.status_code(), StatusCode::OK, "{context}");
            assert_eq!(match_ids(&response.json()), expected, "{context}");
        }
    }

    for (path, name, reported) in [
        ("/Patient", "family", "family"),
        ("/Patient", "family:exact", "family:exact"),
        ("/Patient", "family:contains", "family:contains"),
        ("/Patient", "gender", "gender"),
        ("/Patient", "gender:not", "gender:not"),
        ("/Patient", "gender:text", "gender:text"),
        ("/Patient", "identifier", "identifier"),
        ("/Patient", "identifier:of-type", "identifier:of-type"),
        ("/Patient", "general-practitioner", "general-practitioner"),
        (
            "/Patient",
            "general-practitioner:identifier",
            "general-practitioner:identifier",
        ),
        ("/Patient", "_id", "_id"),
        ("/Patient", "_tag", "_tag"),
        ("/ValueSet", "url", "url"),
        ("/ValueSet", "url:below", "url:below"),
        ("/ValueSet", "url:contains", "url:contains"),
        ("/Observation", "subject.family", "subject.family"),
        (
            "/Observation",
            "subject:Patient.family",
            "subject:Patient.family",
        ),
        (
            "/Patient",
            "_has:Observation:subject:code",
            "_has:Observation:subject:code",
        ),
        (
            "/Patient",
            "_has:Observation:subject:code:text",
            "_has:Observation:subject:code:text",
        ),
        ("/Observation", "code-value-quantity", "code-value-quantity"),
    ] {
        for value in ["Zzz,", ",Zzz", "Zzz,,Abc", ",", ",,", "Zzz, "] {
            for (context, response) in each_way(&server, path, name, value).await {
                assert_empty_value_outcome(&response, reported, &context);
            }
        }
    }

    // An empty composite component.
    for (context, response) in
        each_way(&server, "/Observation", "code-value-quantity", "$5.4").await
    {
        assert_empty_value_outcome(&response, "code-value-quantity", &context);
    }
}

#[tokio::test]
async fn an_escaped_comma_is_data() {
    let server = seeded_server().await;
    for (value, expected) in [
        ("Comma\\,", &["p-comma"][..]),
        ("Comma\\, J", &["p-comma"]),
        ("Comma\\, J,Zzz", &["p-comma", "p-zzz"]),
    ] {
        for (context, response) in each_way(&server, "/Patient", "family", value).await {
            assert_eq!(response.status_code(), StatusCode::OK, "{context}");
            assert_eq!(match_ids(&response.json()), expected, "{context}");
        }
    }
}

#[tokio::test]
async fn a_parameter_with_no_value_is_ignored() {
    let server = seeded_server().await;
    let everyone = ["p-abc", "p-bare", "p-comma", "p-zzz"];

    for name in [
        "family",
        "family:exact",
        "family:contains",
        "gender",
        "gender:not",
        "identifier:of-type",
        "general-practitioner",
        "_id",
        "_tag",
        "birthdate",
        "_lastUpdated",
        "general-practitioner.family",
        "_has:Observation:subject:code",
        // Not a parameter at all: there is nothing to be strict about.
        "no-such-parameter",
    ] {
        for value in ["", " "] {
            for (context, response) in each_way(&server, "/Patient", name, value).await {
                assert_eq!(
                    response.status_code(),
                    StatusCode::OK,
                    "{context}: {}",
                    response.text()
                );
                let body: Value = response.json();
                assert_eq!(match_ids(&body), everyone, "{context}");
                assert!(
                    !self_link(&body).contains(name.split(':').next().unwrap()),
                    "{context}: the self link must not claim the parameter was applied: {}",
                    self_link(&body)
                );
            }
        }
    }

    // The other parameters of the request still apply.
    let response = server
        .get("/Patient")
        .add_query_param("family", "")
        .add_query_param("gender", "female")
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Value = response.json();
    assert_eq!(match_ids(&body), ["p-zzz"]);
    assert!(
        self_link(&body).contains("gender=female"),
        "{}",
        self_link(&body)
    );
    assert!(!self_link(&body).contains("family"), "{}", self_link(&body));

    // `:missing` takes a boolean; an absent one stays an error.
    let response = server
        .get("/Patient")
        .add_query_param("family:missing", "")
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compartment_searches_follow_the_same_rule() {
    let server = seeded_server().await;

    let response = server
        .get("/Patient/p-zzz/Observation")
        .add_query_param("code", "8480-6,")
        .await;
    assert_empty_value_outcome(&response, "code", "compartment code=8480-6,");
    let response = server
        .get("/Patient/p-zzz/Observation")
        .add_query_param("code", "")
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(match_ids(&response.json()), ["o-a"]);

    // All types of the compartment at once.
    let response = server
        .get("/Patient/p-zzz/*")
        .add_query_param("_id", "o-a,")
        .await;
    assert_empty_value_outcome(&response, "_id", "compartment/* _id=o-a,");
    let response = server
        .get("/Patient/p-zzz/*")
        .add_query_param("_id", "")
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(match_ids(&response.json()).contains(&"o-a".to_string()));
}

/// A search entry of a batch is a search; a conditional reference guards a
/// write, and an empty value in one must never be dropped — with a single
/// Practitioner stored, `Practitioner?family=` would otherwise resolve to it.
#[tokio::test]
async fn batch_entries_search_like_a_search_and_conditional_references_do_not() {
    let server = seeded_server().await;

    let response = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {"request": {"method": "GET", "url": "Patient?family=&gender=female"}},
            ],
        }))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(
        match_ids(&body["entry"][0]["resource"]),
        ["p-zzz"],
        "{body}"
    );

    let response = server
        .post("/")
        .json(&json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:6f0c1c1e-0c1c-4a57-9a35-3c1d5a1f0001",
                "request": {"method": "POST", "url": "Patient"},
                "resource": {
                    "resourceType": "Patient",
                    "generalPractitioner": [{"reference": "Practitioner?family="}],
                },
            }],
        }))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "{}",
        response.text()
    );
    assert!(
        response.text().contains("Practitioner?family="),
        "{}",
        response.text()
    );
}
