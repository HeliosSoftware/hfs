//! #1624: a type-scoped search-index clear removes only those types, only for
//! that tenant; an empty list clears nothing and `None` clears the tenant.

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::ReindexTarget;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};
use serde_json::json;

pub async fn assert_scoped_clear<B: ResourceStorage + SearchProvider + ReindexTarget>(backend: &B) {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tenant = TenantContext::new(
        TenantId::new(format!("clear-{suffix}")),
        TenantPermissions::full_access(),
    );
    let other = TenantContext::new(
        TenantId::new(format!("clear-other-{suffix}")),
        TenantPermissions::full_access(),
    );
    for tenant in [&tenant, &other] {
        for (resource_type, resource) in [
            (
                "Patient",
                json!({"resourceType": "Patient", "id": "p", "gender": "female"}),
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
    assert_eq!(
        backend
            .search(&tenant, &patients)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    assert_eq!(
        backend
            .search(&tenant, &observations)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    assert_eq!(
        backend
            .clear_search_index_for_types(&tenant, Some(&[]))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        backend
            .search(&tenant, &patients)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );

    let types = ["Patient".to_string()];
    assert!(
        backend
            .clear_search_index_for_types(&tenant, Some(&types))
            .await
            .unwrap()
            > 0
    );
    assert!(
        backend
            .search(&tenant, &patients)
            .await
            .unwrap()
            .resources
            .items
            .is_empty()
    );
    assert_eq!(
        backend
            .search(&tenant, &observations)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    assert_eq!(
        backend
            .search(&other, &patients)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    assert_eq!(
        backend
            .search(&other, &observations)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    assert_eq!(
        backend
            .clear_search_index_for_types(&tenant, Some(&types))
            .await
            .unwrap(),
        0
    );

    assert!(
        backend
            .clear_search_index_for_types(&tenant, None)
            .await
            .unwrap()
            > 0
    );
    assert!(
        backend
            .search(&tenant, &observations)
            .await
            .unwrap()
            .resources
            .items
            .is_empty()
    );
    assert_eq!(
        backend
            .search(&other, &observations)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
}
