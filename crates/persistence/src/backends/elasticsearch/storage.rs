//! ResourceStorage implementation for Elasticsearch.
//!
//! Provides the minimal CRUD operations needed for the SyncManager to propagate
//! changes from the primary backend. The ES backend is primarily a search secondary,
//! but it must implement ResourceStorage for sync support.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use elasticsearch::params::Refresh;
use elasticsearch::{BulkParts, DeleteByQueryParts, DeleteParts, IndexParts};
use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::{PurgableStorage, ResourceStorage};
use crate::error::{BackendError, ResourceError, StorageError, StorageResult};
use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;
use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage};
use crate::search::{FhirDateValue, StorageResolution};
use crate::tenant::{Operation, TenantContext};
use crate::types::StoredResource;

use super::backend::ElasticsearchBackend;
use super::schema;
use super::search_impl::{
    EsFailureClass, MAX_SEARCH_RETRIES, RETRY_BASE_DELAY_MS, ReadOp, classify_es_failure,
    is_index_not_found, send_read_with_retry,
};

/// Upper bound on operations per `_bulk` request, on top of the configured byte
/// budget ([`ElasticsearchConfig::bulk_max_bytes`](super::backend::ElasticsearchConfig::bulk_max_bytes)).
/// Small resources hit this count first, so a load does not pay one refresh
/// wait per handful of documents; large ones hit the byte budget first (#1125).
const BULK_OPS_PER_REQUEST: usize = 500;

/// Why a resource's documents did not index in a `_bulk` request.
struct BulkFailure {
    message: String,
    /// The cluster never judged the document — the request did not arrive, or
    /// the cluster pushed back (`429`) or failed (`5xx`) — so a rerun may
    /// succeed. A `4xx` rejection of the document itself (for example the
    /// nested-object limit, #1050) is permanent.
    transient: bool,
}

/// Whether a `_bulk` request or item status asks for a retry rather than
/// rejecting the document.
pub(super) fn is_transient_bulk_status(status: u64) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Attempts an operation Elasticsearch answers `429` gets, the first included,
/// before it is reported as a transient failure.
const BULK_MAX_ATTEMPTS: u32 = 5;

/// Wait before the first resend of throttled operations; doubles on each
/// further resend, up to [`BULK_BACKOFF_MAX`].
const BULK_BACKOFF_BASE: Duration = Duration::from_millis(100);

/// Upper bound on one back-off wait.
const BULK_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// The wait before the `resend`-th resend (`resend >= 1`) of throttled
/// operations.
fn backoff_delay(resend: u32) -> Duration {
    let doublings = resend.saturating_sub(1).min(16);
    BULK_BACKOFF_BASE
        .saturating_mul(1u32 << doublings)
        .min(BULK_BACKOFF_MAX)
}

/// Groups consecutive operations, given their sizes in bytes, into `_bulk`
/// requests of at most `max_ops` operations and `max_bytes` bytes.
///
/// An operation larger than `max_bytes` on its own gets a request of its own —
/// it is sent, not dropped — so the cap bounds every request that *can* be
/// bounded. Without a byte cap a page of large resources was one request: 500
/// Synthea `Provenance` documents of ~108 KB each is a ~54 MB body, which
/// outlived the client timeout and failed all 500 (#1125).
fn chunk_ranges(sizes: &[usize], max_ops: usize, max_bytes: usize) -> Vec<Range<usize>> {
    let max_ops = max_ops.max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut bytes = 0usize;
    for (i, &size) in sizes.iter().enumerate() {
        if i > start && (i - start == max_ops || bytes.saturating_add(size) > max_bytes) {
            ranges.push(start..i);
            start = i;
            bytes = 0;
        }
        bytes = bytes.saturating_add(size);
    }
    if start < sizes.len() {
        ranges.push(start..sizes.len());
    }
    ranges
}

/// Renders a `_bulk` item's `error` object as `type: reason`, following the
/// `caused_by` chain.
///
/// The chain matters: Elasticsearch reports a document over the nested-object
/// limit as `document_parsing_exception: failed to parse`, and only its
/// `caused_by` says which limit (#1050). Anything that is not an error object
/// is rendered verbatim.
fn describe_item_error(error: &Value) -> String {
    let mut parts = Vec::new();
    let mut current = Some(error);
    while let Some(cause) = current {
        let kind = cause.get("type").and_then(Value::as_str);
        let reason = cause.get("reason").and_then(Value::as_str);
        match (kind, reason) {
            (Some(kind), Some(reason)) => parts.push(format!("{kind}: {reason}")),
            (Some(kind), None) => parts.push(kind.to_string()),
            (None, Some(reason)) => parts.push(reason.to_string()),
            (None, None) => break,
        }
        // Bounded: a malformed response must not make this loop forever-ish.
        if parts.len() == 8 {
            break;
        }
        current = cause.get("caused_by");
    }
    if parts.is_empty() {
        error.to_string()
    } else {
        parts.join("; caused by ")
    }
}

/// The detail of `error`, including the message `BackendError::Unavailable`'s
/// display leaves out.
fn error_detail(error: &StorageError) -> String {
    match error {
        StorageError::Backend(BackendError::Unavailable { message, .. }) => message.clone(),
        other => other.to_string(),
    }
}

/// Whether `error` is an outage (the cluster could not be reached or asked to
/// be retried) rather than a rejection.
fn is_unavailable(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Backend(BackendError::Unavailable { .. })
    )
}

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "elasticsearch".to_string(),
        message,
        source: None,
    })
}

/// The cluster could not be reached (connection refused, DNS, TLS, timeout).
///
/// Distinct from [`internal_error`] because the caller must never confuse it
/// with "the resource is not there": we never got an answer at all.
fn unavailable_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Unavailable {
        backend_name: "elasticsearch".to_string(),
        message,
    })
}

/// Content extracted from a resource for full-text search.
struct SearchableContent {
    narrative: String,
    full_content: String,
}

/// Extracts searchable text content from a FHIR resource.
fn extract_searchable_content(resource: &Value) -> SearchableContent {
    SearchableContent {
        narrative: extract_narrative(resource),
        full_content: extract_all_strings(resource),
    }
}

/// Extracts narrative text from resource.text.div, stripping HTML tags.
fn extract_narrative(resource: &Value) -> String {
    resource
        .get("text")
        .and_then(|t| t.get("div"))
        .and_then(|d| d.as_str())
        .map(strip_html_tags)
        .unwrap_or_default()
}

/// Strips HTML tags from a string, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Normalize whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts all string values from a JSON value recursively.
fn extract_all_strings(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_strings(value, &mut parts);
    parts.join(" ")
}

fn collect_strings(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                // Skip metadata fields and large binary data
                if key == "div" || key == "data" {
                    continue;
                }
                collect_strings(val, parts);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                collect_strings(val, parts);
            }
        }
        _ => {}
    }
}

/// Builds an ES document from a FHIR resource and its extracted search values.
/// Appends a value to a JSON array field on `obj`, creating the array if needed.
fn push_array_field(obj: &mut Value, key: &str, val: Value) {
    match obj.get_mut(key) {
        Some(Value::Array(arr)) => arr.push(val),
        _ => obj[key] = json!([val]),
    }
}

/// The resource and parameter a value was extracted for, for log messages.
#[derive(Clone, Copy)]
struct ValueOrigin<'a> {
    resource_type: &'a str,
    resource_id: &'a str,
    param: &'a str,
}

/// The value indexed into a `date` field for an extracted FHIR date, or
/// `None` when the value is not a date and the field must be left out.
///
/// The extracted string used to be sent as written. The `date` mapping is
/// strict, and Elasticsearch rejects the *whole document* for one value it
/// cannot parse (`mapper_parsing_exception`): a resource with a single bad
/// date, in any element, could not be found by any search (#1314). That
/// included values that are valid FHIR — a leap second (`…T23:59:60Z`), more
/// than nine fraction digits — while a run of digits (`20240315`) was accepted
/// through `epoch_millis` and indexed in 1970.
///
/// Every indexed date is a point: the start of the range the value names at
/// its own precision, which is what [`FhirDateValue`] gives every backend and
/// what the query side compares its `gte`/`lt` bounds against. It is written
/// as a complete UTC instant at millisecond resolution — all an Elasticsearch
/// `date` holds — so the mapping's format never has to interpret a partial
/// date, an offset or a `:60`.
///
/// A value the FHIR grammar rejects gets one more chance through
/// [`repair_iso_date`], which covers the ISO 8601 spellings Elasticsearch
/// accepted as written, so that what was searchable stays searchable.
/// Anything else is skipped with a warning, as the PostgreSQL and MongoDB
/// writers do: the parameter then behaves as absent for this resource and the
/// rest of the document indexes normally. Index-side code must never fail a
/// write because of odd data.
fn es_index_date(origin: ValueOrigin<'_>, raw: &str) -> Option<String> {
    let parsed = FhirDateValue::parse(raw).or_else(|error| {
        repair_iso_date(raw)
            .and_then(|repaired| FhirDateValue::parse(&repaired).ok())
            .ok_or(error)
    });
    match parsed {
        Ok(parsed) => {
            let (start, _) = parsed.range_at(StorageResolution::Millis);
            Some(start.to_rfc3339_opts(SecondsFormat::Millis, true))
        }
        Err(error) => {
            tracing::warn!(
                resource_type = origin.resource_type,
                resource_id = origin.resource_id,
                param = origin.param,
                "Skipping a date value in the Elasticsearch index: {error}"
            );
            None
        }
    }
}

/// Rewrites the ISO 8601 spellings that are not FHIR but that the `date`
/// mapping's `strict_date_optional_time` accepted as written — an hour
/// without minutes (`T10`), a bare trailing `T`, a `,` decimal mark, and a
/// zone written `±hh` or `±hhmm` — into the FHIR grammar, so that they keep
/// indexing at the instant they always did. The result still has to pass
/// [`FhirDateValue::parse`]. `None` when there is nothing to rewrite.
fn repair_iso_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.is_ascii() {
        return None;
    }
    let (date, time) = raw.split_once('T')?;
    if time.is_empty() {
        return Some(date.to_string());
    }
    let (clock, zone) = match time.find(['Z', '+', '-']) {
        Some(at) => time.split_at(at),
        None => (time, ""),
    };
    let clock = match clock.len() {
        2 => format!("{clock}:00"),
        _ => clock.replacen(',', ".", 1),
    };
    let zone = match zone.len() {
        3 => format!("{zone}:00"),
        5 => format!("{}:{}", &zone[..3], &zone[3..]),
        _ => zone.to_string(),
    };
    let repaired = format!("{date}T{clock}{zone}");
    (repaired != raw).then_some(repaired)
}

