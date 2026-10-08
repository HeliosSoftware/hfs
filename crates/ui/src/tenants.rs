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
//! button sends `DELETE /ui/tenants/{id}` (through `tenants.js`, from an
//! element outside the rows, #1851). Every mutation returns the
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
//!
//! ## Registry first, counts deferred (#1851)
//!
//! Every response here (the page, the rows fragment and its search, the
//! provisioning poll, and the create/delete reloads) renders its rows after
//! reading only the tenant registry. Which tenants hold data and how many
//! resources each holds is a cross-tenant scan, so it comes from the mounted
//! app's shared
//! [`TenantInventory`](crate::tenant_inventory::TenantInventory) (#1850),
//! read without awaiting: a request takes its last snapshot and, when that
//! is missing or out of date, the inventory starts one background refresh
//! that every request shares. [`TenantTable::build`] is the pure merge of
//! the two.
//!
//! While a refresh runs, the rows fragment carries a counts poller (2, 5, 15
//! then every 30 seconds, #1848 D16) that stops once the counts settle, and
//! every fragment updates the stat cards and the count status line out of
//! band, so the cards (global, independent of the search term, D15) and the
//! rows never disagree. Without JavaScript the status line offers a plain
//! Refresh link instead (D18). Each cell says what is known: an exact count,
//! a measured zero, presence only, or an em dash whose reason is spoken
//! (pending, unknown, unsupported, unavailable), with stale and approximate
//! figures marked as such (D17).

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
};
use chrono::Utc;
use helios_persistence::core::{
    CountBasis, DiscoveryCoverage, ErasedScope, ResourceStorage, TenantDataEvidence, TenantRecord,
    WriteEvent,
};
use helios_persistence::tenant::{SYSTEM_TENANT, TenantId};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::i18n::{I18n, RequestLocale};
use crate::tenant_inventory::{InventoryPhase, InventoryView, ResourceCell, ResourceTotal};
use crate::{AsOf, RequestTenant, RequestVersion, WebState, current_status, render};

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

/// One row of the tenant table: a registry record, or a data-only tenant the
/// inventory discovered, with what the inventory knows about its resources
/// (#1851); or a provisioning job row standing in for a tenant that is not
/// registered yet (#581).
struct TenantRow {
    id: String,
    display_name: Option<String>,
    /// RFC 3339 date-time, or `None` for a tenant discovered from data only
    /// (or still provisioning).
    created_at: Option<String>,
    registered: bool,
    /// What the shared tenant inventory knows about this tenant's resources,
    /// read from its last snapshot without awaiting storage (#1850). Not
    /// shown on provisioning and failed rows.
    count: ResourceCell,
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

    /// The resources cell (#1851): never a number the inventory did not
    /// measure, and never the same markup for two different states.
    fn count_view(&self, i18n: &I18n) -> CountView {
        match self.count {
            ResourceCell::Number {
                value,
                basis: CountBasis::LiveResources,
                stale,
            } => CountView::figure(compact(value, i18n), "number", stale, None, i18n),
            // Storage pointers (tombstones included), a search index, or a
            // basis added later: a figure, but not a live-resource total.
            ResourceCell::Number { value, stale, .. } => CountView::figure(
                format!("≈{}", compact(value, i18n)),
                "approximate",
                stale,
                Some(i18n.t("tenants-count-approx")),
                i18n,
            ),
            ResourceCell::MeasuredZero { stale } => {
                CountView::figure("0".to_string(), "zero", stale, None, i18n)
            }
            ResourceCell::HasData { .. } => CountView::placeholder(
                i18n.t("tenants-count-present"),
                "present",
                i18n.t("tenants-count-present-note"),
                CountView::MUTED,
            ),
            ResourceCell::Pending => CountView::placeholder(
                "…".to_string(),
                "pending",
                i18n.t("tenants-count-pending"),
                CountView::MUTED,
            ),
            ResourceCell::Unknown => CountView::placeholder(
                "—".to_string(),
                "unknown",
                i18n.t("tenants-count-unknown"),
                CountView::MUTED,
            ),
            ResourceCell::Unsupported => CountView::placeholder(
                "—".to_string(),
                "unsupported",
                i18n.t("tenants-count-unsupported"),
                CountView::MUTED,
            ),
            ResourceCell::Unavailable => CountView::placeholder(
                "—".to_string(),
                "unavailable",
                i18n.t("tenants-count-unavailable"),
                "count-note count-note--error",
            ),
        }
    }

    /// A settled row: neither an in-flight nor a failed provisioning job.
    fn settled(&self) -> bool {
        !self.provisioning && self.failed.is_none()
    }
}

/// A compact human count in the page's locale: `1.28M`, `842.1K`, `13`
/// (`1,28M` in German and Spanish).
fn compact(n: u64, i18n: &I18n) -> String {
    if n >= 1_000_000 {
        format!("{}M", i18n.dec(n as f64 / 1_000_000.0, 2))
    } else if n >= 1_000 {
        format!("{}K", i18n.dec(n as f64 / 1_000.0, 1))
    } else {
        n.to_string()
    }
}

/// How one resources cell renders (#1851, #1848 D17).
struct CountView {
    /// The visible text: a figure, `…` while counting, `—` when unknown.
    text: String,
    /// `data-count-state`: `number`, `approximate`, `zero`, `present`,
    /// `pending`, `unknown`, `unsupported` or `unavailable`.
    state: &'static str,
    /// The figure comes from a stale snapshot (`data-count-stale`).
    stale: bool,
    /// Why the cell reads as it does (its `title`); `None` for an exact,
    /// fresh figure.
    note: Option<String>,
    /// What a screen reader says in place of `text`.
    spoken: String,
    /// The note's classes.
    class: &'static str,
}

impl CountView {
    const PLAIN: &'static str = "count-note";
    const MUTED: &'static str = "count-note count-note--muted";

