use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchQuery, StoredResource};

use super::cursor::EverythingCursor;
use super::params::EverythingParams;
use super::scope::{build_segment_query, build_segments, collect_supporting_refs};
use crate::error::{RestError, RestResult};
use crate::state::AppState;

pub(crate) struct WalkOutput {
    pub matches: Vec<StoredResource>,
    pub included: Vec<StoredResource>,
    pub next: Option<EverythingCursor>,
    pub ceiling_hit: bool,
    /// `true` when the number of distinct supporting-resource references
    /// found on this page's matches exceeded the resolution cap, so some
    /// were not resolved into `included`.
    pub includes_truncated: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct WalkLimits {
    pub page: Option<usize>,
    pub unpaged_ceiling: usize,
    pub per_query: usize,
}

impl WalkLimits {
    fn target(&self) -> usize {
        self.page.unwrap_or(self.unpaged_ceiling).max(1)
    }
}

/// Where the walk stands inside one patient.
struct Position {
    seg: usize,
    inner: Option<String>,
}

enum Step {
    /// Page (or ceiling) reached; resume here.
    Paused(Position),
    /// Every segment of this patient is exhausted.
    Done,
}

/// Walks one patient's segments from `pos`, appending to `matches` until
/// `matches.len() >= target` or the segments run out.
#[allow(clippy::too_many_arguments)]
async fn walk_segments<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    patient_id: &str,
    params: &EverythingParams,
    segments: &[String],
    mut pos: Position,
    target: usize,
    per_query: usize,
    matches: &mut Vec<StoredResource>,
) -> RestResult<Step>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    if pos.seg == 0 {
        let patient = state
            .storage()
            .read(tenant, "Patient", patient_id)
            .await?
            .ok_or_else(|| RestError::NotFound {
                resource_type: "Patient".to_string(),
                id: patient_id.to_string(),
            })?;
        matches.push(patient);
        pos = Position {
            seg: 1,
            inner: None,
        };
    }

    while pos.seg < segments.len() {
        if matches.len() >= target {
            return Ok(Step::Paused(pos));
        }
        let remaining = (target - matches.len()).min(per_query).max(1) as u32;
        let query: SearchQuery = {
            let reg = state.storage().search_param_registry(tenant);
            let registry = reg.read();
            build_segment_query(
                &registry,
                version,
                &segments[pos.seg],
                patient_id,
                params,
                remaining,
                pos.inner.take(),
            )
        };
        let result = state
            .storage()
            .search(tenant, &query)
            .await
            .map_err(RestError::from)?;
        let (items, page_info) = (result.resources.items, result.resources.page_info);
        let has_next = page_info.has_next;
        let next_cursor = page_info.next_cursor;
        matches.extend(items);
        if has_next && next_cursor.is_some() {
            pos.inner = next_cursor;
        } else {
            if has_next {
                tracing::warn!(
                    resource_type = %segments[pos.seg],
                    "backend reported has_next without a cursor; $everything segment truncated"
                );
            }
            pos = Position {
                seg: pos.seg + 1,
                inner: None,
            };
        }
    }
    Ok(Step::Done)
}

/// Resolves the supporting resources referenced by `matches`, stopping after
/// `limit` distinct references. Returns the resolved resources and whether
/// truncation occurred (i.e. there were more than `limit` distinct
/// references).
async fn resolve_supporting<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    matches: &[StoredResource],
    limit: usize,
) -> RestResult<(Vec<StoredResource>, bool)>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let refs = collect_supporting_refs(version, matches);
    let truncated = refs.len() > limit;
    let mut included = Vec::new();
    for (rt, id) in refs.into_iter().take(limit) {
        match state.storage().read(tenant, &rt, &id).await {
            Ok(Some(res)) => included.push(res),
            Ok(None) => {}
            // A supporting resource that has been soft-deleted since the
            // matching resource referenced it is simply omitted, mirroring
            // how a missing reference target is handled — not surfaced as a
            // 410 for the whole $everything response.
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok((included, truncated))
}

pub(crate) async fn walk_patient<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    patient_id: &str,
    params: &EverythingParams,
    resume: Option<EverythingCursor>,
    limits: WalkLimits,
) -> RestResult<WalkOutput>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let segments = build_segments(version, params.types.as_deref());
    let fp_input = params.fingerprint_input(Some(patient_id), tenant.tenant_id().as_str(), version);
    let pos = match resume {
        Some(c) => Position {
            seg: c.seg,
            inner: c.inner,
        },
        None => Position {
            seg: 0,
            inner: None,
        },
    };
    let mut matches = Vec::new();
    let step = walk_segments(
        state,
        tenant,
        version,
        patient_id,
        params,
        &segments,
        pos,
        limits.target(),
        limits.per_query,
        &mut matches,
    )
    .await?;
    let next = match step {
        Step::Paused(p) => Some(EverythingCursor::new(p.seg, p.inner, &fp_input)),
        Step::Done => None,
    };
    let ceiling_hit = limits.page.is_none() && next.is_some();
    let (included, includes_truncated) =
        resolve_supporting(state, tenant, version, &matches, limits.unpaged_ceiling).await?;
    Ok(WalkOutput {
        matches,
        included,
        next,
        ceiling_hit,
        includes_truncated,
    })
}

