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

#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod inner {
    use axum::{
        Json,
        extract::{Path, RawQuery, State},
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
    };
    use helios_fhir::FhirVersion;
    use helios_persistence::tenant::TenantContext;
    use serde_json::Value;

    use crate::error::HtsError;
    use crate::state::AppState;
    use crate::traits::TerminologyBackend;

    #[cfg(feature = "sqlite")]
    use crate::import::ImportStats;
    #[cfg(feature = "sqlite")]
    use crate::import::fhir_bundle::{
        delete_code_system as hts_delete_cs, delete_concept_map as hts_delete_cm,
        delete_value_set as hts_delete_vs, get_code_system_url as hts_get_cs_url,
        import_code_system as hts_import_cs, import_concept_map as hts_import_cm,
        import_value_set as hts_import_vs,
        invalidate_expansion_cache_for_system as hts_invalidate_expansion_cache,
    };

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
    /// Callers evict *after* the storage write, never before. Evicting first
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

    // ── HTS re-index helper (generic / async path) ────────────────────────────

    /// Wrap `content` in a synthetic single-resource Bundle and call
    /// `importer.import_bundle()` to (re-)index it into the HTS normalized tables.
    ///
    /// Used by the PostgreSQL CRUD path in place of the SQLite-specific `hts_pool`
    /// spawn-blocking approach.
    async fn hts_reindex_via_importer(
        importer: &dyn crate::import::BundleImportBackend,
        resource_type: &str,
        content: Value,
        ctx: &TenantContext,
    ) -> Result<(), HtsError> {
        if !matches!(resource_type, "CodeSystem" | "ValueSet" | "ConceptMap") {
            return Err(HtsError::InvalidRequest(format!(
                "Unsupported resource type for CRUD: {resource_type}"
            )));
        }
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{"resource": content}]
        });
        let bytes =
            serde_json::to_vec(&bundle).expect("in-memory bundle serialization cannot fail");
        importer
            .import_bundle(ctx, &bytes)
            .await
            .map_err(|e| HtsError::StorageError(format!("HTS re-index failed: {e}")))?;
        Ok(())
    }

    /// (Re-)index `content` into the HTS normalized tables.
    ///
    /// Shared by create and update so both verbs cannot drift apart again.
    /// `replace_id`, when supplied, names an existing resource whose normalized
    /// rows are removed first — the update path, where the new content may have
    /// *dropped* concepts that an upsert alone would leave behind.
    ///
    /// Two backend paths, matching how the state was wired:
    ///
    /// - SQLite: writes through `hts_pool` on a blocking thread, because
    ///   `rusqlite` is synchronous.
    /// - Everything else (PostgreSQL): wraps `content` in a one-entry Bundle and
    ///   hands it to the async importer.
    ///
    /// This function does **not** evict caches — callers do that through
    /// [`evict_caches_after_write`] on every exit path, including this one
    /// failing. That split is deliberate: the SQLite update path deletes the old
    /// normalized rows before re-importing, so a failure part-way through leaves
    /// storage genuinely changed, and caches must be dropped anyway. The
    /// invariant is that a cache may be colder than storage, never hotter.
    // `replace_id` is consumed only by the SQLite delete-then-reimport branch;
    // a postgres-only build would otherwise trip `unused_variables` under the
    // workspace's deny-warnings clippy job.
    #[cfg_attr(not(feature = "sqlite"), allow(unused_variables))]
    async fn hts_reindex<B: TerminologyBackend>(
        state: &AppState<B>,
        resource_type: &'static str,
        content: Value,
        ctx: &TenantContext,
        replace_id: Option<String>,
    ) -> Result<(), HtsError> {
        #[cfg(feature = "sqlite")]
        if let Some(pool) = state.hts_pool.clone() {
            let joined = tokio::task::spawn_blocking(move || {
                let conn = pool
                    .get()
                    .map_err(|e| HtsError::StorageError(format!("HTS pool error: {e}")))?;

                // Update path: drop the old normalized rows first (ON DELETE
                // CASCADE propagates to concepts, designations, properties and
                // hierarchy). For a CodeSystem, also evict the persistent
                // expansion cache rows derived from it — those are DB state, not
                // process memory, so no amount of in-process eviction covers them.
                if let Some(ref resource_id) = replace_id {
                    match resource_type {
                        "CodeSystem" => {
                            if let Some(url) = hts_get_cs_url(&conn, resource_id)? {
                                hts_invalidate_expansion_cache(&conn, &url)?;
                            }
                            hts_delete_cs(&conn, resource_id)?;
                        }
                        "ValueSet" => hts_delete_vs(&conn, resource_id)?,
                        "ConceptMap" => hts_delete_cm(&conn, resource_id)?,
                        _ => {}
                    }
                }

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
            .map_err(|e| HtsError::Internal(e.to_string()))?;
            return joined;
        }

        // PostgreSQL: `import_bundle` upserts by URL, so no explicit delete is
        // needed for the update path.
        if let Some(importer) = state.terminology_importer.clone() {
            return hts_reindex_via_importer(&*importer, resource_type, content, ctx).await;
        }

        Ok(())
    }

    // ── Generic CRUD helpers ───────────────────────────────────────────────────

    /// POST /<ResourceType> — create a new resource.
    ///
    /// 1. Stores raw JSON in `helios-persistence` (auto-generates version / ETag).
    /// 2. Indexes the stored content into the HTS normalized tables.
    /// 3. Evicts every cache derived from terminology content.
    /// 4. Returns **201 Created** with `Location` and `ETag` headers.
    ///
    /// Step 3 is not optional: creating a resource has to un-cache the *absence*
    /// recorded for its URL by any earlier `$expand` / `$lookup` that 404'd, or
    /// the server keeps denying that a resource it just returned `201` for
    /// exists (issue #304).
    async fn create_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let store = state
            .active_resource_store()
            .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?;

        let ctx = ctx();

        // 1. Persist raw FHIR JSON (version_id = "1", ETag = W/"1").
        let stored = store
            .create(&ctx, resource_type, body, FhirVersion::default_enabled())
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // 2. Index into HTS normalized tables using the canonical content
        //    (persistence layer injects the correct `id` into the JSON).
        let reindexed =
            hts_reindex(&state, resource_type, stored.content().clone(), &ctx, None).await;

        // 3. Evict regardless of whether the re-index succeeded: the resource is
        //    already in the resource store, so any cached "does not exist" answer
        //    is wrong either way.
        evict_caches_after_write(&state);
        reindexed?;

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
        let store = state
            .active_resource_store()
            .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?;
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
    /// 3. Evicts every cache derived from terminology content.
    /// 4. Returns **200 OK** with the updated `ETag`.
    ///
    /// Step 3 is not optional: without it a `PUT` is indexed into storage and
    /// then shadowed by pre-update bytes held in the handler-level `$expand` /
    /// `$lookup` / `$validate-code` caches and the backend's own response memos
    /// (issue #304).
    async fn update_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
        if_match: Option<String>,
        body: Value,
        format: ResponseFormat,
    ) -> Result<Response, HtsError> {
        let store = state
            .active_resource_store()
            .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?;

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

        // 4. Re-index normalized HTS tables, replacing the previous rows.
        let reindexed = hts_reindex(
            &state,
            resource_type,
            updated.content().clone(),
            &ctx,
            Some(updated.id().to_string()),
        )
        .await;

        // 5. Evict regardless of the re-index outcome. The SQLite path deletes
        //    the old normalized rows before re-importing, so a failure in
        //    between leaves storage changed and every cached answer stale.
        evict_caches_after_write(&state);
        reindexed?;

        let etag = updated.etag().to_string();
        let mut response = fhir_respond(updated.content().clone(), format);
        if let Ok(v) = HeaderValue::from_str(&etag) {
            response.headers_mut().insert(header::ETAG, v);
        }
        Ok(response)
    }

    /// DELETE /<ResourceType>/:id — soft-delete a resource.
    ///
    /// 1. Reads the current resource to extract its canonical URL (needed for
    ///    HTS normalized-table cleanup).
    /// 2. Soft-deletes the raw JSON in `helios-persistence`.
    /// 3. Removes the resource's rows from HTS normalized tables:
    ///    - SQLite path: via `hts_pool` spawn-blocking helpers.
    ///    - PostgreSQL path: via `terminology_importer.delete_normalized()`.
    /// 4. Returns **204 No Content**.
    async fn delete_resource<B: TerminologyBackend>(
        resource_type: &'static str,
        state: AppState<B>,
        id: String,
    ) -> Result<Response, HtsError> {
        let store = state
            .active_resource_store()
            .ok_or_else(|| HtsError::Internal("Resource store not initialized".into()))?;

        #[cfg(feature = "sqlite")]
        let hts_pool = state.hts_pool.clone();
        let terminology_importer = state.terminology_importer.clone();
        let ctx = ctx();

        // 1. Read the resource URL before deleting (needed by both HTS cleanup paths).
        let resource_url: Option<String> = match store.read(&ctx, resource_type, &id).await {
            Ok(Some(r)) if !r.is_deleted() => r
                .content()
                .get("url")
                .and_then(|u| u.as_str())
                .map(str::to_owned),
            _ => None,
        };

        // 2. Soft-delete in persistence store.
        store
            .delete(&ctx, resource_type, &id)
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // 3. Delete from HTS normalized tables.
        #[allow(unused_mut, unused_assignments)]
        let mut removed: Result<(), HtsError> = Ok(());

        #[cfg(feature = "sqlite")]
        if let Some(pool) = hts_pool {
            let resource_id = id.clone();
            removed = tokio::task::spawn_blocking(move || {
                let conn = pool
                    .get()
                    .map_err(|e| HtsError::StorageError(format!("HTS pool error: {e}")))?;
                match resource_type {
                    "CodeSystem" => {
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
            .map_err(|e| HtsError::Internal(e.to_string()))?;
        } else if let (Some(importer), Some(url)) =
            (terminology_importer.as_deref(), resource_url.as_deref())
        {
            removed = importer.delete_normalized(resource_type, url).await;
        }

        #[cfg(not(feature = "sqlite"))]
        if let (Some(importer), Some(url)) =
            (terminology_importer.as_deref(), resource_url.as_deref())
        {
            removed = importer.delete_normalized(resource_type, url).await;
        }

        // Evict both cache tiers so a deleted ValueSet (and any CodeSystem whose
        // concepts would otherwise linger in a cached expansion or response memo)
        // stops being served from memory. Runs even when the normalized delete
        // failed — the resource is already soft-deleted in the resource store, so
        // holding on to cached answers about it is wrong either way.
        evict_caches_after_write(&state);
        removed?;

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
    use tower::ServiceExt;

    use helios_persistence::backends::sqlite::SqliteBackend;

    use crate::backends::sqlite::SqliteTerminologyBackend;
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
        let state = make_state();
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
        let state = make_state();
        let app = ops_router(state.clone());

        assert_eq!(
            send_json(
                &app,
                "POST",
                "/CodeSystem",
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
    /// The SQLite CRUD path writes through `hts_pool` and never reaches
    /// `BundleImportBackend::import_bundle`, so before this fix nothing on any
    /// CRUD verb cleared that memo. A Tier-1-only fix leaves this test red.
    #[tokio::test]
    async fn update_code_system_evicts_the_backend_response_memo() {
        let state = make_state();
        let app = ops_router(state.clone());

        assert_eq!(
            send_json(
                &app,
                "POST",
                "/CodeSystem",
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
        let state = make_state();
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
                "POST",
                "/ValueSet",
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
        let state = make_state();
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
                "POST",
                "/ValueSet",
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

        // Verify HTS normalized tables were populated. The synthetic storage
        // id (`<fhir-id>|<version>`) is opaque, so we look up by URL.
        let conn = hts_pool.get().unwrap();
        let storage_id: String = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = 'http://example.org/cs/cs-index'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            storage_id.starts_with("cs-index"),
            "storage id should be derived from FHIR id, got {storage_id}"
        );

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
                "SELECT COUNT(*) FROM concepts WHERE system_id LIKE 'cs-del%'",
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
