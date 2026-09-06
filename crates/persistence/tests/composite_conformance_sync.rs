//! #462: on the sqlite-elasticsearch composite, SearchParameters reach the
//! search secondary but CompartmentDefinitions never do. This differential
//! test drives the same seeding path over a composite whose secondary is a
//! second SQLite backend: if the generic composite/sync plumbing loses the
//! CompartmentDefinitions, it reproduces here without an ES container.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage, SyncMode};
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace data dir")
}

#[tokio::test]
async fn conformance_seeding_reaches_the_search_secondary() {
    let primary = Arc::new(SqliteBackend::in_memory().expect("primary"));
    primary.init_schema().expect("primary schema");
    let secondary = Arc::new(SqliteBackend::in_memory().expect("secondary"));
    secondary.init_schema().expect("secondary schema");

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Asynchronous)
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), secondary.clone() as DynStorage);

    let composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .start_sync_workers();

    // The exact call the server makes at startup and on tenant provisioning.
    helios_persistence::search::seed_tenant_conformance(
        &composite,
        FhirVersion::R4,
        &data_dir(),
        "default",
    )
    .await;

    // Primary holds both sets.
    let t = tenant();
    let primary_sp = primary.count(&t, Some("SearchParameter")).await.unwrap();
    let primary_cd = primary
        .count(&t, Some("CompartmentDefinition"))
        .await
        .unwrap();
    assert!(primary_sp > 1000, "primary SPs: {primary_sp}");
    assert!(primary_cd >= 5, "primary CDs: {primary_cd}");

    // The async worker must drain both sets to the secondary. Poll: the sync
    // queue is bounded and the worker batches.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let (mut sp, mut cd) = (0, 0);
    while std::time::Instant::now() < deadline {
        sp = secondary.count(&t, Some("SearchParameter")).await.unwrap();
        cd = secondary
            .count(&t, Some("CompartmentDefinition"))
            .await
            .unwrap();
        if sp == primary_sp && cd == primary_cd {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(sp, primary_sp, "secondary SearchParameters after sync");
    assert_eq!(
        cd, primary_cd,
        "secondary CompartmentDefinitions after sync (#462: these were the ones going missing)"
    );
}

/// Synchronous sync is the mode the Inferno legs run in, and the one where the
/// seed's writes each waited on the secondary: seeding now goes through
/// `create_many`, which the composite forwards to the secondary as a batch
/// (`SyncManager::sync_creates`). Both sets must be in the secondary the
/// moment seeding returns — no worker to wait for — and a second seed must
/// find everything already there without writing a duplicate.
#[tokio::test]
async fn synchronous_batch_seeding_reaches_the_search_secondary() {
    let primary = Arc::new(SqliteBackend::in_memory().expect("primary"));
    primary.init_schema().expect("primary schema");
    let secondary = Arc::new(SqliteBackend::in_memory().expect("secondary"));
    secondary.init_schema().expect("secondary schema");

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), secondary.clone() as DynStorage);

    let composite = CompositeStorage::new(config, backends).expect("composite");

    let first = helios_persistence::search::seed_tenant_conformance(
        &composite,
        FhirVersion::R4,
        &data_dir(),
        "default",
    )
    .await;
    assert!(first.created > 1000, "first seed created: {first:?}");
    assert_eq!(first.failed, 0, "first seed: {first:?}");

    let t = tenant();
    for resource_type in ["SearchParameter", "CompartmentDefinition"] {
        let in_primary = primary.count(&t, Some(resource_type)).await.unwrap();
        let in_secondary = secondary.count(&t, Some(resource_type)).await.unwrap();
        assert!(in_primary > 0, "{resource_type} in primary");
        assert_eq!(
            in_secondary, in_primary,
            "{resource_type}: synchronous batch sync leaves the secondary complete on return"
        );
    }

    let second = helios_persistence::search::seed_tenant_conformance(
        &composite,
        FhirVersion::R4,
        &data_dir(),
        "default",
    )
    .await;
    assert_eq!(second.created, 0, "second seed: {second:?}");
    assert_eq!(second.failed, 0, "second seed: {second:?}");
    assert_eq!(
        second.existing,
        first.created + first.existing,
        "second seed finds every resource already present"
    );
    let sp_after = secondary.count(&t, Some("SearchParameter")).await.unwrap();
    assert_eq!(
        sp_after,
        primary.count(&t, Some("SearchParameter")).await.unwrap(),
        "re-seeding writes nothing new to the secondary"
    );
}

/// Per-item semantics survive the batch: the primary's `AlreadyExists` for a
/// duplicate id comes back in that item's slot, and only the resources the
/// primary accepted are synced to the secondary.
#[tokio::test]
async fn create_many_reports_per_item_outcomes_and_syncs_only_accepted_ones() {
    use serde_json::json;

    let primary = Arc::new(SqliteBackend::in_memory().expect("primary"));
    primary.init_schema().expect("primary schema");
    let secondary = Arc::new(SqliteBackend::in_memory().expect("secondary"));
    secondary.init_schema().expect("secondary schema");

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), secondary.clone() as DynStorage);
    let composite = CompositeStorage::new(config, backends).expect("composite");

    let t = tenant();
    composite
        .create(
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": "taken"}),
            FhirVersion::R4,
        )
        .await
        .expect("seed the duplicate");

    let results = composite
        .create_many(
            &t,
            "Patient",
            vec![
                json!({"resourceType": "Patient", "id": "fresh-1"}),
                json!({"resourceType": "Patient", "id": "taken"}),
                json!({"resourceType": "Patient", "id": "fresh-2"}),
            ],
            FhirVersion::R4,
        )
        .await;

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_ref().expect("fresh-1").id(), "fresh-1");
    assert!(
        matches!(
            results[1],
            Err(helios_persistence::error::StorageError::Resource(
                helios_persistence::error::ResourceError::AlreadyExists { .. }
            ))
        ),
        "duplicate id reports AlreadyExists in its own slot: {:?}",
        results[1]
    );
    assert_eq!(results[2].as_ref().expect("fresh-2").id(), "fresh-2");

    assert_eq!(primary.count(&t, Some("Patient")).await.unwrap(), 3);
    assert_eq!(
        secondary.count(&t, Some("Patient")).await.unwrap(),
        3,
        "the two accepted resources (plus the earlier one) reached the secondary"
    );
    assert!(
        secondary
            .read(&t, "Patient", "fresh-2")
            .await
            .unwrap()
            .is_some(),
        "an item after a rejected one is still synced"
    );
}
