//! History interaction handlers.
//!
//! Implements the FHIR [history interaction](https://hl7.org/fhir/http.html#history):
//! - Instance history: `GET [base]/[type]/[id]/_history`
//! - Type history: `GET [base]/[type]/_history`
//! - System history: `GET [base]/_history`
//!
//! Also implements FHIR v6.0.0 Trial Use delete history operations:
//! - Delete instance history: `DELETE [base]/[type]/[id]/_history`
//! - Delete specific version: `DELETE [base]/[type]/[id]/_history/[vid]`
//!
//! History reads accept `_count`, `_since`, `_format` and `_cursor` (a `next`
//! link's opaque page position), each at most once. History Bundles carry a
//! `self` link and, when another page exists, a `next` link (#2382).
//! `_pretty`, `_summary` and `_elements` are accepted and not applied. `_at`,
//! `_list` and `_sort` are refused with `400` / `not-supported` at all three
//! levels instead of being silently ignored (#2374, #2381). Any other key is
//! `400` / `invalid` under `Prefer: handling=strict` and ignored otherwise,
//! because a history Bundle has nowhere to report an ignored parameter.
//!
//! History reads are backed by the `InstanceHistoryProvider`,
//! `TypeHistoryProvider`, and `SystemHistoryProvider` traits, which every
//! first-class storage backend (SQLite, PostgreSQL, MongoDB, S3) implements
//! and which `CompositeStorage` delegates to its primary.

use std::collections::HashSet;
use std::num::IntErrorKind;

use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, SecondsFormat, Utc};
use helios_persistence::core::{
    HistoryEntry, HistoryMethod, HistoryParams, InstanceHistoryProvider, ResourceStorage,
    SearchProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use helios_persistence::types::{PageCursor, PageInfo, PaginationMode};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::extractors::query_pairs::parse_query_pairs;
use crate::middleware::content_type::negotiate_format;
use crate::middleware::prefer::PreferHeader;
use crate::patient_compartment::PatientCompartment;
use crate::responses::bundle::BundleLink;
use crate::responses::format_resource_response;
use crate::state::AppState;

/// The supported query parameters of a history read, as classified by
/// [`parse_history_query`].
///
/// Built from the raw query string, not by serde: a derived `Deserialize`
/// drops every key it does not name, which is how `_at`, `_list`, `_sort` and
/// typos such as `_snice` used to be ignored without a word (#2374, #2381).
#[derive(Debug, Default)]
pub(crate) struct HistoryQuery {
    /// The page size (`_count`).
    pub count: Option<usize>,

    /// Only include versions created at or after this instant (`_since`).
    pub since: Option<DateTime<Utc>>,

    /// Response format override (`_format`: `json` / `xml` / full media types).
    pub format: Option<String>,

    /// The page position (`_cursor`), decoded. Opaque to clients: it is only
    /// ever a `next` link's value.
    pub cursor: Option<PageCursor>,
}

/// The `issue[0].details.text` of a refused `_at` (#2374).
const AT_NOT_SUPPORTED: &str = "The _at history parameter is not supported: HFS cannot select the \
     resource versions that were current at a point in time. Use _since to limit the history \
     to the versions created at or after an instant.";

/// The `issue[0].details.text` of a refused `_list` (#2381).
const LIST_NOT_SUPPORTED: &str = "The _list history parameter is not supported: HFS cannot limit a \
     history to the resources a List references. Read the List and request the history of each \
     resource it references instead.";

/// The `issue[0].details.text` of a refused `_sort` (#2381).
const SORT_NOT_SUPPORTED: &str = "The _sort history parameter is not supported: HFS cannot reorder a \
     history. Omit _sort to receive the versions in the server's default order.";

// History parameters FHIR defines that HFS does not implement, with the
// message each is refused with. They are refused rather than ignored (#2374,
// #2381):
// - `_at` needs, per resource, the latest version before the period starts, and
//   no backend can select that. `HistoryParams::before` is not that set and is
//   deliberately not used. `_list` and `_sort` have no backend support either.
// - They are refused whatever the value (even empty) and whatever
//   `Prefer: handling` says: a history Bundle has nowhere to carry search's
//   ignored-parameter warning (no `entry.search` outside a searchset, no
//   `Bundle.issues` in R4/R4B). It does have a self link (#2382), which leaves
//   a refused parameter out, but a client is not expected to diff its request
//   against the link, so the refusal stance stays. An absent `Prefer` is
//   lenient.
// - They are checked first, so all three handlers refuse before any
//   compartment check or storage read, and one of them on a missing instance
//   is this 400, not a 404.
const REFUSED_HISTORY_PARAMS: &[(&str, &str)] = &[
    ("_at", AT_NOT_SUPPORTED),
    ("_list", LIST_NOT_SUPPORTED),
    ("_sort", SORT_NOT_SUPPORTED),
];

/// The history parameters HFS implements.
const SUPPORTED_HISTORY_PARAMS: &[&str] = &["_count", "_since", "_format", "_cursor"];

// FHIR general parameters. The server honours them on read and search (or
// accepts them as a no-op); history accepts them and does not apply them, so a
// client sees full entries with no SUBSETTED tag. They are never refused: a
// parameter the server handles elsewhere is not an error here.
//
// Search-only parameters (`_id`, `_total`, `_offset`, `_include`, ...) are
// deliberately not known here: history does not apply them, and accepting them
// would answer an unfiltered history with a 200. So under strict they are
// unknown (400), and otherwise they are ignored.
const ACCEPTED_UNAPPLIED_HISTORY_PARAMS: &[&str] = &["_pretty", "_summary", "_elements"];

/// Returns the value of `key` when it appears exactly once, `None` when it is
/// absent, and a `400` when the request repeats it.
///
/// A repeated key is refused, not resolved to its last value: two `_since`
/// instants are ambiguous, and picking one silently would repeat the
/// silent-ignore pattern this module exists to avoid. The serde extractor this
/// replaced already answered a repeated key with a `400`.
fn single_value<'a>(pairs: &'a [(String, String)], key: &str) -> RestResult<Option<&'a str>> {
    let mut values = pairs
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str());
    let first = values.next();
    let repeats = values.count();
    if repeats > 0 {
        return Err(RestError::BadRequest {
            message: format!(
                "The {key} history parameter may appear only once, but the request sends it {} times",
                repeats + 1
            ),
        });
    }
    Ok(first)
}

