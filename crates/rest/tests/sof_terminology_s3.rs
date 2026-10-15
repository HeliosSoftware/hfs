//! "Terminology in SQL" (#1824) — standalone S3 (MinIO).
//!
//! Runs the scenarios in `common/terminology_scenarios.rs` (see
//! `sof_terminology.rs` for SQLite) against the S3 backend over a MinIO
//! container, one bucket prefix per server. Two things differ from every
//! other backend, and the scenarios branch on both:
//!
//! - **no search index**: `GET /Type?url=` is unsupported, so canonical
//!   resolution scans the type (`references.rs::scan_candidates`) and the
//!   harness verifies stored definitions by scan too;
//! - **no atomic transactions**: a transaction Bundle is `501` before the
//!   terminology pre-pass runs (#489), which the transaction scenarios pin;
//!   batch Bundles run entry by entry and behave as elsewhere.
//!
//! Requires Docker (testcontainers spins up a MinIO instance).

#![cfg(feature = "s3")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/terminology_scenarios.rs"]
mod terminology_scenarios;

use std::sync::{Arc, Once};

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Credentials;
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
use helios_rest::ServerConfig;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio::sync::OnceCell;

use terminology_scenarios::{Driver, Hfs};

const DEFAULT_MINIO_IMAGE: &str = "ghcr.io/coollabsio/minio";
const DEFAULT_MINIO_TAG: &str = "RELEASE.2025-10-15T17-29-55Z";
const MINIO_ROOT_USER: &str = "minioadmin";
const MINIO_ROOT_PASSWORD: &str = "minioadmin";

struct SharedMinio {
    endpoint_url: String,
    bucket: String,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<GenericImage>,
}

static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();
static MINIO_AWS_ENV: Once = Once::new();

/// The backend reads its credentials from the AWS environment, as `hfs` does.
fn ensure_backend_env_credentials() {
    MINIO_AWS_ENV.call_once(|| {
        // SAFETY: executes exactly once for this test binary before any
        // backend construction in it, and the values never change.
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", MINIO_ROOT_USER);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", MINIO_ROOT_PASSWORD);
            std::env::set_var("AWS_REGION", "us-east-1");
            std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        }
    });
}

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
            let endpoint_url = format!("http://{host}:{port}");

            // One bucket for the binary; every server gets its own prefix.
            let creds = Credentials::new(
                MINIO_ROOT_USER,
                MINIO_ROOT_PASSWORD,
                None,
                None,
                "terminology-tests",
            );
            let sdk_config = aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .endpoint_url(endpoint_url.clone())
                .credentials_provider(creds)
                .load()
                .await;
            let client = Client::from_conf(
                aws_sdk_s3::config::Builder::from(&sdk_config)
                    .force_path_style(true)
                    .build(),
            );
            let bucket = format!("hfs-terminology-{}", uuid::Uuid::new_v4().simple());
            client
                .create_bucket()
                .bucket(&bucket)
                .send()
                .await
                .expect("failed to create MinIO test bucket");

            SharedMinio {
                endpoint_url,
                bucket,
                _container: container,
            }
        })
        .await
}

struct S3Driver;

#[async_trait]
impl Driver for S3Driver {
    async fn server(&self, config: ServerConfig, scopes: Option<&str>) -> Hfs {
        let shared = shared_minio().await;
        ensure_backend_env_credentials();
        let backend = S3Backend::from_env_async(S3BackendConfig {
            tenancy_mode: S3TenancyMode::PrefixPerTenant {
                bucket: shared.bucket.clone(),
            },
            prefix: Some(format!("terminology/{}", uuid::Uuid::new_v4().simple())),
            region: Some("us-east-1".to_string()),
            endpoint_url: Some(shared.endpoint_url.clone()),
            force_path_style: true,
            allow_http: true,
            validate_buckets_on_startup: true,
            ..Default::default()
        })
        .await
        .expect("create S3 backend for MinIO");
        terminology_scenarios::build(Arc::new(backend), config, scopes, false, false).await
    }
}

async fn driver(_name: &str) -> Option<S3Driver> {
    Some(S3Driver)
}

terminology_suite!(driver);
