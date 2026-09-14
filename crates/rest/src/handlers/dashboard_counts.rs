//! The dashboard's live resource counters as a post-commit write observer
//! (#1078).
//!
//! The write paths report every committed write to the server's
//! [`WriteObservers`](helios_persistence::core::WriteObservers) fan-out (see
//! [`crate::handlers::write_event`] and [`crate::WriteObservability`]); this
//! observer turns those events into [`DashboardCounters`] updates the Home
//! dashboard reads in O(1). It is subscribed unconditionally — it does not
//! depend on the `subscriptions` feature or on an engine being configured.
//!
//! - A single resource write records its `live_delta` (`+1` create, `-1`
//!   delete of a live resource; a plain update, a matched conditional create
//!   or a delete of something already gone report `0` and record nothing).
//! - An aggregate write (`$bulk-submit`, conformance seeding) records
//!   `+created` and `-deleted`; updates change no count.
//! - A purge of any scope marks the tenant's figures stale.
//! - A tenant deregistration drops the tenant's counters.
//!
//! The tenant key is the tenant id the write was made under, which is how the
//! dashboard provider keys its reads. The counters are an approximation that a
//! background reconcile corrects; see the module docs of
//! [`helios_observability::dashboard_counters`].

use std::sync::Arc;

use helios_observability::dashboard_counters::DashboardCounters;
use helios_persistence::core::{WriteEvent, WriteObserver};

/// `u64` → `i64`, saturating instead of wrapping.
fn saturating_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Records committed writes into the dashboard counters.
#[derive(Debug)]
pub struct DashboardCountsObserver {
    counters: Arc<DashboardCounters>,
}

impl DashboardCountsObserver {
    /// An observer recording into `counters`.
    pub fn new(counters: Arc<DashboardCounters>) -> Self {
        Self { counters }
    }
}

impl WriteObserver for DashboardCountsObserver {
    fn on_write(&self, event: &WriteEvent) {
        match event {
            WriteEvent::Resource(write) => {
                if write.live_delta != 0 {
                    self.counters.record(
                        write.tenant.as_str(),
                        &write.resource_type,
                        write.live_delta,
                        write.at,
                    );
                }
            }
            WriteEvent::Counts {
                tenant,
                resource_type,
                created,
                deleted,
                at,
                ..
            } => {
                // `record` ignores a zero delta.
                self.counters.record(
                    tenant.as_str(),
                    resource_type,
                    saturating_i64(*created),
                    *at,
                );
                self.counters.record(
                    tenant.as_str(),
                    resource_type,
                    -saturating_i64(*deleted),
                    *at,
                );
            }
            // The purge erased live rows *and* history, which the counters and
            // seeded history rings cannot subtract precisely: mark the tenant
            // stale so the background reconcile reseeds it.
            WriteEvent::Erased { tenant, .. } => self.counters.invalidate_tenant(tenant.as_str()),
            // The tenant was deregistered: drop its counters now rather than
            // waiting for idle eviction. The dashboard provider forgets its own
            // state for the tenant on its next pass, and a later view seeds
            // the tenant again from storage.
            WriteEvent::TenantRemoved { tenant } => {
                self.counters.remove_tenant(tenant.as_str());
            }
        }
    }
}

#[cfg(test)]
mod observer_tests {
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::{ErasedScope, ResourceWrite, WriteOrigin};
    use helios_persistence::tenant::TenantId;

