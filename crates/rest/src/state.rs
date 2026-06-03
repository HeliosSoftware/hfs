//! Application state for the FHIR REST API.
//!
//! This module defines the shared application state that is available to all
//! request handlers. It includes the storage backend, configuration, and any
//! other shared resources.

use std::sync::Arc;

use helios_audit::AuditSink;
use helios_auth::AuthConfig;
use helios_persistence::core::{
    BulkExportJobStore, BulkSubmitJobStore, ExportOutputStore, ResourceStorage, SubmitInputFetcher,
};

use crate::bulk_export_auth::ExportFileAuth;
use crate::config::{BulkExportConfig, BulkSubmitConfig, ServerConfig};
use crate::middleware::auth::AuthMiddlewareState;

/// Shared application state for the REST API.
///
/// This struct holds all the shared state that handlers need access to,
/// including the storage backend and server configuration.
///
/// # Type Parameters
///
/// * `S` - The storage backend type (must implement [`ResourceStorage`])
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::{AppState, ServerConfig};
/// use helios_persistence::backends::sqlite::SqliteBackend;
/// use std::sync::Arc;
///
/// let backend = SqliteBackend::in_memory()?;
/// let config = ServerConfig::default();
/// let state = AppState::new(Arc::new(backend), config);
/// ```
pub struct AppState<S> {
    /// The storage backend.
    storage: Arc<S>,

    /// Server configuration.
    config: Arc<ServerConfig>,

    /// Authentication configuration (always present, may be disabled).
    auth_config: Arc<AuthConfig>,

    /// Auth middleware state (present only when auth is enabled).
    auth: Option<Arc<AuthMiddlewareState>>,

    /// Optional audit sink for handler-level per-entry audit emission.
    audit_sink: Option<Arc<dyn AuditSink>>,

    /// Audit source observer reference used when emitting handler-level events.
    audit_source_observer: String,

    /// Optional subscription engine for FHIR topic-based subscriptions.
    #[cfg(feature = "subscriptions")]
    subscription_engine: Option<Arc<helios_subscriptions::SubscriptionEngine>>,

    /// Bulk export job-state store (claim + worker storage + lifecycle).
    bulk_export_jobs: Option<Arc<dyn BulkExportJobStore>>,

    /// Bulk export output store (NDJSON files).
    bulk_export_output: Option<Arc<dyn ExportOutputStore>>,

    /// Bulk export download authorizer.
    bulk_export_file_auth: Option<Arc<dyn ExportFileAuth>>,

    /// Bulk export configuration.
    bulk_export_config: Arc<BulkExportConfig>,

    /// Bulk submit job-state store (claim + worker storage + lifecycle).
    bulk_submit_jobs: Option<Arc<dyn BulkSubmitJobStore>>,

    /// Bulk submit remote input fetcher (manifest + NDJSON retrieval).
    bulk_submit_fetcher: Option<Arc<dyn SubmitInputFetcher>>,

    /// Bulk submit output store (status-manifest output/error/deleted artifacts).
    bulk_submit_output: Option<Arc<dyn ExportOutputStore>>,

    /// Bulk submit download authorizer (reuses the export file-auth trait).
    bulk_submit_file_auth: Option<Arc<dyn ExportFileAuth>>,

    /// Bulk submit configuration.
    bulk_submit_config: Arc<BulkSubmitConfig>,
}

// Manually implement Clone since S is wrapped in Arc and doesn't need to be Clone
impl<S> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
            config: Arc::clone(&self.config),
            auth_config: Arc::clone(&self.auth_config),
            auth: self.auth.clone(),
            audit_sink: self.audit_sink.clone(),
            audit_source_observer: self.audit_source_observer.clone(),
            #[cfg(feature = "subscriptions")]
            subscription_engine: self.subscription_engine.clone(),
            bulk_export_jobs: self.bulk_export_jobs.clone(),
            bulk_export_output: self.bulk_export_output.clone(),
            bulk_export_file_auth: self.bulk_export_file_auth.clone(),
            bulk_export_config: Arc::clone(&self.bulk_export_config),
            bulk_submit_jobs: self.bulk_submit_jobs.clone(),
            bulk_submit_fetcher: self.bulk_submit_fetcher.clone(),
            bulk_submit_output: self.bulk_submit_output.clone(),
            bulk_submit_file_auth: self.bulk_submit_file_auth.clone(),
            bulk_submit_config: Arc::clone(&self.bulk_submit_config),
        }
    }
}

