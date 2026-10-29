//! #2287: a reconciliation run compares the primary with a search secondary
//! per resource type, by id and version, and reports what is missing, extra
//! and stale. Drift is injected by writing to one side only, so each category
//! is a fact of the test rather than a race.
//!
//! The secondary in the SQLite tests is a second SQLite backend; the
//! `es_integration` module repeats the drift against real Elasticsearch
//! (Docker required, `--features elasticsearch`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, ReconcileOptions,
    ReconcileScan, ReconciliationResult, RepairMode, SecondarySyncFailureLedger, SyncConfig,
    SyncMode,
};
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::error::{BackendError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::{Value, json};

const TYPE: &str = "Organization";

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

/// File-backed: the in-memory database shares its cache and locks tables
/// when a writer and a scan overlap.
fn sqlite(path: &std::path::Path) -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace data dir");
    let backend = SqliteBackend::with_config(
        path.to_str().expect("utf-8 path"),
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("sqlite");
    backend.init_schema().expect("schema");
    backend
}

fn org(name: &str) -> Value {
    json!({"resourceType": TYPE, "name": name})
}

fn opts(repair: RepairMode) -> ReconcileOptions {
    ReconcileOptions {
        page_size: 2,
        inter_page_delay: Duration::ZERO,
        repair,
        ..Default::default()
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    composite: Arc<CompositeStorage>,
    primary: Arc<SqliteBackend>,
    index: Arc<SqliteBackend>,
}

fn rig(with_scan: bool) -> Rig {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut primary = sqlite(&dir.path().join("primary.db"));
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = Arc::new(sqlite(&dir.path().join("index.db")));

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .with_sync_config(SyncConfig {
            mode: SyncMode::Synchronous,
            ..Default::default()
        })
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index.clone() as DynSearchProvider);

    let mut composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary.clone())
        .with_sync_failure_ledger(primary.clone());
    if with_scan {
        let mut scans: HashMap<String, Arc<dyn ReconcileScan>> = HashMap::new();
        scans.insert("search".to_string(), index.clone());
        composite = composite.with_reconcile_scan(primary.clone(), scans);
    }
    Rig {
        _dir: dir,
        composite: Arc::new(composite.start_sync_workers()),
        primary,
        index,
    }
}

async fn put(side: &SqliteBackend, id: &str, name: &str) {
    side.create_or_update(&tenant(), TYPE, id, org(name), FhirVersion::default())
        .await
        .expect("write");
}

/// ok-1 in sync; m-1, m-2 only on the primary; x-1 only on the index; s-1 on
/// both with the primary one version ahead.
async fn inject_drift(rig: &Rig) {
    put(&rig.primary, "ok-1", "a").await;
    put(&rig.index, "ok-1", "a").await;
    put(&rig.primary, "m-1", "a").await;
    put(&rig.primary, "m-2", "a").await;
    put(&rig.index, "x-1", "a").await;
    put(&rig.primary, "s-1", "a").await;
    put(&rig.index, "s-1", "a").await;
    put(&rig.primary, "s-1", "b").await;
}

fn assert_drift(result: &ReconciliationResult) {
    assert!(result.completed);
    assert_eq!(result.resource_type, TYPE);
    assert_eq!(result.missing.count, 2);
    assert_eq!(result.missing.sample, vec!["m-1", "m-2"]);
    assert_eq!(result.extra.count, 1);
    assert_eq!(result.extra.sample, vec!["x-1"]);
    assert_eq!(result.stale.count, 1);
    assert_eq!(result.stale.sample, vec!["s-1"]);
    assert_eq!(result.differences, 4);
    assert_eq!(result.primary_scanned, 4);
    assert_eq!(result.secondary_scanned, 3);
}

async fn run(rig: &Rig, repair: RepairMode) -> Vec<ReconciliationResult> {
    rig.composite
        .reconcile_secondary(&tenant(), "search", None, &opts(repair))
        .await
        .expect("reconcile")
}