    fn figure(
        text: String,
        state: &'static str,
        stale: bool,
        qualifier: Option<String>,
        i18n: &I18n,
    ) -> Self {
        let notes: Vec<String> = qualifier
            .into_iter()
            .chain(stale.then(|| i18n.t("tenants-count-stale")))
            .collect();
        let note = (!notes.is_empty()).then(|| notes.join("; "));
        let spoken = match &note {
            Some(note) => format!("{text}, {note}"),
            None => text.clone(),
        };
        Self {
            text,
            state,
            stale,
            note,
            spoken,
            class: if stale { Self::MUTED } else { Self::PLAIN },
        }
    }

    fn placeholder(text: String, state: &'static str, note: String, class: &'static str) -> Self {
        Self {
            text,
            state,
            stale: false,
            spoken: note.clone(),
            note: Some(note),
            class,
        }
    }
}

/// The two stat cards, global and independent of the search term (#1848
/// D15). Figures are formatted; `None` renders an em dash, and the sub-label
/// always says why.
struct TenantStats {
    /// Settled tenants: registered plus data-only. `None` when the registry
    /// could not be read.
    total: Option<String>,
    total_sub: String,
    /// The resource total, only when one complete discovery counted every
    /// tenant's live resources (#1848 contract 6, D8).
    resources: Option<String>,
    resources_sub: String,
}

/// The count status line above the table (`#tenant-counts-status`): an
/// accessible `role="status"` region, distinct from the provisioning
/// `busy-status` rows.
struct CountsStatus {
    /// `data-counts-state`: `pending`, `refreshing`, `ready`, `partial`,
    /// `stale`, `unavailable` or `unsupported`. Only the first two poll.
    state: &'static str,
    /// What the counts are, as of when, and how they were measured.
    message: String,
    /// A plain Refresh link while counts are on their way, for a browser
    /// without JavaScript (#1848 D18).
    refresh_href: Option<String>,
}

/// The counts poller a fragment embeds while counts are on their way.
struct CountsPoll {
    /// Seconds until the next poll.
    delay_secs: u64,
    /// The `poll` value the next poll sends, selecting its successor's delay.
    next: u32,
}

/// The counts poller's backoff (#1848 D16): one quick look, then slower,
/// capped at thirty seconds. A user action (search, mutation, reload) starts
/// it over.
const COUNTS_POLL_BACKOFF_SECS: [u64; 4] = [2, 5, 15, 30];

/// Everything one render of the table needs: the rows (filtered by the
/// search term), the global cards and the count status (#1851).
struct TenantTable {
    rows: Vec<TenantRow>,
    /// Listing the registry failed; rendered as the rows banner.
    error: Option<String>,
    stats: TenantStats,
    counts: CountsStatus,
    /// At least one row is still provisioning (#581).
    polling: bool,
    /// Counts are on their way (and nothing is provisioning, whose own
    /// poller already refreshes the fragment).
    counts_poll: Option<CountsPoll>,
}

/// Page template. `status` feeds the shared sidebar (brand version); the
/// table fields mirror [`TenantRowsPartial`]'s, because Askama `include`
/// shares the enclosing template's context; `oob` is always `false` here.
#[derive(Template)]
#[template(path = "pages/tenants.html")]
struct TenantsPage {
    status: crate::Status,
    i18n: I18n,
    rows: Vec<TenantRow>,
    stats: TenantStats,
    counts: CountsStatus,
    q: String,
    available: bool,
    error: Option<String>,
    polling: bool,
    counts_poll: Option<CountsPoll>,
    oob: bool,
    /// Which sidebar entry carries `aria-current="page"` (see base.html).
    active_page: &'static str,
}

/// The rows fragment. It also carries the stat cards and the count status
/// out of band (`oob`), so every swap of the rows updates them together.
#[derive(Template)]
#[template(path = "partials/tenant_rows.html")]
struct TenantRowsPartial {
    i18n: I18n,
    rows: Vec<TenantRow>,
    /// Set when listing/mutating the rows themselves failed; rendered as a
    /// banner above the table. The `create` handler no longer routes its own
    /// submit errors (invalid id, duplicate, already-provisioning) through
    /// this field — those render inside the Add Tenant dialog instead (#681
    /// adenda; see [`TenantAddErrorPartial`]), since that fragment lands
    /// behind the modal scrim and was unreadable there. This field still
    /// carries `delete`'s own errors and any failure to reload the rows
    /// themselves. The fragment is always returned with `200` so htmx swaps
    /// it regardless, which keeps the error visible without the
    /// response-targets extension.
    error: Option<String>,
    /// At least one row is still provisioning (#581): the fragment embeds a
    /// self-polling `hx-get` so the table keeps refreshing until every job
    /// settles, then the poller stops rendering on its own.
    polling: bool,
    /// Counts are on their way: the fragment embeds the counts poller.
    counts_poll: Option<CountsPoll>,
    stats: TenantStats,
    counts: CountsStatus,
    /// Always `true`: the cards and status line swap out of band.
    oob: bool,
}

impl TenantRowsPartial {
    fn new(i18n: I18n, table: TenantTable, error: Option<String>) -> Self {
        Self {
            i18n,
            rows: table.rows,
            error,
            polling: table.polling,
            counts_poll: table.counts_poll,
            stats: table.stats,
            counts: table.counts,
            oob: true,
        }
    }
}

/// Out-of-band fragment for the Add Tenant dialog's own submit-error slot
/// (`#tenant-add-error`, `tenants.html`) — see the template comment on
/// `partials/tenant_add_error.html` for the mechanism and why it is OOB
/// rather than folded into [`TenantRowsPartial`].
#[derive(Template)]
#[template(path = "partials/tenant_add_error.html")]
struct TenantAddErrorPartial {
    message: Option<String>,
}

/// Query string for the page and the rows fragment: the `?q=` search term,
/// and `poll`, the counts poller's step (#1851; absent on anything a user
/// did, which restarts the backoff).
#[derive(Debug, Default, Deserialize)]
pub struct TenantsQuery {
    #[serde(default)]
    q: String,
    /// Kept as text: a malformed value restarts the backoff rather than
    /// failing the request.
    #[serde(default)]
    poll: Option<String>,
}

