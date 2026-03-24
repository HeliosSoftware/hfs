//! Application state for the FHIR REST API.
//!
//! This module defines the shared application state that is available to all
//! request handlers. It includes the storage backend, configuration, and any
//! other shared resources.

use std::sync::Arc;

use helios_auth::AuthConfig;
use helios_persistence::core::ResourceStorage;

use crate::config::ServerConfig;
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
}

// Manually implement Clone since S is wrapped in Arc and doesn't need to be Clone
impl<S> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
            config: Arc::clone(&self.config),
            auth_config: Arc::clone(&self.auth_config),
            auth: self.auth.clone(),
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
        Self {
            storage,
            config: Arc::new(config),
            auth_config: Arc::new(AuthConfig::default()),
            auth: None,
        }
    }

    /// Creates a new AppState with auth configuration.
    pub fn with_auth(
        storage: Arc<S>,
        config: ServerConfig,
        auth_config: AuthConfig,
        auth_state: Option<Arc<AuthMiddlewareState>>,
    ) -> Self {
        Self {
            storage,
            config: Arc::new(config),
            auth_config: Arc::new(auth_config),
            auth: auth_state,
        }
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

    /// Returns the auth configuration.
    pub fn auth_config(&self) -> &AuthConfig {
        &self.auth_config
    }

    /// Returns the auth middleware state if auth is enabled.
    pub fn auth_state(&self) -> Option<&Arc<AuthMiddlewareState>> {
        self.auth.as_ref()
    }

    /// Returns whether authentication is enabled.
    pub fn auth_enabled(&self) -> bool {
        self.auth.is_some()
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