#[tokio::test]
async fn reports_missing_extra_and_stale_per_type() {
    let rig = rig(true);
    inject_drift(&rig).await;

    let results = run(&rig, RepairMode::None).await;

    assert_eq!(results.len(), 1);
    assert_drift(&results[0]);
    assert_eq!(results[0].repaired + results[0].queued, 0);
    // Report-only changed nothing.
    assert!(
        rig.index
            .read(&tenant(), TYPE, "m-1")
            .await
            .unwrap()
            .is_none()
    );
    let again = run(&rig, RepairMode::None).await;
    assert_eq!(again[0].differences, 4);
}

#[tokio::test]
async fn deleted_resources_are_not_drift() {
    let rig = rig(true);
    put(&rig.primary, "gone", "a").await;
    put(&rig.index, "gone", "a").await;
    rig.primary.delete(&tenant(), TYPE, "gone").await.unwrap();
    rig.index.delete(&tenant(), TYPE, "gone").await.unwrap();

    let results = run(&rig, RepairMode::None).await;
    assert!(results.iter().all(|r| r.differences == 0));
}

#[tokio::test]
async fn inline_repair_converges_and_is_idempotent() {
    let rig = rig(true);
    inject_drift(&rig).await;

    let first = run(&rig, RepairMode::Inline).await;
    assert_drift(&first[0]);
    assert_eq!(first[0].repaired, 4);
    assert_eq!(first[0].repair_failed, 0);

    let second = run(&rig, RepairMode::Inline).await;
    assert_eq!(second[0].differences, 0);
    assert_eq!(second[0].repaired, 0);

    assert!(
        rig.index
            .read(&tenant(), TYPE, "m-1")
            .await
            .unwrap()
            .is_some()
    );
    // The extra document was deleted: absent, or reported as gone.
    assert!(!matches!(
        rig.index.read(&tenant(), TYPE, "x-1").await,
        Ok(Some(_))
    ));
    let primary_version = rig.primary.read(&tenant(), TYPE, "s-1").await.unwrap();
    let index_version = rig.index.read(&tenant(), TYPE, "s-1").await.unwrap();
    assert_eq!(
        primary_version.unwrap().version_id(),
        index_version.unwrap().version_id()
    );
}

#[tokio::test]
async fn queue_fills_the_ledger_and_the_repair_job_drains_it() {
    let rig = rig(true);
    inject_drift(&rig).await;

    let first = run(&rig, RepairMode::Queue).await;
    assert_drift(&first[0]);
    assert_eq!(first[0].queued, 4);
    assert_eq!(rig.primary.count_sync_failures().await.unwrap(), 4);
    // Queueing alone repairs nothing.
    assert!(
        rig.index
            .read(&tenant(), TYPE, "m-1")
            .await
            .unwrap()
            .is_none()
    );

    let report = rig
        .composite
        .repair_secondary_sync_failures(100)
        .await
        .unwrap();
    assert_eq!(report.repaired, 4);
    assert_eq!(rig.primary.count_sync_failures().await.unwrap(), 0);

    let after = run(&rig, RepairMode::None).await;
    assert_eq!(after[0].differences, 0);
}

#[tokio::test]
async fn a_type_only_the_secondary_holds_is_reported_as_extra() {
    let rig = rig(true);
    rig.index
        .create_or_update(
            &tenant(),
            "Patient",
            "p-1",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let results = run(&rig, RepairMode::None).await;
    let patient = results
        .iter()
        .find(|r| r.resource_type == "Patient")
        .unwrap();
    assert_eq!(patient.extra.sample, vec!["p-1"]);
}

#[tokio::test]
async fn without_a_registered_scan_reconcile_is_unsupported() {
    let rig = rig(false);
    let err = rig
        .composite
        .reconcile_secondary(&tenant(), "search", None, &opts(RepairMode::None))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Backend(BackendError::UnsupportedCapability { .. })
    ));
}

