//! Scenarios for #2554: the REST write paths (create, update, conditional
//! create and update, instance and conditional PATCH, batch and transaction
//! POST/PUT/PATCH entries) store a resource without its empty objects, empty
//! arrays and null members, and keep the `null` placeholders of primitive
//! arrays.
//!
//! Shared by `empty_elements_on_write.rs` (SQLite) and its MongoDB and
//! PostgreSQL twins. Each scenario takes a server whose tenant routing is
//! header-only.

#![allow(dead_code)]

use axum::http::{HeaderName, StatusCode};
use axum_test::TestServer;
use serde_json::{Value, json};

pub const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
pub const TENANT: &str = "test-tenant";

/// The workspace `data/` directory, for the spec SearchParameters.
pub fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| std::path::PathBuf::from("data"))
}

/// The issue's InsurancePlan, with an empty element of every kind.
fn plan_with_empties(identifier: &str) -> Value {
    json!({
        "resourceType": "InsurancePlan",
        "identifier": [{"system": "urn:test:2554", "value": identifier}],
        "status": "active",
        "type": [{}],
        "alias": [],
        "period": {},
        "name": null,
        "contact": [{"name": {"given": []}, "telecom": [null]}],
        "coverage": [{"type": {"coding": []}, "benefit": [{"type": {"text": "visit"}, "requirement": null}]}]
    })
}

/// What `plan_with_empties` is stored as.
fn clean_plan(identifier: &str) -> Value {
    json!({
        "resourceType": "InsurancePlan",
        "identifier": [{"system": "urn:test:2554", "value": identifier}],
        "status": "active",
        "coverage": [{"benefit": [{"type": {"text": "visit"}}]}]
    })
}

/// The body without the server-assigned `id` and `meta`.
fn content(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("id");
        object.remove("meta");
    }
    body
}

async fn read(server: &TestServer, path: &str) -> Value {
    let response = server.get(path).add_header(X_TENANT_ID, TENANT).await;
    response.assert_status_ok();
    response.json()
}

async fn put(server: &TestServer, path: &str, body: &Value) -> axum_test::TestResponse {
    server
        .put(path)
        .add_header(X_TENANT_ID, TENANT)
        .add_header("prefer", "return=representation")
        .json(body)
        .await
}

/// Create, read, history and search all return the cleaned body.
pub async fn create_stores_the_clean_body(server: &TestServer) {
    let response = server
        .post("/InsurancePlan")
        .add_header(X_TENANT_ID, TENANT)
        .add_header("prefer", "return=representation")
        .json(&plan_with_empties("create"))
        .await;
    response.assert_status(StatusCode::CREATED);
    let created: Value = response.json();
    assert_eq!(content(&created), clean_plan("create"), "{created:#}");
    let id = created["id"].as_str().expect("server id").to_string();

    let read_back = read(server, &format!("/InsurancePlan/{id}")).await;
    assert_eq!(content(&read_back), clean_plan("create"), "{read_back:#}");

    let history = read(server, &format!("/InsurancePlan/{id}/_history")).await;
    assert_eq!(
        content(&history["entry"][0]["resource"]),
        clean_plan("create"),
        "{history:#}"
    );

    let found = read(server, "/InsurancePlan?identifier=urn:test:2554|create").await;
    assert_eq!(
        content(&found["entry"][0]["resource"]),
        clean_plan("create"),
        "{found:#}"
    );
}

/// `PUT` that creates and `PUT` that updates both store the cleaned body,
/// and the first version stays clean.
pub async fn put_stores_the_clean_body(server: &TestServer) {
    let mut first = plan_with_empties("put");
    first["id"] = json!("plan-put");
    let response = put(server, "/InsurancePlan/plan-put", &first).await;
    response.assert_status(StatusCode::CREATED);
    let mut expected = clean_plan("put");
    assert_eq!(content(&response.json()), expected);

    let mut second = plan_with_empties("put");
    second["id"] = json!("plan-put");
    second["alias"] = json!(["Kept"]);
    let response = put(server, "/InsurancePlan/plan-put", &second).await;
    response.assert_status_ok();
    expected["alias"] = json!(["Kept"]);
    assert_eq!(content(&response.json()), expected);
    assert_eq!(
        content(&read(server, "/InsurancePlan/plan-put").await),
        expected
    );

    let first_version = read(server, "/InsurancePlan/plan-put/_history/1").await;
    assert_eq!(content(&first_version), clean_plan("put"));
}

