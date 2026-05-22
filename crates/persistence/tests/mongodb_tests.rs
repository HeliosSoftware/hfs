//! MongoDB backend tests.
//!
//! Run compile-only/unit tests with:
//! `cargo test -p helios-persistence --features mongodb --test mongodb_tests`
//!
//! To run integration tests that hit a real MongoDB instance, set:
//! `HFS_TEST_MONGODB_URL=mongodb://localhost:27017`

#![cfg(feature = "mongodb")]

use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
use helios_persistence::core::{
    Backend, BackendCapability, BackendKind, BundleEntry, BundleMethod, BundleProvider,
    BundleResult, ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage,
    ConditionalUpdateResult, HistoryParams, IncludeProvider, InstanceHistoryProvider, PatchFormat,
    ResourceStorage, RevincludeProvider, SearchProvider, SystemHistoryProvider,
    TypeHistoryProvider, VersionedStorage,
};
use helios_persistence::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, TransactionError,
};
use helios_persistence::search::SearchParameterStatus;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    IncludeDirective, IncludeType, SearchParamType, SearchParameter, SearchQuery, SearchValue,
    SortDirective,
};
use mongodb::Client;
use mongodb::bson::{Document, doc};
use serde_json::json;

const MONGODB_MAX_DATABASE_NAME_LEN: usize = 63;
const MONGODB_TEST_DB_PREFIX: &str = "hfs_phase2_mongo_";

fn build_test_database_name(test_name: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let reserved_len = MONGODB_TEST_DB_PREFIX.len() + 1 + suffix.len();
    let max_test_name_len = MONGODB_MAX_DATABASE_NAME_LEN.saturating_sub(reserved_len);
    let truncated_test_name: String = test_name.chars().take(max_test_name_len).collect();

    format!("{MONGODB_TEST_DB_PREFIX}{truncated_test_name}_{suffix}")
}

fn extract_resource_id_from_location(location: &str) -> String {
    let resource_path = location.split("/_history").next().unwrap_or(location);
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

#[test]
fn test_mongodb_config_defaults() {
    let config = MongoBackendConfig::default();
    assert_eq!(config.connection_string, "mongodb://localhost:27017");
    assert_eq!(config.database_name, "helios");
    assert_eq!(config.max_connections, 10);
    assert_eq!(config.connect_timeout_ms, 5000);
    assert!(!config.search_offloaded);
    assert_eq!(config.fhir_version, FhirVersion::default());
}

#[test]
fn test_mongodb_config_serialization() {
    let config = MongoBackendConfig {
        connection_string: "mongodb://mongo.test:27018".to_string(),
        database_name: "phase2".to_string(),
        max_connections: 24,
        connect_timeout_ms: 7000,
        ..Default::default()
    };

    let serialized = serde_json::to_string(&config).unwrap();
    let decoded: MongoBackendConfig = serde_json::from_str(&serialized).unwrap();

    assert_eq!(decoded.connection_string, "mongodb://mongo.test:27018");
    assert_eq!(decoded.database_name, "phase2");
    assert_eq!(decoded.max_connections, 24);
    assert_eq!(decoded.connect_timeout_ms, 7000);
}

#[test]
fn test_mongodb_backend_kind_display() {
    assert_eq!(BackendKind::MongoDB.to_string(), "mongodb");
}

#[test]
fn test_mongodb_integration_database_name_within_limit() {
    let db_name = build_test_database_name("create_or_update");

    assert!(db_name.len() <= MONGODB_MAX_DATABASE_NAME_LEN);

    let (name_without_uuid, uuid_suffix) = db_name.rsplit_once('_').unwrap();
    assert!(name_without_uuid.starts_with(MONGODB_TEST_DB_PREFIX));
    assert_eq!(uuid_suffix.len(), 32);
}

#[test]
fn test_mongodb_phase4_capabilities() {
    let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();

    assert_eq!(backend.kind(), BackendKind::MongoDB);
    assert_eq!(backend.name(), "mongodb");

    assert!(backend.supports(BackendCapability::Crud));
    assert!(backend.supports(BackendCapability::Versioning));
    assert!(backend.supports(BackendCapability::InstanceHistory));
    assert!(backend.supports(BackendCapability::TypeHistory));
    assert!(backend.supports(BackendCapability::SystemHistory));
    assert!(backend.supports(BackendCapability::BasicSearch));
    assert!(backend.supports(BackendCapability::DateSearch));
    assert!(backend.supports(BackendCapability::ReferenceSearch));
    assert!(backend.supports(BackendCapability::Sorting));
    assert!(backend.supports(BackendCapability::OffsetPagination));
    assert!(backend.supports(BackendCapability::CursorPagination));
    assert!(backend.supports(BackendCapability::OptimisticLocking));
    assert!(backend.supports(BackendCapability::SharedSchema));

    assert!(backend.supports(BackendCapability::Transactions));
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_topology_behavior() {
    let Some(backend) = create_backend("bundle_topology_behavior").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_topology_behavior (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-topology");

    match backend.process_transaction(&tenant, vec![]).await {
        Ok(bundle_result) => assert!(bundle_result.entries.is_empty()),
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_topology_behavior (MongoDB topology does not support transactions)"
            );
        }
        Err(other) => panic!("Unexpected transaction result: {}", other),
    }
}

#[tokio::test]
async fn test_mongodb_bundle_provider_batch_not_supported() {
    let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
    let tenant = create_tenant("tenant-bundle-batch");

    let result = backend.process_batch(&tenant, vec![]).await;
    assert!(matches!(
        result,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
}

async fn process_transaction_or_skip(
    backend: &MongoBackend,
    tenant: &TenantContext,
    entries: Vec<BundleEntry>,
    test_name: &str,
) -> Option<BundleResult> {
    match backend.process_transaction(tenant, entries).await {
        Ok(result) => Some(result),
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping {} (MongoDB topology does not support transactions)",
                test_name
            );
            None
        }
        Err(e) => panic!("{} failed: {}", test_name, e),
    }
}

fn test_mongo_url() -> Option<String> {
    std::env::var("HFS_TEST_MONGODB_URL").ok()
}

fn create_tenant(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

async fn create_backend(test_name: &str) -> Option<MongoBackend> {
    create_backend_with_search_offloaded(test_name, false).await
}

async fn create_backend_with_search_offloaded(
    test_name: &str,
    search_offloaded: bool,
) -> Option<MongoBackend> {
    let connection_string = test_mongo_url()?;

    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        search_offloaded,
        ..Default::default()
    };

    let backend = MongoBackend::new(config)
        .expect("failed to create MongoBackend for mongodb integration tests");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema for integration tests");

    Some(backend)
}

async fn search_index_entry_count(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    resource_id: &str,
) -> u64 {
    let client = Client::with_uri_str(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for search_index assertions");
    let database = client.database(&backend.config().database_name);
    let search_index = database.collection::<Document>("search_index");

    search_index
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "resource_id": resource_id,
        })
        .await
        .expect("failed to count search_index entries")
}

