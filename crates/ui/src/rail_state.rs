//! Server-side state for the eight sidebar "rails" (Resources, Search, Saved
//! Queries, Search Parameters, Compartments, View Definitions, SQL Queries,
//! SQL Views): per user, per tenant, per page, what was picked most recently
//! and what the very last pick was.
//!
//! This module is the shared contract [`crate::resolve_prefs`] and every rail
//! page build on: it owns the document shape, reads it off the request with no
//! extra store access, mutates it purely, and writes it back best-effort. It
//! has no dependency on Askama or on any handler — everything here is either
//! pure logic over [`serde_json::Value`] or the one `async fn` that persists a
//! result, so it is trivially unit-testable and safe to call from any page.
//!
//! # Document shape
//!
//! One top-level `rails` key in the `/_user/settings` document
//! (`helios_persistence::core::user_settings`), keyed by [`RailPage::key`]:
//!
//! ```json
//! "rails": {
//!   "resources":        { "last": "Observation", "recent": [{"id": "Observation"}, {"id": "Patient"}] },
//!   "searchParameters":  { "last": "", "recent": [{"id": "Encounter"}] },
//!   "viewDefinitions":  { "last": "abc", "recent": [{"id": "abc", "name": "active_patients", "meta": "Patient"}] }
//! }
//! ```
//!
//! - `recent` is newest first, holds no duplicate `id`s, and is capped at
//!   [`MAX_RECENT`]. `name`/`meta` are an optional snapshot — the type rails
//!   never write them (their `id` *is* the label, and the live rail always has
//!   the item); the SQL rails do, because their rail is a server-paged search
//!   (#741) that will not always have the recent item on the current page.
//! - `last`, when non-empty, is always an `id` and always equals `recent[0]`.
//!   An empty string is the one documented exception: Search Parameters'
//!   explicit "All types", which is not an item and therefore cannot be a
//!   recent — it sets `last: ""` and leaves `recent` untouched.
//! - `rails` (or one `rails.<page>` entry) being entirely absent means "never
//!   selected on this page", not "reset to a default" — pages fall back to
//!   their own default in that case.
//! - `recent` is a JSON array — not sibling-keyed like `savedQueries` — on
//!   purpose: it is a small, bounded cache rewritten wholesale on every
//!   change, the same shape and justification as `recentSearches`.
//!
//! Like every other key not in
//! [`GLOBAL_SETTINGS_KEYS`](helios_persistence::core::GLOBAL_SETTINGS_KEYS),
//! `rails` is tenant-scoped by default: it lives under `byTenant.<tenant>` and
//! is erased by that tenant's purge, with no code change required here.
//!
//! # How a page uses this module
//!
//! 1. **Read**: [`resolve_prefs`](crate::resolve_prefs) already fetched the
//!    settings document once for the request and stamped a [`RequestSettings`]
//!    extension. A handler extracts it (`RequestSettings` implements
//!    [`axum::extract::FromRequestParts`]) and calls
//!    [`RequestSettings::rail`] with the page and the request's effective
//!    tenant to get a sanitized [`RailState`] — no further store access.
//! 2. **Mutate**: [`RailState::select`], [`RailState::select_all`], and
//!    [`RailState::prune`] are pure functions that return `Some(new_state)`
//!    only when something actually changed, so a handler writes only when it
//!    must.
//! 3. **Persist**: when a mutation returned `Some`, [`persist`] writes it back
//!    as a tenant-scoped merge patch, best-effort.
//! 4. **Render**: [`RailState::resolve_recents`] turns the stored `recent`
//!    list into render-ready rows against the page's *live* rail, applying the
//!    live-vs-snapshot rule and an optional validity filter.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use helios_persistence::core::{SettingsStore, project_for_tenant, scope_merge_patch};
use serde::Serialize;
use serde_json::{Map, Value};

/// The cap on `rails.<page>.recent`: the newest [`MAX_RECENT`] selections are
/// kept, oldest dropped. The single source of truth for the number — reads,
/// mutations, and any template that publishes it to the client (as
/// `data-max-recent`, so a tampered client can never be shown more than the
/// server itself would keep) all take it from here.
pub(crate) const MAX_RECENT: usize = 5;

/// One of the eight pages that carries rail state, and the JSON key its
/// record lives under in `rails.<page>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RailPage {
    /// The Resources workspace's type rail (#282, #541).
    Resources,
    /// The Search page's type rail (#255, #541).
    Search,
    /// The Saved Queries page's type rail (#234, #541).
    Queries,
    /// The SearchParameter viewer's type rail (#238) — the only page that uses
    /// [`RailState::select_all`]'s `last: ""` "All types" state.
    SearchParameters,
    /// The Compartment viewer's definition rail (#237). Remembers `last` only;
    /// its template does not render a "Recently used" group.
    Compartments,
    /// The SQL on FHIR View Definitions rail (#649, server-paged since #741).
    ViewDefinitions,
    /// The SQL on FHIR Queries rail (#649).
    SqlQueries,
    /// The SQL on FHIR Views rail (#649).
    SqlViews,
}

