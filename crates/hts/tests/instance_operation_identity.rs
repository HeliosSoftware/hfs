//! Instance operations must preserve older and unversioned public identities.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

const CS: &str = "http://example.org/instance-selection/cs";
const MEMBERS: &str = "http://example.org/instance-selection/members";
const VS: &str = "http://example.org/instance-selection/vs";

// The same HTTP assertions exercise both database implementations.
enum InstanceApp {
    #[cfg(feature = "sqlite")]
    Sqlite(common::TestApp),
    #[cfg(feature = "postgres")]
    Postgres(common::TestAppPg),
}

impl InstanceApp {
    async fn get(&self, path: &str) -> (StatusCode, Value) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(app) => app.get_fhir(path).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(app) => app.get_fhir(path).await,
        }
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(app) => app.post_fhir(path, body.to_string()).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(app) => app.post_fhir(path, body.to_string()).await,
        }
    }

    async fn import(&self, resources: Vec<Value>) {
        let bundle = json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": resources
                .into_iter()
                .map(|resource| json!({"resource": resource}))
                .collect::<Vec<_>>()
        });
        let (status, body) = self.post("/import", bundle).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

fn query(path: &str, pairs: &[(&str, &str)]) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish();
    format!("{path}?{query}")
}

fn parameter<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["parameter"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)
}

fn assert_lookup(body: &Value, display: &str, version: Option<&str>) {
    assert_eq!(
        parameter(body, "display").and_then(|p| p["valueString"].as_str()),
        Some(display),
        "{body}"
    );
    assert_eq!(
        parameter(body, "version").and_then(|p| p["valueString"].as_str()),
        version,
        "{body}"
    );
}

fn assert_valid(body: &Value, expected: bool) {
    assert_eq!(
        parameter(body, "result").and_then(|p| p["valueBoolean"].as_bool()),
        Some(expected),
        "{body}"
    );
}

fn code_system(
    id: &str,
    version: Option<&str>,
    display: &str,
    own_code: &str,
    language: &str,
) -> Value {
    let mut resource = json!({
        "resourceType": "CodeSystem", "id": id, "url": CS, "name": id,
        "status": "active", "content": "complete", "language": language,
        "concept": [{"code": "shared", "display": display}, {"code": own_code, "display": own_code}]
    });
    if let Some(version) = version {
        resource["version"] = json!(version);
    }
    resource
}

fn value_set(id: &str, url: &str, version: Option<&str>, code: &str) -> Value {
    let mut resource = json!({
        "resourceType": "ValueSet", "id": id, "url": url, "status": "active",
        "compose": {"include": [{"system": MEMBERS, "concept": [{"code": code}]}]}
    });
    if let Some(version) = version {
        resource["version"] = json!(version);
    }
    resource
}

