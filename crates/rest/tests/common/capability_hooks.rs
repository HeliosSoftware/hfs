//! A storage that overrides a CapabilityStatement hook, for the `/metadata`
//! tests in `capabilities_search_params.rs`.
//!
//! [`HookStub`] forwards every storage call to an in-memory SQLite backend, so
//! the statement is built over the full spec SearchParameter registry, and
//! answers [`SearchProvider::serves_search_param`] from its own field. It is
//! served by [`metadata_server`], a router with only the capabilities route: the
//! handler's bound is all the stub has to meet, so no history, versioning,
//! export or include trait is implemented here.
//!
//! Included with `#[path = "common/capability_hooks.rs"] mod capability_hooks;`.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BundleEntry, BundleProvider, BundleResult, ConditionalCreateResult, ConditionalDeleteResult,
    ConditionalInteraction, ConditionalStorage, ConditionalUpdateResult, EntityTagPrecondition,
    PatchCandidateValidator, ResourceStorage, SearchProvider, SearchResult,
};
use helios_persistence::error::{StorageResult, TransactionError};
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchParamType, SearchQuery, StoredResource};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::Value;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

/// Which search parameters a [`HookStub`] serves: `(resource_type, code,
/// param_type)`, as [`SearchProvider::serves_search_param`] takes them.
pub type Serves = fn(&str, &str, SearchParamType) -> bool;

/// Serves every parameter: the trait default.
pub fn serves_all(_: &str, _: &str, _: SearchParamType) -> bool {
    true
}

/// Serves only token, date and URI parameters.
pub fn serves_token_date_uri(_: &str, _: &str, param_type: SearchParamType) -> bool {
    matches!(
        param_type,
        SearchParamType::Token | SearchParamType::Date | SearchParamType::Uri
    )
}

/// SQLite behind the hook.
pub struct HookStub {
    inner: Arc<SqliteBackend>,
    serves: Serves,
}

impl HookStub {
    /// The hook as given.
    pub fn with_params(serves: Serves) -> Self {
        Self {
            inner: Arc::new(sqlite()),
            serves,
        }
    }
}

/// In-memory SQLite with the spec SearchParameters from the repository's
/// `data/` directory loaded, so the registry is the one a server builds.
pub fn sqlite() -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");
    backend
}

fn config() -> ServerConfig {
    ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    }
}

/// A server with only `GET /metadata`, over `stub`.
pub fn metadata_server(stub: HookStub) -> TestServer {
    let state = helios_rest::AppState::new(Arc::new(stub), config());
    let app = axum::Router::new()
        .route(
            "/metadata",
            axum::routing::get(helios_rest::handlers::capabilities_handler::<HookStub>),
        )
        .with_state(state);
    TestServer::new(app).expect("test server")
}

/// The CapabilityStatement `server` answers for `version`.
pub async fn statement(server: &TestServer, version: FhirVersion) -> Value {
    let accept = format!(
        "application/fhir+json; fhirVersion={}",
        version.as_mime_param()
    );
    let response = server
        .get("/metadata")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            header::ACCEPT,
            HeaderValue::from_str(&accept).expect("accept"),
        )
        .await;
    response.assert_status(StatusCode::OK);
    serde_json::from_str(&response.text()).expect("metadata is JSON")
}

/// The `code`s of an `interaction` array.
pub fn interaction_codes(interactions: &Value) -> Vec<String> {
    interactions
        .as_array()
        .expect("interaction array")
        .iter()
        .filter_map(|i| i["code"].as_str().map(str::to_string))
        .collect()
}

#[async_trait]
impl ResourceStorage for HookStub {
    fn backend_name(&self) -> &'static str {
        "hook-stub"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

#[async_trait]
impl SearchProvider for HookStub {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.inner.search(tenant, query).await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.inner.search_count(tenant, query).await
    }

    fn search_param_registry(
        &self,
        tenant: &TenantContext,
    ) -> Arc<parking_lot::RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }

    fn supports_contained_search(&self) -> bool {
        self.inner.supports_contained_search()
    }

    fn modifiers_for_param_type(&self, param_type: SearchParamType) -> Vec<&'static str> {
        self.inner.modifiers_for_param_type(param_type)
    }

    fn serves_search_param(
        &self,
        resource_type: &str,
        code: &str,
        param_type: SearchParamType,
    ) -> bool {
        (self.serves)(resource_type, code, param_type)
    }
}

#[async_trait]
impl ConditionalStorage for HookStub {
    fn supports_conditional(&self, interaction: ConditionalInteraction) -> bool {
        self.inner.supports_conditional(interaction)
    }

    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        self.inner
            .conditional_create(tenant, resource_type, resource, search_params, fhir_version)
            .await
    }

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
        if_match: &EntityTagPrecondition,
    ) -> StorageResult<ConditionalUpdateResult> {
        self.inner
            .conditional_update(
                tenant,
                resource_type,
                resource,
                search_params,
                upsert,
                fhir_version,
                if_match,
            )
            .await
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        if_match: &EntityTagPrecondition,
    ) -> StorageResult<ConditionalDeleteResult> {
        self.inner
            .conditional_delete(tenant, resource_type, search_params, if_match)
            .await
    }
}

#[async_trait]
impl BundleProvider for HookStub {
    fn supports_atomic_transactions(&self) -> bool {
        self.inner.supports_atomic_transactions()
    }

    fn supports_conditional_in_transaction(&self) -> bool {
        self.inner.supports_conditional_in_transaction()
    }

    async fn process_transaction_with_patch_validator(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
    ) -> Result<BundleResult, TransactionError> {
        self.inner
            .process_transaction_with_patch_validator(tenant, entries, fhir_version, validator)
            .await
    }
}
