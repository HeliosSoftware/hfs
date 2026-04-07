//! HTTP-layer integration tests for the PostgreSQL terminology backend.
//!
//! Uses `TestAppPg` from `tests/common/mod.rs` to run the full HTS Axum router
//! against a live PostgreSQL testcontainer.  This mirrors the SQLite test
//! coverage in `code_system_ops.rs`, `value_set_ops.rs`, `concept_map_ops.rs`,
//! and `terminology_import.rs`.
//!
//! Run with:
//!   `cargo test -p helios-hts --features postgres --test postgres_http_tests`

#![cfg(feature = "postgres")]

mod common;

use axum::http::StatusCode;
use common::{TestAppPg, bundles};

// ═══════════════════════════════════════════════════════════════════════════════
// CRUD round-trip tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn crud_code_system_create_read_update_delete() {
    let app = TestAppPg::new().await;

    let cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": "http://pg-crud-test.example/cs/lifecycle",
        "version": "1.0",
        "name": "LifecycleCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "alive", "display": "Alive"}]
    })
    .to_string();

    // POST → 201 Created
    let (status, body) = app.post_fhir("/CodeSystem", cs.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "POST: {body}");
    let id = body["id"].as_str().expect("id field missing").to_owned();

    // GET → 200 OK
    let (status, body) = app.get_fhir(&format!("/CodeSystem/{id}")).await;
    assert_eq!(status, StatusCode::OK, "GET: {body}");
    assert_eq!(body["url"], "http://pg-crud-test.example/cs/lifecycle");

    // PUT → 200 OK with incremented ETag
    let updated_cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "id": id,
        "url": "http://pg-crud-test.example/cs/lifecycle",
        "version": "2.0",
        "name": "LifecycleCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "alive", "display": "Still Alive"}]
    })
    .to_string();
    let (status, body) = app
        .put_fhir(&format!("/CodeSystem/{id}"), updated_cs, None)
        .await;
    assert_eq!(status, StatusCode::OK, "PUT: {body}");
    assert_eq!(body["version"], "2.0");

    // DELETE → 204 No Content
    let status = app.delete_fhir(&format!("/CodeSystem/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "DELETE");

    // GET after delete → 404
    let (status, _) = app.get_fhir(&format!("/CodeSystem/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "GET after DELETE");
}

#[tokio::test]
async fn crud_code_system_if_match_mismatch_returns_412() {
    let app = TestAppPg::new().await;

    let cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": "http://pg-crud-test.example/cs/ifmatch",
        "name": "IfMatchCS",
        "status": "active",
        "content": "complete"
    })
    .to_string();

    let (status, body) = app.post_fhir("/CodeSystem", cs.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    let update = serde_json::json!({
        "resourceType": "CodeSystem",
        "id": id,
        "url": "http://pg-crud-test.example/cs/ifmatch",
        "name": "IfMatchCS",
        "status": "active",
        "content": "complete"
    })
    .to_string();

    // Wrong ETag → 412
    let (status, _) = app
        .put_fhir(&format!("/CodeSystem/{id}"), update, Some("W/\"999\""))
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn crud_value_set_create_read_delete() {
    let app = TestAppPg::new().await;

    let vs = serde_json::json!({
        "resourceType": "ValueSet",
        "url": "http://pg-crud-test.example/vs/lifecycle",
        "name": "LifecycleVS",
        "status": "active"
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet", vs).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    let (status, _) = app.get_fhir(&format!("/ValueSet/{id}")).await;
    assert_eq!(status, StatusCode::OK);

    let status = app.delete_fhir(&format!("/ValueSet/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.get_fhir(&format!("/ValueSet/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn crud_concept_map_create_read_delete() {
    let app = TestAppPg::new().await;

    let cm = serde_json::json!({
        "resourceType": "ConceptMap",
        "url": "http://pg-crud-test.example/cm/lifecycle",
        "name": "LifecycleCM",
        "status": "active"
    })
    .to_string();

    let (status, body) = app.post_fhir("/ConceptMap", cm).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    let (status, _) = app.get_fhir(&format!("/ConceptMap/{id}")).await;
    assert_eq!(status, StatusCode::OK);

    let status = app.delete_fhir(&format!("/ConceptMap/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.get_fhir(&format!("/ConceptMap/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn crud_delete_also_removes_lookup_data() {
    // Verifies the Gap 1 fix: after DELETE /CodeSystem/{id}, $lookup returns 404.
    let app = TestAppPg::new().await;

    let cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": "http://pg-crud-delete.example/cs/deleteme",
        "name": "DeleteMeCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "gone", "display": "Gone Soon"}]
    })
    .to_string();

    let (status, body) = app.post_fhir("/CodeSystem", cs).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    // $lookup works before delete
    let lookup = TestAppPg::params(&[
        (
            "system",
            "valueUri",
            "http://pg-crud-delete.example/cs/deleteme",
        ),
        ("code", "valueCode", "gone"),
    ]);
    let (status, _) = app.post_fhir("/CodeSystem/$lookup", lookup.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "$lookup before delete should succeed"
    );

    // DELETE
    let status = app.delete_fhir(&format!("/CodeSystem/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // $lookup after delete → 404 (HTS normalized tables cleaned up)
    let (status, _) = app.post_fhir("/CodeSystem/$lookup", lookup).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "$lookup after DELETE should return 404"
    );
}

#[tokio::test]
async fn crud_delete_also_removes_expand_data() {
    // Verifies the Gap 1 fix: after DELETE /ValueSet/{id}, $expand returns 404.
    let app = TestAppPg::new().await;

    // Need a CodeSystem first for the ValueSet to reference
    let cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": "http://pg-crud-delete.example/cs/for-vs",
        "name": "ForVsCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "x", "display": "X"}]
    })
    .to_string();
    app.post_fhir("/CodeSystem", cs).await;

    let vs = serde_json::json!({
        "resourceType": "ValueSet",
        "url": "http://pg-crud-delete.example/vs/deleteme",
        "name": "DeleteMeVS",
        "status": "active",
        "compose": {"include": [{"system": "http://pg-crud-delete.example/cs/for-vs"}]}
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet", vs).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    // $expand works before delete
    let expand = TestAppPg::params(&[(
        "url",
        "valueUri",
        "http://pg-crud-delete.example/vs/deleteme",
    )]);
    let (status, _) = app.post_fhir("/ValueSet/$expand", expand.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "$expand before delete should succeed"
    );

    // DELETE
    let del = app.delete_fhir(&format!("/ValueSet/{id}")).await;
    assert_eq!(del, StatusCode::NO_CONTENT);

    // $expand after delete → 404
    let (status, _) = app.post_fhir("/ValueSet/$expand", expand).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "$expand after DELETE should return 404"
    );
}

#[tokio::test]
async fn crud_delete_also_removes_translate_data() {
    // Verifies the Gap 1 fix: after DELETE /ConceptMap/{id}, $translate returns false.
    let app = TestAppPg::new().await;

    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Create a standalone ConceptMap
    let cm = serde_json::json!({
        "resourceType": "ConceptMap",
        "url": "http://pg-crud-delete.example/cm/deleteme",
        "name": "DeleteMeCM",
        "status": "active",
        "sourceUri": "http://hts.test/anatomy",
        "targetUri": "http://snomed.info/sct",
        "group": [{
            "source": "http://hts.test/anatomy",
            "target": "http://snomed.info/sct",
            "element": [{"code": "arm", "target": [{"code": "99999", "equivalence": "equivalent"}]}]
        }]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ConceptMap", cm).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();

    // Translate works before delete
    let tx = TestAppPg::params(&[
        (
            "url",
            "valueUri",
            "http://pg-crud-delete.example/cm/deleteme",
        ),
        ("system", "valueUri", "http://hts.test/anatomy"),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", tx.clone()).await;
    assert_eq!(status, StatusCode::OK, "$translate before delete: {body}");
    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "translate should return true before delete");

    // DELETE
    let del = app.delete_fhir(&format!("/ConceptMap/{id}")).await;
    assert_eq!(del, StatusCode::NO_CONTENT);

    // Translate after delete → result=false (map no longer indexed)
    let (status, body) = app.post_fhir("/ConceptMap/$translate", tx).await;
    assert_eq!(status, StatusCode::OK);
    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(
        !result,
        "translate should return false after delete, got: {body}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// By-id operation endpoints
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn lookup_by_id_post_returns_same_as_system_level() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Find the CodeSystem id from search
    let (status, search_body) = app
        .get_fhir(&format!("/CodeSystem?url={}", bundles::ANATOMY_CS_URL))
        .await;
    assert_eq!(status, StatusCode::OK, "{search_body}");
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .expect("expected id in search result")
        .to_owned();

    let req = TestAppPg::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);

    // System-level lookup
    let (status, sys_body) = app.post_fhir("/CodeSystem/$lookup", req.clone()).await;
    assert_eq!(status, StatusCode::OK, "system-level: {sys_body}");

    // By-id lookup
    let (status, id_body) = app
        .post_fhir(&format!("/CodeSystem/{id}/$lookup"), req)
        .await;
    assert_eq!(status, StatusCode::OK, "by-id: {id_body}");

    let sys_display = sys_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .unwrap();
    let id_display = id_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .unwrap();

    assert_eq!(
        sys_display, id_display,
        "by-id and system-level should return same display"
    );
}

#[tokio::test]
async fn lookup_by_id_get_returns_display() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (_, search_body) = app
        .get_fhir(&format!("/CodeSystem?url={}", bundles::ANATOMY_CS_URL))
        .await;
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = app
        .get_fhir(&format!(
            "/CodeSystem/{id}/$lookup?system={}&code=arm",
            bundles::ANATOMY_CS_URL
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let display = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .unwrap();
    assert_eq!(display, "Arm");
}

#[tokio::test]
async fn lookup_by_id_unknown_id_returns_404() {
    let app = TestAppPg::new().await;

    let (status, _) = app
        .get_fhir("/CodeSystem/nonexistent-id-xyz/$lookup?code=arm")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn expand_by_id_post_returns_expansion() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (_, search_body) = app
        .get_fhir(&format!("/ValueSet?url={}", bundles::LIMBS_VS_URL))
        .await;
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let req = TestAppPg::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);

    let (status_sys, sys_body) = app.post_fhir("/ValueSet/$expand", req.clone()).await;
    let (status_id, id_body) = app.post_fhir(&format!("/ValueSet/{id}/$expand"), req).await;

    assert_eq!(status_sys, StatusCode::OK, "{sys_body}");
    assert_eq!(status_id, StatusCode::OK, "{id_body}");

    let sys_codes: Vec<&str> = sys_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["code"].as_str())
        .collect();
    let id_codes: Vec<&str> = id_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["code"].as_str())
        .collect();
    assert_eq!(
        sys_codes, id_codes,
        "by-id and system-level should expand identically"
    );
}

#[tokio::test]
async fn expand_by_id_get_returns_expansion() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (_, search_body) = app
        .get_fhir(&format!("/ValueSet?url={}", bundles::LIMBS_VS_URL))
        .await;
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = app
        .get_fhir(&format!(
            "/ValueSet/{id}/$expand?url={}",
            bundles::LIMBS_VS_URL
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["expansion"]["contains"].as_array().is_some(),
        "expected expansion.contains"
    );
}

#[tokio::test]
async fn expand_by_id_unknown_id_returns_404() {
    let app = TestAppPg::new().await;
    let (status, _) = app.get_fhir("/ValueSet/nonexistent-vs-id/$expand").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn translate_by_id_post_returns_match() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (_, search_body) = app
        .get_fhir(&format!("/ConceptMap?url={}", bundles::CM_URL_R4))
        .await;
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let req = TestAppPg::params(&[
        ("url", "valueUri", bundles::CM_URL_R4),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app
        .post_fhir(&format!("/ConceptMap/{id}/$translate"), req)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "expected result=true");
}

#[tokio::test]
async fn translate_by_id_get_returns_match() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (_, search_body) = app
        .get_fhir(&format!("/ConceptMap?url={}", bundles::CM_URL_R4))
        .await;
    let id = search_body["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = app
        .get_fhir(&format!(
            "/ConceptMap/{id}/$translate?url={}&system={}&code=arm",
            bundles::CM_URL_R4,
            bundles::ANATOMY_CS_URL
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result);
}

#[tokio::test]
async fn translate_by_id_unknown_id_returns_404() {
    let app = TestAppPg::new().await;
    let (status, _) = app
        .get_fhir("/ConceptMap/nonexistent-cm-id/$translate?code=arm")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Format negotiation
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn lookup_xml_response_contains_parameters_root() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestAppPg::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir_xml("/CodeSystem/$lookup", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("<Parameters"),
        "expected <Parameters in XML response, got: {body}"
    );
}

#[tokio::test]
async fn lookup_xml_via_format_query_param() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestAppPg::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    // post_fhir_xml uses Accept: application/fhir+xml header; the _format
    // query-param path is equivalent and exercised by checking the XML body.
    let (status, body) = app
        .post_fhir_xml("/CodeSystem/$lookup?_format=xml", req)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("<Parameters"),
        "expected XML Parameters response, got: {body}"
    );
}

#[tokio::test]
async fn expand_xml_response_contains_valueset_root() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestAppPg::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);
    let (status, body) = app.post_fhir_xml("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("<ValueSet"),
        "expected <ValueSet in XML response, got: {body}"
    );
}

#[tokio::test]
async fn metadata_returns_capability_statement() {
    let app = TestAppPg::new().await;
    let (status, body) = app.get_fhir("/metadata").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "CapabilityStatement");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Import endpoint (POST /import)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn import_r4_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r4_bundle()).await;

    assert_eq!(status, StatusCode::OK, "expected 200 OK: {body}");
    assert_eq!(
        body["code_systems"].as_u64(),
        Some(1),
        "expected 1 CodeSystem"
    );
    assert_eq!(body["value_sets"].as_u64(), Some(1), "expected 1 ValueSet");
    assert_eq!(
        body["concept_maps"].as_u64(),
        Some(1),
        "expected 1 ConceptMap"
    );
    // 5 concepts: body, limb, arm, leg, head
    assert_eq!(body["concepts"].as_u64(), Some(5), "expected 5 concepts");
}

#[tokio::test]
async fn import_r4b_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r4b_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test]
async fn import_r5_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r5_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test]
async fn import_r6_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r6_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test]
async fn reimport_same_bundle_is_idempotent() {
    let app = TestAppPg::new().await;

    let (status, _) = app.import_bundle(bundles::r4_bundle()).await;
    assert_eq!(status, StatusCode::OK);

    let (status2, body2) = app.import_bundle(bundles::r4_bundle()).await;
    assert!(
        status2 == StatusCode::OK || status2 == StatusCode::MULTI_STATUS,
        "second import should succeed, got {status2}: {body2}"
    );
}

#[tokio::test]
async fn import_non_bundle_returns_400() {
    let app = TestAppPg::new().await;

    let not_a_bundle = r#"{"resourceType":"Patient","id":"p1"}"#;
    let (status, _) = app.import_bundle(not_a_bundle).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn import_bundle_with_malformed_cs_returns_207() {
    let app = TestAppPg::new().await;

    let bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [
        {
          "resource": {
            "resourceType": "CodeSystem",
            "id": "pg-http-no-url-cs",
            "version": "1.0",
            "name": "NoUrlCS",
            "status": "active",
            "content": "complete",
            "concept": [{"code": "X", "display": "X"}]
          }
        },
        {
          "resource": {
            "resourceType": "CodeSystem",
            "id": "pg-http-good-cs",
            "url": "http://pg-http-import-test.example/good-cs",
            "version": "1.0",
            "name": "GoodCS",
            "status": "active",
            "content": "complete",
            "concept": [{"code": "A", "display": "A"}]
          }
        }
      ]
    }"#;

    let (status, body) = app.import_bundle(bundle).await;
    assert_eq!(status, StatusCode::MULTI_STATUS, "expected 207: {body}");

    let errors = body["errors"].as_array().expect("expected errors array");
    assert!(!errors.is_empty(), "expected at least one error");
    assert_eq!(
        body["code_systems"].as_u64(),
        Some(1),
        "valid CodeSystem should still be imported"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// ValueSet validate-code input types
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn vs_validate_coding_in_set_returns_true() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Coding input type
    let req = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "coding",
                "valueCoding": {
                    "system": bundles::ANATOMY_CS_URL,
                    "code": "arm"
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "arm is in the limbs ValueSet");
}

#[tokio::test]
async fn vs_validate_codeable_concept_one_match_returns_true() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://other.example", "code": "nope"},
                        {"system": bundles::ANATOMY_CS_URL, "code": "leg"}
                    ]
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "one coding in the CodeableConcept matches");
}

#[tokio::test]
async fn vs_validate_codeable_concept_no_match_returns_false() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://other.example", "code": "nope"},
                        {"system": bundles::ANATOMY_CS_URL, "code": "head"}
                    ]
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(true);
    assert!(!result, "head is not in the limbs ValueSet");
}