/// Merges one composite component's value into the composite instance object,
/// placing it in the array field matching the component's value type. All
/// components of one instance share a nested object, so a single nested query
/// can require every component to match within the same instance.
///
/// A date component that is not a date is left out (see [`es_index_date`]);
/// the instance keeps its other components.
fn merge_composite_component(entry: &mut Value, origin: ValueOrigin<'_>, value: &IndexValue) {
    match value {
        IndexValue::String(s) => push_array_field(entry, "string", json!(s)),
        IndexValue::Token { system, code, .. } => {
            push_array_field(entry, "token_code", json!(code));
            if let Some(sys) = system {
                push_array_field(entry, "token_system", json!(sys));
            }
        }
        IndexValue::Number(n) => push_array_field(entry, "number", json!(n)),
        IndexValue::Quantity {
            value,
            unit,
            system,
            ..
        } => {
            push_array_field(entry, "quantity_value", json!(value));
            if let Some(u) = unit {
                push_array_field(entry, "quantity_unit", json!(u));
            }
            if let Some(s) = system {
                push_array_field(entry, "quantity_system", json!(s));
            }
        }
        IndexValue::Date { value, .. } => {
            if let Some(value) = es_index_date(origin, value) {
                push_array_field(entry, "date", json!(value));
            }
        }
        IndexValue::Reference { reference, .. } => {
            push_array_field(entry, "reference", json!(reference))
        }
        IndexValue::Uri(u) => push_array_field(entry, "uri", json!(u)),
    }
}

pub(crate) fn build_es_document(
    tenant_id: &str,
    resource_type: &str,
    resource_id: &str,
    version_id: &str,
    content: &Value,
    fhir_version: FhirVersion,
    extracted_values: &[ExtractedValue],
) -> Value {
    let searchable = extract_searchable_content(content);

    let mut string_params: Vec<Value> = Vec::new();
    let mut token_params: Vec<Value> = Vec::new();
    let mut date_params: Vec<Value> = Vec::new();
    let mut number_params: Vec<Value> = Vec::new();
    let mut quantity_params: Vec<Value> = Vec::new();
    let mut reference_params: Vec<Value> = Vec::new();
    let mut uri_params: Vec<Value> = Vec::new();
    // Composite instances, keyed by (param name, group) so all components of one
    // instance land in a single nested object with inline (array) component values.
    let mut composite_groups: std::collections::BTreeMap<(String, u32), Value> =
        std::collections::BTreeMap::new();

    for ev in extracted_values {
        let origin = ValueOrigin {
            resource_type,
            resource_id,
            param: &ev.param_name,
        };

        // Composite component values are accumulated into their instance object
        // rather than the per-type arrays.
        if let Some(group) = ev.composite_group {
            let entry = composite_groups
                .entry((ev.param_name.clone(), group))
                .or_insert_with(|| json!({ "name": ev.param_name, "group_id": group }));
            merge_composite_component(entry, origin, &ev.value);
            continue;
        }

        match &ev.value {
            IndexValue::String(s) => {
                string_params.push(json!({
                    "name": ev.param_name,
                    "value": s,
                    "folded": crate::search::fold_text(s),
                }));
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                let mut token = json!({
                    "name": ev.param_name,
                    "code": code,
                });
                if let Some(sys) = system {
                    token["system"] = json!(sys);
                }
                if let Some(disp) = display {
                    token["display"] = json!(disp);
                }
                if let Some(its) = identifier_type_system {
                    token["identifier_type_system"] = json!(its);
                }
                if let Some(itc) = identifier_type_code {
                    token["identifier_type_code"] = json!(itc);
                }
                token_params.push(token);
            }
            IndexValue::Date { value, precision } => {
                if let Some(value) = es_index_date(origin, value) {
                    date_params.push(json!({
                        "name": ev.param_name,
                        "value": value,
                        "precision": format!("{:?}", precision).to_lowercase(),
                    }));
                }
            }
            IndexValue::Number(n) => {
                number_params.push(json!({
                    "name": ev.param_name,
                    "value": n,
                }));
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                code,
            } => {
                let mut qty = json!({
                    "name": ev.param_name,
                    "value": value,
                });
                if let Some(u) = unit {
                    qty["unit"] = json!(u);
                }
                if let Some(s) = system {
                    qty["system"] = json!(s);
                }
                if let Some(c) = code {
                    qty["code"] = json!(c);
                }
                // UCUM-canonical value/unit so quantity search matches equivalent
                // units (g ⇄ mg). Uses the code if present, else the unit display.
                if let Some((cv, cu)) = code
                    .as_deref()
                    .or(unit.as_deref())
                    .and_then(|u| helios_fhirpath::ucum::canonicalize_quantity(*value, u))
                {
                    qty["canonical_value"] = json!(cv);
                    qty["canonical_unit"] = json!(cu);
                }
                quantity_params.push(qty);
            }
            IndexValue::Reference {
                reference,
                resource_type: ref_type,
                resource_id: ref_id,
                display,
            } => {
                let mut ref_doc = json!({
                    "name": ev.param_name,
                    "reference": reference,
                });
                if let Some(rt) = ref_type {
                    ref_doc["resource_type"] = json!(rt);
                }
                if let Some(ri) = ref_id {
                    ref_doc["resource_id"] = json!(ri);
                }
                if let Some(d) = display {
                    ref_doc["display"] = json!(d);
                }
                reference_params.push(ref_doc);
            }
            IndexValue::Uri(u) => {
                uri_params.push(json!({
                    "name": ev.param_name,
                    "value": u,
                }));
            }
        }
    }

    let composite_params: Vec<Value> = composite_groups.into_values().collect();

    json!({
        "resource_type": resource_type,
        "resource_id": resource_id,
        "tenant_id": tenant_id,
        "version_id": version_id,
        "last_updated": Utc::now().to_rfc3339(),
        "fhir_version": fhir_version.as_mime_param(),
        "is_deleted": false,
        "content": content,
        "narrative_text": searchable.narrative,
        "content_text": searchable.full_content,
        "search_params": {
            "string": string_params,
            "token": token_params,
            "date": date_params,
            "number": number_params,
            "quantity": quantity_params,
            "reference": reference_params,
            "uri": uri_params,
            "composite": composite_params,
        }
    })
}

/// Synthetic ES `resource_id` for a contained-resource document: the
/// container's id plus the contained local id, so it is unique within the
/// contained type's index and stable across re-indexing.
pub(crate) fn contained_resource_id(container_id: &str, local_id: &str) -> String {
    format!("{}#{}", container_id, local_id)
}

/// Builds an ES document for a contained resource (`_contained` search). The
/// doc describes the contained resource (its `resource_type`, `content`, and
/// `search_params`) so it matches normally within that type's index, and is
/// tagged with the container's identity for result resolution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_es_contained_document(
    tenant_id: &str,
    container_type: &str,
    container_id: &str,
    contained_type: &str,
    local_id: &str,
    contained_content: &Value,
    version_id: &str,
    fhir_version: FhirVersion,
    extracted_values: &[ExtractedValue],
) -> Value {
    let synthetic_id = contained_resource_id(container_id, local_id);
    let mut doc = build_es_document(
        tenant_id,
        contained_type,
        &synthetic_id,
        version_id,
        contained_content,
        fhir_version,
        extracted_values,
    );
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("is_contained".to_string(), json!(true));
        obj.insert("container_type".to_string(), json!(container_type));
        obj.insert("container_id".to_string(), json!(container_id));
        obj.insert("contained_local_id".to_string(), json!(local_id));
    }
    doc
}

// ============================================================================
// Single-request writes
//
// Every write that is one HTTP request — index a document, delete a document,
// delete by query — goes through `send_write_with_retry`, the write-side
// sibling of `search_impl::send_read_with_retry` (#1382). `_bulk` keeps its own
// loop (`send_bulk_index`): it retries per item and splits oversized requests,
// neither of which applies to a single document.
//
// Retrying is safe because every one of these requests is repeatable:
//
// - Documents are indexed under an explicit `_id` with plain index semantics.
//   HFS never sends `op_type=create`, an external `version`, or
//   `if_seq_no`/`if_primary_term`, so a resend whose first attempt was applied
//   but whose response was lost overwrites the document with itself; it cannot
//   be answered `409`. (A `409` is therefore not expected at all, and is a
//   permanent failure like any other 4xx.)
// - A resent delete-by-id that was already applied is answered `not_found`,
//   which after a failed attempt is read as success — see `WriteOp::Delete`.
// - A delete-by-query that is run again deletes what is still there.
// ============================================================================

/// The write APIs that share one attempt/retry/classify path.
#[derive(Debug, Clone, Copy)]
enum WriteOp<'a> {
    /// `PUT {index}/_doc/{doc_id}`, with the body as the document.
    Index {
        doc_id: &'a str,
        refresh: Option<Refresh>,
    },
    /// `DELETE {index}/_doc/{doc_id}`; the body is not sent.
    ///
    /// Answers [`WriteOutcome::NotFound`] only when the *first* attempt is told
    /// the document is absent. On a resend the same answer most likely means
    /// the earlier attempt was applied and its response lost (a dropped
    /// connection, a gateway's `503`/`504`); either way the document is gone,
    /// which is what was asked for, so it is [`WriteOutcome::Done`].
    Delete {
        doc_id: &'a str,
        refresh: Option<Refresh>,
    },
    /// `POST {index}/_delete_by_query` with a forced refresh, the body as the
    /// query. `index` may be a pattern; missing indices are not an error.
    ///
    /// Version conflicts do not abort it (`conflicts=proceed`), but a run that
    /// reports conflicts, shard failures or a timeout left documents behind and
    /// is repeated like a transient failure. The `deleted` of the outcome is
    /// the total over all runs.
    DeleteByQuery,
}

/// What a write that did not fail achieved.
enum WriteOutcome {
    /// Applied. Carries the response body of a delete-by-query (the only one
    /// read); `Value::Null` for the others.
    Done(Value),
    /// Elasticsearch says the document to delete (or its index) does not exist.
    NotFound,
}

