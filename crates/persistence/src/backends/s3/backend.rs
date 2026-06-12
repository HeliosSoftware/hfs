//! AWS S3 backend — struct definition, capability matrix, and Backend trait
//! implementation.
use std::any::Any;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId};

use super::client::{AwsS3Client, AwsS3ClientOptions, S3Api, S3ClientError};
use super::config::{S3BackendConfig, S3TenancyMode};
use super::keyspace::S3Keyspace;

/// AWS S3 backend for object-storage persistence.
#[derive(Clone)]
pub struct S3Backend {
    pub(crate) config: S3BackendConfig,
    pub(crate) client: Arc<dyn S3Api>,
}

impl std::fmt::Debug for S3Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Backend")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Opaque connection handle for the S3 backend.
///
/// S3 is stateless from the client's perspective — there is no persistent TCP
/// connection to acquire per-request. This marker type satisfies the `Backend`
/// trait's associated `Connection` type without holding any resources.
#[derive(Debug)]
pub struct S3Connection;

/// Resolved bucket name and key hierarchy for a single tenant.
///
/// Computed once per storage operation from the `TenantContext` and the
/// backend configuration, then passed through the call stack within that
/// operation.
#[derive(Debug, Clone)]
pub(crate) struct TenantLocation {
    /// S3 bucket that holds this tenant's data.
    pub bucket: String,
    /// Keyspace builder scoped to this tenant's prefix hierarchy.
    pub keyspace: S3Keyspace,
}

impl S3Backend {
    /// Returns the backend-level capability declarations for S3.
    ///
    /// Kept independent of client construction so reporting/tests do not need
    /// AWS SDK initialization.
    pub fn declared_capabilities() -> Vec<BackendCapability> {
        vec![
            BackendCapability::Crud,
            BackendCapability::Versioning,
            BackendCapability::InstanceHistory,
            BackendCapability::TypeHistory,
            BackendCapability::SystemHistory,
            BackendCapability::OptimisticLocking,
            BackendCapability::CursorPagination,
            BackendCapability::BulkExport,
            BackendCapability::BulkSubmitIngest,
            BackendCapability::SharedSchema,
            BackendCapability::DatabasePerTenant,
        ]
    }

    /// Creates a new S3 backend using AWS standard credential provider chain.
    pub fn new(config: S3BackendConfig) -> StorageResult<Self> {
        Self::from_env(config)
    }

    /// Creates a new S3 backend using environment/provider chain credentials.
    ///
    /// The region is resolved in priority order: `config.region`, then the
    /// `AWS_REGION` environment variable, then the standard AWS SDK provider
    /// chain (shared config file, EC2 instance metadata, etc.).
    ///
    /// If `validate_buckets_on_startup` is set, every configured bucket is
    /// verified with a `HeadBucket` call before this function returns.
    pub fn from_env(config: S3BackendConfig) -> StorageResult<Self> {
        block_on(Self::from_env_async(config))?
    }

    /// Async constructor for S3 backend using environment/provider chain credentials.
    pub async fn from_env_async(mut config: S3BackendConfig) -> StorageResult<Self> {
        config.validate()?;

        if config.region.is_none() {
            config.region = std::env::var("AWS_REGION").ok();
        }

        apply_s3_compatible_endpoint_defaults(&mut config);

        let sdk_config = AwsS3Client::load_sdk_config(config.region.as_deref()).await;
        let endpoint_url = config
            .endpoint_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string);

        let client = Arc::new(AwsS3Client::from_sdk_config_with_options(
            &sdk_config,
            AwsS3ClientOptions {
                endpoint_url,
                force_path_style: config.force_path_style,
            },
        ));

        let backend = Self { config, client };

        if backend.config.validate_buckets_on_startup {
            backend.validate_buckets().await?;
        }