/// The parameters a history read accepts (supported, then accepted-and-unapplied)
/// as a sentence fragment: `_count, _since, _format, _cursor, _pretty, _summary
/// and _elements`.
///
/// Built from [`SUPPORTED_HISTORY_PARAMS`] and
/// [`ACCEPTED_UNAPPLIED_HISTORY_PARAMS`] so the strict-rejection message cannot
/// drift from what is actually accepted.
fn accepted_history_params() -> String {
    let all: Vec<&str> = SUPPORTED_HISTORY_PARAMS
        .iter()
        .chain(ACCEPTED_UNAPPLIED_HISTORY_PARAMS)
        .copied()
        .collect();
    match all.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_string(),
        None => String::new(),
    }
}

/// Parses a `_count` value.
///
/// A value of ASCII digits too large for `usize` is still a non-negative
/// integer, so it saturates to `usize::MAX` and `build_history_params` then
/// clamps it to the configured maximum page size: FHIR lets a server return
/// fewer than `_count`. Anything else that is not a `usize` (`abc`, `-1`,
/// `1.5`, empty, ...) is a `400`.
fn parse_count(value: &str) -> RestResult<usize> {
    match value.parse::<usize>() {
        Ok(count) => Ok(count),
        Err(e)
            if *e.kind() == IntErrorKind::PosOverflow
                && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Ok(usize::MAX)
        }
        Err(_) => Err(RestError::BadRequest {
            message: format!("Invalid _count value (expected a non-negative integer): {value}"),
        }),
    }
}

/// Classifies the query pairs of a history read and parses the supported ones.
///
/// `strict` is whether the request sent `Prefer: handling=strict`. The checks
/// run in this order and the first failure wins:
///
/// 1. `_at`, `_list` and `_sort` ([`REFUSED_HISTORY_PARAMS`]) give
///    [`RestError::NotSupported`], in query order, under any `strict` and
///    whatever the value (even empty or bare).
/// 2. Keys that are neither supported, refused nor accepted-and-unapplied
///    (`_pretty`, `_summary`, `_elements`) are unknown. Under `strict` they
///    give [`RestError::InvalidParameter`] naming them (de-duplicated, in
///    first-seen order). Otherwise they are ignored and logged by name.
/// 3. A repeated `_count`, `_since`, `_format` or `_cursor` gives
///    [`RestError::BadRequest`], under any `strict`.
/// 4. `_count` must be a non-negative integer and `_since` an RFC 3339
///    instant, else [`RestError::BadRequest`]. A `_count` too large for `usize`
///    saturates to `usize::MAX` (see [`parse_count`]). `_cursor` must decode as
///    a server-issued cursor ([`PageCursor::decode`]), else
///    [`RestError::BadRequest`] with the `SearchError::InvalidCursor` message
///    search renders (`invalid pagination cursor: <value>`). `_format` is not
///    validated here.
///
/// Empty values are not dropped the way search drops them: `_at=` is still
/// refused and `_since=` / `_count=` are still `400`.
pub(crate) fn parse_history_query(
    pairs: &[(String, String)],
    strict: bool,
) -> RestResult<HistoryQuery> {
    // 1. Recognised but refused.
    for (key, _) in pairs {
        if let Some((_, message)) = REFUSED_HISTORY_PARAMS
            .iter()
            .find(|(refused, _)| *refused == key.as_str())
        {
            return Err(RestError::NotSupported {
                feature: (*message).to_string(),
            });
        }
    }

    // 2. Unknown keys.
    // `unknown` keeps first-seen order; `seen` makes the de-duplication O(1).
    let mut unknown: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (key, _) in pairs {
        let key = key.as_str();
        if !SUPPORTED_HISTORY_PARAMS.contains(&key)
            && !ACCEPTED_UNAPPLIED_HISTORY_PARAMS.contains(&key)
            && seen.insert(key)
        {
            unknown.push(key);
        }
    }
    if !unknown.is_empty() {
        let names = unknown.join(", ");
        if strict {
            return Err(RestError::InvalidParameter {
                param: names.clone(),
                message: format!(
                    "unknown history parameter(s) rejected under Prefer: handling=strict: {names}; \
                     history reads accept {}",
                    accepted_history_params()
                ),
            });
        }
        // Key names only: the values are client input.
        debug!(
            params = %names,
            "Ignoring unknown history parameter(s) under lenient handling"
        );
    }

    // 3. Repeated supported keys.
    let count = single_value(pairs, "_count")?;
    let since = single_value(pairs, "_since")?;
    let format = single_value(pairs, "_format")?;
    let cursor = single_value(pairs, "_cursor")?;

    // 4. Values.
    let count = count.map(parse_count).transpose()?;
    let since = since
        .map(|v| {
            DateTime::parse_from_rfc3339(v)
                .map(|instant| instant.with_timezone(&Utc))
                .map_err(|_| RestError::BadRequest {
                    message: format!("Invalid _since value (expected an RFC3339 instant): {v}"),
                })
        })
        .transpose()?;
    let cursor = cursor
        .map(|v| PageCursor::decode(v).map_err(RestError::from))
        .transpose()?;

    Ok(HistoryQuery {
        count,
        since,
        format: format.map(str::to_string),
        cursor,
    })
}

