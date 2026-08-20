//! Tenant-maintenance page (`/ui/tenants`) — server-rendered, htmx-driven.
//!
//! Lists the server's tenants with their resource counts and creation dates,
//! and provides add / delete against the live registry. It reads and writes the
//! same [`ResourceStorage`] tenant registry that backs the JSON
//! `/admin/tenants` API; this page is the human surface, that API the
//! programmatic one.
//!
//! Everything is server-rendered: the table is an htmx fragment
//! (`/ui/tenants/rows`), the add form posts to `/ui/tenants`, and each delete
//! button issues `hx-delete /ui/tenants/{id}`. Every mutation returns the
//! refreshed rows fragment, so the page works the same whether or not the
//! browser ran the swap.
//!
//! Provisioning a tenant runs in the background (#581): a successful `POST`
//! means the id was *accepted*, not that `register_tenant` and the
//! conformance seed have finished. The id is claimed under
//! [`ProvisioningRegistry`], the work is handed to a `tokio::spawn`, and the
//! response returns immediately with `HX-Trigger: tenant-created` (so the
//! page script resets the form) and a rows fragment that shows the tenant as
//! a spinner row. The fragment polls itself every few seconds while any row
//! is still in flight, so the table settles into a normal row — or a
//! dismissable failure — without the client holding the original request
//! open.

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, current_status, render};

/// In-flight and failed tenant provisioning jobs, keyed by tenant id (#581).
/// In-memory only: a server restart loses these notices; the storage registry
/// stays the source of truth for completed tenants.
///
/// A `std::sync::Mutex`, not `tokio::sync::Mutex`: every critical section
/// below is a plain map lookup/insert with no `.await` inside it.
pub(crate) type ProvisioningRegistry = Arc<std::sync::Mutex<HashMap<String, Provisioning>>>;

/// State of one provisioning job.
#[derive(Debug, Clone)]
pub(crate) enum Provisioning {
    /// `register_tenant` + conformance seeding are still running.
    InFlight { display_name: Option<String> },
    /// The background job failed; shown until the user dismisses the row.
    Failed {
        display_name: Option<String>,
        message: String,
    },
}

/// One row of the tenant table (a registry record joined with its live count),
/// or a provisioning job row standing in for a tenant that is not registered
/// yet (#581).
struct TenantRow {
    id: String,
    display_name: Option<String>,
    /// RFC 3339 date-time, or `None` for a tenant discovered from data only
    /// (or still provisioning).
    created_at: Option<String>,
    registered: bool,
    resources: u64,
    /// `register_tenant` + conformance seeding are still running in the
    /// background for this id.
    provisioning: bool,
    /// The background job for this id failed; carries the error message shown
    /// as the row's tooltip.
    failed: Option<String>,
}

impl TenantRow {
    /// The date portion of `created_at` (`YYYY-MM-DD`), or an em dash when the
    /// tenant has no registration (data-only) and thus no known creation date.
    fn created_date(&self) -> String {
        match &self.created_at {
            Some(ts) => ts.split('T').next().unwrap_or(ts).to_string(),
            None => "—".to_string(),
        }
    }

    /// Up-to-two-letter avatar initials from the display name or id.
    fn initials(&self) -> String {
        let source = self.display_name.as_deref().unwrap_or(&self.id);
        let letters: String = source
            .split([' ', '-', '_', '/'])
            .filter(|w| !w.is_empty())
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect();
        if letters.is_empty() {
            "?".to_string()
        } else {
            letters.to_uppercase()
        }
    }

