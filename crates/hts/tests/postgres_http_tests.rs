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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn lookup_by_id_unknown_id_returns_404() {
    let app = TestAppPg::new().await;

    let (status, _) = app
        .get_fhir("/CodeSystem/nonexistent-id-xyz/$lookup?code=arm")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
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

    let mut sys_codes: Vec<&str> = sys_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["code"].as_str())
        .collect();
    sys_codes.sort();
    let mut id_codes: Vec<&str> = id_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["code"].as_str())
        .collect();
    id_codes.sort();
    assert_eq!(
        sys_codes, id_codes,
        "by-id and system-level should expand identically"
    );
}

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn expand_by_id_unknown_id_returns_404() {
    let app = TestAppPg::new().await;
    let (status, _) = app.get_fhir("/ValueSet/nonexistent-vs-id/$expand").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn metadata_returns_capability_statement() {
    let app = TestAppPg::new().await;
    let (status, body) = app.get_fhir("/metadata").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "CapabilityStatement");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Import endpoint (POST /import)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn import_r4b_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r4b_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn import_r5_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r5_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn import_r6_bundle_returns_correct_counts() {
    let app = TestAppPg::new().await;
    let (status, body) = app.import_bundle(bundles::r6_bundle()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn import_non_bundle_returns_400() {
    let app = TestAppPg::new().await;

    let not_a_bundle = r#"{"resourceType":"Patient","id":"p1"}"#;
    let (status, _) = app.import_bundle(not_a_bundle).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A chunked import into a running server with `?defer_closure=true` leaves
/// `concept_closure` empty, and `?finalize=true` rebuilds it. Same scenario as the SQLite test in
/// `operations::import_bundle`, but the symptom differs per backend: PostgreSQL
/// answers `$subsumes` from a recursive CTE over `concept_hierarchy`, so it is
/// unaffected there; what reads the closure on PostgreSQL is the `is-a`
/// ValueSet path, so the implicit `?fhir_vs=isa/A` expansion is what goes
/// empty. `$subsumes` is asserted alongside to document that difference.
#[tokio::test(flavor = "multi_thread")]
async fn import_finalize_rebuilds_closure_dropped_by_a_chunked_import() {
    // Own database: any other test's `TestAppPg::new` would rebuild the
    // closure this test drops on purpose, before `finalize=true` gets to it.
    let app = TestAppPg::new_isolated().await;
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let sys = format!("http://chunked.{uid}/cs");
    const DEFER: &str = "/import?defer_closure=true";

    let chunk = |concepts: &str| {
        format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"resource":{{
                 "resourceType":"CodeSystem","id":"chunked-{uid}","url":"{sys}",
                 "version":"1.0","name":"ChunkedCS","status":"active",
                 "content":"complete","hierarchyMeaning":"is-a",
                 "property":[{{"code":"parent","uri":"http://hl7.org/fhir/concept-properties#parent","type":"code"}}],
                 "concept":[{concepts}]
               }}}}]}}"#
        )
    };
    async fn subsumes(app: &TestAppPg, sys: &str, a: &str, b: &str) -> String {
        let path = format!("/CodeSystem/$subsumes?system={sys}&codeA={a}&codeB={b}");
        let (status, body) = app.get_fhir(&path).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["parameter"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| p["name"] == "outcome"))
            .and_then(|p| p["valueCode"].as_str())
            .unwrap_or("<none>")
            .to_string()
    }
    /// Codes in the implicit `<sys>?fhir_vs=isa/A` ValueSet, sorted.
    async fn isa_members(app: &TestAppPg, sys: &str) -> Vec<String> {
        let path = format!("/ValueSet/$expand?url={sys}%3Ffhir_vs%3Disa%2FA");
        let (status, body) = app.get_fhir(&path).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut codes: Vec<String> = body["expansion"]["contains"]
            .as_array()
            .map(|cs| {
                cs.iter()
                    .filter_map(|c| c["code"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        codes.sort();
        codes
    }

    // Chunk 1: metadata only. Chunk 2: parent + child, builds the closure.
    // Chunk 3: another child; the system is no longer empty, so the closure is
    // deleted and not rebuilt.
    let (status, body) = app.post_fhir(DEFER, chunk("")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .post_fhir(
            DEFER,
            chunk(
                r#"{"code":"A","display":"A"},
               {"code":"A.0","display":"A zero","property":[{"code":"parent","valueCode":"A"}]}"#,
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .post_fhir(
            DEFER,
            chunk(
            r#"{"code":"A.1","display":"A one","property":[{"code":"parent","valueCode":"A"}]}"#,
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(
        isa_members(&app, &sys).await.is_empty(),
        "chunked import should have left the closure empty"
    );
    // Unaffected on PostgreSQL: walks concept_hierarchy, not the closure.
    assert_eq!(subsumes(&app, &sys, "A", "A.0").await, "subsumes");

    let (status, body) = app.post_fhir("/import?finalize=true", chunk("")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["closures_rebuilt"].as_u64(), Some(1), "{body}");

    assert_eq!(isa_members(&app, &sys).await, ["A", "A.0", "A.1"]);
    assert_eq!(subsumes(&app, &sys, "A", "A.1").await, "subsumes");
}

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn expand_hierarchical_false_returns_flat_list() {
    let app = TestAppPg::new().await;
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Expand the limbs ValueSet in flat (non-hierarchical) mode.
    let req_flat = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

// ═══════════════════════════════════════════════════════════════════════════════
// Re-import refreshes filtered $expand — PostgreSQL parity for issue #295
// ═══════════════════════════════════════════════════════════════════════════════

/// #295 parity: the fix that makes SQLite rebuild its concept FTS on re-import
/// is a no-op for PostgreSQL, which searches a live `pg_trgm` GIN index on
/// `concepts.display` that the engine maintains transactionally on every write.
/// This test pins that invariant — a re-imported display is reflected by a
/// filtered `$expand` with no application-side index maintenance and no restart.
#[tokio::test(flavor = "multi_thread")]
async fn reimport_changed_display_refreshes_filtered_expand_pg() {
    let app = TestAppPg::new().await;

    let bundle = |display: &str| {
        serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{"resource": {
                "resourceType": "CodeSystem",
                "url": "http://pg-295.example/cs/reindex",
                "version": "1.0.0",
                "status": "active",
                "content": "complete",
                "concept": [{"code": "x", "display": display}]
            }}]
        })
        .to_string()
    };

    let expand = |filter: &str| {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "filter", "valueString": filter},
                {"name": "valueSet", "resource": {
                    "resourceType": "ValueSet",
                    "url": "http://pg-295.example/vs/reindex",
                    "status": "active",
                    "compose": {"include": [{"system": "http://pg-295.example/cs/reindex"}]}
                }}
            ]
        })
        .to_string()
    };

    let contains_x = |body: &serde_json::Value| -> bool {
        body["expansion"]["contains"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|c| c["code"].as_str() == Some("x")))
    };

    app.import_bundle_ok(&bundle("originalword marker")).await;
    let (status, body) = app
        .post_fhir("/ValueSet/$expand", expand("originalword"))
        .await;
    assert_eq!(status, StatusCode::OK, "initial expand failed: {body}");
    assert!(contains_x(&body), "original display should match: {body}");

    app.import_bundle_ok(&bundle("changedword marker")).await;

    let (status, body) = app
        .post_fhir("/ValueSet/$expand", expand("changedword"))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "post-reimport expand failed: {body}"
    );
    assert!(
        contains_x(&body),
        "changed display should match after re-import: {body}"
    );

    let (status, body) = app
        .post_fhir("/ValueSet/$expand", expand("originalword"))
        .await;
    assert_eq!(status, StatusCode::OK, "stale-filter expand failed: {body}");
    assert!(
        !contains_x(&body),
        "stale display must not match after re-import: {body}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cache invalidation on CRUD writes (issue #304)
// ═══════════════════════════════════════════════════════════════════════════════

/// Creating a ValueSet must un-cache the *absence* recorded for its URL by an
/// earlier `$expand` that 404'd.
///
/// The handler-level negative cache (`AppState::not_found_urls`) is
/// backend-agnostic, so this defect was identical on PostgreSQL and SQLite:
/// `clear_expand_cache()` was called only by `POST /import` and the CRUD delete
/// path, never by create or update. The server therefore kept answering 404 for
/// a ValueSet it had just returned `201 Created` for, indefinitely — nothing
/// expires that set.
///
/// The SQLite twin of this test lives in `operations/crud.rs`; this one exists
/// because the eviction seam is generic over the backend and the PostgreSQL CRUD
/// path reaches it through a different re-index route (the async importer rather
/// than the `hts_pool` blocking path).
#[tokio::test(flavor = "multi_thread")]
async fn create_value_set_evicts_the_expand_negative_cache() {
    let app = TestAppPg::new().await;

    let cs_url = "http://pg-cache-test.example/cs/neg";
    let vs_url = "http://pg-cache-test.example/vs/neg";

    let cs = serde_json::json!({
        "resourceType": "CodeSystem",
        "url": cs_url,
        "version": "1.0",
        "name": "NegCacheCS",
        "status": "active",
        "content": "complete",
        "concept": [{"code": "A", "display": "Alpha"}]
    })
    .to_string();
    let (status, body) = app.post_fhir("/CodeSystem", cs).await;
    assert_eq!(status, StatusCode::CREATED, "POST CodeSystem: {body}");

    let expand_uri = format!("/ValueSet/$expand?url={vs_url}");

    // 1. Absent → 404, which records the URL as definitively missing.
    let (status, _) = app.get_fhir(&expand_uri).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "precondition: the ValueSet must not exist yet"
    );

    // 2. Create exactly that ValueSet.
    let vs = serde_json::json!({
        "resourceType": "ValueSet",
        "url": vs_url,
        "name": "NegCacheVS",
        "status": "active",
        "compose": {"include": [{"system": cs_url, "concept": [{"code": "A"}]}]}
    })
    .to_string();
    let (status, body) = app.post_fhir("/ValueSet", vs).await;
    assert_eq!(status, StatusCode::CREATED, "POST ValueSet: {body}");

    // 3. It must expand now. Before the fix this stayed 404 forever.
    let (status, body) = app.get_fhir(&expand_uri).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a just-created ValueSet must not still be reported absent: {body}"
    );
    assert_eq!(
        body["expansion"]["contains"][0]["code"], "A",
        "expansion should carry the included code: {body}"
    );
}