#[tokio::test]
async fn writes_during_a_run_leave_no_drift_once_it_settles() {
    let rig = rig(true);
    for i in 0..30 {
        put(&rig.primary, &format!("seed-{i:02}"), "a").await;
    }

    let writer = {
        let composite = rig.composite.clone();
        tokio::spawn(async move {
            for i in 0..30 {
                composite
                    .create_or_update(
                        &tenant(),
                        TYPE,
                        &format!("live-{i:02}"),
                        org("b"),
                        FhirVersion::default(),
                    )
                    .await
                    .expect("composite write");
                tokio::task::yield_now().await;
            }
        })
    };
    // Repairs inline while writes land; each repair re-reads the primary.
    let during = run(&rig, RepairMode::Inline).await;
    writer.await.unwrap();
    assert_eq!(during[0].repair_failed, 0);

    let settled = run(&rig, RepairMode::Inline).await;
    let after = run(&rig, RepairMode::None).await;
    assert_eq!(after[0].differences, 0, "settled run: {settled:?}");
    assert_eq!(after[0].primary_scanned, 60);
    assert_eq!(after[0].secondary_scanned, 60);
}

#[cfg(feature = "elasticsearch")]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[cfg(feature = "elasticsearch")]
mod es_integration {
    use super::*;
    use helios_persistence::backends::elasticsearch::{
        ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
    };
    use helios_persistence::core::Backend;
    use helios_persistence::search::ReindexTarget;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use uuid::Uuid;

    const ES_IMAGE_TAG: &str = "7.17.29";

    /// SQLite primary, real Elasticsearch as the search secondary.
    #[tokio::test]
    async fn drift_against_real_elasticsearch_is_reported_and_repaired() {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = super::container_cleanup::with_cleanup_label(
            ElasticSearch::default()
                .with_tag(ES_IMAGE_TAG)
                .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                .with_label("github.run_id", &run_id)
                .with_startup_timeout(Duration::from_secs(300)),
        )
        .start()
        .await
        .expect("start Elasticsearch");
        let host = container.get_host().await.expect("ES host");
        let port = container.get_host_port_ipv4(9200).await.expect("ES port");

        let dir = tempfile::tempdir().expect("tempdir");
        let mut primary = sqlite(&dir.path().join("primary.db"));
        primary.set_search_offloaded(true);
        let primary = Arc::new(primary);
        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![format!("http://{host}:{port}")],
                    index_prefix: format!("hfs_{}", Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    write_refresh: WriteRefreshPolicy::True,
                    ..Default::default()
                },
                primary.tenant_registries().clone(),
            )
            .expect("ES backend"),
        );
        es.initialize().await.expect("initialize ES");

        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .with_sync_config(SyncConfig {
                mode: SyncMode::Synchronous,
                ..Default::default()
            })
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let mut scans: HashMap<String, Arc<dyn ReconcileScan>> = HashMap::new();
        scans.insert("es".to_string(), es.clone());
        let composite = CompositeStorage::new(config, backends)
            .expect("composite")
            .with_full_primary(primary.clone())
            .with_sync_failure_ledger(primary.clone())
            .with_reconcile_scan(primary.clone(), scans)
            .start_sync_workers();

        // Index a primary resource into ES only.
        let index = |id: &'static str| {
            let primary = primary.clone();
            let es = es.clone();
            async move {
                let stored = primary.read(&tenant(), TYPE, id).await.unwrap().unwrap();
                es.write_search_entries(&tenant(), &stored)
                    .await
                    .expect("index");
            }
        };
        put(&primary, "ok-1", "a").await;
        index("ok-1").await;
        put(&primary, "m-1", "a").await;
        put(&primary, "m-2", "a").await;
        put(&primary, "x-1", "a").await;
        index("x-1").await;
        primary.delete(&tenant(), TYPE, "x-1").await.unwrap();
        put(&primary, "s-1", "a").await;
        index("s-1").await;
        put(&primary, "s-1", "b").await;

        let reconcile = |repair| {
            let composite = &composite;
            async move {
                composite
                    .reconcile_secondary(&tenant(), "es", Some(&[TYPE.to_string()]), &opts(repair))
                    .await
                    .expect("reconcile")
            }
        };
        let first = reconcile(RepairMode::Inline).await;
        assert_drift(&first[0]);
        assert_eq!(first[0].repaired, 4);

        let second = reconcile(RepairMode::None).await;
        assert_eq!(second[0].differences, 0);
    }
}