    /// A compact human count: `1.28M`, `842.1K`, `13`.
    fn resources_human(&self) -> String {
        let n = self.resources;
        if n >= 1_000_000 {
            format!("{:.2}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }
}

/// Page template. `status` feeds the shared sidebar (brand version); `rows` is
/// the table body, `total`/`registered_count` the summary stats, `q` the active
/// search term, `available` whether the backend has a registry. `polling`
/// mirrors the included rows partial's field (Askama `include` shares the
/// enclosing template's context) — see [`TenantRowsPartial::polling`].
#[derive(Template)]
#[template(path = "pages/tenants.html")]
struct TenantsPage {
    status: crate::Status,
    i18n: I18n,
    rows: Vec<TenantRow>,
    total: usize,
    registered_count: usize,
    resources_total: u64,
    q: String,
    available: bool,
    error: Option<String>,
    polling: bool,
    /// Which sidebar entry carries `aria-current="page"` (see base.html).
    active_page: &'static str,
}

#[derive(Template)]
#[template(path = "partials/tenant_rows.html")]
struct TenantRowsPartial {
    i18n: I18n,
    rows: Vec<TenantRow>,
    /// Set when a mutation failed; rendered as a banner above the table. The
    /// fragment is always returned with `200` so htmx swaps it regardless, which
    /// keeps the error visible without the response-targets extension.
    error: Option<String>,
    /// At least one row is still provisioning (#581): the fragment embeds a
    /// self-polling `hx-get` so the table keeps refreshing until every job
    /// settles, then the poller stops rendering on its own.
    polling: bool,
}

/// Query string for the page and the rows fragment (`?q=` search term).
#[derive(Debug, Default, Deserialize)]
pub struct TenantsQuery {
    #[serde(default)]
    q: String,
}

/// Form body for the add-tenant slide-over (`POST /ui/tenants`).
#[derive(Debug, Deserialize)]
pub struct CreateForm {
    id: String,
    #[serde(default)]
    display_name: String,
}

/// Query string for delete (`?purge=true` also tears down data).
#[derive(Debug, Default, Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    purge: bool,
}

/// Mirrors the API's id validation so the page rejects the same inputs.
///
/// Delegates to [`TenantId::parse`](helios_persistence::tenant::TenantId::parse),
/// the single reservation authority shared with the `/admin/tenants` API, the
/// request-time tenant extractor, and the storage-layer lifecycle guard, so this
/// page cannot drift from them (issue #317).
fn validate_id(id: &str) -> Result<(), String> {
    use helios_persistence::tenant::{MAX_TENANT_ID_LEN, TenantId, TenantIdError};

    TenantId::parse(id).map(|_| ()).map_err(|e| match e {
        TenantIdError::Empty => "Tenant id must not be empty.".to_string(),
        TenantIdError::TooLong { .. } => {
            format!("Tenant id exceeds {MAX_TENANT_ID_LEN} characters.")
        }
        TenantIdError::ReservedSegment { .. } => {
            "That id is reserved for internal shared resources.".to_string()
        }
        TenantIdError::InvalidCharacter { .. } => {
            "Tenant id may contain only letters, digits, '-', '_', '.', and '/'.".to_string()
        }
        TenantIdError::EmptySegment => {
            "Tenant id must not begin or end with '/' or contain an empty segment.".to_string()
        }
        // `TenantIdError` is `#[non_exhaustive]`; a variant added later falls
        // back to its own `Display` rather than failing to compile here.
        other => other.to_string(),
    })
}

/// Builds the tenant rows from the registry joined with live per-tenant counts,
/// prefixed with any in-flight/failed provisioning jobs (#581) not yet
/// reflected in the registry, filtered by the search term (matches id or
/// display name, case-insensitive).
async fn load_rows(
    storage: &Arc<dyn ResourceStorage>,
    q: &str,
    jobs: &HashMap<String, Provisioning>,
) -> Result<Vec<TenantRow>, String> {
    let registered = storage
        .list_tenants()
        .await
        .map_err(|e| format!("Failed to list tenants: {e}"))?;
    let counts: HashMap<String, u64> = storage
        .count_by_tenant()
        .await
        .map_err(|e| format!("Failed to count resources: {e}"))?
        .into_iter()
        .collect();

    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();

    let registered_ids: std::collections::HashSet<&str> =
        registered.iter().map(|r| r.id.as_str()).collect();

    // Provisioning jobs go first, in id order. The provisioning row wins
    // while the job is in flight: an `InFlight` id always renders here, even
    // if the registry read above already shows it registered (registration
    // finishes in milliseconds, seeding takes far longer, so that window is
    // routine, not a race to paper over). A `Failed` leftover for an id that
    // *is* registered yields to the registered row below instead — `Failed`
    // is only ever written when `register_tenant` itself failed, so a
    // registered id with a `Failed` entry is a stale notice, not the tenant
    // that failed.
    let mut job_ids: Vec<&String> = jobs.keys().collect();
    job_ids.sort();
    for id in job_ids {
        seen.insert(id.clone());
        match &jobs[id] {
            Provisioning::InFlight { display_name } => rows.push(TenantRow {
                id: id.clone(),
                display_name: display_name.clone(),
                created_at: None,
                registered: false,
                resources: 0,
                provisioning: true,
                failed: None,
            }),
            Provisioning::Failed {
                display_name,
                message,
            } => {
                if !registered_ids.contains(id.as_str()) {
                    rows.push(TenantRow {
                        id: id.clone(),
                        display_name: display_name.clone(),
                        created_at: None,
                        registered: false,
                        resources: 0,
                        provisioning: false,
                        failed: Some(message.clone()),
                    });
                }
            }
        }
    }

    for rec in registered {
        // An in-flight job for this id already rendered its spinner row above;
        // the registered row would just duplicate it until the seed finishes.
        if matches!(jobs.get(&rec.id), Some(Provisioning::InFlight { .. })) {
            continue;
        }
        seen.insert(rec.id.clone());
        rows.push(TenantRow {
            resources: counts.get(&rec.id).copied().unwrap_or(0),
            registered: true,
            created_at: Some(rec.created_at),
            display_name: rec.display_name,
            id: rec.id,
            provisioning: false,
            failed: None,
        });
    }
    // Data-only tenants (have rows but never registered), excluding the system tenant.
    let mut discovered: Vec<(String, u64)> = counts
        .into_iter()
        .filter(|(id, _)| id != helios_persistence::tenant::SYSTEM_TENANT && !seen.contains(id))
        .collect();
    discovered.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, n) in discovered {
        rows.push(TenantRow {
            id,
            display_name: None,
            created_at: None,
            registered: false,
            resources: n,
            provisioning: false,
            failed: None,
        });
    }

    let needle = q.trim().to_lowercase();
    if !needle.is_empty() {
        rows.retain(|r| {
            r.id.to_lowercase().contains(&needle)
                || r.display_name
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&needle))
        });
    }
    Ok(rows)
}