    use super::*;

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id.to_string())
    }

    #[test]
    fn records_resource_deltas_counts_and_purges() {
        let counters = Arc::new(DashboardCounters::new());
        let observer = DashboardCountsObserver::new(Arc::clone(&counters));
        let resource = |t: &str, delta: i64| {
            WriteEvent::Resource(ResourceWrite {
                tenant: tenant(t),
                fhir_version: FhirVersion::default(),
                resource_type: "Patient".to_string(),
                live_delta: delta,
                notice: None,
                at: Utc::now(),
            })
        };

        observer.on_write(&resource("a", 1));
        observer.on_write(&resource("a", 0));
        observer.on_write(&resource("b", -1));
        assert_eq!(counters.live_delta("a", "Patient"), 1);
        assert_eq!(counters.live_delta("b", "Patient"), -1);

        observer.on_write(&WriteEvent::Counts {
            tenant: tenant("a"),
            resource_type: "Patient".to_string(),
            created: 5,
            updated: 7,
            deleted: 2,
            origin: WriteOrigin::BulkSubmit,
            at: Utc::now(),
        });
        assert_eq!(
            counters.live_delta("a", "Patient"),
            4,
            "+created −deleted; updates change no count"
        );

        for scope in [
            ErasedScope::Instance {
                resource_type: "Patient".to_string(),
                id: "p1".to_string(),
            },
            ErasedScope::Type("Patient".to_string()),
            ErasedScope::Tenant,
        ] {
            let other = Arc::new(DashboardCounters::new());
            other.record("a", "Patient", 1, Utc::now());
            DashboardCountsObserver::new(Arc::clone(&other)).on_write(&WriteEvent::Erased {
                tenant: tenant("a"),
                scope: scope.clone(),
            });
            assert!(other.needs_reseed("a"), "{scope:?}");
        }
        assert!(!counters.needs_reseed("a"));
    }

    /// A deregistered tenant's counters are dropped at once; other tenants
    /// keep theirs.
    #[test]
    fn tenant_removed_drops_the_tenants_counters() {
        let counters = Arc::new(DashboardCounters::new());
        let observer = DashboardCountsObserver::new(Arc::clone(&counters));
        counters.record("a", "Patient", 2, Utc::now());
        counters.record("b", "Patient", 1, Utc::now());

        observer.on_write(&WriteEvent::TenantRemoved {
            tenant: tenant("a"),
        });
        assert_eq!(counters.live_delta("a", "Patient"), 0);
        assert_eq!(counters.tenants(), vec!["b".to_string()]);

        // Removing a tenant with no state is harmless.
        observer.on_write(&WriteEvent::TenantRemoved {
            tenant: tenant("ghost"),
        });
        assert_eq!(counters.live_delta("b", "Patient"), 1);
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_observability::dashboard_counters::DashboardCounters;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::{
        ErasedScope, PurgableStorage, WriteEvent, WriteKind, WriteObserver,
    };
    use serde_json::json;

    use crate::ServerConfig;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const TENANT: &str = "dash-counts";
    const OTHER_TENANT: &str = "dash-counts-other";

    /// Every event reported to the state's observer, in order.
    #[derive(Default)]
    struct Recording {
        events: Mutex<Vec<WriteEvent>>,
    }

    impl WriteObserver for Recording {
        fn on_write(&self, event: &WriteEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    impl Recording {
        fn take(&self) -> Vec<WriteEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    struct Harness {
        server: TestServer,
        counters: Arc<DashboardCounters>,
        recording: Arc<Recording>,
    }

    fn harness() -> Harness {
        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");
        let state = crate::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_purge(backend as Arc<dyn PurgableStorage>);
        let recording = Arc::new(Recording::default());
        state.write_observer().subscribe(recording.clone());
        let counters = Arc::clone(state.dashboard_counters());
        let app = crate::routing::fhir_routes::create_routes(state);
        Harness {
            server: TestServer::new(app).expect("test server"),
            counters,
            recording,
        }
    }

    fn header(tenant: &str) -> HeaderValue {
        HeaderValue::from_str(tenant).expect("tenant header")
    }

    impl Harness {
        fn live(&self, tenant: &str, resource_type: &str) -> i64 {
            self.counters.live_delta(tenant, resource_type)
        }
    }

    /// `(resource_type, live_delta, notice kind, notice id)` of each resource
    /// event, and a label for the others.
    fn summarize(events: &[WriteEvent]) -> Vec<(String, i64, Option<WriteKind>, Option<String>)> {
        events
            .iter()
            .map(|event| match event {
                WriteEvent::Resource(write) => (
                    write.resource_type.clone(),
                    write.live_delta,
                    write.notice.as_ref().map(|n| n.kind),
                    write.notice.as_ref().map(|n| n.resource_id.clone()),
                ),
                other => (format!("{other:?}"), 0, None, None),
            })
            .collect()
    }

    #[tokio::test]
    async fn create_records_plus_one() {
        let h = harness();
        let response = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "name": [{"family": "A"}]}))
            .await;
        response.assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1);
        assert_eq!(h.live(OTHER_TENANT, "Patient"), 0, "tenants are separate");

        // A refused create records nothing.
        let response = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Observation"}))
            .await;
        assert!(response.status_code().is_client_error());
        assert_eq!(h.live(TENANT, "Patient"), 1);
        assert_eq!(h.live(TENANT, "Observation"), 0);
    }

    #[tokio::test]
    async fn conditional_create_that_matches_records_nothing() {
        let h = harness();
        let if_none_exist = HeaderName::from_static("if-none-exist");
        let body = json!({"resourceType": "Patient"});

        // No match: the conditional create creates.
        let first = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .add_header(
                if_none_exist.clone(),
                HeaderValue::from_static("_id=dash-none"),
            )
            .json(&body)
            .await;
        first.assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1);
        let id = first.json::<serde_json::Value>()["id"]
            .as_str()
            .expect("id")
            .to_string();

        // A match answers 200 with the existing resource and writes nothing.
        let second = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .add_header(
                if_none_exist,
                HeaderValue::from_str(&format!("_id={id}")).expect("header"),
            )
            .json(&body)
            .await;
        second.assert_status_ok();
        assert_eq!(
            h.live(TENANT, "Patient"),
            1,
            "the matched create wrote nothing"
        );
    }

    #[tokio::test]
    async fn update_counts_only_when_it_creates() {
        let h = harness();
        let put = |family: &'static str| {
            h.server
                .put("/Patient/dash-p1")
                .add_header(X_TENANT_ID, header(TENANT))
                .json(&json!({"resourceType": "Patient", "id": "dash-p1", "name": [{"family": family}]}))
        };

        put("A").await.assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1, "update-as-create counts");

        put("B").await.assert_status_ok();
        assert_eq!(h.live(TENANT, "Patient"), 1, "a plain update does not");

        h.server
            .delete("/Patient/dash-p1")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(h.live(TENANT, "Patient"), 0);

        put("C").await.assert_status(StatusCode::CREATED);
        assert_eq!(
            h.live(TENANT, "Patient"),
            1,
            "resurrecting a deleted resource counts as a create"
        );
    }

    #[tokio::test]
    async fn delete_records_minus_one_only_for_a_live_resource() {
        let h = harness();
        let created = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient"}))
            .await;
        created.assert_status(StatusCode::CREATED);
        let id = created.json::<serde_json::Value>()["id"]
            .as_str()
            .expect("id")
            .to_string();
        assert_eq!(h.live(TENANT, "Patient"), 1);

        h.server
            .delete(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(h.live(TENANT, "Patient"), 0);

        // Already gone / never existed: no live-count change.
        let again = h
            .server
            .delete(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, header(TENANT))
            .await;
        assert!(!again.status_code().is_success());
        let missing = h
            .server
            .delete("/Patient/never-existed")
            .add_header(X_TENANT_ID, header(TENANT))
            .await;
        assert!(!missing.status_code().is_success());
        assert_eq!(h.live(TENANT, "Patient"), 0);
    }

    #[tokio::test]
    async fn conditional_delete_records_the_deleted_resource() {
        let h = harness();
        h.server
            .put("/Patient/dash-cd")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "dash-cd"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1);

        h.server
            .delete("/Patient?_id=dash-cd")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(h.live(TENANT, "Patient"), 0);

        // No match is a success that deletes nothing.
        h.server
            .delete("/Patient?_id=dash-cd")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(h.live(TENANT, "Patient"), 0);
    }

    #[tokio::test]
    async fn transaction_bundle_records_committed_entries() {
        let h = harness();
        h.server
            .put("/Patient/dash-existing")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "dash-existing"}))
            .await
            .assert_status(StatusCode::CREATED);
        h.server
            .put("/Observation/dash-doomed")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Observation", "id": "dash-doomed", "status": "final", "code": {"text": "x"}}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1);
        assert_eq!(h.live(TENANT, "Observation"), 1);

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
        let response = h
            .server
            .post("/")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&bundle)
            .await;
        response.assert_status_ok();
        assert_eq!(
            h.live(TENANT, "Patient"),
            3,
            "POST and update-as-create count; the plain update does not"
        );
        assert_eq!(h.live(TENANT, "Observation"), 0);

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
        h.recording.take();
        let response = h
            .server
            .post("/")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&failing)
            .await;
        assert!(!response.status_code().is_success());
        assert_eq!(h.live(TENANT, "Patient"), 3);
        assert!(
            h.recording.take().is_empty(),
            "nothing committed, nothing reported"
        );
    }

    #[tokio::test]
    async fn batch_bundle_records_each_successful_entry() {
        let h = harness();
        h.server
            .put("/Patient/dash-b-existing")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "dash-b-existing"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(h.live(TENANT, "Patient"), 1);

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
        let response = h
            .server
            .post("/")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&bundle)
            .await;
        response.assert_status_ok();
        // +1 POST, +1 update-as-create, 0 update, -1 delete, 0 failed delete.
        assert_eq!(h.live(TENANT, "Patient"), 2);
    }

    // -- Observer-level: what the write paths report ------------------------

    #[tokio::test]
    async fn single_create_reports_plus_one_and_a_create_notice() {
        let h = harness();
        let created = h
            .server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient"}))
            .await;
        created.assert_status(StatusCode::CREATED);
        let body = created.json::<serde_json::Value>();
        let id = body["id"].as_str().expect("id").to_string();

        let events = h.recording.take();
        assert_eq!(events.len(), 1, "exactly one event per committed write");
        let WriteEvent::Resource(write) = &events[0] else {
            panic!("expected a resource event, got {:?}", events[0]);
        };
        assert_eq!(write.tenant.as_str(), TENANT);
        assert_eq!(write.resource_type, "Patient");
        assert_eq!(write.live_delta, 1);
        let notice = write.notice.as_ref().expect("a create announces");
        assert_eq!(notice.kind, WriteKind::Create);
        assert_eq!(notice.resource_id, id);
        assert_eq!(notice.version_id, "1");
        let resource = notice.resource.as_ref().expect("content");
        assert_eq!(resource["id"], json!(id));
        assert_eq!(
            resource["meta"]["versionId"],
            json!("1"),
            "content with meta"
        );
        assert!(notice.previous.is_none());
    }

    #[tokio::test]
    async fn plain_update_reports_no_delta_and_an_update_notice() {
        let h = harness();
        h.server
            .put("/Patient/obs-u1")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "obs-u1"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(
            summarize(&h.recording.take()),
            vec![(
                "Patient".to_string(),
                1,
                Some(WriteKind::Create),
                Some("obs-u1".to_string())
            )]
        );

        h.server
            .put("/Patient/obs-u1")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "obs-u1", "active": true}))
            .await
            .assert_status_ok();
        let events = h.recording.take();
        assert_eq!(
            summarize(&events),
            vec![(
                "Patient".to_string(),
                0,
                Some(WriteKind::Update),
                Some("obs-u1".to_string())
            )]
        );
        let WriteEvent::Resource(write) = &events[0] else {
            unreachable!()
        };
        assert_eq!(write.notice.as_ref().unwrap().version_id, "2");
    }

    #[tokio::test]
    async fn conditional_create_reports_plus_one_without_a_notice() {
        let h = harness();
        h.server
            .post("/Patient")
            .add_header(X_TENANT_ID, header(TENANT))
            .add_header(
                HeaderName::from_static("if-none-exist"),
                HeaderValue::from_static("_id=obs-none"),
            )
            .json(&json!({"resourceType": "Patient"}))
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(
            summarize(&h.recording.take()),
            vec![("Patient".to_string(), 1, None, None)],
            "conditional writes keep announcing nothing"
        );
    }

    #[tokio::test]
    async fn delete_reports_minus_one_and_a_delete_notice_with_previous() {
        let h = harness();
        h.server
            .put("/Patient/obs-d1")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "obs-d1", "active": true}))
            .await
            .assert_status(StatusCode::CREATED);
        h.recording.take();

        h.server
            .delete("/Patient/obs-d1")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let events = h.recording.take();
        assert_eq!(
            summarize(&events),
            vec![(
                "Patient".to_string(),
                -1,
                Some(WriteKind::Delete),
                Some("obs-d1".to_string())
            )]
        );
        let WriteEvent::Resource(write) = &events[0] else {
            unreachable!()
        };
        let notice = write.notice.as_ref().unwrap();
        assert!(notice.version_id.is_empty());
        assert!(notice.resource.is_none());
        assert_eq!(
            notice.previous.as_ref().expect("previous content")["active"],
            json!(true)
        );
    }

    /// The transaction path keeps deciding notices by HTTP status (#1023), so
    /// an `ifNoneExist` POST that matched (200, nothing written) still reports
    /// an Update notice — but its live delta, read from the entry's effect, is
    /// 0.
    #[tokio::test]
    async fn transaction_if_none_exist_match_reports_no_delta_and_an_update_notice() {
        let h = harness();
        h.server
            .put("/Patient/obs-t1")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&json!({"resourceType": "Patient", "id": "obs-t1"}))
            .await
            .assert_status(StatusCode::CREATED);
        h.recording.take();

        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:9d8a0a3e-0000-4000-8000-0000000000a1",
                "resource": {"resourceType": "Patient"},
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": "_id=obs-t1"}
            }]
        });
        h.server
            .post("/")
            .add_header(X_TENANT_ID, header(TENANT))
            .json(&bundle)
            .await
            .assert_status_ok();
        assert_eq!(
            summarize(&h.recording.take()),
            vec![(
                "Patient".to_string(),
                0,
                Some(WriteKind::Update),
                Some("obs-t1".to_string())
            )]
        );
        assert_eq!(h.live(TENANT, "Patient"), 1);
    }

    #[tokio::test]
    async fn purges_report_erased() {
        let h = harness();
        for id in ["obs-p1", "obs-p2"] {
            h.server
                .put(&format!("/Patient/{id}"))
                .add_header(X_TENANT_ID, header(TENANT))
                .json(&json!({"resourceType": "Patient", "id": id}))
                .await
                .assert_status(StatusCode::CREATED);
        }
        h.recording.take();

        h.server
            .delete("/Patient/obs-p1/$purge")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status_ok();
        let events = h.recording.take();
        assert!(
            matches!(
                events.as_slice(),
                [WriteEvent::Erased { tenant, scope: ErasedScope::Instance { resource_type, id } }]
                    if tenant.as_str() == TENANT && resource_type == "Patient" && id == "obs-p1"
            ),
            "{events:?}"
        );
        assert!(h.counters.needs_reseed(TENANT));

        h.server
            .post("/Patient/$purge")
            .add_header(X_TENANT_ID, header(TENANT))
            .await
            .assert_status_ok();
        let events = h.recording.take();
        assert!(
            matches!(
                events.as_slice(),
                [WriteEvent::Erased { tenant, scope: ErasedScope::Type(resource_type) }]
                    if tenant.as_str() == TENANT && resource_type == "Patient"
            ),
            "{events:?}"
        );
    }
}