impl TenantsQuery {
    /// The counts poller's step; 0 when absent or malformed.
    fn poll(&self) -> u32 {
        self.poll
            .as_deref()
            .and_then(|poll| poll.parse().ok())
            .unwrap_or(0)
    }
}

/// Form body for the add-tenant slide-over (`POST /ui/tenants`). `q` is the
/// active search term (`hx-include`), so the reloaded rows keep it.
#[derive(Debug, Deserialize)]
pub struct CreateForm {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    q: String,
}

/// Query string for delete (`?purge=true` also tears down data; `q` is the
/// active search term, kept by the reloaded rows).
#[derive(Debug, Default, Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    purge: bool,
    #[serde(default)]
    q: String,
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

/// Reads the tenant registry: the only read a Tenants response awaits to
/// render its rows (#1851). Resource counts and data-only tenants come from
/// the inventory instead.
async fn load_registry(storage: &Arc<dyn ResourceStorage>) -> Result<Vec<TenantRecord>, String> {
    storage
        .list_tenants()
        .await
        .map_err(|e| format!("Failed to list tenants: {e}"))
}

/// The inventory's current snapshot for this app, without awaiting storage
/// (starting its background refresh when one is due). A state without an
/// inventory (no storage wired) reads as nothing measured yet.
fn inventory_view(state: &WebState) -> InventoryView {
    match state.tenant_inventory.as_ref() {
        Some(inventory) => inventory.view(),
        None => InventoryView {
            phase: InventoryPhase::Pending,
            discovered: Arc::default(),
            coverage: None,
            completed_at: None,
            refreshing: false,
            invalidated: BTreeSet::new(),
            counted: false,
        },
    }
}

/// Merges the registry with in-flight/failed provisioning jobs (#581) and
/// the inventory's snapshot (#1851): every row the page knows, unfiltered.
/// Pure: no storage, no clock.
fn merge_rows(
    registered: &[TenantRecord],
    jobs: &HashMap<String, Provisioning>,
    view: &InventoryView,
) -> Vec<TenantRow> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();

    let registered_ids: HashSet<&str> = registered.iter().map(|r| r.id.as_str()).collect();

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
                count: ResourceCell::Unknown,
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
                        count: ResourceCell::Unknown,
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
            count: view.cell(&rec.id),
            registered: true,
            created_at: Some(rec.created_at.clone()),
            display_name: rec.display_name.clone(),
            id: rec.id.clone(),
            provisioning: false,
            failed: None,
        });
    }
    // Data-only tenants (hold data but were never registered, or were
    // deregistered with their data kept), in id order. The inventory never
    // holds the system tenant; the filter is a second guard.
    for id in view.discovered.keys() {
        if id == SYSTEM_TENANT || seen.contains(id) {
            continue;
        }
        rows.push(TenantRow {
            id: id.clone(),
            display_name: None,
            created_at: None,
            registered: false,
            count: view.cell(id),
            provisioning: false,
            failed: None,
        });
    }
    rows
}

/// Keeps the rows whose id or display name contains the search term
/// (case-insensitive).
fn filter_rows(rows: &mut Vec<TenantRow>, q: &str) {
    let needle = q.trim().to_lowercase();
    if !needle.is_empty() {
        rows.retain(|r| {
            r.id.to_lowercase().contains(&needle)
                || r.display_name
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&needle))
        });
    }
}

/// The snapshot holds figures that are not live-resource counts: presence
/// only (S3), storage pointers or a search index. No exact total exists for
/// it, on this storage (#1848 D8).
fn non_live_evidence(view: &InventoryView) -> bool {
    view.discovered.values().any(|evidence| {
        !matches!(
            evidence,
            TenantDataEvidence::Counted {
                basis: CountBasis::LiveResources,
                ..
            }
        )
    })
}

