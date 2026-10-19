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

const MIME_VS: &str = "http://hl7.org/fhir/ValueSet/mimetypes";
const BCP13: &str = "urn:ietf:bcp:13";

fn mime_value_set(id: &str, version: Option<&str>, compose: Value) -> Value {
    let mut resource = json!({
        "resourceType": "ValueSet", "id": id, "url": MIME_VS,
        "status": "active", "date": "2020-01-01", "compose": compose
    });
    if let Some(version) = version {
        resource["version"] = json!(version);
    }
    resource
}

async fn stored_mime_instance_contract(app: InstanceApp) {
    // A genuinely unstored canonical MIME ValueSet retains syntax validation.
    for (code, expected) in [("text/cql", true), ("not-a-mime", false)] {
        let (status, body) = app
            .get(&query(
                "/ValueSet/$validate-code",
                &[("url", MIME_VS), ("system", BCP13), ("code", code)],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, expected);
    }

    let unrestricted = json!({"include": [{"system": BCP13}]});
    let annotated = json!({
        "id": "mime-compose", "inactive": false, "exclude": [],
        "extension": [{"url": "http://example.org/source-note", "valueString": "Core MIME types"}],
        "include": [{
            "id": "all-mime-types", "system": BCP13,
            "extension": [{"url": "http://example.org/source-note", "valueString": "Whole system"}]
        }]
    });
    let restricted = json!({"include": [{
        "system": BCP13, "concept": [{"code": "application/json"}]
    }]});
    let excluded = json!({
        "include": [{"system": BCP13}],
        "exclude": [{"system": BCP13, "concept": [{"code": "text/cql"}]}]
    });
    // No CodeSystem is imported: the unrestricted MIME definition must use
    // syntax validation, without treating restricted siblings as unrestricted.
    app.import(vec![
        mime_value_set("mime-unversioned", None, annotated),
        mime_value_set("mime-old", Some("1.0.0"), unrestricted.clone()),
        mime_value_set("mime-restricted", Some("2.0.0"), restricted),
        mime_value_set("mime-excluded", Some("3.0.0"), excluded.clone()),
    ])
    .await;

    // Canonical requests follow the stored definition they select, so they
    // agree with the instance requests below.
    for (version, code, expected) in [
        ("1.0.0", "text/cql", true),
        ("1.0.0", "not-a-mime", false),
        ("2.0.0", "text/cql", false),
        ("3.0.0", "text/cql", false),
    ] {
        let piped_url = format!("{MIME_VS}|{version}");
        for mut params in [
            vec![("url", MIME_VS), ("valueSetVersion", version)],
            vec![("url", piped_url.as_str())],
        ] {
            params.extend([("system", BCP13), ("code", code)]);
            let (status, body) = app.get(&query("/ValueSet/$validate-code", &params)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, expected);
        }
    }

    let (status, body) = app
        .get("/ValueSet/mime-unversioned/$validate-code?code=text%2Fcql")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_valid(&body, true);

    // Presentation metadata and a date that includes the selected definition
    // do not turn a valid MIME code into a missing member.
    let (status, body) = app
        .get(&query(
            "/ValueSet/mime-unversioned/$validate-code",
            &[
                ("system", BCP13),
                ("code", "text/cql"),
                ("display", "Clinical Quality Language"),
                ("date", "2021-01-01"),
            ],
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_valid(&body, true);
    let (status, body) = app
        .get(&query(
            "/ValueSet/mime-unversioned/$validate-code",
            &[
                ("system", BCP13),
                ("code", "text/cql"),
                ("date", "2019-01-01"),
            ],
        ))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // A stored canonical definition must not become a synthetic ValueSet when
    // its requested date or business version cannot be resolved.
    for (version, date) in [("1.0.0", Some("2019-01-01")), ("missing-version", None)] {
        let piped_url = format!("{MIME_VS}|{version}");
        for mut params in [
            vec![("url", MIME_VS), ("valueSetVersion", version)],
            vec![("url", piped_url.as_str())],
        ] {
            params.extend([("system", BCP13), ("code", "text/cql")]);
            if let Some(date) = date {
                params.push(("date", date));
            }
            let (status, body) = app.get(&query("/ValueSet/$validate-code", &params)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
            assert_eq!(body["resourceType"], "OperationOutcome", "{body}");
            assert!(
                body["issue"][0]["diagnostics"]
                    .as_str()
                    .unwrap()
                    .contains(&piped_url),
                "the selected canonical must remain in the resolver error: {body}"
            );
        }
    }

    // An explicit version overrides a piped version on canonical requests;
    // neither caller-supplied selector may override the instance identity.
    let piped_old = format!("{MIME_VS}|1.0.0");
    for (path, expected) in [
        ("/ValueSet/$validate-code", false),
        ("/ValueSet/mime-old/$validate-code", true),
    ] {
        let (status, body) = app
            .get(&query(
                path,
                &[
                    ("url", piped_old.as_str()),
                    ("valueSetVersion", "2.0.0"),
                    ("system", BCP13),
                    ("code", "text/cql"),
                ],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, expected);
    }
    let (status, body) = app
        .post(
            "/ValueSet/$validate-code",
            json!({"resourceType": "Parameters", "parameter": [
                {"name": "url", "valueUri": piped_old},
                {"name": "valueSetVersion", "valueString": "2.0.0"},
                {"name": "system", "valueUri": BCP13},
                {"name": "code", "valueCode": "text/cql"}
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_valid(&body, false);

    // Alternating exact identities must not share cached membership decisions.
    for _ in 0..2 {
        for (id, expected) in [
            ("mime-unversioned", true),
            ("mime-restricted", false),
            ("mime-old", true),
            ("mime-excluded", false),
        ] {
            let (status, body) = app
                .get(&query(
                    &format!("/ValueSet/{id}/$validate-code"),
                    &[
                        ("system", BCP13),
                        ("code", "text/cql"),
                        ("url", "http://example.org/wrong-valueset"),
                        ("valueSetVersion", "1.0.0"),
                    ],
                ))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, expected);
        }
    }
    for id in ["mime-unversioned", "mime-old"] {
        let (status, body) = app
            .get(&query(
                &format!("/ValueSet/{id}/$validate-code"),
                &[("system", BCP13), ("code", "not-a-mime")],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, false);
    }

    // Caller-supplied definitions and version pins cannot replace the instance,
    // in either direction: a restricted body cannot narrow the selected all-MIME
    // definition, and an all-MIME body cannot widen the selected restricted one.
    for (id, expected, conflicting_compose, conflicting_version) in [
        ("mime-old", true, &excluded, "3.0.0"),
        ("mime-restricted", false, &unrestricted, "1.0.0"),
    ] {
        for coding in [
            vec![
                json!({"name": "system", "valueUri": BCP13}),
                json!({"name": "code", "valueCode": "text/cql"}),
            ],
            vec![json!({
                "name": "coding", "valueCoding": {"system": BCP13, "code": "text/cql"}
            })],
            vec![json!({
                "name": "codeableConcept",
                "valueCodeableConcept": {"coding": [{"system": BCP13, "code": "text/cql"}]}
            })],
        ] {
            let mut params = coding;
            params.extend([
                json!({"name": "url", "valueUri": "http://example.org/wrong-valueset"}),
                json!({"name": "valueSetVersion", "valueString": conflicting_version}),
                json!({
                    "name": "valueSet", "resource": mime_value_set(
                        "caller-supplied", Some(conflicting_version), conflicting_compose.clone()
                    )
                }),
                json!({
                    "name": "default-valueset-version",
                    "valueCanonical": format!("{MIME_VS}|{conflicting_version}")
                }),
            ]);
            let (status, body) = app
                .post(
                    &format!("/ValueSet/{id}/$validate-code"),
                    json!({"resourceType": "Parameters", "parameter": params}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, expected);
        }
    }

    // Once a local definition exists, its membership is authoritative for
    // canonical and instance requests. The earlier successful syntax-only
    // responses must not survive this import.
    app.import(vec![json!({
        "resourceType": "CodeSystem", "id": "loaded-bcp13", "url": BCP13,
        "status": "active", "content": "complete",
        "concept": [{"code": "application/json", "display": "JSON"}]
    })])
    .await;
    for (code, expected) in [("text/cql", false), ("application/json", true)] {
        let (status, body) = app
            .get(&query(
                "/ValueSet/mime-old/$validate-code",
                &[("system", BCP13), ("code", code)],
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_valid(&body, expected);
        let piped_url = format!("{MIME_VS}|1.0.0");
        for mut params in [
            vec![("url", MIME_VS), ("valueSetVersion", "1.0.0")],
            vec![("url", piped_url.as_str())],
        ] {
            params.extend([("system", BCP13), ("code", code)]);
            let (status, body) = app.get(&query("/ValueSet/$validate-code", &params)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_valid(&body, expected);
        }
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_stored_mime_validation_preserves_selected_definition() {
    stored_mime_instance_contract(InstanceApp::Sqlite(common::TestApp::new())).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_stored_mime_validation_preserves_selected_definition() {
    stored_mime_instance_contract(InstanceApp::Postgres(
        common::TestAppPg::new_isolated().await,
    ))
    .await;
}
