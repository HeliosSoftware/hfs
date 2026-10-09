//! `GET /admin/tenants` and `GET /console/metrics/tenants` over a real S3 API
//! (MinIO) — #1913.
//!
//! Both endpoints read S3 tenant discovery (one delimiter page per 1,000
//! tenant groups plus one `MaxKeys=1` probe per group) instead of
//! `count_by_tenant`, which lists every current pointer of every tenant. On
//! the real SDK path this suite checks:
//!
//! - prefix-per-tenant reports presence (`resources: null`, `has_data`,
//!   `resources_evidence: "presence"`), never a pointer count;
//! - data-only tenants stay visible: one deregistered without purge, and one
//!   whose only objects are a delete tombstone and its history;
//! - a registered tenant without data is `has_data: false`, and a purged
//!   tenant drops out;
//! - bucket-per-tenant reports `resources_evidence: "unsupported"`, not an
//!   empty store.
//!
//! Requires Docker: MinIO runs via testcontainers, like the other
//! Docker-backed rest suites (`sof_export_s3_restart.rs`). There is no env
//! opt-in.

#![cfg(feature = "s3")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod tenant_inventory_minio_tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Once};

    use aws_config::{BehaviorVersion, Region};
    use aws_sdk_s3::config::Credentials;
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::{AppState, ServerConfig};
    use serde_json::{Value, json};
    use testcontainers::core::{IntoContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{GenericImage, ImageExt};
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    const DEFAULT_MINIO_IMAGE: &str = "ghcr.io/coollabsio/minio";
    const DEFAULT_MINIO_TAG: &str = "RELEASE.2025-10-15T17-29-55Z";
    const MINIO_ROOT_USER: &str = "minioadmin";
    const MINIO_ROOT_PASSWORD: &str = "minioadmin";

    struct SharedMinio {
        endpoint_url: String,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<GenericImage>,
    }

    static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();
    static BACKEND_ENV: Once = Once::new();

    async fn shared_minio() -> &'static SharedMinio {
        SHARED_MINIO
            .get_or_init(|| async {
                let image = std::env::var("MINIO_IMAGE")
                    .unwrap_or_else(|_| DEFAULT_MINIO_IMAGE.to_string());
                let tag =
                    std::env::var("MINIO_TAG").unwrap_or_else(|_| DEFAULT_MINIO_TAG.to_string());
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();

                let container = super::container_cleanup::with_cleanup_label(
                    GenericImage::new(image, tag)
                        .with_wait_for(WaitFor::message_on_stderr("API:"))
                        .with_exposed_port(9000.tcp())
                        .with_exposed_port(9001.tcp())
                        .with_env_var("MINIO_ROOT_USER", MINIO_ROOT_USER)
                        .with_env_var("MINIO_ROOT_PASSWORD", MINIO_ROOT_PASSWORD)
                        .with_env_var("MINIO_CONSOLE_ADDRESS", ":9001")
                        .with_cmd(["server", "/data", "--console-address", ":9001"])
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("failed to start MinIO container");

                let host = container
                    .get_host()
                    .await
                    .expect("failed to resolve MinIO host")
                    .to_string();
                let port = container
                    .get_host_port_ipv4(9000)
                    .await
                    .expect("failed to resolve MinIO API port");

                // `S3Backend::from_env_async` reads the SDK provider chain.
                BACKEND_ENV.call_once(|| {
                    // SAFETY: runs once, before any backend in this binary is
                    // built, and the values never change afterwards.
                    unsafe {
                        std::env::set_var("AWS_ACCESS_KEY_ID", MINIO_ROOT_USER);
                        std::env::set_var("AWS_SECRET_ACCESS_KEY", MINIO_ROOT_PASSWORD);
                        std::env::set_var("AWS_REGION", "us-east-1");
                        std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
                    }
                });

                SharedMinio {
                    endpoint_url: format!("http://{host}:{port}"),
                    _container: container,
                }
            })
            .await
    }

    async fn fresh_bucket(shared: &SharedMinio) -> String {
        let creds = Credentials::new(
            MINIO_ROOT_USER,
            MINIO_ROOT_PASSWORD,
            None,
            None,
            "tenant-inventory-tests",
        );
        let cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(shared.endpoint_url.clone())
            .credentials_provider(creds)
            .load()
            .await;
        let client = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&cfg)
                .force_path_style(true)
                .build(),
        );
        let bucket = format!("hfs-inventory-{}", Uuid::new_v4().simple());
        client
            .create_bucket()
            .bucket(&bucket)
            .send()
            .await
            .expect("failed to create MinIO test bucket");
        bucket
    }

    async fn backend(shared: &SharedMinio, tenancy_mode: S3TenancyMode) -> Arc<S3Backend> {
        let config = S3BackendConfig {
            tenancy_mode,
            prefix: Some(format!("inventory/{}", Uuid::new_v4())),
            region: Some("us-east-1".to_string()),
            endpoint_url: Some(shared.endpoint_url.clone()),
            force_path_style: true,
            allow_http: true,
            validate_buckets_on_startup: true,
            ..Default::default()
        };
        Arc::new(
            S3Backend::from_env_async(config)
                .await
                .expect("create S3 backend for MinIO"),
        )
    }

    fn server(storage: Arc<S3Backend>) -> TestServer {
        let config = ServerConfig {
            seed_conformance: false,
            ..ServerConfig::for_testing()
        };
        let state = AppState::new(storage, config);
        let router = helios_rest::routing::admin_tenants::routes(state.clone())
            .merge(helios_rest::routing::console_metrics::admin_routes(state));
        TestServer::new(router).expect("test server")
    }

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    async fn create_patient(storage: &S3Backend, tenant_id: &str, id: &str) {
        storage
            .create(
                &tenant(tenant_id),
                "Patient",
                json!({ "resourceType": "Patient", "id": id }),
                FhirVersion::default(),
            )
            .await
            .expect("create Patient");
    }

    /// Rows keyed by `key`, so assertions do not depend on listing order.
    fn rows<'a>(body: &'a Value, key: &str) -> HashMap<&'a str, &'a Value> {
        body["tenants"]
            .as_array()
            .expect("tenants array")
            .iter()
            .map(|row| (row[key].as_str().expect("row id"), row))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prefix_mode_reports_presence_not_counts() {
        let shared = shared_minio().await;
        let bucket = fresh_bucket(shared).await;
        let storage = backend(shared, S3TenancyMode::PrefixPerTenant { bucket }).await;

        // live: registered with two resources (a pointer count would say 2).
        storage.register_tenant("live", None).await.unwrap();
        create_patient(&storage, "live", "p1").await;
        create_patient(&storage, "live", "p2").await;
        // empty: registered, never wrote.
        storage.register_tenant("empty", None).await.unwrap();
        // dereg: wrote, then deregistered without purge.
        storage.register_tenant("dereg", None).await.unwrap();
        create_patient(&storage, "dereg", "p1").await;
        assert!(storage.deregister_tenant("dereg").await.unwrap());
        // tomb: never registered; its only resource is deleted, leaving a
        // tombstone and history (no live resource).
        create_patient(&storage, "tomb", "p1").await;
        storage
            .delete(&tenant("tomb"), "Patient", "p1")
            .await
            .unwrap();
        // purged: wrote, then its data was purged.
        create_patient(&storage, "purged", "p1").await;
        storage.purge_tenant_data("purged").await.unwrap();

        let server = server(Arc::clone(&storage));

        let admin = server.get("/admin/tenants").await;
        admin.assert_status_ok();
        let admin = admin.json::<Value>();
        assert_eq!(admin["resources_evidence"], "presence", "{admin:#}");
        assert_eq!(admin["discovery_complete"], true);
        let by_id = rows(&admin, "id");
        let mut ids: Vec<&str> = by_id.keys().copied().collect();
        ids.sort_unstable();
        assert_eq!(ids, ["dereg", "empty", "live", "tomb"], "{admin:#}");
        assert_eq!(admin["tenant_count"], 4);
        for (id, registered, has_data) in [
            ("live", true, true),
            ("empty", true, false),
            ("dereg", false, true),
            ("tomb", false, true),
        ] {
            let row = by_id[id];
            assert_eq!(row["registered"], registered, "{id}: {row}");
            assert_eq!(row["has_data"], has_data, "{id}: {row}");
            assert_eq!(row["resources"], Value::Null, "{id}: {row}");
        }

        let console = server.get("/console/metrics/tenants").await;
        console.assert_status_ok();
        let console = console.json::<Value>();
        assert_eq!(console["resources_evidence"], "presence", "{console:#}");
        assert_eq!(console["discovery_complete"], true);
        assert_eq!(console["resources_scope"], "cluster");
        let by_tenant = rows(&console, "tenant");
        for id in ["live", "dereg", "tomb"] {
            let row = by_tenant
                .get(id)
                .unwrap_or_else(|| panic!("{id} missing: {console:#}"));
            assert_eq!(row["has_data"], true, "{id}: {row}");
            assert_eq!(row["resources"], Value::Null, "{id}: {row}");
        }
        assert!(!by_tenant.contains_key("purged"), "{console:#}");
        assert!(!by_tenant.contains_key("empty"), "{console:#}");
    }

    /// Bucket-per-tenant has no bucket to discover strays in: the registry is
    /// still listed (it lives in the system bucket), but data is unknown, not
    /// empty.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bucket_per_tenant_reports_unsupported_not_empty() {
        let shared = shared_minio().await;
        let acme_bucket = fresh_bucket(shared).await;
        let system_bucket = fresh_bucket(shared).await;
        let storage = backend(
            shared,
            S3TenancyMode::BucketPerTenant {
                tenant_bucket_map: HashMap::from([("acme".to_string(), acme_bucket)]),
                default_system_bucket: Some(system_bucket),
            },
        )
        .await;
        storage.register_tenant("acme", None).await.unwrap();
        create_patient(&storage, "acme", "p1").await;

        let server = server(Arc::clone(&storage));

        let admin = server.get("/admin/tenants").await;
        admin.assert_status_ok();
        let admin = admin.json::<Value>();
        assert_eq!(admin["resources_evidence"], "unsupported", "{admin:#}");
        assert_eq!(admin["tenant_count"], 1);
        let acme = rows(&admin, "id")["acme"];
        assert_eq!(acme["registered"], true);
        assert_eq!(acme["resources"], Value::Null);
        assert_eq!(acme["has_data"], Value::Null);

        // The console lists traffic only here; give it a row to check. The
        // request log is process-global, hence the unique id.
        let visitor = "minio-bucket-per-tenant-visitor";
        helios_observability::reqlog::record(200, 0.010, visitor);
        let console = server.get("/console/metrics/tenants").await;
        console.assert_status_ok();
        let console = console.json::<Value>();
        assert_eq!(console["resources_evidence"], "unsupported", "{console:#}");
        assert_eq!(console["discovery_complete"], false);
        let by_tenant = rows(&console, "tenant");
        assert!(by_tenant.contains_key(visitor), "{console:#}");
        for row in by_tenant.values() {
            assert_eq!(row["resources"], Value::Null, "{row}");
            assert_eq!(row["has_data"], Value::Null, "{row}");
        }
    }
}
