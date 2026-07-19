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

    /// Key for a tenant's registry record — one JSON object per registered
    /// tenant. The registry spans tenants, so this is only meaningful on an
    /// un-tenanted keyspace (no `with_tenant_prefix`).
    pub fn tenant_registry_key(&self, tenant_id: &str) -> String {
        self.join(&["tenants", &format!("{}.json", sanitize(tenant_id))])
    }

    /// Prefix covering all tenant registry records.
    pub fn tenant_registry_prefix(&self) -> String {
        self.join(&["tenants/"])
    }

    /// Prefix covering all history index events (type- and system-level).
    pub fn history_root_prefix(&self) -> String {
        self.join(&["history/"])
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

    /// Key for a single user's per-user settings object.
    ///
    /// `object_id` **must** be an opaque, injective digest of the user key (see
    /// `settings_object_id` in the `user_settings` module), never the raw key.
    /// The raw key is `"{issuer}|{subject}"` built from unvalidated JWT claims: it
    /// can contain `/`, `..`, or be empty, and `sanitize` below is *lossy*, so
    /// embedding it here would let two distinct users collide on one object — a
    /// cross-user settings leak.
    ///
    /// This key is deliberately built from the *base* keyspace, without
    /// [`with_tenant_prefix`](Self::with_tenant_prefix): settings are user-global,
    /// not per-tenant (see [`crate::core::user_settings`]).
    ///
    /// # Why a tenant cannot reach these objects
    ///
    /// The guarantee is **structural, not lexical**. A settings object sits
    /// *directly* under the `_system.user-settings/` segment as `{digest}.json`,
    /// whereas every tenant-scoped key lives under a `resources/`, `history/`, or
    /// `bulk/` **sub**-prefix of its tenant segment. So even a tenant somehow
    /// named `_system.user-settings` would write to
    /// `_system.user-settings/resources/…`, which can never equal a
    /// `{digest}.json` leaf — and `purge_tenant_data` sweeps only those
    /// sub-prefixes, so it cannot delete a settings object either.
    ///
    /// Do **not** weaken this to "tenant IDs cannot contain `.`". That is true of
    /// the routing validators (`is_valid_tenant_id`), but *not* of
    /// `admin_tenants::validate_tenant_id`, which permits `.` and `/`, nor of the
    /// JWT tenant extractor, which validates nothing. The name is dotted to keep
    /// it unroutable, but the safety of this namespace must not depend on that.
    /// In particular, a future change that widened a tenant purge to sweep the
    /// whole tenant prefix would break the structural argument above and must
    /// exclude this namespace explicitly.
    pub fn user_settings_key(&self, object_id: &str) -> String {
        self.join(&["_system.user-settings", &format!("{object_id}.json")])
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
///
/// This mapping is **lossy and therefore not injective** — `"a/b"` and `"a_b"`
/// both collapse to `"a_b"`. It is only sound for the history *index* keys here,
/// where the filename also carries a timestamp, version, and random suffix and a
/// collision merely duplicates an index entry. Never use it to derive a key that
/// establishes *identity* or *ownership*: two principals colliding on one key is
/// a cross-user data leak. See `S3Keyspace::user_settings_key`, which hashes
/// instead.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ' ' => '_',
            _ => c,
        })
        .collect()
}
