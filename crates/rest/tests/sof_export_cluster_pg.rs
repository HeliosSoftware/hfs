//! T2 cluster suite — SoF `$export` on the database-backed controller
//! (docs/cluster-testing-strategy.md §8 Phase 1, row A1).
//!
//! Two independently constructed `DatabaseExportJobController`s over one
//! shared Postgres container play "instance A" and "instance B"
//! (docs/cluster-testing-methodology.md §4 — fresh handles, never a cloned
//! `Arc`). The sink is one shared directory, standing in for the shared
//! object store the cluster validator requires. Work execution is driven
//! deterministically via `run_next_sof_export_job` (the worker pool's unit
//! of work) — no polling loops, no sleeps.

#![cfg(feature = "postgres")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::cluster_job_store::{ClusterJobStore, WorkerId};
use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::export::{
    DatabaseExportJobController, ExportJobController, ExportTask, ExportWork, FilesystemSink,
    JobStatus, NamedView, run_next_sof_export_job,
};

use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{Mutex, OnceCell};

struct SharedPg {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();
/// `claim_next` is deliberately cross-tenant, so tests sharing the container
/// must not run their claim/execute phases concurrently.
static CLUSTER_EXPORT_TEST_LOCK: Mutex<()> = Mutex::const_new(());
/// One shared output directory for the whole binary — the stand-in for the
/// shared object store; per-test job ids keep outputs disjoint.
static SHARED_EXPORT_DIR: OnceCell<tempfile::TempDir> = OnceCell::const_new();

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = Postgres::default()
                .with_label("github.run_id", &run_id)
                .start()
                .await
                .expect("failed to start PostgreSQL container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("failed to get host")
                .to_string();

            let backend = PostgresBackend::new(pg_config(&host, port))
                .await
                .expect("failed to create PostgresBackend");
            backend
                .init_schema()
                .await
                .expect("failed to initialize schema");

            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn pg_config(host: &str, port: u16) -> PostgresConfig {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(data_dir),
        ..Default::default()
    }
}

/// A fresh backend handle — one simulated `hfs` instance.
async fn create_backend() -> PostgresBackend {
    let pg = shared_pg().await;
    PostgresBackend::new(pg_config(&pg.host, pg.port))
        .await
        .expect("failed to create PostgresBackend")
}

fn create_tenant(id: &str) -> TenantContext {
    let unique = format!("{}_{}", id, uuid::Uuid::new_v4().simple());
    TenantContext::new(TenantId::new(&unique), TenantPermissions::full_access())
}

async fn shared_sink() -> FilesystemSink {
    let dir = SHARED_EXPORT_DIR
        .get_or_init(|| async { tempfile::tempdir().expect("create shared export dir") })
        .await;
    FilesystemSink::new(
        dir.path().to_str().expect("utf-8 temp path"),
        "http://front",
    )
}

/// One simulated instance: fresh backend handle + the seams the server wires
/// from it (job store, runner) + the shared sink.
struct Instance {
    backend: PostgresBackend,
    store: Arc<dyn ClusterJobStore>,
    runner: Arc<dyn SofRunner>,
    controller: DatabaseExportJobController<FilesystemSink>,
}

async fn instance() -> Instance {
    let backend = create_backend().await;
    let store = backend
        .cluster_job_store()
        .expect("postgres backs a cluster job store");
    let runner = backend.sof_runner().expect("postgres provides a SofRunner");
    let controller = DatabaseExportJobController::new(Arc::clone(&store), shared_sink().await);
    Instance {
        backend,
        store,
        runner,
        controller,
    }
}

fn patient_view() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [{
            "column": [
                {"path": "id", "name": "patient_id", "type": "string"}
            ]
        }]
    })
}

fn export_task(tenant: &TenantContext) -> ExportTask {
    ExportTask {
        work: ExportWork::Views(vec![NamedView {
            name: "patients".to_string(),
            view: patient_view(),
        }]),
        tenant: tenant.clone(),
        filters: ViewFilters::default(),
        format: "ndjson".to_string(),
        header: false,
        client_tracking_id: Some("cluster-t2".to_string()),
    }
}