/// Result of a single write attempt.
enum WriteAttempt {
    Done(Value),
    NotFound,
    /// The cluster could not be reached, or asked to be retried.
    Retryable(String),
    /// A delete-by-query answered `200` but left documents behind.
    Incomplete {
        deleted: u64,
        message: String,
    },
    Failed(StorageError),
}

/// Whether a `404` is Elasticsearch saying "this index exists and has no
/// document with that id": the delete API answers exactly that with
/// `"result": "not_found"`. The bare status proves nothing — a proxy or a wrong
/// base path answers `404` too (see `search_impl::is_document_not_found`).
fn is_delete_not_found(status: u16, body: &str) -> bool {
    status == 404
        && serde_json::from_str::<Value>(body)
            .is_ok_and(|parsed| parsed.get("result").and_then(Value::as_str) == Some("not_found"))
}

/// Sends one write request and classifies the response. `what` names the write
/// in error messages ("index document").
async fn send_write_once(
    backend: &ElasticsearchBackend,
    op: WriteOp<'_>,
    index: &str,
    body: &Value,
    what: &str,
) -> WriteAttempt {
    let client = backend.client();
    let response = match op {
        WriteOp::Index { doc_id, refresh } => {
            let mut request = client.index(IndexParts::IndexId(index, doc_id)).body(body);
            if let Some(refresh) = refresh {
                request = request.refresh(refresh);
            }
            request.send().await
        }
        WriteOp::Delete { doc_id, refresh } => {
            let mut request = client.delete(DeleteParts::IndexId(index, doc_id));
            if let Some(refresh) = refresh {
                request = request.refresh(refresh);
            }
            request.send().await
        }
        WriteOp::DeleteByQuery => {
            client
                .delete_by_query(DeleteByQueryParts::Index(&[index]))
                .ignore_unavailable(true)
                .refresh(true)
                .conflicts(elasticsearch::params::Conflicts::Proceed)
                .body(body)
                .send()
                .await
        }
    };

    let response = match response {
        Ok(response) => response,
        // The cluster stopped answering. Unlike a refused connection this is
        // not resent: each further attempt would wait out another full request
        // timeout (and under `refresh=wait_for` a slow answer is the normal
        // cost of the write, not a fault), as `send_bulk_index` found for a
        // lone document (#1125).
        Err(e) if e.is_timeout() => {
            return WriteAttempt::Failed(unavailable_error(format!(
                "Failed to {what}: no answer within {} ms: {e}",
                backend.request_timeout_ms()
            )));
        }
        Err(e) => return WriteAttempt::Retryable(format!("Elasticsearch unreachable: {e}")),
    };

    let status = response.status_code().as_u16();
    if (200..300).contains(&status) {
        if !matches!(op, WriteOp::DeleteByQuery) {
            // An acknowledged write is done; nothing in its body is needed.
            return WriteAttempt::Done(Value::Null);
        }
        let payload: Value = match response.json().await {
            Ok(payload) => payload,
            Err(e) => {
                return WriteAttempt::Failed(internal_error(format!(
                    "Failed to {what}: unreadable delete-by-query response (status {status}): {e}"
                )));
            }
        };
        // How many documents went is what the caller reports; an answer that
        // does not say is not an answer of `0`.
        let Some(deleted) = payload.get("deleted").and_then(Value::as_u64) else {
            return WriteAttempt::Failed(internal_error(format!(
                "Failed to {what}: delete-by-query response carries no deleted count: {payload}"
            )));
        };
        let conflicts = payload
            .get("version_conflicts")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let failures = payload
            .get("failures")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let timed_out = payload.get("timed_out").and_then(Value::as_bool) == Some(true);
        if conflicts > 0 || failures > 0 || timed_out {
            return WriteAttempt::Incomplete {
                deleted,
                message: format!(
                    "delete-by-query left documents behind ({conflicts} version conflicts, \
                     {failures} failures, timed_out={timed_out})"
                ),
            };
        }
        return WriteAttempt::Done(payload);
    }

    let text = response.text().await.unwrap_or_default();
    match op {
        WriteOp::Delete { .. }
            if is_index_not_found(status, &text) || is_delete_not_found(status, &text) =>
        {
            return WriteAttempt::NotFound;
        }
        // `ignore_unavailable` already covers this; kept for a cluster or
        // proxy that answers it anyway. Nothing there means nothing to delete.
        WriteOp::DeleteByQuery if is_index_not_found(status, &text) => {
            return WriteAttempt::Done(json!({ "deleted": 0 }));
        }
        _ => {}
    }

    match classify_es_failure(status, &text) {
        EsFailureClass::Retryable => WriteAttempt::Retryable(format!("status {status}: {text}")),
        // `BadQuery` is about search values; on a write it is a rejection of
        // the document (or of the query HFS built) like any other.
        EsFailureClass::BadQuery | EsFailureClass::Permanent => WriteAttempt::Failed(
            internal_error(format!("Failed to {what} (status {status}): {text}")),
        ),
    }
}

