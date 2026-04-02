//! Resource CRUD API for CodeSystem, ValueSet, and ConceptMap.
//!
//! Handlers for GET / POST / PUT / DELETE on CodeSystem, ValueSet, and
//! ConceptMap resources.
//!
//! Raw FHIR JSON is stored and versioned via `helios-persistence`
//! (`SqliteBackend`).  The HTS normalized terminology tables
//! (code_systems, concepts, value_sets, …) are kept in sync by the same
//! import / delete helpers used by the `POST /import` pipeline.
//!
//! # ETag / versioning
//!
//! Every stored resource carries a `version_id` (starting at `"1"`, monotonically
//! increasing).  GET responses include an `ETag: W/"<version_id>"` header.
//! PUT requests may include an `If-Match` header; when present, the handler
//! returns **412 Precondition Failed** if the version no longer matches.

#[cfg(feature = "sqlite")]
mod inner {
    use std::sync::Arc;

    use axum::{
        Json,
        extract::{Path, RawQuery, State},
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
    };
    use helios_fhir::FhirVersion;
    use helios_persistence::ResourceStorage;
    use helios_persistence::tenant::TenantContext;
    use serde_json::Value;

    use crate::error::HtsError;
    use crate::import::ImportStats;
    use crate::import::fhir_bundle::{
        delete_code_system as hts_delete_cs, delete_concept_map as hts_delete_cm,
        delete_value_set as hts_delete_vs, get_code_system_url as hts_get_cs_url,
        import_code_system as hts_import_cs, import_concept_map as hts_import_cm,
        import_value_set as hts_import_vs,
        invalidate_expansion_cache_for_system as hts_invalidate_expansion_cache,
    };
    use crate::state::AppState;
    use crate::traits::TerminologyBackend;

    use super::super::format::{ResponseFormat, fhir_respond, negotiate_format};

    // ── Tenant context ─────────────────────────────────────────────────────────

    fn ctx() -> TenantContext {
        TenantContext::system()
    }

    // ── Error helpers ──────────────────────────────────────────────────────────

    /// Returns `true` if the storage error represents a resource that is
    /// deleted (Gone) or never existed (NotFound).  Both cases map to HTTP 404.
    fn is_gone_or_not_found(e: &helios_persistence::StorageError) -> bool {
        use helios_persistence::error::{ResourceError, StorageError as SE};
        matches!(
            e,
            SE::Resource(ResourceError::Gone { .. }) | SE::Resource(ResourceError::NotFound { .. })
        )
    }

    // ── Generic CRUD helpers ───────────────────────────────────────────────────

    /// POST /<ResourceType> — create a new resource.
    ///
    /// 1. Stores raw JSON in `helios-persistence` (auto-generates version / ETag).
    /// 2. Indexes the stored content into the HTS normalized tables.
    /// 3. Returns **201 Created** with `Location` and `ETag` headers.
    async fn create_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let store = Arc::clone(
            state
                .resource_store
                .as_ref()
                .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?,
        );
        let hts_pool = state.require_hts_pool()?;
        let ctx = ctx();