impl RailPage {
    /// The JSON key this page's record lives under in `rails.<page>`.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Resources => "resources",
            Self::Search => "search",
            Self::Queries => "queries",
            Self::SearchParameters => "searchParameters",
            Self::Compartments => "compartments",
            Self::ViewDefinitions => "viewDefinitions",
            Self::SqlQueries => "sqlQueries",
            Self::SqlViews => "sqlViews",
        }
    }
}

/// One entry of `rails.<page>.recent`: an id plus the optional snapshot the
/// SQL rails write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RailEntry {
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) meta: Option<String>,
}

impl RailEntry {
    /// A recent entry carrying only its id — what every "type" rail
    /// (Resources, Search, Saved Queries, Search Parameters) writes. Their
    /// live rail always has the item and the id doubles as the label, so no
    /// snapshot is needed.
    pub(crate) fn id_only(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            meta: None,
        }
    }

    /// A recent entry with a name/meta snapshot — what the SQL rails (View
    /// Definitions, SQL Queries, SQL Views) write, so a recent still renders
    /// when the live item is off the current search page (#741).
    pub(crate) fn with_snapshot(
        id: impl Into<String>,
        name: impl Into<String>,
        meta: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: Some(name.into()),
            meta,
        }
    }
}

/// One page's sanitized rail record: the last selection and the recents list.
///
/// `last: Some("")` is the documented "explicitly all" state (Search
/// Parameters only); `last: None` means no explicit last selection is
/// recorded. Every value obtained through [`RequestSettings::rail`] already
/// satisfies the invariants in the module docs — the mutation methods
/// preserve them, so a `RailState` is always well-formed once read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RailState {
    pub(crate) last: Option<String>,
    pub(crate) recent: Vec<RailEntry>,
}

impl RailState {
    /// Reads and sanitizes one page's already tenant-projected `rails.<page>`
    /// value. Tolerates any malformed shape without panicking:
    ///
    /// - missing or non-object input yields the empty (never-selected) state;
    /// - a `recent` entry with no string `id` is dropped;
    /// - a duplicate `id` collapses to its first (most recent) occurrence;
    /// - the surviving list is capped at [`MAX_RECENT`];
    /// - a non-empty `last` that does not match `recent[0]` — including a
    ///   missing `last` alongside a non-empty `recent` — is corrected to
    ///   `recent[0]`, or to `None` when `recent` ends up empty.
    ///
    /// The one exception to that last rule is `last: ""` ("All types"), which
    /// is preserved verbatim and never touches `recent`.
    fn from_raw(raw: Option<&Value>) -> Self {
        let object = raw.and_then(Value::as_object);

        let mut recent = Vec::new();
        let mut seen_ids = HashSet::new();
        if let Some(items) = object
            .and_then(|o| o.get("recent"))
            .and_then(Value::as_array)
        {
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if !seen_ids.insert(id.to_string()) {
                    continue;
                }
                recent.push(RailEntry {
                    id: id.to_string(),
                    name: item.get("name").and_then(Value::as_str).map(str::to_string),
                    meta: item.get("meta").and_then(Value::as_str).map(str::to_string),
                });
            }
        }
        recent.truncate(MAX_RECENT);

        let raw_last = object.and_then(|o| o.get("last")).and_then(Value::as_str);
        let last = match raw_last {
            Some("") => Some(String::new()),
            Some(id) if recent.first().is_some_and(|first| first.id == id) => Some(id.to_string()),
            _ => recent.first().map(|first| first.id.clone()),
        };

