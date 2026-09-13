//! Dashboard live-count recording for the REST write paths (#1078).
//!
//! Every handler that commits a change to a tenant's live resource count calls
//! one of these after the write succeeded, feeding the process-global
//! [`helios_observability::dashboard_counters`] the Home dashboard reads in
//! O(1). Unlike subscription events this is unconditional — it does not depend
//! on the `subscriptions` feature or on an engine being configured.
//!
//! Only real live-count changes are recorded: a create (or an update that
//! created, including one that brought a deleted resource back) is `+1`, a
//! delete that removed a live resource is `-1`, and everything else — a plain
//! update, a conditional create that matched, a delete of something already
//! gone — records nothing. The tenant key is always the request's
//! `TenantContext` id, which is how the dashboard provider keys its reads.
//!
//! The counters are an approximation that a background reconcile corrects; see
//! the module docs of [`helios_observability::dashboard_counters`].

use helios_observability::dashboard_counters;
use helios_persistence::tenant::TenantContext;

/// Records one committed create of `resource_type`.
pub(crate) fn created(tenant: &TenantContext, resource_type: &str) {
    dashboard_counters::record_created(tenant.tenant_id().as_str(), resource_type, 1);
}

/// Records a committed create-or-update: counts only when it created.
pub(crate) fn upserted(tenant: &TenantContext, resource_type: &str, created: bool) {
    if created {
        self::created(tenant, resource_type);
    }
}

/// Records `n` committed deletes of live `resource_type` resources.
pub(crate) fn deleted(tenant: &TenantContext, resource_type: &str, n: u64) {
    dashboard_counters::record_deleted(tenant.tenant_id().as_str(), resource_type, n);
}

/// Records a signed live-count change (`+n` created, `-n` deleted).
pub(crate) fn changed(tenant: &TenantContext, resource_type: &str, delta: i64) {
    dashboard_counters::global().record(
        tenant.tenant_id().as_str(),
        resource_type,
        delta,
        chrono::Utc::now(),
    );
}