/// Reads the history query from the raw query string and the `Prefer` header.
///
/// Runs first in every history handler, before any compartment check or
/// storage read.
fn history_query_from_request(
    raw_query: Option<&str>,
    headers: &HeaderMap,
) -> RestResult<HistoryQuery> {
    let pairs = parse_query_pairs(raw_query);
    let strict = PreferHeader::from_headers(headers).is_strict();
    parse_history_query(&pairs, strict)
}

/// Builds [`HistoryParams`] from the parsed query, applying the configured
/// page-size limits, the optional `_since` instant and the optional page
/// position (`_cursor`).
///
/// The cursor is set on `pagination.mode` directly. `Pagination::with_cursor`
/// is not used: it silently falls back to page 1 on a cursor it cannot decode,
/// and the cursor was already validated by [`parse_history_query`].
fn build_history_params<S>(params: &HistoryQuery, state: &AppState<S>) -> HistoryParams
where
    S: ResourceStorage + Send + Sync,
{
    let count = params
        .count
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size()) as u32;

    let mut history_params = HistoryParams::new().count(count).include_deleted(true);

    if let Some(since) = params.since {
        history_params = history_params.since(since);
    }

    if let Some(cursor) = &params.cursor {
        history_params.pagination.mode = PaginationMode::Cursor(Some(cursor.clone()));
    }

    history_params
}

/// The query string of a history `self` / `next` link.
///
/// It carries only what the read applied, in a fixed order: `_count` (the
/// effective page size, after the default and the maximum), `_since` and
/// `_format` when the request gave them, and `_cursor` when `cursor` is `Some`.
/// `_pretty`, `_summary`, `_elements` and unknown keys are never carried. Every
/// value is form-urlencoded, so `+` in `application/fhir+json` becomes `%2B`.
fn history_link_query(count: u32, query: &HistoryQuery, cursor: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("_count", &count.to_string());
    if let Some(since) = query.since {
        serializer.append_pair(
            "_since",
            &since.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        );
    }
    if let Some(format) = &query.format {
        serializer.append_pair("_format", format);
    }
    if let Some(cursor) = cursor {
        serializer.append_pair("_cursor", cursor);
    }
    serializer.finish()
}

/// The cursor a history `next` link should carry, or `None` when the page has
/// no usable next page.
///
/// `next` is offered only when `count > 0`, storage reports another page
/// (`has_next`) with a `next_cursor`, and that cursor differs from the
/// request's own re-encoded cursor `own`. A `_count=0` page returns no entries,
/// so a backend that still answers `has_next` hands back a cursor at the same
/// position, and a client following it would read the same empty page forever.
/// Search guards the same case in `bundle_shell`.
fn next_page_cursor<'a>(count: u32, own: Option<&str>, page_info: &'a PageInfo) -> Option<&'a str> {
    if count == 0 || !page_info.has_next {
        return None;
    }
    page_info
        .next_cursor
        .as_deref()
        .filter(|next| Some(*next) != own)
}