pub(crate) async fn walk_all_patients<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    params: &EverythingParams,
    resume: Option<EverythingCursor>,
    limits: WalkLimits,
) -> RestResult<WalkOutput>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let segments = build_segments(version, params.types.as_deref());
    let fp_input = params.fingerprint_input(None, tenant.tenant_id().as_str(), version);
    let target = limits.target();

    // `pat` is the backend cursor that yields the NEXT patient; `pid`/`pos`
    // describe the patient currently being walked, if any.
    let (mut pat, mut pid, mut pos) = match resume {
        Some(c) => (
            c.pat,
            c.pid,
            Position {
                seg: c.seg,
                inner: c.inner,
            },
        ),
        None => (
            None,
            None,
            Position {
                seg: 0,
                inner: None,
            },
        ),
    };
    let mut matches = Vec::new();
    let mut exhausted = false;

    loop {
        if pid.is_none() {
            let mut q = SearchQuery::new("Patient");
            q.count = Some(1);
            q.cursor = pat.take();
            let page = state
                .storage()
                .search(tenant, &q)
                .await
                .map_err(RestError::from)?;
            let (items, page_info) = (page.resources.items, page.resources.page_info);
            let Some(next_patient) = items.into_iter().next() else {
                exhausted = true;
                break;
            };
            if page_info.has_next && page_info.next_cursor.is_none() {
                tracing::warn!(
                    resource_type = "Patient",
                    "backend reported has_next without a cursor; $everything segment truncated"
                );
            }
            pat = page_info.next_cursor.filter(|_| page_info.has_next);
            pid = Some(next_patient.id().to_string());
            pos = Position {
                seg: 0,
                inner: None,
            };
        }
        let current = pid.clone().expect("set above");
        match walk_segments(
            state,
            tenant,
            version,
            &current,
            params,
            &segments,
            pos,
            target,
            limits.per_query,
            &mut matches,
        )
        .await?
        {
            Step::Paused(p) => {
                pos = p;
                break;
            }
            Step::Done => {
                pid = None;
                pos = Position {
                    seg: 0,
                    inner: None,
                };
                if pat.is_none() {
                    exhausted = true;
                    break;
                }
                if matches.len() >= target {
                    break;
                }
            }
        }
    }

    let next = if exhausted {
        None
    } else {
        let mut c = EverythingCursor::new(pos.seg, pos.inner, &fp_input);
        c.pat = pat;
        c.pid = pid;
        Some(c)
    };
    let ceiling_hit = limits.page.is_none() && next.is_some();
    let (included, includes_truncated) =
        resolve_supporting(state, tenant, version, &matches, limits.unpaged_ceiling).await?;
    Ok(WalkOutput {
        matches,
        included,
        next,
        ceiling_hit,
        includes_truncated,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use super::*;
    use crate::config::ServerConfig;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    async fn state_with_backend() -> AppState<SqliteBackend> {
        let backend = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir()),
                ..Default::default()
            },
        )
        .expect("create sqlite backend");
        backend.init_schema().expect("init schema");
        AppState::new(Arc::new(backend), ServerConfig::for_testing())
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("test"), TenantPermissions::full_access())
    }

    /// `resolve_supporting` stops after `limit` distinct supporting refs and
    /// reports truncation whenever there were more than `limit` distinct
    /// refs to resolve — regardless of whether the excess ones would have
    /// resolved successfully. This is asserted directly (rather than via a
    /// full router walk) because the include cap and the match-page ceiling
    /// share one knob (`unpaged_ceiling`): a router-level page small enough
    /// to trigger the include cap also caps matches-per-page so low that
    /// only a single match (with a single supporting ref) reaches
    /// `resolve_supporting`, so the truncation path is never exercised at
    /// the router layer with the shared seed data.
    #[tokio::test]
    async fn resolve_supporting_caps_distinct_refs_and_reports_truncation() {
        let state = state_with_backend().await;
        let t = tenant();

        for id in ["dr1", "dr2", "dr3"] {
            state
                .storage()
                .create(
                    &t,
                    "Practitioner",
                    serde_json::json!({"resourceType": "Practitioner", "id": id}),
                    FhirVersion::R4,
                )
                .await
                .expect("create practitioner");
        }
        let patient = StoredResource::new(
            "Patient",
            "p1",
            TenantId::new("test"),
            serde_json::json!({
                "resourceType": "Patient",
                "id": "p1",
                "generalPractitioner": [
                    {"reference": "Practitioner/dr1"},
                    {"reference": "Practitioner/dr2"},
                    {"reference": "Practitioner/dr3"}
                ]
            }),
            FhirVersion::R4,
        );

        // Cap below the number of distinct refs: resolution stops early and
        // truncation is reported.
        let (included, truncated) = resolve_supporting(
            &state,
            &t,
            FhirVersion::R4,
            std::slice::from_ref(&patient),
            2,
        )
        .await
        .unwrap();
        assert_eq!(included.len(), 2, "{included:?}");
        assert!(truncated);

        // Cap at (or above) the number of distinct refs: nothing is
        // truncated.
        let (included, truncated) = resolve_supporting(
            &state,
            &t,
            FhirVersion::R4,
            std::slice::from_ref(&patient),
            3,
        )
        .await
        .unwrap();
        assert_eq!(included.len(), 3, "{included:?}");
        assert!(!truncated);
    }
}