/// Conditional create (`If-None-Exist`, no match) and conditional update store
/// the cleaned body.
pub async fn conditional_writes_store_the_clean_body(server: &TestServer) {
    let response = server
        .post("/InsurancePlan")
        .add_header(X_TENANT_ID, TENANT)
        .add_header("prefer", "return=representation")
        .add_header("if-none-exist", "identifier=urn:test:2554|conditional")
        .json(&plan_with_empties("conditional"))
        .await;
    response.assert_status(StatusCode::CREATED);
    let created: Value = response.json();
    assert_eq!(content(&created), clean_plan("conditional"));
    let id = created["id"].as_str().expect("server id").to_string();

    let mut update = plan_with_empties("conditional");
    update["status"] = json!("retired");
    let response = put(
        server,
        "/InsurancePlan?identifier=urn:test:2554|conditional",
        &update,
    )
    .await;
    response.assert_status_ok();
    let mut expected = clean_plan("conditional");
    expected["status"] = json!("retired");
    assert_eq!(content(&response.json()), expected);
    assert_eq!(
        content(&read(server, &format!("/InsurancePlan/{id}")).await),
        expected
    );
}

/// A PATCH that removes the last item of an array, or adds an empty value,
/// stores the resource without the element; on the instance and the
/// conditional endpoint.
pub async fn patch_stores_the_clean_result(server: &TestServer) {
    let mut plan = clean_plan("patch");
    plan["id"] = json!("plan-patch");
    plan["alias"] = json!(["Only"]);
    put(server, "/InsurancePlan/plan-patch", &plan)
        .await
        .assert_status(StatusCode::CREATED);

    let response = server
        .patch("/InsurancePlan/plan-patch")
        .add_header(X_TENANT_ID, TENANT)
        .add_header("content-type", "application/json-patch+json")
        .add_header("prefer", "return=representation")
        .bytes(
            json!([
                {"op": "remove", "path": "/alias/0"},
                {"op": "add", "path": "/type", "value": [{}]}
            ])
            .to_string()
            .into(),
        )
        .await;
    response.assert_status_ok();
    assert_eq!(content(&response.json()), clean_plan("patch"));
    assert_eq!(
        content(&read(server, "/InsurancePlan/plan-patch").await),
        clean_plan("patch")
    );

    let response = server
        .patch("/InsurancePlan?identifier=urn:test:2554|patch")
        .add_header(X_TENANT_ID, TENANT)
        .add_header("content-type", "application/json-patch+json")
        .add_header("prefer", "return=representation")
        .bytes(
            json!([{"op": "add", "path": "/period", "value": {}}])
                .to_string()
                .into(),
        )
        .await;
    response.assert_status_ok();
    assert_eq!(
        content(&read(server, "/InsurancePlan/plan-patch").await),
        clean_plan("patch")
    );
}

/// A FHIRPath Patch Parameters deleting `alias[0]`.
fn delete_first_alias() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "operation", "part": [
            {"name": "type", "valueCode": "delete"},
            {"name": "path", "valueString": "InsurancePlan.alias[0]"}
        ]}]
    })
}

