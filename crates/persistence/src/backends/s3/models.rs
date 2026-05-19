//! S3-specific persistence models for history indexing and bulk submission
//! state.
//!
//! These types are serialised as JSON objects in S3 and are never exposed
//! outside the `s3` backend module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::bulk_submit::{SubmissionManifest, SubmissionSummary};
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionState {
    /// Submission summary including status and aggregate counts.
    pub summary: SubmissionSummary,
    /// Human-readable reason recorded when the submission is aborted.
    pub abort_reason: Option<String>,
}

/// Wrapper persisted to S3 for each manifest within a bulk submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionManifestState {
    /// The manifest metadata and current processing status.
    pub manifest: SubmissionManifest,
}