        Self { last, recent }
    }

    /// Moves (or inserts) `entry` to the front of `recent`, capped at
    /// [`MAX_RECENT`], and sets `last` to its id.
    ///
    /// If `entry.id` was already present in `recent`, its old position is
    /// dropped and its snapshot is replaced by `entry`'s — a re-selection
    /// always refreshes the snapshot along with the position, and expels the
    /// oldest entry once the list would exceed the cap.
    ///
    /// Returns `None` when nothing would change: `entry` was already
    /// `recent[0]` with an identical snapshot, and `last` already named it.
    ///
    /// `entry.id` must not be the empty string — that sentinel is reserved for
    /// [`Self::select_all`]'s "All types" state.
    pub(crate) fn select(&self, entry: RailEntry) -> Option<Self> {
        let unchanged =
            self.last.as_deref() == Some(entry.id.as_str()) && self.recent.first() == Some(&entry);
        if unchanged {
            return None;
        }

        let mut recent = Vec::with_capacity((self.recent.len() + 1).min(MAX_RECENT));
        recent.push(entry.clone());
        recent.extend(self.recent.iter().filter(|e| e.id != entry.id).cloned());
        recent.truncate(MAX_RECENT);

        Some(Self {
            last: Some(entry.id),
            recent,
        })
    }

    /// Sets `last` to `Some("")` ("explicitly all"), leaving `recent`
    /// untouched — Search Parameters' only user of this method. Returns
    /// `None` when `last` was already `""`.
    pub(crate) fn select_all(&self) -> Option<Self> {
        if self.last.as_deref() == Some("") {
            return None;
        }
        Some(Self {
            last: Some(String::new()),
            recent: self.recent.clone(),
        })
    }

    /// Removes `id` from `recent`, and clears `last` when it named `id`
    /// (without promoting a new `recent[0]` — the page's normal resolution
    /// order takes over from an absent `last`). Returns `None` when `id` was
    /// neither in `recent` nor `last`, the no-op case a caller can skip
    /// writing for.
    ///
    /// The type rails never call this: a stale type is simply hidden by
    /// [`Self::resolve_recents`]'s `is_valid` filter, never written away,
    /// since a stale `last` already falls back to the page default in
    /// silence. The SQL rails prune on a stale explicit selection's click.
    pub(crate) fn prune(&self, id: &str) -> Option<Self> {
        let in_recent = self.recent.iter().any(|e| e.id == id);
        let is_last = self.last.as_deref() == Some(id);
        if !in_recent && !is_last {
            return None;
        }
        Some(Self {
            last: if is_last { None } else { self.last.clone() },
            recent: if in_recent {
                self.recent.iter().filter(|e| e.id != id).cloned().collect()
            } else {
                self.recent.clone()
            },
        })
    }

    /// Resolves `recent`, in order, against a rail's currently-live items
    /// into render-ready rows — the "Recently used" group.
    ///
    /// For each recent entry: if its id is a key of `live`, the live item's
    /// label/meta/count/href/current are used, since the rail already knows
    /// the real, current values; otherwise the entry's own snapshot is used
    /// (`name` as the label, falling back to the id when there is none, plus
    /// `meta`), and `snapshot_href(id)` builds its link — that is the caller's
    /// job, since only it knows the page's URL shape.
    ///
    /// `is_valid`, when given, hides an entry whose id it rejects (e.g. a
    /// resource type no longer in the active FHIR version) — filtered before
    /// resolution, so a hidden id costs nothing and is never pruned from the
    /// stored state just because a render skipped it.
    pub(crate) fn resolve_recents(
        &self,
        live: &HashMap<String, LiveRailItem>,
        snapshot_href: impl Fn(&str) -> String,
        is_valid: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<ResolvedRailEntry> {
        self.recent
            .iter()
            .filter(|entry| is_valid.is_none_or(|valid| valid(&entry.id)))
            .map(|entry| match live.get(&entry.id) {
                Some(item) => ResolvedRailEntry {
                    id: entry.id.clone(),
                    label: item.label.clone(),
                    meta: item.meta.clone(),
                    href: item.href.clone(),
                    count: item.count.clone(),
                    current: item.current,
                },
                None => ResolvedRailEntry {
                    id: entry.id.clone(),
                    label: entry.name.clone().unwrap_or_else(|| entry.id.clone()),
                    meta: entry.meta.clone(),
                    href: snapshot_href(&entry.id),
                    count: None,
                    current: false,
                },
            })
            .collect()
    }
}

/// Render-ready data for one item currently present in a rail — the "live"
/// half of [`RailState::resolve_recents`]'s resolution. Built by the
/// caller from whatever already renders that page's primary list (e.g. the
/// shared type rail's entries, or a View Definition search hit); `current`
/// carries whatever the caller already computed for "is this the selection
/// this page is rendering right now".
#[derive(Debug, Clone)]
pub(crate) struct LiveRailItem {
    pub(crate) label: String,
    pub(crate) meta: Option<String>,
    pub(crate) count: Option<String>,
    pub(crate) href: String,
    pub(crate) current: bool,
}

/// One resolved "Recently used" row, ready to render: whichever of the live
/// rail's current data or the stored snapshot applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRailEntry {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) meta: Option<String>,
    pub(crate) href: String,
    pub(crate) count: Option<String>,
    pub(crate) current: bool,
}

