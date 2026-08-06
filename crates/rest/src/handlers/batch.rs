//! Batch and transaction processing handler.
//!
//! Implements the FHIR [batch/transaction interaction](https://hl7.org/fhir/http.html#transaction):
//! `POST [base]` with a Bundle of type "batch" or "transaction"

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::stream::{self, StreamExt};
use helios_audit::{AuditAction, AuditCorrelation, AuditEventBuilder};
use helios_auth::{FhirOperation, Principal, SmartScopePolicy};
use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, ResourceStorage,
    bundle_if_match_gate,
};
use helios_persistence::error::{ResourceError, StorageError, TransactionError};
use serde_json::Value;
use tracing::{debug, error, warn};

use crate::error::{RestError, RestResult, create_operation_outcome};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::handlers::extract_patient_from_resource;
use crate::middleware::prefer::PreferHeader;
use crate::state::AppState;

/// Handler for batch/transaction processing.
///
/// Processes a Bundle of type "batch" or "transaction".
///
/// # HTTP Request
///
/// `POST [base]`
///
/// # Request Body
///
/// A Bundle resource with type "batch" or "transaction" containing entries
/// with request information.
///
/// # Response
///
/// Returns a Bundle of type "batch-response" or "transaction-response"
/// with the results of each operation.
///
/// # Batch vs Transaction
///
/// - **Batch**: Each entry is processed independently. Failures don't affect other entries.
/// - **Transaction**: All entries are processed atomically. Any failure rolls back all changes.
pub async fn batch_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    prefer: PreferHeader,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + BundleProvider + helios_persistence::core::SearchProvider + Send + Sync,
{
    // Extract the Principal from request extensions (set by auth middleware).
    // If present, per-entry scope checks will be enforced.
    let principal = request.extensions().get::<Principal>().cloned();

    // One bundle, one version: every entry the bundle creates or updates is
    // stamped with the request's negotiated version, exactly as a
    // single-resource endpoint would stamp it (#350).
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);

    // Parse the body as JSON
    let bundle: Value = serde_json::from_slice(
        &axum::body::to_bytes(request.into_body(), state.config().max_body_size)
            .await
            .map_err(|_| RestError::BadRequest {
                message: "Failed to read request body".to_string(),
            })?,
    )?;
    // Validate it's a Bundle
    let resource_type = bundle
        .get("resourceType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RestError::BadRequest {
            message: "Request must be a Bundle resource".to_string(),
        })?;

    if resource_type != "Bundle" {
        return Err(RestError::BadRequest {
            message: format!("Expected Bundle, got {}", resource_type),
        });
    }

    // Get Bundle type
    let bundle_type =
        bundle
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RestError::BadRequest {
                message: "Bundle must have a type".to_string(),
            })?;

    match bundle_type {
        "batch" => {
            process_batch(
                &state,
                tenant,
                fhir_version,
                &prefer,
                &bundle,
                principal.as_ref(),
            )
            .await
        }
        "transaction" => {
            process_transaction(
                &state,
                tenant,
                fhir_version,
                &prefer,
                &bundle,
                principal.as_ref(),
            )
            .await
        }
        _ => Err(RestError::BadRequest {
            message: format!(
                "Bundle type must be 'batch' or 'transaction', got '{}'",
                bundle_type
            ),
        }),
    }
}

/// Hard ceiling on batch entry concurrency, independent of configuration.
///
/// Caps the damage a backend could do by returning an absurd
/// [`ResourceStorage::bulk_write_concurrency`], and bounds a `ServerConfig`
/// built programmatically without going through `validate()`.
const MAX_BATCH_CONCURRENCY: usize = 64;

/// Resolves how many entries of this bundle may execute at once.
///
/// The backend states its own tolerance via
/// [`ResourceStorage::bulk_write_concurrency`] — SQLite keeps the default of 1
/// (a single writer behind synchronous rusqlite, whose storage calls contain no
/// await points, so they could not interleave regardless), PostgreSQL, MongoDB
/// and Elasticsearch declare 8, S3 declares 32, and a composite delegates to
/// its primary. `HFS_BATCH_MAX_CONCURRENCY` caps that answer; it never raises
/// it, because only the backend knows what its pool absorbs.
///
/// The floor of 1 is load-bearing: `buffered(0)` never polls its inner futures,
/// so a zero bound would hang the request until the timeout — precisely the
/// symptom this bound exists to remove.
fn batch_concurrency<S>(state: &AppState<S>, entries: &[Value]) -> usize
where
    S: ResourceStorage + Send + Sync,
{
    // A StructureDefinition written by entry i is folded into the tenant
    // profile registry by `upsert_stored_profile` (the POST and PUT arms of
    // `process_batch_entry`) before entry i+1's `check_write` resolves against
    // it. That read-your-writes is the only cross-entry dependency on this
    // path, and it is a server-side conformance side effect rather than a
    // resource read, so FHIR's "entries are independent" does not sanction
    // racing it. Fall back to today's exact semantics for exactly the bundles
    // that rely on it.
    //
    // Keyed off `request.url` through the same `parse_request_url` the side
    // effect itself keys off, so the scan and the write cannot disagree. Since
    // #503 that parse strips the query, so `StructureDefinition?url=…` matches
    // here where it did not before. Such an entry is refused as conditional
    // before it writes, which makes the clamp conservative rather than
    // load-bearing — but the scan and the write still agree, which is the
    // invariant this is keyed for.
    //
    // NOTE: extend this scan in lockstep with any new cross-entry
    // `state.validation()` mutation added to `process_batch_entry`.
    let writes_conformance = entries.iter().any(|entry| {
        entry
            .get("request")
            .and_then(|request| request.get("url"))
            .and_then(Value::as_str)
            .and_then(|url| parse_request_url(url).ok())
            .is_some_and(|(resource_type, _)| resource_type == "StructureDefinition")
    });
    if writes_conformance {
        return 1;
    }

    state
        .storage()
        .bulk_write_concurrency()
        .min(state.config().batch_max_concurrency)
        .clamp(1, MAX_BATCH_CONCURRENCY)
}

/// Records how far a batch got, and says so if the handler future is dropped.
///
/// On expiry the `TimeoutLayer` (`crate::lib`) drops the handler without
/// propagating an error and manufactures an empty-bodied 408, so the
/// batch-response Bundle naming the entries that committed is discarded before
/// the client ever sees it. `Drop` still runs; this is the only place that can
/// leave a trace of what landed.
struct BatchProgress {
    total: usize,
    completed: AtomicUsize,
    bundle_id: String,
    finished: bool,
}

impl BatchProgress {
    fn new(total: usize, bundle_id: String) -> Self {
        Self {
            total,
            completed: AtomicUsize::new(0),
            bundle_id,
            finished: false,
        }
    }

    fn record(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for BatchProgress {
    fn drop(&mut self) {
        if !self.finished {
            warn!(
                completed = self.completed.load(Ordering::Relaxed),
                total = self.total,
                bundle_id = %self.bundle_id,
                "Batch abandoned before completion (request timed out or client \
                 disconnected). Entries already written are durable and were not \
                 rolled back; where auditing is enabled, their events carry this \
                 bundle-id."
            );
        }
    }
}

/// Processes a batch Bundle.
async fn process_batch<S>(
    state: &AppState<S>,
    tenant: TenantExtractor,
    fhir_version: FhirVersion,
    prefer: &PreferHeader,
    bundle: &Value,
    principal: Option<&Principal>,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!(
        tenant = %tenant.tenant_id(),
        "Processing batch request"
    );
    let correlation = AuditCorrelation::new("batch");

    let entries = bundle
        .get("entry")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let base_url = state.base_url();
    let concurrency = batch_concurrency(state, &entries);
    let mut progress = BatchProgress::new(entries.len(), correlation.bundle_id.clone());

    // Re-borrow the owned locals. The per-entry closure is `FnMut`, so it can
    // only capture things it may reproduce on every call — shared references
    // are `Copy`, so they qualify while the values themselves would not.
    //
    // `buffered` polls its futures in place on this task and never spawns, so
    // nothing here needs `'static` or an `Arc` clone, and dropping the handler
    // drops every in-flight entry synchronously.
    let entries_ref = &entries;
    let tenant = &tenant;
    let correlation = &correlation;
    let progress_ref = &progress;

    // Entries are independent per the FHIR spec ("the server may process the
    // entries in any order"), so they run with bounded concurrency. The stream
    // is driven over indices rather than over `entries.iter()` deliberately:
    // a closure whose returned future borrows its *argument* needs a
    // higher-ranked lifetime that inference cannot supply here, and the
    // resulting error is reported against the route registration in
    // `routing::fhir_routes` rather than against this function.
    //
    // `buffered` — NOT `buffer_unordered` — is backed by `FuturesOrdered` and
    // yields in submission order, so response entry i answers request entry i
    // by construction.
    let results: Vec<(usize, Value)> = stream::iter(0..entries_ref.len())
        .map(|index| async move {
            let entry = &entries_ref[index];
            let result =
                process_batch_entry(state, tenant, fhir_version, entry, index, principal).await;

            // Audit is emitted inside the entry future rather than after
            // collection. `emit_batch_entry_audit` hands off to a detached
            // task and carries position as an explicit `entry-index` detail,
            // so completion-order emission costs nothing — and it means an
            // entry whose write committed before a timeout still gets its
            // event, which post-collection emission would drop for the whole
            // bundle.
            let correlation_details = EntryAuditCorrelation::from_bundle(correlation, index);
            emit_batch_entry_audit(
                state,
                entry,
                &result,
                principal,
                None,
                Some(&correlation_details),
            );

            progress_ref.record();
            (
                index,
                bundle_entry_result_to_json(&result, base_url, prefer),
            )
        })
        .buffered(concurrency)
        .collect()
        .await;

    // The positional contract is guaranteed by the combinator; assert it rather
    // than trust it. Nothing in the response entry carries an index, so a
    // regression here would be invisible to every existing test and to most
    // clients.
    debug_assert!(
        results
            .iter()
            .enumerate()
            .all(|(position, (index, _))| position == *index),
        "batch response entries must remain positional"
    );

    let response_entries: Vec<Value> = results.into_iter().map(|(_, entry)| entry).collect();
    progress.finished = true;

    let response_bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "batch-response",
        "entry": response_entries
    });

    debug!(
        entries = response_entries.len(),
        concurrency, "Batch processing completed"
    );

    Ok((StatusCode::OK, Json(response_bundle)).into_response())
}