/// `GET /ui/tenants` — the full page.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    Query(query): Query<TenantsQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(state.version, rv.0, &rt);

    let Some(storage) = state.tenants.as_ref() else {
        return render(TenantsPage {
            status,
            i18n,
            rows: Vec::new(),
            total: 0,
            registered_count: 0,
            resources_total: 0,
            q: query.q,
            available: false,
            error: None,
            polling: false,
            active_page: "tenants",
        });
    };

    let jobs = state.provisioning.lock().unwrap().clone();
    let (rows, error) = match load_rows(storage, &query.q, &jobs).await {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let registered_count = rows.iter().filter(|r| r.registered).count();
    let resources_total = rows.iter().map(|r| r.resources).sum();
    let polling = rows.iter().any(|r| r.provisioning);
    render(TenantsPage {
        total: rows.len(),
        registered_count,
        resources_total,
        rows,
        status,
        i18n,
        q: query.q,
        available: true,
        error,
        polling,
        active_page: "tenants",
    })
}

/// `GET /ui/tenants/rows` — the table body fragment (htmx search + refresh,
/// and the self-poll while a provisioning job is in flight, #581).
pub async fn rows(
    State(state): State<WebState>,
    locale: RequestLocale,
    Query(query): Query<TenantsQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let Some(storage) = state.tenants.as_ref() else {
        return render(TenantRowsPartial {
            i18n,
            rows: Vec::new(),
            error: None,
            polling: false,
        });
    };
    let jobs = state.provisioning.lock().unwrap().clone();
    let (rows, error) = match load_rows(storage, &query.q, &jobs).await {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    rows_response(i18n, rows, error)
}

/// Returns the refreshed (unfiltered) rows fragment, with an optional error
/// banner, always `200` so htmx swaps it. `polling` (whether the fragment
/// re-embeds its own poller, #581) is derived here so every call site gets it
/// for free.
fn rows_response(i18n: I18n, rows: Vec<TenantRow>, error: Option<String>) -> Response {
    let polling = rows.iter().any(|r| r.provisioning);
    render(TenantRowsPartial {
        i18n,
        rows,
        error,
        polling,
    })
}

/// `POST /ui/tenants` — claim the id and return immediately with the
/// refreshed rows (with an inline error banner if the id was invalid or
/// already taken/claimed). On the accepted path the response carries
/// `HX-Trigger: tenant-created`; the actual `register_tenant` call and
/// conformance seed run in the background (#581), and the returned rows
/// fragment already shows the tenant as an in-flight (spinner) row.
pub async fn create(
    State(state): State<WebState>,
    locale: RequestLocale,
    axum::extract::Form(form): axum::extract::Form<CreateForm>,
) -> Response {
    let i18n = I18n::new(locale);
    let Some(storage) = state.tenants.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant registry unavailable",
        )
            .into_response();
    };

    let id = form.id.trim().to_string();
    let load = |err: Option<String>| async {
        let jobs = state.provisioning.lock().unwrap().clone();
        match load_rows(storage, "", &jobs).await {
            Ok(rows) => rows_response(i18n, rows, err),
            Err(e) => rows_response(i18n, Vec::new(), err.or(Some(e))),
        }
    };

    if let Err(msg) = validate_id(&id) {
        return load(Some(msg)).await;
    }
    let display_name = {
        let d = form.display_name.trim();
        if d.is_empty() {
            None
        } else {
            Some(d.to_string())
        }
    };

    match storage.get_tenant(&id).await {
        Ok(Some(_)) => return load(Some(format!("Tenant '{id}' already exists."))).await,
        Ok(None) => {}
        Err(e) => return load(Some(e.to_string())).await,
    }

    // Claim the id under one lock so a double submit cannot race the check
    // above: a second POST for the same id while this job is in flight is
    // rejected here, not raced against `register_tenant`. The guard is
    // dropped (block ends) before the `await` below — a `std::sync::Mutex`
    // guard held across an await point would make this handler's future
    // `!Send`, which axum's `Handler` bound rejects outright.
    let already_in_flight = {
        let mut jobs = state.provisioning.lock().unwrap();
        if matches!(jobs.get(&id), Some(Provisioning::InFlight { .. })) {
            true
        } else {
            // A leftover `Failed` entry for this id is replaced by the retry.
            jobs.insert(
                id.clone(),
                Provisioning::InFlight {
                    display_name: display_name.clone(),
                },
            );
            false
        }
    };
    if already_in_flight {
        return load(Some(format!("Tenant '{id}' is already being provisioned."))).await;
    }

    // Provision in the background: the claimed row above reports progress,
    // the page polls while the job runs, and a client disconnect no longer
    // aborts the work halfway (axum drops the handler future, not the spawned
    // task, when the socket closes).
    let registry = state.provisioning.clone();
    let job_storage = storage.clone();
    let data_dir = state
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    let fhir_version = state.fhir_version;
    let job_id = id.clone();
    let job_name = display_name.clone();
    tokio::spawn(async move {
        match job_storage
            .register_tenant(&job_id, job_name.as_deref())
            .await
        {
            Ok(_) => {
                // Seed the new tenant with the conformance resources
                // (SearchParameters and CompartmentDefinitions), matching the
                // per-tenant startup seed. Best-effort, as before: a failed
                // seed still counts as provisioned; the next startup
                // completes it.
                helios_persistence::search::seed_tenant_conformance(
                    job_storage.as_ref(),
                    fhir_version,
                    &data_dir,
                    &job_id,
                )
                .await;
                registry.lock().unwrap().remove(&job_id);
            }
            Err(e) => {
                registry.lock().unwrap().insert(
                    job_id.clone(),
                    Provisioning::Failed {
                        display_name: job_name,
                        message: e.to_string(),
                    },
                );
            }
        }
    });

    // The rows fragment below already contains the in-flight row. Tell htmx
    // the request was accepted: every path answers 200 with the rows fragment
    // (error banners included), so this header is the only signal the client
    // has to reset the form (tenants.js). Failures deliberately omit it.
    let mut response = load(None).await;
    response
        .headers_mut()
        .insert("HX-Trigger", HeaderValue::from_static("tenant-created"));
    response
}

