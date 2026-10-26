//! Resource CRUD API for CodeSystem, ValueSet, and ConceptMap.
//!
//! Handlers for GET / POST / PUT / DELETE on CodeSystem, ValueSet, and
//! ConceptMap resources.
//!
//! Raw FHIR JSON is stored and versioned via `helios-persistence`.
//! The HTS normalized terminology tables
//! (code_systems, concepts, value_sets, …) are kept in sync by the same
//! import / delete helpers used by the `POST /import` pipeline.
//!
//! # ETag / versioning
//!
//! Persistence-managed resources carry a `version_id` and an `ETag` header.
//! Imported-only reads have no ETag until a write establishes real history.
//! PUT requests may include an `If-Match` header; when present, the handler
//! returns **412 Precondition Failed** if the version no longer matches.

#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod inner {
    use crate::backends::resource_identity::ResourceKind;
    use crate::operations::crud_transaction::{self, CrudTransaction, storage_error};
    use axum::{
        Json,
        extract::{Path, RawQuery, State},
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
    };
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;
    use serde_json::Value;

    use crate::error::HtsError;
    use crate::state::AppState;
    use crate::traits::TerminologyBackend;

    use super::super::format::{ResponseFormat, fhir_respond, negotiate_format};

    // ── Tenant context ─────────────────────────────────────────────────────────

    fn ctx() -> TenantContext {
        TenantContext::system()
    }

    // ── Cache invalidation seam ───────────────────────────────────────────────

    /// Drop every cached answer derived from terminology content, across both
    /// cache tiers.
    ///
    /// **Call this after every mutation, on every exit path.** It is the single
    /// seam that keeps CRUD writes honest, and it exists because attaching
    /// eviction to the write *mechanism* rather than to the write *verb* is what
    /// produced issue #304: eviction lived inside `BundleImportBackend::
    /// import_bundle`, the SQLite CRUD path writes through its own pooled
    /// connection instead, and so no SQLite CRUD verb evicted anything.
    ///
    /// Two tiers, both required:
    ///
    /// - [`AppState::clear_expand_cache`] — the handler-level response caches
    ///   (`$expand`, `$lookup`, both `$validate-code` directions) and, critically,
    ///   the two *negative* caches. A URL recorded as absent by an `$expand` that
    ///   404'd stays absent until something clears it, so without this a
    ///   `POST /ValueSet` creating that exact URL is followed by a 404 for a
    ///   resource that demonstrably exists.
    /// - [`crate::traits::TerminologyCaches::invalidate_caches`] — the backend's own per-instance
    ///   memos. Clearing only the handler tier is not enough: a `PUT` that changes
    ///   a display forces the handler to recompute, and the recomputation is then
    ///   served from the backend's stale `lookup_response_cache`.
    ///
    /// # Ordering
    ///
    /// Callers evict after the shared transaction settles, never before.
    /// Evicting first
    /// leaves a window in which a concurrent reader repopulates from pre-write
    /// state and that entry never expires. Evicting afterwards still leaves a
    /// narrow race — a reader that began its query before the write commits may
    /// insert just after this clear — but that window is bounded by one in-flight
    /// request rather than unbounded. Closing it entirely needs a generation
    /// counter compared at insert time across ~30 insertion sites, which is a
    /// separate change.
    fn evict_caches_after_write<B: TerminologyBackend>(state: &AppState<B>) {
        state.clear_expand_cache();
        state.backend().invalidate_caches();
    }

    /// Settle the transaction explicitly. No raw resource, history entry or
    /// normalized row is committed when any step of a CRUD request fails.
    async fn finish<T>(
        transaction: Box<dyn CrudTransaction>,
        result: Result<T, HtsError>,
    ) -> Result<T, HtsError> {
        match result {
            Ok(value) => {
                transaction.commit().await.map_err(storage_error)?;
                Ok(value)
            }
            Err(error) => {
                transaction.rollback().await.map_err(|rollback| {
                    HtsError::StorageError(format!("{error}; rollback failed: {rollback}"))
                })?;
                Err(error)
            }
        }
    }

    fn resource_kind(resource_type: &str) -> Result<ResourceKind, HtsError> {
        ResourceKind::from_resource_type(resource_type).ok_or_else(|| {
            HtsError::InvalidRequest(format!("Unsupported resource type: {resource_type}"))
        })
    }

    /// Imported resources acquire a history baseline before deletion.
    async fn current_resource(
        transaction: &mut dyn CrudTransaction,
        kind: ResourceKind,
        id: &str,
    ) -> Result<StoredResource, HtsError> {
        let resource_type = kind.resource_type();
        if let Some(current) = transaction
            .read(resource_type, id)
            .await
            .map_err(storage_error)?
        {
            return Ok(current);
        }
        let content = transaction
            .normalized_resource(kind, id)
            .await?
            .ok_or_else(|| HtsError::NotFound(format!("{resource_type}/{id}")))?;
        transaction
            .create(resource_type, content)
            .await
            .map_err(storage_error)
    }

    /// Version metadata belongs to persistence, not to the submitted resource.
    fn clear_server_meta(body: &mut Value) {
        if let Some(meta) = body.get_mut("meta").and_then(Value::as_object_mut) {
            meta.remove("versionId");
            meta.remove("lastUpdated");
        }
    }

    fn resource_response(
        stored: &StoredResource,
        format: ResponseFormat,
        status: StatusCode,
    ) -> Response {
        let mut response = fhir_respond(stored.content_with_meta(), format);
        *response.status_mut() = status;
        if let Ok(value) = HeaderValue::from_str(stored.etag()) {
            response.headers_mut().insert(header::ETAG, value);
        }
        let modified = stored.last_modified().format("%a, %d %b %Y %H:%M:%S GMT");
        if let Ok(value) = HeaderValue::from_str(&modified.to_string()) {
            response.headers_mut().insert(header::LAST_MODIFIED, value);
        }
        if status == StatusCode::CREATED
            && let Ok(value) = HeaderValue::from_str(&format!("/{}", stored.versioned_url()))
        {
            response.headers_mut().insert(header::LOCATION, value);
        }
        response
    }

    /// Create raw content and normalized terminology in one transaction.
    async fn create_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        mut body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let id = helios_persistence::types::new_resource_id();
        body.as_object_mut()
            .ok_or_else(|| HtsError::InvalidRequest("Resource must be a JSON object".into()))?
            .insert("id".into(), Value::String(id.clone()));
        clear_server_meta(&mut body);
        crate::import::parse_crud_resource(resource_type, &body)?;
        let kind = resource_kind(resource_type)?;
        let mut transaction = crud_transaction::begin(&state, kind, &id).await?;
        let result = async {
            transaction.prepare(kind, &id, Some(&body)).await?;
            if let Some(id) = body.get("id").and_then(Value::as_str)
                && transaction.normalized_resource(kind, id).await?.is_some()
            {
                return Err(HtsError::InvalidRequest(format!(
                    "{resource_type}/{id} already exists"
                )));
            }
            let stored = transaction
                .create(resource_type, body)
                .await
                .map_err(storage_error)?;
            transaction
                .normalize(kind, stored.content_with_meta(), false)
                .await?;
            Ok(stored)
        }
        .await;
        let result = finish(transaction, result).await;
        evict_caches_after_write(&state);
        let stored = result?;
        Ok(resource_response(&stored, format, StatusCode::CREATED))
    }

    /// GET /<ResourceType>/:id — read stored FHIR JSON.
    ///
    /// Returns **200 OK**, with an `ETag` for persistence-managed resources,
    /// or **404 Not Found** if absent or deleted.
    async fn read_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        if let Some(store) = state.active_resource_store() {
            match store.read(&ctx(), resource_type, &id).await {
                Ok(Some(resource)) if !resource.is_deleted() => {
                    return Ok(resource_response(&resource, format, StatusCode::OK));
                }
                Ok(Some(_)) => return Err(HtsError::NotFound(format!("{resource_type}/{id}"))),
                Ok(None) => {}
                // A tombstone is authoritative. Never resurrect it from imports.
                Err(helios_persistence::StorageError::Resource(
                    helios_persistence::error::ResourceError::Gone { .. },
                )) => return Err(HtsError::NotFound(format!("{resource_type}/{id}"))),
                Err(helios_persistence::StorageError::Resource(
                    helios_persistence::error::ResourceError::NotFound { .. },
                )) => {}
                Err(error) => return Err(HtsError::StorageError(error.to_string())),
            }
        }
        state
            .backend()
            .resource_by_id(resource_type, &id)?
            .map(|resource| fhir_respond(resource, format))
            .ok_or_else(|| HtsError::NotFound(format!("{resource_type}/{id}")))
    }

    /// Read the named persistence revision, never substituting current or
    /// import-only content for a version that has no history entry.
    async fn read_resource_version<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        version: String,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let identity = format!("{resource_type}/{id}/_history/{version}");
        let store = state
            .active_versioned_resource_store()
            .ok_or_else(|| HtsError::NotFound(identity.clone()))?;
        let stored = store
            .vread(&ctx(), resource_type, &id, &version)
            .await
            .map_err(storage_error)?
            .ok_or_else(|| HtsError::NotFound(identity.clone()))?;
        if stored.is_deleted() {
            return Err(HtsError::Gone(identity));
        }
        Ok(resource_response(&stored, format, StatusCode::OK))
    }

    /// Create, update or restore a logical resource atomically, retaining its
    /// history and keeping normalized terminology in the same transaction.
    async fn update_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        if_match: Option<String>,
        mut body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        if body.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            return Err(HtsError::InvalidRequest(
                "Body id must be present and match the request URL".into(),
            ));
        }
        clear_server_meta(&mut body);
        crate::import::parse_crud_resource(resource_type, &body)?;
        let kind = resource_kind(resource_type)?;
        let mut transaction = crud_transaction::begin(&state, kind, &id).await?;
        let result = async {
            transaction.prepare(kind, &id, Some(&body)).await?;
            let mut current = transaction
                .read_including_deleted(resource_type, &id)
                .await
                .map_err(storage_error)?;
            if let Some(etag) = if_match.as_deref() {
                let live = current.as_ref().filter(|resource| !resource.is_deleted());
                match live {
                    Some(resource) if resource.matches_etag(etag) => {}
                    Some(resource) => {
                        return Err(HtsError::PreconditionFailed(format!(
                            "ETag mismatch: supplied {etag}, current {}",
                            resource.etag()
                        )));
                    }
                    None => {
                        return Err(HtsError::PreconditionFailed(
                            "If-Match requires an existing resource with a persisted ETag".into(),
                        ));
                    }
                }
            }
            // Import-only content establishes version 1 before the update,
            // preserving the original resource in persistence history.
            if current.is_none()
                && let Some(original) = transaction.normalized_resource(kind, &id).await?
            {
                current = Some(
                    transaction
                        .create(resource_type, original)
                        .await
                        .map_err(storage_error)?,
                );
            }
            let replace = current.is_some();
            let (stored, status) = match current {
                Some(current) if current.is_deleted() => (
                    transaction
                        .restore(&current, body)
                        .await
                        .map_err(storage_error)?,
                    StatusCode::CREATED,
                ),
                Some(current) => (
                    transaction
                        .update(&current, body)
                        .await
                        .map_err(storage_error)?,
                    StatusCode::OK,
                ),
                None => (
                    transaction
                        .create(resource_type, body)
                        .await
                        .map_err(storage_error)?,
                    StatusCode::CREATED,
                ),
            };
            transaction
                .normalize(kind, stored.content_with_meta(), replace)
                .await?;
            Ok((stored, status))
        }
        .await;
        let result = finish(transaction, result).await;
        evict_caches_after_write(&state);
        let (stored, status) = result?;
        Ok(resource_response(&stored, format, status))
    }

    /// Delete the resource and its exact normalized identity in one transaction.
    async fn delete_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
    ) -> Result<Response, HtsError> {
        let kind = resource_kind(resource_type)?;
        let mut transaction = crud_transaction::begin(&state, kind, &id).await?;
        let result = async {
            transaction.prepare(kind, &id, None).await?;
            current_resource(&mut *transaction, kind, &id).await?;
            transaction
                .delete(resource_type, &id)
                .await
                .map_err(storage_error)?;
            transaction.remove_normalized(kind, &id).await
        }
        .await;
        let result = finish(transaction, result).await;
        evict_caches_after_write(&state);
        result?;
        Ok(StatusCode::NO_CONTENT.into_response())
    }

    // ── Format negotiation helper ──────────────────────────────────────────────

    fn crud_format(headers: &HeaderMap, raw: Option<&str>) -> ResponseFormat {
        let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
        negotiate_format(raw, accept)
    }

    // ── CodeSystem CRUD ────────────────────────────────────────────────────────

    pub async fn create_code_system<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        create_resource(
            "CodeSystem",
            state,
            body,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn read_code_system<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource(
            "CodeSystem",
            state,
            id,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn read_code_system_version<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path((id, version)): Path<(String, String)>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource_version(
            "CodeSystem",
            state,
            id,
            version,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn update_code_system<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        let format = crud_format(&headers, raw.as_deref());
        let if_match = headers
            .get(header::IF_MATCH)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        update_resource("CodeSystem", state, id, if_match, body, format).await
    }

    pub async fn delete_code_system<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
    ) -> Result<Response, HtsError> {
        delete_resource("CodeSystem", state, id).await
    }

    // ── ValueSet CRUD ──────────────────────────────────────────────────────────

    pub async fn create_value_set<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        create_resource(
            "ValueSet",
            state,
            body,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn read_value_set<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource("ValueSet", state, id, crud_format(&headers, raw.as_deref())).await
    }

    pub async fn read_value_set_version<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path((id, version)): Path<(String, String)>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource_version(
            "ValueSet",
            state,
            id,
            version,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn update_value_set<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        let format = crud_format(&headers, raw.as_deref());
        let if_match = headers
            .get(header::IF_MATCH)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        update_resource("ValueSet", state, id, if_match, body, format).await
    }

    pub async fn delete_value_set<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
    ) -> Result<Response, HtsError> {
        delete_resource("ValueSet", state, id).await
    }

    // ── ConceptMap CRUD ────────────────────────────────────────────────────────

    pub async fn create_concept_map<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        create_resource(
            "ConceptMap",
            state,
            body,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn read_concept_map<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource(
            "ConceptMap",
            state,
            id,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn read_concept_map_version<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path((id, version)): Path<(String, String)>,
        headers: HeaderMap,
        RawQuery(raw): RawQuery,
    ) -> Result<Response, HtsError> {
        read_resource_version(
            "ConceptMap",
            state,
            id,
            version,
            crud_format(&headers, raw.as_deref()),
        )
        .await
    }

    pub async fn update_concept_map<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
        RawQuery(raw): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Response, HtsError> {
        let format = crud_format(&headers, raw.as_deref());
        let if_match = headers
            .get(header::IF_MATCH)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        update_resource("ConceptMap", state, id, if_match, body, format).await
    }

    pub async fn delete_concept_map<B: TerminologyBackend>(
        State(state): State<AppState<B>>,
        Path(id): Path<String>,
    ) -> Result<Response, HtsError> {
        delete_resource("ConceptMap", state, id).await
    }
}

