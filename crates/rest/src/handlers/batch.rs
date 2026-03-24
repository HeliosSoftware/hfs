//! Batch and transaction processing handler.
//!
//! Implements the FHIR [batch/transaction interaction](https://hl7.org/fhir/http.html#transaction):
//! `POST [base]` with a Bundle of type "batch" or "transaction"

use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_auth::{FhirOperation, Principal, SmartScopePolicy};
use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, ResourceStorage,
};
use helios_persistence::error::TransactionError;
use serde_json::Value;
use tracing::{debug, error, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
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
    prefer: PreferHeader,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + BundleProvider + Send + Sync,
{
    // Extract the Principal from request extensions (set by auth middleware).
    // If present, per-entry scope checks will be enforced.
    let principal = request.extensions().get::<Principal>().cloned();

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
        "batch" => process_batch(&state, tenant, &prefer, &bundle, principal.as_ref()).await,
        "transaction" => {
            process_transaction(&state, tenant, &prefer, &bundle, principal.as_ref()).await
        }
        _ => Err(RestError::BadRequest {
            message: format!(
                "Bundle type must be 'batch' or 'transaction', got '{}'",
                bundle_type
            ),
        }),
    }
}

/// Processes a batch Bundle.
async fn process_batch<S>(
    state: &AppState<S>,
    tenant: TenantExtractor,
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

    let entries = bundle
        .get("entry")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let base_url = state.base_url();
    let mut response_entries = Vec::with_capacity(entries.len());

    for (index, entry) in entries.iter().enumerate() {
        let result = process_batch_entry(state, &tenant, entry, index, principal).await;
        response_entries.push(bundle_entry_result_to_json(&result, base_url, prefer));
    }

    let response_bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "batch-response",
        "entry": response_entries
    });

    debug!(
        entries = response_entries.len(),
        "Batch processing completed"
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
    prefer: &PreferHeader,
    bundle: &Value,
    principal: Option<&Principal>,
) -> RestResult<Response>
where
    S: ResourceStorage + BundleProvider + Send + Sync,
{
    debug!(
        tenant = %tenant.tenant_id(),
        "Processing transaction request"
    );

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
                // Enforce per-entry scope authorization for transactions.
                // Transactions are atomic so any denied entry rejects the whole bundle.
                if let Some(principal) = principal {
                    let (resource_type, _) =
                        parse_request_url(&bundle_entry.url).map_err(|e| RestError::BadRequest {
                            message: format!("Entry {}: {}", index, e),
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
                indexed_entries.push((index, bundle_entry, full_url));
            }
            Err(e) => {
                // For transactions, any parse error fails the whole bundle
                return Err(RestError::BadRequest {
                    message: format!("Entry {}: {}", index, e),
                });
            }
        }
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
        .process_transaction(tenant.context(), entries_for_processing)
        .await;

    match result {
        Ok(bundle_result) => {
            // Reorder results back to original entry order
            let mut ordered_results: Vec<(usize, &BundleEntryResult)> = indexed_entries
                .iter()
                .zip(bundle_result.entries.iter())
                .map(|((orig_idx, _, _), result)| (*orig_idx, result))
                .collect();
            ordered_results.sort_by_key(|(idx, _)| *idx);

            let base_url = state.base_url();
            let response_entries: Vec<Value> = ordered_results
                .into_iter()
                .map(|(_, result)| bundle_entry_result_to_json(result, base_url, prefer))
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
            error!(error = %e, "Transaction failed");
            transaction_error_to_response(e)
        }
    }
}

/// Processes a single batch entry, returning a structured BundleEntryResult.
async fn process_batch_entry<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
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
            return create_error_result(400, &format!("Entry {} missing request", index));
        }
    };

    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let url = request.get("url").and_then(|v| v.as_str()).unwrap_or("");

    // Parse the URL to extract resource type and ID
    let (resource_type, id) = match parse_request_url(url) {
        Ok(parsed) => parsed,
        Err(e) => {
            return create_error_result(400, &e);
        }
    };

    // Enforce per-entry scope authorization
    if let Some(principal) = principal {
        let operation = match method {
            "GET" => FhirOperation::Read,
            "POST" => FhirOperation::Create,
            "PUT" | "PATCH" => FhirOperation::Update,
            "DELETE" => FhirOperation::Delete,
            _ => FhirOperation::Read, // will be caught by unsupported method below
        };
        if SmartScopePolicy::check(principal, &resource_type, operation).is_err() {
            return create_error_result(
                403,
                &format!(
                    "Insufficient scope for {} on {} (batch entry {})",
                    operation, resource_type, index
                ),
            );
        }
    }

    match method {
        "GET" => {
            // Read operation
            match state
                .storage()
                .read(tenant.context(), &resource_type, &id)
                .await
            {
                Ok(Some(stored)) => BundleEntryResult::ok(stored),
                Ok(None) => create_error_result(404, "Resource not found"),
                Err(e) => create_error_result(500, &e.to_string()),
            }
        }
        "POST" => {
            // Create operation
            let resource = match entry.get("resource") {
                Some(r) => r.clone(),
                None => {
                    return create_error_result(400, "POST entry missing resource");
                }
            };

            // Use default FHIR version for batch operations
            match state
                .storage()
                .create(
                    tenant.context(),
                    &resource_type,
                    resource,
                    FhirVersion::default(),
                )
                .await
            {
                Ok(stored) => BundleEntryResult::created(stored),
                Err(e) => create_error_result(400, &e.to_string()),
            }
        }
        "PUT" => {
            // Update operation
            let resource = match entry.get("resource") {
                Some(r) => r.clone(),
                None => {
                    return create_error_result(400, "PUT entry missing resource");
                }
            };

            // Use default FHIR version for batch operations
            match state
                .storage()
                .create_or_update(
                    tenant.context(),
                    &resource_type,
                    &id,
                    resource,
                    FhirVersion::default(),
                )
                .await
            {
                Ok((stored, created)) => {
                    if created {
                        BundleEntryResult::created(stored)
                    } else {
                        // For updates, include location with versioned URL
                        let mut result = BundleEntryResult::ok(stored);
                        result.location = Some(format!("{}/{}", resource_type, id));
                        result
                    }
                }
                Err(e) => create_error_result(400, &e.to_string()),
            }
        }
        "DELETE" => {
            // Delete operation
            match state
                .storage()
                .delete(tenant.context(), &resource_type, &id)
                .await
            {
                Ok(()) => BundleEntryResult::deleted(),
                Err(e) => create_error_result(404, &e.to_string()),
            }
        }
        _ => {
            warn!(method = method, "Unsupported batch method");
            create_error_result(405, &format!("Unsupported method: {}", method))
        }
    }
}

