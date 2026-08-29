//! Regression coverage for FHIR `string` search on terminology resources.

mod common;

use axum::http::StatusCode;
#[cfg(feature = "sqlite")]
use common::TestApp;
#[cfg(feature = "postgres")]
use common::TestAppPg;
use serde_json::Value;

const SEARCH_BUNDLE: &str = r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {"resource": {
      "resourceType": "CodeSystem", "id": "cafe-cs",
      "url": "http://hts.test/719/cs/cafe", "version": "1.0",
      "name": "CafeCodeSystem", "title": "Café terminology", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "CodeSystem", "id": "literal-cs",
      "url": "http://hts.test/719/cs/literal", "version": "1.0",
      "name": "LiteralCodeSystem", "title": "Rate_100% terminology", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "CodeSystem", "id": "wildcard-decoy-cs",
      "url": "http://hts.test/719/cs/decoy", "version": "1.0",
      "name": "WildcardDecoy", "title": "RateX100Y terminology", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "CodeSystem", "id": "page-one-cs",
      "url": "http://hts.test/719/cs/page-one", "version": "1.0",
      "name": "PageOne", "title": "First page candidate", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "CodeSystem", "id": "page-two-cs",
      "url": "http://hts.test/719/cs/page-two", "version": "1.0",
      "name": "PageTwo", "title": "Second page candidate", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "CodeSystem", "id": "nameless-cs",
      "url": "http://hts.test/719/cs/nameless", "version": "1.0",
      "title": "Nameless candidate", "status": "active",
      "content": "complete", "concept": [{"code": "one", "display": "One"}]
    }},
    {"resource": {
      "resourceType": "ValueSet", "id": "cafe-vs",
      "url": "http://hts.test/719/vs/cafe", "version": "1.0",
      "name": "CafeValueSet", "title": "Cafe\u0301 value set", "status": "active",
      "compose": {"include": [{"system": "http://hts.test/719/cs/cafe"}]}
    }},
    {"resource": {
      "resourceType": "ConceptMap", "id": "cafe-cm",
      "url": "http://hts.test/719/cm/cafe", "version": "1.0",
      "name": "CafeConceptMap", "title": "CAFÉ concept map", "status": "active",
      "sourceUri": "http://hts.test/719/cs/cafe", "targetUri": "http://example.org/target",
      "group": []
    }}
  ]
}"#;

fn entries(body: &Value) -> &[Value] {
    body["entry"].as_array().expect("searchset entry array")
}

fn resource_id(resource: &Value) -> &str {
    resource["id"].as_str().expect("resource id")
}