// Re-export when any storage backend feature is active.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub use inner::*;

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        response::Response,
        routing::{delete, get, post, put},
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use helios_persistence::backends::sqlite::SqliteBackend;

    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::state::AppState;

    use super::inner::*;

    // ── Helpers ────────────────────────────────────────────────────────────────

    fn make_state() -> (TempDir, AppState<SqliteTerminologyBackend>) {
        let directory = tempfile::tempdir().expect("temporary SQLite directory");
        let path = directory.path().join("crud.db");
        let resource_store = SqliteBackend::open(&path).expect("open persistence store");
        resource_store
            .init_schema()
            .expect("init persistence schema");
        let backend = SqliteTerminologyBackend::new(path.to_str().expect("UTF-8 database path"))
            .expect("open terminology store");
        let state = AppState::new(backend).with_resource_store(resource_store);
        (directory, state)
    }

    fn code_system_router() -> (TempDir, Router) {
        let (directory, state) = make_state();
        let router = Router::new()
            .route(
                "/CodeSystem",
                post(create_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                get(read_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                put(update_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                delete(delete_code_system::<SqliteTerminologyBackend>),
            )
            .with_state(state);
        (directory, router)
    }

    fn value_set_router() -> (TempDir, Router) {
        let (directory, state) = make_state();
        let router = Router::new()
            .route(
                "/ValueSet",
                post(create_value_set::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet/{id}",
                get(read_value_set::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet/{id}",
                put(update_value_set::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet/{id}",
                delete(delete_value_set::<SqliteTerminologyBackend>),
            )
            .with_state(state);
        (directory, router)
    }

    fn concept_map_router() -> (TempDir, Router) {
        let (directory, state) = make_state();
        let router = Router::new()
            .route(
                "/ConceptMap",
                post(create_concept_map::<SqliteTerminologyBackend>),
            )
            .route(
                "/ConceptMap/{id}",
                get(read_concept_map::<SqliteTerminologyBackend>),
            )
            .route(
                "/ConceptMap/{id}",
                put(update_concept_map::<SqliteTerminologyBackend>),
            )
            .route(
                "/ConceptMap/{id}",
                delete(delete_concept_map::<SqliteTerminologyBackend>),
            )
            .with_state(state);
        (directory, router)
    }

    fn cs_body(id: &str) -> Value {
        json!({
            "resourceType": "CodeSystem",
            "id": id,
            "url": format!("http://example.org/cs/{id}"),
            "version": "1.0",
            "name": "TestCS",
            "status": "active",
            "content": "complete",
            "concept": [
                { "code": "A", "display": "Concept A" },
                { "code": "B", "display": "Concept B" }
            ]
        })
    }

    fn vs_body(id: &str) -> Value {
        json!({
            "resourceType": "ValueSet",
            "id": id,
            "url": format!("http://example.org/vs/{id}"),
            "name": "TestVS",
            "status": "active"
        })
    }

    fn cm_body(id: &str) -> Value {
        json!({
            "resourceType": "ConceptMap",
            "id": id,
            "url": format!("http://example.org/cm/{id}"),
            "name": "TestCM",
            "status": "active"
        })
    }

    async fn json_body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ── Cache-invalidation harness (issue #304) ────────────────────────────────
    //
    // These tests need CRUD handlers and *operation* handlers sharing ONE
    // `AppState`, because the whole defect lives in the caches that state owns.
    // Building a router per step, or per request, would hand every request a
    // fresh set of empty caches and the tests would pass on unfixed code.

    /// Router wiring CRUD writes and the `$expand` / `$lookup` reads that read
    /// through the caches, all over a single shared state.
    fn ops_router(state: AppState<SqliteTerminologyBackend>) -> Router {
        use crate::operations::expand::get_expand_handler;
        use crate::operations::lookup::get_lookup_handler;

        Router::new()
            // Static operation paths before `/{id}`, mirroring `server.rs`.
            .route(
                "/ValueSet/$expand",
                get(get_expand_handler::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/$lookup",
                get(get_lookup_handler::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem",
                post(create_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                put(update_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet",
                post(create_value_set::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet/{id}",
                put(update_value_set::<SqliteTerminologyBackend>)
                    .delete(delete_value_set::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn send_json(app: &Router, method: &str, uri: &str, body: Value) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn send(app: &Router, method: &str, uri: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// A CodeSystem with a caller-chosen concept list.
    fn cs_with(id: &str, concept: Value) -> Value {
        json!({
            "resourceType": "CodeSystem",
            "id": id,
            "url": format!("http://example.org/cs/{id}"),
            "version": "1.0",
            "name": "TestCS",
            "status": "active",
            "content": "complete",
            "concept": concept
        })
    }

    /// A ValueSet enumerating `codes` from `system`, optionally versioned.
    fn vs_including(id: &str, version: Option<&str>, system: &str, codes: &[&str]) -> Value {
        let concept: Vec<Value> = codes.iter().map(|c| json!({ "code": c })).collect();
        let mut vs = json!({
            "resourceType": "ValueSet",
            "id": id,
            "url": format!("http://example.org/vs/{id}"),
            "name": "TestVS",
            "status": "active",
            "compose": { "include": [{ "system": system, "concept": concept }] }
        });
        if let Some(v) = version {
            vs["version"] = json!(v);
        }
        vs
    }

    /// Pull a named `valueString` out of a FHIR `Parameters` response.
    fn param_str(params: &Value, name: &str) -> Option<String> {
        params["parameter"]
            .as_array()?
            .iter()
            .find(|p| p["name"] == name)
            .and_then(|p| p["valueString"].as_str())
            .map(str::to_owned)
    }

    /// The codes in an `$expand` response, in response order.
    fn expansion_codes(body: &Value) -> Vec<String> {
        body["expansion"]["contains"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|c| c["code"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Creating a ValueSet must un-cache the *absence* an earlier `$expand`
    /// recorded for its URL.
    ///
    /// Before the fix `not_found_urls` was cleared only by `POST /import` and
    /// the CRUD delete path, so the server kept answering 404 for a ValueSet it
    /// had just returned `201 Created` for — permanently, since nothing expires
    /// that set.
    #[tokio::test]
    async fn create_value_set_evicts_the_expand_negative_cache() {
        let (_directory, state) = make_state();
        let app = ops_router(state.clone());

        // A CodeSystem to include, so the post-create expansion is non-empty.
        assert_eq!(
            send_json(&app, "POST", "/CodeSystem", cs_body("cs-neg"))
                .await
                .status(),
            StatusCode::CREATED
        );

        let vs_url = "http://example.org/vs/vs-neg";
        let expand_uri = format!("/ValueSet/$expand?url={vs_url}");

        // 1. Absent → 404.
        assert_eq!(
            send(&app, "GET", &expand_uri).await.status(),
            StatusCode::NOT_FOUND
        );

        // The load-bearing precondition: the 404 must actually have poisoned the
        // negative cache. Without this assert the test would still pass if the
        // caching path stopped being reached at all, and would then silently
        // stop testing the defect it exists to catch.
        let poisoned = { state.not_found_urls.read().unwrap().contains(vs_url) };
        assert!(
            poisoned,
            "precondition: the 404 must record the URL in not_found_urls, \
             otherwise this test cannot observe the bug"
        );

        // 2. Create exactly that ValueSet.
        assert_eq!(
            send_json(
                &app,
                "POST",
                "/ValueSet",
                vs_including("vs-neg", None, "http://example.org/cs/cs-neg", &["A"])
            )
            .await
            .status(),
            StatusCode::CREATED
        );

        // 3. It must expand now. Before the fix this stayed 404 forever.
        let resp = send(&app, "GET", &expand_uri).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a just-created ValueSet must not still be reported absent"
        );
        assert_eq!(expansion_codes(&json_body(resp).await), vec!["A"]);

        // Negative control: eviction must not amount to disabling 404s.
        assert_eq!(
            send(
                &app,
                "GET",
                "/ValueSet/$expand?url=http://example.org/vs/never-created"
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }

    /// Updating a CodeSystem must un-cache the absence recorded for a code the
    /// update adds — the `$lookup` twin of the `$expand` negative cache.
    #[tokio::test]
    async fn update_code_system_evicts_the_lookup_negative_cache() {
        let (_directory, state) = make_state();
        let app = ops_router(state.clone());

        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/CodeSystem/cs-lk",
                cs_with("cs-lk", json!([{ "code": "A", "display": "Alpha" }]))
            )
            .await
            .status(),
            StatusCode::CREATED
        );

        let lookup_uri = "/CodeSystem/$lookup?system=http://example.org/cs/cs-lk&code=B";
        assert_eq!(
            send(&app, "GET", lookup_uri).await.status(),
            StatusCode::NOT_FOUND
        );

        let poisoned = { !state.lookup_not_found_cache.read().unwrap().is_empty() };
        assert!(
            poisoned,
            "precondition: the 404 must populate lookup_not_found_cache"
        );

        // Add code B.
        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/CodeSystem/cs-lk",
                cs_with(
                    "cs-lk",
                    json!([
                        { "code": "A", "display": "Alpha" },
                        { "code": "B", "display": "Bravo" }
                    ])
                )
            )
            .await
            .status(),
            StatusCode::OK
        );

        assert_eq!(
            send(&app, "GET", lookup_uri).await.status(),
            StatusCode::OK,
            "a code added by PUT must not still be reported unknown"
        );
    }

    /// Updating a CodeSystem must also evict the **backend's** per-instance
    /// response memo, not just the handler-level cache.
    ///
    /// This is the SQLite-specific half of #304 and it is deliberately not
    /// covered by the tests above: `AppState::clear_expand_cache` alone makes
    /// the handler recompute, and the recomputation is then served straight back
    /// from `SqliteTerminologyBackend::lookup_response_cache`, whose key
    /// (`system|code|version|lang|date|props`) is unchanged by a display edit.
    /// CRUD normalizes within the persistence transaction and does not call
    /// `BundleImportBackend::import_bundle`. It must therefore invalidate the
    /// backend memo explicitly as well as the handler-level cache.
    #[tokio::test]
    async fn update_code_system_evicts_the_backend_response_memo() {
        let (_directory, state) = make_state();
        let app = ops_router(state.clone());

        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/CodeSystem/cs-disp",
                cs_with("cs-disp", json!([{ "code": "A", "display": "Alpha" }]))
            )
            .await
            .status(),
            StatusCode::CREATED
        );

        // Warm both cache tiers.
        let warm = json_body(
            send(
                &app,
                "GET",
                "/CodeSystem/$lookup?system=http://example.org/cs/cs-disp&code=A",
            )
            .await,
        )
        .await;
        assert_eq!(param_str(&warm, "display").as_deref(), Some("Alpha"));

        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/CodeSystem/cs-disp",
                cs_with("cs-disp", json!([{ "code": "A", "display": "Alpha v2" }]))
            )
            .await
            .status(),
            StatusCode::OK
        );

        let after = json_body(
            send(
                &app,
                "GET",
                "/CodeSystem/$lookup?system=http://example.org/cs/cs-disp&code=A",
            )
            .await,
        )
        .await;
        assert_eq!(
            param_str(&after, "display").as_deref(),
            Some("Alpha v2"),
            "$lookup must reflect the updated display, not the backend's memo of the old one"
        );
    }

    /// Deleting a *versioned* ValueSet must actually remove it.
    ///
    /// `write_value_set` stores versioned ValueSets under the synthetic id
    /// `<fhir-id>|<version>`, but `delete_value_set` matched `id = ?1` only, so
    /// the delete silently matched nothing: the handler returned `204` and the
    /// ValueSet stayed fully expandable. Every pre-existing CRUD test missed
    /// this because its ValueSet fixtures carry no `version`.
    #[tokio::test]
    async fn deleting_a_versioned_value_set_stops_it_expanding() {
        let (_directory, state) = make_state();
        let app = ops_router(state.clone());

        assert_eq!(
            send_json(&app, "POST", "/CodeSystem", cs_body("cs-ver"))
                .await
                .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/ValueSet/vs-ver",
                vs_including(
                    "vs-ver",
                    Some("1.0.0"),
                    "http://example.org/cs/cs-ver",
                    &["A"]
                )
            )
            .await
            .status(),
            StatusCode::CREATED
        );

        let expand_uri = "/ValueSet/$expand?url=http://example.org/vs/vs-ver";
        assert_eq!(
            send(&app, "GET", expand_uri).await.status(),
            StatusCode::OK,
            "precondition: the versioned ValueSet expands before deletion"
        );

        assert_eq!(
            send(&app, "DELETE", "/ValueSet/vs-ver").await.status(),
            StatusCode::NO_CONTENT
        );

        assert_eq!(
            send(&app, "GET", expand_uri).await.status(),
            StatusCode::NOT_FOUND,
            "a deleted ValueSet must not keep expanding"
        );
    }

    /// Updating a ValueSet must not keep serving the previous expansion.
    ///
    /// Two independent defects could produce a stale answer here: the versioned
    /// storage-id mismatch above (so the delete-then-reimport never deleted),
    /// and `write_value_set` retaining `value_set_expansions` rows materialized
    /// from the old compose. Both are fixed; this asserts the observable
    /// behaviour rather than either mechanism.
    #[tokio::test]
    async fn updating_a_value_set_refreshes_its_expansion() {
        let (_directory, state) = make_state();
        let app = ops_router(state.clone());

        let system = "http://example.org/cs/cs-upd";
        assert_eq!(
            send_json(&app, "POST", "/CodeSystem", cs_body("cs-upd"))
                .await
                .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/ValueSet/vs-upd",
                vs_including("vs-upd", Some("1.0.0"), system, &["A"])
            )
            .await
            .status(),
            StatusCode::CREATED
        );

        let expand_uri = "/ValueSet/$expand?url=http://example.org/vs/vs-upd";
        // Materialize the expansion for the original compose.
        let before = json_body(send(&app, "GET", expand_uri).await).await;
        assert_eq!(expansion_codes(&before), vec!["A"]);

        // Swap the enumerated concept A → B.
        assert_eq!(
            send_json(
                &app,
                "PUT",
                "/ValueSet/vs-upd",
                vs_including("vs-upd", Some("1.0.0"), system, &["B"])
            )
            .await
            .status(),
            StatusCode::OK
        );

        let after = json_body(send(&app, "GET", expand_uri).await).await;
        assert_eq!(
            expansion_codes(&after),
            vec!["B"],
            "the expansion must follow the updated compose"
        );
    }

    // ── CodeSystem CRUD round-trip ─────────────────────────────────────────────

    #[tokio::test]
    async fn cs_create_returns_201_with_etag() {
        let (_directory, app) = code_system_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs1").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        assert!(resp.headers().contains_key(header::ETAG));
        assert!(resp.headers().contains_key(header::LAST_MODIFIED));
        let location = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned();
        let body = json_body(resp).await;
        let id = body["id"].as_str().unwrap();
        assert_ne!(id, "cs1");
        assert_eq!(body["meta"]["versionId"], "1");
        assert!(body["meta"]["lastUpdated"].is_string());
        assert_eq!(location, format!("/CodeSystem/{id}/_history/1"));
    }

    #[tokio::test]
    async fn cs_read_after_create_returns_200() {
        let (_directory, app) = code_system_router();
        // POST
        let post_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-read").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::CREATED);
        let etag = post_resp
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let created = json_body(post_resp).await;
        let id = created["id"].as_str().unwrap();
        assert_ne!(id, "cs-read");

        // GET
        let get_resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/CodeSystem/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);
        assert_eq!(get_resp.headers()[header::ETAG], etag);
        let body = json_body(get_resp).await;
        assert_eq!(body["id"], id);
        assert_eq!(body["resourceType"], "CodeSystem");
    }

    #[tokio::test]
    async fn cs_get_nonexistent_returns_404() {
        let (_directory, app) = code_system_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/CodeSystem/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cs_full_crud_round_trip() {
        let (_directory, app) = code_system_router();

        // 1. POST → 201
        let post_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-rt").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::CREATED);
        let etag_v1 = post_resp
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let created = json_body(post_resp).await;
        let id = created["id"].as_str().unwrap();
        let uri = format!("/CodeSystem/{id}");

        // 2. GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        // 3. PUT → 200 with incremented ETag
        let updated = json!({
            "resourceType": "CodeSystem",
            "id": id,
            "url": "http://example.org/cs/cs-rt",
            "name": "UpdatedCS",
            "status": "active",
            "content": "complete",
            "concept": [{ "code": "X", "display": "Concept X" }]
        });
        let put_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .header(header::IF_MATCH, &etag_v1)
                    .body(Body::from(updated.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_resp.status(), StatusCode::OK);
        let etag_v2 = put_resp
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(etag_v1, etag_v2, "version should be incremented");

        // 4. GET after PUT → updated content
        let get2_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get2_resp.status(), StatusCode::OK);
        let body2 = json_body(get2_resp).await;
        assert_eq!(body2["name"], "UpdatedCS");

        // 5. DELETE → 204
        let del_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

        // 6. GET after DELETE → 404
        let get3_resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get3_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cs_put_with_wrong_etag_returns_412() {
        let (_directory, app) = code_system_router();

        // Create the exact ID before testing a live-resource precondition.
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/CodeSystem/cs-etag")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-etag").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(created.status(), StatusCode::CREATED);

        // PUT with wrong ETag
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/CodeSystem/cs-etag")
                    .header("content-type", "application/json")
                    .header(header::IF_MATCH, "W/\"999\"")
                    .body(Body::from(cs_body("cs-etag").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn cs_create_indexes_into_hts_tables() {
        let (_directory, state) = make_state();
        let hts_pool = state.backend().pool().clone();

        let app = Router::new()
            .route(
                "/CodeSystem",
                post(create_code_system::<SqliteTerminologyBackend>),
            )
            .with_state(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-index").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let created = json_body(resp).await;
        let public_id = created["id"].as_str().unwrap();

        // Verify HTS normalized tables were populated. The synthetic storage
        // id (`<fhir-id>|<version>`) is opaque, so we look up by URL.
        let conn = hts_pool.get().unwrap();
        let (storage_id, normalized_id): (String, String) = conn
            .query_row(
                "SELECT id, json_extract(resource_json, '$.id') FROM code_systems
                 WHERE url = 'http://example.org/cs/cs-index'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(normalized_id, public_id);

        let concept_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id = ?1",
                [&storage_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 2, "both concepts should be indexed");
    }

    #[tokio::test]
    async fn cs_delete_removes_hts_normalized_rows() {
        let (_directory, state) = make_state();
        let hts_pool = state.backend().pool().clone();

        let app = Router::new()
            .route(
                "/CodeSystem",
                post(create_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                delete(delete_code_system::<SqliteTerminologyBackend>),
            )
            .with_state(state);

        // Create
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-del").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json_body(created).await;
        let id = created["id"].as_str().unwrap();
        let storage_id: String = hts_pool
            .get()
            .unwrap()
            .query_row(
                "SELECT id FROM code_systems WHERE url = 'http://example.org/cs/cs-del'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // Delete
        let del = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/CodeSystem/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);

        // Verify normalized rows are gone. Match by URL to avoid coupling to
        // the synthetic storage-id format.
        let conn = hts_pool.get().unwrap();
        let cs_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_systems WHERE url = 'http://example.org/cs/cs-del'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cs_count, 0, "code_systems row should be deleted");

        let concept_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id = ?1",
                [&storage_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 0, "concepts should be cascade-deleted");
    }

    // ── ValueSet CRUD round-trip ───────────────────────────────────────────────

    #[tokio::test]
    async fn vs_full_crud_round_trip() {
        let (_directory, app) = value_set_router();

        // POST → 201
        let post_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ValueSet")
                    .header("content-type", "application/json")
                    .body(Body::from(vs_body("vs-rt").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::CREATED);

        let created = json_body(post_resp).await;
        let id = created["id"].as_str().unwrap();
        let uri = format!("/ValueSet/{id}");

        // GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        // DELETE → 204
        let del_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

        // GET after DELETE → 404
        let get2 = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get2.status(), StatusCode::NOT_FOUND);
    }

    // ── ConceptMap CRUD round-trip ─────────────────────────────────────────────

    #[tokio::test]
    async fn cm_full_crud_round_trip() {
        let (_directory, app) = concept_map_router();

        // POST → 201
        let post_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ConceptMap")
                    .header("content-type", "application/json")
                    .body(Body::from(cm_body("cm-rt").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::CREATED);

        let created = json_body(post_resp).await;
        let id = created["id"].as_str().unwrap();
        let uri = format!("/ConceptMap/{id}");

        // GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        // DELETE → 204
        let del_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

        // GET after DELETE → 404
        let get2 = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get2.status(), StatusCode::NOT_FOUND);
    }

    // ── Integration: POST CodeSystem → $lookup ─────────────────────────────────

    #[tokio::test]
    async fn post_code_system_then_lookup_works() {
        use crate::traits::CodeSystemOperations;
        use crate::types::LookupRequest;
        use helios_persistence::tenant::TenantContext;

        let (_directory, state) = make_state();

        let app = Router::new()
            .route(
                "/CodeSystem",
                post(create_code_system::<SqliteTerminologyBackend>),
            )
            .with_state(state.clone());

        // POST the CodeSystem
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-lookup").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // $lookup via the backend directly (terminology tables should be populated)
        let ctx = TenantContext::system();
        let result = state
            .backend()
            .lookup(
                &ctx,
                LookupRequest {
                    system: "http://example.org/cs/cs-lookup".to_string(),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.display, Some("Concept A".into()));
    }

    // ── DELETE of content loaded through POST /import (issue #2143) ───────────
    //
    // `POST /import` writes only the HTS normalized tables, never a persistence
    // store row, so DELETE used to fail on the unconditional store delete
    // (500 "resource not found") and leave the content served.

    /// Router wiring `POST /import`, the reads that prove content is served,
    /// and DELETE for all three types, over one shared state.
    fn import_delete_router(state: AppState<SqliteTerminologyBackend>) -> Router {
        use crate::operations::expand::get_expand_handler;
        use crate::operations::import_bundle::import_handler;
        use crate::operations::lookup::get_lookup_handler;

        Router::new()
            .route("/import", post(import_handler::<SqliteTerminologyBackend>))
            .route(
                "/ValueSet/$expand",
                get(get_expand_handler::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/$lookup",
                get(get_lookup_handler::<SqliteTerminologyBackend>),
            )
            .route(
                "/CodeSystem/{id}",
                delete(delete_code_system::<SqliteTerminologyBackend>),
            )
            .route(
                "/ValueSet/{id}",
                delete(delete_value_set::<SqliteTerminologyBackend>),
            )
            .route(
                "/ConceptMap/{id}",
                delete(delete_concept_map::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    /// A versioned CodeSystem, a versioned ValueSet over it, and a ConceptMap —
    /// the issue's reproduction plus a ConceptMap.
    fn imported_bundle() -> Value {
        json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {"resource": {
                    "resourceType": "CodeSystem", "id": "imported-cs",
                    "url": "http://example.org/imported-cs", "version": "1.0",
                    "status": "active", "content": "complete",
                    "concept": [{"code": "RETIRED"}]
                }},
                {"resource": {
                    "resourceType": "ValueSet", "id": "imported-vs",
                    "url": "http://example.org/imported-vs", "version": "1.0",
                    "status": "active",
                    "compose": {"include": [{"system": "http://example.org/imported-cs"}]}
                }},
                {"resource": {
                    "resourceType": "ConceptMap", "id": "imported-cm",
                    "url": "http://example.org/imported-cm", "status": "active",
                    "group": [{
                        "source": "http://example.org/imported-cs",
                        "target": "http://example.org/other-cs",
                        "element": [{"code": "RETIRED",
                            "target": [{"code": "X", "equivalence": "equivalent"}]}]
                    }]
                }}
            ]
        })
    }

    fn concept_map_rows(state: &AppState<SqliteTerminologyBackend>) -> i64 {
        state
            .backend()
            .pool()
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM concept_maps WHERE url = 'http://example.org/imported-cm'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn deleting_imported_resources_removes_them() {
        let (_directory, state) = make_state();
        let app = import_delete_router(state.clone());

        assert_eq!(
            send_json(&app, "POST", "/import", imported_bundle())
                .await
                .status(),
            StatusCode::OK
        );

        let lookup_uri = "/CodeSystem/$lookup?system=http://example.org/imported-cs&code=RETIRED";
        let expand_uri = "/ValueSet/$expand?url=http://example.org/imported-vs";
        assert_eq!(send(&app, "GET", lookup_uri).await.status(), StatusCode::OK);
        assert_eq!(send(&app, "GET", expand_uri).await.status(), StatusCode::OK);
        assert_eq!(concept_map_rows(&state), 1, "precondition: map imported");

        // ValueSet first: its expansion depends on the CodeSystem, so deleting
        // the CodeSystem first would make `$expand` fail for the wrong reason.
        assert_eq!(
            send(&app, "DELETE", "/ValueSet/imported-vs").await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(&app, "GET", expand_uri).await.status(),
            StatusCode::NOT_FOUND,
            "an imported ValueSet must stop expanding once deleted"
        );

        assert_eq!(
            send(&app, "DELETE", "/CodeSystem/imported-cs")
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(&app, "GET", lookup_uri).await.status(),
            StatusCode::NOT_FOUND,
            "an imported CodeSystem must stop answering $lookup once deleted"
        );

        assert_eq!(
            send(&app, "DELETE", "/ConceptMap/imported-cm")
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            concept_map_rows(&state),
            0,
            "imported ConceptMap must be gone"
        );
    }
}