/// Processes a transaction Bundle.
///
/// Transactions are atomic - all entries succeed or all fail.
/// Per the FHIR specification, entries are processed in this order:
/// 1. DELETE operations
/// 2. POST (create) operations
/// 3. PUT/PATCH (update) operations
/// 4. GET operations
async fn process_transaction<S>(
    state: &AppState<S>,
    tenant: TenantExtractor,
    fhir_version: FhirVersion,
    prefer: &PreferHeader,
    bundle: &Value,
    principal: Option<&Principal>,
) -> RestResult<Response>
where
    S: ResourceStorage + BundleProvider + helios_persistence::core::SearchProvider + Send + Sync,
{
    debug!(
        tenant = %tenant.tenant_id(),
        "Processing transaction request"
    );
    let correlation = AuditCorrelation::new("transaction");

    let json_entries = bundle
        .get("entry")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Parse entries and track their original indices for response ordering
    let mut indexed_entries: Vec<(usize, BundleEntry, Option<String>)> =
        Vec::with_capacity(json_entries.len());

    for (index, entry) in json_entries.iter().enumerate() {
        match parse_bundle_entry(entry) {
            Ok((bundle_entry, full_url)) => {
                // Entry URLs reach the backends unparsed, and every backend's
                // `parse_url` splits on `/` alone and takes the last two
                // segments — sqlite, postgres and mongodb carry byte-equivalent
                // copies. A query string therefore lands in storage as part of
                // the resource type or the id: `PUT Patient?identifier=http://…`
                // commits a row typed `Patient?identifier=http:`, and
                // `PUT Patient/123?_format=json` commits one whose id is
                // `123?_format=json`. `PUT Patient?name=peter` yields a single
                // segment and fails the whole bundle with a message about the
                // URL format instead. Decline here, before anything executes, so
                // the bundle is declined intact (#503).
                //
                // GET is exempt — but not because this path resolves searches.
                // It does not: a GET entry still reaches the backend's
                // `parse_url`, and a query-bearing one still fails there. The
                // exemption keeps this guard off the arm #478 is rewriting, so
                // that work lands on an untouched dispatch path instead of
                // merging against a refusal it is about to replace.
                //
                // `ifNoneExist` is left alone too — MongoDB resolves it inside
                // the session, so refusing it here would remove a working,
                // atomic feature. Resolving URL criteria within a transaction's
                // atomic scope is #511.
                if !matches!(bundle_entry.method, BundleMethod::Get)
                    && bundle_entry.url.contains('?')
                {
                    return Err(RestError::NotSupported {
                        feature: format!(
                            "Transaction entry {} ({} {}) carries a query string. This \
                             server cannot resolve one inside a transaction's atomic \
                             scope, so no entries were applied. Submit it in a batch \
                             Bundle, or address the instance directly.",
                            index,
                            bundle_method_to_http_method(&bundle_entry.method),
                            bundle_entry.url
                        ),
                    });
                }

                // Enforce per-entry scope authorization for transactions.
                // Transactions are atomic so any denied entry rejects the whole bundle.
                if let Some(principal) = principal {
                    let (resource_type, _) = parse_request_url(&bundle_entry.url).map_err(|e| {
                        // `value`, not its parent `invalid`: `request.url` is
                        // present and its value cannot be used. Its batch twin
                        // makes the same choice, so the arms classify it
                        // identically (#504).
                        RestError::InvalidElementValue {
                            message: format!("Entry {}: {}", index, e),
                        }
                    })?;
                    let operation = bundle_method_to_fhir_operation(&bundle_entry.method);
                    SmartScopePolicy::check(principal, &resource_type, operation).map_err(
                        |_| RestError::Forbidden {
                            message: format!(
                                "Insufficient scope for {} on {} (transaction entry {})",
                                operation, resource_type, index
                            ),
                        },
                    )?;
                }
                // Decline PATCH before anything executes, at the same 501 the
                // batch arm returns and all three backends already return from
                // inside the transaction. Today such a bundle executes its
                // earlier entries, hits the backend's 501, rolls back, and
                // surfaces as a generic "Transaction failed at entry N" — the
                // status the client sees never mentions PATCH. Raised here, the
                // bundle is declined intact and says why.
                if matches!(bundle_entry.method, BundleMethod::Patch) {
                    return Err(RestError::NotImplemented {
                        feature: format!("PATCH in a Bundle entry (transaction entry {index})"),
                    });
                }

                indexed_entries.push((index, bundle_entry, full_url));
            }
            Err(e) => {
                // For transactions, any parse error fails the whole bundle.
                // Rendered through the error itself rather than flattened to a
                // 400: a HEAD entry is 405 here exactly as it is per-entry in a
                // batch, which is the agreement #502 asks for.
                return Err(e.into_rest_error(index));
            }
        }
    }

    // Conditional references (`Type?query`) resolve against the server's
    // content before anything executes, per the transaction processing rules:
    // exactly one match rewrites the reference to `Type/id`; zero or several
    // fail the bundle (#459). They used to be stored verbatim — unsearchable
    // and unresolvable. References to entries created by this same bundle use
    // `fullUrl`s, which the storage layer resolves during processing.
    resolve_conditional_references(state, &tenant, &mut indexed_entries).await?;

    // Write-path validation: transactions are atomic, so any invalid write
    // entry rejects the whole bundle before anything executes.
    for (index, entry, _) in &indexed_entries {
        if !matches!(entry.method, BundleMethod::Post | BundleMethod::Put) {
            continue;
        }
        let Some(resource) = &entry.resource else {
            continue;
        };
        let (resource_type, _) =
            parse_request_url(&entry.url).map_err(|e| RestError::InvalidElementValue {
                message: format!("Entry {}: {}", index, e),
            })?;
        state
            .validation()
            .check_write(tenant.tenant_id(), fhir_version, &resource_type, resource)
            .await?;
    }

    // Sort by processing order: DELETE -> POST -> PUT/PATCH -> GET
    indexed_entries.sort_by_key(|(_, entry, _)| method_processing_order(&entry.method));

    // Build the entries list for processing, setting full_url on each entry
    let entries_for_processing: Vec<BundleEntry> = indexed_entries
        .iter()
        .cloned()
        .map(|(_, mut entry, full_url)| {
            entry.full_url = full_url;
            entry
        })
        .collect();

    // Call the persistence layer
    let result = state
        .storage()
        .process_transaction(tenant.context(), entries_for_processing, fhir_version)
        .await;

    match result {
        Ok(bundle_result) => {
            // Stored StructureDefinitions feed the tenant's profile
            // registry. The request content is what was stored (modulo
            // server-assigned id/meta, which the converter does not read).
            for (_, entry, _) in &indexed_entries {
                if matches!(entry.method, BundleMethod::Post | BundleMethod::Put)
                    && let Some(resource) = &entry.resource
                    && resource.get("resourceType").and_then(Value::as_str)
                        == Some("StructureDefinition")
                {
                    state.validation().upsert_stored_profile(
                        tenant.tenant_id(),
                        fhir_version,
                        resource,
                    );
                }
            }

            // Reorder results back to original entry order
            let mut ordered_results: Vec<(usize, &BundleEntry, &BundleEntryResult)> =
                indexed_entries
                    .iter()
                    .zip(bundle_result.entries.iter())
                    .map(|((orig_idx, entry, _), result)| (*orig_idx, entry, result))
                    .collect();
            ordered_results.sort_by_key(|(idx, _, _)| *idx);

            for (orig_idx, entry, result) in &ordered_results {
                let correlation_details =
                    EntryAuditCorrelation::from_bundle(&correlation, *orig_idx);
                emit_transaction_entry_audit(
                    state,
                    entry,
                    result,
                    principal,
                    None,
                    Some(&correlation_details),
                );
            }

            let base_url = state.base_url();
            let response_entries: Vec<Value> = ordered_results
                .into_iter()
                .map(|(_, _, result)| bundle_entry_result_to_json(result, base_url, prefer))
                .collect();

            let response_bundle = serde_json::json!({
                "resourceType": "Bundle",
                "type": "transaction-response",
                "entry": response_entries
            });

            debug!(
                entries = response_entries.len(),
                "Transaction processing completed successfully"
            );

            Ok((StatusCode::OK, Json(response_bundle)).into_response())
        }
        Err(e) => {
            // Derive a sanitized reason so backend detail carried by a
            // rolled-back/internal transaction error never reaches the client
            // response, the audit trail, or the entry outcome. The raw detail is
            // preserved server-side by the `error!` log below.
            //
            // The status and the code come from the same triple as the reason,
            // so this synthetic per-entry result cannot contradict the
            // whole-bundle response it accompanies — a rollback is `transient`
            // and retryable, where the hardcoded `processing` it carried meant
            // "there is no point resubmitting the same content unchanged".
            // Audit-only: the client gets `transaction_error_to_response(e)`
            // below, so nothing here is observable on the wire (#504).
            let (rollback_status, rollback_code, rollback_reason) =
                transaction_error_response_parts(&e);
            let rollback_result = BundleEntryResult::error(
                rollback_status.as_u16(),
                create_operation_outcome(
                    "error",
                    rollback_code,
                    &format!("Transaction rolled back: {rollback_reason}"),
                ),
            );
            for (orig_idx, entry, _) in &indexed_entries {
                let correlation_details =
                    EntryAuditCorrelation::from_bundle(&correlation, *orig_idx);
                emit_transaction_entry_audit(
                    state,
                    entry,
                    &rollback_result,
                    principal,
                    Some(&rollback_reason),
                    Some(&correlation_details),
                );
            }
            error!(error = %e, "Transaction failed");
            transaction_error_to_response(e)
        }
    }
}

/// Evaluates a batch entry's `ifMatch` precondition against stored state.
///
/// Returns `Some` when the entry must not proceed — either the 412 the gate
/// produced, or a storage error rendered as an entry result. Returns `None`
/// when there was no precondition to check, or it was satisfied.
///
/// `ifMatch` is a list, satisfied when any listed tag matches (#311), and `*`
/// requires a current representation — so a supplied `ifMatch` against an
/// absent or deleted resource fails rather than silently creating.
///
/// **This is a read-then-write check, not an atomic compare-and-swap.** The
/// backends reach an atomic re-check through `update_with_match`, which lives on
/// [`VersionedStorage`] — a trait the FHIR router does not bound `S` with, so
/// this path cannot call it. The window is the same one `handlers::update`
/// already carries for single-resource updates, with one addition worth naming:
/// entries within a bundle now run concurrently, so two entries carrying
/// `ifMatch` for the same id can both pass this gate and both write.
///
/// [`VersionedStorage`]: helios_persistence::core::VersionedStorage
async fn check_entry_if_match<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    resource_type: &str,
    id: &str,
    if_match: Option<&str>,
) -> Option<BundleEntryResult>
where
    S: ResourceStorage + Send + Sync,
{
    // Entries that send no precondition pay nothing — not even the read.
    if_match?;

    let current = match state
        .storage()
        .read(tenant.context(), resource_type, id)
        .await
    {
        Ok(current) => current,
        // A deleted resource has no current representation, which is a failed
        // precondition rather than a storage error — the same mapping
        // `handlers::update` and the backends' own batch arms make.
        Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
        Err(e) => return Some(entry_storage_failure(e)),
    };

    bundle_if_match_gate(if_match, current.as_ref().map(|r| r.version_id()))
}