fn assert_id(resource: &Value, expected: &str) {
    let actual = resource_id(resource);
    assert!(
        actual == expected
            || actual
                .strip_prefix(expected)
                .is_some_and(|suffix| suffix.starts_with('|')),
        "expected {expected} (optionally version-qualified), got {actual}"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_supports_prefix_contains_exact_and_decoded_modifier_keys() {
    let app = TestApp::new();
    app.import_bundle_ok(SEARCH_BUNDLE).await;

    for (path, expected_type, expected_id) in [
        ("/CodeSystem?title=cafe", "CodeSystem", "cafe-cs"),
        ("/ValueSet?name%3Acontains=fevalue", "ValueSet", "cafe-vs"),
        (
            "/ConceptMap?title:contains=FE%20concept",
            "ConceptMap",
            "cafe-cm",
        ),
    ] {
        let (status, body) = app.get_fhir(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(entries(&body).len(), 1, "{path}: {body}");
        assert_eq!(entries(&body)[0]["resource"]["resourceType"], expected_type);
        assert_id(&entries(&body)[0]["resource"], expected_id);
    }

    let (status, exact) = app
        .get_fhir("/CodeSystem?title:exact=Caf%C3%A9%20terminology")
        .await;
    assert_eq!(status, StatusCode::OK, "{exact}");
    assert_eq!(entries(&exact).len(), 1, "{exact}");

    for path in [
        "/CodeSystem?title:exact=caf%C3%A9%20terminology",
        "/CodeSystem?title:exact=Cafe%CC%81%20terminology",
        "/CodeSystem?title=terminology",
    ] {
        let (status, body) = app.get_fhir(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert!(entries(&body).is_empty(), "{path}: {body}");
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_applies_and_semantics_literal_matching_and_filter_before_pagination() {
    let app = TestApp::new();
    app.import_bundle_ok(SEARCH_BUNDLE).await;

    let (status, combined) = app
        .get_fhir(
            "/CodeSystem?name=cafe&title:contains=terminology&url=http%3A%2F%2Fhts.test%2F719%2Fcs%2Fcafe&status=active",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{combined}");
    assert_eq!(entries(&combined).len(), 1, "{combined}");

    let (_, wrong_status) = app.get_fhir("/CodeSystem?name=cafe&status=retired").await;
    assert!(entries(&wrong_status).is_empty(), "{wrong_status}");
    let (_, missing_name) = app.get_fhir("/CodeSystem?name=nameless").await;
    assert!(entries(&missing_name).is_empty(), "{missing_name}");

    let (_, literal) = app.get_fhir("/CodeSystem?title=rate_100%25").await;
    assert_eq!(entries(&literal).len(), 1, "{literal}");
    assert_id(&entries(&literal)[0]["resource"], "literal-cs");

    let (_, all_matches) = app.get_fhir("/CodeSystem?name=page").await;
    assert_eq!(entries(&all_matches).len(), 2, "{all_matches}");
    let (_, page) = app
        .get_fhir("/CodeSystem?name=page&_offset=1&_count=1")
        .await;
    assert_eq!(entries(&page).len(), 1, "{page}");
    assert_eq!(
        resource_id(&entries(&page)[0]["resource"]),
        resource_id(&entries(&all_matches)[1]["resource"])
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn empty_and_normalization_empty_parameters_are_ignored_first() {
    let app = TestApp::new();
    app.import_bundle_ok(SEARCH_BUNDLE).await;

    let (_, baseline) = app.get_fhir("/CodeSystem").await;
    let baseline_ids = entries(&baseline)
        .iter()
        .map(|entry| resource_id(&entry["resource"]))
        .collect::<Vec<_>>();

    for path in [
        "/CodeSystem?name=",
        "/CodeSystem?url=",
        "/CodeSystem?version=",
        "/CodeSystem?title=",
        "/CodeSystem?status=",
        "/CodeSystem?name:not=",
        "/CodeSystem?url:not=",
        "/CodeSystem?_count=",
        "/CodeSystem?_offset=",
        "/CodeSystem?_summary=",
        "/CodeSystem?name:contains=%CC%81",
        "/CodeSystem?unknown=value",
    ] {
        let (status, body) = app.get_fhir(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        let ids = entries(&body)
            .iter()
            .map(|entry| resource_id(&entry["resource"]))
            .collect::<Vec<_>>();
        assert_eq!(ids, baseline_ids, "{path}: {body}");
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn unsupported_modifiers_on_announced_parameters_return_operation_outcome() {
    let app = TestApp::new();

    for parameter in ["url", "version", "name", "title", "status"] {
        let path = format!("/CodeSystem?{parameter}:not=value");
        let (status, body) = app.get_fhir(&path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(body["resourceType"], "OperationOutcome", "{path}: {body}");
        assert_eq!(body["issue"][0]["code"], "invalid", "{path}: {body}");
    }

    let (status, body) = app.get_fhir("/CodeSystem?_count=not-a-number").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome", "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_preserves_resource_specific_hydration_behavior() {
    let app = TestApp::new();
    app.import_bundle_ok(SEARCH_BUNDLE).await;

    let (_, code_system) = app.get_fhir("/CodeSystem?name=cafe").await;
    assert!(entries(&code_system)[0]["resource"]["concept"].is_array());
    let (_, code_system_summary) = app.get_fhir("/CodeSystem?name=cafe&_summary=true").await;
    assert!(entries(&code_system_summary)[0]["resource"]["concept"].is_null());

    let (_, value_set) = app.get_fhir("/ValueSet?name=cafe").await;
    assert!(entries(&value_set)[0]["resource"]["compose"].is_object());
    let (_, value_set_summary) = app.get_fhir("/ValueSet?name=cafe&_summary=true").await;
    assert!(entries(&value_set_summary)[0]["resource"]["compose"].is_null());

    let (_, concept_map_summary) = app.get_fhir("/ConceptMap?name=cafe&_summary=true").await;
    assert!(entries(&concept_map_summary)[0]["resource"]["group"].is_array());

    let (_, unfiltered_code_systems) = app.get_fhir("/CodeSystem").await;
    assert!(
        entries(&unfiltered_code_systems)
            .iter()
            .all(|entry| entry["resource"]["concept"].is_null())
    );
    let (_, unfiltered_concept_maps) = app.get_fhir("/ConceptMap").await;
    assert!(entries(&unfiltered_concept_maps)[0]["resource"]["group"].is_array());
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_matches_string_and_full_hydration_contract() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(SEARCH_BUNDLE).await;

    for (path, full_field, expected_id) in [
        ("/CodeSystem?title=cafe&_summary=true", "concept", "cafe-cs"),
        (
            "/ValueSet?name:contains=fevalue&_summary=true",
            "compose",
            "cafe-vs",
        ),
        (
            "/ConceptMap?title:contains=fe%20concept&_summary=true",
            "group",
            "cafe-cm",
        ),
    ] {
        let (status, body) = app.get_fhir(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(entries(&body).len(), 1, "{path}: {body}");
        let resource = &entries(&body)[0]["resource"];
        assert!(!resource[full_field].is_null(), "{path}: {body}");
        assert_id(resource, expected_id);
    }

    let (_, exact_miss) = app
        .get_fhir("/ConceptMap?title:exact=caf%C3%A9%20concept%20map")
        .await;
    assert!(entries(&exact_miss).is_empty(), "{exact_miss}");

    let (status, outcome) = app.get_fhir("/ConceptMap?status:not=active").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{outcome}");
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{outcome}");
}
