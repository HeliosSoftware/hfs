// Consumed by the $everything handler (Task 7).
#![allow(dead_code)]

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
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
            pos = Position {
                seg: pos.seg + 1,
                inner: None,
            };
        }
    }
    Ok(Step::Done)
}

async fn resolve_supporting<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    matches: &[StoredResource],
) -> RestResult<Vec<StoredResource>>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let mut included = Vec::new();
    for (rt, id) in collect_supporting_refs(version, matches) {
        if let Some(res) = state.storage().read(tenant, &rt, &id).await? {
            included.push(res);
        }
    }
    Ok(included)
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
    let fp_input = params.fingerprint_input(Some(patient_id));
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
    let included = resolve_supporting(state, tenant, version, &matches).await?;
    Ok(WalkOutput {
        matches,
        included,
        next,
        ceiling_hit,
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
    let fp_input = params.fingerprint_input(None);
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
    let included = resolve_supporting(state, tenant, version, &matches).await?;
    Ok(WalkOutput {
        matches,
        included,
        next,
        ceiling_hit,
    })
}