impl TenantTable {
    /// The pure merge of one registry read (or its failure), the
    /// provisioning jobs and one inventory view (#1851). `poll` is the
    /// counts poller's step (0 for anything a user did).
    fn build(
        registry: Result<Vec<TenantRecord>, String>,
        jobs: &HashMap<String, Provisioning>,
        view: &InventoryView,
        q: &str,
        poll: u32,
        i18n: &I18n,
    ) -> Self {
        let (mut rows, error) = match registry {
            Ok(registered) => (merge_rows(&registered, jobs, view), None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let stats = Self::stats(&rows, error.is_none(), view, i18n);
        let counts = Self::counts(view, q, i18n);
        filter_rows(&mut rows, q);
        let polling = rows.iter().any(|r| r.provisioning);
        let counts_poll = (counts.refresh_href.is_some() && !polling).then(|| CountsPoll {
            delay_secs: COUNTS_POLL_BACKOFF_SECS
                [(poll as usize).min(COUNTS_POLL_BACKOFF_SECS.len() - 1)],
            next: poll.saturating_add(1),
        });
        Self {
            rows,
            error,
            stats,
            counts,
            polling,
            counts_poll,
        }
    }

    /// An empty table, for a page without storage (the template does not
    /// render it).
    fn unavailable(i18n: &I18n) -> Self {
        Self {
            rows: Vec::new(),
            error: None,
            stats: TenantStats {
                total: None,
                total_sub: String::new(),
                resources: None,
                resources_sub: String::new(),
            },
            counts: CountsStatus {
                state: "unsupported",
                message: i18n.t("tenants-counts-unsupported"),
                refresh_href: None,
            },
            polling: false,
            counts_poll: None,
        }
    }

    /// The cards, from every row (not the filtered ones), without the
    /// transient provisioning and failed rows (#1848 D15).
    fn stats(
        rows: &[TenantRow],
        registry_ok: bool,
        view: &InventoryView,
        i18n: &I18n,
    ) -> TenantStats {
        let settled: Vec<&TenantRow> = rows.iter().filter(|r| r.settled()).collect();
        let registered = settled.iter().filter(|r| r.registered).count() as u64;
        let discovery_complete = view.coverage == Some(DiscoveryCoverage::Complete);
        let (total, total_sub) = if !registry_ok {
            (None, i18n.t("tenants-stat-unavailable"))
        } else if discovery_complete {
            (
                Some(i18n.num(settled.len())),
                i18n.t_arg("tenants-stat-total-sub", "count", registered),
            )
        } else {
            // Data-only tenants are not all known yet: the figure counts
            // what is, and says so.
            (
                Some(i18n.num(settled.len())),
                i18n.t_arg("tenants-stat-total-sub-partial", "count", registered),
            )
        };

        // A row the snapshot cannot speak for (a hierarchical id under
        // presence-only discovery, a purge awaiting its recount) leaves the
        // total unknown even when every discovered tenant was counted.
        let every_row_known = settled.iter().all(|r| {
            matches!(
                r.count,
                ResourceCell::Number { .. } | ResourceCell::MeasuredZero { .. }
            )
        });
        let (resources, resources_sub) = match view.resources_total() {
            ResourceTotal::Exact { value, stale } if every_row_known => (
                Some(i18n.num(value)),
                if stale {
                    i18n.t("tenants-stat-resources-sub-stale")
                } else {
                    i18n.t("tenants-stat-resources-sub")
                },
            ),
            _ => {
                let unsupported =
                    matches!(view.coverage, Some(DiscoveryCoverage::Unsupported { .. }))
                        || non_live_evidence(view);
                let reason = if unsupported {
                    "tenants-stat-unsupported"
                } else if view.refreshing || view.phase == InventoryPhase::Pending {
                    "tenants-stat-pending"
                } else {
                    "tenants-stat-unavailable"
                };
                (None, i18n.t(reason))
            }
        };
        TenantStats {
            total,
            total_sub,
            resources,
            resources_sub,
        }
    }

    /// The count status line: what state the counts are in, as of when, how
    /// they were measured, and (while they are on their way) a Refresh link
    /// that keeps the search term.
    fn counts(view: &InventoryView, q: &str, i18n: &I18n) -> CountsStatus {
        let as_of = || {
            view.completed_at
                .map(|at| AsOf::new(at, Utc::now()).label)
                .unwrap_or_else(|| "—".to_string())
        };
        let has_snapshot = view.coverage.is_some();
        let (state, mut message) =
            if matches!(view.coverage, Some(DiscoveryCoverage::Unsupported { .. })) {
                // Never polls: nothing will change by asking again.
                ("unsupported", i18n.t("tenants-counts-unsupported"))
            } else if view.refreshing && has_snapshot {
                (
                    "refreshing",
                    i18n.t_arg("tenants-counts-refreshing", "time", as_of()),
                )
            } else if view.refreshing {
                ("pending", i18n.t("tenants-counts-pending"))
            } else {
                match &view.phase {
                    InventoryPhase::Pending => ("pending", i18n.t("tenants-counts-pending")),
                    InventoryPhase::Unavailable { .. } => {
                        ("unavailable", i18n.t("tenants-counts-unavailable"))
                    }
                    InventoryPhase::Stale { .. } => {
                        ("stale", i18n.t_arg("tenants-counts-stale", "time", as_of()))
                    }
                    InventoryPhase::Fresh
                        if matches!(view.coverage, Some(DiscoveryCoverage::Complete)) =>
                    {
                        ("ready", i18n.t_arg("tenants-counts-ready", "time", as_of()))
                    }
                    InventoryPhase::Fresh => (
                        "partial",
                        i18n.t_arg("tenants-counts-partial", "time", as_of()),
                    ),
                }
            };
        // How the figures on show were measured, when not as exact live
        // counts (#1848 D17).
        let provenance: BTreeSet<&'static str> = view
            .discovered
            .values()
            .filter_map(|evidence| match evidence {
                TenantDataEvidence::Counted {
                    basis: CountBasis::LiveResources,
                    ..
                } => None,
                TenantDataEvidence::Counted {
                    basis: CountBasis::CurrentPointersInclTombstones,
                    ..
                } => Some("tenants-counts-approx-pointers"),
                TenantDataEvidence::Counted {
                    basis: CountBasis::IndexedLiveDocuments,
                    ..
                } => Some("tenants-counts-approx-index"),
                TenantDataEvidence::Present { .. } => Some("tenants-counts-presence"),
                _ => Some("tenants-count-approx"),
            })
            .collect();
        for key in provenance {
            message.push(' ');
            message.push_str(&i18n.t(key));
        }
        let refresh_href = matches!(state, "pending" | "refreshing").then(|| {
            let q = q.trim();
            if q.is_empty() {
                "/ui/tenants".to_string()
            } else {
                let query: String = form_urlencoded::Serializer::new(String::new())
                    .append_pair("q", q)
                    .finish();
                format!("/ui/tenants?{query}")
            }
        });
        CountsStatus {
            state,
            message,
            refresh_href,
        }
    }
}

/// Reads the registry and merges it with the provisioning jobs and the
/// inventory's current view: everything a Tenants response renders, without
/// awaiting anything but the registry (#1851).
async fn load_table(
    state: &WebState,
    storage: &Arc<dyn ResourceStorage>,
    q: &str,
    poll: u32,
    i18n: &I18n,
) -> TenantTable {
    let registry = load_registry(storage).await;
    let jobs = state.provisioning.lock().unwrap().clone();
    let view = inventory_view(state);
    TenantTable::build(registry, &jobs, &view, q, poll, i18n)
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
    let status = current_status(&state, rv.0, &rt);

    let (table, available) = match state.tenants.as_ref() {
        Some(storage) => (load_table(&state, storage, &query.q, 0, &i18n).await, true),
        None => (TenantTable::unavailable(&i18n), false),
    };
    render(TenantsPage {
        status,
        i18n,
        rows: table.rows,
        stats: table.stats,
        counts: table.counts,
        q: query.q,
        available,
        error: table.error,
        polling: table.polling,
        counts_poll: table.counts_poll,
        oob: false,
        active_page: "tenants",
    })
}

/// `GET /ui/tenants/rows` — the table body fragment (htmx search + refresh,
/// the self-poll while a provisioning job is in flight, #581, and the counts
/// poll while counts are on their way, #1851).
pub async fn rows(
    State(state): State<WebState>,
    locale: RequestLocale,
    Query(query): Query<TenantsQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let Some(storage) = state.tenants.as_ref() else {
        let table = TenantTable::unavailable(&i18n);
        return render(TenantRowsPartial::new(i18n, table, None));
    };
    let table = load_table(&state, storage, &query.q, query.poll(), &i18n).await;
    rows_response(i18n, table, None)
}

/// Returns the rows fragment (filtered by the request's search term, with
/// the cards and status line out of band), with an optional error banner,
/// always `200` so htmx swaps it. A failure to list the registry itself wins
/// over `error` only when there is none.
fn rows_response(i18n: I18n, mut table: TenantTable, error: Option<String>) -> Response {
    let error = error.or(table.error.take());
    render(TenantRowsPartial::new(i18n, table, error))
}

/// `create`'s own response builder (#681 adenda): the ordinary rows fragment
/// (`table.error` is a failure to reload the table itself, e.g. the registry
/// going away mid-request — unrelated to this submission) plus the Add
/// Tenant dialog's out-of-band error slot (`dialog_message` — the submission
/// failure itself: invalid id, duplicate, already provisioning). Kept
/// separate from [`rows_response`] because `delete` and the plain rows
/// fragment endpoint have no dialog open to report into and must keep
/// rendering their error as the page-level banner.
fn create_response(i18n: I18n, mut table: TenantTable, dialog_message: Option<String>) -> Response {
    let list_error = table.error.take();
    let rendered = (|| -> askama::Result<String> {
        let rows_html = TenantRowsPartial::new(i18n, table, list_error).render()?;
        let error_html = TenantAddErrorPartial {
            message: dialog_message,
        }
        .render()?;
        Ok(format!("{rows_html}{error_html}"))
    })();
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("template render error: {error}"),
        )
            .into_response(),
    }
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
    // `err` is this submission's own failure (invalid id, duplicate,
    // already provisioning) and always renders inside the Add Tenant dialog
    // (#681 adenda), never as the page-level rows banner — see
    // `create_response`. A failure to reload the rows themselves while
    // building this response is a separate, page-level concern and keeps
    // using the ordinary banner. The reload keeps the active search term
    // (#1851), so the table and the search box never disagree.
    let load = |err: Option<String>| async {
        let table = load_table(&state, storage, &form.q, 0, &i18n).await;
        create_response(i18n, table, err)
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
    let observer = state.write_observer.clone();
    let inventory = state.tenant_inventory.clone();
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
                    observer.as_deref(),
                )
                .await;
                // The seed wrote resources into the new tenant (#1850).
                // Marked before the job leaves the registry, so a poll that
                // sees the settled row also sees the counts refreshing, and
                // keeps polling until they include it (#1851).
                if let Some(inventory) = inventory {
                    inventory.mark_stale();
                }
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