/// POST, PUT and PATCH entries of a batch or a transaction store the cleaned
/// body, and the entry responses carry it.
async fn bundle_entries_store_the_clean_body(server: &TestServer, bundle_type: &str) {
    let suffix = bundle_type;
    let patch_id = format!("plan-{suffix}-patch");
    let mut target = clean_plan(&format!("{suffix}-patch"));
    target["id"] = json!(patch_id);
    target["alias"] = json!(["Only"]);
    put(server, &format!("/InsurancePlan/{patch_id}"), &target)
        .await
        .assert_status(StatusCode::CREATED);

    let put_id = format!("plan-{suffix}-put");
    let mut put_body = plan_with_empties(&format!("{suffix}-put"));
    put_body["id"] = json!(put_id);
    let bundle = json!({
        "resourceType": "Bundle",
        "type": bundle_type,
        "entry": [
            {
                "fullUrl": "urn:uuid:5a0c1d0e-2554-4f00-9000-000000000001",
                "resource": plan_with_empties(&format!("{suffix}-post")),
                "request": {"method": "POST", "url": "InsurancePlan"}
            },
            {
                "resource": put_body,
                "request": {"method": "PUT", "url": format!("InsurancePlan/{put_id}")}
            },
            {
                "resource": delete_first_alias(),
                "request": {"method": "PATCH", "url": format!("InsurancePlan/{patch_id}")}
            }
        ]
    });
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, TENANT)
        .add_header("prefer", "return=representation")
        .json(&bundle)
        .await;
    response.assert_status_ok();
    let result: Value = response.json();
    let entries = result["entry"].as_array().expect("response entries");
    assert_eq!(entries.len(), 3, "{result:#}");
    for entry in entries {
        let status = entry["response"]["status"].as_str().unwrap_or_default();
        assert!(
            status.starts_with("200") || status.starts_with("201"),
            "{result:#}"
        );
    }

    let post_id = entries[0]["response"]["location"]
        .as_str()
        .and_then(|location| location.split('/').nth(1))
        .expect("POST entry location")
        .to_string();
    let expected = [
        (
            format!("/InsurancePlan/{post_id}"),
            clean_plan(&format!("{suffix}-post")),
        ),
        (
            format!("/InsurancePlan/{put_id}"),
            clean_plan(&format!("{suffix}-put")),
        ),
        (
            format!("/InsurancePlan/{patch_id}"),
            clean_plan(&format!("{suffix}-patch")),
        ),
    ];
    for (index, (path, body)) in expected.iter().enumerate() {
        if let Some(returned) = entries[index].get("resource") {
            assert_eq!(&content(returned), body, "entry {index}: {result:#}");
        }
        assert_eq!(&content(&read(server, path).await), body, "{path}");
    }
}

pub async fn batch_entries_store_the_clean_body(server: &TestServer) {
    bundle_entries_store_the_clean_body(server, "batch").await;
}

pub async fn transaction_entries_store_the_clean_body(server: &TestServer) {
    bundle_entries_store_the_clean_body(server, "transaction").await;
}

/// The `null` placeholders that line `given` up with `_given` are kept, and an
/// item that has neither a value nor an extension is removed from both arrays.
pub async fn primitive_placeholders_are_kept(server: &TestServer) {
    let extension = json!({"extension": [{"url": "http://example.org/x", "valueString": "y"}]});
    let patient = json!({
        "resourceType": "Patient",
        "id": "p-2554",
        "name": [{"family": "Doe", "given": ["Ann", null], "_given": [null, extension]}]
    });
    put(server, "/Patient/p-2554", &patient)
        .await
        .assert_status(StatusCode::CREATED);
    assert_eq!(
        content(&read(server, "/Patient/p-2554").await),
        content(&patient)
    );

    let patient = json!({
        "resourceType": "Patient",
        "id": "p-2554",
        "name": [{
            "family": "Doe",
            "given": [null, "Ann", null],
            "_given": [{}, null, extension]
        }]
    });
    put(server, "/Patient/p-2554", &patient)
        .await
        .assert_status_ok();
    assert_eq!(
        content(&read(server, "/Patient/p-2554").await),
        json!({
            "resourceType": "Patient",
            "name": [{"family": "Doe", "given": ["Ann", null], "_given": [null, extension]}]
        })
    );
}

/// One test per scenario for a backend's `create_test_server`, which returns
/// `None` when that backend is not available (the test then reports
/// `Skipping`).
#[macro_export]
macro_rules! empty_elements_suite {
    ($create:ident) => {
        $crate::empty_elements_suite!(@one $create, create_stores_the_clean_body);
        $crate::empty_elements_suite!(@one $create, put_stores_the_clean_body);
        $crate::empty_elements_suite!(@one $create, conditional_writes_store_the_clean_body);
        $crate::empty_elements_suite!(@one $create, patch_stores_the_clean_result);
        $crate::empty_elements_suite!(@one $create, batch_entries_store_the_clean_body);
        $crate::empty_elements_suite!(@one $create, transaction_entries_store_the_clean_body);
        $crate::empty_elements_suite!(@one $create, primitive_placeholders_are_kept);
    };
    (@one $create:ident, $scenario:ident) => {
        #[tokio::test]
        async fn $scenario() {
            let Some(server) = $create(stringify!($scenario)).await else {
                eprintln!("Skipping {}: backend not available", stringify!($scenario));
                return;
            };
            $crate::empty_elements::$scenario(&server).await;
        }
    };
}
