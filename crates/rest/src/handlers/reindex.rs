//! `$reindex` — rebuild the search index from the stored resources.
//!
//! Needed after a `SearchParameter` is added or changed: existing resources were
//! indexed under the old definition and will not match the new one until they
//! are re-extracted.
//!
//! | Route | Effect |
//! |---|---|
//! | `POST /$reindex` | Reindex every resource type in the tenant |
//! | `POST /{type}/$reindex` | Reindex one resource type |
//! | `GET /$reindex-status/{job_id}` | Poll progress |
//! | `DELETE /$reindex-status/{job_id}` | Cancel a running job |
//!
//! Kick-off returns `202 Accepted` immediately with a job id; the rebuild runs
//! in the background.
//!
//! # Authorization
//!
//! Gated on the `system/reindex` operation scope, following `system/bulk-submit`
//! — not on ordinary `Update` scope. A reindex rewrites the entire search index
//! for a tenant, which is an administrative operation, not a resource write.
//!
//! # Job state is per-process
//!
//! Progress lives in an in-memory map on the node that accepted the kick-off, so
//! `GET /$reindex-status/{job_id}` returns 404 on any other node. In a
//! multi-node deployment, poll the node you kicked off against. (Persisting job
//! state across the cluster is tracked separately.)
//!
//! # Composite deployments
//!
//! The reindex driver writes to *every* search index — the primary's own index
//! table and the Elasticsearch secondary. Rebuilding only the primary on a
//! deployment where Elasticsearch serves search would rebuild an index nothing
//! queries and leave search stale.

use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_auth::Principal;
use helios_persistence::core::ResourceStorage;
use helios_persistence::search::{ReindexOperation, ReindexRequest};
use serde_json::json;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// The operation scope a caller must hold to reindex.
const REINDEX_SCOPE: &str = "reindex";

/// `$reindex` needs somewhere to write. The `s3` backend standalone has no
/// search index of any kind — its `SearchProvider` reports search unsupported —
/// so there is genuinely nothing to rebuild, and saying so is more honest than
/// accepting the job and reporting that it processed zero resources.
fn reindex_unavailable() -> RestError {
    RestError::NotImplemented {
        feature: "$reindex is not available: this storage backend has no search index to rebuild"
            .to_string(),
    }
}

/// Enforces the `system/reindex` operation scope. Auth disabled → allowed.
fn check_reindex_scope(principal: Option<&Principal>) -> RestResult<()> {
    if let Some(p) = principal
        && !p.scopes.grants_operation(REINDEX_SCOPE)
    {
        return Err(RestError::Forbidden {
            message: "the `system/reindex` scope is required".to_string(),
        });
    }
    Ok(())
}

/// Returns the configured driver, or 501 when the backend has no search index.
fn driver<S>(state: &AppState<S>) -> RestResult<&std::sync::Arc<ReindexOperation>>
where
    S: ResourceStorage,
{
    let op = state.reindex().ok_or_else(reindex_unavailable)?;
    if !op.has_writers() {
        return Err(reindex_unavailable());
    }
    Ok(op)
}

/// Shared kick-off for the system- and type-scoped routes.
async fn kickoff<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    principal: Option<&Principal>,
    resource_types: Option<Vec<String>>,
    body: Option<&serde_json::Value>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    check_reindex_scope(principal)?;
    let op = driver(state)?;

    let mut request = ReindexRequest::default();
    request.resource_types = resource_types;

    // Optional Parameters body: `clearExisting` drops the existing entries
    // before rebuilding, `batchSize` tunes the page size.
    if let Some(params) = body {
        for param in params
            .get("parameter")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
        {
            match param.get("name").and_then(|n| n.as_str()) {
                Some("clearExisting") => {
                    request.clear_existing = param
                        .get("valueBoolean")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                }
                Some("batchSize") => {
                    if let Some(size) = param
                        .get("valueInteger")
                        .and_then(serde_json::Value::as_u64)
                    {
                        request.batch_size = size.max(1) as u32;
                    }
                }
                _ => {}
            }
        }
    }

    // The requesting principal is threaded into the job so the terminal audit
    // event can attribute the rebuild. The job runs in the background, long
    // after this request is gone.
    let agent = principal.map(|p| p.subject.clone());

    let job_id = op
        .start(tenant.context().clone(), request, agent)
        .await
        .map_err(|e| RestError::BadRequest {
            message: format!("failed to start reindex: {e}"),
        })?;

    Ok((
        StatusCode::ACCEPTED,
        axum::Json(json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "jobId", "valueString": job_id},
                {"name": "status", "valueString": "queued"}
            ]
        })),
    )
        .into_response())
}

/// `POST /$reindex` — reindex every resource type in the tenant.
pub async fn reindex_system_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();
    let body = read_optional_json_body(request).await;
    kickoff(&state, &tenant, principal.as_ref(), None, body.as_ref()).await
}

/// `POST /{resource_type}/$reindex` — reindex a single resource type.
pub async fn reindex_type_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path(resource_type): Path<String>,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();
    let body = read_optional_json_body(request).await;
    kickoff(
        &state,
        &tenant,
        principal.as_ref(),
        Some(vec![resource_type]),
        body.as_ref(),
    )
    .await
}

/// `GET /$reindex-status/{job_id}` — poll a job's progress.
pub async fn reindex_status_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();
    check_reindex_scope(principal.as_ref())?;
    let op = driver(&state)?;

    let progress = op
        .get_progress(&job_id)
        .await
        .ok_or_else(|| RestError::NotFound {
            resource_type: "ReindexJob".to_string(),
            id: job_id.clone(),
        })?;

    Ok((StatusCode::OK, axum::Json(progress.to_parameters())).into_response())
}

/// `DELETE /$reindex-status/{job_id}` — cancel a running job.
pub async fn reindex_cancel_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let principal = request.extensions().get::<Principal>().cloned();
    check_reindex_scope(principal.as_ref())?;
    let op = driver(&state)?;

    op.cancel(&job_id).await.map_err(|_| RestError::NotFound {
        resource_type: "ReindexJob".to_string(),
        id: job_id.clone(),
    })?;

    Ok(StatusCode::ACCEPTED.into_response())
}

/// Reads a JSON body if one was sent. A reindex kick-off with no body is valid
/// (reindex everything with the defaults), so a missing or unparsable body is
/// not an error.
async fn read_optional_json_body(request: Request) -> Option<serde_json::Value> {
    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 64)
        .await
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use helios_auth::scope::ScopeSet;

    fn principal(scopes: &str) -> Principal {
        Principal {
            subject: "client-1".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: Some("t1".to_string()),
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    /// Ordinary update scope must not authorize a full index rebuild.
    #[test]
    fn test_update_scope_does_not_grant_reindex() {
        let p = principal("system/Patient.u user/Patient.cruds");
        assert!(check_reindex_scope(Some(&p)).is_err());
    }

    #[test]
    fn test_explicit_reindex_scope_grants_reindex() {
        let p = principal("system/reindex");
        assert!(check_reindex_scope(Some(&p)).is_ok());
    }

    #[test]
    fn test_system_wildcard_grants_reindex() {
        let p = principal("system/*.crud");
        assert!(check_reindex_scope(Some(&p)).is_ok());
    }

    #[test]
    fn test_no_principal_allows_reindex() {
        assert!(check_reindex_scope(None).is_ok());
    }
}