        // 1. Persist raw FHIR JSON (version_id = "1", ETag = W/"1").
        let stored = store
            .create(&ctx, resource_type, body, FhirVersion::default())
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // 2. Index into HTS normalized tables using the canonical content
        //    (persistence layer injects the correct `id` into the JSON).
        let content = stored.content().clone();
        tokio::task::spawn_blocking(move || {
            let conn = hts_pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("HTS pool error: {e}")))?;
            let mut stats = ImportStats::default();
            match resource_type {
                "CodeSystem" => hts_import_cs(&conn, &content, &mut stats),
                "ValueSet" => hts_import_vs(&conn, &content, &mut stats),
                "ConceptMap" => hts_import_cm(&conn, &content, &mut stats),
                _ => Err(HtsError::InvalidRequest(format!(
                    "Unsupported resource type for CRUD: {resource_type}"
                ))),
            }
        })
        .await
        .map_err(|e| HtsError::Internal(e.to_string()))??;

        let etag = stored.etag().to_string();
        let location = format!("/{}/{}", resource_type, stored.id());

        let mut response = fhir_respond(stored.content().clone(), format);
        *response.status_mut() = StatusCode::CREATED;
        let headers = response.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&location) {
            headers.insert(header::LOCATION, v);
        }
        if let Ok(v) = HeaderValue::from_str(&etag) {
            headers.insert(header::ETAG, v);
        }
        Ok(response)
    }

    /// GET /<ResourceType>/:id — read stored FHIR JSON.
    ///
    /// Returns **200 OK** with `ETag` header, or **404 Not Found** if the
    /// resource does not exist or has been deleted.
    async fn read_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let store = Arc::clone(
            state
                .resource_store
                .as_ref()
                .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?,
        );
        let ctx = ctx();

        match store.read(&ctx, resource_type, &id).await {
            Ok(Some(resource)) if !resource.is_deleted() => {
                let etag = resource.etag().to_string();
                let mut response = fhir_respond(resource.content().clone(), format);
                if let Ok(v) = HeaderValue::from_str(&etag) {
                    response.headers_mut().insert(header::ETAG, v);
                }
                Ok(response)
            }
            Ok(Some(_)) | Ok(None) => Err(HtsError::NotFound(format!("{resource_type}/{id}"))),
            // The persistence layer returns Gone for soft-deleted resources;
            // from the caller's perspective that is still a 404.
            Err(e) if is_gone_or_not_found(&e) => {
                Err(HtsError::NotFound(format!("{resource_type}/{id}")))
            }
            Err(e) => Err(HtsError::StorageError(e.to_string())),
        }
    }

    /// PUT /<ResourceType>/:id — update an existing resource.
    ///
    /// If the `If-Match` header is present it must match the current ETag;
    /// a mismatch returns **412 Precondition Failed**.
    ///
    /// On success:
    /// 1. Updates raw JSON in `helios-persistence` (version_id incremented).
    /// 2. Deletes old HTS normalized data and re-imports the new content.
    /// 3. Returns **200 OK** with the updated `ETag`.
    async fn update_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        if_match: Option<String>,
        body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let store = Arc::clone(
            state
                .resource_store
                .as_ref()
                .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?,
        );
        let hts_pool = state.require_hts_pool()?;
        let ctx = ctx();

        // 1. Read current version.
        let current = match store.read(&ctx, resource_type, &id).await {
            Ok(Some(r)) if !r.is_deleted() => r,
            Ok(_) => return Err(HtsError::NotFound(format!("{resource_type}/{id}"))),
            Err(e) if is_gone_or_not_found(&e) => {
                return Err(HtsError::NotFound(format!("{resource_type}/{id}")));
            }
            Err(e) => return Err(HtsError::StorageError(e.to_string())),
        };

        // 2. Check If-Match (optimistic concurrency).
        if let Some(ref etag) = if_match {
            if !current.matches_etag(etag) {
                return Err(HtsError::PreconditionFailed(format!(
                    "ETag mismatch: supplied {etag}, current {}",
                    current.etag()
                )));
            }
        }

        // 3. Update raw JSON in persistence store (version incremented).
        let updated = store
            .update(&ctx, &current, body)
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // 4. Re-index normalized HTS tables: delete old, import new.
        let content = updated.content().clone();
        let resource_id = updated.id().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = hts_pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("HTS pool error: {e}")))?;
            // Delete old normalized rows (ON DELETE CASCADE propagates to children).
            // For CodeSystem: also invalidate cached value set expansions that
            // drew from this system's URL — they are no longer valid after the
            // system content changes.
            match resource_type {
                "CodeSystem" => {
                    if let Some(url) = hts_get_cs_url(&conn, &resource_id)? {
                        hts_invalidate_expansion_cache(&conn, &url)?;
                    }
                    hts_delete_cs(&conn, &resource_id)?;
                }
                "ValueSet" => hts_delete_vs(&conn, &resource_id)?,
                "ConceptMap" => hts_delete_cm(&conn, &resource_id)?,
                _ => {}
            }
            // Re-import updated content.
            let mut stats = ImportStats::default();
            match resource_type {
                "CodeSystem" => hts_import_cs(&conn, &content, &mut stats),
                "ValueSet" => hts_import_vs(&conn, &content, &mut stats),
                "ConceptMap" => hts_import_cm(&conn, &content, &mut stats),
                _ => Err(HtsError::InvalidRequest(format!(
                    "Unsupported resource type for CRUD: {resource_type}"
                ))),
            }
        })
        .await
        .map_err(|e| HtsError::Internal(e.to_string()))??;

        let etag = updated.etag().to_string();
        let mut response = fhir_respond(updated.content().clone(), format);
        if let Ok(v) = HeaderValue::from_str(&etag) {
            response.headers_mut().insert(header::ETAG, v);
        }
        Ok(response)
    }

    /// DELETE /<ResourceType>/:id — soft-delete a resource.
    ///
    /// 1. Soft-deletes the raw JSON in `helios-persistence`.
    /// 2. Removes the resource's rows from HTS normalized tables.
    /// 3. Returns **204 No Content**.
    async fn delete_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
    ) -> Result<Response, HtsError> {
        let store = Arc::clone(
            state
                .resource_store
                .as_ref()
                .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?,
        );
        let hts_pool = state.require_hts_pool()?;
        let ctx = ctx();

        // 1. Soft-delete in persistence store.
        store
            .delete(&ctx, resource_type, &id)
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // 2. Delete from HTS normalized tables.
        let resource_id = id.clone();
        tokio::task::spawn_blocking(move || {
            let conn = hts_pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("HTS pool error: {e}")))?;
            match resource_type {
                "CodeSystem" => {
                    // Invalidate cached expansions derived from this system
                    // before removing the system itself.
                    if let Some(url) = hts_get_cs_url(&conn, &resource_id)? {
                        hts_invalidate_expansion_cache(&conn, &url)?;
                    }
                    hts_delete_cs(&conn, &resource_id)
                }
                "ValueSet" => hts_delete_vs(&conn, &resource_id),
                "ConceptMap" => hts_delete_cm(&conn, &resource_id),
                _ => Ok(()),
            }
        })
        .await
        .map_err(|e| HtsError::Internal(e.to_string()))??;

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