/// Sends a write and retries it while the failure is one the cluster may
/// recover from — `429`, `502`/`503`/`504`, a rejected execution, a refused
/// connection — on the schedule the reads use ([`MAX_SEARCH_RETRIES`] resends,
/// [`RETRY_BASE_DELAY_MS`] doubling: at most ~300 ms of waiting). A permanent
/// rejection (any other 4xx, a bare `500`) is never resent.
///
/// An exhausted retry is [`BackendError::Unavailable`], as `ensure_index`
/// reports the same condition: the cluster never judged the write, so repeating
/// it later (the composite's own retry, a `$reindex`) can succeed. A rejection
/// is [`BackendError::Internal`].
///
/// Under `write_refresh=wait_for` every attempt that reaches the cluster also
/// waits for the next refresh, so a resend after a gateway error can pay that
/// wait again; attempts Elasticsearch itself rejected did not wait for one.
async fn send_write_with_retry(
    backend: &ElasticsearchBackend,
    op: WriteOp<'_>,
    index: &str,
    body: &Value,
    what: &str,
) -> StorageResult<WriteOutcome> {
    let mut last_failure = String::new();
    // Documents removed by delete-by-query runs that then had to be repeated.
    let mut already_deleted = 0u64;

    for attempt in 0..=MAX_SEARCH_RETRIES {
        match send_write_once(backend, op, index, body, what).await {
            WriteAttempt::Done(mut payload) => {
                if already_deleted > 0 {
                    let total = payload.get("deleted").and_then(Value::as_u64).unwrap_or(0)
                        + already_deleted;
                    payload["deleted"] = json!(total);
                }
                return Ok(WriteOutcome::Done(payload));
            }
            WriteAttempt::NotFound if attempt > 0 => return Ok(WriteOutcome::Done(Value::Null)),
            WriteAttempt::NotFound => return Ok(WriteOutcome::NotFound),
            WriteAttempt::Failed(error) => return Err(error),
            WriteAttempt::Retryable(message) => last_failure = message,
            WriteAttempt::Incomplete { deleted, message } => {
                already_deleted += deleted;
                last_failure = message;
            }
        }

        if attempt < MAX_SEARCH_RETRIES {
            let delay_ms = RETRY_BASE_DELAY_MS << attempt;
            tracing::warn!(
                attempt = attempt + 1,
                max = MAX_SEARCH_RETRIES + 1,
                delay_ms,
                index,
                failure = %last_failure,
                "Retryable ES write failure ({what}), retrying"
            );
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    let attempts = MAX_SEARCH_RETRIES + 1;
    Err(unavailable_error(format!(
        "Failed to {what} after {attempts} attempts: {last_failure}"
    )))
}

/// Indexes one document under `doc_id` with the given refresh policy.
async fn index_document(
    backend: &ElasticsearchBackend,
    index: &str,
    doc_id: &str,
    doc: &Value,
    refresh: Option<Refresh>,
    what: &str,
) -> StorageResult<()> {
    let op = WriteOp::Index { doc_id, refresh };
    send_write_with_retry(backend, op, index, doc, what)
        .await
        .map(|_| ())
}

// Contained-document maintenance.
//
// A container's `contained[]` resources are separate documents, written after
// the container's own. Elasticsearch has no transaction to put around the two,
// so a failure here leaves them out of step: the container is indexed (or
// deleted) and its contained documents are stale. Such a failure FAILS THE
// WRITE, on every path alike (`create`, `update`, `create_or_update`, `delete`,
// `write_search_entries`), with an error that says the container's own document
// was already written. It is not reported as a success with a warning because:
//
// - Elasticsearch is only ever a search secondary, so "the write failed" never
//   loses the resource — the primary has it — while a swallowed failure leaves
//   `_contained` searches wrong with nothing on record (#1382).
// - Every step is repeatable, so the caller's remedy is to repeat the write:
//   the container document is overwritten with itself, the sweep and the
//   contained documents are redone. A repeated `delete` finds the container
//   gone and still runs the sweep (see `delete`).
impl ElasticsearchBackend {
    /// Deletes all contained-resource docs derived from the given container
    /// (across the tenant's indices), used before re-indexing or on container
    /// delete so removed contained resources don't linger.
    pub(crate) async fn delete_contained_docs(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
    ) -> StorageResult<()> {
        let pattern = self.tenant_index_pattern(tenant_id);
        let body = json!({
            "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } },
                { "term": { "is_contained": true } },
                { "term": { "container_type": container_type } },
                { "term": { "container_id": container_id } }
            ]}}
        });
        // Missing indices are fine (nothing to delete). Anything else that is
        // not a completed sweep is an error: the response used to be discarded
        // whole, so a `503` left the stale documents matching `_contained`
        // searches while the write reported success (#1382).
        let what = format!(
            "delete the stale contained documents of {container_type}/{container_id} \
             (its own document is already written; repeat the write or reindex the \
             resource to bring them back in step)"
        );
        delete_by_query_scoped(self, &pattern, body, &what).await?;
        Ok(())
    }

    /// Extracts and indexes a container's `contained[]` resources as separate
    /// `is_contained` docs in their respective type indices. When `delete_first`
    /// is set, stale contained docs for this container are removed first
    /// (needed on update, where contained resources may have changed/been removed).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn index_contained_docs(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        resource: &Value,
        fhir_version: FhirVersion,
        version_id: &str,
        delete_first: bool,
    ) -> StorageResult<()> {
        if delete_first {
            self.delete_contained_docs(tenant_id, container_type, container_id)
                .await?;
        }

        for contained in self.tenant_extractor(tenant_id).extract_contained(resource) {
            let doc = build_es_contained_document(
                tenant_id,
                container_type,
                container_id,
                &contained.contained_type,
                &contained.local_id,
                &contained.content,
                version_id,
                fhir_version,
                &contained.values,
            );
            schema::ensure_index(self, tenant_id, &contained.contained_type).await?;
            let index = self.index_name(tenant_id, &contained.contained_type);
            let doc_id = Self::document_id(
                &contained.contained_type,
                &contained_resource_id(container_id, &contained.local_id),
            );
            let what = format!(
                "index contained document {}#{} of {container_type}/{container_id} \
                 (its own document is already written; repeat the write or reindex the \
                 resource to bring them back in step)",
                contained.contained_type, contained.local_id
            );
            index_document(
                self,
                &index,
                &doc_id,
                &doc,
                self.write_refresh_param(),
                &what,
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl ResourceStorage for ElasticsearchBackend {
    fn backend_name(&self) -> &'static str {
        "elasticsearch"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn bulk_write_concurrency(&self) -> usize {
        // One index request per resource; ES absorbs parallel writers.
        8
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

        let tenant_id = tenant.tenant_id().as_str();

        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(crate::types::new_resource_id);

        let version_id = "1";

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.clone()));
        }

        // Extract search parameters
        let extracted_values = self
            .tenant_extractor(tenant_id)
            .extract(&resource, resource_type)
            .unwrap_or_default();

        // Build ES document
        let doc = build_es_document(
            tenant_id,
            resource_type,
            &id,
            version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        // Ensure index exists
        schema::ensure_index(self, tenant_id, resource_type).await?;

        // Index the document
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, &id);

        index_document(
            self,
            &index,
            &doc_id,
            &doc,
            self.write_refresh_param(),
            "index document",
        )
        .await?;

        // Index any contained resources for `_contained` search. New resource,
        // so no stale docs to delete first.
        if resource
            .get("contained")
            .and_then(|c| c.as_array())
            .is_some_and(|a| !a.is_empty())
        {
            self.index_contained_docs(
                tenant_id,
                resource_type,
                &id,
                &resource,
                fhir_version,
                version_id,
                false,
            )
            .await?;
        }

        let now = Utc::now();
        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    /// `_bulk` requests of at most [`BULK_OPS_PER_REQUEST`] operations and the
    /// configured byte budget, with the configured write-refresh policy applied
    /// once per request rather than once per document.
    ///
    /// This is what makes a bulk load survivable under `refresh=wait_for`:
    /// that policy blocks each write until the next scheduled refresh (the
    /// index's `refresh_interval`, 1s by default), so N per-document writes
    /// cost N refresh waits — the ~1.4k-resource conformance seed took over
    /// twenty minutes and the server never came up — while one bulk request
    /// costs one.
    async fn create_many(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resources: Vec<Value>,
        fhir_version: FhirVersion,
    ) -> Vec<StorageResult<StoredResource>> {
        if resources.is_empty() {
            return Vec::new();
        }
        if tenant
            .check_permission(Operation::Create, resource_type)
            .is_err()
        {
            return resources
                .iter()
                .map(|_| {
                    tenant
                        .check_permission(Operation::Create, resource_type)
                        .map(|()| unreachable!("permission check failed a moment ago"))
                        .map_err(StorageError::from)
                })
                .collect();
        }

        let tenant_id = tenant.tenant_id().as_str();
        let version_id = "1";
        let extractor = self.tenant_extractor(tenant_id);

        // Every document each resource contributes — its own, plus one per
        // `contained` entry — addressed by (index, doc id). Built up front so
        // the indices can be ensured once each and the operations streamed
        // out in a few requests.
        struct Prepared {
            id: String,
            resource: Value,
            docs: Vec<(String, String, Value)>,
        }
        let mut types_touched = vec![resource_type.to_string()];
        let prepared: Vec<Prepared> = resources
            .into_iter()
            .map(|mut resource| {
                let id = resource
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(crate::types::new_resource_id);
                if let Some(obj) = resource.as_object_mut() {
                    obj.insert(
                        "resourceType".to_string(),
                        Value::String(resource_type.to_string()),
                    );
                    obj.insert("id".to_string(), Value::String(id.clone()));
                }
                let extracted_values = extractor
                    .extract(&resource, resource_type)
                    .unwrap_or_default();
                let mut docs = vec![(
                    self.index_name(tenant_id, resource_type),
                    Self::document_id(resource_type, &id),
                    build_es_document(
                        tenant_id,
                        resource_type,
                        &id,
                        version_id,
                        &resource,
                        fhir_version,
                        &extracted_values,
                    ),
                )];
                for contained in extractor.extract_contained(&resource) {
                    types_touched.push(contained.contained_type.clone());
                    docs.push((
                        self.index_name(tenant_id, &contained.contained_type),
                        Self::document_id(
                            &contained.contained_type,
                            &contained_resource_id(&id, &contained.local_id),
                        ),
                        build_es_contained_document(
                            tenant_id,
                            resource_type,
                            &id,
                            &contained.contained_type,
                            &contained.local_id,
                            &contained.content,
                            version_id,
                            fhir_version,
                            &contained.values,
                        ),
                    ));
                }
                Prepared { id, resource, docs }
            })
            .collect();

        // Ensure every index touched exists, once each.
        let mut ensured = std::collections::HashSet::new();
        for ty in types_touched {
            if ensured.insert(ty.clone())
                && let Err(e) = schema::ensure_index(self, tenant_id, &ty).await
            {
                let message = error_detail(&e);
                return prepared
                    .iter()
                    .map(|_| Err(internal_error(message.clone())))
                    .collect();
            }
        }

        // Flatten to operations, remembering which resource each belongs to,
        // and send them in bounded requests. A resource's outcome is the first
        // failure among its own operations, if any.
        let ops: Vec<(usize, &str, &str, &Value)> = prepared
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                p.docs
                    .iter()
                    .map(move |(index, doc_id, doc)| (i, index.as_str(), doc_id.as_str(), doc))
            })
            .collect();
        let failures = self
            .send_bulk_index(&ops, prepared.len(), self.write_refresh_param())
            .await;

        let now = Utc::now();
        prepared
            .into_iter()
            .zip(failures)
            .map(|(p, failure)| match failure {
                Some(failure) => Err(internal_error(failure.message)),
                None => Ok(StoredResource::from_storage(
                    resource_type,
                    &p.id,
                    version_id,
                    tenant.tenant_id().clone(),
                    p.resource,
                    now,
                    now,
                    None,
                    fhir_version,
                )),
            })
            .collect()
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        let tenant_id = tenant.tenant_id().as_str();

        // Check if document exists
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        // Deciding "this resource is new, start at version 1" requires knowing that
        // it does not already exist. Only Elasticsearch saying so establishes that
        // (`"found": false`, or the index does not exist) — the same two answers
        // `read` accepts, through the same retried request (#1364). If the
        // existence check failed, or was answered by something else with a bare
        // 404, we must not guess "new" — doing so would reset the version of a
        // resource that does exist, silently clobbering its history.
        let op = ReadOp::Get { doc_id: &doc_id };
        let existing = send_read_with_retry(self, op, &index, Value::Null).await?;

        let (version_id, is_new) = match existing {
            Some(body) => {
                let source = body.get("_source");
                // Belt-and-braces tenant guard, mirroring `read` (see below).
                //
                // After the #384 fix an injective `index_name` already guarantees
                // this index belongs to exactly one tenant, so this check should
                // never fire. It is kept because this is the one place the backend
                // reads *foreign state* to make a *write* decision, and a future
                // regression in the naming derivation would otherwise silently
                // resume deriving one tenant's version from another's document.
                //
                // A mismatch is treated as **absent**, not as an error. An index
                // upgraded from the pre-fix layout can still hold documents left
                // behind by a colliding tenant; erroring would brick the rightful
                // owner on exactly those ids, permanently, with no operator
                // remedy. Treating them as absent is self-healing — the foreign
                // document is overwritten and leaves an index it never belonged
                // in — and is correct on the merits: from this tenant's
                // perspective the resource genuinely does not exist. Resetting the
                // version is harmless because Elasticsearch keeps no history and,
                // in every supported composite mode, the primary is authoritative
                // for version assignment (writes always land there first).
                let doc_tenant = source
                    .and_then(|s| s.get("tenant_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if doc_tenant != tenant_id {
                    tracing::warn!(
                        tenant = %tenant_id,
                        found_tenant = %doc_tenant,
                        resource_type,
                        id,
                        "Elasticsearch document at this address belongs to another tenant; \
                         treating as absent and overwriting (see issue #384)"
                    );
                    ("1".to_string(), true)
                } else {
                    let current_version: u64 = source
                        .and_then(|s| s.get("version_id"))
                        .and_then(|v| v.as_str())
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    ((current_version + 1).to_string(), false)
                }
            }
            None => ("1".to_string(), true),
        };

        // Ensure resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

        // Extract search parameters
        let extracted_values = self
            .tenant_extractor(tenant_id)
            .extract(&resource, resource_type)
            .unwrap_or_default();

        let doc = build_es_document(
            tenant_id,
            resource_type,
            id,
            &version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        // Ensure index exists
        schema::ensure_index(self, tenant_id, resource_type).await?;

        index_document(
            self,
            &index,
            &doc_id,
            &doc,
            self.write_refresh_param(),
            "index document",
        )
        .await?;

        // Re-sync contained docs (delete stale, then re-index) so updates that
        // add/remove/change `contained[]` entries are reflected.
        self.index_contained_docs(
            tenant_id,
            resource_type,
            id,
            &resource,
            fhir_version,
            &version_id,
            true,
        )
        .await?;

        let now = Utc::now();
        Ok((
            StoredResource::from_storage(
                resource_type,
                id,
                &version_id,
                tenant.tenant_id().clone(),
                resource,
                now,
                now,
                None,
                fhir_version,
            ),
            is_new,
        ))
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        // `Ok(None)` means "this resource does not exist" — a factual claim about
        // the data. Only ES itself can license that claim, and it does so in
        // exactly two ways: a 404 with `"found": false`, or a 404 naming an
        // `index_not_found_exception`. A bare 404 (a proxy, a wrong base path), a
        // transport failure (cluster down, DNS, TLS, timeout) or a 5xx/401/403
        // means we never learned anything, and must surface as an error — after
        // the same retries a search gets, when it is transient (#1364). Reporting
        // it as "not found" would make a down cluster indistinguishable from an
        // empty one, which is exactly the misleading result this contract forbids.
        let op = ReadOp::Get { doc_id: &doc_id };
        let Some(body) = send_read_with_retry(self, op, &index, Value::Null).await? else {
            return Ok(None);
        };

        // A found document always carries its `_source` (the mapping never
        // disables it), so a 200 without one did not come from the get API.
        let source = body.get("_source").ok_or_else(|| {
            internal_error(format!(
                "Get response for {resource_type}/{id} carries no _source: {body}"
            ))
        })?;

        // Check if deleted
        if source
            .get("is_deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }

        // Verify tenant
        let doc_tenant = source
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if doc_tenant != tenant_id {
            return Ok(None);
        }

        parse_stored_resource(source, tenant)
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let tenant_id = tenant.tenant_id().as_str();
        let id = current.id();
        let new_version: u64 = current.version_id().parse::<u64>().unwrap_or(0) + 1;
        let version_id = new_version.to_string();
        let fhir_version = current.fhir_version();

        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

        let extracted_values = self
            .tenant_extractor(tenant_id)
            .extract(&resource, resource_type)
            .unwrap_or_default();

        let doc = build_es_document(
            tenant_id,
            resource_type,
            id,
            &version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        index_document(
            self,
            &index,
            &doc_id,
            &doc,
            self.write_refresh_param(),
            "update document",
        )
        .await?;

        // Re-sync contained docs exactly as `create_or_update` does. This path
        // used to skip it, so an update that changed `contained[]` left the old
        // contained documents matching `_contained` searches (#1382).
        self.index_contained_docs(
            tenant_id,
            resource_type,
            id,
            &resource,
            fhir_version,
            &version_id,
            true,
        )
        .await?;

        let now = Utc::now();
        Ok(StoredResource::from_storage(
            resource_type,
            id,
            &version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        // "Not found" is a claim only Elasticsearch can make — by
        // `"result": "not_found"` or an `index_not_found_exception`. A bare
        // `404` (a proxy, a wrong base path) used to be taken for it; it is an
        // error now, as it is for `read` (#1364, #1382).
        let op = WriteOp::Delete {
            doc_id: &doc_id,
            refresh: self.write_refresh_param(),
        };
        let outcome =
            send_write_with_retry(self, op, &index, &Value::Null, "delete document").await?;

        // Remove any contained-resource docs derived from this container —
        // also when the container's own document is already gone: a delete that
        // is repeated because this sweep failed finds exactly that, and the
        // stale contained documents must not outlive every repeat.
        self.delete_contained_docs(tenant_id, resource_type, id)
            .await?;

        match outcome {
            WriteOutcome::Done(_) => Ok(()),
            WriteOutcome::NotFound => Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })),
        }
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();

        let index_pattern = match resource_type {
            Some(rt) => self.index_name(tenant_id, rt),
            None => self.tenant_index_pattern(tenant_id),
        };

        let query = json!({
            "query": {
                "bool": {
                    "filter": [
                        { "term": { "tenant_id": tenant_id } },
                        { "term": { "is_deleted": false } }
                    ]
                }
            }
        });

        // As in `read`: a count of 0 is a claim about the data. Make it only when
        // the cluster says so, or when it says the index does not exist (yet) —
        // by the parsed `index_not_found_exception`, not by a bare 404. Anything
        // else goes the way it does for `search_count`: retried when transient,
        // then an error, never "zero resources" (#1364).
        match send_read_with_retry(self, ReadOp::Count, &index_pattern, query).await? {
            None => Ok(0),
            Some(body) => body
                .get("count")
                .and_then(Value::as_u64)
                .ok_or_else(|| internal_error(format!("Count response carries no count: {body}"))),
        }
    }

    // `supports_tenant_registry` stays `false`: ES is a search secondary, never
    // the registry of record. Only the data purge is implemented, so that
    // composite storage can clear a purged tenant's offloaded search documents
    // (in `*-elasticsearch` modes the primary's own search index is empty).
    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        // Documents are matched by an exact `tenant_id` term, not by the index
        // pattern alone: the pattern is a prefix glob, so tenant `a`'s pattern
        // `{prefix}_a_*` also matches tenant `a_b`'s indices. The term filter,
        // not the glob, is what bounds this to one tenant.
        let pattern = self.tenant_index_pattern(id);
        let body = json!({
            "query": { "bool": { "filter": [
                { "term": { "tenant_id": id } }
            ]}}
        });
        // Missing indices are fine (nothing to delete). A failed purge is not:
        // the status used to go unread here, so any failure at all was a
        // successful purge of `0` documents (#1382).
        //
        // Deleted-doc count (includes contained/tombstone docs) — informational;
        // the composite reports the primary's count to the admin API.
        delete_by_query_scoped(self, &pattern, body, "purge tenant data").await
    }
}

/// Parses a StoredResource from an ES `_source` document.
fn parse_stored_resource(
    source: &Value,
    tenant: &TenantContext,
) -> StorageResult<Option<StoredResource>> {
    let resource_type = source
        .get("resource_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| internal_error("Missing resource_type in ES document".to_string()))?;

    let resource_id = source
        .get("resource_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| internal_error("Missing resource_id in ES document".to_string()))?;

    let version_id = source
        .get("version_id")
        .and_then(|v| v.as_str())
        .unwrap_or("1");

    let content = source.get("content").cloned().unwrap_or_else(|| json!({}));

    let fhir_version_str = source
        .get("fhir_version")
        .and_then(|v| v.as_str())
        .unwrap_or("4.0");

    let fhir_version = FhirVersion::from_mime_param(fhir_version_str)
        .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

    let last_updated = source
        .get("last_updated")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Ok(Some(StoredResource::from_storage(
        resource_type,
        resource_id,
        version_id,
        tenant.tenant_id().clone(),
        content,
        last_updated,
        last_updated,
        None,
        fhir_version,
    )))
}

// ============================================================================
// PurgableStorage
//
// Elasticsearch keeps no history — a document is the current version and
// nothing else — so `purge` and `delete` collapse to the same operation here.
// `delete` is already a hard delete (see above), so purge reuses it.
//
// Elasticsearch is never a standalone backend; it is always a search secondary
// in front of a SQL, MongoDB, or S3 primary. Purging it is therefore always
// part of a composite fan-out, and losing a document here is recoverable by
// reindexing from the primary.
// ============================================================================

#[async_trait]
impl PurgableStorage for ElasticsearchBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        // ResourceStorage::delete already removes the document outright and
        // sweeps its contained-resource docs, which is exactly purge semantics
        // for a backend with no history.
        match ResourceStorage::delete(self, tenant, resource_type, id).await {
            // A purge whose document is already absent has nothing to undo. The
            // composite fan-out retries a failed purge, so this must be
            // idempotent or the retry would fail on the backend that succeeded.
            Err(StorageError::Resource(ResourceError::NotFound { .. })) => Ok(()),
            other => other,
        }
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        let deleted = delete_by_query_scoped(
            self,
            &index,
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } }
            ]}}}),
            "purge all documents of a type",
        )
        .await?;

        // Contained-resource docs derived from this type live in *other* type
        // indices, so they need a second, tenant-scoped sweep.
        let pattern = tenant_index_pattern(self, tenant_id);
        delete_by_query_scoped(
            self,
            &pattern,
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } },
                { "term": { "is_contained": true } },
                { "term": { "container_type": resource_type } }
            ]}}}),
            "purge the contained documents of a type (its own documents are already purged)",
        )
        .await?;

        Ok(deleted)
    }
}