/// Parses a request URL to extract resource type and optional ID.
fn parse_request_url(url: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = url.trim_start_matches('/').split('/').collect();

    match parts.len() {
        0 => Err("Empty URL".to_string()),
        1 => Ok((parts[0].to_string(), String::new())),
        2 => Ok((parts[0].to_string(), parts[1].to_string())),
        _ => {
            // Handle URLs like Patient/123/_history/1
            Ok((parts[0].to_string(), parts[1].to_string()))
        }
    }
}

/// Creates an error BundleEntryResult.
fn create_error_result(status: u16, message: &str) -> BundleEntryResult {
    let outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "processing",
            "details": {
                "text": message
            }
        }]
    });
    BundleEntryResult::error(status, outcome)
}

/// Returns HTTP status text for a status code.
fn status_text(code: &str) -> &'static str {
    match code {
        "200" => "OK",
        "201" => "Created",
        "204" => "No Content",
        "400" => "Bad Request",
        "404" => "Not Found",
        "405" => "Method Not Allowed",
        "409" => "Conflict",
        "412" => "Precondition Failed",
        "500" => "Internal Server Error",
        "501" => "Not Implemented",
        _ => "Unknown",
    }
}

/// Parses a bundle entry from JSON into a BundleEntry struct.
///
/// Returns the BundleEntry and optionally the fullUrl for reference resolution.
fn parse_bundle_entry(entry: &Value) -> Result<(BundleEntry, Option<String>), String> {
    let request = entry
        .get("request")
        .ok_or_else(|| "Entry missing 'request'".to_string())?;

    let method_str = request
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Entry request missing 'method'".to_string())?;

    let method = match method_str.to_uppercase().as_str() {
        "GET" => BundleMethod::Get,
        "POST" => BundleMethod::Post,
        "PUT" => BundleMethod::Put,
        "PATCH" => BundleMethod::Patch,
        "DELETE" => BundleMethod::Delete,
        _ => return Err(format!("Unsupported method: {}", method_str)),
    };

    let url = request
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Entry request missing 'url'".to_string())?
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

/// Converts a TransactionError to an HTTP response with OperationOutcome.
fn transaction_error_to_response(err: TransactionError) -> RestResult<Response> {
    let (status_code, issue_code, message) = match &err {
        TransactionError::BundleError { index, message } => (
            StatusCode::BAD_REQUEST,
            "processing",
            format!("Transaction failed at entry {}: {}", index, message),
        ),
        TransactionError::RolledBack { reason } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "transient",
            format!("Transaction rolled back: {}", reason),
        ),
        TransactionError::Timeout { timeout_ms } => (
            StatusCode::INTERNAL_SERVER_ERROR,
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
    };

    let outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": issue_code,
            "details": {
                "text": message
            }
        }]
    });

    Ok((status_code, Json(outcome)).into_response())
}