/// The per-user settings document [`crate::resolve_prefs`] already fetched for
/// this request, stamped into request extensions so every subsequent handler
/// can read a rail's state without another [`SettingsStore`] round trip.
///
/// Carries the resolved settings key, the raw stored document (`None` when no
/// settings store is configured, the user has never stored anything, or the
/// read failed), and its optimistic-lock `version`. Nothing here is
/// tenant-scoped yet — call [`RequestSettings::rail`] to project and sanitize
/// one page's rail state for the request's effective tenant.
#[derive(Debug, Clone)]
pub(crate) struct RequestSettings {
    pub(crate) user_key: String,
    pub(crate) document: Option<Value>,
    /// Not read by any handler yet: [`persist`] deliberately sends no
    /// `if_match_version` precondition (see its doc — a rail record is a
    /// single small value where last-write-wins is correct), so nothing in
    /// this ticket needs it. Kept for parity with the version every other
    /// `get_settings` caller in this crate already carries, and in case a
    /// future rail (or a future page reading more than one key off the same
    /// fetch) ever does need to condition a write on it.
    #[allow(dead_code)]
    pub(crate) version: i64,
}

impl RequestSettings {
    /// The sanitized [`RailState`] for `page`, scoped to `tenant` — the entry
    /// point every handler reads a rail's stored selection through.
    ///
    /// Reads through [`project_for_tenant`] against the document this
    /// request's [`resolve_prefs`](crate::resolve_prefs) already fetched, so
    /// this call performs no I/O of its own: it is pure in-memory JSON
    /// navigation over data already in hand, exactly like a legacy document
    /// and a normalized one project to the same view here.
    pub(crate) fn rail(&self, page: RailPage, tenant: &str) -> RailState {
        let Some(document) = &self.document else {
            return RailState::default();
        };
        let projected = project_for_tenant(document, tenant);
        let raw = projected
            .get("rails")
            .and_then(|rails| rails.get(page.key()));
        RailState::from_raw(raw)
    }
}

impl<S> axum::extract::FromRequestParts<S> for RequestSettings
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    /// Returns the extension [`crate::resolve_prefs`] stamped, or — when it
    /// did not run (a request extractor test, or the `/ui/json-view/render`
    /// fast path that skips the settings read entirely) — a default with a
    /// freshly-resolved user key and no document, mirroring how
    /// [`crate::RequestVersion`] and [`crate::RequestTenant`] fall back.
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        if let Some(settings) = parts.extensions.get::<Self>() {
            return Ok(settings.clone());
        }
        Ok(Self {
            user_key: crate::settings_user_key(parts.extensions.get::<helios_auth::Principal>()),
            document: None,
            version: 0,
        })
    }
}