/// DoD rows (A1): visibility + isolation + cross-instance execution — submit
/// on instance A, observe/poll on instance B, execute the work with B's
/// worker, download via B.
#[tokio::test]
async fn sof_export_cluster_submit_on_a_completes_and_downloads_on_b() {
    let _guard = CLUSTER_EXPORT_TEST_LOCK.lock().await;
    let a = instance().await;
    let b = instance().await;
    let tenant = create_tenant("sof-cluster");

    // Seed one patient so the export produces a shard.
    let created = a
        .backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Cluster"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("seed patient");

    let job_id = a.controller.submit(export_task(&tenant)).await;

    // Visibility: the job submitted on A is pollable on B immediately.
    match b
        .controller
        .get_status(tenant.tenant_id().as_str(), &job_id)
        .await
    {
        Some(JobStatus::Running { percent, .. }) => assert_eq!(percent, 0),
        other => panic!("job submitted on A must poll as running on B, got {other:?}"),
    }

    // Isolation: another tenant sees nothing, on either instance.
    assert!(
        b.controller
            .get_status(
                create_tenant("sof-cluster-other").tenant_id().as_str(),
                &job_id
            )
            .await
            .is_none(),
        "tenant isolation violated on poll"
    );

    // Instance B's worker claims and runs the job (deterministic single cycle).
    let ran = run_next_sof_export_job(
        &b.store,
        &b.runner,
        &shared_sink().await,
        500_000,
        &WorkerId::new(format!("t2-worker-{}", uuid::Uuid::new_v4())),
        Duration::from_secs(60),
    )
    .await
    .expect("worker cycle");
    assert!(ran, "B's worker must claim the queued job");

    // Both instances now see completion, with the manifest metadata intact.
    for (name, ctl) in [("A", &a.controller), ("B", &b.controller)] {
        match ctl.get_status(tenant.tenant_id().as_str(), &job_id).await {
            Some(JobStatus::Completed {
                files,
                format,
                client_tracking_id,
                ..
            }) => {
                assert_eq!(files.len(), 1, "instance {name}: one shard expected");
                assert_eq!(format, "ndjson");
                assert_eq!(client_tracking_id.as_deref(), Some("cluster-t2"));
            }
            other => panic!("instance {name}: expected Completed, got {other:?}"),
        }
    }

    // Download via B: URL resolves and the shard bytes contain the patient.
    let Some(JobStatus::Completed { files, .. }) = b
        .controller
        .get_status(tenant.tenant_id().as_str(), &job_id)
        .await
    else {
        unreachable!("checked above");
    };
    let filename = &files[0].filename;
    assert!(
        b.controller
            .download_url(tenant.tenant_id().as_str(), &job_id, filename)
            .await
            .is_some(),
        "download URL must resolve via instance B"
    );
    let bytes = b
        .controller
        .read_shard(tenant.tenant_id().as_str(), &job_id, filename)
        .await
        .expect("shard must be readable via instance B");
    let body = String::from_utf8(bytes).expect("ndjson shard is utf-8");
    assert!(
        body.contains(created.id()),
        "shard must contain the exported patient id"
    );

    // Isolation on the data path too.
    assert!(
        b.controller
            .read_shard(
                create_tenant("sof-cluster-other").tenant_id().as_str(),
                &job_id,
                filename
            )
            .await
            .is_none(),
        "tenant isolation violated on download"
    );
}

/// Cross-instance cancel: submitted on A, cancelled via B before any worker
/// runs — pollers on both instances see `Cancelled`, and no worker can claim
/// the job afterwards.
#[tokio::test]
async fn sof_export_cluster_cancel_via_b_prevents_execution() {
    let _guard = CLUSTER_EXPORT_TEST_LOCK.lock().await;
    let a = instance().await;
    let b = instance().await;
    let tenant = create_tenant("sof-cluster-cancel");

    let job_id = a.controller.submit(export_task(&tenant)).await;

    // Wrong tenant cannot cancel it (indistinguishable from missing)…
    assert!(
        !b.controller
            .cancel(
                create_tenant("sof-cluster-cancel-other")
                    .tenant_id()
                    .as_str(),
                &job_id
            )
            .await
    );
    // …the owner can, via the other instance.
    assert!(
        b.controller
            .cancel(tenant.tenant_id().as_str(), &job_id)
            .await
    );

    for (name, ctl) in [("A", &a.controller), ("B", &b.controller)] {
        match ctl.get_status(tenant.tenant_id().as_str(), &job_id).await {
            Some(JobStatus::Cancelled { .. }) => {}
            other => panic!("instance {name}: expected Cancelled, got {other:?}"),
        }
    }

    // No worker on any instance can claim the cancelled job.
    let ran = run_next_sof_export_job(
        &b.store,
        &b.runner,
        &shared_sink().await,
        500_000,
        &WorkerId::new(format!("t2-worker-{}", uuid::Uuid::new_v4())),
        Duration::from_secs(60),
    )
    .await
    .expect("worker cycle");
    assert!(!ran, "a cancelled job must not be claimable");
}
