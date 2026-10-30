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
//! History reads accept `_count`, `_since` and `_format`, each at most once.
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

use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use helios_persistence::core::{
    HistoryEntry, HistoryMethod, HistoryParams, InstanceHistoryProvider, ResourceStorage,
    SearchProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::extractors::query_pairs::parse_query_pairs;
use crate::middleware::content_type::negotiate_format;
use crate::middleware::prefer::PreferHeader;
use crate::patient_compartment::PatientCompartment;
use crate::responses::format_resource_response;
use crate::state::AppState;

/// The supported query parameters of a history read, as classified by
/// [`parse_history_query`].
///
/// Built from the raw query string, not by serde: a derived `Deserialize`
/// drops every key it does not name, which is how `_at`, `_list`, `_sort` and
/// typos such as `_snice` used to be ignored without a word (#2374, #2381).
#[derive(Debug, Default)]
pub struct HistoryQuery {
    /// The page size (`_count`).
    pub count: Option<usize>,

    /// Only include versions created at or after this instant (`_since`).
    pub since: Option<DateTime<Utc>>,

    /// Response format override (`_format`: `json` / `xml` / full media types).
    pub format: Option<String>,
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
//   `Bundle.issues` in R4/R4B, no self link), and an absent `Prefer` is
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
const SUPPORTED_HISTORY_PARAMS: &[&str] = &["_count", "_since", "_format"];

// FHIR general parameters. The server honours them on read and search (or
// accepts them as a no-op); history accepts them and does not apply them, so a
// client sees full entries with no SUBSETTED tag. They are never refused: a
// parameter the server handles elsewhere is not an error here.
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
                "The {key} history parameter may appear only once, but the request repeats it ({} times)",
                repeats + 1
            ),
        });
    }
    Ok(first)
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
/// 3. A repeated `_count`, `_since` or `_format` gives
///    [`RestError::BadRequest`], under any `strict`.
/// 4. `_count` must be a non-negative integer and `_since` an RFC 3339
///    instant, else [`RestError::BadRequest`]. `_format` is not validated here.
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
    let mut unknown: Vec<&str> = Vec::new();
    for (key, _) in pairs {
        let key = key.as_str();
        if !SUPPORTED_HISTORY_PARAMS.contains(&key)
            && !ACCEPTED_UNAPPLIED_HISTORY_PARAMS.contains(&key)
            && !unknown.contains(&key)
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
                     history reads accept _count, _since and _format"
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

    // 4. Values.
    let count = count
        .map(|v| {
            v.parse::<usize>().map_err(|_| RestError::BadRequest {
                message: format!("Invalid _count value (expected a non-negative integer): {v}"),
            })
        })
        .transpose()?;
    let since = since
        .map(|v| {
            DateTime::parse_from_rfc3339(v)
                .map(|instant| instant.with_timezone(&Utc))
                .map_err(|_| RestError::BadRequest {
                    message: format!("Invalid _since value (expected an RFC3339 instant): {v}"),
                })
        })
        .transpose()?;

    Ok(HistoryQuery {
        count,
        since,
        format: format.map(str::to_string),
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
/// page-size limits and the optional `_since` instant.
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

    history_params
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
fn build_history_bundle<S>(
    entries: &[HistoryEntry],
    state: &AppState<S>,
    tenant: &TenantExtractor,
    total: u64,
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
/// - `_pretty`, `_summary`, `_elements` - Accepted and not applied
/// - `_at`, `_list`, `_sort` - Refused with `400` / `not-supported` (#2374, #2381)
/// - Any other key - `400` / `invalid` under `Prefer: handling=strict`,
///   ignored otherwise
///
/// # Response
///
/// - `200 OK` - Returns a Bundle of type "history"
/// - `400 Bad Request` - `_at`/`_list`/`_sort` sent; `_count` or `_since`
///   malformed or repeated; `_format` repeated; an unknown parameter under
///   strict
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
    // so an empty *unfiltered* page means the resource never existed. When a
    // `_since` filter is in play, an empty page just means nothing matched, so
    // we return an empty history Bundle rather than a 404.
    if page.items.is_empty() && params.since.is_none() {
        return Err(RestError::NotFound { resource_type, id });
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
    let bundle = build_history_bundle(&items, &state, &tenant, total);

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
/// Accepts `_count`, `_since` and `_format`; `_at`, `_list` and `_sort` are
/// refused with `400`, and an unknown parameter is `400` under
/// `Prefer: handling=strict` and ignored otherwise (#2374, #2381).
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
    let bundle = build_history_bundle(&page.items, &state, &tenant, total);

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
/// Accepts `_count`, `_since` and `_format`; `_at`, `_list` and `_sort` are
/// refused with `400`, and an unknown parameter is `400` under
/// `Prefer: handling=strict` and ignored otherwise (#2374, #2381).
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
    let bundle = build_history_bundle(&page.items, &state, &tenant, total);

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
        }
    }

    #[test]
    fn parses_supported_params() {
        let query = parse_history_query(
            &pairs(&[
                ("_count", "5"),
                ("_since", "2020-01-01T00:00:00Z"),
                ("_format", "xml"),
            ]),
            false,
        )
        .unwrap();
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
        assert!(
            message.contains("_snice, foo") && message.contains("handling=strict"),
            "message was: {message}"
        );

        // Search-only parameters are unknown on history.
        for (key, value) in [("_id", "1"), ("_offset", "1"), ("_total", "accurate")] {
            let (param, _) = invalid_parameter(parse_history_query(&pairs(&[(key, value)]), true));
            assert_eq!(param, key);
        }
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
    fn repeated_supported_param_is_bad_request() {
        for (key, first, second) in [
            ("_count", "1", "2"),
            ("_since", "2000-01-01T00:00:00Z", "2001-01-01T00:00:00Z"),
            ("_format", "json", "json"),
        ] {
            for strict in [false, true] {
                let message = bad_request(parse_history_query(
                    &pairs(&[(key, first), (key, second)]),
                    strict,
                ));
                assert!(
                    message.contains(key),
                    "{key} strict={strict}: message was: {message}"
                );
            }
        }
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
}