async fn mongodb_total_created_connections(connection_string: &str) -> Option<i64> {
    let client = Client::with_uri_str(connection_string).await.ok()?;
    let status = client
        .database("admin")
        .run_command(doc! { "serverStatus": 1_i32 })
        .await
        .ok()?;
    let connections = status.get_document("connections").ok()?;

    connections
        .get_i64("totalCreated")
        .or_else(|_| connections.get_i32("totalCreated").map(i64::from))
        .ok()
}

#[tokio::test]
async fn mongodb_integration_create_read_update_delete() {
    let Some(backend) = create_backend("crud").await else {
        eprintln!(
            "Skipping mongodb_integration_create_read_update_delete (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Phase2"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());

    let updated = backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Updated"}]
            }),
        )
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Updated");

    backend
        .delete(&tenant, "Patient", updated.id())
        .await
        .unwrap();

    let read_after_delete = backend.read(&tenant, "Patient", updated.id()).await;
    assert!(matches!(
        read_after_delete,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reuses_client_pool_under_concurrent_read_search() {
    let Some(connection_string) = test_mongo_url() else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let config = MongoBackendConfig {
        connection_string: connection_string.clone(),
        database_name: build_test_database_name("client_pool_reuse"),
        max_connections: 8,
        ..Default::default()
    };
    let backend = MongoBackend::new(config).expect("failed to create MongoBackend");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema");

    let tenant = create_tenant("tenant-client-pool");
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-client-pool",
                "identifier": [{
                    "system": "http://hospital.org/mrn",
                    "value": "MRN-CLIENT-POOL"
                }],
                "name": [{ "family": "Pool" }],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let Some(before) = mongodb_total_created_connections(&connection_string).await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (serverStatus unavailable)"
        );
        return;
    };

    let backend = Arc::new(backend);
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let backend = backend.clone();
        let tenant = tenant.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..20 {
                backend
                    .read(&tenant, "Patient", "patient-client-pool")
                    .await
                    .unwrap();

                let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                    name: "identifier".to_string(),
                    param_type: SearchParamType::Token,
                    modifier: None,
                    values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-CLIENT-POOL")],
                    chain: vec![],
                    components: vec![],
                });
                backend.search(&tenant, &query).await.unwrap();
            }
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    let Some(after) = mongodb_total_created_connections(&connection_string).await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (serverStatus unavailable)"
        );
        return;
    };

    let created_during_test = after - before;
    assert!(
        created_during_test <= 50,
        "MongoDB backend should reuse one client pool; created {} connections during concurrent read/search",
        created_during_test
    );
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_create_and_resolve_references() {
    let Some(backend) = create_backend("bundle_create_resolve_references").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_create_and_resolve_references (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-resolve");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "BundleRefPatient"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-patient".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "subject": {"reference": "urn:uuid:new-patient"}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-observation".to_string()),
        },
    ];

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        entries,
        "mongodb_integration_transaction_bundle_create_and_resolve_references",
    )
    .await
    else {
        return;
    };

    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.entries[0].status, 201);
    assert_eq!(result.entries[1].status, 201);

    let patient_location = result.entries[0]
        .location
        .as_deref()
        .expect("patient location should be present");
    let expected_patient_reference = patient_location
        .split("/_history")
        .next()
        .unwrap_or(patient_location)
        .to_string();

    let observation_location = result.entries[1]
        .location
        .as_deref()
        .expect("observation location should be present");
    let observation_id = extract_resource_id_from_location(observation_location);

    let observation = backend
        .read(&tenant, "Observation", &observation_id)
        .await
        .unwrap()
        .unwrap();

    let resolved_reference = observation.content()["subject"]["reference"]
        .as_str()
        .expect("resolved subject reference should be present");

    assert_eq!(resolved_reference, expected_patient_reference);
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete() {
    let Some(backend) = create_backend("bundle_mixed_operations").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-mixed");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "update-me",
                "name": [{"family": "BeforeUpdate"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "delete-me",
                "name": [{"family": "BeforeDelete"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Delete,
            url: "Patient/delete-me".to_string(),
            resource: None,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "new-from-transaction",
                "name": [{"family": "CreatedInTransaction"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-created".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Put,
            url: "Patient/update-me".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "update-me",
                "name": [{"family": "AfterUpdate"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
    ];

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        entries,
        "mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete",
    )
    .await
    else {
        return;
    };

    assert_eq!(result.entries.len(), 3);
    assert_eq!(result.entries[0].status, 204);
    assert_eq!(result.entries[1].status, 201);
    assert_eq!(result.entries[2].status, 200);

    let updated = backend
        .read(&tenant, "Patient", "update-me")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.content()["name"][0]["family"], "AfterUpdate");

    let deleted = backend.read(&tenant, "Patient", "delete-me").await;
    assert!(matches!(
        deleted,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));

    let created = backend
        .read(&tenant, "Patient", "new-from-transaction")
        .await
        .unwrap();
    assert!(created.is_some());

    let idempotent_delete = vec![BundleEntry {
        method: BundleMethod::Delete,
        url: "Patient/non-existent-delete".to_string(),
        resource: None,
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let Some(idempotent_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        idempotent_delete,
        "mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete/idempotent",
    )
    .await
    else {
        return;
    };

    assert_eq!(idempotent_result.entries.len(), 1);
    assert_eq!(idempotent_result.entries[0].status, 204);
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_conditional_headers() {
    let Some(backend) = create_backend("bundle_conditional_headers").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_conditional_headers (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-conditional");

    let conditional_create = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN-TX-COND-1"}],
            "name": [{"family": "ConditionalCreate"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org/mrn|MRN-TX-COND-1".to_string()),
        full_url: Some("urn:uuid:conditional-create".to_string()),
    }];

    let Some(first_create) = process_transaction_or_skip(
        &backend,
        &tenant,
        conditional_create.clone(),
        "mongodb_integration_transaction_bundle_conditional_headers/create-first",
    )
    .await
    else {
        return;
    };
    assert_eq!(first_create.entries[0].status, 201);

    let Some(second_create) = process_transaction_or_skip(
        &backend,
        &tenant,
        conditional_create,
        "mongodb_integration_transaction_bundle_conditional_headers/create-second",
    )
    .await
    else {
        return;
    };
    assert_eq!(second_create.entries[0].status, 200);

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "if-match-target",
                "name": [{"family": "BeforeIfMatch"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let good_if_match = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/if-match-target".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "if-match-target",
            "name": [{"family": "AfterIfMatch"}]
        })),
        if_match: Some("W/\"1\"".to_string()),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let Some(good_if_match_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        good_if_match,
        "mongodb_integration_transaction_bundle_conditional_headers/if-match-good",
    )
    .await
    else {
        return;
    };
    assert_eq!(good_if_match_result.entries[0].status, 200);

    let bad_if_match = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/if-match-target".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "if-match-target",
            "name": [{"family": "ShouldNotPersist"}]
        })),
        if_match: Some("W/\"999\"".to_string()),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    match backend.process_transaction(&tenant, bad_if_match).await {
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_conditional_headers/if-match-bad (MongoDB topology does not support transactions)"
            );
            return;
        }
        Err(TransactionError::BundleError { .. }) => {}
        Err(other) => panic!("Unexpected transaction error: {}", other),
        Ok(_) => panic!("Expected if-match failure transaction to return BundleError"),
    }

    let read_after_bad_if_match = backend
        .read(&tenant, "Patient", "if-match-target")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read_after_bad_if_match.content()["name"][0]["family"],
        "AfterIfMatch"
    );
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_rolls_back_on_failure() {
    let Some(backend) = create_backend("bundle_rollback_failure").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_rolls_back_on_failure (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-rollback");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "already-exists",
                "name": [{"family": "PreExisting"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "should-rollback",
                "name": [{"family": "ShouldRollback"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:rollback-created".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "already-exists",
                "name": [{"family": "Duplicate"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:rollback-fail".to_string()),
        },
    ];

    match backend.process_transaction(&tenant, entries).await {
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_rolls_back_on_failure (MongoDB topology does not support transactions)"
            );
            return;
        }
        Err(TransactionError::BundleError { .. }) => {}
        Err(other) => panic!("Unexpected transaction error: {}", other),
        Ok(_) => panic!("Expected rollback scenario to fail transaction"),
    }

    let rolled_back = backend
        .read(&tenant, "Patient", "should-rollback")
        .await
        .unwrap();
    assert!(rolled_back.is_none());
}

#[tokio::test]
async fn mongodb_integration_tenant_isolation() {
    let Some(backend) = create_backend("tenant").await else {
        eprintln!("Skipping mongodb_integration_tenant_isolation (set HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let created = backend
        .create(
            &tenant_a,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "shared-id",
                "name": [{"family": "TenantA"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let read_a = backend
        .read(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_a.is_some());

    let read_b = backend
        .read(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_b.is_none());

    let exists_a = backend
        .exists(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    let exists_b = backend
        .exists(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(exists_a);
    assert!(!exists_b);
}

#[tokio::test]
async fn mongodb_integration_count_and_batch() {
    let Some(backend) = create_backend("count_batch").await else {
        eprintln!("Skipping mongodb_integration_count_and_batch (set HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant = create_tenant("tenant-count");

    let mut ids = Vec::new();
    for idx in 0..3 {
        let created = backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": format!("obs-{}", idx),
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        ids.push(created.id().to_string());
    }

    let count = backend.count(&tenant, Some("Observation")).await.unwrap();
    assert_eq!(count, 3);

    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let batch = backend
        .read_batch(&tenant, "Observation", &id_refs)
        .await
        .unwrap();
    assert_eq!(batch.len(), 3);
}

#[tokio::test]
async fn mongodb_integration_create_or_update() {
    let Some(backend) = create_backend("create_or_update").await else {
        eprintln!("Skipping mongodb_integration_create_or_update (set HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant = create_tenant("tenant-cou");

    let (created, was_created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-1",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "First"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(was_created);
    assert_eq!(created.version_id(), "1");

    let (updated, was_created_again) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-1",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Second"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(!was_created_again);
    assert_eq!(updated.version_id(), "2");
}

#[tokio::test]
async fn mongodb_integration_versioned_storage_vread_and_list_versions() {
    let Some(backend) = create_backend("versioned_vread").await else {
        eprintln!(
            "Skipping mongodb_integration_versioned_storage_vread_and_list_versions (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-versioned");

    let v1 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-v",
                "name": [{"family": "Version1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let v2 = backend
        .update(
            &tenant,
            &v1,
            json!({
                "resourceType": "Patient",
                "id": "patient-v",
                "name": [{"family": "Version2"}]
            }),
        )
        .await
        .unwrap();

    backend.delete(&tenant, "Patient", v2.id()).await.unwrap();

    let read_v1 = backend
        .vread(&tenant, "Patient", v1.id(), "1")
        .await
        .unwrap()
        .unwrap();
    let read_v2 = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap()
        .unwrap();
    let read_v3 = backend
        .vread(&tenant, "Patient", v1.id(), "3")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read_v1.version_id(), "1");
    assert_eq!(read_v1.content()["name"][0]["family"], "Version1");
    assert_eq!(read_v2.version_id(), "2");
    assert_eq!(read_v2.content()["name"][0]["family"], "Version2");
    assert_eq!(read_v3.version_id(), "3");
    assert!(read_v3.deleted_at().is_some());

    let versions = backend
        .list_versions(&tenant, "Patient", v1.id())
        .await
        .unwrap();
    assert_eq!(versions, vec!["1", "2", "3"]);
}

#[tokio::test]
async fn mongodb_integration_update_with_match_and_delete_with_match() {
    let Some(backend) = create_backend("if_match").await else {
        eprintln!(
            "Skipping mongodb_integration_update_with_match_and_delete_with_match (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-match");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-if-match",
                "name": [{"family": "Original"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            "W/\"1\"",
            json!({
                "resourceType": "Patient",
                "id": created.id(),
                "name": [{"family": "Updated"}]
            }),
        )
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Updated");

    let stale_update = backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            "1",
            json!({
                "resourceType": "Patient",
                "id": created.id(),
                "name": [{"family": "ShouldFail"}]
            }),
        )
        .await;

    assert!(matches!(
        stale_update,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));

    let stale_delete = backend
        .delete_with_match(&tenant, "Patient", created.id(), "1")
        .await;
    assert!(matches!(
        stale_delete,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));

    backend
        .delete_with_match(&tenant, "Patient", created.id(), "2")
        .await
        .unwrap();
}

#[tokio::test]
async fn mongodb_integration_history_providers() {
    let Some(backend) = create_backend("history_providers").await else {
        eprintln!("Skipping mongodb_integration_history_providers (set HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant = create_tenant("tenant-history");

    let patient_v1 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-history",
                "name": [{"family": "One"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let patient_v2 = backend
        .update(
            &tenant,
            &patient_v1,
            json!({
                "resourceType": "Patient",
                "id": "patient-history",
                "name": [{"family": "Two"}]
            }),
        )
        .await
        .unwrap();

    backend
        .delete(&tenant, "Patient", patient_v2.id())
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-history",
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let params = HistoryParams::new().count(20).include_deleted(true);

    let instance_history = backend
        .history_instance(&tenant, "Patient", patient_v1.id(), &params)
        .await
        .unwrap();
    assert_eq!(instance_history.items.len(), 3);
    assert_eq!(instance_history.items[0].resource.version_id(), "3");

    let type_history = backend
        .history_type(&tenant, "Patient", &params)
        .await
        .unwrap();
    assert!(type_history.items.len() >= 3);

    let system_history = backend.history_system(&tenant, &params).await.unwrap();
    assert!(system_history.items.len() >= 4);

    let instance_count = backend
        .history_instance_count(&tenant, "Patient", patient_v1.id())
        .await
        .unwrap();
    assert_eq!(instance_count, 3);

    let type_count = backend
        .history_type_count(&tenant, "Patient")
        .await
        .unwrap();
    assert_eq!(type_count, 3);

    let system_count = backend.history_system_count(&tenant).await.unwrap();
    assert!(system_count >= 4);
}

#[tokio::test]
async fn mongodb_integration_history_delete_trial_use_not_supported() {
    let Some(backend) = create_backend("history_delete_not_supported").await else {
        eprintln!(
            "Skipping mongodb_integration_history_delete_trial_use_not_supported (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-history-not-supported");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-history-delete",
                "name": [{"family": "TrialUse"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let delete_all_history = backend
        .delete_instance_history(&tenant, "Patient", created.id())
        .await;

    assert!(matches!(
        delete_all_history,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));

    let delete_single_version = backend
        .delete_version(&tenant, "Patient", created.id(), "1")
        .await;

    assert!(matches!(
        delete_single_version,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
}

#[tokio::test]
async fn mongodb_integration_search_token_string_and_offset_pagination() {
    let Some(backend) = create_backend("search_token_string").await else {
        eprintln!(
            "Skipping mongodb_integration_search_token_string_and_offset_pagination (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search");

    for (id, mrn, family) in [
        ("patient-search-1", "MRN-SEARCH-1", "Smith"),
        ("patient-search-2", "MRN-SEARCH-2", "Smiley"),
        ("patient-search-3", "MRN-SEARCH-3", "Jones"),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "identifier": [{"system": "http://hospital.org/mrn", "value": mrn}],
                    "name": [{"family": family}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let token_query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-SEARCH-1")],
        chain: vec![],
        components: vec![],
    });

    let token_result = backend.search(&tenant, &token_query).await.unwrap();
    assert_eq!(token_result.resources.items.len(), 1);
    assert_eq!(token_result.resources.items[0].id(), "patient-search-1");

    let mut string_query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smi")],
            chain: vec![],
            components: vec![],
        })
        .with_sort(SortDirective::parse("_id"))
        .with_count(1);

    let first_page = backend.search(&tenant, &string_query).await.unwrap();
    assert_eq!(first_page.resources.items.len(), 1);
    assert!(first_page.resources.page_info.has_next);
    let first_id = first_page.resources.items[0].id().to_string();

    string_query.offset = Some(1);
    let second_page = backend.search(&tenant, &string_query).await.unwrap();
    assert_eq!(second_page.resources.items.len(), 1);
    assert_ne!(second_page.resources.items[0].id(), first_id);
}

#[tokio::test]
async fn mongodb_integration_search_cursor_pagination_roundtrip() {
    let Some(backend) = create_backend("search_cursor_roundtrip").await else {
        eprintln!(
            "Skipping mongodb_integration_search_cursor_pagination_roundtrip (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-cursor");

    for id in ["patient-cursor-1", "patient-cursor-2", "patient-cursor-3"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{"family": format!("Cursor-{}", id)}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let query = SearchQuery::new("Patient").with_count(1);

    let page1 = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(page1.resources.items.len(), 1);
    assert!(page1.resources.page_info.has_next);
    assert!(!page1.resources.page_info.has_previous);

    let first_id = page1.resources.items[0].id().to_string();
    let next_cursor = page1
        .resources
        .page_info
        .next_cursor
        .clone()
        .expect("first page should include next cursor");

    let page2 = backend
        .search(&tenant, &query.clone().with_cursor(next_cursor))
        .await
        .unwrap();

    assert_eq!(page2.resources.items.len(), 1);
    assert!(page2.resources.page_info.has_previous);
    let second_id = page2.resources.items[0].id().to_string();
    assert_ne!(second_id, first_id);

    let previous_cursor = page2
        .resources
        .page_info
        .previous_cursor
        .clone()
        .expect("second page should include previous cursor");

    let page_back = backend
        .search(&tenant, &query.with_cursor(previous_cursor))
        .await
        .unwrap();

    assert_eq!(page_back.resources.items.len(), 1);
    assert_eq!(page_back.resources.items[0].id(), first_id.as_str());
}

#[tokio::test]
async fn mongodb_integration_conditional_create_exists() {
    let Some(backend) = create_backend("conditional_create").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_create_exists (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-create");

    let created = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-1"}],
                "name": [{"family": "Original"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-1",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let created_id = match created {
        ConditionalCreateResult::Created(resource) => resource.id().to_string(),
        other => panic!("expected Created result, got {:?}", other),
    };

    let second = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-1"}],
                "name": [{"family": "Duplicate"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-1",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    match second {
        ConditionalCreateResult::Exists(existing) => assert_eq!(existing.id(), created_id),
        other => panic!("expected Exists result, got {:?}", other),
    }
}

#[tokio::test]
async fn mongodb_integration_conditional_update_delete_and_no_match() {
    let Some(backend) = create_backend("conditional_update_delete").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_update_delete_and_no_match (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-update-delete");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-cond-update",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-UPDATE"}],
                "name": [{"family": "Before"}],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .conditional_update(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-cond-update",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-UPDATE"}],
                "name": [{"family": "After"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
            false,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated_id = match updated {
        ConditionalUpdateResult::Updated(resource) => {
            assert_eq!(resource.content()["name"][0]["family"], "After");
            resource.id().to_string()
        }
        other => panic!("expected Updated result, got {:?}", other),
    };

    let deleted = backend
        .conditional_delete(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
        )
        .await
        .unwrap();
    assert!(matches!(deleted, ConditionalDeleteResult::Deleted));

    let no_match = backend
        .conditional_delete(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
        )
        .await
        .unwrap();
    assert!(matches!(no_match, ConditionalDeleteResult::NoMatch));

    let read_after_delete = backend.read(&tenant, "Patient", &updated_id).await;
    assert!(matches!(
        read_after_delete,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test]
async fn mongodb_integration_conditional_create_multiple_matches() {
    let Some(backend) = create_backend("conditional_multiple_matches").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_create_multiple_matches (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-multi");

    for (id, system) in [
        ("patient-cond-multi-1", "http://system-a.org"),
        ("patient-cond-multi-2", "http://system-b.org"),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "identifier": [{"system": system, "value": "SHARED-VALUE"}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let result = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"value": "SHARED-VALUE"}],
            }),
            "identifier=SHARED-VALUE",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    match result {
        ConditionalCreateResult::MultipleMatches(count) => assert_eq!(count, 2),
        other => panic!("expected MultipleMatches result, got {:?}", other),
    }
}

#[tokio::test]
async fn mongodb_integration_conditional_patch_not_supported() {
    let Some(backend) = create_backend("conditional_patch_not_supported").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_patch_not_supported (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-patch");

    let result = backend
        .conditional_patch(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-PATCH",
            &PatchFormat::MergePatch(json!({ "active": true })),
        )
        .await;

    assert!(matches!(
        result,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
}

#[tokio::test]
async fn mongodb_integration_search_parameter_create_registers_active() {
    let Some(backend) = create_backend("search_param_create_active").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_create_registers_active (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-create-active");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-patient-nickname",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-patient-nickname",
                "name": "MongoPatientNickname",
                "status": "active",
                "code": "mongo-nickname",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.name.where(use='nickname').given"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let registry = backend.search_registry().read();
    let param = registry.get_param("Patient", "mongo-nickname");
    assert!(
        param.is_some(),
        "Active SearchParameter should be registered"
    );

    let param = param.unwrap();
    assert_eq!(
        param.url,
        "http://example.org/fhir/SearchParameter/mongo-custom-patient-nickname"
    );
    assert_eq!(param.status, SearchParameterStatus::Active);
}

#[tokio::test]
async fn mongodb_integration_search_parameter_create_draft_not_registered() {
    let Some(backend) = create_backend("search_param_create_draft").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_create_draft_not_registered (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-create-draft");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-draft-param",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-draft-param",
                "name": "MongoDraftParam",
                "status": "draft",
                "code": "mongo-draft",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.extension('draft')"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let registry = backend.search_registry().read();
    let param = registry.get_param("Patient", "mongo-draft");
    assert!(
        param.is_none(),
        "Draft SearchParameter should not be registered"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_update_status_change() {
    let Some(backend) = create_backend("search_param_update_status").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_update_status_change (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-update-status");

    let created = backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-status-change",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-status-change",
                "name": "MongoStatusChange",
                "status": "active",
                "code": "mongo-statuschange",
                "base": ["Condition"],
                "type": "token",
                "expression": "Condition.code"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let registry = backend.search_registry().read();
        let param = registry.get_param("Condition", "mongo-statuschange");
        assert!(
            param.is_some(),
            "Parameter should be registered after create"
        );
        assert_eq!(
            param.unwrap().status,
            SearchParameterStatus::Active,
            "Initial status should be active"
        );
    }

    backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-status-change",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-status-change",
                "name": "MongoStatusChange",
                "status": "retired",
                "code": "mongo-statuschange",
                "base": ["Condition"],
                "type": "token",
                "expression": "Condition.code"
            }),
        )
        .await
        .unwrap();

    let registry = backend.search_registry().read();
    let param = registry.get_param("Condition", "mongo-statuschange");
    assert!(param.is_some(), "Parameter should still exist in registry");
    assert_eq!(
        param.unwrap().status,
        SearchParameterStatus::Retired,
        "Status should be updated to retired"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_delete_unregisters() {
    let Some(backend) = create_backend("search_param_delete_unregister").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_delete_unregisters (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-delete");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-to-delete",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-to-delete",
                "name": "MongoToDelete",
                "status": "active",
                "code": "mongo-todelete",
                "base": ["Observation"],
                "type": "token",
                "expression": "Observation.code"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let registry = backend.search_registry().read();
        assert!(
            registry
                .get_param("Observation", "mongo-todelete")
                .is_some()
        );
    }

    backend
        .delete(&tenant, "SearchParameter", "mongo-custom-to-delete")
        .await
        .unwrap();

    let registry = backend.search_registry().read();
    assert!(
        registry
            .get_param("Observation", "mongo-todelete")
            .is_none(),
        "Deleted SearchParameter should be unregistered"
    );
}

#[tokio::test]
async fn mongodb_integration_search_offloaded_prevents_search_index_writes() {
    let Some(backend) =
        create_backend_with_search_offloaded("search_offloaded_no_index", true).await
    else {
        eprintln!(
            "Skipping mongodb_integration_search_offloaded_prevents_search_index_writes (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-offloaded");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "mongo-offloaded-patient",
                "name": [{"family": "Offloaded"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "OFFLOADED-1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let resource_id = created.id().to_string();

    let after_create = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_create, 0,
        "search_index should remain empty when search_offloaded=true (create)"
    );

    let updated = backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "Patient",
                "id": "mongo-offloaded-patient",
                "name": [{"family": "StillOffloaded"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "OFFLOADED-1"}]
            }),
        )
        .await
        .unwrap();

    let after_update = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_update, 0,
        "search_index should remain empty when search_offloaded=true (update)"
    );

    backend
        .delete(&tenant, "Patient", updated.id())
        .await
        .unwrap();

    let after_delete = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_delete, 0,
        "search_index should remain empty when search_offloaded=true (delete)"
    );
}

#[tokio::test]
async fn mongodb_integration_standalone_search_writes_search_index() {
    let Some(backend) = create_backend("search_index_written_standalone").await else {
        eprintln!(
            "Skipping mongodb_integration_standalone_search_writes_search_index (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-standalone");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "mongo-standalone-patient",
                "name": [{"family": "Indexed"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "INDEXED-1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let count = search_index_entry_count(&backend, &tenant, "Patient", created.id()).await;
    assert!(
        count > 0,
        "search_index should contain entries in standalone mode"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_registry_updates_when_offloaded() {
    let Some(backend) =
        create_backend_with_search_offloaded("search_param_offloaded_registry", true).await
    else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_registry_updates_when_offloaded (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-offloaded");

    let created = backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-offloaded-search-param",
                "url": "http://example.org/fhir/SearchParameter/mongo-offloaded-search-param",
                "name": "MongoOffloadedSearchParam",
                "status": "active",
                "code": "mongo-offloaded-code",
                "base": ["Patient"],
                "type": "token",
                "expression": "Patient.identifier"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let registry = backend.search_registry().read();
        let param = registry.get_param("Patient", "mongo-offloaded-code");
        assert!(
            param.is_some(),
            "Active SearchParameter should register when offloaded"
        );
        assert_eq!(param.unwrap().status, SearchParameterStatus::Active);
    }

    let search_index_count =
        search_index_entry_count(&backend, &tenant, "SearchParameter", created.id()).await;
    assert_eq!(
        search_index_count, 0,
        "SearchParameter resources should not write Mongo search_index when offloaded"
    );

    backend
        .delete(&tenant, "SearchParameter", created.id())
        .await
        .unwrap();

    let registry = backend.search_registry().read();
    assert!(
        registry
            .get_param("Patient", "mongo-offloaded-code")
            .is_none(),
        "Deleted SearchParameter should unregister when offloaded"
    );
}

#[tokio::test]
async fn mongodb_integration_resolve_include_and_revinclude() {
    let Some(connection_string) = test_mongo_url() else {
        eprintln!(
            "Skipping mongodb_integration_resolve_include_and_revinclude (set HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    // Point at the workspace-root spec file so that the registry knows about
    // Observation.subject — without it, no reference index entries get written
    // and revinclude resolution would have nothing to match.
    let workspace_data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name("includes"),
        data_dir: Some(workspace_data_dir),
        ..Default::default()
    };
    let backend = MongoBackend::new(config)
        .expect("failed to create MongoBackend for include/revinclude test");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema for include/revinclude test");

    let tenant = create_tenant("tenant-includes");

    let patient = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Includer"}],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let observation = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "subject": {"reference": format!("Patient/{}", patient.id())},
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let forward = IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    };
    let included = backend
        .resolve_includes(&tenant, std::slice::from_ref(&observation), &[forward])
        .await
        .expect("forward include resolution must succeed");
    assert_eq!(included.len(), 1, "exactly one Patient should be included");
    assert_eq!(included[0].resource_type(), "Patient");
    assert_eq!(included[0].id(), patient.id());

    let reverse = IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: None,
        iterate: false,
    };
    let revincluded = backend
        .resolve_revincludes(
            &tenant,
            std::slice::from_ref(&patient),
            std::slice::from_ref(&reverse),
        )
        .await
        .expect("revinclude resolution must succeed");
    assert_eq!(
        revincluded.len(),
        1,
        "exactly one Observation should be revincluded"
    );
    assert_eq!(revincluded[0].resource_type(), "Observation");
    assert_eq!(revincluded[0].id(), observation.id());

    let query = SearchQuery::new("Patient").with_include(reverse);
    let result = backend
        .search(&tenant, &query)
        .await
        .expect("search with _revinclude must not be rejected");
    assert!(
        result
            .resources
            .items
            .iter()
            .any(|r| r.resource_type() == "Patient" && r.id() == patient.id()),
        "primary results should still contain the Patient"
    );
    assert!(
        result
            .included
            .iter()
            .any(|r| r.resource_type() == "Observation" && r.id() == observation.id()),
        "search() should populate `included` from revinclude resolution"
    );
}
