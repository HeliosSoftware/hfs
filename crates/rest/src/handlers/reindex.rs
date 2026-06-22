//! `$reindex` operation handlers.
//!
//! Asynchronous reindex kick-off, status polling, and cancellation. The actual
//! reindex work is driven by a [`ReindexController`] held in [`AppState`];
//! when none is wired, the handlers return `501 Not Implemented`.
//!
//! At kickoff and at every job lifecycle transition, the handler emits a
//! BALP-compliant `AuditEvent` (action `Execute`) via the helper in
//! [`helios_persistence::search::reindex::audit`].
//!
//! # Routes
//!
//! - `POST /$reindex` — system-level reindex (every resource type in tenant).
//! - `POST /{resource_type}/$reindex` — type-scoped reindex.
//! - `GET /$reindex-status/{job_id}` — poll job progress.
//! - `DELETE /$reindex-status/{job_id}` — cancel a running job.
//!
//! # Request Body
//!
//! Either a [`ReindexRequest`] JSON object or an empty body (uses defaults).
//! Type-scoped kickoff overrides the `resource_types` field with the path
//! component to prevent privilege escalation.
//!
//! [`ReindexController`]: crate::reindex::ReindexController

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::Response,
};
use helios_auth::Principal;
use helios_persistence::core::ResourceStorage;
use helios_persistence::search::reindex::ReindexRequest;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

fn not_implemented() -> RestError {
    RestError::NotImplemented {
        feature: "$reindex is not supported by the configured storage backend".to_string(),
    }
}

async fn parse_reindex_body(request: Request) -> Result<ReindexRequest, RestError> {
    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .map_err(|e| RestError::BadRequest {
            message: format!("failed to read request body: {e}"),
        })?;
    if bytes.is_empty() {
        return Ok(ReindexRequest::default());
    }
    serde_json::from_slice(&bytes).map_err(|e| RestError::BadRequest {
        message: format!("invalid $reindex Parameters JSON: {e}"),
    })
}

/// `POST /$reindex` — system-level reindex kick-off.
pub async fn reindex_system_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    run_reindex_kickoff(state, tenant, request, None).await
}

/// `POST /{resource_type}/$reindex` — type-scoped reindex kick-off.
pub async fn reindex_type_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    run_reindex_kickoff(state, tenant, request, Some(resource_type)).await
}

async fn run_reindex_kickoff<S>(
    state: AppState<S>,
    tenant: TenantExtractor,
    request: Request,
    scoped_type: Option<String>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = state.reindex_controller().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();

    // Authorization: an update scope on the target type (or wildcard) is
    // required when auth is enabled — reindex mutates search index entries.
    if let Some(p) = principal.as_ref() {
        let auth_target = scoped_type.as_deref().unwrap_or("*");
        if let Err(e) =
            helios_auth::SmartScopePolicy::check(p, auth_target, helios_auth::FhirOperation::Update)
        {
            let msg = e.to_string();
            let scoped_vec: Vec<String> = scoped_type.clone().into_iter().collect();
            emit_reindex_audit(
                &state,
                principal.as_ref(),
                "",
                "kickoff",
                &scoped_vec,
                0,
                "8",
                Some(&msg),
            )
            .await;
            return Err(RestError::Forbidden { message: msg });
        }
    }

    let mut req = parse_reindex_body(request).await?;
    if let Some(rt) = scoped_type.as_ref() {
        // Path takes precedence over body for type-scoped kickoff.
        req.resource_types = Some(vec![rt.clone()]);
    }
    let types_for_audit: Vec<String> = req.resource_types.clone().unwrap_or_default();

    match controller.start(tenant.context().clone(), req).await {
        Ok(job_id) => {
            emit_reindex_audit(
                &state,
                principal.as_ref(),
                &job_id,
                "kickoff",
                &types_for_audit,
                0,
                "0",
                None,
            )
            .await;
            let status_url = format!(
                "{}/$reindex-status/{}",
                state.base_url().trim_end_matches('/'),
                job_id
            );
            let body = serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "jobId", "valueString": job_id},
                ]
            });
            let bytes = serde_json::to_vec(&body).map_err(|e| RestError::InternalError {
                message: e.to_string(),
            })?;
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .header("Content-Location", status_url)
                .header("Content-Type", "application/fhir+json")
                .body(Body::from(bytes))
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        Err(e) => {
            let msg = e.to_string();
            emit_reindex_audit(
                &state,
                principal.as_ref(),
                "",
                "kickoff",
                &types_for_audit,
                0,
                "8",
                Some(&msg),
            )
            .await;
            Err(RestError::InternalError { message: msg })
        }
    }
}

/// `GET /$reindex-status/{job_id}` — return progress as a FHIR `Parameters`.
pub async fn reindex_status_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    _tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = state.reindex_controller().ok_or_else(not_implemented)?;
    let progress = controller
        .progress(&job_id)
        .await
        .ok_or_else(|| RestError::NotFound {
            resource_type: "reindex-job".to_string(),
            id: job_id.clone(),
        })?;
    let body = progress.to_parameters();
    let bytes = serde_json::to_vec(&body).map_err(|e| RestError::InternalError {
        message: e.to_string(),
    })?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/fhir+json")
        .body(Body::from(bytes))
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// `DELETE /$reindex-status/{job_id}` — cooperatively cancel a running job.
pub async fn reindex_cancel_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    _tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = state.reindex_controller().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();

    match controller.cancel(&job_id).await {
        Ok(()) => {
            emit_reindex_audit(
                &state,
                principal.as_ref(),
                &job_id,
                "cancel",
                &[],
                0,
                "0",
                None,
            )
            .await;
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .body(Body::empty())
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        Err(e) => {
            let msg = e.to_string();
            emit_reindex_audit(
                &state,
                principal.as_ref(),
                &job_id,
                "cancel",
                &[],
                0,
                "8",
                Some(&msg),
            )
            .await;
            Err(RestError::NotFound {
                resource_type: "reindex-job".to_string(),
                id: job_id,
            })
        }
    }
}

/// Emits a `$reindex` `AuditEvent` when an audit sink is configured.
#[allow(clippy::too_many_arguments)]
async fn emit_reindex_audit<S>(
    state: &AppState<S>,
    principal: Option<&Principal>,
    job_id: &str,
    phase: &str,
    resource_types: &[String],
    resources_processed: u64,
    outcome: &str,
    outcome_desc: Option<&str>,
) where
    S: ResourceStorage,
{
    let Some(sink) = state.audit_sink() else {
        return;
    };
    let mut builder = helios_audit::AuditEventBuilder::new(state.audit_source_observer())
        .event_type(
            "http://terminology.hl7.org/CodeSystem/audit-event-type",
            "object",
        )
        .action(helios_audit::AuditAction::Execute)
        .outcome(outcome)
        .detail("audit-operation", "reindex")
        .detail("phase", phase)
        .detail("resources-processed", resources_processed.to_string());
    if !job_id.is_empty() {
        builder = builder.detail("job-id", job_id);
    }
    if !resource_types.is_empty() {
        builder = builder.detail("resource-types", resource_types.join(","));
    }
    if let Some(desc) = outcome_desc {
        builder = builder.outcome_desc(desc);
    }
    if let Some(p) = principal {
        builder = builder.agent(&p.subject, None, true);
    }
    sink.record(builder.build()).await;
}
