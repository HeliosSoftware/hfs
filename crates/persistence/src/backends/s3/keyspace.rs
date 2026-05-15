//! S3 key construction for all FHIR storage namespaces.
//!
//! Keys are structured as hierarchical paths that encode the tenant prefix,
//! resource type, resource ID, version, and operation type. [`S3Keyspace`]
//! derives every key shape used by the backend from a common base prefix.

use chrono::{DateTime, Utc};

/// Keyspace builder for S3 object paths.
///
/// Holds an optional base prefix that is prepended to every generated key.
/// All key-building methods ensure segments are joined with `/` and that the
/// prefix never has leading or trailing slashes.
#[derive(Debug, Clone)]
pub struct S3Keyspace {
    /// Optional prefix prepended to all keys, with surrounding slashes stripped.
    base_prefix: Option<String>,
}

impl S3Keyspace {
    /// Creates a new keyspace with an optional base prefix.
    ///
    /// Leading and trailing slashes in `base_prefix` are stripped. An empty
    /// string is treated as no prefix.
    pub fn new(base_prefix: Option<String>) -> Self {
        let base_prefix = base_prefix
            .map(|p| p.trim_matches('/').to_string())
            .filter(|p| !p.is_empty());
        Self { base_prefix }
    }

    /// Returns a new keyspace with `tenant_id` appended to the base prefix.
    ///
    /// Used in `PrefixPerTenant` mode to scope all keys under a per-tenant
    /// directory segment without changing the bucket.
    pub fn with_tenant_prefix(&self, tenant_id: &str) -> Self {
        let tenant = tenant_id.trim_matches('/');
        let merged = match &self.base_prefix {
            Some(base) => format!("{}/{}", base, tenant),
            None => tenant.to_string(),
        };
        Self::new(Some(merged))
    }

    /// Key for the mutable "current" pointer of a resource.
    ///
    /// This object is overwritten on every create, update, and delete.
    pub fn current_resource_key(&self, resource_type: &str, id: &str) -> String {
        self.join(&["resources", resource_type, id, "current.json"])
    }

    /// Immutable key for a specific historical version of a resource.
    pub fn history_version_key(&self, resource_type: &str, id: &str, version_id: &str) -> String {
        self.join(&[
            "resources",
            resource_type,
            id,
            "_history",
            &format!("{}.json", version_id),
        ])
    }

    /// Prefix covering all history version objects for a resource.
    pub fn history_versions_prefix(&self, resource_type: &str, id: &str) -> String {
        self.join(&["resources", resource_type, id, "_history/"])
    }

    /// Prefix covering all current resource objects across all types.
    pub fn resources_prefix(&self) -> String {
        self.join(&["resources/"])
    }

    /// Prefix covering all current objects of a specific resource type.
    pub fn resource_type_prefix(&self, resource_type: &str) -> String {
        self.join(&["resources", resource_type, "/"])
    }

    /// Key for a type-level history index event.
    ///
    /// The filename encodes the event timestamp in milliseconds, resource ID,
    /// version ID, and a random suffix to prevent key collisions during
    /// concurrent writes to the same resource.
    pub fn history_type_event_key(
        &self,
        resource_type: &str,
        timestamp: DateTime<Utc>,
        id: &str,
        version_id: &str,
        suffix: &str,
    ) -> String {
        self.join(&[
            "history",
            "type",
            resource_type,
            &format!(
                "{}_{}_{}_{}.json",
                timestamp.timestamp_millis(),
                sanitize(id),
                version_id,
                suffix
            ),
        ])
    }

    /// Key for a system-level history index event.
    ///
    /// Analogous to `history_type_event_key` but stored under the system
    /// history prefix so that cross-type queries scan a single directory.
    pub fn history_system_event_key(
        &self,
        resource_type: &str,
        timestamp: DateTime<Utc>,
        id: &str,
        version_id: &str,
        suffix: &str,
    ) -> String {
        self.join(&[
            "history",
            "system",
            &format!(
                "{}_{}_{}_{}_{}.json",
                timestamp.timestamp_millis(),
                sanitize(resource_type),
                sanitize(id),
                version_id,
                suffix
            ),
        ])
    }

    /// Prefix covering all type-level history index events for a resource type.
    pub fn history_type_prefix(&self, resource_type: &str) -> String {
        self.join(&["history", "type", resource_type, "/"])
    }