impl<S: ResourceStorage> AppState<S> {
    /// Creates a new AppState with the given storage and configuration.
    ///
    /// # Arguments
    ///
    /// * `storage` - The storage backend (wrapped in Arc)
    /// * `config` - Server configuration
    pub fn new(storage: Arc<S>, config: ServerConfig) -> Self {
        let bulk_export_config = Arc::new(config.bulk_export.clone());
        let bulk_submit_config = Arc::new(config.bulk_submit.clone());
        Self {
            storage,
            config: Arc::new(config),
            auth_config: Arc::new(AuthConfig::default()),
            auth: None,
            audit_sink: None,
            audit_source_observer: "Device/hfs".to_string(),
            #[cfg(feature = "subscriptions")]
            subscription_engine: None,
            bulk_export_jobs: None,
            bulk_export_output: None,
            bulk_export_file_auth: None,
            bulk_export_config,
            bulk_submit_jobs: None,
            bulk_submit_fetcher: None,
            bulk_submit_output: None,
            bulk_submit_file_auth: None,
            bulk_submit_config,
        }
    }

    /// Creates a new AppState with auth configuration.
    pub fn with_auth(
        storage: Arc<S>,
        config: ServerConfig,
        auth_config: AuthConfig,
        auth_state: Option<Arc<AuthMiddlewareState>>,
    ) -> Self {
        Self::with_auth_and_audit(storage, config, auth_config, auth_state, None, "Device/hfs")
    }

    /// Creates a new AppState with auth and audit configuration.
    pub fn with_auth_and_audit(
        storage: Arc<S>,
        config: ServerConfig,
        auth_config: AuthConfig,
        auth_state: Option<Arc<AuthMiddlewareState>>,
        audit_sink: Option<Arc<dyn AuditSink>>,
        audit_source_observer: impl Into<String>,
    ) -> Self {
        let bulk_export_config = Arc::new(config.bulk_export.clone());
        let bulk_submit_config = Arc::new(config.bulk_submit.clone());
        Self {
            storage,
            config: Arc::new(config),
            auth_config: Arc::new(auth_config),
            auth: auth_state,
            audit_sink,
            audit_source_observer: audit_source_observer.into(),
            #[cfg(feature = "subscriptions")]
            subscription_engine: None,
            bulk_export_jobs: None,
            bulk_export_output: None,
            bulk_export_file_auth: None,
            bulk_export_config,
            bulk_submit_jobs: None,
            bulk_submit_fetcher: None,
            bulk_submit_output: None,
            bulk_submit_file_auth: None,
            bulk_submit_config,
        }
    }

    /// Wires the bulk-export job store, output store, and file authorizer.
    pub fn with_bulk_export(
        mut self,
        jobs: Arc<dyn BulkExportJobStore>,
        output: Arc<dyn ExportOutputStore>,
        file_auth: Arc<dyn ExportFileAuth>,
    ) -> Self {
        self.bulk_export_jobs = Some(jobs);
        self.bulk_export_output = Some(output);
        self.bulk_export_file_auth = Some(file_auth);
        self
    }

    /// Returns the bulk-export job store, if configured.
    pub fn bulk_export_jobs(&self) -> Option<&Arc<dyn BulkExportJobStore>> {
        self.bulk_export_jobs.as_ref()
    }

    /// Returns the bulk-export output store, if configured.
    pub fn bulk_export_output(&self) -> Option<&Arc<dyn ExportOutputStore>> {
        self.bulk_export_output.as_ref()
    }

    /// Returns the bulk-export download authorizer, if configured.
    pub fn bulk_export_file_auth(&self) -> Option<&Arc<dyn ExportFileAuth>> {
        self.bulk_export_file_auth.as_ref()
    }

    /// Returns the bulk-export configuration.
    pub fn bulk_export_config(&self) -> &BulkExportConfig {
        &self.bulk_export_config
    }

    /// Wires the bulk-submit job store, input fetcher, output store, and file authorizer.
    pub fn with_bulk_submit(
        mut self,
        jobs: Arc<dyn BulkSubmitJobStore>,
        fetcher: Arc<dyn SubmitInputFetcher>,
        output: Arc<dyn ExportOutputStore>,
        file_auth: Arc<dyn ExportFileAuth>,
    ) -> Self {
        self.bulk_submit_jobs = Some(jobs);
        self.bulk_submit_fetcher = Some(fetcher);
        self.bulk_submit_output = Some(output);
        self.bulk_submit_file_auth = Some(file_auth);
        self
    }

    /// Returns the bulk-submit job store, if configured.
    pub fn bulk_submit_jobs(&self) -> Option<&Arc<dyn BulkSubmitJobStore>> {
        self.bulk_submit_jobs.as_ref()
    }

