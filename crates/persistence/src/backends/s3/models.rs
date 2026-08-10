//! S3-specific persistence models for history indexing and bulk submission
//! state.
//!
//! These types are serialised as JSON objects in S3 and are never exposed
//! outside the `s3` backend module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::bulk_submit::{SubmissionManifest, SubmissionStatus, SubmissionSummary};
use crate::core::history::HistoryMethod;

/// A small index record written to S3 for each resource mutation.
///
/// One event is stored under the type-level history prefix and another under
/// the system-level prefix. They are later scanned to reconstruct history
/// without loading the full resource bodies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryIndexEvent {
    /// FHIR resource type of the mutated resource.
    pub resource_type: String,
    /// Logical resource ID.
    pub id: String,
    /// Version ID assigned to this mutation.
    pub version_id: String,
    /// Wall-clock time of the mutation.
    pub timestamp: DateTime<Utc>,
    /// HTTP method that produced this version.
    pub method: HistoryMethod,
    /// True if this mutation is a logical delete (tombstone).
    pub deleted: bool,
}

/// Durable state of a bulk submission stored in S3.
///
/// Written to `bulk/submit/<submitter>/<id>/state.json` when a submission is
/// created and updated on every lifecycle transition.
///
/// Everything below `abort_reason` is `$bulk-submit` REST/worker state, added
/// when S3 gained a native submit job store. All of it is `#[serde(default)]`
/// so submissions written before that still deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionState {
    /// Submission summary including status and aggregate counts.
    pub summary: SubmissionSummary,
    /// Human-readable reason recorded when the submission is aborted.
    pub abort_reason: Option<String>,
    /// OAuth subject that kicked the submission off, for ownership checks on
    /// status/cancel/file requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_subject: Option<String>,
    /// The kickoff request URL, echoed into the status manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    /// Whether status-manifest artifacts require an access token to fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_access_token: Option<bool>,
    /// Opaque token the Data Provider polls the submission's status with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_token: Option<String>,
    /// Time the status manifest was first finalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_time: Option<DateTime<Utc>>,
}

/// Wrapper persisted to S3 for each manifest within a bulk submission.
///
/// This object is also the manifest's **lease record**: `worker_id`,
/// `lease_expiry`, and `fencing_token` are compare-and-swapped against the
/// object's S3 ETag, which is what lets a standalone S3 primary host the
/// `$bulk-submit` worker without a sidecar database. Everything after
/// `manifest` is `#[serde(default)]` so objects written before that still
/// deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionManifestState {
    /// The manifest metadata and current processing status.
    pub manifest: SubmissionManifest,
    /// Worker currently holding the lease, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    /// When the current lease expires and the manifest becomes reclaimable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expiry: Option<DateTime<Utc>>,
    /// Monotonically increasing token, bumped on every claim.
    #[serde(default)]
    pub fencing_token: u64,
    /// Base URL for resolving relative references in ingested resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fhir_base_url: Option<String>,
    /// The kickoff `outputFormat` (MIME), used to derive the FHIR version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    /// Headers the Data Provider asked us to send when fetching its files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_request_headers: Vec<(String, String)>,
    /// OAuth 2.0 metadata endpoints for acquiring file-retrieval tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oauth_metadata_urls: Vec<String>,
    /// JWE file-encryption key descriptor, if the provider encrypts files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_encryption_key: Option<Value>,
    /// Pre-coordinated `import` directives as `(parameterUrl, parameterValue)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_directives: Vec<(String, String)>,
    /// Pre-coordinated `metadata` parts as `(parameterUrl, parameterValue)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submission_metadata: Vec<(String, String)>,
    /// Resume cursor: lines already processed for this manifest.
    #[serde(default)]
    pub last_processed_line: u64,
    /// Failure detail recorded when the manifest is marked `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl SubmissionManifestState {
    /// Wraps a freshly added manifest, with no lease and no fetch parameters.
    pub fn new(manifest: SubmissionManifest) -> Self {
        Self {
            manifest,
            worker_id: None,
            lease_expiry: None,
            fencing_token: 0,
            fhir_base_url: None,
            output_format: None,
            file_request_headers: Vec::new(),
            oauth_metadata_urls: Vec::new(),
            file_encryption_key: None,
            import_directives: Vec::new(),
            submission_metadata: Vec::new(),
            last_processed_line: 0,
            error_message: None,
        }
    }
}

/// An entry in the cross-tenant `$bulk-submit` worker index.
///
/// The worker claims manifests, resolves poll tokens, and sweeps expired
/// submissions with no tenant in hand, so each of those lookups needs a record
/// outside any tenant's prefix pointing back at the tenant-scoped objects. One
/// shape serves all three namespaces; `manifest_id` is set only in `queue`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitIndexEntry {
    /// Tenant owning the submission.
    pub tenant_id: String,
    /// Submitter half of the submission id.
    pub submitter: String,
    /// Submission half of the submission id.
    pub submission_id: String,
    /// The manifest this entry queues, for `queue` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_id: Option<String>,
    /// When the manifest was added — the queue's claim order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<DateTime<Utc>>,
    /// Last time the submission changed, for the TTL sweep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    /// The submission's status, so the per-tenant concurrency cap can be
    /// answered from this index instead of reading every submission object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<SubmissionStatus>,
}