    /// Prefix covering all system-level history index events.
    pub fn history_system_prefix(&self) -> String {
        self.join(&["history", "system/"])
    }

    /// Key for the JSON state object of a bulk export job.
    #[allow(dead_code)]
    pub fn export_job_state_key(&self, job_id: &str) -> String {
        self.join(&["bulk", "export", "jobs", job_id, "state.json"])
    }

    /// Key for per-type export progress within a job.
    #[allow(dead_code)]
    pub fn export_job_progress_key(&self, job_id: &str, resource_type: &str) -> String {
        self.join(&[
            "bulk",
            "export",
            "jobs",
            job_id,
            "progress",
            &format!("{}.json", resource_type),
        ])
    }

    /// Key for the completed export manifest of a job.
    #[allow(dead_code)]
    pub fn export_job_manifest_key(&self, job_id: &str) -> String {
        self.join(&["bulk", "export", "jobs", job_id, "manifest.json"])
    }

    /// Key for a single NDJSON output part within an export job.
    #[allow(dead_code)]
    pub fn export_job_output_key(&self, job_id: &str, resource_type: &str, part: u32) -> String {
        self.join(&[
            "bulk",
            "export",
            "jobs",
            job_id,
            "output",
            resource_type,
            &format!("part-{}.ndjson", part),
        ])
    }

    /// Prefix covering all export job objects.
    #[allow(dead_code)]
    pub fn export_jobs_prefix(&self) -> String {
        self.join(&["bulk", "export", "jobs/"])
    }

    /// Prefix covering all objects belonging to a single export job.
    #[allow(dead_code)]
    pub fn export_job_prefix(&self, job_id: &str) -> String {
        self.join(&["bulk", "export", "jobs", job_id, "/"])
    }

    /// Key for the JSON state object of a bulk submission.
    pub fn submit_state_key(&self, submitter: &str, submission_id: &str) -> String {
        self.join(&["bulk", "submit", submitter, submission_id, "state.json"])
    }

    /// Key for a manifest within a bulk submission.
    pub fn submit_manifest_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            submitter,
            submission_id,
            "manifests",
            &format!("{}.json", manifest_id),
        ])
    }

    /// Key for a single raw NDJSON line within a submission manifest.
    pub fn submit_raw_line_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            submitter,
            submission_id,
            "raw",
            manifest_id,
            &format!("line-{}.ndjson", line),
        ])
    }

    /// Key for the processing result of a single NDJSON line.
    pub fn submit_result_line_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            submitter,
            submission_id,
            "results",
            manifest_id,
            &format!("line-{}.json", line),
        ])
    }

    /// Key for a recorded change (create or update) within a submission.
    pub fn submit_change_key(
        &self,
        submitter: &str,
        submission_id: &str,
        change_id: &str,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            submitter,
            submission_id,
            "changes",
            &format!("{}.json", change_id),
        ])
    }

    /// Prefix covering all objects belonging to a single submission.
    pub fn submit_prefix(&self, submitter: &str, submission_id: &str) -> String {
        self.join(&["bulk", "submit", submitter, submission_id, "/"])
    }

    /// Prefix covering all bulk-submit objects across all submissions.
    pub fn submit_root_prefix(&self) -> String {
        self.join(&["bulk", "submit/"])
    }

    /// Joins `parts` with `/`, prepending the base prefix when set.
    ///
    /// Trailing slashes are preserved only when the final part itself ends with
    /// `/` (used to produce consistent list prefixes for S3 pagination).
    fn join(&self, parts: &[&str]) -> String {
        let mut segs: Vec<String> = Vec::new();
        if let Some(prefix) = &self.base_prefix {
            segs.push(prefix.clone());
        }

        for part in parts {
            let trimmed = part.trim_matches('/');
            if trimmed.is_empty() {
                continue;
            }
            segs.push(trimmed.to_string());
        }

        let mut out = segs.join("/");
        if parts.last().map(|p| p.ends_with('/')).unwrap_or(false) && !out.ends_with('/') {
            out.push('/');
        }
        out
    }
}

/// Replaces characters that are unsafe in S3 key path segments.
///
/// Slashes, backslashes, and spaces are replaced with underscores so that
/// resource IDs and type names can be embedded in key paths without
/// accidentally splitting path segments.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ' ' => '_',
            _ => c,
        })
        .collect()
}