/// Processes a single batch entry, returning a structured BundleEntryResult.
async fn process_batch_entry<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    fhir_version: FhirVersion,
    entry: &Value,
    index: usize,
    principal: Option<&Principal>,
) -> BundleEntryResult
where
    S: ResourceStorage + Send + Sync,
{
    let request = match entry.get("request") {
        Some(r) => r,
        None => {
            return entry_failure(missing_request(index));
        }
    };

    // Resolved through the shared seam, so this arm and the transaction arm
    // accept exactly the same set of codes and refuse the rest with the same
    // status (#502). It runs before the URL parse below, so an entry that is
    // wrong in both ways now reports the method rather than the URL.
    let method = match parse_entry_method(request) {
        Ok(method) => method,
        Err(refusal) => {
            return entry_failure(refusal.into_rest_error(index));
        }
    };
    // An absent `request.url` is a cardinality violation (1..1), reported as
    // `required` — distinct from a url that is present and unusable, which
    // `parse_request_url` refuses below as `value`. Splitting them also closes
    // a divergence: an absent url was caught by `parse_bundle_entry` on the
    // transaction arm and, one line later, by `unwrap_or("")` here, so the two
    // arms described the same entry differently (#504).
    let Some(url) = request.get("url").and_then(|v| v.as_str()) else {
        return entry_failure(missing_url(index));
    };
    let if_match = request.get("ifMatch").and_then(|v| v.as_str());

    // Parse the URL to extract resource type and ID
    let (resource_type, id) = match parse_request_url(url) {
        Ok(parsed) => parsed,
        Err(e) => {
            return entry_failure(RestError::InvalidElementValue { message: e });
        }
    };

    // Enforce per-entry scope authorization
    if let Some(principal) = principal {
        // The same enum-typed table the transaction arm uses. The raw-string
        // copy this replaced ended in `_ => FhirOperation::Read`, which was only
        // safe while an unsupported method was caught further down — with the
        // catch-all gone, a method that slipped through would have been
        // authorized as a read and then executed as whatever it was.
        let operation = bundle_method_to_fhir_operation(&method);
        if SmartScopePolicy::check(principal, &resource_type, operation).is_err() {
            // `forbidden` is a child of `security`; `processing` is not an
            // ancestor of it in any supported version, so a client filtering
            // `code is-a security` to trigger re-auth or re-consent saw a
            // false negative on this denial and only this one (#504).
            return entry_failure(RestError::Forbidden {
                message: format!(
                    "Insufficient scope for {} on {} (batch entry {})",
                    operation, resource_type, index
                ),
            });
        }
    }

    // A query on a type-level URL is FHIR conditional criteria, and this path
    // cannot resolve one — that needs a search, which is #511. Refuse it
    // explicitly rather than dispatch something else: before #503 the criteria
    // rode along in `resource_type`, so a conditional PUT reached
    // `create_or_update` with an empty id and wrote a row no search can address.
    //
    // GET is exempt. A query there is a search rather than a condition, and
    // executing it is #478's deliverable; leaving the arm untouched keeps this
    // fix off that diff.
    //
    // Since #502 the predicate is enum-typed, matching its transaction twin. As
    // a raw `method != "GET"` this was the third case-sensitive comparison in
    // the file: a lowercase `get` on a search URL failed it and was refused as a
    // conditional interaction. Such an entry is now refused at the seam and
    // never reaches here. `{method}` below renders the canonical spelling rather
    // than echoing raw client bytes.
    if !matches!(method, BundleMethod::Get)
        && let Some(criteria) = conditional_criteria(url, &id)
    {
        // `NotSupported` passes `feature` through verbatim, so the message
        // survives byte-for-byte while the code moves to `not-supported` — the
        // pair the transaction twin already returns for the same entry.
        return entry_failure(RestError::NotSupported {
            feature: format!(
                "Conditional interactions are not supported in Bundle entries \
                 (entry {index}: {method} {url}). Criteria were not applied and \
                 nothing was written. Address the instance directly, or perform \
                 the conditional interaction against the resource endpoint. \
                 Criteria: {criteria}"
            ),
        });
    }

    match method {
        BundleMethod::Get => {
            // Read operation
            match state
                .storage()
                .read(tenant.context(), &resource_type, &id)
                .await
            {
                Ok(Some(stored)) => BundleEntryResult::ok(stored),
                // The one expression this PR changes on the arm #478/#481 is
                // rewriting. `not-found` is the only issue-type code whose
                // definition names HTTP 404, and all three backends already
                // emit it for the byte-identical condition inside their own
                // transaction executors — this entry was the outlier.
                Ok(None) => entry_failure(RestError::NotFound {
                    resource_type: resource_type.clone(),
                    id: id.clone(),
                }),
                Err(e) => entry_storage_failure(e),
            }
        }
        BundleMethod::Post => {
            // Create operation
            let resource = match entry.get("resource") {
                Some(r) => r.clone(),
                None => {
                    // `invalid`, not `required`: `Bundle.entry.resource` is
                    // 0..1, and only R5/R6's bdl-3c makes it mandatory for a
                    // POST/PUT/PATCH entry. R4 and R4B have no equivalent —
                    // bdl-5 is satisfied by a request-only entry — so a single
                    // call site serving four versions must not claim a rule
                    // half of them do not have.
                    return entry_failure(RestError::BadRequest {
                        message: "POST entry missing resource".to_string(),
                    });
                }
            };

            // Write-path validation (per-entry outcome in batch semantics).
            //
            // The error carries the validator's own multi-issue outcome —
            // per-issue code, severity and `expression` — and
            // `client_outcome` surfaces it verbatim. It used to be flattened
            // to a joined string of `details.text` and re-wrapped under
            // `processing`, which is the lossiest case in #504.
            if let Err(e) = state
                .validation()
                .check_write(tenant.tenant_id(), fhir_version, &resource_type, &resource)
                .await
            {
                return entry_failure(e);
            }

            match state
                .storage()
                .create(tenant.context(), &resource_type, resource, fhir_version)
                .await
            {
                Ok(stored) => {
                    if resource_type == "StructureDefinition" {
                        state.validation().upsert_stored_profile(
                            tenant.tenant_id(),
                            fhir_version,
                            stored.content(),
                        );
                    }
                    BundleEntryResult::created(stored)
                }
                Err(e) => entry_storage_failure(e),
            }
        }
        BundleMethod::Put => {
            // Update operation
            let resource = match entry.get("resource") {
                Some(r) => r.clone(),
                None => {
                    // See the POST arm: `invalid` rather than `required`,
                    // because R4 and R4B do not require the element.
                    return entry_failure(RestError::BadRequest {
                        message: "PUT entry missing resource".to_string(),
                    });
                }
            };

            // `PUT Patient` names no instance to update. Left to fall through it
            // reaches `create_or_update` with an empty id, and that writes a row
            // rather than rejecting: the backend inserts `"id": ""` into the
            // resource before delegating to `create`, whose id fallback fires on
            // an absent id, not an empty one. Every later such entry then reads
            // that row back and overwrites it (#503).
            if id.is_empty() {
                // `value`: the element is present and its value cannot address
                // what the method needs. Its sibling guard on DELETE makes the
                // same choice.
                return entry_failure(RestError::InvalidElementValue {
                    message: "PUT entry request.url must address an instance ('[type]/[id]')"
                        .to_string(),
                });
            }

            // Ahead of validation, because every backend evaluates `ifMatch`
            // first: a stale precondition carrying an invalid body is a 412,
            // not a 422.
            if let Some(failure) =
                check_entry_if_match(state, tenant, &resource_type, &id, if_match).await
            {
                return failure;
            }

            // Write-path validation (per-entry outcome in batch semantics).
            if let Err(e) = state
                .validation()
                .check_write(tenant.tenant_id(), fhir_version, &resource_type, &resource)
                .await
            {
                return entry_failure(e);
            }

            match state
                .storage()
                .create_or_update(
                    tenant.context(),
                    &resource_type,
                    &id,
                    resource,
                    fhir_version,
                )
                .await
            {
                Ok((stored, created)) => {
                    if resource_type == "StructureDefinition" {
                        state.validation().upsert_stored_profile(
                            tenant.tenant_id(),
                            fhir_version,
                            stored.content(),
                        );
                    }
                    if created {
                        BundleEntryResult::created(stored)
                    } else {
                        // For updates, include location with versioned URL
                        let mut result = BundleEntryResult::ok(stored);
                        result.location = Some(format!("{}/{}", resource_type, id));
                        result
                    }
                }
                // Also closes a divergence internal to the batch arm: an
                // optimistic-lock failure surfacing here now answers 412 +
                // `conflict`, the pair the `ifMatch` gate above already
                // answers, where it used to answer 412 + `processing`.
                Err(e) => entry_storage_failure(e),
            }
        }
        BundleMethod::Delete => {
            // Mirror of the PUT guard above. FHIR defines no unconditional
            // type-level delete, and an empty id would otherwise target the
            // empty-id row a pre-#503 conditional PUT could have written.
            if id.is_empty() {
                return entry_failure(RestError::InvalidElementValue {
                    message: "DELETE entry request.url must address an instance ('[type]/[id]')"
                        .to_string(),
                });
            }

            // Honour `ifMatch` on DELETE: a client asking to delete only the
            // version it reviewed must not destroy a concurrent amendment.
            if let Some(failure) =
                check_entry_if_match(state, tenant, &resource_type, &id, if_match).await
            {
                return failure;
            }

            // Delete operation
            match state
                .storage()
                .delete(tenant.context(), &resource_type, &id)
                .await
            {
                Ok(()) => BundleEntryResult::deleted(),
                Err(e) => entry_storage_failure(e),
            }
        }
        // Declined rather than dispatched, matching the transaction arm and all
        // three backends, which already return 501 for a bundle PATCH. A
        // bundle entry carries no Content-Type, and `parse_patch_format`
        // derives the patch format entirely from it, so there is nothing here
        // to dispatch on; R4 designates FHIRPath Patch as the bundle format and
        // `apply_patch` does not implement it. Tracked by #502's follow-up.
        //
        // `NotImplemented` wraps its `feature` as "Feature '…' is not
        // implemented", so this message changes shape while keeping its
        // guidance. The arm label stays: an audit trail wants to know which
        // arm refused, and the transaction twin identifies itself the same way.
        BundleMethod::Patch => entry_failure(RestError::NotImplemented {
            feature: format!(
                "PATCH in a Bundle entry (batch entry {index}); nothing was applied — \
                 send the patch to the instance endpoint instead, PATCH [base]/[type]/[id]"
            ),
        }),
        // No catch-all: the match is exhaustive over `BundleMethod`, so adding a
        // variant is a compile error here rather than a silent 405. Codes
        // outside the value set never reach this point — `parse_entry_method`
        // refuses them at the top of this function.
    }
}

#[derive(Debug, Clone)]
struct EntryAuditCorrelation {
    bundle_id: String,
    bundle_type: String,
    entry_index: usize,
}

impl EntryAuditCorrelation {
    fn from_bundle(correlation: &AuditCorrelation, entry_index: usize) -> Self {
        Self {
            bundle_id: correlation.bundle_id.clone(),
            bundle_type: correlation.bundle_type.clone(),
            entry_index,
        }
    }
}

/// Emits an audit event for a processed batch entry.
fn emit_batch_entry_audit<S>(
    state: &AppState<S>,
    entry: &Value,
    result: &BundleEntryResult,
    principal: Option<&Principal>,
    rollback_reason: Option<&str>,
    correlation: Option<&EntryAuditCorrelation>,
) where
    S: ResourceStorage + Send + Sync,
{
    let request = match entry.get("request") {
        Some(request) => request,
        None => return,
    };
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let url = request.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let request_resource = entry.get("resource");
    emit_entry_audit(
        state,
        method,
        url,
        request_resource,
        result,
        principal,
        rollback_reason,
        correlation,
    );
}

/// Emits an audit event for a processed transaction entry.
fn emit_transaction_entry_audit<S>(
    state: &AppState<S>,
    entry: &BundleEntry,
    result: &BundleEntryResult,
    principal: Option<&Principal>,
    rollback_reason: Option<&str>,
    correlation: Option<&EntryAuditCorrelation>,
) where
    S: ResourceStorage + Send + Sync,
{
    emit_entry_audit(
        state,
        bundle_method_to_http_method(&entry.method),
        &entry.url,
        entry.resource.as_ref(),
        result,
        principal,
        rollback_reason,
        correlation,
    );
}

/// Builds and records an audit event for a bundle entry result.
#[allow(clippy::too_many_arguments)]
fn emit_entry_audit<S>(
    state: &AppState<S>,
    method: &str,
    url: &str,
    request_resource: Option<&Value>,
    result: &BundleEntryResult,
    principal: Option<&Principal>,
    rollback_reason: Option<&str>,
    correlation: Option<&EntryAuditCorrelation>,
) where
    S: ResourceStorage + Send + Sync,
{
    let Some(sink) = state.audit_sink() else {
        return;
    };

    let action = method_to_audit_action(method);
    let outcome = if rollback_reason.is_some() || result.status >= 400 {
        "8"
    } else {
        "0"
    };

    let parsed = parse_request_url(url).ok();
    let mut resource_type = parsed
        .as_ref()
        .map(|(rt, _)| rt.clone())
        .unwrap_or_default();
    let mut resource_id = parsed
        .as_ref()
        .and_then(|(_, id)| (!id.is_empty()).then_some(id.clone()));

    if let Some(resource) = result.resource.as_ref().or(request_resource) {
        if let Some(rt) = resource.get("resourceType").and_then(|v| v.as_str()) {
            resource_type = rt.to_string();
        }
        if let Some(id) = resource.get("id").and_then(|v| v.as_str()) {
            resource_id = Some(id.to_string());
        }
    }

    let patient_ref = result
        .resource
        .as_ref()
        .and_then(|resource| {
            let rt = resource
                .get("resourceType")
                .and_then(|v| v.as_str())
                .unwrap_or(&resource_type);
            extract_patient_from_resource(rt, resource)
        })
        .or_else(|| {
            request_resource.and_then(|resource| {
                let rt = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&resource_type);
                extract_patient_from_resource(rt, resource)
            })
        });

    let mut builder = AuditEventBuilder::new(state.audit_source_observer())
        .action(action)
        .outcome(outcome);

    if let Some(reason) = rollback_reason {
        builder = builder.outcome_desc(format!("Transaction rolled back: {reason}"));
    } else if let Some(desc) = extract_outcome_description(result.outcome.as_ref()) {
        builder = builder.outcome_desc(desc);
    }

    if let Some(id) = resource_id.as_deref()
        && !resource_type.is_empty()
    {
        builder = builder.resource(&resource_type, id);
    }
    if let Some(correlation) = correlation {
        builder = builder
            .detail("bundle-id", &correlation.bundle_id)
            .detail("bundle-type", &correlation.bundle_type)
            .detail("entry-index", correlation.entry_index.to_string());
    }

    if let Some(patient_ref) = patient_ref {
        builder = builder.patient(patient_ref);
    }
    if let Some(principal) = principal {
        builder = builder.agent(principal.subject(), None, true);
    }

    let sink = Arc::clone(sink);
    let event = builder.build();
    tokio::spawn(async move {
        sink.record(event).await;
    });
}

