//! #945 L3: the time-scoped reindex source view
//! (`ReindexSource::with_updated_since`) that bounds the deferred rebuild
//! after a bulk-submit manifest to what the manifest wrote. Shared by every
//! backend whose source can scope by `last_updated`: SQLite, PostgreSQL and
//! MongoDB run it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::search::ReindexSource;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;

/// What the suite needs of a backend.
pub trait SinceBackend: ResourceStorage + ReindexSource + 'static {}

impl<B: ResourceStorage + ReindexSource + 'static> SinceBackend for B {}

fn new_tenant(prefix: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{prefix}-{}", uuid::Uuid::new_v4().simple())),
        TenantPermissions::full_access(),
    )
}

async fn create_patient<B: SinceBackend>(backend: &B, tenant: &TenantContext, id: &str) {
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "gender": "female"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
}

/// Every id the view's pages hand out for `resource_type`, walking
/// `limit` at a time.
async fn walk(
    source: &Arc<dyn ReindexSource>,
    tenant: &TenantContext,
    resource_type: &str,
    limit: u32,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let page = source
            .fetch_resources_page(tenant, resource_type, cursor.as_deref(), limit)
            .await
            .unwrap();
        ids.extend(page.resources.iter().map(|r| r.id().to_string()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return ids,
        }
    }
    panic!("the walk did not end: {ids:?}");
}

/// Resources stored before the floor are outside the view; resources created
/// or updated at or after it are inside, each once; another type and another
/// tenant are untouched; a floor in the future holds nothing and one in the
/// distant past holds everything.
pub async fn assert_since_view_bounds_counts_and_pages<B: SinceBackend>(backend: Arc<B>) {
    let tenant = new_tenant("since-view");
    for id in ["old-1", "old-2", "old-3"] {
        create_patient(backend.as_ref(), &tenant, id).await;
    }
    // Stamps are taken from the wall clock on write; leave clear air around
    // the floor so stamp precision cannot straddle it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let floor = Utc::now();
    tokio::time::sleep(Duration::from_millis(50)).await;
    for id in ["new-1", "new-2"] {
        create_patient(backend.as_ref(), &tenant, id).await;
    }
    // An update moves an old resource into the window.
    let old = backend
        .read(&tenant, "Patient", "old-2")
        .await
        .unwrap()
        .expect("old-2 stored");
    backend
        .update(
            &tenant,
            &old,
            json!({"resourceType": "Patient", "id": "old-2", "gender": "male"}),
        )
        .await
        .unwrap();
    // Another tenant writes inside the window too.
    let other = new_tenant("since-view-other");
    create_patient(backend.as_ref(), &other, "other-1").await;

    let view = backend.clone().with_updated_since(floor).unwrap();
    assert_eq!(view.count_resources(&tenant, "Patient").await.unwrap(), 3);
    assert_eq!(
        view.count_resources(&tenant, "Observation").await.unwrap(),
        0
    );
    for limit in [1, 2, 1000] {
        let walked = walk(&view, &tenant, "Patient", limit).await;
        assert_eq!(walked.len(), 3, "limit {limit}: {walked:?}");
        let walked: BTreeSet<String> = walked.into_iter().collect();
        let expected: BTreeSet<String> = ["new-1", "new-2", "old-2"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(walked, expected, "limit {limit}");
    }
    assert_eq!(walk(&view, &other, "Patient", 10).await, ["other-1"]);

    let future = backend
        .clone()
        .with_updated_since(Utc::now() + chrono::Duration::hours(1))
        .unwrap();
    assert_eq!(future.count_resources(&tenant, "Patient").await.unwrap(), 0);
    assert!(walk(&future, &tenant, "Patient", 10).await.is_empty());

    let past = backend
        .clone()
        .with_updated_since(floor - chrono::Duration::days(365))
        .unwrap();
    assert_eq!(past.count_resources(&tenant, "Patient").await.unwrap(), 5);
    assert_eq!(walk(&past, &tenant, "Patient", 2).await.len(), 5);
}
