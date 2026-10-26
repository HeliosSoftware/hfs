//! SQL query ValueSet/ConceptMap dependencies (#2185), end to end on the S3
//! backend (MinIO). The scenarios are shared by every backend; see
//! `common/sof_terminology_e2e.rs`.
//!
//! Requires Docker: MinIO runs via testcontainers, like
//! `tenant_inventory_minio.rs`. Every scenario gets its own bucket and prefix,
//! so the registry compare-and-swap runs against the real SDK's conditional
//! writes.

#![cfg(feature = "s3")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[path = "common/sof_terminology_e2e.rs"]
mod sof_terminology_e2e;

use std::sync::{Arc, Once};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::config::Credentials;
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
use sof_terminology_e2e::Harness;
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
            let image =
                std::env::var("MINIO_IMAGE").unwrap_or_else(|_| DEFAULT_MINIO_IMAGE.to_string());
            let tag = std::env::var("MINIO_TAG").unwrap_or_else(|_| DEFAULT_MINIO_TAG.to_string());
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();

            let container = container_cleanup::with_cleanup_label(
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
        "sof-terminology-tests",
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
    let bucket = format!("hfs-softerm-{}", Uuid::new_v4().simple());
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
        prefix: Some(format!("softerm/{}", Uuid::new_v4())),
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

async fn s3(terminology: Option<String>) -> Option<Harness> {
    let shared = shared_minio().await;
    let bucket = fresh_bucket(shared).await;
    let storage = backend(shared, S3TenancyMode::PrefixPerTenant { bucket }).await;
    Some(sof_terminology_e2e::harness(storage, terminology))
}

sof_terminology_e2e_suite!(s3);