fn method_to_audit_action(method: &str) -> AuditAction {
    match method {
        "GET" => AuditAction::Read,
        "POST" => AuditAction::Create,
        "PUT" | "PATCH" => AuditAction::Update,
        "DELETE" => AuditAction::Delete,
        _ => AuditAction::Execute,
    }
}

fn bundle_method_to_http_method(method: &BundleMethod) -> &'static str {
    match method {
        BundleMethod::Get => "GET",
        BundleMethod::Post => "POST",
        BundleMethod::Put => "PUT",
        BundleMethod::Patch => "PATCH",
        BundleMethod::Delete => "DELETE",
    }
}

fn extract_outcome_description(outcome: Option<&Value>) -> Option<String> {
    outcome
        .and_then(|value| value.get("issue"))
        .and_then(|issues| issues.as_array())
        .and_then(|issues| issues.first())
        .and_then(|issue| issue.get("details"))
        .and_then(|details| details.get("text"))
        .and_then(|text| text.as_str())
        .map(ToString::to_string)
}

/// Parses a request URL to extract resource type and optional ID.
///
/// The query string is split off **before** the path is parsed. FHIR conditional
/// criteria routinely contain `/` — the spec's own transaction example carries
/// `Patient?identifier=http:/example.org/fhir/ids|456456` — so splitting the raw
/// URL on `/` first folds the criteria into the resource type, and the caller
/// then addresses storage with a type like `Patient?identifier=http:` (#503).
///
/// Empty segments are dropped rather than yielded, so a leading `/` and the
/// `[type]/?[criteria]` form that `http.html` prints for conditional delete both
/// reduce to the type alone instead of producing an empty id.
///
/// The query itself is deliberately not returned. Callers refuse conditional
/// entries via [`conditional_criteria`]; resolving them is #511.
fn parse_request_url(url: &str) -> Result<(String, String), String> {
    let path = url.split_once('?').map_or(url, |(path, _)| path);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());

    // Unlike the previous `Vec`-and-`match` shape, this arm is reachable: an
    // absent or empty `request.url` used to parse as the resource type `""`,
    // which the POST arm then created a row under.
    let resource_type = segments
        .next()
        .ok_or_else(|| "Entry request.url is empty".to_string())?;

    // `Patient/123/_history/1` addresses `Patient/123`; anything past the id
    // qualifies that address rather than extending it.
    Ok((
        resource_type.to_string(),
        segments.next().unwrap_or_default().to_string(),
    ))
}

/// Returns the conditional criteria an entry URL carries, if any.
///
/// A query on a **type-level** URL (`Patient?identifier=x`) is FHIR conditional
/// criteria. A query on an **instance** URL (`Patient/123?_format=json`) is a
/// control parameter — the entry addresses a known resource either way — so it
/// is not reported here.
///
/// A bare `Patient?` carries no criteria and is not conditional; treating it as
/// one would match every resource of the type.
fn conditional_criteria<'a>(url: &'a str, id: &str) -> Option<&'a str> {
    if !id.is_empty() {
        return None;
    }
    url.split_once('?')
        .map(|(_, query)| query)
        .filter(|query| !query.is_empty())
}

/// Why a bundle entry's `request.method` was refused.
///
/// The refusal carries its own status so the batch and transaction arms cannot
/// disagree about it. Batch renders it as a per-entry response and transaction
/// as the whole-bundle error, but the status is decided once, here — which is
/// the divergence #502 is about.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryMethodRefusal {
    /// `request.method` is absent, or is not a JSON string.
    Missing,
    /// Present, but not an `http-verb` code. Carries the raw spelling so the
    /// message can show the client exactly what was sent.
    NotCanonical(String),
    /// `HEAD` — a legal `http-verb` code this server does not accept in a Bundle.
    Head,
}

impl EntryMethodRefusal {
    /// Renders the refusal as the error **both** arms report.
    ///
    /// #515 gave this type a `status()` beside this function and pinned the two
    /// with a test — agreement by hand. There is now one function, so the
    /// status *and* the issue code are decided once, by the [`RestError`] this
    /// produces: the batch arm wraps it with [`entry_failure`] and the
    /// transaction arm returns it as the whole-bundle error (#504).
    ///
    /// The per-variant text was previously a separate `message()`, whose `Head`
    /// arm this function computed and then discarded — which is why the batch
    /// arm printed HEAD guidance the transaction arm never showed.
    fn into_rest_error(self, index: usize) -> RestError {
        match self {
            // 405 + `not-supported`. HEAD is a legal `http-verb` code, so the
            // entry is well-formed instance data and it is the interaction the
            // server does not implement inside a Bundle. The guidance rides in
            // `resource_type` because `MethodNotAllowed` renders
            // "Method {method} not allowed on {resource_type}" and has no
            // detail slot — that is how both arms come to print it.
            Self::Head => RestError::MethodNotAllowed {
                method: "HEAD".to_string(),
                resource_type: format!(
                    "a Bundle entry (entry {index}) — use GET, or send HEAD to the \
                     instance endpoint directly"
                ),
            },
            // 400 + `required`: `Bundle.entry.request.method` is 1..1 in every
            // supported version, so an absent code is a cardinality violation
            // rather than an unusable value.
            Self::Missing => RestError::MissingElement {
                message: format!("Entry {index}: request.method is required"),
            },
            // 400 + `value`: the element is present and its value fails the
            // required binding. Deliberately not `code-invalid`, which names
            // that mechanism precisely but is a **child of `processing`** —
            // emitting it would move a malformed-instance failure back into the
            // branch #504 exists to escape.
            Self::NotCanonical(raw) => RestError::InvalidElementValue {
                message: format!(
                    "Entry {index}: '{raw}' is not an http-verb code. \
                     Bundle.entry.request.method is a code with a required binding to \
                     http://hl7.org/fhir/ValueSet/http-verb, and FHIR codes are \
                     case-sensitive — use GET, POST, PUT, PATCH or DELETE."
                ),
            },
        }
    }
}

/// `Bundle.entry.request` is absent.
///
/// Shared by both arms so the batch entry outcome and the whole-bundle
/// transaction error carry one message and one code. `request` is 0..1 in the
/// StructureDefinition, but it is mandatory for these bundle types — `bdl-3` in
/// R4/R4B, and transitively `bdl-3c` in R5/R6, which requires
/// `request.method.exists()`. The invariant key stays out of the message: the
/// handler serves all four versions and `bdl-3` does not exist in R5 or R6.
fn missing_request(index: usize) -> RestError {
    RestError::MissingElement {
        message: format!(
            "Entry {index}: request is required — a batch or transaction entry must carry it."
        ),
    }
}

/// `Bundle.entry.request.url` is absent.
///
/// Distinct from a url that is present but names nothing (`""`, `"/"`,
/// `"?identifier=x"`), which [`parse_request_url`] refuses as `value`.
fn missing_url(index: usize) -> RestError {
    RestError::MissingElement {
        message: format!("Entry {index}: request.url is required (it is 1..1)."),
    }
}

/// Parses a bundle entry's `request.method` into a [`BundleMethod`].
///
/// **This is the only `&str` -> `BundleMethod` table in this crate.** Both the
/// batch and the transaction arm go through it, which is the point: they used
/// to carry two independently-written matchers that disagreed, so the same
/// Bundle succeeded as a `transaction` and failed as a `batch` (#502).
///
/// The match is deliberately **case-sensitive**. `Bundle.entry.request.method`
/// is a `code` with a *required* binding to `http://hl7.org/fhir/ValueSet/http-verb`,
/// whose concepts are `caseSensitive: true` and uppercase in every FHIR version
/// this server supports. A lowercase `"post"` is therefore invalid instance
/// data, not a valid entry a strict server wrongly rejects — so the previous
/// `to_uppercase()` on the transaction path was the non-conformant matcher, and
/// removing it is the fix rather than copying it across.
fn parse_entry_method(request: &Value) -> Result<BundleMethod, EntryMethodRefusal> {
    let Some(raw) = request.get("method").and_then(Value::as_str) else {
        return Err(EntryMethodRefusal::Missing);
    };

    match raw {
        "GET" => Ok(BundleMethod::Get),
        "POST" => Ok(BundleMethod::Post),
        "PUT" => Ok(BundleMethod::Put),
        "PATCH" => Ok(BundleMethod::Patch),
        "DELETE" => Ok(BundleMethod::Delete),
        // A legal code, but one no bundle arm implements. HEAD *is* served on
        // the instance-read route; it is Bundle entries it is refused in.
        "HEAD" => Err(EntryMethodRefusal::Head),
        _ => Err(EntryMethodRefusal::NotCanonical(raw.to_string())),
    }
}

/// Why a bundle entry could not be parsed at all.
///
/// Split from a bare `String` so the method refusal keeps its status across the
/// transaction boundary; flattening it there is what would re-create #502's
/// divergence in a new place.
#[derive(Debug)]
enum EntryParseError {
    Method(EntryMethodRefusal),
    /// `Bundle.entry.request` is absent.
    MissingRequest,
    /// `Bundle.entry.request.url` is absent.
    MissingUrl,
}

impl EntryParseError {
    fn into_rest_error(self, index: usize) -> RestError {
        match self {
            Self::Method(refusal) => refusal.into_rest_error(index),
            // Both arms now reach the same helper, so an absent element is
            // described once rather than by whichever arm noticed it first.
            Self::MissingRequest => missing_request(index),
            Self::MissingUrl => missing_url(index),
        }
    }
}

/// Renders a failed Bundle entry.
///
/// **Replaces `create_error_result`,** which hardcoded `"code": "processing"`
/// across nineteen call sites, so a scope denial, a missing resource, a
/// malformed entry and an unsupported method were distinguishable only by
/// `response.status` and free-text English (#504).
///
/// The status and the OperationOutcome both come from
/// [`RestError::client_outcome`] — the same function `impl IntoResponse for
/// RestError` uses — so a per-entry outcome and the single-resource response
/// for the identical failure are produced by one mapping rather than by two
/// kept in step by review. That is what makes the two describable as the same
/// error rather than two errors that happen to agree.
fn entry_failure(err: RestError) -> BundleEntryResult {
    let (status, outcome) = err.client_outcome();
    BundleEntryResult::error(status.as_u16(), outcome)
}

/// Renders a storage error as a failed Bundle entry.
///
/// **Replaces `entry_error`,** which called `client_response()`, bound the
/// correct FHIR issue code to `_code` and discarded it, after which
/// `create_error_result` stamped `processing` over the result. Deleting that
/// one underscore-binding corrects five call sites by construction.
///
/// The sanitizing behaviour is unchanged and is now strictly stronger: the code
/// comes from the same sanitized triple as the message, so it cannot classify
/// an error more specifically than the message is permitted to describe it.
fn entry_storage_failure(err: StorageError) -> BundleEntryResult {
    entry_failure(RestError::from(err))
}

/// Returns HTTP status text for a status code.
fn status_text(code: &str) -> &'static str {
    match code {
        "200" => "OK",
        "201" => "Created",
        "204" => "No Content",
        "400" => "Bad Request",
        "401" => "Unauthorized",
        "403" => "Forbidden",
        "404" => "Not Found",
        "405" => "Method Not Allowed",
        "406" => "Not Acceptable",
        "409" => "Conflict",
        "410" => "Gone",
        "412" => "Precondition Failed",
        "415" => "Unsupported Media Type",
        "422" => "Unprocessable Entity",
        "500" => "Internal Server Error",
        "501" => "Not Implemented",
        _ => "Unknown",
    }
}