    /// Returns the bulk-submit input fetcher, if configured.
    pub fn bulk_submit_fetcher(&self) -> Option<&Arc<dyn SubmitInputFetcher>> {
        self.bulk_submit_fetcher.as_ref()
    }

    /// Returns the bulk-submit output store, if configured.
    pub fn bulk_submit_output(&self) -> Option<&Arc<dyn ExportOutputStore>> {
        self.bulk_submit_output.as_ref()
    }

    /// Returns the bulk-submit download authorizer, if configured.
    pub fn bulk_submit_file_auth(&self) -> Option<&Arc<dyn ExportFileAuth>> {
        self.bulk_submit_file_auth.as_ref()
    }

    /// Returns the bulk-submit configuration.
    pub fn bulk_submit_config(&self) -> &BulkSubmitConfig {
        &self.bulk_submit_config
    }

    /// Sets the subscription engine on this AppState.
    #[cfg(feature = "subscriptions")]
    pub fn with_subscription_engine(
        mut self,
        engine: Arc<helios_subscriptions::SubscriptionEngine>,
    ) -> Self {
        self.subscription_engine = Some(engine);
        self
    }

    /// Returns a reference to the storage backend.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Returns a clone of the storage Arc.
    pub fn storage_arc(&self) -> Arc<S> {
        Arc::clone(&self.storage)
    }

    /// Returns a reference to the server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Returns the default tenant ID from configuration.
    pub fn default_tenant(&self) -> &str {
        &self.config.default_tenant
    }

    /// Returns the base URL for the server.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Returns whether versioning is enabled.
    pub fn versioning_enabled(&self) -> bool {
        self.config.enable_versioning
    }

    /// Returns whether If-Match is required for updates.
    pub fn require_if_match(&self) -> bool {
        self.config.require_if_match
    }

    /// Returns the default page size for search results.
    pub fn default_page_size(&self) -> usize {
        self.config.default_page_size
    }

    /// Returns the maximum page size for search results.
    pub fn max_page_size(&self) -> usize {
        self.config.max_page_size
    }

    /// Returns whether deleted resources should return 410 Gone.
    pub fn return_gone(&self) -> bool {
        self.config.return_gone
    }

    /// Returns a reference to the auth configuration.
    pub fn auth_config(&self) -> &AuthConfig {
        &self.auth_config
    }

    /// Returns the configured terminology server URL, if any.
    ///
    /// When `Some`, the search handler will use this URL to expand ValueSets
    /// for `:in` and `:not-in` search modifiers.
    pub fn terminology_server_url(&self) -> Option<&str> {
        self.config.terminology_server.as_deref()
    }

    /// Returns the audit sink for handler-level audit emission, if configured.
    pub fn audit_sink(&self) -> Option<&Arc<dyn AuditSink>> {
        self.audit_sink.as_ref()
    }

    /// Returns the configured audit source observer reference.
    pub fn audit_source_observer(&self) -> &str {
        &self.audit_source_observer
    }

    /// Returns the subscription engine, if configured.
    #[cfg(feature = "subscriptions")]
    pub fn subscription_engine(&self) -> Option<&Arc<helios_subscriptions::SubscriptionEngine>> {
        self.subscription_engine.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::error::StorageResult;
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;
    use serde_json::Value;

    // Mock storage for testing
    struct MockStorage;

    #[async_trait]
    impl ResourceStorage for MockStorage {
        fn backend_name(&self) -> &'static str {
            "mock"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!()
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    #[test]
    fn test_app_state_creation() {
        let storage = Arc::new(MockStorage);
        let config = ServerConfig::default();
        let state = AppState::new(storage, config);

        assert_eq!(state.storage().backend_name(), "mock");
        assert_eq!(state.default_tenant(), "default");
    }

    #[test]
    fn test_app_state_config_access() {
        let storage = Arc::new(MockStorage);
        let config = ServerConfig {
            default_tenant: "custom-tenant".to_string(),
            base_url: "https://fhir.example.com".to_string(),
            enable_versioning: true,
            default_page_size: 50,
            max_page_size: 500,
            ..Default::default()
        };
        let state = AppState::new(storage, config);

        assert_eq!(state.default_tenant(), "custom-tenant");
        assert_eq!(state.base_url(), "https://fhir.example.com");
        assert!(state.versioning_enabled());
        assert_eq!(state.default_page_size(), 50);
        assert_eq!(state.max_page_size(), 500);
    }

    #[test]
    fn test_app_state_clone() {
        let storage = Arc::new(MockStorage);
        let config = ServerConfig::default();
        let state = AppState::new(storage, config);
        let cloned = state.clone();

        assert_eq!(state.default_tenant(), cloned.default_tenant());
    }
}
