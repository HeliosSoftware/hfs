//! #1624: a `$reindex` named by resource IDs with `clearExisting` clears and
//! rebuilds only those resources; every other resource of the same type, other
//! types and other tenants stay searchable.

use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::{
    ReindexOperation, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget, ResourceRef,
    TenantSearchRegistries,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};
use serde_json::json;

pub async fn assert_resource_scoped_clear<B>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
) where
    B: ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static,
{
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tenant = TenantContext::new(
        TenantId::new(format!("clear-ids-{suffix}")),
        TenantPermissions::full_access(),
    );
    let other = TenantContext::new(
        TenantId::new(format!("clear-ids-other-{suffix}")),
        TenantPermissions::full_access(),
    );
    for tenant in [&tenant, &other] {
        for (resource_type, resource) in [
            (
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "gender": "female"}),
            ),
            (
                "Patient",
                json!({"resourceType": "Patient", "id": "p2", "gender": "female"}),
            ),
            (
                "Observation",
                json!({"resourceType": "Observation", "id": "o", "status": "final", "code": {"text": "scope test"}}),
            ),
        ] {
            backend
                .create(tenant, resource_type, resource, FhirVersion::default())
                .await
                .unwrap();
        }
    }

    let query = |resource_type, parameter: &str, value| {
        SearchQuery::new(resource_type).with_parameter(SearchParameter {
            name: parameter.to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        })
    };
    let patients = query("Patient", "gender", "female");
    let observations = query("Observation", "status", "final");
    let found = |tenant: TenantContext, query: SearchQuery| {
        let backend = backend.clone();
        async move {
            let mut ids: Vec<String> = backend
                .search(&tenant, &query)
                .await
                .unwrap()
                .resources
                .items
                .iter()
                .map(|resource| resource.id().to_string())
                .collect();
            ids.sort();
            ids
        }
    };

    let op = ReindexOperation::with_parts(backend.clone(), vec![backend.clone()], registries);
    let job_id = op
        .start(
            tenant.clone(),
            ReindexRequest::for_resources([ResourceRef::new("Patient", "p1")]).clear_existing(),
            None,
        )
        .await
        .unwrap();
    let mut finished = None;
    for _ in 0..400 {
        let progress = op.get_progress(&job_id).await.unwrap();
        if progress.status.is_finished() {
            finished = Some(progress);
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
    }
    let progress = finished.expect("reindex did not finish");
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(progress.errors.is_empty(), "{:?}", progress.errors);
    assert_eq!(progress.processed_resources, 1);

    // The named Patient is rebuilt; its sibling, the Observation and the other
    // tenant were never cleared.
    assert_eq!(found(tenant.clone(), patients.clone()).await, ["p1", "p2"]);
    assert_eq!(found(tenant.clone(), observations.clone()).await, ["o"]);
    assert_eq!(found(other.clone(), patients).await, ["p1", "p2"]);
    assert_eq!(found(other, observations).await, ["o"]);
}