/// `DELETE /ui/tenants/{id}` — deregister (and optionally purge), return rows.
///
/// Deregister-only by default; data teardown requires the explicit
/// `?purge=true` (the trash button never purges, so a single click can't
/// destroy data).
pub async fn delete(
    State(state): State<WebState>,
    locale: RequestLocale,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let Some(storage) = state.tenants.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant registry unavailable",
        )
            .into_response();
    };

    let load = |err: Option<String>| async {
        let jobs = state.provisioning.lock().unwrap().clone();
        match load_rows(storage, "", &jobs).await {
            Ok(rows) => rows_response(i18n, rows, err),
            Err(e) => rows_response(i18n, Vec::new(), err.or(Some(e))),
        }
    };

    // Refuse reserved ids before touching the registry. This route is destructive
    // (`?purge=true` permanently deletes the tenant's data) and, unlike the
    // `/admin/tenants` API, it validated nothing — so `DELETE
    // /ui/tenants/__system__?purge=true` would have wiped the AuditEvent trail
    // and shared terminology (issue #317).
    //
    // This closes the reserved-id hole only. The separate, known question of
    // `/ui/*` being mounted outside the auth layer is deliberately untouched
    // here; the storage-layer guard in `helios-persistence` backs this up
    // regardless of how the route is reached.
    if let Err(message) = validate_id(&id) {
        return load(Some(message)).await;
    }

    // A provisioning job owns this id (#581): an in-flight job is refused
    // rather than raced against `register_tenant` finishing concurrently
    // (nothing to deregister yet either way); a failed job never wrote
    // anything to storage, so dismissing it just clears the registry entry.
    // The lock guard is scoped to this block, dropped before either `await`
    // below — held across an await it would make the handler's future
    // `!Send`, which axum's `Handler` bound rejects outright.
    enum JobOutcome {
        InFlight,
        Dismissed,
        None,
    }
    let outcome = {
        let mut jobs = state.provisioning.lock().unwrap();
        match jobs.get(&id) {
            Some(Provisioning::InFlight { .. }) => JobOutcome::InFlight,
            Some(Provisioning::Failed { .. }) => {
                jobs.remove(&id);
                JobOutcome::Dismissed
            }
            None => JobOutcome::None,
        }
    };
    match outcome {
        JobOutcome::InFlight => {
            return load(Some(format!("Tenant '{id}' is still being provisioned."))).await;
        }
        JobOutcome::Dismissed => return load(None).await,
        JobOutcome::None => {}
    }

    let _ = storage.deregister_tenant(&id).await;
    // A failed purge must be surfaced, not swallowed. The tenant has already
    // been deregistered by the line above, so discarding this error leaves the
    // data on disk with nothing in the registry pointing at it, while the page
    // renders an ordinary success — an operator told "purged" who still holds
    // every resource. `purge_tenant_data` is transactional on the SQL backends,
    // so on failure nothing was removed and a retry is safe.
    let purge_error = if query.purge {
        storage
            .purge_tenant_data(&id)
            .await
            .err()
            .map(|e| format!("Tenant '{id}' was deregistered but its data was NOT purged: {e}"))
    } else {
        None
    };

    load(purge_error).await
}