async fn exact_instance_contract(app: InstanceApp) {
    app.import(vec![
        code_system(
            "lookup-unversioned",
            None,
            "Unversioned",
            "only-unversioned",
            "en",
        ),
        code_system("lookup-old", Some("1.0.0"), "Old", "only-old", "en"),
        code_system(
            "lookup-latest",
            Some("2.0.0"),
            "Latest",
            "only-latest",
            "de",
        ),
        json!({
            "resourceType": "CodeSystem", "id": "membership-system", "url": MEMBERS,
            "status": "active", "content": "complete",
            "concept": [
                {"code": "old", "display": "Old member"},
                {"code": "latest", "display": "Latest member"},
                {"code": "unversioned", "display": "Unversioned member"}
            ]
        }),
        value_set("validate-unversioned", VS, None, "unversioned"),
        value_set("validate-old", VS, Some("1.0.0"), "old"),
        value_set("validate-latest", VS, Some("2.0.0"), "latest"),
    ])
    .await;

    // Prime canonical caches with a newer sibling before calling either instance.
    let (status, body) = app
        .get(&query(
            "/CodeSystem/$lookup",
            &[("system", CS), ("code", "shared")],
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_lookup(&body, "Latest", Some("2.0.0"));
    for (id, display, version) in [
        ("lookup-old", "Old", Some("1.0.0")),
        ("lookup-unversioned", "Unversioned", None),
    ] {
        for _ in 0..2 {
            let path = format!("/CodeSystem/{id}/$lookup");
            let (status, body) = app.get(&query(&path, &[("code", "shared")])).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_lookup(&body, display, version);
            // Primary language must come from the selected row, not latest.
            assert!(
                body["parameter"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["name"] == "designation"
                        && p["part"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|part| part["name"] == "language" && part["valueCode"] == "en")),
                "{body}"
            );
            let (status, body) = app
                .post(
                    &path,
                    json!({"resourceType": "Parameters", "parameter": [
                        {"name": "code", "valueCode": "shared"},
                        {"name": "system", "valueUri": "http://example.org/wrong-system"},
                        {"name": "version", "valueString": "2.0.0"}
                    ]}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_lookup(&body, display, version);
            let (status, body) = app
                .get(&query(
                    &path,
                    &[
                        ("code", "shared"),
                        ("system", "http://example.org/wrong-system"),
                        ("version", "2.0.0"),
                    ],
                ))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_lookup(&body, display, version);
        }
    }
    // Negative handler/backend cache entries must not leak between identities.
    let (status, _) = app
        .get(&query(
            "/CodeSystem/$lookup",
            &[("system", CS), ("code", "only-unversioned")],
        ))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = app
        .get("/CodeSystem/lookup-unversioned/$lookup?code=only-unversioned")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = app
        .get("/CodeSystem/lookup-unversioned/$lookup?code=only-latest")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = app
        .get(&query(
            "/CodeSystem/$lookup",
            &[("system", CS), ("code", "only-latest")],
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for code in ["unversioned", "latest"] {
        let (status, body) = app
            .get(&query(
                "/ValueSet/$validate-code",
                &[("url", VS), ("system", MEMBERS), ("code", code)],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, code == "latest");
    }
    for (id, member, version) in [
        ("validate-old", "old", Some("1.0.0")),
        ("validate-unversioned", "unversioned", None),
    ] {
        let path = format!("/ValueSet/{id}/$validate-code");
        for _ in 0..2 {
            let (status, body) = app
                .get(&query(&path, &[("system", MEMBERS), ("code", member)]))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, true);
            let (status, body) = app
                .get(&query(&path, &[("system", MEMBERS), ("code", "latest")]))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, false);
            let text = body.to_string();
            assert!(
                !text.contains(&format!("{VS}|2.0.0")),
                "selected version changed in diagnostics: {body}"
            );
            if let Some(version) = version {
                assert!(text.contains(&format!("{VS}|{version}")), "{body}");
            }
        }
        // Exercise every POST coding form, with conflicting resource selectors.
        for coding in [
            vec![
                json!({"name": "system", "valueUri": MEMBERS}),
                json!({"name": "code", "valueCode": member}),
            ],
            vec![json!({"name": "coding", "valueCoding": {"system": MEMBERS, "code": member}})],
            vec![json!({
                "name": "codeableConcept",
                "valueCodeableConcept": {"coding": [{"system": MEMBERS, "code": member}]}
            })],
        ] {
            let mut params = coding;
            params.extend([
                json!({"name": "url", "valueUri": "http://example.org/wrong-valueset"}),
                json!({"name": "valueSetVersion", "valueString": "2.0.0"}),
                json!({
                    "name": "valueSet",
                    "resource": value_set("wrong", VS, Some("2.0.0"), "latest")
                }),
                json!({
                    "name": "default-valueset-version",
                    "valueCanonical": format!("{VS}|2.0.0")
                }),
            ]);
            let (status, body) = app
                .post(
                    &path,
                    json!({"resourceType": "Parameters", "parameter": params}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, true);
        }
        let (status, body) = app
            .get(&query(
                &path,
                &[
                    ("system", MEMBERS),
                    ("code", member),
                    ("url", "http://example.org/wrong-valueset"),
                    ("valueSetVersion", "2.0.0"),
                ],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, true);
    }

    // Metadata checks must not pick the latest sibling's missing import or supplement.
    let metadata_url = format!("{VS}/metadata");
    let mut latest = value_set("metadata-latest", &metadata_url, Some("2.0.0"), "latest");
    latest["compose"] = json!({"include": [{"valueSet": ["http://example.org/missing-import"]}]});
    latest["extension"] = json!([{
        "url": "http://hl7.org/fhir/StructureDefinition/valueset-supplement",
        "valueCanonical": "http://example.org/missing-supplement"
    }]);
    app.import(vec![
        value_set("metadata-old", &metadata_url, Some("1.0.0"), "old"),
        value_set("metadata-unversioned", &metadata_url, None, "unversioned"),
        latest,
        value_set("stored-implicit-url", &format!("{CS}?fhir_vs"), None, "old"),
    ])
    .await;
    for (id, code) in [
        ("metadata-old", "old"),
        ("metadata-unversioned", "unversioned"),
        ("stored-implicit-url", "old"),
    ] {
        let (status, body) = app
            .get(&query(
                &format!("/ValueSet/{id}/$validate-code"),
                &[("system", MEMBERS), ("code", code)],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, true);
    }
    let (status, body) = app
        .get(&query(
            "/ValueSet/stored-implicit-url/$validate-code",
            &[("system", CS), ("code", "shared")],
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_valid(&body, false);
    for path in [
        "/CodeSystem/missing-instance/$lookup?code=shared",
        "/ValueSet/missing-instance/$validate-code?code=old",
    ] {
        assert_eq!(app.get(path).await.0, StatusCode::NOT_FOUND);
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_operations_preserve_exact_instance_identity() {
    exact_instance_contract(InstanceApp::Sqlite(common::TestApp::new())).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_operations_preserve_exact_instance_identity() {
    exact_instance_contract(InstanceApp::Postgres(
        common::TestAppPg::new_isolated().await,
    ))
    .await;
}