    // The reload keeps the active search term (#1851).
    let load = |err: Option<String>| async {
        let table = load_table(&state, storage, &query.q, 0, &i18n).await;
        rows_response(i18n, table, err)
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

    let deregistered = storage.deregister_tenant(&id).await.is_ok();
    // A failed purge must be surfaced, not swallowed. The tenant has already
    // been deregistered by the line above, so discarding this error leaves the
    // data on disk with nothing in the registry pointing at it, while the page
    // renders an ordinary success — an operator told "purged" who still holds
    // every resource. `purge_tenant_data` is transactional on the SQL backends,
    // so on failure nothing was removed and a retry is safe.
    let purge_error = if query.purge {
        match storage.purge_tenant_data(&id).await {
            Ok(_) => {
                // The tenant's data is gone; whoever holds figures for it (the
                // dashboard's live counters) must drop them, or the Home chart
                // would keep showing it (#1078).
                if let Some(observer) = state.write_observer.as_deref() {
                    observer.on_write(&WriteEvent::Erased {
                        tenant: TenantId::new(id.as_str()),
                        scope: ErasedScope::Tenant,
                    });
                }
                // The Tenants inventory must not resurrect the purged data
                // from a scan that began before the purge (#1850).
                if let Some(inventory) = state.unobserved_tenant_inventory() {
                    inventory.invalidate_purged(&id);
                }
                None
            }
            Err(e) => Some(format!(
                "Tenant '{id}' was deregistered but its data was NOT purged: {e}"
            )),
        }
    } else {
        None
    };
    if deregistered {
        // Whether or not its data was purged, the tenant is gone from the
        // registry; whoever holds state for it (the dashboard's live figures)
        // can drop that state (#1078). Reported after the purge's own event.
        if let Some(observer) = state.write_observer.as_deref() {
            observer.on_write(&WriteEvent::TenantRemoved {
                tenant: TenantId::new(id.as_str()),
            });
        }
        // Registry metadata changed; a data-only tenant stays in the
        // inventory until a refresh finds its data gone (#1850).
        if let Some(inventory) = state.unobserved_tenant_inventory() {
            inventory.mark_stale();
        }
    }

    load(purge_error).await
}

// (helpers below)

/// The pure merge and the rendering of every count state (#1851), over
/// hand-built inventory views: no storage, no runtime.
#[cfg(test)]
mod table_tests {
    use super::*;
    use helios_persistence::core::{DiscoveryCursor, PresenceBasis};
    use std::collections::BTreeMap;

    fn record(id: &str) -> TenantRecord {
        TenantRecord {
            id: id.to_string(),
            display_name: None,
            created_at: "2026-10-08T12:00:00Z".to_string(),
        }
    }

    fn counted(resources: u64, basis: CountBasis) -> TenantDataEvidence {
        TenantDataEvidence::Counted { resources, basis }
    }

    fn live(resources: u64) -> TenantDataEvidence {
        counted(resources, CountBasis::LiveResources)
    }