// (helpers below)

#[cfg(test)]
mod provisioning_row_tests {
    use super::*;
    use helios_persistence::backends::sqlite::SqliteBackend;

    /// Regression for the double-row bug: while a job is still in flight the
    /// tenant is already registered (seeding runs long after registration), and
    /// the table must show the provisioning row only.
    #[tokio::test]
    async fn an_in_flight_job_suppresses_the_registered_row() {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        let storage: Arc<dyn ResourceStorage> = Arc::new(backend);
        storage
            .register_tenant("acme", Some("Acme Health"))
            .await
            .expect("register");

        let mut jobs = HashMap::new();
        jobs.insert(
            "acme".to_string(),
            Provisioning::InFlight {
                display_name: Some("Acme Health".to_string()),
            },
        );

        let rows = load_rows(&storage, "", &jobs).await.expect("rows");
        let acme: Vec<_> = rows.iter().filter(|r| r.id == "acme").collect();
        assert_eq!(acme.len(), 1, "one row per tenant, not one per source");
        assert!(acme[0].provisioning, "the provisioning row wins");

        // A Failed leftover for a registered id yields to the registered row.
        jobs.insert(
            "acme".to_string(),
            Provisioning::Failed {
                display_name: None,
                message: "boom".to_string(),
            },
        );
        let rows = load_rows(&storage, "", &jobs).await.expect("rows");
        let acme: Vec<_> = rows.iter().filter(|r| r.id == "acme").collect();
        assert_eq!(acme.len(), 1);
        assert!(!acme[0].provisioning);
        assert!(
            acme[0].failed.is_none(),
            "registered row, not the failed notice"
        );
    }
}