/// Updating a CodeSystem must not leave `$lookup` answering from pre-update
/// content.
#[tokio::test(flavor = "multi_thread")]
async fn update_code_system_refreshes_lookup_display() {
    let app = TestAppPg::new().await;

    let cs_url = "http://pg-cache-test.example/cs/display";
    let make_cs = |display: &str| {
        serde_json::json!({
            "resourceType": "CodeSystem",
            "url": cs_url,
            "version": "1.0",
            "name": "DisplayCS",
            "status": "active",
            "content": "complete",
            "concept": [{"code": "A", "display": display}]
        })
        .to_string()
    };

    let (status, body) = app.post_fhir("/CodeSystem", make_cs("Alpha")).await;
    assert_eq!(status, StatusCode::CREATED, "POST: {body}");
    let id = body["id"].as_str().expect("id field missing").to_owned();

    let lookup_uri = format!("/CodeSystem/$lookup?system={cs_url}&code=A");
    let display_of = |body: &serde_json::Value| -> Option<String> {
        body["parameter"]
            .as_array()?
            .iter()
            .find(|p| p["name"] == "display")
            .and_then(|p| p["valueString"].as_str())
            .map(str::to_owned)
    };

    // Warm the caches.
    let (status, body) = app.get_fhir(&lookup_uri).await;
    assert_eq!(status, StatusCode::OK, "warm lookup: {body}");
    assert_eq!(display_of(&body).as_deref(), Some("Alpha"));

    let (status, body) = app
        .put_fhir(&format!("/CodeSystem/{id}"), make_cs("Alpha v2"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "PUT: {body}");

    let (status, body) = app.get_fhir(&lookup_uri).await;
    assert_eq!(status, StatusCode::OK, "post-update lookup: {body}");
    assert_eq!(
        display_of(&body).as_deref(),
        Some("Alpha v2"),
        "$lookup must reflect the updated display: {body}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// VCL (Value Set Composition Language) — mirrors tests/vcl_expand.rs
// ═══════════════════════════════════════════════════════════════════════════════

mod vcl {
    use super::{StatusCode, TestAppPg, bundles};
    use serde_json::{Value, json};
    use tokio::sync::OnceCell;

    const SYS: &str = bundles::VCL_CS_URL;

    /// The VCL bundle uses fixed URLs and every test shares one database, so
    /// import it exactly once instead of racing re-imports across tests.
    static IMPORTED: OnceCell<()> = OnceCell::const_new();

    async fn app() -> TestAppPg {
        let app = TestAppPg::new().await;
        IMPORTED
            .get_or_init(|| async {
                app.import_bundle_ok(bundles::vcl_bundle()).await;
            })
            .await;
        app
    }

    fn vcl_url(expr: &str) -> String {
        format!("http://fhir.org/VCL?v1={expr}")
    }

    /// Percent-encodes everything but unreserved characters, as a GET client must.
    fn encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    fn collect(items: Option<&Vec<Value>>, out: &mut Vec<String>) {
        for e in items.into_iter().flatten() {
            if let Some(code) = e["code"].as_str() {
                out.push(code.to_string());
            }
            collect(e["contains"].as_array(), out);
        }
    }

    fn codes_of(body: &Value) -> Vec<String> {
        let mut codes = Vec::new();
        collect(body["expansion"]["contains"].as_array(), &mut codes);
        codes.sort();
        codes
    }

    async fn expand_with(
        app: &TestAppPg,
        expr: &str,
        extra: &[(&str, &str, &str)],
    ) -> (StatusCode, Value) {
        let url = vcl_url(expr);
        let mut entries = vec![("url", "valueUri", url.as_str())];
        entries.extend_from_slice(extra);
        app.post_fhir("/ValueSet/$expand", TestAppPg::params(&entries))
            .await
    }

    async fn expand_codes(app: &TestAppPg, expr: &str) -> Vec<String> {
        let (status, body) = expand_with(app, expr, &[]).await;
        assert_eq!(status, StatusCode::OK, "expand failed for {expr}: {body}");
        codes_of(&body)
    }

    fn issue_text(body: &Value) -> String {
        body["issue"][0]["details"]["text"]
            .as_str()
            .or_else(|| body["issue"][0]["diagnostics"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    // ── $expand: operators ─────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_is_a() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
        assert_eq!(codes, ["arm", "leg", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_is_a_without_property_extension() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})<<limb")).await;
        assert_eq!(codes, ["arm", "leg", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_exclusion_issue_example() {
        let app = app().await;
        let expr = format!("({SYS})<<body - ({SYS})<<limb");
        assert_eq!(expand_codes(&app, &expr).await, ["body", "head"]);

        // With paging the exclusion must survive the paged fast paths.
        let (status, body) = expand_with(&app, &expr, &[("count", "valueInteger", "10")]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(codes_of(&body), ["body", "head"]);

        // And with a text filter.
        let (status, body) = expand_with(&app, &expr, &[("filter", "valueString", "e")]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(codes_of(&body), ["head"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_is_not_a() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept~<<limb")).await;
        assert_eq!(codes, ["body", "head"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_descendent_of() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept<limb")).await;
        assert_eq!(codes, ["arm", "leg"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_child_of() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept<!body")).await;
        assert_eq!(codes, ["head", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_generalizes() {
        // `>>` maps to `generalizes`: the code plus its ancestors.
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept>>arm")).await;
        assert_eq!(codes, ["arm", "body", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_property_equals_and_conjunction() {
        let app = app().await;
        assert_eq!(
            expand_codes(&app, &format!("({SYS})region=upper")).await,
            ["arm", "head"]
        );
        assert_eq!(
            expand_codes(&app, &format!("({SYS})(region=upper,paired=true)")).await,
            ["arm"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_regex() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})code/\"a.*\"")).await;
        assert_eq!(codes, ["arm"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_regex_on_concept() {
        // #2115: `concept/"re"` names the concept code, same as `code/"re"`.
        let app = app().await;
        for prop in ["concept", "code"] {
            assert_eq!(
                expand_codes(&app, &format!("({SYS}){prop}/\"l.*\"")).await,
                ["leg", "limb"],
                "{prop}"
            );
            // After an earlier filter has built a candidate set.
            assert_eq!(
                expand_codes(&app, &format!("({SYS})(concept<<limb,{prop}/\"a.*\")")).await,
                ["arm"],
                "{prop}"
            );
            // With a text filter (FTS-first path on SQLite).
            let expr = format!("({SYS}){prop}/\"l.*\"");
            let (status, body) =
                expand_with(&app, &expr, &[("filter", "valueString", "leg")]).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(codes_of(&body), ["leg"], "{prop}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_in_list() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})concept^{{arm,leg}}")).await;
        assert_eq!(codes, ["arm", "leg"]);
        // `~^` on `concept` is rejected by the translator (use `~<<` or `-`).
        let (status, body) = expand_with(&app, &format!("({SYS})concept~^{{arm,leg}}"), &[]).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_code_list_and_disjunction() {
        let app = app().await;
        assert_eq!(
            expand_codes(&app, &format!("({SYS})(arm;head)")).await,
            ["arm", "head"]
        );
        assert_eq!(
            expand_codes(&app, &format!("({SYS})<<limb;({SYS})head")).await,
            ["arm", "head", "leg", "limb"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_whole_system() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS})*")).await;
        assert_eq!(codes, ["arm", "body", "head", "leg", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_system_with_version() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("({SYS}|1.0)<<limb")).await;
        assert_eq!(codes, ["arm", "leg", "limb"]);
    }

    // ── $expand: value set references ──────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_nested_stored_value_set() {
        let app = app().await;
        let codes = expand_codes(&app, &format!("^({})", bundles::VCL_LIMBS_VS_URL)).await;
        assert_eq!(codes, ["arm", "leg", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_self_importing_value_set_is_rejected() {
        // PostgreSQL turns a cyclic include into `VsInvalid` rather than the
        // warning + empty set SQLite produces. The type-level `$expand`
        // handler renders that message as 422 (`vs-invalid`).
        let app = app().await;
        let (status, body) = expand_with(
            &app,
            &format!("^({})", bundles::VCL_SELF_IMPORT_VS_URL),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["resourceType"], "OperationOutcome", "{body}");
        assert!(
            issue_text(&body).contains("Cyclic reference"),
            "{}",
            issue_text(&body)
        );
    }

    // ── $expand: transport, errors, caching ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_get_with_percent_encoded_url() {
        let app = app().await;
        let url = vcl_url(&encode(&format!("({SYS})concept<<limb")));
        let (status, body) = app
            .get_fhir(&format!("/ValueSet/$expand?url={}", encode(&url)))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(codes_of(&body), ["arm", "leg", "limb"]);
        assert_eq!(body["url"], url);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_percent_encoded_expression_in_post() {
        let app = app().await;
        let encoded = encode(&format!("({SYS})concept<<limb"));
        assert_eq!(expand_codes(&app, &encoded).await, ["arm", "leg", "limb"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_distinct_expressions_do_not_share_cache() {
        let app = app().await;
        let a = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
        let b = expand_codes(&app, &format!("({SYS})head")).await;
        let a_again = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
        assert_eq!(a, ["arm", "leg", "limb"]);
        assert_eq!(b, ["head"]);
        assert_eq!(a, a_again);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_malformed_returns_400_with_position() {
        let app = app().await;
        let (status, body) = expand_with(&app, &format!("({SYS})<<"), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["resourceType"], "OperationOutcome");
        let text = issue_text(&body);
        assert!(
            text.contains("Invalid VCL expression at position"),
            "{text}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_mixed_operators_without_parentheses_is_400() {
        let app = app().await;
        let (status, body) = expand_with(&app, &format!("({SYS})(a,b;c)"), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_missing_system_is_400() {
        let app = app().await;
        let (status, body) = expand_with(&app, "arm", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_expand_unsupported_construct_is_501() {
        let app = app().await;
        let (status, body) = expand_with(&app, &format!("({SYS})concept!!<body"), &[]).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    }

    // ── $validate-code ─────────────────────────────────────────────────────────

    async fn validate(app: &TestAppPg, expr: &str, code: &str) -> (StatusCode, Value) {
        let url = vcl_url(expr);
        app.post_fhir(
            "/ValueSet/$validate-code",
            TestAppPg::params(&[
                ("url", "valueUri", url.as_str()),
                ("system", "valueUri", SYS),
                ("code", "valueCode", code),
            ]),
        )
        .await
    }

    fn result_of(body: &Value) -> Option<bool> {
        body["parameter"]
            .as_array()?
            .iter()
            .find(|p| p["name"] == "result")?["valueBoolean"]
            .as_bool()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_validate_code_member_and_non_member() {
        let app = app().await;
        let expr = format!("({SYS})concept<<limb");
        let (status, body) = validate(&app, &expr, "arm").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(result_of(&body), Some(true), "{body}");

        let (status, body) = validate(&app, &expr, "head").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(result_of(&body), Some(false), "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_validate_code_honours_exclusion() {
        let app = app().await;
        let expr = format!("({SYS})<<body - ({SYS})<<limb");
        let (_, body) = validate(&app, &expr, "head").await;
        assert_eq!(result_of(&body), Some(true), "{body}");
        let (_, body) = validate(&app, &expr, "arm").await;
        assert_eq!(result_of(&body), Some(false), "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_validate_code_unresolvable_nested_value_set_is_false() {
        let app = app().await;
        let (status, body) = validate(&app, "^(http://hts.test/vs/does-not-exist)", "arm").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(result_of(&body), Some(false), "{body}");
        assert!(
            body.to_string().contains("Unable_to_resolve_value_Set_"),
            "{body}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_validate_code_malformed_is_400() {
        let app = app().await;
        let (status, body) = validate(&app, &format!("({SYS})<<"), "arm").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // ── $batch-validate-code ───────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn vcl_batch_validate_code_uses_batch_level_url() {
        let app = app().await;
        let url = vcl_url(&format!("({SYS})concept<<limb"));
        let validation = |code: &str| {
            json!({
                "name": "validation",
                "resource": {
                    "resourceType": "Parameters",
                    "parameter": [
                        { "name": "system", "valueUri": SYS },
                        { "name": "code", "valueCode": code }
                    ]
                }
            })
        };
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": url },
                validation("arm"),
                validation("head")
            ]
        });
        let (status, out) = app
            .post_fhir("/ValueSet/$batch-validate-code", body.to_string())
            .await;
        assert_eq!(status, StatusCode::OK, "{out}");
        let results: Vec<Option<bool>> = out["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| result_of(&v["resource"]))
            .collect();
        assert_eq!(results, [Some(true), Some(false)], "{out}");
    }
}