    fn view(
        phase: InventoryPhase,
        coverage: Option<DiscoveryCoverage>,
        refreshing: bool,
        discovered: &[(&str, TenantDataEvidence)],
    ) -> InventoryView {
        let discovered: BTreeMap<String, TenantDataEvidence> = discovered
            .iter()
            .map(|(id, evidence)| (id.to_string(), evidence.clone()))
            .collect();
        InventoryView {
            phase,
            counted: discovered
                .values()
                .any(|e| matches!(e, TenantDataEvidence::Counted { .. })),
            discovered: Arc::new(discovered),
            completed_at: coverage.as_ref().map(|_| Utc::now()),
            coverage,
            refreshing,
            invalidated: BTreeSet::new(),
        }
    }

    fn fresh(discovered: &[(&str, TenantDataEvidence)]) -> InventoryView {
        view(
            InventoryPhase::Fresh,
            Some(DiscoveryCoverage::Complete),
            false,
            discovered,
        )
    }

    fn i18n() -> I18n {
        I18n::new(RequestLocale::default())
    }

    fn table(registered: &[&str], view: &InventoryView, q: &str, poll: u32) -> TenantTable {
        let records: Vec<TenantRecord> = registered.iter().map(|id| record(id)).collect();
        TenantTable::build(Ok(records), &HashMap::new(), view, q, poll, &i18n())
    }

    fn render_rows(table: TenantTable) -> String {
        TenantRowsPartial::new(i18n(), table, None)
            .render()
            .expect("renders")
    }