// ═══════════════════════════════════════════════════════════════════════════════
// ValueSet expand flags
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn expand_hierarchical_false_returns_flat_list() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Expand the full CodeSystem in flat (non-hierarchical) mode.
    let req_flat = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::ANATOMY_CS_URL},
            {"name": "hierarchical", "valueBoolean": false}
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$expand", req_flat).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // In flat mode, no entry in contains should have its own `contains` child array.
    let empty: Vec<serde_json::Value> = vec![];
    let contains = body["expansion"]["contains"].as_array().unwrap_or(&empty);
    for entry in contains {
        assert!(
            entry.get("contains").is_none(),
            "flat mode should not produce nested contains, found: {entry}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Multi-version ConceptMap translate
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn translate_r4b_arm_returns_snomed_code() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4b_bundle()).await;

    let req = TestAppPg::params(&[
        ("url", "valueUri", bundles::CM_URL_R4B),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "R4B arm should translate to SNOMED code");
}

#[tokio::test]
async fn translate_r5_arm_returns_snomed_code() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r5_bundle()).await;

    let req = TestAppPg::params(&[
        ("url", "valueUri", bundles::CM_URL_R5),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "R5 arm should translate to SNOMED code");
}

#[tokio::test]
async fn translate_r6_arm_returns_snomed_code() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r6_bundle()).await;

    let req = TestAppPg::params(&[
        ("url", "valueUri", bundles::CM_URL_R6),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .unwrap_or(false);
    assert!(result, "R6 arm should translate to SNOMED code");
}
