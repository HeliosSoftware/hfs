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

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, current_status, render};

/// One row of the tenant table (a registry record joined with its live count).
struct TenantRow {
    id: String,
    display_name: Option<String>,
    /// RFC 3339 date-time, or `None` for a tenant discovered from data only.
    created_at: Option<String>,
    registered: bool,
    resources: u64,
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
/// search term, `available` whether the backend has a registry.
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
        TenantIdError::Reserved { .. } => {
            "That id is reserved for internal shared resources.".to_string()
        }
        TenantIdError::InvalidChar { .. } => {
            "Tenant id may contain only letters, digits, '-', '_', '.', and '/'.".to_string()
        }
        // `TenantIdError` is `#[non_exhaustive]`; a variant added later falls
        // back to its own `Display` rather than failing to compile here.
        other => other.to_string(),
    })
}

/// Builds the tenant rows from the registry joined with live per-tenant counts,
/// filtered by the search term (matches id or display name, case-insensitive).
async fn load_rows(storage: &Arc<dyn ResourceStorage>, q: &str) -> Result<Vec<TenantRow>, String> {
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
    for rec in registered {
        seen.insert(rec.id.clone());
        rows.push(TenantRow {
            resources: counts.get(&rec.id).copied().unwrap_or(0),
            registered: true,
            created_at: Some(rec.created_at),
            display_name: rec.display_name,
            id: rec.id,
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
            active_page: "tenants",
        });
    };

    let (rows, error) = match load_rows(storage, &query.q).await {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let registered_count = rows.iter().filter(|r| r.registered).count();
    let resources_total = rows.iter().map(|r| r.resources).sum();
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
        active_page: "tenants",
    })
}

/// `GET /ui/tenants/rows` — the table body fragment (htmx search + refresh).
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
        });
    };
    let (rows, error) = match load_rows(storage, &query.q).await {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    render(TenantRowsPartial { i18n, rows, error })
}

/// Returns the refreshed (unfiltered) rows fragment, with an optional error
/// banner, always `200` so htmx swaps it.
fn rows_response(i18n: I18n, rows: Vec<TenantRow>, error: Option<String>) -> Response {
    render(TenantRowsPartial { i18n, rows, error })
}

/// `POST /ui/tenants` — register a tenant, then return the refreshed rows (with
/// an inline error banner if the id was invalid or already taken).
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
        match load_rows(storage, "").await {
            Ok(rows) => rows_response(i18n, rows, err),
            Err(e) => rows_response(i18n, Vec::new(), err.or(Some(e))),
        }
    };

    if let Err(msg) = validate_id(&id) {
        return load(Some(msg)).await;
    }
    let display_name = {
        let d = form.display_name.trim();
        if d.is_empty() { None } else { Some(d) }
    };

    match storage.get_tenant(&id).await {
        Ok(Some(_)) => return load(Some(format!("Tenant '{id}' already exists."))).await,
        Ok(None) => {}
        Err(e) => return load(Some(e.to_string())).await,
    }
    if let Err(e) = storage.register_tenant(&id, display_name).await {
        return load(Some(e.to_string())).await;
    }
    // Seed the new tenant with the conformance resources (SearchParameters and
    // CompartmentDefinitions), matching the per-tenant startup seed. Best-effort:
    // a failed seed still returns the refreshed rows; the next startup completes
    // it.
    let data_dir = state
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    helios_persistence::search::seed_tenant_conformance(
        storage.as_ref(),
        state.fhir_version,
        &data_dir,
        &id,
    )
    .await;
    load(None).await
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
        let rows = load_rows(storage, "").await.unwrap_or_default();
        return rows_response(i18n, rows, Some(message));
    }

    let _ = storage.deregister_tenant(&id).await;
    if query.purge {
        let _ = storage.purge_tenant_data(&id).await;
    }

    match load_rows(storage, "").await {
        Ok(rows) => rows_response(i18n, rows, None),
        Err(e) => rows_response(i18n, Vec::new(), Some(e)),
    }
}

// (helpers below)