/// Parses a bundle entry from JSON into a BundleEntry struct.
///
/// Returns the BundleEntry and optionally the fullUrl for reference resolution.
/// Resolves conditional references (`Type?query`) in the bundle's resources
/// against the server's content, per the transaction processing rules:
/// exactly one match rewrites the reference to `Type/id`, zero or several
/// fail the bundle (#459). They used to pass through into storage verbatim,
/// where nothing can search or resolve them.
async fn resolve_conditional_references<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    indexed_entries: &mut [(usize, BundleEntry, Option<String>)],
) -> RestResult<()>
where
    S: ResourceStorage + helios_persistence::core::SearchProvider + Send + Sync,
{
    use std::collections::HashMap;

    // Collect every distinct conditional reference first: bundles repeat the
    // same one heavily (every Synthea entry names its location), and each
    // lookup is a search.
    let mut conditionals: HashMap<String, Option<String>> = HashMap::new();
    for (_, entry, _) in indexed_entries.iter() {
        if let Some(resource) = &entry.resource {
            collect_conditional_references(resource, &mut conditionals);
        }
    }
    if conditionals.is_empty() {
        return Ok(());
    }

    for (reference, resolved) in conditionals.iter_mut() {
        let (resource_type, query_string) =
            reference.split_once('?').expect("collected with a '?'");
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query_string.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let registry = state.storage().search_param_registry(tenant.context());
        let mut query = {
            let registry = registry.read();
            crate::extractors::build_search_query_from_pairs(resource_type, &pairs, &registry)
                .map_err(|e| RestError::BadRequest {
                    message: format!(
                        "Conditional reference '{reference}' is not a valid search: {e}"
                    ),
                })?
        };
        // Two is enough to prove the match is not unique.
        query.count = Some(2);
        let result = state
            .storage()
            .search(tenant.context(), &query)
            .await
            .map_err(RestError::from)?;
        match result.resources.items.as_slice() {
            [only] => {
                *resolved = Some(format!("{}/{}", only.resource_type(), only.id()));
            }
            [] => {
                return Err(RestError::BadRequest {
                    message: format!(
                        "Conditional reference '{reference}' matches no existing resource"
                    ),
                });
            }
            _ => {
                return Err(RestError::BadRequest {
                    message: format!(
                        "Conditional reference '{reference}' matches more than one resource"
                    ),
                });
            }
        }
    }

    for (_, entry, _) in indexed_entries.iter_mut() {
        if let Some(resource) = &mut entry.resource {
            rewrite_conditional_references(resource, &conditionals);
        }
    }
    Ok(())
}

/// Whether a reference literal is a conditional reference (`Type?query`).
fn is_conditional_reference(reference: &str) -> bool {
    match reference.split_once('?') {
        Some((head, query)) => {
            !head.is_empty()
                && !query.is_empty()
                && head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && head.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// Walks a resource collecting conditional `reference` literals.
fn collect_conditional_references(
    value: &Value,
    out: &mut std::collections::HashMap<String, Option<String>>,
) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("reference")
                && is_conditional_reference(reference)
            {
                out.entry(reference.clone()).or_insert(None);
            }
            for v in map.values() {
                collect_conditional_references(v, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_conditional_references(item, out);
            }
        }
        _ => {}
    }
}

/// Rewrites collected conditional `reference` literals to their resolutions.
fn rewrite_conditional_references(
    value: &mut Value,
    resolved: &std::collections::HashMap<String, Option<String>>,
) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("reference")
                && let Some(Some(target)) = resolved.get(reference)
            {
                map.insert("reference".to_string(), Value::String(target.clone()));
            }
            for v in map.values_mut() {
                rewrite_conditional_references(v, resolved);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                rewrite_conditional_references(item, resolved);
            }
        }
        _ => {}
    }
}

fn parse_bundle_entry(entry: &Value) -> Result<(BundleEntry, Option<String>), EntryParseError> {
    let request = entry
        .get("request")
        .ok_or(EntryParseError::MissingRequest)?;

    // Was an independently-written `to_uppercase()` ladder — the second of the
    // two matchers #502 is about. It no longer case-folds: `request.method` is a
    // `code` with a required binding, and folding it was the only thing standing
    // between invalid instance data and a real write. The refusal keeps its
    // status across this boundary so the whole-bundle error the caller raises
    // agrees with the per-entry result the batch arm would produce.
    let method = parse_entry_method(request).map_err(EntryParseError::Method)?;

    let url = request
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or(EntryParseError::MissingUrl)?
        .to_string();

    let resource = entry.get("resource").cloned();
    let full_url = entry
        .get("fullUrl")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Parse conditional headers
    let if_match = request
        .get("ifMatch")
        .and_then(|v| v.as_str())
        .map(String::from);
    let if_none_match = request
        .get("ifNoneMatch")
        .and_then(|v| v.as_str())
        .map(String::from);
    let if_none_exist = request
        .get("ifNoneExist")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok((
        BundleEntry {
            method,
            url,
            resource,
            if_match,
            if_none_match,
            if_none_exist,
            full_url: None, // Will be set later
        },
        full_url,
    ))
}

/// Maps a [`BundleMethod`] to a [`FhirOperation`] for scope checking.
fn bundle_method_to_fhir_operation(method: &BundleMethod) -> FhirOperation {
    match method {
        BundleMethod::Get => FhirOperation::Read,
        BundleMethod::Post => FhirOperation::Create,
        BundleMethod::Put | BundleMethod::Patch => FhirOperation::Update,
        BundleMethod::Delete => FhirOperation::Delete,
    }
}

/// Returns a processing order for bundle methods per FHIR spec.
/// DELETE (0) -> POST (1) -> PUT/PATCH (2) -> GET (3)
fn method_processing_order(method: &BundleMethod) -> u8 {
    match method {
        BundleMethod::Delete => 0,
        BundleMethod::Post => 1,
        BundleMethod::Put | BundleMethod::Patch => 2,
        BundleMethod::Get => 3,
    }
}

/// Converts a BundleEntryResult to JSON for the response bundle.
fn bundle_entry_result_to_json(
    result: &BundleEntryResult,
    base_url: &str,
    prefer: &PreferHeader,
) -> Value {
    let mut response = serde_json::Map::new();

    let status_code = result.status.to_string();
    let status_str = format!("{} {}", status_code, status_text(&status_code));
    response.insert("status".to_string(), Value::String(status_str));

    if let Some(ref location) = result.location {
        response.insert("location".to_string(), Value::String(location.clone()));
    }

    if let Some(ref etag) = result.etag {
        response.insert("etag".to_string(), Value::String(etag.clone()));
    }

    if let Some(ref last_modified) = result.last_modified {
        response.insert(
            "lastModified".to_string(),
            Value::String(last_modified.clone()),
        );
    }

    // Place outcome in response.outcome (not entry.resource)
    if let Some(ref outcome) = result.outcome {
        response.insert("outcome".to_string(), outcome.clone());
    }

    let mut entry = serde_json::Map::new();

    // Include resource based on Prefer header
    if let Some(ref resource) = result.resource {
        match prefer.return_preference() {
            Some("minimal") => {
                // Omit resource body
            }
            Some("OperationOutcome") => {
                // Return an OperationOutcome instead of the resource
                let outcome = serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "information",
                        "code": "informational",
                        "details": {
                            "text": format!("Operation completed with status {}", result.status)
                        }
                    }]
                });
                entry.insert("resource".to_string(), outcome);
            }
            _ => {
                // Default: return=representation — include the resource
                entry.insert("resource".to_string(), resource.clone());
            }
        }
    }

    // Build fullUrl from location or resource content
    if let Some(full_url) = build_full_url(result, base_url) {
        entry.insert("fullUrl".to_string(), Value::String(full_url));
    }

    entry.insert("response".to_string(), Value::Object(response));

    Value::Object(entry)
}

/// Builds the fullUrl for a response entry.
///
/// Uses the location (stripping the _history suffix) or falls back to
/// extracting resourceType/id from the resource content.
fn build_full_url(result: &BundleEntryResult, base_url: &str) -> Option<String> {
    // Try to derive from location (e.g., "Patient/123/_history/1" -> base_url/Patient/123)
    if let Some(ref location) = result.location {
        let resource_url = if let Some(idx) = location.find("/_history/") {
            &location[..idx]
        } else {
            location.as_str()
        };
        return Some(format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            resource_url
        ));
    }

    // Fall back to resource content
    if let Some(ref resource) = result.resource {
        let resource_type = resource.get("resourceType").and_then(|v| v.as_str());
        let id = resource.get("id").and_then(|v| v.as_str());
        if let (Some(rt), Some(id)) = (resource_type, id) {
            return Some(format!("{}/{}/{}", base_url.trim_end_matches('/'), rt, id));
        }
    }

    None
}

/// Computes the sanitized `(status, issue code, message)` for a failed
/// transaction.
///
/// Status codes and issue codes preserve the FHIR mapping used for the overall
/// transaction response. Only the rolled-back case is sanitized: its `reason`
/// can embed raw backend/driver/SQL detail, so it is collapsed to a generic
/// message (the raw detail is logged separately by the caller). Validation,
/// conditional-match, timeout, and not-supported errors keep their specific,
/// non-sensitive message.
fn transaction_error_response_parts(err: &TransactionError) -> (StatusCode, &'static str, String) {
    match err {
        TransactionError::BundleError { index, message } => (
            StatusCode::BAD_REQUEST,
            "processing",
            format!("Transaction failed at entry {}: {}", index, message),
        ),
        TransactionError::RolledBack { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "transient",
            "The transaction could not be completed and was rolled back.".to_string(),
        ),
        // 504, not 500: the backend is healthy and deliberately stopped work
        // that exceeded its time budget. Kept in step with
        // `From<TransactionError> for RestError`, so a transaction timeout
        // reports the same status whether it surfaces through this bundle path
        // or the single-resource one (issue #353).
        TransactionError::Timeout { timeout_ms } => (
            StatusCode::GATEWAY_TIMEOUT,
            "timeout",
            format!("Transaction timed out after {}ms", timeout_ms),
        ),
        TransactionError::MultipleMatches { operation, count } => (
            StatusCode::PRECONDITION_FAILED,
            "multiple-matches",
            format!("Conditional {} matched {} resources", operation, count),
        ),
        TransactionError::InvalidTransaction => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "exception",
            "Transaction is no longer valid".to_string(),
        ),
        TransactionError::NestedNotSupported => (
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            "Nested transactions are not supported".to_string(),
        ),
        TransactionError::UnsupportedIsolationLevel { level } => (
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            format!("Isolation level '{}' is not supported", level),
        ),
        // 501 + `not-supported`, matching the two sibling capability gaps
        // above. This is a property of the configured storage backend, not of
        // the request: the same bundle succeeds against a PostgreSQL or MongoDB
        // deployment. The message names `batch` because that is the actionable
        // alternative — it carries no atomicity requirement and every backend
        // supports it (#489).
        //
        // Raised before any entry is written, so a client that retries finds
        // the server in exactly the state it left it.
        TransactionError::AtomicityUnsupported { backend_name } => (
            StatusCode::NOT_IMPLEMENTED,
            "not-supported",
            format!(
                "The configured storage backend ('{}') cannot guarantee the all-or-nothing \
                 semantics a transaction Bundle requires, so no entries were applied. Submit \
                 the entries as a batch Bundle if partial success is acceptable, or use a \
                 backend with transaction support. This server's CapabilityStatement lists \
                 the interactions it supports.",
                backend_name
            ),
        ),
    }
}

