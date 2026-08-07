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