/// Builds the `link` array of a history Bundle: `self` always, and `next` only
/// when storage reports another page the client can advance to
/// ([`next_page_cursor`]).
///
/// Built the way search builds its links: the public base and tenant handling
/// are the ones `fullUrl` uses (`public_url_for_request_with_query`). `self`
/// reproduces the current page, so it carries the request's own cursor
/// re-encoded. `next` carries the backend's cursor as given. A cursor is never
/// decoded or altered on the way out.
fn history_links<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    segments: &[&str],
    query: &HistoryQuery,
    count: u32,
    page_info: &PageInfo,
) -> Vec<Value>
where
    S: ResourceStorage,
{
    let url = |cursor: Option<&str>| {
        state.public_url_for_request_with_query(
            tenant,
            segments.iter().copied(),
            Some(&history_link_query(count, query, cursor)),
        )
    };

    let own_cursor = query.cursor.as_ref().map(PageCursor::encode);
    let mut links = vec![BundleLink::self_link(url(own_cursor.as_deref())).to_json()];
    if let Some(next) = next_page_cursor(count, own_cursor.as_deref(), page_info) {
        links.push(BundleLink::next(url(Some(next))).to_json());
    }
    links
}

/// Converts a single [`HistoryEntry`] into a FHIR history Bundle entry.
fn history_entry_to_json<S>(
    entry: &HistoryEntry,
    state: &AppState<S>,
    tenant: &TenantExtractor,
) -> Value
where
    S: ResourceStorage,
{
    let resource = &entry.resource;
    let resource_type = resource.resource_type();
    let id = resource.id();
    let version_id = resource.version_id();

    let (method, request_url, status) = match entry.method {
        HistoryMethod::Post => ("POST", resource_type.to_string(), "201"),
        HistoryMethod::Put => ("PUT", format!("{resource_type}/{id}"), "200"),
        HistoryMethod::Patch => ("PATCH", format!("{resource_type}/{id}"), "200"),
        HistoryMethod::Delete => ("DELETE", format!("{resource_type}/{id}"), "204"),
    };

    let mut bundle_entry = json!({
        "fullUrl": state.public_url_for_request(tenant, [resource_type, id]),
        "request": {
            "method": method,
            "url": request_url,
        },
        "response": {
            "status": status,
            "etag": format!("W/\"{version_id}\""),
            "lastModified": entry.timestamp.to_rfc3339(),
        },
    });

    // Deleted versions carry no resource body.
    if entry.method != HistoryMethod::Delete {
        bundle_entry["resource"] = resource.content_with_meta();
    }

    bundle_entry
}

/// Builds a FHIR `history` Bundle from a page of history entries.
///
/// `links` is the Bundle's `link` array from [`history_links`]. It is emitted
/// between `total` and `entry`, the key order search Bundles use.
fn build_history_bundle<S>(
    entries: &[HistoryEntry],
    state: &AppState<S>,
    tenant: &TenantExtractor,
    total: u64,
    links: Vec<Value>,
) -> Value
where
    S: ResourceStorage,
{
    let bundle_entries: Vec<Value> = entries
        .iter()
        .map(|entry| history_entry_to_json(entry, state, tenant))
        .collect();

    json!({
        "resourceType": "Bundle",
        "type": "history",
        "total": total,
        "link": links,
        "entry": bundle_entries,
    })
}

/// Serializes a history Bundle in the negotiated response format.
fn respond_with_bundle(
    bundle: &Value,
    req_headers: &HeaderMap,
    format_param: Option<&str>,
) -> Response {
    let negotiated = negotiate_format(req_headers, format_param);
    format_resource_response(StatusCode::OK, HeaderMap::new(), bundle, negotiated.format)
        .unwrap_or_else(|refusal| refusal.into_response())
}