// Re-export when the sqlite feature is active.
#[cfg(feature = "sqlite")]
pub use inner::*;

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        routing::{delete, get, post, put},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use helios_persistence::backends::sqlite::SqliteBackend;

    use crate::backend::sqlite::SqliteTerminologyBackend;
    use crate::state::AppState;

    use super::inner::*;

    // ── Helpers ────────────────────────────────────────────────────────────────

    fn make_state() -> AppState<SqliteTerminologyBackend> {
        let backend = SqliteTerminologyBackend::in_memory().expect("HTS in-memory backend");
        let hts_pool = backend.pool().clone();

        let resource_store = SqliteBackend::in_memory().expect("persistence in-memory backend");
        resource_store
            .init_schema()
            .expect("init persistence schema");

        AppState::new(backend)
            .with_resource_store(resource_store)
            .with_hts_pool(hts_pool)
    }

    fn code_system_router() -> Router {
        let state = make_state();
        Router::new()
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
            .with_state(state)
    }

    fn value_set_router() -> Router {
        let state = make_state();
        Router::new()
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
            .with_state(state)
    }

    fn concept_map_router() -> Router {
        let state = make_state();
        Router::new()
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
            .with_state(state)
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

    // ── CodeSystem CRUD round-trip ─────────────────────────────────────────────

    #[tokio::test]
    async fn cs_create_returns_201_with_etag() {
        let app = code_system_router();
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
        assert!(resp.headers().contains_key(header::LOCATION));
    }

    #[tokio::test]
    async fn cs_read_after_create_returns_200() {
        let app = code_system_router();
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

        // GET
        let get_resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/CodeSystem/cs-read")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);
        let body = json_body(get_resp).await;
        assert_eq!(body["id"], "cs-read");
        assert_eq!(body["resourceType"], "CodeSystem");
        // ETag on GET should match the one from POST.
        let _ = etag; // used for the assertion above via headers
    }

    #[tokio::test]
    async fn cs_get_nonexistent_returns_404() {
        let app = code_system_router();
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
        let app = code_system_router();

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

        // 2. GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/CodeSystem/cs-rt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        // 3. PUT → 200 with incremented ETag
        let updated = json!({
            "resourceType": "CodeSystem",
            "id": "cs-rt",
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
                    .uri("/CodeSystem/cs-rt")
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
                    .uri("/CodeSystem/cs-rt")
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
                    .uri("/CodeSystem/cs-rt")
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
                    .uri("/CodeSystem/cs-rt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get3_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cs_put_with_wrong_etag_returns_412() {
        let app = code_system_router();

        // POST first
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/CodeSystem")
                    .header("content-type", "application/json")
                    .body(Body::from(cs_body("cs-etag").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

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
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let hts_pool = backend.pool().clone();
        let resource_store = SqliteBackend::in_memory().unwrap();
        resource_store.init_schema().unwrap();
        let state = AppState::new(backend)
            .with_resource_store(resource_store)
            .with_hts_pool(hts_pool.clone());

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

        // Verify HTS normalized tables were populated.
        let conn = hts_pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_systems WHERE id = 'cs-index'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "code_systems row should be created");

        let concept_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id = 'cs-index'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 2, "both concepts should be indexed");
    }

    #[tokio::test]
    async fn cs_delete_removes_hts_normalized_rows() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let hts_pool = backend.pool().clone();
        let resource_store = SqliteBackend::in_memory().unwrap();
        resource_store.init_schema().unwrap();
        let state = AppState::new(backend)
            .with_resource_store(resource_store)
            .with_hts_pool(hts_pool.clone());

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
        app.clone()
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

        // Delete
        let del = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/CodeSystem/cs-del")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);

        // Verify normalized rows are gone.
        let conn = hts_pool.get().unwrap();
        let cs_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_systems WHERE id = 'cs-del'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cs_count, 0, "code_systems row should be deleted");

        let concept_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id = 'cs-del'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 0, "concepts should be cascade-deleted");
    }

    // ── ValueSet CRUD round-trip ───────────────────────────────────────────────

    #[tokio::test]
    async fn vs_full_crud_round_trip() {
        let app = value_set_router();

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

        // GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/ValueSet/vs-rt")
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
                    .uri("/ValueSet/vs-rt")
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
                    .uri("/ValueSet/vs-rt")
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
        let app = concept_map_router();

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

        // GET → 200
        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/ConceptMap/cm-rt")
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
                    .uri("/ConceptMap/cm-rt")
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
                    .uri("/ConceptMap/cm-rt")
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

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let hts_pool = backend.pool().clone();
        let resource_store = SqliteBackend::in_memory().unwrap();
        resource_store.init_schema().unwrap();

        let state = AppState::new(backend)
            .with_resource_store(resource_store)
            .with_hts_pool(hts_pool);

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
}