        Ok(backend)
    }
    /// Creates a backend with an injected `S3Api` implementation.
    ///
    /// Intended exclusively for unit tests that supply a mock client.
    /// Not compiled into non-test builds.
    #[cfg(test)]
    pub(crate) fn with_client(
        config: S3BackendConfig,
        client: Arc<dyn S3Api>,
    ) -> StorageResult<Self> {
        config.validate()?;
        Ok(Self { config, client })
    }

    /// Verifies that every bucket referenced in the configuration exists and
    /// is accessible to the current credentials.
    ///
    /// Issues a `HeadBucket` request for each distinct bucket. Returns the
    /// first error encountered; does not attempt to create missing buckets.
    pub(crate) async fn validate_buckets(&self) -> StorageResult<()> {
        for bucket in self.config.configured_buckets() {
            self.client
                .head_bucket(&bucket)
                .await
                .map_err(|e| self.map_client_error(e))?;
        }
        Ok(())
    }

    /// Resolves the bucket and keyspace for the given tenant.
    ///
    /// In `PrefixPerTenant` mode all tenants share one bucket and are separated
    /// by a key prefix derived from the tenant ID. In `BucketPerTenant` mode
    /// each tenant maps to a dedicated bucket looked up from the config map.
    ///
    /// Returns a `TenantError` if the tenant has no bucket assignment in the
    /// `BucketPerTenant` mapping.
    pub(crate) fn tenant_location(&self, tenant: &TenantContext) -> StorageResult<TenantLocation> {
        let global_prefix = self
            .config
            .prefix
            .as_ref()
            .map(|p| p.trim_matches('/').to_string())
            .filter(|p| !p.is_empty());

        match &self.config.tenancy_mode {
            S3TenancyMode::PrefixPerTenant { bucket } => Ok(TenantLocation {
                bucket: bucket.clone(),
                keyspace: S3Keyspace::new(global_prefix)
                    .with_tenant_prefix(tenant.tenant_id().as_str()),
            }),
            S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            } => {
                let tenant_id = tenant.tenant_id().as_str();
                let bucket = tenant_bucket_map
                    .get(tenant_id)
                    .cloned()
                    .or_else(|| {
                        if tenant.tenant_id().is_system() {
                            default_system_bucket.clone()
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| {
                        StorageError::Tenant(crate::error::TenantError::InvalidTenant {
                            tenant_id: TenantId::new(tenant_id),
                        })
                    })?;

                Ok(TenantLocation {
                    bucket,
                    keyspace: S3Keyspace::new(global_prefix),
                })
            }
        }
    }

    /// Maps a low-level `S3ClientError` to the shared `StorageError` taxonomy.
    ///
    /// This is the error boundary between the S3 SDK layer and the storage
    /// trait layer. Keeping the translation here ensures all storage operations
    /// return consistent error variants regardless of the underlying transport.
    pub(crate) fn map_client_error(&self, error: S3ClientError) -> StorageError {
        match error {
            S3ClientError::NotFound => StorageError::Backend(BackendError::Unavailable {
                backend_name: "s3".to_string(),
                message: "resource not found in S3".to_string(),
            }),
            S3ClientError::PreconditionFailed => StorageError::Backend(BackendError::QueryError {
                message: "S3 precondition failed".to_string(),
            }),
            S3ClientError::Throttled(message) => StorageError::Backend(BackendError::Unavailable {
                backend_name: "s3".to_string(),
                message,
            }),
            S3ClientError::Unavailable(message) => {
                StorageError::Backend(BackendError::Unavailable {
                    backend_name: "s3".to_string(),
                    message,
                })
            }
            S3ClientError::InvalidInput(message) => {
                StorageError::Validation(crate::error::ValidationError::InvalidResource {
                    message,
                    details: Vec::new(),
                })
            }
            S3ClientError::Internal(message) => StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message,
                source: None,
            }),
        }
    }
}

#[async_trait]
impl Backend for S3Backend {
    type Connection = S3Connection;

    fn kind(&self) -> BackendKind {
        BackendKind::S3
    }

    fn name(&self) -> &'static str {
        "s3"
    }

    fn supports(&self, capability: BackendCapability) -> bool {
        Self::declared_capabilities().contains(&capability)
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        Self::declared_capabilities()
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        Ok(S3Connection)
    }

    async fn release(&self, _conn: Self::Connection) {}

    async fn health_check(&self) -> Result<(), BackendError> {
        self.validate_buckets().await.map_err(|err| match err {
            StorageError::Backend(backend_err) => backend_err,
            other => BackendError::Internal {
                backend_name: "s3".to_string(),
                message: other.to_string(),
                source: None,
            },
        })
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.health_check().await
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        // No schema migrations for object storage.
        self.health_check().await
    }
}

/// Applies endpoint-mode defaults without changing standard AWS mode behavior.
fn apply_s3_compatible_endpoint_defaults(config: &mut S3BackendConfig) {
    let has_endpoint_url = config
        .endpoint_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .is_some();

    if !has_endpoint_url {
        return;
    }

    if !config.force_path_style {
        config.force_path_style = true;
    }

    if config.region.is_none() {
        config.region = Some("us-east-1".to_string());
    }
}

/// Drives an async future to completion from synchronous code.
///
/// If a Tokio runtime is already active, the future is driven on a detached
/// thread to avoid nesting runtimes. Otherwise a temporary single-threaded
/// runtime is created for the duration of the call.
fn block_on<F>(future: F) -> StorageResult<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: format!("failed to create runtime: {e}"),
                        source: None,
                    })
                })?;
            Ok(rt.block_on(future))
        })
        .join()
        .map_err(|panic_payload| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: format!(
                    "failed to join detached runtime thread: {}",
                    panic_payload_to_message(panic_payload)
                ),
                source: None,
            })
        })?
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "s3".to_string(),
                    message: format!("failed to create runtime: {e}"),
                    source: None,
                })
            })?;
        Ok(rt.block_on(future))
    }
}

fn panic_payload_to_message(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> S3BackendConfig {
        S3BackendConfig {
            tenancy_mode: S3TenancyMode::PrefixPerTenant {
                bucket: "test-bucket".to_string(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn endpoint_defaults_not_applied_in_aws_mode() {
        let mut config = base_config();
        config.endpoint_url = None;
        config.region = None;
        config.force_path_style = false;

        apply_s3_compatible_endpoint_defaults(&mut config);

        assert!(config.region.is_none());
        assert!(!config.force_path_style);
    }

    #[test]
    fn endpoint_defaults_applied_when_endpoint_is_set() {
        let mut config = base_config();
        config.endpoint_url = Some("http://127.0.0.1:9000".to_string());
        config.region = None;
        config.force_path_style = false;

        apply_s3_compatible_endpoint_defaults(&mut config);

        assert_eq!(config.region.as_deref(), Some("us-east-1"));
        assert!(config.force_path_style);
    }

    #[test]
    fn block_on_works_inside_current_thread_runtime() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");

        rt.block_on(async {
            let value = block_on(async { 7usize }).expect("block_on should work");
            assert_eq!(value, 7);
        });
    }
}