/// Handler for instance history.
///
/// Returns the version history for a specific resource instance.
///
/// # HTTP Request
///
/// `GET [base]/[type]/[id]/_history`
///
/// # Query Parameters
///
/// - `_count` - Page size
/// - `_since` - Only versions since this time
/// - `_format` - Response format override
/// - `_cursor` - Page position, taken from a `next` link (opaque)
/// - `_pretty`, `_summary`, `_elements` - Accepted and not applied
/// - `_at`, `_list`, `_sort` - Refused with `400` / `not-supported` (#2374, #2381)
/// - Any other key - `400` / `invalid` under `Prefer: handling=strict`,
///   ignored otherwise
///
/// # Response
///
/// - `200 OK` - Returns a Bundle of type "history". The Bundle carries a
///   `self` link and, when there is another page, a `next` link (#2382).
/// - `400 Bad Request` - `_at`/`_list`/`_sort` sent; `_count` or `_since`
///   malformed or repeated; `_format` repeated; `_cursor` undecodable or
///   repeated; an unknown parameter under strict
/// - `404 Not Found` - The resource has no history (never existed)
pub async fn history_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + SearchProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing instance history request"
    );

    let params = history_query_from_request(raw_query.as_deref(), &req_headers)?;
    let history_params = build_history_params(&params, &state);

    // A `patient/` scope sees the history of the launch patient's resources
    // only (#1618): the resource must currently be in the compartment — a
    // deleted or absent one has no payload to prove it with — and versions from
    // before it joined are left out of the page below.
    if compartment.is_confined() {
        let current = match state
            .storage()
            .read(tenant.context(), &resource_type, &id)
            .await
        {
            Ok(current) => current,
            Err(helios_persistence::error::StorageError::Resource(
                helios_persistence::error::ResourceError::Gone { .. },
            )) => None,
            Err(e) => return Err(e.into()),
        };
        let Some(current) = current else {
            return Err(compartment.outside_compartment(format!(
                "history of {resource_type}/{id} is not available under a patient/ scope: the resource has no current version in the launch patient's compartment"
            )));
        };
        compartment.assert_member(
            || state.storage().search_param_registry(tenant.context()),
            current.fhir_version(),
            current.content(),
            "history",
        )?;
    }

    let page = state
        .storage()
        .history_instance(tenant.context(), &resource_type, &id, &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "Instance history failed");
            RestError::from(e)
        })?;

    // An existing resource always has at least its current version in history,
    // so an empty *unfiltered* first page means the resource never existed. When
    // a `_since` filter is in play, an empty page just means nothing matched, so
    // we return an empty history Bundle rather than a 404. An empty page after a
    // `_cursor` means the walk ran past the end (for example because versions
    // were deleted between pages), not that the resource never existed, so that
    // is an empty Bundle too.
    //
    // A `_count=0` page is empty whether or not the resource exists, so it
    // proves nothing. Only then is storage asked how many versions the resource
    // has: none means it never existed (404), anything else is the empty 200
    // Bundle. Every other request keeps the rule above and makes no extra call.
    if params.since.is_none() && params.cursor.is_none() {
        let never_existed = if history_params.pagination.count == 0 {
            state
                .storage()
                .history_instance_count(tenant.context(), &resource_type, &id)
                .await
                .map_err(|e| {
                    warn!(error = %e, "Instance history count failed");
                    RestError::from(e)
                })?
                == 0
        } else {
            page.items.is_empty()
        };
        if never_existed {
            return Err(RestError::NotFound { resource_type, id });
        }
    }

    // Under a confined grant, keep the versions that are in the compartment and
    // the delete markers (no payload to judge); drop the rest (#1618).
    let items: Vec<HistoryEntry> = if compartment.is_confined() {
        let registry = state.storage().search_param_registry(tenant.context());
        page.items
            .iter()
            .filter(|entry| {
                entry.resource.is_deleted()
                    || compartment.is_member(
                        || registry.clone(),
                        entry.resource.fhir_version(),
                        entry.resource.content(),
                    )
            })
            .cloned()
            .collect()
    } else {
        page.items
    };
    let total = page.page_info.total.unwrap_or(items.len() as u64);
    let links = history_links(
        &state,
        &tenant,
        &[resource_type.as_str(), id.as_str(), "_history"],
        &params,
        history_params.pagination.count,
        &page.page_info,
    );
    let bundle = build_history_bundle(&items, &state, &tenant, total, links);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for type history.
///
/// Returns the version history for all resources of a type.
///
/// # HTTP Request
///
/// `GET [base]/[type]/_history`
///
/// Accepts `_count`, `_since`, `_format` and `_cursor`; `_at`, `_list` and
/// `_sort` are refused with `400`, and an unknown parameter is `400` under
/// `Prefer: handling=strict` and ignored otherwise (#2374, #2381). The Bundle
/// carries a `self` link and, when there is another page, a `next` link
/// (#2382).
pub async fn history_type_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + TypeHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        "Processing type history request"
    );

    let params = history_query_from_request(raw_query.as_deref(), &req_headers)?;
    let history_params = build_history_params(&params, &state);

    let page = state
        .storage()
        .history_type(tenant.context(), &resource_type, &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "Type history failed");
            RestError::from(e)
        })?;

    let total = page.page_info.total.unwrap_or(page.items.len() as u64);
    let links = history_links(
        &state,
        &tenant,
        &[resource_type.as_str(), "_history"],
        &params,
        history_params.pagination.count,
        &page.page_info,
    );
    let bundle = build_history_bundle(&page.items, &state, &tenant, total, links);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for system history.
///
/// Returns the version history for all resources.
///
/// # HTTP Request
///
/// `GET [base]/_history`
///
/// Accepts `_count`, `_since`, `_format` and `_cursor`; `_at`, `_list` and
/// `_sort` are refused with `400`, and an unknown parameter is `400` under
/// `Prefer: handling=strict` and ignored otherwise (#2374, #2381). The Bundle
/// carries a `self` link and, when there is another page, a `next` link
/// (#2382).
pub async fn history_system_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + SystemHistoryProvider + Send + Sync,
{
    debug!(
        tenant = %tenant.tenant_id(),
        "Processing system history request"
    );

    let params = history_query_from_request(raw_query.as_deref(), &req_headers)?;
    let history_params = build_history_params(&params, &state);

    let page = state
        .storage()
        .history_system(tenant.context(), &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "System history failed");
            RestError::from(e)
        })?;

    let total = page.page_info.total.unwrap_or(page.items.len() as u64);
    let links = history_links(
        &state,
        &tenant,
        &["_history"],
        &params,
        history_params.pagination.count,
        &page.page_info,
    );
    let bundle = build_history_bundle(&page.items, &state, &tenant, total, links);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for deleting instance history.