    /// The `data-count-state` of `id`'s cell in a rendered fragment.
    fn cell_state(html: &str, id: &str) -> String {
        let row = html
            .split("<tr>")
            .find(|row| row.contains(&format!(r#"tenant-id__slug">{id}<"#)))
            .unwrap_or_else(|| panic!("no row for {id}: {html}"));
        let start = row.find(r#"data-count-state=""#).expect("a count cell") + 18;
        row[start..].split('"').next().unwrap().to_string()
    }

    #[test]
    fn every_count_state_renders_distinctly() {
        let view = view(
            InventoryPhase::Stale { last_error: None },
            Some(DiscoveryCoverage::Complete),
            false,
            &[
                ("exact", live(1_412)),
                (
                    "pointers",
                    counted(7, CountBasis::CurrentPointersInclTombstones),
                ),
                ("index", counted(9, CountBasis::IndexedLiveDocuments)),
                (
                    "present",
                    TenantDataEvidence::Present {
                        basis: PresenceBasis::ResourceObjects,
                    },
                ),
            ],
        );
        let rows = vec![
            ("exact", view.cell("exact")),
            ("pointers", view.cell("pointers")),
            ("index", view.cell("index")),
            ("present", view.cell("present")),
            ("empty", view.cell("empty")),
            ("pending", ResourceCell::Pending),
            ("unknown", ResourceCell::Unknown),
            ("unsupported", ResourceCell::Unsupported),
            ("unavailable", ResourceCell::Unavailable),
            (
                "fresh",
                ResourceCell::Number {
                    value: 13,
                    basis: CountBasis::LiveResources,
                    stale: false,
                },
            ),
            ("fresh-zero", ResourceCell::MeasuredZero { stale: false }),
        ];
        let i18n = i18n();
        let views: Vec<(&str, CountView)> = rows
            .iter()
            .map(|(id, cell)| {
                let row = TenantRow {
                    id: id.to_string(),
                    display_name: None,
                    created_at: None,
                    registered: true,
                    count: *cell,
                    provisioning: false,
                    failed: None,
                };
                (*id, row.count_view(&i18n))
            })
            .collect();
        let by_id = |id: &str| &views.iter().find(|(i, _)| *i == id).unwrap().1;

        // Exact live figures: compact, no note when fresh, marked when stale.
        assert_eq!(by_id("fresh").text, "13");
        assert_eq!(by_id("fresh").state, "number");
        assert!(by_id("fresh").note.is_none());
        assert_eq!(by_id("exact").text, "1.4K");
        assert!(by_id("exact").stale);
        assert!(
            by_id("exact")
                .note
                .as_deref()
                .unwrap()
                .contains("out of date")
        );
        // A measured zero is a zero, never a dash, and never "unknown".
        assert_eq!(by_id("fresh-zero").text, "0");
        assert_eq!(by_id("fresh-zero").state, "zero");
        assert_eq!(by_id("empty").state, "zero");
        assert!(by_id("empty").stale);
        // Non-live bases are figures, but flagged approximate.
        for id in ["pointers", "index"] {
            assert_eq!(by_id(id).state, "approximate");
            assert!(by_id(id).text.starts_with('≈'), "{id}");
            assert!(by_id(id).note.as_deref().unwrap().contains("approximate"));
        }
        // Presence is never a number.
        assert_eq!(by_id("present").state, "present");
        assert!(!by_id("present").text.chars().any(|c| c.is_ascii_digit()));
        // The figure-less states differ in state and in what is spoken.
        let figureless = ["pending", "unknown", "unsupported", "unavailable"];
        for id in figureless {
            assert_eq!(by_id(id).state, id);
            assert!(by_id(id).note.is_some());
            assert_eq!(by_id(id).spoken, by_id(id).note.clone().unwrap());
        }
        let spoken: BTreeSet<&str> = figureless
            .iter()
            .map(|id| by_id(id).spoken.as_str())
            .collect();
        assert_eq!(
            spoken.len(),
            figureless.len(),
            "each says something different"
        );
        assert_eq!(by_id("pending").text, "…");
        assert_eq!(by_id("unavailable").class, "count-note count-note--error");

        // And in the markup: one data-count-state per row, never the
        // provisioning busy-status.
        let rows: Vec<TenantRow> = rows
            .iter()
            .map(|(id, cell)| TenantRow {
                id: id.to_string(),
                display_name: None,
                created_at: None,
                registered: true,
                count: *cell,
                provisioning: false,
                failed: None,
            })
            .collect();
        let mut table = table(&[], &view, "", 0);
        table.rows = rows;
        let html = render_rows(table);
        for (id, cell) in &views {
            assert_eq!(cell_state(&html, id), cell.state, "{id}");
        }
        assert!(!html.contains("busy-status"), "{html}");
        assert!(html.contains(r#"data-count-stale="true""#));
    }

    #[test]
    fn data_only_tenants_merge_after_the_registry_without_the_system_tenant() {
        let view = fresh(&[
            ("acme", live(3)),
            ("northwind", live(5)),
            (SYSTEM_TENANT, live(99)),
        ]);
        let rows = merge_rows(&[record("acme"), record("empty")], &HashMap::new(), &view);
        let ids: Vec<(&str, bool)> = rows.iter().map(|r| (r.id.as_str(), r.registered)).collect();
        assert_eq!(
            ids,
            [("acme", true), ("empty", true), ("northwind", false)],
            "registered first, then data-only; never the system tenant"
        );
        assert_eq!(rows[1].count, ResourceCell::MeasuredZero { stale: false });
    }

    #[test]
    fn the_cards_are_global_and_only_claim_what_was_measured() {
        let view = fresh(&[("acme", live(3)), ("northwind", live(5))]);
        let all = table(&["acme", "empty"], &view, "", 0);
        let filtered = table(&["acme", "empty"], &view, "north", 0);
        assert_eq!(filtered.rows.len(), 1, "the rows follow the search");
        for table in [&all, &filtered] {
            assert_eq!(table.stats.total.as_deref(), Some("3"), "the cards do not");
            assert_eq!(table.stats.total_sub, "2 registered");
            assert_eq!(table.stats.resources.as_deref(), Some("8"));
        }

        // Discovery not finished: the tenant figure says so, and no
        // resource total is claimed.
        let pending = view_pending();
        let table = table(&["acme"], &pending, "", 0);
        assert_eq!(table.stats.total.as_deref(), Some("1"));
        assert!(table.stats.total_sub.contains("discovery incomplete"));
        assert_eq!(table.stats.resources, None);
        assert_eq!(table.stats.resources_sub, "Counting…");

        // Presence only (S3, s3-es): per-tenant presence, no total (#1848 D8).
        let presence = fresh(&[(
            "acme",
            TenantDataEvidence::Present {
                basis: PresenceBasis::ResourceObjects,
            },
        )]);
        let table = TenantTable::build(
            Ok(vec![record("acme")]),
            &HashMap::new(),
            &presence,
            "",
            0,
            &i18n(),
        );
        assert_eq!(table.stats.resources, None);
        assert_eq!(table.stats.resources_sub, "Not available on this storage");
        assert!(table.counts.message.contains("not how many"));

        // A registry failure leaves the tenant figure unknown, not zero.
        let table = TenantTable::build(
            Err("Failed to list tenants: boom".to_string()),
            &HashMap::new(),
            &view,
            "",
            0,
            &i18n(),
        );
        assert_eq!(table.stats.total, None);
        assert!(table.error.is_some());
    }

    fn view_pending() -> InventoryView {
        view(InventoryPhase::Pending, None, true, &[])
    }

    #[test]
    fn the_counts_poller_backs_off_and_stops_once_settled() {
        let delay = |poll| {
            table(&["acme"], &view_pending(), "", poll)
                .counts_poll
                .map(|p| (p.delay_secs, p.next))
        };
        assert_eq!(delay(0), Some((2, 1)));
        assert_eq!(delay(1), Some((5, 2)));
        assert_eq!(delay(2), Some((15, 3)));
        assert_eq!(delay(3), Some((30, 4)));
        assert_eq!(delay(40), Some((30, 41)), "capped");

        let cases = [
            (view_pending(), "pending", true),
            (
                view(
                    InventoryPhase::Stale { last_error: None },
                    Some(DiscoveryCoverage::Complete),
                    true,
                    &[("acme", live(1))],
                ),
                "refreshing",
                true,
            ),
            (fresh(&[("acme", live(1))]), "ready", false),
            (
                view(
                    InventoryPhase::Fresh,
                    Some(DiscoveryCoverage::Partial {
                        resume: Some(DiscoveryCursor::new("m")),
                    }),
                    false,
                    &[],
                ),
                "partial",
                false,
            ),
            (
                view(
                    InventoryPhase::Stale {
                        last_error: Some("boom".to_string()),
                    },
                    Some(DiscoveryCoverage::Complete),
                    false,
                    &[("acme", live(1))],
                ),
                "stale",
                false,
            ),
            (
                view(
                    InventoryPhase::Unavailable {
                        error: "boom".to_string(),
                        retry_after: std::time::Instant::now(),
                    },
                    None,
                    false,
                    &[],
                ),
                "unavailable",
                false,
            ),
            (
                view(
                    InventoryPhase::Fresh,
                    Some(DiscoveryCoverage::Unsupported {
                        capability: "tenant-discovery",
                    }),
                    false,
                    &[],
                ),
                "unsupported",
                false,
            ),
        ];
        for (view, state, polls) in cases {
            let table = table(&["acme"], &view, "a b", 0);
            assert_eq!(table.counts.state, state);
            assert_eq!(table.counts_poll.is_some(), polls, "{state}");
            assert_eq!(
                table.counts.refresh_href.as_deref(),
                polls.then_some("/ui/tenants?q=a+b"),
                "{state}: the no-JS Refresh link keeps the search term"
            );
        }

        // A provisioning job's own poller already refreshes the fragment.
        let mut jobs = HashMap::new();
        jobs.insert(
            "newco".to_string(),
            Provisioning::InFlight { display_name: None },
        );
        let table = TenantTable::build(Ok(Vec::new()), &jobs, &view_pending(), "", 0, &i18n());
        assert!(table.polling);
        assert!(table.counts_poll.is_none());
        let html = render_rows(table);
        assert!(html.contains(r#"hx-trigger="every 3s""#));
        assert!(!html.contains("data-counts-poll"));
    }

    #[test]
    fn a_fragment_carries_the_cards_and_status_out_of_band() {
        let html = render_rows(table(&["acme"], &view_pending(), "", 0));
        assert!(html.contains(
            r#"id="tenant-stats" class="stat-grid stat-grid--2" hx-swap-oob="outerHTML""#
        ));
        assert!(html.contains(r#"id="tenant-counts-status" class="counts-status" role="status" hx-swap-oob="innerHTML""#));
        assert!(html.contains(r#"data-counts-state="pending""#));
        assert!(html.contains(r#"hx-trigger="every 2s""#));
        assert!(html.contains(r#"hx-sync="closest .table-card:abort""#));
    }
}

#[cfg(test)]
mod inventory_hook_tests {
    use super::*;
    use crate::tenant_inventory::{InventoryPhase, ResourceCell, TenantInventory};
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::{CountBasis, WriteObserver, WriteObservers};
    use helios_persistence::tenant::{TenantContext, TenantPermissions};

    /// A [`WebState`] over `storage` with its inventory, as `mount` builds it.
    fn state(
        storage: Arc<dyn ResourceStorage>,
        write_observer: Option<Arc<dyn WriteObserver>>,
    ) -> WebState {
        let source: Arc<dyn crate::ConformanceSource> = Arc::new(
            crate::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data")),
        );
        let inventory = TenantInventory::over_storage(Arc::clone(&storage));
        if let Some(fan_out) = write_observer.as_deref().and_then(|o| o.fan_out()) {
            fan_out.subscribe(inventory.observer());
        }
        WebState {
            version: "9.9.9",
            sp_catalog: Arc::new(crate::search_params::SpCatalog::new(source.clone())),
            nl: Arc::new(crate::NlSearch::default()),
            compartments: Arc::new(crate::compartments::CompartmentCatalog::new(source.clone())),
            conformance: source,
            tenants: Some(storage),
            tenant_inventory: Some(inventory),
            provisioning: Default::default(),
            data_dir: None,
            public_base_url: "http://localhost:8080".to_string(),
            self_base_url: "http://localhost:8080".to_string(),
            outbound_auth: Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            tenant_path_routing: false,
            fhir_version: helios_fhir::FhirVersion::R4,
            default_tenant: "default".to_string(),
            terminology: None,
            settings: None,
            bulk_provider: None,
            write_observer,
            patient_name_search: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            login: None,
        }
    }

    /// SQLite with `acme` registered and holding one Patient.
    async fn storage() -> Arc<dyn ResourceStorage> {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        let storage: Arc<dyn ResourceStorage> = Arc::new(backend);
        storage
            .register_tenant("acme", None)
            .await
            .expect("register");
        storage
            .create(
                &TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access()),
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("create");
        storage
    }

    /// Takes a view and waits until the refresh it started has published.
    async fn settled(inventory: &Arc<TenantInventory>) -> crate::tenant_inventory::InventoryView {
        let mut rx = inventory.subscribe();
        if inventory.view().refreshing {
            tokio::time::timeout(std::time::Duration::from_secs(10), rx.changed())
                .await
                .expect("the refresh ended in time")
                .expect("sender alive");
        }
        inventory.view()
    }

    async fn delete_acme(state: &WebState, purge: bool) {
        let response = delete(
            State(state.clone()),
            RequestLocale::default(),
            Path("acme".to_string()),
            Query(DeleteQuery {
                purge,
                q: String::new(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Deregistering keeps the data-only tenant in the inventory; purging
    /// invalidates it, and the next refresh measures it as gone.
    async fn deregistration_then_purge(state: WebState) {
        let inventory = state.tenant_inventory.clone().expect("inventory");
        let one = ResourceCell::Number {
            value: 1,
            basis: CountBasis::LiveResources,
            stale: false,
        };
        assert_eq!(settled(&inventory).await.cell("acme"), one);

        delete_acme(&state, false).await;
        assert_eq!(inventory.metrics().marked_stale, 1, "exactly once");
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Stale { last_error: None });
        assert!(
            view.discovered.contains_key("acme"),
            "data-only membership stays"
        );
        assert_eq!(settled(&inventory).await.cell("acme"), one);

        delete_acme(&state, true).await;
        assert_eq!(inventory.metrics().purges, 1, "exactly once");
        assert!(!inventory.view().discovered.contains_key("acme"));
        assert_eq!(
            settled(&inventory).await.cell("acme"),
            ResourceCell::MeasuredZero { stale: false }
        );
    }

    /// Without a server fan-out, the handler notifies the inventory itself.
    #[tokio::test]
    async fn ui_deletes_notify_the_inventory_directly() {
        deregistration_then_purge(state(storage().await, None)).await;
    }

    /// With the server's fan-out, the UI's own events reach the inventory
    /// through it, once; purges from elsewhere (`/admin/tenants`) arrive the
    /// same way.
    #[tokio::test]
    async fn ui_deletes_reach_a_subscribed_inventory_once_through_the_fan_out() {
        let observers = Arc::new(WriteObservers::new());
        let state = state(
            storage().await,
            Some(observers.clone() as Arc<dyn WriteObserver>),
        );
        assert_eq!(observers.len(), 1, "the inventory subscribed");
        deregistration_then_purge(state).await;
    }
}

#[cfg(test)]
mod provisioning_row_tests {
    use super::*;
    use helios_persistence::backends::sqlite::SqliteBackend;

    /// A view with nothing measured yet.
    fn inventory_view_for_tests() -> InventoryView {
        InventoryView {
            phase: InventoryPhase::Pending,
            discovered: Arc::default(),
            coverage: None,
            completed_at: None,
            refreshing: true,
            invalidated: BTreeSet::new(),
            counted: false,
        }
    }

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

        let view = inventory_view_for_tests();
        let registered = load_registry(&storage).await.expect("registry");
        let rows = merge_rows(&registered, &jobs, &view);
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
        let rows = merge_rows(&registered, &jobs, &view);
        let acme: Vec<_> = rows.iter().filter(|r| r.id == "acme").collect();
        assert_eq!(acme.len(), 1);
        assert!(!acme[0].provisioning);
        assert!(
            acme[0].failed.is_none(),
            "registered row, not the failed notice"
        );
    }
}