// ============================================================================
// ReindexSource / ReindexTarget
//
// Elasticsearch has no separate search-index structure: the indexed document
// *is* the search entry. That makes it a legitimate ReindexTarget but means the
// two write methods behave differently from a SQL backend's — see each below.
// ============================================================================

impl ElasticsearchBackend {
    /// Indexes `ops` — `(owner, index, document id, document)` — in bounded
    /// `_bulk` requests, and returns the first failure of each of the `owners`.
    ///
    /// Owners are positions in the caller's own list — a resource usually
    /// contributes more than one document (its own plus one per `contained`
    /// entry), and one bad document fails the resource that produced it and
    /// nothing else.
    ///
    /// Shared by [`ResourceStorage::create_many`] and
    /// [`ReindexTarget::write_search_entries_page`] so a rebuild and a bulk
    /// create put the same documents on the wire the same way, with `refresh`
    /// applied once per request rather than once per document.
    ///
    /// Each operation is serialized once; its size bounds requests by bytes as
    /// well as by [`BULK_OPS_PER_REQUEST`] operations, and the same bytes are
    /// resent however often a request is retried. A failure is handled by what
    /// it says about the documents (#1125):
    ///
    /// - The client timed out, or the cluster (or a proxy) answered `413`,
    ///   `408` or `504`: the request was too large, not its documents wrong. It
    ///   is split in half and each half is resent, down to a single document;
    ///   only a lone document that still times out (transient) or is too large
    ///   (permanent) is reported. A lone document timing out on the client
    ///   fails the rest of the page as transient instead of splitting further:
    ///   the cluster is stalled, and every further halving would wait out
    ///   another timeout.
    /// - The cluster answered `429`, for the request or for items in it: the
    ///   rejected operations alone are resent after a back-off, up to
    ///   [`BULK_MAX_ATTEMPTS`] attempts, then reported as transient.
    /// - An item rejected with another `4xx` is permanent: rerunning it fails
    ///   the same way. Its message names the error's `type` and `reason`.
    /// - Anything else on the whole request (connection refused, `5xx`, an
    ///   unreadable response) fails its operations as transient.
    async fn send_bulk_index(
        &self,
        ops: &[(usize, &str, &str, &Value)],
        owners: usize,
        refresh: Option<Refresh>,
    ) -> Vec<Option<BulkFailure>> {
        /// Operations sent together, by position in `ops`.
        struct Batch {
            ops: Vec<usize>,
            /// How many times these operations were already resent after `429`.
            resends: u32,
            /// How many halvings produced this batch.
            depth: u32,
        }

        let mut failures: Vec<Option<BulkFailure>> = (0..owners).map(|_| None).collect();
        fn fail(
            failures: &mut [Option<BulkFailure>],
            owner: usize,
            message: &str,
            transient: bool,
        ) {
            failures[owner].get_or_insert_with(|| BulkFailure {
                message: message.to_string(),
                transient,
            });
        }

        // The action and document lines of every operation, serialized once.
        let mut lines: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(ops.len());
        let mut sendable: Vec<usize> = Vec::with_capacity(ops.len());
        let mut sizes: Vec<usize> = Vec::with_capacity(ops.len());
        for (position, (owner, index, doc_id, doc)) in ops.iter().enumerate() {
            let action =
                serde_json::to_vec(&json!({ "index": { "_index": index, "_id": doc_id } }));
            match (action, serde_json::to_vec(doc)) {
                (Ok(action), Ok(document)) => {
                    // `NdBody` ends each line with a newline.
                    sizes.push(action.len() + document.len() + 2);
                    sendable.push(position);
                    lines.push((action, document));
                }
                (Err(e), _) | (_, Err(e)) => {
                    fail(
                        &mut failures,
                        *owner,
                        &format!("Failed to serialize document: {e}"),
                        false,
                    );
                    lines.push((Vec::new(), Vec::new()));
                }
            }
        }

        let roots: Vec<Batch> = chunk_ranges(&sizes, BULK_OPS_PER_REQUEST, self.bulk_max_bytes())
            .into_iter()
            .map(|range| Batch {
                ops: sendable[range].to_vec(),
                resends: 0,
                depth: 0,
            })
            .collect();

        // Requests of one page never touch the same document, so they may go
        // out together. What has to stay sequential is the chain a request
        // produces: its halves, its back-off resends and the order they are
        // sent in. So each root chunk gets its own queue, and
        // `HFS_ELASTICSEARCH_BULK_CONCURRENCY` says how many of those chains
        // are in flight (default `1` — one request at a time, as before).
        let concurrency = self.bulk_concurrency().clamp(1, roots.len().max(1));
        // A single document timing out means the cluster stopped answering,
        // not that the request was too large: once one chain finds that out,
        // the others stop sending what they still have queued instead of
        // waiting out a timeout each (#1125). Chains that are already halving
        // in lockstep still pay their own way down, so a stalled cluster costs
        // about `concurrency * log2(chunk)` requests — far short of one per
        // document, and in parallel.
        let stalled = AtomicBool::new(false);
        let lines = &lines;
        let stalled = &stalled;
        let send_chain = |mut queue: VecDeque<Batch>| async move {
            let mut failures: Vec<Option<BulkFailure>> = (0..owners).map(|_| None).collect();

            // Halves `batch` and puts both halves at the front of the queue, in order.
            let split = |queue: &mut VecDeque<Batch>, mut batch: Batch, reason: &str| {
                if batch.depth == 0 {
                    tracing::warn!(
                        operations = batch.ops.len(),
                        reason,
                        "Elasticsearch _bulk request too large; splitting it in half and resending each half"
                    );
                } else {
                    tracing::debug!(
                        operations = batch.ops.len(),
                        depth = batch.depth,
                        reason,
                        "splitting _bulk request again"
                    );
                }
                let right = batch.ops.split_off(batch.ops.len() / 2);
                queue.push_front(Batch {
                    ops: right,
                    resends: 0,
                    depth: batch.depth + 1,
                });
                queue.push_front(Batch {
                    ops: batch.ops,
                    resends: 0,
                    depth: batch.depth + 1,
                });
            };

            while let Some(batch) = queue.pop_front() {
                let fail_batch =
                    |failures: &mut [Option<BulkFailure>], message: &str, transient: bool| {
                        for &position in &batch.ops {
                            fail(failures, ops[position].0, message, transient);
                        }
                    };
                if stalled.load(Ordering::Relaxed) {
                    fail_batch(
                        &mut failures,
                        "Not sent: Elasticsearch stopped answering _bulk requests for this page",
                        true,
                    );
                    continue;
                }
                if batch.resends > 0 {
                    tokio::time::sleep(backoff_delay(batch.resends)).await;
                }

                let body: Vec<&[u8]> = batch
                    .ops
                    .iter()
                    .flat_map(|&position| {
                        let (action, document) = &lines[position];
                        [action.as_slice(), document.as_slice()]
                    })
                    .collect();
                let mut request = self.client().bulk(BulkParts::None).body(body);
                if let Some(refresh) = refresh {
                    request = request.refresh(refresh);
                }

                let response = match request.send().await {
                    Ok(response) => response,
                    Err(e) if e.is_timeout() && batch.ops.len() > 1 => {
                        split(&mut queue, batch, "timeout");
                        continue;
                    }
                    Err(e) if e.is_timeout() => {
                        let message = format!(
                            "Bulk index request for a single document timed out after {} ms: {e}",
                            self.request_timeout_ms()
                        );
                        fail_batch(&mut failures, &message, true);
                        // A lone document timing out means the cluster, not the
                        // request size, is the problem: halving the rest of the
                        // queue would wait out a timeout per document.
                        let remaining: usize = queue.iter().map(|rest| rest.ops.len()).sum();
                        if remaining > 0 {
                            tracing::warn!(
                                remaining,
                                "Elasticsearch did not answer a single-document _bulk request in time; failing the rest of the page as transient"
                            );
                        }
                        stalled.store(true, Ordering::Relaxed);
                        for rest in queue.drain(..) {
                            for position in rest.ops {
                                fail(&mut failures, ops[position].0, &message, true);
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        fail_batch(
                            &mut failures,
                            &format!("Failed to send bulk index request: {e}"),
                            true,
                        );
                        continue;
                    }
                };

                // The status decides before the body is read: a `413` or a proxy's
                // error page need not be JSON.
                let status = response.status_code().as_u16();
                // `408`/`504` are a proxy's or gateway's timeout: the same failure
                // as the client's own, reported by something in between.
                let too_large = status == 413;
                let timed_out = status == 408 || status == 504;
                if too_large || timed_out {
                    if batch.ops.len() > 1 {
                        let reason = if too_large {
                            "413 Request Entity Too Large"
                        } else {
                            "request timed out upstream (408/504)"
                        };
                        split(&mut queue, batch, reason);
                    } else {
                        let body = response.text().await.unwrap_or_default();
                        let message = if too_large {
                            format!("Document too large for a bulk request (status 413): {body}")
                        } else {
                            format!(
                                "Bulk index request for a single document timed out (status {status}): {body}"
                            )
                        };
                        fail_batch(&mut failures, &message, timed_out);
                    }
                    continue;
                }
                if status == 429 {
                    let body = response.text().await.unwrap_or_default();
                    if batch.resends + 1 < BULK_MAX_ATTEMPTS {
                        queue.push_front(Batch {
                            ops: batch.ops,
                            resends: batch.resends + 1,
                            depth: batch.depth,
                        });
                    } else {
                        fail_batch(
                            &mut failures,
                            &format!(
                                "Bulk index request throttled (status 429) {BULK_MAX_ATTEMPTS} times: {body}"
                            ),
                            true,
                        );
                    }
                    continue;
                }
                if !(200..300).contains(&status) {
                    let body = response.text().await.unwrap_or_default();
                    fail_batch(
                        &mut failures,
                        &format!("Bulk index request failed (status {status}): {body}"),
                        is_transient_bulk_status(u64::from(status)),
                    );
                    continue;
                }

                let payload: Value = match response.json().await {
                    Ok(payload) => payload,
                    Err(e) if e.is_timeout() && batch.ops.len() > 1 => {
                        split(&mut queue, batch, "timeout reading the response");
                        continue;
                    }
                    Err(e) => {
                        fail_batch(
                            &mut failures,
                            &format!("Failed to read bulk index response: {e}"),
                            true,
                        );
                        continue;
                    }
                };
                let items = payload.get("items").and_then(Value::as_array);
                let mut throttled: Vec<usize> = Vec::new();
                let mut throttle_message: Option<String> = None;
                for (item_position, &position) in batch.ops.iter().enumerate() {
                    let item = items
                        .and_then(|items| items.get(item_position))
                        .and_then(|item| item.get("index"));
                    let item_status = item
                        .and_then(|v| v.get("status"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if (200..300).contains(&item_status) {
                        continue;
                    }
                    let error = item
                        .and_then(|v| v.get("error"))
                        .map(describe_item_error)
                        .unwrap_or_else(|| "no item in bulk response".to_string());
                    let message =
                        format!("Failed to index document (status {item_status}): {error}");
                    if item_status == 429 {
                        throttled.push(position);
                        throttle_message.get_or_insert(message);
                        continue;
                    }
                    // A missing item (status 0) means the response did not
                    // account for the document, not that it was rejected.
                    fail(
                        &mut failures,
                        ops[position].0,
                        &message,
                        item_status == 0 || is_transient_bulk_status(item_status),
                    );
                }
                if !throttled.is_empty() {
                    if batch.resends + 1 < BULK_MAX_ATTEMPTS {
                        queue.push_front(Batch {
                            ops: throttled,
                            resends: batch.resends + 1,
                            depth: batch.depth,
                        });
                    } else {
                        let message = format!(
                            "{} (throttled {BULK_MAX_ATTEMPTS} times)",
                            throttle_message.unwrap_or_default()
                        );
                        for position in throttled {
                            fail(&mut failures, ops[position].0, &message, true);
                        }
                    }
                }
            }
            failures
        };

        // Merged in chunk order, so which message a resource ends up with does
        // not depend on which chain finished first.
        let chains: Vec<Vec<Option<BulkFailure>>> = if concurrency <= 1 {
            vec![send_chain(roots.into_iter().collect()).await]
        } else {
            futures::stream::iter(
                roots
                    .into_iter()
                    .map(|batch| send_chain(VecDeque::from([batch]))),
            )
            .buffered(concurrency)
            .collect()
            .await
        };
        for chain in chains {
            for (slot, failure) in failures.iter_mut().zip(chain) {
                if slot.is_none() {
                    *slot = failure;
                }
            }
        }
        failures
    }
}

#[async_trait]
impl ReindexTarget for ElasticsearchBackend {
    /// A no-op, deliberately.
    ///
    /// For a SQL backend, search entries are rows that must be cleared before
    /// being rewritten or stale ones survive. For Elasticsearch the entries are
    /// *fields of the resource document*, and `write_search_entries` re-indexes
    /// that whole document under the same `_id`, which replaces it wholesale —
    /// no stale field can survive. Actually deleting here would remove the
    /// resource itself between the delete and the write, so the reindex would
    /// briefly (and, if it then failed, permanently) drop it from search.
    async fn delete_search_entries(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource_id: &str,
    ) -> StorageResult<u64> {
        Ok(0)
    }

    /// Rebuilds a page of resources in `_bulk` requests bounded by
    /// [`BULK_OPS_PER_REQUEST`] documents and the configured byte budget, under
    /// the rebuild's own refresh policy
    /// ([`ElasticsearchConfig::reindex_refresh`](super::backend::ElasticsearchConfig::reindex_refresh)).
    ///
    /// The default trait implementation walks the page one resource at a time,
    /// which against Elasticsearch is one HTTP round trip each and held
    /// `$reindex` to ~160 resources/s however fast the source read was — at
    /// which a corpus-sized rebuild cannot finish, so the deferred-indexing
    /// fast-load path had no usable recovery (#1021). SQLite already overrode
    /// this for the same reason, batching a page into one transaction; this is
    /// its Elasticsearch counterpart.
    ///
    /// Under `refresh=wait_for` the saving is larger still: that policy blocks
    /// each write until the next scheduled refresh, so per-document writes cost
    /// one refresh wait each while a bulk request costs one for the page.
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        if resources.is_empty() {
            return Vec::new();
        }
        let tenant_id = tenant.tenant_id().as_str();
        let extractor = self.tenant_extractor(tenant_id);

        // Every document each resource contributes, plus the value count its
        // successful outcome reports.
        struct Prepared {
            values: usize,
            docs: Vec<(String, String, Value)>,
            failure: Option<String>,
        }
        let mut types_touched: Vec<String> = Vec::new();
        let prepared: Vec<Prepared> = resources
            .iter()
            .map(|resource| {
                let resource_type = resource.resource_type();
                let id = resource.id();
                let content = resource.content();
                let fhir_version = resource.fhir_version();
                let extracted_values = match extractor.extract(content, resource_type) {
                    Ok(values) => values,
                    Err(e) => {
                        return Prepared {
                            values: 0,
                            docs: Vec::new(),
                            failure: Some(format!("Search parameter extraction failed: {e}")),
                        };
                    }
                };
                types_touched.push(resource_type.to_string());
                let mut docs = vec![(
                    self.index_name(tenant_id, resource_type),
                    Self::document_id(resource_type, id),
                    build_es_document(
                        tenant_id,
                        resource_type,
                        id,
                        resource.version_id(),
                        content,
                        fhir_version,
                        &extracted_values,
                    ),
                )];
                for contained in extractor.extract_contained(content) {
                    types_touched.push(contained.contained_type.clone());
                    docs.push((
                        self.index_name(tenant_id, &contained.contained_type),
                        Self::document_id(
                            &contained.contained_type,
                            &contained_resource_id(id, &contained.local_id),
                        ),
                        build_es_contained_document(
                            tenant_id,
                            resource_type,
                            id,
                            &contained.contained_type,
                            &contained.local_id,
                            &contained.content,
                            resource.version_id(),
                            fhir_version,
                            &contained.values,
                        ),
                    ));
                }
                Prepared {
                    values: extracted_values.len(),
                    docs,
                    failure: None,
                }
            })
            .collect();

        // Ensure every index touched exists, once each — not once per resource.
        let mut ensured = std::collections::HashSet::new();
        for ty in types_touched {
            if ensured.insert(ty.clone())
                && let Err(e) = schema::ensure_index(self, tenant_id, &ty).await
            {
                // Keep whether it was an outage: the rebuild retries an
                // unreachable cluster, not a rejected resource (#1125).
                let message = error_detail(&e);
                let unavailable = is_unavailable(&e);
                return resources
                    .iter()
                    .map(|_| {
                        Err(if unavailable {
                            unavailable_error(message.clone())
                        } else {
                            internal_error(message.clone())
                        })
                    })
                    .collect();
            }
        }

        let ops: Vec<(usize, &str, &str, &Value)> = prepared
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                p.docs
                    .iter()
                    .map(move |(index, doc_id, doc)| (i, index.as_str(), doc_id.as_str(), doc))
            })
            .collect();
        let failures = self
            .send_bulk_index(&ops, prepared.len(), self.reindex_refresh_param())
            .await;

        prepared
            .into_iter()
            .zip(failures)
            .map(|(p, failure)| match (p.failure, failure) {
                (Some(message), _) => Err(internal_error(message)),
                // `$reindex` retries a run only for transient failures, so a
                // rejection must not pass for an outage or the reverse.
                (None, Some(failure)) if failure.transient => {
                    Err(unavailable_error(failure.message))
                }
                (None, Some(failure)) => Err(internal_error(failure.message)),
                (None, None) => Ok(p.values),
            })
            .collect()
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = resource.resource_type();
        let resource_id = resource.id();
        let content = resource.content();
        let fhir_version = resource.fhir_version();

        let extracted_values = self
            .tenant_extractor(tenant_id)
            .extract(content, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {e}")))?;

        // The document carries version_id and fhir_version, neither of which is
        // recoverable from the resource JSON — which is why ReindexTarget hands
        // over the whole StoredResource rather than just its content.
        let doc = build_es_document(
            tenant_id,
            resource_type,
            resource_id,
            resource.version_id(),
            content,
            fhir_version,
            &extracted_values,
        );

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, resource_id);

        index_document(
            self,
            &index,
            &doc_id,
            &doc,
            self.write_refresh_param(),
            "index document",
        )
        .await?;

        self.index_contained_docs(
            tenant_id,
            resource_type,
            resource_id,
            content,
            fhir_version,
            resource.version_id(),
            true,
        )
        .await?;

        Ok(extracted_values.len())
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();

        // MUST be a delete-by-query with a `tenant_id` term filter, never a
        // delete of the indices matching the tenant's index pattern. The
        // pattern `{prefix}_{tenant}_*` is a prefix glob, so tenant "a" matches
        // tenant "a_b"'s indices — deleting by pattern would destroy a
        // separator-sharing tenant's data. The term filter is what actually
        // bounds this to one tenant; the pattern only narrows which indices to
        // scan. See `tenant_index_pattern` for why the over-match is deliberate.
        let pattern = tenant_index_pattern(self, tenant_id);
        delete_by_query_scoped(
            self,
            &pattern,
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } }
            ]}}}),
            "clear the search index",
        )
        .await
    }
}