/// Writes `state`'s `rails.<page>` slice, scoped to `tenant`, as a JSON merge
/// patch. Best-effort: a store error is logged at `warn` and otherwise
/// swallowed — this never surfaces to the caller or the HTTP response, the
/// same pattern `set_version`/`set_tenant` already use for preference writes.
///
/// A `None` store (no settings backend configured) is a silent no-op: there is
/// nothing to write to, and every caller already renders correctly without a
/// stored rail state.
///
/// No `if_match_version` precondition is sent: the store performs the
/// read-modify-write atomically, and a page's rail record is a single small
/// value overwritten wholesale on each call, so last-write-wins is correct —
/// two tabs racing to record a selection settle on whichever lands last,
/// exactly what a user watching only one of them would expect.
///
/// Does not normalize a legacy (pre-#313) document itself: the REST handler
/// already normalizes on the client's first write to `/_user/settings`, and
/// [`RequestSettings::rail`] prefers the normalized `byTenant` subtree over a
/// same-named legacy top-level key — so even a hand-edited legacy `rails` key
/// would simply be superseded once normalized, never silently lost.
pub(crate) async fn persist(
    settings: &Option<Arc<dyn SettingsStore>>,
    user_key: &str,
    tenant: &str,
    page: RailPage,
    state: &RailState,
) {
    let Some(store) = settings else {
        return;
    };

    let page_value = match serde_json::to_value(state) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(page = page.key(), error = %error, "failed to serialize rail state");
            return;
        }
    };
    let mut rails = Map::new();
    rails.insert(page.key().to_string(), page_value);
    let mut patch = Map::new();
    patch.insert("rails".to_string(), Value::Object(rails));
    let patch = scope_merge_patch(Value::Object(patch), tenant);

    if let Err(error) = store.patch_settings(user_key, patch, None).await {
        tracing::warn!(page = page.key(), tenant, error = %error, "failed to persist rail state");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Reading and sanitizing ────────────────────────────────────────────

    #[test]
    fn absent_document_reads_as_the_empty_state() {
        let settings = RequestSettings {
            user_key: "l2:".to_string(),
            document: None,
            version: 0,
        };
        assert_eq!(
            settings.rail(RailPage::Resources, "acme"),
            RailState::default()
        );
    }

    #[test]
    fn absent_page_key_reads_as_the_empty_state() {
        let settings = RequestSettings {
            user_key: "l2:".to_string(),
            document: Some(
                json!({"byTenant": {"acme": {"rails": {"search": {"last": "Patient", "recent": [{"id": "Patient"}]}}}}}),
            ),
            version: 3,
        };
        assert_eq!(
            settings.rail(RailPage::Resources, "acme"),
            RailState::default()
        );
    }

    #[test]
    fn reads_the_normalized_tenant_subtree_and_not_another_tenants() {
        let document = json!({
            "byTenant": {
                "acme": {"rails": {"resources": {"last": "Observation", "recent": [{"id": "Observation"}]}}},
                "beta": {"rails": {"resources": {"last": "Patient", "recent": [{"id": "Patient"}]}}}
            }
        });
        let settings = RequestSettings {
            user_key: "l2:".to_string(),
            document: Some(document),
            version: 1,
        };
        let acme = settings.rail(RailPage::Resources, "acme");
        assert_eq!(acme.last.as_deref(), Some("Observation"));
        assert_eq!(acme.recent, vec![RailEntry::id_only("Observation")]);

        let other = settings.rail(RailPage::Resources, "gamma");
        assert_eq!(other, RailState::default());
    }

    /// A pre-#313 legacy document with `rails` at the top level is read
    /// according to `project_for_tenant`'s attribution rules — same as every
    /// other tenant-scoped key.
    #[test]
    fn reads_a_legacy_top_level_rails_key_via_the_shared_attribution_rules() {
        let document = json!({
            "tenantId": "acme",
            "rails": {"resources": {"last": "Observation", "recent": [{"id": "Observation"}]}}
        });
        let settings = RequestSettings {
            user_key: "l2:".to_string(),
            document: Some(document),
            version: 1,
        };
        assert_eq!(
            settings.rail(RailPage::Resources, "acme").last.as_deref(),
            Some("Observation")
        );
        assert_eq!(
            settings.rail(RailPage::Resources, "beta"),
            RailState::default()
        );
    }

    #[test]
    fn sanitizes_entries_without_a_string_id() {
        let raw = json!({"recent": [{"id": "Patient"}, {"name": "no id"}, {"id": 42}]});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.recent, vec![RailEntry::id_only("Patient")]);
    }

    #[test]
    fn sanitizes_duplicate_ids_keeping_the_first() {
        let raw = json!({
            "recent": [
                {"id": "Patient", "name": "first"},
                {"id": "Observation"},
                {"id": "Patient", "name": "second"}
            ]
        });
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(
            state.recent,
            vec![
                RailEntry::with_snapshot("Patient", "first", None),
                RailEntry::id_only("Observation"),
            ]
        );
    }

    #[test]
    fn sanitizes_recent_longer_than_max_recent() {
        let items: Vec<Value> = (0..MAX_RECENT + 3)
            .map(|i| json!({"id": format!("id{i}")}))
            .collect();
        let raw = json!({"recent": items});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.recent.len(), MAX_RECENT);
        assert_eq!(state.recent[0].id, "id0");
    }

    #[test]
    fn sanitizes_a_last_that_disagrees_with_recent_zero() {
        let raw = json!({"last": "Observation", "recent": [{"id": "Patient"}]});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.last.as_deref(), Some("Patient"));
    }

    #[test]
    fn sanitizes_a_missing_last_alongside_a_non_empty_recent() {
        let raw = json!({"recent": [{"id": "Patient"}]});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.last.as_deref(), Some("Patient"));
    }

    #[test]
    fn sanitizes_a_stale_last_with_an_empty_recent_to_absent() {
        let raw = json!({"last": "Patient", "recent": []});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.last, None);
    }

    #[test]
    fn preserves_the_all_types_exception_without_touching_recent() {
        let raw = json!({"last": "", "recent": [{"id": "Encounter"}]});
        let state = RailState::from_raw(Some(&raw));
        assert_eq!(state.last.as_deref(), Some(""));
        assert_eq!(state.recent, vec![RailEntry::id_only("Encounter")]);
    }

    #[test]
    fn a_malformed_non_object_page_value_never_panics() {
        assert_eq!(
            RailState::from_raw(Some(&json!("corrupt"))),
            RailState::default()
        );
        assert_eq!(
            RailState::from_raw(Some(&json!(["corrupt"]))),
            RailState::default()
        );
        assert_eq!(RailState::from_raw(None), RailState::default());
    }

    // ── Pure mutations ────────────────────────────────────────────────────

    #[test]
    fn select_inserts_at_the_front() {
        let state = RailState::default();
        let next = state.select(RailEntry::id_only("Patient")).unwrap();
        assert_eq!(next.last.as_deref(), Some("Patient"));
        assert_eq!(next.recent, vec![RailEntry::id_only("Patient")]);
    }

    #[test]
    fn select_moves_a_reselection_to_the_front_without_duplicating() {
        let state = RailState {
            last: Some("Observation".to_string()),
            recent: vec![
                RailEntry::id_only("Observation"),
                RailEntry::id_only("Patient"),
            ],
        };
        let next = state.select(RailEntry::id_only("Patient")).unwrap();
        assert_eq!(next.last.as_deref(), Some("Patient"));
        assert_eq!(
            next.recent,
            vec![
                RailEntry::id_only("Patient"),
                RailEntry::id_only("Observation"),
            ]
        );
    }

    #[test]
    fn select_caps_at_max_recent_expelling_the_oldest() {
        let recent: Vec<RailEntry> = (0..MAX_RECENT)
            .map(|i| RailEntry::id_only(format!("id{i}")))
            .collect();
        let state = RailState {
            last: Some("id0".to_string()),
            recent,
        };
        let next = state.select(RailEntry::id_only("new")).unwrap();
        assert_eq!(next.recent.len(), MAX_RECENT);
        assert_eq!(next.recent[0].id, "new");
        assert!(
            !next
                .recent
                .iter()
                .any(|e| e.id == format!("id{}", MAX_RECENT - 1))
        );
    }

    #[test]
    fn select_updates_the_snapshot_of_an_existing_entry() {
        let state = RailState {
            last: Some("abc".to_string()),
            recent: vec![RailEntry::with_snapshot("abc", "old_name", None)],
        };
        let next = state
            .select(RailEntry::with_snapshot(
                "abc",
                "new_name",
                Some("Patient".to_string()),
            ))
            .unwrap();
        assert_eq!(
            next.recent,
            vec![RailEntry::with_snapshot(
                "abc",
                "new_name",
                Some("Patient".to_string())
            )]
        );
    }

    #[test]
    fn select_is_a_no_op_when_already_current() {
        let state = RailState {
            last: Some("Patient".to_string()),
            recent: vec![RailEntry::id_only("Patient")],
        };
        assert_eq!(state.select(RailEntry::id_only("Patient")), None);
    }

    #[test]
    fn select_all_sets_last_to_empty_string_and_keeps_recent() {
        let state = RailState {
            last: Some("Encounter".to_string()),
            recent: vec![RailEntry::id_only("Encounter")],
        };
        let next = state.select_all().unwrap();
        assert_eq!(next.last.as_deref(), Some(""));
        assert_eq!(next.recent, state.recent);
    }

    #[test]
    fn select_all_is_a_no_op_when_already_all() {
        let state = RailState {
            last: Some(String::new()),
            recent: vec![RailEntry::id_only("Encounter")],
        };
        assert_eq!(state.select_all(), None);
    }

    #[test]
    fn prune_removes_the_entry_and_clears_a_matching_last() {
        let state = RailState {
            last: Some("Patient".to_string()),
            recent: vec![
                RailEntry::id_only("Patient"),
                RailEntry::id_only("Observation"),
            ],
        };
        let next = state.prune("Patient").unwrap();
        assert_eq!(next.last, None);
        assert_eq!(next.recent, vec![RailEntry::id_only("Observation")]);
    }

    #[test]
    fn prune_leaves_last_alone_when_it_named_a_different_id() {
        let state = RailState {
            last: Some("Observation".to_string()),
            recent: vec![
                RailEntry::id_only("Observation"),
                RailEntry::id_only("Patient"),
            ],
        };
        let next = state.prune("Patient").unwrap();
        assert_eq!(next.last.as_deref(), Some("Observation"));
        assert_eq!(next.recent, vec![RailEntry::id_only("Observation")]);
    }

    #[test]
    fn prune_is_a_no_op_when_the_id_was_never_recorded() {
        let state = RailState {
            last: Some("Observation".to_string()),
            recent: vec![RailEntry::id_only("Observation")],
        };
        assert_eq!(state.prune("Patient"), None);
    }

    // ── Resolving recents against the live rail ──────────────────────────

    #[test]
    fn resolve_recents_prefers_live_data_over_the_snapshot() {
        let state = RailState {
            last: Some("abc".to_string()),
            recent: vec![RailEntry::with_snapshot(
                "abc",
                "stale_name",
                Some("stale_meta".to_string()),
            )],
        };
        let mut live = HashMap::new();
        live.insert(
            "abc".to_string(),
            LiveRailItem {
                label: "fresh_name".to_string(),
                meta: Some("fresh_meta".to_string()),
                count: Some("3".to_string()),
                href: "/ui/sql/view-definitions?id=abc".to_string(),
                current: true,
            },
        );
        let resolved = state.resolve_recents(&live, |id| format!("/fallback?id={id}"), None);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, "fresh_name");
        assert_eq!(resolved[0].meta.as_deref(), Some("fresh_meta"));
        assert_eq!(resolved[0].href, "/ui/sql/view-definitions?id=abc");
        assert!(resolved[0].current);
    }

    #[test]
    fn resolve_recents_falls_back_to_the_snapshot_for_an_absent_live_item() {
        let state = RailState {
            last: Some("abc".to_string()),
            recent: vec![RailEntry::with_snapshot(
                "abc",
                "snapshot_name",
                Some("Patient".to_string()),
            )],
        };
        let live = HashMap::new();
        let resolved = state.resolve_recents(&live, |id| format!("/fallback?id={id}"), None);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].label, "snapshot_name");
        assert_eq!(resolved[0].meta.as_deref(), Some("Patient"));
        assert_eq!(resolved[0].href, "/fallback?id=abc");
        assert!(!resolved[0].current);
    }

    #[test]
    fn resolve_recents_hides_but_does_not_prune_an_invalid_entry() {
        let state = RailState {
            last: Some("Observation".to_string()),
            recent: vec![
                RailEntry::id_only("Observation"),
                RailEntry::id_only("RetiredType"),
            ],
        };
        let live = HashMap::new();
        let is_valid: &dyn Fn(&str) -> bool = &|id| id != "RetiredType";
        let resolved = state.resolve_recents(&live, |id| id.to_string(), Some(is_valid));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "Observation");
        // Hiding must not have pruned the stored state.
        assert_eq!(state.recent.len(), 2);
    }

    // ── Persistence ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn persist_is_a_silent_no_op_with_no_settings_store() {
        let settings: Option<Arc<dyn SettingsStore>> = None;
        let state = RailState {
            last: Some("Patient".to_string()),
            recent: vec![RailEntry::id_only("Patient")],
        };
        // No store to observe; this must simply not panic or block.
        persist(&settings, "l2:", "acme", RailPage::Resources, &state).await;
    }

    #[tokio::test]
    async fn persist_writes_a_tenant_scoped_patch_and_nothing_else() {
        let store = Arc::new(mock_store::MockSettingsStore::default());
        // Pre-existing content on the same page (search) and tenant (beta)
        // must survive untouched.
        store
            .patch_settings(
                "l2:",
                json!({"theme": "dark", "byTenant": {"beta": {"rails": {"search": {"last": "X", "recent": [{"id": "X"}]}}}}}),
                None,
            )
            .await
            .unwrap();

        let settings: Option<Arc<dyn SettingsStore>> = Some(store.clone());
        let state = RailState {
            last: Some("Patient".to_string()),
            recent: vec![RailEntry::id_only("Patient")],
        };
        persist(&settings, "l2:", "acme", RailPage::Resources, &state).await;

        let document = store.get_settings("l2:").await.unwrap().unwrap().document;
        assert_eq!(
            document["byTenant"]["acme"]["rails"]["resources"],
            json!({"last": "Patient", "recent": [{"id": "Patient"}]})
        );
        assert_eq!(document["theme"], "dark");
        assert_eq!(
            document["byTenant"]["beta"]["rails"]["search"]["last"], "X",
            "another page/tenant's record must be untouched"
        );
    }

    #[tokio::test]
    async fn persist_swallows_a_store_error() {
        let store = Arc::new(mock_store::MockSettingsStore::failing());
        let settings: Option<Arc<dyn SettingsStore>> = Some(store);
        let state = RailState {
            last: Some("Patient".to_string()),
            recent: vec![RailEntry::id_only("Patient")],
        };
        // Must not propagate a panic or error to the caller.
        persist(&settings, "l2:", "acme", RailPage::Resources, &state).await;
    }

    // ── Middleware wiring: exactly one read, and a handler can extract
    //    RequestSettings ────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_prefs_stamps_request_settings_readable_by_a_handler() {
        use axum::{Router, body::Body, http::Request, routing::get};
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        async fn probe(settings: RequestSettings) -> String {
            format!(
                "{}|{}|{}",
                settings.user_key,
                settings.document.is_some(),
                settings.version
            )
        }

        let store = Arc::new(mock_store::MockSettingsStore::default());
        store
            .patch_settings("l2:", json!({"theme": "dark"}), None)
            .await
            .unwrap();
        let state = test_web_state(Some(store.clone()));

        let app = Router::new()
            .route("/probe", get(probe))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::resolve_prefs,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(body, "l2:|true|1");
        assert_eq!(
            store.get_settings_calls(),
            1,
            "resolve_prefs must read the settings store exactly once per request"
        );
    }

    /// A minimal but real [`crate::WebState`] for driving [`crate::resolve_prefs`]
    /// directly in a unit test, without mounting the full production router.
    fn test_web_state(settings: Option<Arc<dyn SettingsStore>>) -> crate::WebState {
        let source: Arc<dyn crate::ConformanceSource> = Arc::new(
            crate::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data")),
        );
        crate::WebState {
            version: "9.9.9",
            sp_catalog: Arc::new(crate::search_params::SpCatalog::new(source.clone())),
            nl: Arc::new(crate::NlSearch::default()),
            compartments: Arc::new(crate::compartments::CompartmentCatalog::new(source.clone())),
            conformance: source,
            tenants: None,
            provisioning: Default::default(),
            data_dir: None,
            public_base_url: "http://localhost:8080".to_string(),
            self_base_url: "http://localhost:8080".to_string(),
            tenant_path_routing: false,
            fhir_version: helios_fhir::FhirVersion::R4,
            default_tenant: "default".to_string(),
            terminology: None,
            settings,
            bulk_provider: None,
            patient_name_search: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// A tiny in-memory [`SettingsStore`] double, local to this module's unit
    /// tests. A separate, `pub` sibling reusable from integration tests
    /// (`crates/ui/tests/support/mod.rs`) exists for the handler-level HTTP
    /// tests — integration tests compile as their own crate and cannot
    /// reach this module's `pub(crate)` items, so the two cannot be shared
    /// despite the near-identical implementation.
    mod mock_store {
        use super::*;
        use async_trait::async_trait;
        use chrono::Utc;
        use helios_persistence::{
            StorageResult,
            core::{StoredUserSettings, apply_merge_patch},
            error::{ConcurrencyError, StorageError},
        };
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Default)]
        pub(super) struct MockSettingsStore {
            documents: Mutex<HashMap<String, StoredUserSettings>>,
            reads: AtomicUsize,
            /// When set, every write fails — exercises [`super::persist`]'s
            /// best-effort error handling.
            fail_writes: bool,
        }

        impl MockSettingsStore {
            pub(super) fn failing() -> Self {
                Self {
                    fail_writes: true,
                    ..Default::default()
                }
            }

            pub(super) fn get_settings_calls(&self) -> usize {
                self.reads.load(Ordering::SeqCst)
            }
        }

        #[async_trait]
        impl SettingsStore for MockSettingsStore {
            async fn get_settings(
                &self,
                user_key: &str,
            ) -> StorageResult<Option<StoredUserSettings>> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(self.documents.lock().unwrap().get(user_key).cloned())
            }

            async fn put_settings(
                &self,
                user_key: &str,
                document: Value,
                if_match_version: Option<i64>,
            ) -> StorageResult<StoredUserSettings> {
                if self.fail_writes {
                    return Err(write_failure(user_key));
                }
                let mut documents = self.documents.lock().unwrap();
                let current_version = documents.get(user_key).map(|s| s.version).unwrap_or(0);
                if let Some(expected) = if_match_version
                    && expected != current_version
                {
                    return Err(lock_failure(user_key, expected, current_version));
                }
                let stored = StoredUserSettings {
                    user_key: user_key.to_string(),
                    document,
                    version: current_version + 1,
                    updated_at: Utc::now(),
                };
                documents.insert(user_key.to_string(), stored.clone());
                Ok(stored)
            }

            async fn patch_settings(
                &self,
                user_key: &str,
                merge_patch: Value,
                if_match_version: Option<i64>,
            ) -> StorageResult<StoredUserSettings> {
                if self.fail_writes {
                    return Err(write_failure(user_key));
                }
                let mut documents = self.documents.lock().unwrap();
                let current = documents.get(user_key).cloned();
                let current_version = current.as_ref().map(|s| s.version).unwrap_or(0);
                if let Some(expected) = if_match_version
                    && expected != current_version
                {
                    return Err(lock_failure(user_key, expected, current_version));
                }
                let base = current
                    .map(|s| s.document)
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let stored = StoredUserSettings {
                    user_key: user_key.to_string(),
                    document: apply_merge_patch(base, &merge_patch),
                    version: current_version + 1,
                    updated_at: Utc::now(),
                };
                documents.insert(user_key.to_string(), stored.clone());
                Ok(stored)
            }

            async fn delete_settings(&self, user_key: &str) -> StorageResult<bool> {
                Ok(self.documents.lock().unwrap().remove(user_key).is_some())
            }

            async fn purge_tenant_settings(&self, tenant_id: &str) -> StorageResult<u64> {
                let mut documents = self.documents.lock().unwrap();
                let mut changed = 0;
                for stored in documents.values_mut() {
                    if helios_persistence::core::purge_tenant_subtree(
                        &mut stored.document,
                        tenant_id,
                    ) {
                        stored.version += 1;
                        changed += 1;
                    }
                }
                Ok(changed)
            }
        }

        fn lock_failure(user_key: &str, expected: i64, actual: i64) -> StorageError {
            StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
                resource_type: "UserSettings".to_string(),
                id: user_key.to_string(),
                expected_etag: format!("\"{expected}\""),
                actual_etag: Some(format!("\"{actual}\"")),
            })
        }

        fn write_failure(user_key: &str) -> StorageError {
            StorageError::Concurrency(ConcurrencyError::Deadlock {
                resource_type: "UserSettings".to_string(),
                id: user_key.to_string(),
            })
        }
    }
}