/// Converts a TransactionError to an HTTP response with OperationOutcome.
///
/// The twin of [`RestError::client_outcome`] for the one error type that is not
/// a [`RestError`]: it builds the outcome through the same
/// `create_operation_outcome` every other error in this crate uses, rather than
/// carrying its own `json!` literal.
fn transaction_error_to_response(err: TransactionError) -> RestResult<Response> {
    let (status_code, issue_code, message) = transaction_error_response_parts(&err);
    let outcome = create_operation_outcome("error", issue_code, &message);
    Ok((status_code, Json(outcome)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    use async_trait::async_trait;
    use helios_audit::AuditSink;
    use helios_fhir::FhirVersion;
    use helios_fhir::r4::{AuditEvent, AuditEventEntityDetailValue};
    use helios_persistence::error::StorageResult;
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;
    use tokio::sync::Mutex;

    struct MockStorage;

    #[async_trait]
    impl ResourceStorage for MockStorage {
        fn backend_name(&self) -> &'static str {
            "mock"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!()
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    struct CollectorSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    #[async_trait]
    impl AuditSink for CollectorSink {
        async fn record(&self, event: AuditEvent) {
            self.events.lock().await.push(event);
        }

        async fn flush(&self) {}

        fn name(&self) -> &str {
            "collector"
        }
    }

    fn detail_map(event: &AuditEvent) -> HashMap<String, String> {
        let mut details = HashMap::new();
        for entity in event.entity.as_ref().into_iter().flatten() {
            for detail in entity.detail.as_ref().into_iter().flatten() {
                let Some(key) = detail.r#type.value.clone() else {
                    continue;
                };
                let value = match &detail.value {
                    Some(AuditEventEntityDetailValue::String(s)) => {
                        s.value.clone().unwrap_or_default()
                    }
                    _ => String::new(),
                };
                details.insert(key, value);
            }
        }
        details
    }

    /// The first issue of an entry result's outcome.
    fn entry_issue(result: &BundleEntryResult) -> &Value {
        &result
            .outcome
            .as_ref()
            .expect("a failed entry carries an outcome")["issue"][0]
    }

    #[test]
    fn entry_storage_failure_sanitizes_backend_detail() {
        // A backend/internal storage error whose Display embeds sensitive DB
        // detail (table/column names, SQL fragments) must be collapsed to the
        // generic client message with a 5xx status.
        use helios_persistence::error::BackendError;

        let raw_detail = "query execution failed: table \"resources\" column x does not exist";
        let err = StorageError::Backend(BackendError::QueryError {
            message: raw_detail.to_string(),
        });

        let result = entry_storage_failure(err);
        assert_eq!(result.status, 500);

        let issue = entry_issue(&result);
        // Threading the code strengthens this guarantee rather than diluting
        // it: the code now comes from the same sanitized `client_response`
        // triple as the message, so it cannot classify the error more
        // specifically than the message is permitted to describe it (#504).
        assert_eq!(issue["code"], "exception");

        let message = issue["details"]["text"].as_str().unwrap();
        assert!(
            !message.contains("resources"),
            "entry outcome leaked raw backend detail: {message}"
        );
        assert!(
            !message.contains("column x"),
            "entry outcome leaked raw backend detail: {message}"
        );
        assert_eq!(
            message,
            "An internal error occurred while processing the request."
        );
    }

    #[test]
    fn entry_storage_failure_preserves_not_found() {
        // Safe error classes keep their specific message and correct status.
        use helios_persistence::error::ResourceError;

        let err = StorageError::Resource(ResourceError::NotFound {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
        });

        let result = entry_storage_failure(err);
        assert_eq!(result.status, 404);

        let issue = entry_issue(&result);
        assert_eq!(issue["code"], "not-found");
        let message = issue["details"]["text"].as_str().unwrap();
        assert!(message.contains("Patient/123"), "message was: {message}");
    }

    #[test]
    fn test_transaction_rollback_reason_is_sanitized() {
        // The rolled-back reason from the persistence layer can carry backend
        // detail; it must not appear in the transaction response/audit text.
        let raw_detail = "connection failed to postgres: password authentication failed";
        let err = TransactionError::RolledBack {
            reason: raw_detail.to_string(),
        };
        let (status, _code, message) = transaction_error_response_parts(&err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !message.contains("password"),
            "rollback text leaked raw backend detail: {message}"
        );
        assert!(
            !message.contains("postgres"),
            "leaked backend name: {message}"
        );
    }

    #[test]
    fn test_transaction_error_response_parts_maps_every_variant() {
        // Exhaustively map each variant to its (status, code) so a future variant
        // or a re-mapped status is caught. Complements
        // `test_transaction_rollback_reason_is_sanitized`, which covers RolledBack.
        let cases: Vec<(TransactionError, StatusCode, &str)> = vec![
            (
                TransactionError::BundleError {
                    index: 2,
                    message: "boom".to_string(),
                },
                StatusCode::BAD_REQUEST,
                "processing",
            ),
            // 504 since #353 — a backend that stopped over-budget work is not
            // reporting a server defect.
            (
                TransactionError::Timeout { timeout_ms: 1500 },
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
            ),
            (
                TransactionError::MultipleMatches {
                    operation: "update".to_string(),
                    count: 3,
                },
                StatusCode::PRECONDITION_FAILED,
                "multiple-matches",
            ),
            (
                TransactionError::InvalidTransaction,
                StatusCode::INTERNAL_SERVER_ERROR,
                "exception",
            ),
            (
                TransactionError::NestedNotSupported,
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
            ),
            (
                TransactionError::UnsupportedIsolationLevel {
                    level: "serializable".to_string(),
                },
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
            ),
        ];

        for (err, want_status, want_code) in cases {
            let (status, code, message) = transaction_error_response_parts(&err);
            assert_eq!(status, want_status, "status for {err:?}");
            assert_eq!(code, want_code, "code for {err:?}");
            assert!(!message.is_empty(), "message for {err:?} must be non-empty");
        }

        // Detail-bearing variants surface their specifics in the message text.
        let (_, _, msg) =
            transaction_error_response_parts(&TransactionError::Timeout { timeout_ms: 1500 });
        assert!(msg.contains("1500"), "timeout message: {msg}");
        let (_, _, msg) =
            transaction_error_response_parts(&TransactionError::UnsupportedIsolationLevel {
                level: "serializable".to_string(),
            });
        assert!(msg.contains("serializable"), "isolation message: {msg}");
    }

    /// A failed entry carries the issue code the single-resource endpoint
    /// would return for the identical `RestError`.
    ///
    /// This is #504's whole claim in one table. Every row is a `RestError` the
    /// batch arm now constructs, and the pair asserted is the one
    /// [`RestError::client_outcome`] produces — the same function
    /// `impl IntoResponse for RestError` uses to build an HTTP body.
    #[test]
    fn entry_failure_renders_the_single_resource_mapping() {
        let cases: Vec<(RestError, u16, &str)> = vec![
            (
                RestError::MissingElement {
                    message: "m".to_string(),
                },
                400,
                "required",
            ),
            (
                RestError::InvalidElementValue {
                    message: "m".to_string(),
                },
                400,
                "value",
            ),
            (
                RestError::BadRequest {
                    message: "m".to_string(),
                },
                400,
                "invalid",
            ),
            (
                RestError::NotSupported {
                    feature: "m".to_string(),
                },
                400,
                "not-supported",
            ),
            (
                RestError::Forbidden {
                    message: "m".to_string(),
                },
                403,
                "forbidden",
            ),
            (
                RestError::NotFound {
                    resource_type: "Patient".to_string(),
                    id: "ghost".to_string(),
                },
                404,
                "not-found",
            ),
            (
                RestError::MethodNotAllowed {
                    method: "HEAD".to_string(),
                    resource_type: "a Bundle entry".to_string(),
                },
                405,
                "not-supported",
            ),
            (
                RestError::Gone {
                    resource_type: "Patient".to_string(),
                    id: "p1".to_string(),
                },
                410,
                "deleted",
            ),
            (
                RestError::PreconditionFailed {
                    message: "m".to_string(),
                },
                412,
                "conflict",
            ),
            (
                RestError::NotImplemented {
                    feature: "m".to_string(),
                },
                501,
                "not-supported",
            ),
            (
                RestError::ServiceUnavailable {
                    message: "m".to_string(),
                },
                503,
                "transient",
            ),
            (
                RestError::InternalError {
                    message: "m".to_string(),
                },
                500,
                "exception",
            ),
        ];

        for (err, status, code) in cases {
            let label = format!("{err:?}");
            let result = entry_failure(err);
            assert_eq!(result.status, status, "{label}");
            assert!(result.resource.is_none(), "{label}");

            let issue = entry_issue(&result);
            assert_eq!(issue["code"], code, "{label}");
            assert_eq!(issue["severity"], "error", "{label}");
            assert!(
                issue["details"]["text"].is_string(),
                "{label} carried no details.text"
            );
        }
    }

    #[test]
    fn test_status_text_covers_known_and_unknown_codes() {
        // The batch response builder renders a reason phrase per entry status; the
        // full table is only exercised when entries produce these codes.
        let known = [
            ("200", "OK"),
            ("201", "Created"),
            ("204", "No Content"),
            ("400", "Bad Request"),
            ("401", "Unauthorized"),
            ("403", "Forbidden"),
            ("404", "Not Found"),
            ("405", "Method Not Allowed"),
            ("406", "Not Acceptable"),
            ("409", "Conflict"),
            ("410", "Gone"),
            ("412", "Precondition Failed"),
            ("415", "Unsupported Media Type"),
            ("422", "Unprocessable Entity"),
            ("500", "Internal Server Error"),
            ("501", "Not Implemented"),
        ];
        for (code, phrase) in known {
            assert_eq!(status_text(code), phrase, "reason phrase for {code}");
        }
        // Any unmapped code falls through to the catch-all.
        assert_eq!(status_text("418"), "Unknown");
        assert_eq!(status_text(""), "Unknown");
    }

    #[tokio::test]
    async fn test_emit_batch_entry_audit_records_per_entry() {
        let sink = Arc::new(CollectorSink {
            events: Mutex::new(Vec::new()),
        });
        let state = AppState::with_auth_and_audit(
            Arc::new(MockStorage),
            crate::config::ServerConfig::default(),
            helios_auth::AuthConfig::default(),
            None,
            Some(Arc::clone(&sink) as Arc<dyn AuditSink>),
            "Device/hfs",
        );

        let entry_1 = serde_json::json!({
            "request": { "method": "GET", "url": "Patient/123" }
        });
        let entry_2 = serde_json::json!({
            "request": { "method": "POST", "url": "Observation" },
            "resource": {
                "resourceType": "Observation",
                "id": "obs-1",
                "subject": { "reference": "Patient/123" }
            }
        });

        let result_1 = BundleEntryResult {
            status: 200,
            location: None,
            etag: None,
            last_modified: None,
            resource: Some(serde_json::json!({
                "resourceType": "Patient",
                "id": "123"
            })),
            outcome: None,
        };
        let result_2 = BundleEntryResult {
            status: 201,
            location: None,
            etag: None,
            last_modified: None,
            resource: Some(serde_json::json!({
                "resourceType": "Observation",
                "id": "obs-1",
                "subject": { "reference": "Patient/123" }
            })),
            outcome: None,
        };
        let correlation = AuditCorrelation::new("batch");
        let correlation_0 = EntryAuditCorrelation::from_bundle(&correlation, 0);
        let correlation_1 = EntryAuditCorrelation::from_bundle(&correlation, 1);

        emit_batch_entry_audit(
            &state,
            &entry_1,
            &result_1,
            None,
            None,
            Some(&correlation_0),
        );
        emit_batch_entry_audit(
            &state,
            &entry_2,
            &result_2,
            None,
            None,
            Some(&correlation_1),
        );

        for _ in 0..20 {
            if sink.events.lock().await.len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let events = sink.events.lock().await;
        assert_eq!(events.len(), 2);

        let event_details: Vec<HashMap<String, String>> = events.iter().map(detail_map).collect();

        let bundle_ids: HashSet<String> = event_details
            .iter()
            .filter_map(|d| d.get("bundle-id").cloned())
            .collect();
        assert_eq!(bundle_ids.len(), 1);

        assert!(event_details.iter().all(|d| {
            d.get("bundle-type")
                .is_some_and(|bundle_type| bundle_type == "batch")
        }));

        let entry_indexes: HashSet<String> = event_details
            .iter()
            .filter_map(|d| d.get("entry-index").cloned())
            .collect();
        assert_eq!(
            entry_indexes,
            HashSet::from_iter(["0".to_string(), "1".to_string()])
        );
    }

    // ---- Batch entry concurrency (#501) ------------------------------------

    /// A backend that makes entry execution observable.
    ///
    /// It declares a `bulk_write_concurrency` — which is what the batch loop
    /// actually consults, so a mock that does not override it pins nothing —
    /// records the high-water mark of simultaneous reads, and delays each read
    /// so that a sequential loop and a concurrent one are distinguishable in
    /// both wall clock and completion order.
    struct DelayStorage {
        concurrency: usize,
        delay: std::time::Duration,
        /// When set, entry `n` of `total` sleeps `(total - n) * delay`, so
        /// entry 0 finishes *last* and completion order is the exact reverse of
        /// request order. That is what distinguishes `buffered` from
        /// `buffer_unordered`.
        reverse_of: Option<usize>,
        in_flight: AtomicUsize,
        peak_in_flight: AtomicUsize,
    }

    impl DelayStorage {
        fn new(concurrency: usize, delay_ms: u64) -> Self {
            Self {
                concurrency,
                delay: std::time::Duration::from_millis(delay_ms),
                reverse_of: None,
                in_flight: AtomicUsize::new(0),
                peak_in_flight: AtomicUsize::new(0),
            }
        }

        fn reversing(concurrency: usize, delay_ms: u64, total: usize) -> Self {
            Self {
                reverse_of: Some(total),
                ..Self::new(concurrency, delay_ms)
            }
        }

        fn peak(&self) -> usize {
            self.peak_in_flight.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl ResourceStorage for DelayStorage {
        fn backend_name(&self) -> &'static str {
            "delay"
        }

        fn bulk_write_concurrency(&self) -> usize {
            self.concurrency
        }

        async fn read(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            let entered = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(entered, Ordering::SeqCst);

            let delay = match self.reverse_of {
                Some(total) => {
                    let n: usize = id.trim_start_matches('p').parse().unwrap_or(0);
                    self.delay * (total.saturating_sub(n)) as u32
                }
                None => self.delay,
            };
            tokio::time::sleep(delay).await;

            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            Ok(Some(StoredResource::new(
                resource_type,
                id,
                tenant.tenant_id().clone(),
                serde_json::json!({ "resourceType": resource_type, "id": id }),
                FhirVersion::default(),
            )))
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    /// A batch Bundle of `count` GET entries, targeting `Patient/p0..p{count}`.
    fn get_bundle(count: usize) -> Value {
        let entries: Vec<Value> = (0..count)
            .map(|i| {
                serde_json::json!({
                    "request": { "method": "GET", "url": format!("Patient/p{i}") }
                })
            })
            .collect();
        serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": entries,
        })
    }

    async fn run_batch<S>(
        state: &AppState<S>,
        bundle: &Value,
        principal: Option<&Principal>,
    ) -> Value
    where
        S: ResourceStorage + Send + Sync,
    {
        let tenant = TenantExtractor::new("test-tenant", crate::tenant::TenantSource::Default);
        let response = process_batch(
            state,
            tenant,
            FhirVersion::default(),
            &PreferHeader::default(),
            bundle,
            principal,
        )
        .await
        .expect("batch should always produce a response");

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("response body is JSON")
    }

    fn state_with(storage: DelayStorage) -> AppState<DelayStorage> {
        AppState::new(Arc::new(storage), crate::config::ServerConfig::default())
    }

    /// Like [`state_with`], but with write-path validation in `enforce` mode.
    /// `ServerConfig::default()`'s validation mode is `off`, so no other test
    /// in this module can reach the 422 arm.
    fn enforcing_state_with(storage: DelayStorage) -> AppState<DelayStorage> {
        AppState::new(
            Arc::new(storage),
            crate::config::ServerConfig {
                validation: crate::config::ValidationConfig {
                    mode: "enforce".to_string(),
                    ..Default::default()
                },
                ..crate::config::ServerConfig::default()
            },
        )
    }

    /// A validation failure carries the validator's own issues, and is refused
    /// before the entry reaches storage.
    ///
    /// The wire-level parity with `POST [base]/Patient` is asserted by
    /// `the_two_surfaces_report_the_same_validation_issues` in
    /// `tests/validation_enforcement_tests.rs`; what this adds is the ordering
    /// guarantee. `DelayStorage::create` is `unimplemented!()`, so a validation
    /// failure moved after dispatch panics here rather than quietly writing,
    /// and `peak() == 0` proves not even a read occurred.
    #[tokio::test]
    async fn a_validation_failure_carries_the_validators_own_issues() {
        let state = enforcing_state_with(DelayStorage::new(8, 0));

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [{
                "request": { "method": "POST", "url": "Patient" },
                "resource": { "resourceType": "Patient", "bogusElement": true }
            }]
        });

        let response = run_batch(&state, &bundle, None).await;
        let entry = &response["entry"][0]["response"];
        assert_eq!(entry["status"], "422 Unprocessable Entity", "{response}");

        let issues = entry["outcome"]["issue"]
            .as_array()
            .expect("the validator's issue array");
        assert!(
            issues.iter().any(|i| {
                i["code"] == "structure" && i["expression"][0] == "Patient.bogusElement"
            }),
            "the entry must carry the validator's coded, located issues: {entry}"
        );

        assert_eq!(
            state.storage().peak(),
            0,
            "the entry must not reach storage"
        );
    }

    /// Response entry *i* must answer request entry *i*, even when entry *i*
    /// finishes last.
    ///
    /// The mock resolves entries in exact reverse order, so this fails under
    /// `buffer_unordered` and passes under `buffered`. Nothing in a response
    /// entry carries its index, so without this test a scramble is invisible.
    #[tokio::test]
    async fn batch_response_entries_stay_positional_under_concurrency() {
        const ENTRIES: usize = 16;

        let state = state_with(DelayStorage::reversing(ENTRIES, 10, ENTRIES));
        let body = run_batch(&state, &get_bundle(ENTRIES), None).await;

        let entries = body["entry"].as_array().expect("entry array");
        assert_eq!(entries.len(), ENTRIES);

        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(
                entry["resource"]["id"].as_str(),
                Some(format!("p{i}").as_str()),
                "response entry {i} answered a different request entry"
            );
        }

        assert!(
            state.storage().peak() > 1,
            "the ordering guarantee is only meaningful if entries really overlapped"
        );
    }

    /// Entries run concurrently, and never more concurrently than the backend
    /// declared.
    #[tokio::test]
    async fn batch_entries_run_concurrently_up_to_the_bound() {
        const ENTRIES: usize = 32;
        const BOUND: usize = 8;
        const DELAY_MS: u64 = 40;

        let state = state_with(DelayStorage::new(BOUND, DELAY_MS));
        let started = std::time::Instant::now();
        let body = run_batch(&state, &get_bundle(ENTRIES), None).await;
        let elapsed = started.elapsed();

        assert_eq!(
            body["entry"].as_array().expect("entry array").len(),
            ENTRIES
        );

        let peak = state.storage().peak();
        assert!(peak > 1, "entries did not overlap at all (peak {peak})");
        assert!(
            peak <= BOUND,
            "exceeded the bound the backend declared: peak {peak} > {BOUND}"
        );

        // Sequential would be ENTRIES * DELAY_MS; half of that is a wide margin
        // that still cannot be met without real concurrency.
        let sequential = std::time::Duration::from_millis(DELAY_MS * ENTRIES as u64);
        assert!(
            elapsed < sequential / 2,
            "no speedup: {elapsed:?} against a sequential floor of {sequential:?}"
        );
    }

    /// A single-writer backend keeps today's behaviour exactly. This is the
    /// claim that lets SQLite stay untouched by this change.
    #[tokio::test]
    async fn sequential_backend_still_processes_entries_one_at_a_time() {
        let state = state_with(DelayStorage::new(1, 1));
        let body = run_batch(&state, &get_bundle(8), None).await;

        assert_eq!(body["entry"].as_array().expect("entry array").len(), 8);
        assert_eq!(
            state.storage().peak(),
            1,
            "a backend declaring 1 must never have two entries in flight"
        );
    }

    /// The configured ceiling lowers a backend's declared tolerance and never
    /// raises it.
    #[test]
    fn batch_concurrency_caps_but_never_raises() {
        // (backend declares, HFS_BATCH_MAX_CONCURRENCY, effective)
        let cases = [
            (32, 4, 4),                          // config lowers
            (1, 32, 1),                          // config cannot raise a single-writer backend
            (32, 32, 32),                        // both agree
            (8, 16, 8),                          // the default ceiling leaves 8 alone
            (32, 0, 1),     // a config that skipped validate() still cannot hang
            (9999, 16, 16), // absurd backend, capped by config first
            (9999, 9999, MAX_BATCH_CONCURRENCY), // then by the hard ceiling
        ];

        for (declared, configured, expected) in cases {
            let config = crate::config::ServerConfig {
                batch_max_concurrency: configured,
                ..Default::default()
            };
            let state = AppState::new(Arc::new(DelayStorage::new(declared, 0)), config);

            assert_eq!(
                batch_concurrency(&state, &[]),
                expected,
                "backend {declared} with config {configured}"
            );
        }
    }

    /// A bundle that writes a StructureDefinition falls back to sequential.
    ///
    /// `upsert_stored_profile` folds the profile into the tenant registry, and
    /// later entries' `check_write` resolve against it — a read-your-writes
    /// dependency that concurrency would make load-dependent.
    #[test]
    fn batch_concurrency_is_one_when_a_structure_definition_is_written() {
        let state = state_with(DelayStorage::new(32, 0));

        let conformance = [serde_json::json!({
            "request": { "method": "POST", "url": "StructureDefinition" },
            "resource": { "resourceType": "StructureDefinition", "id": "sd-1" }
        })];
        assert_eq!(batch_concurrency(&state, &conformance), 1);

        // Keyed off `request.url`, exactly like the side effect it protects —
        // a body without `resourceType` must still be caught.
        let url_only = [serde_json::json!({
            "request": { "method": "PUT", "url": "StructureDefinition/sd-1" },
            "resource": { "id": "sd-1" }
        })];
        assert_eq!(batch_concurrency(&state, &url_only), 1);

        // Since #503 the parse strips the query, so a conditional conformance
        // URL is caught here where it silently was not before. Such an entry is
        // refused before it writes, which makes this clamp conservative — but
        // the scan and the write must not disagree, which is what it is keyed
        // for.
        let conditional = [serde_json::json!({
            "request": { "method": "PUT", "url": "StructureDefinition?url=http://example.org/sd" },
            "resource": { "resourceType": "StructureDefinition" }
        })];
        assert_eq!(batch_concurrency(&state, &conditional), 1);

        // A bundle with no conformance writes resolves normally. Compared
        // against the empty bundle rather than a literal, so this test stays
        // about the carve-out; the cap itself is pinned by
        // `batch_concurrency_caps_but_never_raises`.
        let data_only = [serde_json::json!({
            "request": { "method": "GET", "url": "Patient/p0" }
        })];
        assert_eq!(
            batch_concurrency(&state, &data_only),
            batch_concurrency(&state, &[])
        );
        assert!(batch_concurrency(&state, &data_only) > 1);
    }

    /// The query is split off before the path, so conditional criteria never
    /// ride along in the resource type (#503).
    #[test]
    fn parse_request_url_splits_the_query_off_before_the_path() {
        // The shape from the issue: the `//` inside the criteria produced an
        // empty path segment, and the criteria became the resource type.
        assert_eq!(
            parse_request_url("Patient?identifier=http://example.org|12345").unwrap(),
            ("Patient".to_string(), String::new())
        );
        // The spec's own transaction example carries `/` inside its criteria.
        assert_eq!(
            parse_request_url("Patient?identifier=http:/example.org/fhir/ids|456456").unwrap(),
            ("Patient".to_string(), String::new())
        );
        // `[type]/?[criteria]` is the form `http.html` prints for conditional
        // delete; the empty segment must not become an id.
        assert_eq!(
            parse_request_url("Patient/?identifier=x").unwrap(),
            ("Patient".to_string(), String::new())
        );
        // A query on an instance URL qualifies the request; it is not the id.
        assert_eq!(
            parse_request_url("Patient/p1?_format=json").unwrap(),
            ("Patient".to_string(), "p1".to_string())
        );
    }

    /// Shapes that already resolved keep resolving identically.
    #[test]
    fn parse_request_url_still_addresses_types_instances_and_history() {
        for (url, expected_type, expected_id) in [
            ("Patient", "Patient", ""),
            ("Patient/p1", "Patient", "p1"),
            ("/Patient/p1", "Patient", "p1"),
            ("Patient/p1/_history/2", "Patient", "p1"),
        ] {
            assert_eq!(
                parse_request_url(url).unwrap(),
                (expected_type.to_string(), expected_id.to_string()),
                "url: {url}"
            );
        }
    }

    /// The empty-URL arm used to be unreachable — `str::split` always yields at
    /// least one element, so an absent `request.url` parsed as the resource type
    /// `""` and the POST arm created a row under it.
    #[test]
    fn parse_request_url_rejects_a_url_with_no_resource_type() {
        for url in ["", "/", "?identifier=x"] {
            assert!(parse_request_url(url).is_err(), "url: {url}");
        }
    }

    #[test]
    fn conditional_criteria_only_fires_on_a_type_level_url() {
        assert_eq!(
            conditional_criteria("Patient?identifier=x", ""),
            Some("identifier=x")
        );
        // An instance URL already addresses its target.
        assert_eq!(conditional_criteria("Patient/p1?_format=json", "p1"), None);
        // Nothing to condition on. A bare `Patient?` in particular must not be
        // read as criteria — that would match every Patient.
        assert_eq!(conditional_criteria("Patient", ""), None);
        assert_eq!(conditional_criteria("Patient?", ""), None);
    }

    /// `request.method` is a `code` with a required binding to `http-verb`,
    /// whose concepts are case-sensitive and uppercase. Only those five codes
    /// dispatch; everything else is refused, and the refusal carries the status
    /// both bundle arms will use (#502).
    #[test]
    fn parse_entry_method_accepts_only_the_canonical_http_verb_codes() {
        for (raw, expected) in [
            ("GET", BundleMethod::Get),
            ("POST", BundleMethod::Post),
            ("PUT", BundleMethod::Put),
            ("PATCH", BundleMethod::Patch),
            ("DELETE", BundleMethod::Delete),
        ] {
            let request = serde_json::json!({ "method": raw, "url": "Patient" });
            assert_eq!(parse_entry_method(&request), Ok(expected), "raw: {raw}");
        }

        // A legal http-verb code this server does not accept inside a Bundle.
        // HEAD *is* served on the instance-read route.
        let head = serde_json::json!({ "method": "HEAD", "url": "Patient/p1" });
        assert_eq!(parse_entry_method(&head), Err(EntryMethodRefusal::Head));
        // Read through the only remaining source of the status since #504
        // deleted `EntryMethodRefusal::status()`, which held a second copy of
        // it beside `into_rest_error`'s choice of variant.
        assert_eq!(
            EntryMethodRefusal::Head
                .into_rest_error(0)
                .client_response()
                .0,
            StatusCode::METHOD_NOT_ALLOWED
        );

        // Case-folded spellings are invalid instance data, not valid entries a
        // strict server wrongly rejects — this is the premise #502 inverted.
        for raw in ["post", "Post", "get", "Patch", "delete", "FOO", ""] {
            let request = serde_json::json!({ "method": raw, "url": "Patient" });
            assert_eq!(
                parse_entry_method(&request),
                Err(EntryMethodRefusal::NotCanonical(raw.to_string())),
                "raw: {raw}"
            );
        }
        assert_eq!(
            EntryMethodRefusal::NotCanonical("post".to_string())
                .into_rest_error(0)
                .client_response()
                .0,
            StatusCode::BAD_REQUEST
        );

        // Absent or non-string is distinguishable from a bogus code. It used to
        // read as `""` via `unwrap_or("")`, yielding "Unsupported method: ".
        for request in [
            serde_json::json!({ "url": "Patient" }),
            serde_json::json!({ "method": 42, "url": "Patient" }),
            serde_json::json!({ "method": null, "url": "Patient" }),
        ] {
            assert_eq!(
                parse_entry_method(&request),
                Err(EntryMethodRefusal::Missing),
                "request: {request}"
            );
        }
        assert_eq!(
            EntryMethodRefusal::Missing
                .into_rest_error(0)
                .client_response()
                .0,
            StatusCode::BAD_REQUEST
        );
    }

    /// A refusal is rendered identically by both arms — status, issue code and
    /// message.
    ///
    /// #515 pinned only the status, and did it by asserting the `RestError`
    /// *variant* as a proxy for the pair. That assertion survives #504
    /// unchanged while saying nothing about the code, so it is replaced rather
    /// than kept: the batch arm's per-entry outcome and the transaction arm's
    /// whole-bundle error must now agree on all three, which is strictly
    /// stronger than what it replaces.
    #[test]
    fn a_method_refusal_renders_identically_on_both_arms() {
        // (refusal, status, issue code)
        let cases = [
            (
                EntryMethodRefusal::Head,
                StatusCode::METHOD_NOT_ALLOWED,
                "not-supported",
            ),
            (
                EntryMethodRefusal::Missing,
                StatusCode::BAD_REQUEST,
                "required",
            ),
            (
                EntryMethodRefusal::NotCanonical("post".to_string()),
                StatusCode::BAD_REQUEST,
                "value",
            ),
        ];

        for (refusal, status, code) in cases {
            // What the transaction arm returns as the whole-bundle error…
            let (tx_status, tx_code, tx_message) =
                refusal.clone().into_rest_error(7).client_response();
            // …and what the batch arm records for that entry.
            let entry = entry_failure(refusal.clone().into_rest_error(7));
            let issue = entry_issue(&entry);

            assert_eq!(tx_status, status, "{refusal:?}");
            assert_eq!(tx_code, code, "{refusal:?}");
            assert_eq!(entry.status, status.as_u16(), "{refusal:?}");
            assert_eq!(issue["code"], code, "{refusal:?}");
            assert_eq!(
                issue["details"]["text"], tx_message,
                "the two arms must print one sentence for {refusal:?}"
            );
        }
    }

    /// The transaction matcher no longer case-folds. `to_uppercase()` was the
    /// only gate between an invalid `code` and a real write.
    #[test]
    fn the_transaction_matcher_no_longer_accepts_a_lowercase_method() {
        let entry = serde_json::json!({
            "request": { "method": "post", "url": "Patient" },
            "resource": { "resourceType": "Patient" }
        });
        let err = parse_bundle_entry(&entry).expect_err("must be refused");
        assert!(matches!(
            err,
            EntryParseError::Method(EntryMethodRefusal::NotCanonical(_))
        ));

        // The canonical spelling still parses.
        let ok = serde_json::json!({
            "request": { "method": "POST", "url": "Patient" },
            "resource": { "resourceType": "Patient" }
        });
        assert_eq!(
            parse_bundle_entry(&ok).unwrap().0.method,
            BundleMethod::Post
        );
    }

    /// Refused methods are answered per-entry and never dispatch.
    ///
    /// `DelayStorage`'s write methods are `unimplemented!()`, so a refusal moved
    /// after dispatch panics rather than silently writing.
    #[tokio::test]
    async fn refused_methods_are_answered_per_entry_and_never_reach_storage() {
        let state = state_with(DelayStorage::new(8, 0));

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "PATCH", "url": "Patient/p1" },
                    "resource": { "resourceType": "Patient" }
                },
                { "request": { "method": "HEAD", "url": "Patient/p1" } },
                {
                    "request": { "method": "post", "url": "Patient" },
                    "resource": { "resourceType": "Patient" }
                },
                { "request": { "url": "Patient/p1" } },
            ]
        });

        let response = run_batch(&state, &bundle, None).await;
        let entries = response["entry"].as_array().unwrap();
        let statuses: Vec<&str> = entries
            .iter()
            .map(|e| e["response"]["status"].as_str().unwrap())
            .collect();
        assert_eq!(
            statuses,
            vec![
                "501 Not Implemented",
                "405 Method Not Allowed",
                "400 Bad Request",
                "400 Bad Request",
            ]
        );
        // The four refusals were indistinguishable below the status line until
        // #504 — every one carried `processing`. PATCH and HEAD are capability
        // gaps, a lowercase verb is an unusable value, and an absent method is
        // a missing element.
        let codes: Vec<&str> = entries
            .iter()
            .map(|e| {
                e["response"]["outcome"]["issue"][0]["code"]
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            codes,
            vec!["not-supported", "not-supported", "value", "required"]
        );
        assert_eq!(state.storage().peak(), 0, "no entry may reach storage");
    }

    /// A conditional write is refused per-entry and never reaches storage.
    ///
    /// `DelayStorage::create_or_update` and `::delete` are `unimplemented!()`,
    /// so this panics rather than merely failing if a refusal is ever moved
    /// after dispatch.
    #[tokio::test]
    async fn conditional_write_entries_are_refused_before_they_reach_storage() {
        let state = state_with(DelayStorage::new(8, 0));

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "PUT", "url": "Patient?identifier=http://example.org|1" },
                    "resource": { "resourceType": "Patient" }
                },
                { "request": { "method": "DELETE", "url": "Patient?identifier=x" } },
                {
                    "request": { "method": "POST", "url": "Patient?identifier=x" },
                    "resource": { "resourceType": "Patient" }
                },
            ]
        });

        let response = run_batch(&state, &bundle, None).await;
        let entries = response["entry"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(
                entry["response"]["status"], "400 Bad Request",
                "entry {index}: {entry}"
            );
            // A conditional interaction is a named FHIR capability this server
            // does not have (#511) — the entry itself is valid. Same code the
            // transaction arm returns for the same entry.
            assert_eq!(
                entry["response"]["outcome"]["issue"][0]["code"], "not-supported",
                "entry {index}: {entry}"
            );
        }
        assert_eq!(state.storage().peak(), 0, "no entry may reach storage");
    }

    /// A type-level URL with no criteria names no instance. Left to fall
    /// through, `PUT Patient` reached `create_or_update` with an empty id, and
    /// the backend wrote a row rather than rejecting (#503).
    #[tokio::test]
    async fn type_level_writes_without_an_id_are_refused() {
        let state = state_with(DelayStorage::new(8, 0));

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": { "method": "PUT", "url": "Patient" },
                    "resource": { "resourceType": "Patient" }
                },
                { "request": { "method": "DELETE", "url": "Patient" } },
            ]
        });

        let response = run_batch(&state, &bundle, None).await;
        let entries = response["entry"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(
                entry["response"]["status"], "400 Bad Request",
                "entry {index}: {entry}"
            );
            // `request.url` is present; its value cannot address an instance.
            assert_eq!(
                entry["response"]["outcome"]["issue"][0]["code"], "value",
                "entry {index}: {entry}"
            );
        }
        assert_eq!(state.storage().peak(), 0, "no entry may reach storage");
    }

    /// An instance-addressed entry carrying a control parameter still resolves:
    /// the query is dropped, not treated as criteria.
    #[tokio::test]
    async fn an_instance_url_with_a_query_still_addresses_its_instance() {
        let state = state_with(DelayStorage::new(8, 0));

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                { "request": { "method": "GET", "url": "Patient/p1?_format=json" } },
            ]
        });

        let response = run_batch(&state, &bundle, None).await;
        let entry = &response["entry"][0];
        assert_eq!(entry["response"]["status"], "200 OK", "{entry}");
        assert_eq!(entry["resource"]["id"], "p1");
    }

    /// Scope enforcement stays per-entry when entries run concurrently: denied
    /// entries become 403 response entries and permitted ones still succeed.
    ///
    /// `POST [base]` has no upstream authorization gate, so this inline check is
    /// the only one — and there was no test asserting a batch scope denial
    /// before this change made the loop concurrent.
    #[tokio::test]
    async fn batch_scope_denial_is_still_per_entry_under_concurrency() {
        let state = state_with(DelayStorage::new(8, 1));

        let entries: Vec<Value> = (0..8)
            .map(|i| {
                let resource_type = if i % 2 == 0 { "Patient" } else { "Observation" };
                serde_json::json!({
                    "request": { "method": "GET", "url": format!("{resource_type}/p{i}") }
                })
            })
            .collect();
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": entries,
        });

        let principal = Principal {
            subject: "client".to_string(),
            issuer: "https://issuer.example".to_string(),
            tenant_id: None,
            scopes: helios_auth::ScopeSet::parse("system/Patient.rs"),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        };

        let body = run_batch(&state, &bundle, Some(&principal)).await;
        let entries = body["entry"].as_array().expect("entry array");
        assert_eq!(entries.len(), 8);

        for (i, entry) in entries.iter().enumerate() {
            let status = entry["response"]["status"].as_str().unwrap_or_default();
            if i % 2 == 0 {
                assert!(status.starts_with("200"), "entry {i} (Patient): {status}");
            } else {
                assert!(
                    status.starts_with("403"),
                    "entry {i} (Observation) must be denied: {status}"
                );
                // `forbidden` is a child of `security`, and `processing` — what
                // this denial carried until #504 — is not an ancestor of it in
                // any supported version. A client filtering `code is-a
                // security` to trigger re-auth saw a false negative here.
                assert_eq!(
                    entry["response"]["outcome"]["issue"][0]["code"], "forbidden",
                    "entry {i}: {entry}"
                );
            }
        }
    }

    /// A backend that cannot honour transaction atomicity must produce a client
    /// -actionable refusal, not a 500.
    ///
    /// 501 + `not-supported` matches the two sibling capability gaps
    /// (`NestedNotSupported`, `UnsupportedIsolationLevel`) already mapped that
    /// way, and the message has to name `batch` — that is the alternative the
    /// caller can actually act on, and the fallback the Inferno loader uses for
    /// S3 (#489).
    #[test]
    fn atomicity_unsupported_maps_to_501_not_supported() {
        let (status, code, message) =
            transaction_error_response_parts(&TransactionError::AtomicityUnsupported {
                backend_name: "s3".to_string(),
            });

        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(code, "not-supported");
        assert!(message.contains("s3"), "should name the backend: {message}");
        assert!(
            message.contains("batch"),
            "should point at the workable alternative: {message}"
        );
        assert!(
            message.contains("no entries were applied"),
            "must state that nothing was written, so a retry is known-safe: {message}"
        );
    }
}