#[async_trait]
impl ReindexSource for ElasticsearchBackend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let tenant_id = tenant.tenant_id().as_str();
        let pattern = tenant_index_pattern(self, tenant_id);

        let body = json!({
            "size": 0,
            "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } },
                { "term": { "is_deleted": false } }
            ]}},
            "aggs": { "types": { "terms": { "field": "resource_type", "size": 1000 } } }
        });

        // A tenant with no indices is an empty `200` (a wildcard that matches
        // nothing is allowed by default). A failed listing is an error, never
        // "no resource types": that would let a reindex finish successfully
        // having done nothing (#1364).
        let Some(body) = send_read_with_retry(self, ReadOp::Search, &pattern, body).await? else {
            return Ok(Vec::new());
        };

        Ok(body
            .pointer("/aggregations/types/buckets")
            .and_then(|b| b.as_array())
            .map(|buckets| {
                buckets
                    .iter()
                    .filter_map(|b| b.get("key").and_then(|k| k.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        ResourceStorage::count(self, tenant, Some(resource_type)).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        // `search_after` keyset pagination over a total sort order. Excludes
        // contained docs, which are synthetic and get rebuilt from their
        // container rather than being reindexed in their own right.
        let mut body = json!({
            "size": limit,
            "query": { "bool": {
                "filter": [
                    { "term": { "tenant_id": tenant_id } },
                    { "term": { "is_deleted": false } }
                ],
                "must_not": [ { "term": { "is_contained": true } } ]
            }},
            "sort": [
                { "last_updated": "asc" },
                { "resource_id": "asc" }
            ]
        });

        if let Some(raw) = cursor {
            let after: Value = serde_json::from_str(raw)
                .map_err(|e| internal_error(format!("Malformed reindex cursor: {e}")))?;
            body["search_after"] = after;
        }

        // Only a missing index is an empty page. A failed page is an error, never
        // "the last page": that would end the walk early and silently truncate
        // whatever is being rebuilt from it (#1364).
        let Some(payload) = send_read_with_retry(self, ReadOp::Search, &index, body).await? else {
            return Ok(ResourcePage {
                resources: Vec::new(),
                next_cursor: None,
                skipped: Vec::new(),
            });
        };

        let hits = payload
            .pointer("/hits/hits")
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();

        let mut resources = Vec::new();
        for hit in &hits {
            if let Some(source) = hit.get("_source")
                && let Some(stored) = parse_stored_resource(source, tenant)?
            {
                resources.push(stored);
            }
        }

        // Only a full page can have a successor; the sort values of the last hit
        // become the next `search_after`.
        let next_cursor = match (hits.len() as u32 == limit, hits.last()) {
            (true, Some(last)) => last
                .get("sort")
                .map(|s| s.to_string())
                .filter(|_| !resources.is_empty()),
            _ => None,
        };

        Ok(ResourcePage {
            resources,
            next_cursor,
            skipped: Vec::new(),
        })
    }
}

/// The index glob covering every one of a tenant's type indices.
///
/// This is a *prefix* glob, so it can over-match. The example previously given
/// here — tenant "a" matching tenant "ab" — was wrong: the pattern is
/// `{prefix}_a_*`, which requires the literal separator, so "ab" does not match.
/// The real case is the *underscore-bearing* one: tenant "a"'s `{prefix}_a_*`
/// does match tenant "a_b"'s `{prefix}_a_b_patient`.
///
/// That over-match is deliberate — `_` is kept in the encoder's safe set so that
/// `my_tenant`-shaped ids need no escaping and conforming deployments never see
/// an index rename (see [`super::naming`]). Every query built on this glob MUST
/// therefore also carry a `tenant_id` term filter: the pattern narrows which
/// indices are scanned, the filter is what enforces tenant isolation.
fn tenant_index_pattern(backend: &ElasticsearchBackend, tenant_id: &str) -> String {
    backend.tenant_index_pattern(tenant_id)
}

/// Runs a delete-by-query and returns how many documents it removed.
///
/// Missing indices are not an error — a tenant that has never been written to
/// simply has nothing to delete. Everything else that is not a completed
/// delete is: transient failures are retried ([`send_write_with_retry`]), and a
/// `200` that does not say how many documents it deleted is not a delete of `0`
/// (#1382). `what` names the delete in error messages.
async fn delete_by_query_scoped(
    backend: &ElasticsearchBackend,
    index: &str,
    body: Value,
    what: &str,
) -> StorageResult<u64> {
    match send_write_with_retry(backend, WriteOp::DeleteByQuery, index, &body, what).await? {
        WriteOutcome::Done(payload) => payload
            .get("deleted")
            .and_then(Value::as_u64)
            .ok_or_else(|| internal_error(format!("Failed to {what}: no deleted count"))),
        // Only a delete-by-id answers this.
        WriteOutcome::NotFound => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BULK_BACKOFF_BASE, BULK_BACKOFF_MAX, ValueOrigin, backoff_delay, build_es_document,
        chunk_ranges, describe_item_error, error_detail, es_index_date, is_transient_bulk_status,
        is_unavailable,
    };
    use crate::error::{BackendError, StorageError};
    use crate::search::converters::IndexValue;
    use crate::search::extractor::ExtractedValue;
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn index_date(raw: &str) -> Option<String> {
        let origin = ValueOrigin {
            resource_type: "Patient",
            resource_id: "p1",
            param: "birthdate",
        };
        es_index_date(origin, raw)
    }

    /// #1314: every FHIR date form is indexed as the UTC instant its range
    /// starts at, in the one spelling the mapping cannot misread.
    #[test]
    fn index_dates_are_complete_utc_instants() {
        for (raw, indexed) in [
            ("2024", "2024-01-01T00:00:00.000Z"),
            ("2024-03", "2024-03-01T00:00:00.000Z"),
            ("2024-03-15", "2024-03-15T00:00:00.000Z"),
            ("2024-03-15T10:30", "2024-03-15T10:30:00.000Z"),
            ("2024-03-15T10:30:45", "2024-03-15T10:30:45.000Z"),
            ("2024-03-15T10:30:45Z", "2024-03-15T10:30:45.000Z"),
            ("2024-03-15T10:30:45+05:30", "2024-03-15T05:00:45.000Z"),
            ("2024-03-15T22:30:45-04:00", "2024-03-16T02:30:45.000Z"),
            ("2024-03-15T10:30:45.1Z", "2024-03-15T10:30:45.100Z"),
            ("2024-03-15T10:30:45.123456Z", "2024-03-15T10:30:45.123Z"),
            ("2024-03-15T10:30:45.123456789Z", "2024-03-15T10:30:45.123Z"),
            // Elasticsearch rejected both of these, and the document with them.
            (
                "2024-03-15T10:30:45.1234567891Z",
                "2024-03-15T10:30:45.123Z",
            ),
            ("2016-12-31T23:59:60Z", "2017-01-01T00:00:00.000Z"),
            (" 2024-03-15 ", "2024-03-15T00:00:00.000Z"),
        ] {
            assert_eq!(index_date(raw).as_deref(), Some(indexed), "{raw}");
        }
    }

    /// ISO 8601 spellings outside the FHIR grammar that Elasticsearch indexed
    /// as written keep the instant they had.
    #[test]
    fn index_dates_elasticsearch_used_to_accept_keep_their_instant() {
        for (raw, indexed) in [
            ("2024-03-15T10", "2024-03-15T10:00:00.000Z"),
            ("2024-03-15T10Z", "2024-03-15T10:00:00.000Z"),
            ("2024-03-15T10+05:30", "2024-03-15T04:30:00.000Z"),
            ("2024-03-15T10:30:45+0530", "2024-03-15T05:00:45.000Z"),
            ("2024-03-15T10:30:45-05", "2024-03-15T15:30:45.000Z"),
            ("2024-03-15T10:30:45,123Z", "2024-03-15T10:30:45.123Z"),
            ("2024-03-15T", "2024-03-15T00:00:00.000Z"),
        ] {
            assert_eq!(index_date(raw).as_deref(), Some(indexed), "{raw}");
        }
    }

    #[test]
    fn index_dates_that_are_not_dates_are_skipped() {
        for raw in [
            "",
            "not-a-date",
            "Tuesday",
            "2024-02-30",
            "2024-13-01",
            "2024-03-15T25:00:00Z",
            "2024-03-15T24:00:00Z",
            "2024-03-15 10:30:45",
            "2024-03-15T10:30:45.Z",
            "2024-03-15T10:30:45+15:00",
            "2024-3-5",
            "0000-01-01",
            // Read by the mapping as epoch milliseconds: 1970.
            "20240315",
            "1710498645000",
            "2024-03-15T10:30:45+05:3é",
        ] {
            assert_eq!(index_date(raw), None, "{raw:?}");
        }
    }

    /// A bad date costs the document that one entry — top-level or inside a
    /// composite instance — and nothing else.
    #[test]
    fn document_leaves_out_only_the_unparseable_date() {
        let value = |param: &str, value: IndexValue, composite_group: Option<u32>| {
            let url = format!("http://hl7.org/fhir/SearchParameter/{param}");
            let extracted = ExtractedValue::new(param, url, value.param_type(), value);
            match composite_group {
                Some(group) => extracted.with_composite_group(group),
                None => extracted,
            }
        };
        let doc = build_es_document(
            "t1",
            "Patient",
            "p1",
            "1",
            &json!({ "resourceType": "Patient", "id": "p1" }),
            FhirVersion::default(),
            &[
                value("family", IndexValue::String("Smith".into()), None),
                value("birthdate", IndexValue::date("2024-02-30"), None),
                value(
                    "death-date",
                    IndexValue::date("2024-03-15T10:30:00+05:30"),
                    None,
                ),
                value("combo", IndexValue::date("not-a-date"), Some(0)),
                value("combo", IndexValue::String("kept".into()), Some(0)),
                value("combo", IndexValue::date("2016-12-31T23:59:60Z"), Some(1)),
            ],
        );

        let params = &doc["search_params"];
        assert_eq!(params["string"][0]["value"], "Smith");
        assert_eq!(
            params["date"],
            json!([{
                "name": "death-date",
                "value": "2024-03-15T05:00:00.000Z",
                "precision": "second",
            }])
        );
        assert_eq!(
            params["composite"],
            json!([
                { "name": "combo", "group_id": 0, "string": ["kept"] },
                { "name": "combo", "group_id": 1, "date": ["2017-01-01T00:00:00.000Z"] },
            ])
        );
    }

    #[test]
    fn chunk_ranges_caps_operations_per_request() {
        assert_eq!(chunk_ranges(&[1; 5], 2, usize::MAX), vec![0..2, 2..4, 4..5]);
        assert_eq!(chunk_ranges(&[1; 4], 2, usize::MAX), vec![0..2, 2..4]);
        assert!(chunk_ranges(&[], 2, usize::MAX).is_empty());
        // A zero operation cap still makes progress, one per request.
        assert_eq!(chunk_ranges(&[1; 2], 0, usize::MAX), vec![0..1, 1..2]);
    }

    #[test]
    fn chunk_ranges_caps_bytes_per_request() {
        // 40 + 40 fits 100, the third would make 120.
        assert_eq!(
            chunk_ranges(&[40, 40, 40, 40, 20], 500, 100),
            vec![0..2, 2..5]
        );
        // Exactly at the cap still fits.
        assert_eq!(chunk_ranges(&[50, 50, 50], 500, 100), vec![0..2, 2..3]);
    }

    /// The #1125 shape: a page of Provenance-sized documents must not become
    /// one ~54 MB request.
    #[test]
    fn chunk_ranges_keeps_large_documents_under_the_byte_cap() {
        let sizes = vec![108_000; 500];
        let cap = 10 * 1024 * 1024;
        let ranges = chunk_ranges(&sizes, 500, cap);
        assert!(ranges.len() > 1);
        assert_eq!(ranges.iter().map(|r| r.len()).sum::<usize>(), 500);
        for range in &ranges {
            assert!(
                sizes[range.clone()].iter().sum::<usize>() <= cap,
                "{range:?}"
            );
        }
    }

    #[test]
    fn chunk_ranges_sends_an_oversized_document_alone_rather_than_dropping_it() {
        assert_eq!(
            chunk_ranges(&[10, 500, 10, 10], 500, 100),
            vec![0..1, 1..2, 2..4]
        );
    }

    #[test]
    fn bulk_statuses_that_ask_for_a_retry_are_transient() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_transient_bulk_status(status), "{status}");
        }
        // 400 is how Elasticsearch rejects a document over the nested-object
        // limit (#1050); rerunning it changes nothing.
        for status in [200, 201, 400, 404, 409, 413] {
            assert!(!is_transient_bulk_status(status), "{status}");
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(backoff_delay(1), BULK_BACKOFF_BASE);
        assert_eq!(backoff_delay(2), BULK_BACKOFF_BASE * 2);
        assert_eq!(backoff_delay(3), BULK_BACKOFF_BASE * 4);
        assert_eq!(backoff_delay(40), BULK_BACKOFF_MAX);
    }

    /// Elasticsearch puts the nested-object limit in `caused_by`; a message
    /// that stopped at the top-level reason would say only "failed to parse".
    #[test]
    fn item_errors_name_their_type_reason_and_cause() {
        let error = json!({
            "type": "document_parsing_exception",
            "reason": "[1:2] failed to parse",
            "caused_by": {
                "type": "illegal_argument_exception",
                "reason": "The number of nested documents has exceeded the allowed limit of [10000]."
            }
        });
        assert_eq!(
            describe_item_error(&error),
            "document_parsing_exception: [1:2] failed to parse; caused by \
             illegal_argument_exception: The number of nested documents has exceeded the allowed limit of [10000]."
        );
        assert_eq!(
            describe_item_error(&json!({"type": "version_conflict_engine_exception"})),
            "version_conflict_engine_exception"
        );
        assert_eq!(describe_item_error(&json!("plain")), "\"plain\"");
    }

    #[test]
    fn a_page_wide_error_keeps_the_detail_unavailable_does_not_display() {
        let outage = StorageError::Backend(BackendError::Unavailable {
            backend_name: "elasticsearch".to_string(),
            message: "Failed to check index existence for hfs_t_patient: timed out".to_string(),
        });
        assert!(is_unavailable(&outage));
        assert!(error_detail(&outage).contains("timed out"));

        let rejection = StorageError::Backend(BackendError::Internal {
            backend_name: "elasticsearch".to_string(),
            message: "mapper_parsing_exception".to_string(),
            source: None,
        });
        assert!(!is_unavailable(&rejection));
        assert!(error_detail(&rejection).contains("mapper_parsing_exception"));
    }
}