///
/// Deletes all historical versions of a resource, preserving only the current version.
/// This is a FHIR v6.0.0 Trial Use feature.
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]/_history`
///
/// # Response
///
/// - `200 OK` with OperationOutcome indicating number of versions deleted
/// - `404 Not Found` if the resource doesn't exist
/// - `501 Not Implemented` if the backend doesn't support this operation
pub async fn delete_instance_history_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing delete instance history request"
    );

    let deleted_count = state
        .storage()
        .delete_instance_history(tenant.context(), &resource_type, &id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Delete instance history failed");
            RestError::from(e)
        })?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        deleted_count = deleted_count,
        "Instance history deleted successfully"
    );

    // Return OperationOutcome with success message
    let outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "information",
            "code": "informational",
            "diagnostics": format!(
                "Deleted {} historical version(s) of {}/{}. Current version preserved.",
                deleted_count, resource_type, id
            )
        }]
    });

    Ok((StatusCode::OK, Json(outcome)).into_response())
}

/// Handler for deleting a specific version from history.
///
/// Deletes a specific historical version of a resource. Cannot delete the current version.
/// This is a FHIR v6.0.0 Trial Use feature.
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]/_history/[vid]`
///
/// # Response
///
/// - `204 No Content` on successful deletion
/// - `404 Not Found` if the resource or version doesn't exist
/// - `400 Bad Request` if attempting to delete the current version
/// - `501 Not Implemented` if the backend doesn't support this operation
pub async fn delete_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        version_id = %version_id,
        tenant = %tenant.tenant_id(),
        "Processing delete version request"
    );

    state
        .storage()
        .delete_version(tenant.context(), &resource_type, &id, &version_id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Delete version failed");
            RestError::from(e)
        })?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        version_id = %version_id,
        "Version deleted successfully"
    );

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_persistence::types::CursorValue;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The `feature` of a `NotSupported`, or a panic naming what came back.
    fn not_supported(result: RestResult<HistoryQuery>) -> String {
        match result {
            Err(RestError::NotSupported { feature }) => feature,
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    /// The `(param, message)` of an `InvalidParameter`.
    fn invalid_parameter(result: RestResult<HistoryQuery>) -> (String, String) {
        match result {
            Err(RestError::InvalidParameter { param, message }) => (param, message),
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }

    /// The `message` of a `BadRequest`.
    fn bad_request(result: RestResult<HistoryQuery>) -> String {
        match result {
            Err(RestError::BadRequest { message }) => message,
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn empty_query_is_defaults() {
        for strict in [false, true] {
            let query = parse_history_query(&[], strict).unwrap();
            assert!(query.count.is_none(), "strict={strict}");
            assert!(query.since.is_none(), "strict={strict}");
            assert!(query.format.is_none(), "strict={strict}");
            assert!(query.cursor.is_none(), "strict={strict}");
        }
    }

    /// A well-formed encoded cursor, as a backend would hand out.
    fn valid_cursor() -> String {
        PageCursor::new(vec![CursorValue::String("3".into())], "p").encode()
    }

    #[test]
    fn parses_supported_params() {
        let cursor = valid_cursor();
        let query = parse_history_query(
            &pairs(&[
                ("_count", "5"),
                ("_since", "2020-01-01T00:00:00Z"),
                ("_format", "xml"),
                ("_cursor", cursor.as_str()),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(query.cursor.unwrap().resource_id(), "p");
        assert_eq!(query.count, Some(5));
        assert_eq!(
            query.since,
            Some(
                DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
        assert_eq!(query.format.as_deref(), Some("xml"));
    }

    #[test]
    fn refuses_at_list_sort_under_any_handling() {
        for strict in [false, true] {
            for key in ["_at", "_list", "_sort"] {
                for value in ["", "x"] {
                    let feature =
                        not_supported(parse_history_query(&pairs(&[(key, value)]), strict));
                    assert!(
                        feature.contains(key),
                        "strict={strict} {key}={value:?}: message should name {key}: {feature}"
                    );
                    if key == "_at" {
                        assert_eq!(feature, AT_NOT_SUPPORTED);
                    }
                }
            }
        }
    }

    #[test]
    fn refused_param_wins_over_every_other_error() {
        for (query, strict, key) in [
            (vec![("_since", "bad"), ("_list", "x")], false, "_list"),
            (vec![("_count", "abc"), ("_sort", "x")], false, "_sort"),
            (
                vec![("_since", "a"), ("_since", "b"), ("_at", "x")],
                false,
                "_at",
            ),
            (vec![("_snice", "x"), ("_list", "y")], true, "_list"),
        ] {
            let feature = not_supported(parse_history_query(&pairs(&query), strict));
            assert!(
                feature.contains(key),
                "{query:?} strict={strict}: message should name {key}: {feature}"
            );
        }
    }

    #[test]
    fn first_refused_param_in_query_order_is_reported() {
        let feature = not_supported(parse_history_query(
            &pairs(&[("_sort", "x"), ("_list", "y"), ("_at", "z")]),
            false,
        ));
        assert_eq!(feature, SORT_NOT_SUPPORTED);
    }

    #[test]
    fn unknown_param_is_invalid_parameter_under_strict() {
        let (param, message) = invalid_parameter(parse_history_query(
            &pairs(&[("_snice", "x"), ("foo", "y"), ("_snice", "z")]),
            true,
        ));
        assert_eq!(param, "_snice, foo");
        assert_eq!(
            message,
            "unknown history parameter(s) rejected under Prefer: handling=strict: _snice, foo; history reads accept _count, _since, _format, _cursor, _pretty, _summary and _elements"
        );

        // Search-only parameters are unknown on history.
        for (key, value) in [("_id", "1"), ("_offset", "1"), ("_total", "accurate")] {
            let (param, _) = invalid_parameter(parse_history_query(&pairs(&[(key, value)]), true));
            assert_eq!(param, key);
        }
    }

    #[test]
    fn strict_rejection_message_lists_every_accepted_param() {
        let (_, message) = invalid_parameter(parse_history_query(&pairs(&[("_snice", "x")]), true));
        for accepted in SUPPORTED_HISTORY_PARAMS
            .iter()
            .chain(ACCEPTED_UNAPPLIED_HISTORY_PARAMS)
        {
            assert!(
                message.contains(accepted),
                "message should list {accepted}: {message}"
            );
        }
        assert_eq!(
            accepted_history_params(),
            "_count, _since, _format, _cursor, _pretty, _summary and _elements"
        );
    }

    #[test]
    fn unknown_param_is_ignored_without_strict() {
        let query =
            parse_history_query(&pairs(&[("_snice", "x"), ("_count", "2")]), false).unwrap();
        assert_eq!(query.count, Some(2));
        assert!(query.since.is_none());
    }

    #[test]
    fn general_params_are_accepted_under_strict() {
        let query = parse_history_query(
            &pairs(&[
                ("_pretty", "true"),
                ("_summary", "true"),
                ("_elements", "id"),
            ]),
            true,
        )
        .unwrap();
        assert!(query.count.is_none());
        assert!(query.since.is_none());
        assert!(query.format.is_none());
        assert!(query.cursor.is_none());
    }

    #[test]
    fn unknown_beats_malformed_value_under_strict() {
        let (param, _) = invalid_parameter(parse_history_query(
            &pairs(&[("_count", "abc"), ("_snice", "x")]),
            true,
        ));
        assert_eq!(param, "_snice");
    }

    #[test]
    fn malformed_count_is_bad_request() {
        for value in ["abc", "-1", "1.5", ""] {
            let message = bad_request(parse_history_query(&pairs(&[("_count", value)]), false));
            assert!(
                message.contains("_count"),
                "_count={value:?}: message was: {message}"
            );
        }
        let query = parse_history_query(&pairs(&[("_count", "0")]), false).unwrap();
        assert_eq!(query.count, Some(0));
    }

    #[test]
    fn overflowing_count_saturates() {
        for value in [
            "99999999999999999999",
            "18446744073709551616",
            "0099999999999999999999999999",
            "340282366920938463463374607431768211456",
        ] {
            for strict in [false, true] {
                let query = parse_history_query(&pairs(&[("_count", value)]), strict).unwrap();
                assert_eq!(
                    query.count,
                    Some(usize::MAX),
                    "_count={value} strict={strict}"
                );
            }
        }
        // The largest value that fits is not saturated, only parsed.
        let max = usize::MAX.to_string();
        let query = parse_history_query(&pairs(&[("_count", max.as_str())]), false).unwrap();
        assert_eq!(query.count, Some(usize::MAX));
    }

    #[test]
    fn non_integer_count_stays_bad_request_with_its_message() {
        // " 5" is what `_count=+5` decodes to on the wire (`+` is a space in a
        // query string). Overflowing values that are not plain digits are not
        // saturated either.
        for value in [
            "abc",
            "-1",
            "1.5",
            "",
            " 5",
            "5 ",
            "1e3",
            "0x10",
            "-99999999999999999999",
            "+99999999999999999999",
            "99999999999999999999.5",
            "99999999999999999999x",
        ] {
            let message = bad_request(parse_history_query(&pairs(&[("_count", value)]), false));
            assert_eq!(
                message,
                format!("Invalid _count value (expected a non-negative integer): {value}")
            );
        }
    }

    #[test]
    fn repeated_supported_param_is_bad_request() {
        for (key, first, second) in [
            ("_count", "1", "2"),
            ("_since", "2000-01-01T00:00:00Z", "2001-01-01T00:00:00Z"),
            ("_format", "json", "json"),
            ("_cursor", "a", "b"),
        ] {
            for strict in [false, true] {
                let message = bad_request(parse_history_query(
                    &pairs(&[(key, first), (key, second)]),
                    strict,
                ));
                assert_eq!(
                    message,
                    format!(
                        "The {key} history parameter may appear only once, but the request sends it 2 times"
                    ),
                    "{key} strict={strict}"
                );
            }
        }
    }

    #[test]
    fn repeated_param_message_counts_total_occurrences() {
        let message = bad_request(parse_history_query(
            &pairs(&[("_count", "1"), ("_count", "2"), ("_count", "3")]),
            false,
        ));
        assert_eq!(
            message,
            "The _count history parameter may appear only once, but the request sends it 3 times"
        );
    }

    #[test]
    fn since_error_message_is_unchanged() {
        let message = bad_request(parse_history_query(
            &pairs(&[("_since", "not-a-date")]),
            false,
        ));
        assert_eq!(
            message,
            "Invalid _since value (expected an RFC3339 instant): not-a-date"
        );
    }

    #[test]
    fn cursor_is_accepted_under_strict() {
        let cursor = valid_cursor();
        for strict in [false, true] {
            let query =
                parse_history_query(&pairs(&[("_cursor", cursor.as_str())]), strict).unwrap();
            assert_eq!(
                query.cursor.expect("cursor should be parsed").resource_id(),
                "p",
                "strict={strict}"
            );
        }
    }

    #[test]
    fn malformed_cursor_is_bad_request() {
        // "" and "abc" decode to nothing JSON; "not-a-cursor!!" is not base64;
        // "e30" is base64 of `{}`, JSON of the wrong shape.
        for value in ["", "not-a-cursor!!", "abc", "e30"] {
            for strict in [false, true] {
                let message =
                    bad_request(parse_history_query(&pairs(&[("_cursor", value)]), strict));
                assert_eq!(
                    message,
                    format!("invalid pagination cursor: {value}"),
                    "_cursor={value:?} strict={strict}"
                );
            }
        }
    }

    #[test]
    fn refused_and_unknown_still_win_over_bad_cursor() {
        let feature = not_supported(parse_history_query(
            &pairs(&[("_at", "x"), ("_cursor", "not-a-cursor!!")]),
            false,
        ));
        assert_eq!(feature, AT_NOT_SUPPORTED);

        let (param, _) = invalid_parameter(parse_history_query(
            &pairs(&[("_snice", "x"), ("_cursor", "not-a-cursor!!")]),
            true,
        ));
        assert_eq!(param, "_snice");
    }

    #[test]
    fn history_link_query_carries_only_applied_params() {
        let query = parse_history_query(
            &pairs(&[
                ("_count", "99"),
                ("_since", "2020-01-01T00:00:00+02:00"),
                ("_format", "application/fhir+json"),
                ("_pretty", "true"),
                ("_summary", "true"),
                ("_elements", "id"),
                ("_snice", "x"),
            ]),
            false,
        )
        .unwrap();
        // The count is the effective one passed in, not the requested 99.
        assert_eq!(
            history_link_query(2, &query, Some("abc-_")),
            "_count=2&_since=2019-12-31T22%3A00%3A00Z&_format=application%2Ffhir%2Bjson&_cursor=abc-_"
        );
        assert_eq!(
            history_link_query(2, &query, None),
            "_count=2&_since=2019-12-31T22%3A00%3A00Z&_format=application%2Ffhir%2Bjson"
        );
        assert_eq!(
            history_link_query(20, &HistoryQuery::default(), None),
            "_count=20"
        );
    }

    /// Page info that reports another page with the cursor `next`.
    fn more_pages(next: &str) -> PageInfo {
        PageInfo {
            has_next: true,
            next_cursor: Some(next.to_string()),
            ..PageInfo::end()
        }
    }

    #[test]
    fn next_page_cursor_is_offered_for_a_page_that_can_advance() {
        let info = more_pages("next");
        assert_eq!(next_page_cursor(2, None, &info), Some("next"));
        assert_eq!(next_page_cursor(2, Some("own"), &info), Some("next"));
    }

    #[test]
    fn next_page_cursor_is_none_for_a_count_zero_page() {
        // S3 answers has_next with a cursor at the same offset; following it
        // would read the same empty page forever.
        let info = more_pages("next");
        assert_eq!(next_page_cursor(0, None, &info), None);
        assert_eq!(next_page_cursor(0, Some("own"), &info), None);
    }

    #[test]
    fn next_page_cursor_is_none_without_a_next_page() {
        let no_next = PageInfo {
            has_next: false,
            next_cursor: Some("next".to_string()),
            ..PageInfo::end()
        };
        assert_eq!(next_page_cursor(2, None, &no_next), None);

        let no_cursor = PageInfo {
            has_next: true,
            next_cursor: None,
            ..PageInfo::end()
        };
        assert_eq!(next_page_cursor(2, None, &no_cursor), None);
    }

    #[test]
    fn next_page_cursor_is_none_when_it_equals_the_requests_own() {
        let info = more_pages("same");
        assert_eq!(next_page_cursor(2, Some("same"), &info), None);
    }
}