/// Forgets the tenant's counters after its data was purged, so the dashboard
/// waits for a fresh reconcile instead of showing figures for erased data.
pub(crate) fn invalidated(tenant_id: &str) {
    dashboard_counters::invalidate_tenant(tenant_id);
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_observability::dashboard_counters::global;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use serde_json::json;

    use crate::ServerConfig;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    /// A tenant id no other test (in this or any parallel test) records into:
    /// the counters are process-global.
    fn unique_tenant(label: &str) -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!(
            "dash-counts-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn server() -> TestServer {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        let state = crate::AppState::new(Arc::new(backend), ServerConfig::for_testing());
        let app = crate::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("test server")
    }

    fn header(tenant: &str) -> HeaderValue {
        HeaderValue::from_str(tenant).expect("tenant header")
    }

    fn live(tenant: &str, resource_type: &str) -> i64 {
        global().live_delta(tenant, resource_type)
    }

    #[tokio::test]
    async fn create_records_plus_one() {
        let server = server();
        let tenant = unique_tenant("create");
        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Patient", "name": [{"family": "A"}]}))
            .await;
        response.assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1);

        // A refused create records nothing.
        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Observation"}))
            .await;
        assert!(response.status_code().is_client_error());
        assert_eq!(live(&tenant, "Patient"), 1);
        assert_eq!(live(&tenant, "Observation"), 0);
    }

    #[tokio::test]
    async fn conditional_create_that_matches_records_nothing() {
        let server = server();
        let tenant = unique_tenant("cond-create");
        let if_none_exist = HeaderName::from_static("if-none-exist");
        let body = json!({"resourceType": "Patient"});

        // No match: the conditional create creates.
        let first = server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(&tenant))
            .add_header(
                if_none_exist.clone(),
                HeaderValue::from_static("_id=dash-none"),
            )
            .json(&body)
            .await;
        first.assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1);
        let id = first.json::<serde_json::Value>()["id"]
            .as_str()
            .expect("id")
            .to_string();

        // A match answers 200 with the existing resource and writes nothing.
        let second = server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(&tenant))
            .add_header(
                if_none_exist,
                HeaderValue::from_str(&format!("_id={id}")).expect("header"),
            )
            .json(&body)
            .await;
        second.assert_status_ok();
        assert_eq!(
            live(&tenant, "Patient"),
            1,
            "the matched create wrote nothing"
        );
    }

    #[tokio::test]
    async fn update_counts_only_when_it_creates() {
        let server = server();
        let tenant = unique_tenant("update");
        let put = |family: &'static str| {
            server
                .put("/Patient/dash-p1")
                .add_header(X_TENANT_ID, header(&tenant))
                .json(&json!({"resourceType": "Patient", "id": "dash-p1", "name": [{"family": family}]}))
        };

        put("A").await.assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1, "update-as-create counts");

        put("B").await.assert_status_ok();
        assert_eq!(live(&tenant, "Patient"), 1, "a plain update does not");

        server
            .delete("/Patient/dash-p1")
            .add_header(X_TENANT_ID, header(&tenant))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(live(&tenant, "Patient"), 0);

        put("C").await.assert_status(StatusCode::CREATED);
        assert_eq!(
            live(&tenant, "Patient"),
            1,
            "resurrecting a deleted resource counts as a create"
        );
    }

    #[tokio::test]
    async fn delete_records_minus_one_only_for_a_live_resource() {
        let server = server();
        let tenant = unique_tenant("delete");
        let created = server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Patient"}))
            .await;
        created.assert_status(StatusCode::CREATED);
        let id = created.json::<serde_json::Value>()["id"]
            .as_str()
            .expect("id")
            .to_string();
        assert_eq!(live(&tenant, "Patient"), 1);

        server
            .delete(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, header(&tenant))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(live(&tenant, "Patient"), 0);

        // Already gone / never existed: no live-count change.
        let again = server
            .delete(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, header(&tenant))
            .await;
        assert!(!again.status_code().is_success());
        let missing = server
            .delete("/Patient/never-existed")
            .add_header(X_TENANT_ID, header(&tenant))
            .await;
        assert!(!missing.status_code().is_success());
        assert_eq!(live(&tenant, "Patient"), 0);
    }

    #[tokio::test]
    async fn conditional_delete_records_the_deleted_resource() {
        let server = server();
        let tenant = unique_tenant("cond-delete");
        server
            .put("/Patient/dash-cd")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Patient", "id": "dash-cd"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1);

        server
            .delete("/Patient?_id=dash-cd")
            .add_header(X_TENANT_ID, header(&tenant))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(live(&tenant, "Patient"), 0);

        // No match is a success that deletes nothing.
        server
            .delete("/Patient?_id=dash-cd")
            .add_header(X_TENANT_ID, header(&tenant))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(live(&tenant, "Patient"), 0);
    }

    #[tokio::test]
    async fn transaction_bundle_records_committed_entries() {
        let server = server();
        let tenant = unique_tenant("transaction");
        server
            .put("/Patient/dash-existing")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Patient", "id": "dash-existing"}))
            .await
            .assert_status(StatusCode::CREATED);
        server
            .put("/Observation/dash-doomed")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Observation", "id": "dash-doomed", "status": "final", "code": {"text": "x"}}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1);
        assert_eq!(live(&tenant, "Observation"), 1);

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "fullUrl": "urn:uuid:9d8a0a3e-0000-4000-8000-000000000001",
                    "resource": {"resourceType": "Patient"},
                    "request": {"method": "POST", "url": "Patient"}
                },
                {
                    "resource": {"resourceType": "Patient", "id": "dash-new"},
                    "request": {"method": "PUT", "url": "Patient/dash-new"}
                },
                {
                    "resource": {"resourceType": "Patient", "id": "dash-existing", "active": true},
                    "request": {"method": "PUT", "url": "Patient/dash-existing"}
                },
                {
                    "request": {"method": "DELETE", "url": "Observation/dash-doomed"}
                }
            ]
        });
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&bundle)
            .await;
        response.assert_status_ok();
        assert_eq!(
            live(&tenant, "Patient"),
            3,
            "POST and update-as-create count; the plain update does not"
        );
        assert_eq!(live(&tenant, "Observation"), 0);

        // A rolled-back transaction records nothing.
        let failing = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                {
                    "resource": {"resourceType": "Patient"},
                    "request": {"method": "POST", "url": "Patient"}
                },
                {
                    "request": {"method": "DELETE", "url": "Observation/never-existed"}
                }
            ]
        });
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&failing)
            .await;
        assert!(!response.status_code().is_success());
        assert_eq!(live(&tenant, "Patient"), 3);
    }

    #[tokio::test]
    async fn batch_bundle_records_each_successful_entry() {
        let server = server();
        let tenant = unique_tenant("batch");
        server
            .put("/Patient/dash-b-existing")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&json!({"resourceType": "Patient", "id": "dash-b-existing"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(live(&tenant, "Patient"), 1);

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "resource": {"resourceType": "Patient"},
                    "request": {"method": "POST", "url": "Patient"}
                },
                {
                    "resource": {"resourceType": "Patient", "id": "dash-b-new"},
                    "request": {"method": "PUT", "url": "Patient/dash-b-new"}
                },
                {
                    "resource": {"resourceType": "Patient", "id": "dash-b-existing", "active": true},
                    "request": {"method": "PUT", "url": "Patient/dash-b-existing"}
                },
                {
                    "request": {"method": "DELETE", "url": "Patient/dash-b-existing"}
                },
                {
                    "request": {"method": "DELETE", "url": "Patient/never-existed"}
                }
            ]
        });
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, header(&tenant))
            .json(&bundle)
            .await;
        response.assert_status_ok();
        // +1 POST, +1 update-as-create, 0 update, -1 delete, 0 failed delete.
        assert_eq!(live(&tenant, "Patient"), 2);
    }
}
