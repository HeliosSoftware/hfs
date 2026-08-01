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
    ///
    /// The id is escaped with [`registry_object_id`], **not** [`sanitize`]: this
    /// key establishes tenant identity and ownership, so a lossy mapping here
    /// would let two tenants share one record (see that function's docs).
    pub fn tenant_registry_key(&self, tenant_id: &str) -> String {
        self.join(&[
            "tenants",
            &format!("{}.json", registry_object_id(tenant_id)),
        ])
    }

    /// The pre-fix key shape for a registry record, which escaped `/`, `\`, and
    /// space to `_` via [`sanitize`] and so collided for ids differing only in
    /// those characters.
    ///
    /// Read-only, for falling back to records written before the escaping fix so
    /// an upgrade does not orphan them. Never write through this. Once existing
    /// deployments have re-registered (or a backfill has run) this and its call
    /// sites in `list_tenants`/`get_tenant`/`deregister_tenant` can be deleted.
    pub fn legacy_tenant_registry_key(&self, tenant_id: &str) -> String {
        self.join(&["tenants", &format!("{}.json", sanitize(tenant_id))])
    }

    /// Prefix covering all tenant registry records.
    ///
    /// # Why tenant data cannot be mistaken for a registry record
    ///
    /// The guarantee is **structural, not lexical**, mirroring
    /// [`user_settings_key`](Self::user_settings_key). A registry record sits
    /// *directly* under this segment as a `{escaped-id}.json` leaf, whereas every
    /// tenant-scoped key lives under a `resources/`, `history/`, or `bulk/`
    /// **sub**-prefix of its tenant segment. So a tenant named `tenants` writes
    /// `tenants/resources/…`, never a `tenants/{leaf}.json`.
    ///
    /// That invariant is only worth anything if the reader enforces it: S3
    /// listings are recursive, so `list_tenants` must keep **direct children
    /// only**. Before that filter existed a tenant named `tenants` injected
    /// phantom rows into the registry and, once it accumulated a history event,
    /// made `list_tenants` fail permanently (issue #271).
    ///
    /// Do **not** weaken this to "no tenant may be named `tenants`". The admin
    /// API does reserve that name, but the JWT tenant extractor validates
    /// nothing, so the safety of this namespace must not depend on it.
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
    /// The raw key is derived from JWT claims the server does not constrain: it
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
    ///
    /// # What a tenant purge *does* reach
    ///
    /// The paragraph above says a tenant purge cannot **delete** these objects,
    /// and that remains true and deliberate: they are user-global, and one
    /// tenant's offboarding must not destroy a user's theme or their other
    /// tenants' saved queries. Since issue #313 a purge instead **edits** each
    /// object, removing the `byTenant.{tenant}` subtree from the document inside
    /// (see [`user_settings_prefix`](Self::user_settings_prefix) and
    /// `crate::core::user_settings::purge_tenant_subtree`). The object survives;
    /// the purged tenant's content in it does not.
    pub fn user_settings_key(&self, object_id: &str) -> String {
        self.join(&["_system.user-settings", &format!("{object_id}.json")])
    }

    /// Prefix covering every per-user settings object.
    ///
    /// Used only by the tenant-settings purge (issue #313), which must visit
    /// every user's document to remove the offboarded tenant's subtree — the
    /// settings store is keyed by an opaque digest of the user key, so there is
    /// no way to select the affected documents other than listing them.
    ///
    /// Structurally disjoint from every tenant's keyspace, by the same argument
    /// as [`user_settings_key`](Self::user_settings_key): a tenant's objects live
    /// under `resources/`, `history/` or `bulk/` sub-prefixes of its own segment,
    /// so listing this prefix can never return one.
    pub fn user_settings_prefix(&self) -> String {
        self.join(&["_system.user-settings/"])
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

/// Escapes a tenant id into a single, injective S3 key segment.
///
/// Unlike [`sanitize`], distinct ids always produce distinct segments, so this
/// is safe for a key that establishes identity. `%` is escaped first so the
/// mapping stays reversible and no other escape can be forged; `/`, `\`, and
/// space — the characters `sanitize` collapses — follow.
///
/// Percent-encoding rather than a hash (as `settings_object_id` uses) because a
/// tenant id is validated and non-secret, and keeping the bucket greppable by
/// tenant is worth more here than fixed-length keys. Ids reaching this point are
/// length-capped upstream, so S3's 1024-byte key limit is not a concern.
///
/// Only `/` is reachable through the admin API's charset — `\` and space are
/// already rejected there — so in practice this changes the key of exactly the
/// hierarchical ids that were previously colliding.
fn registry_object_id(tenant_id: &str) -> String {
    let mut out = String::with_capacity(tenant_id.len());
    for c in tenant_id.chars() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            '\\' => out.push_str("%5C"),
            ' ' => out.push_str("%20"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry key establishes tenant identity: ids that a lossy sanitiser
    /// would collapse together must map to distinct objects. A collision here
    /// lets one tenant read, overwrite, and deregister another's record.
    #[test]
    fn tenant_registry_key_is_injective_for_ids_a_sanitiser_would_collide() {
        let ks = S3Keyspace::new(None);
        // `sanitize()` maps '/', '\\', and ' ' all to '_', so these four collided.
        let keys = [
            ks.tenant_registry_key("a/b"),
            ks.tenant_registry_key("a_b"),
            ks.tenant_registry_key("a\\b"),
            ks.tenant_registry_key("a b"),
        ];
        for (i, a) in keys.iter().enumerate() {
            for b in keys.iter().skip(i + 1) {
                assert_ne!(a, b, "registry keys must be injective");
            }
        }
        // The escape itself must not be forgeable: a literal '%2F' cannot be
        // made to collide with an encoded '/'.
        assert_ne!(
            ks.tenant_registry_key("a/b"),
            ks.tenant_registry_key("a%2Fb")
        );
    }

    /// Every registry record is a direct child of the registry prefix. This is
    /// the invariant `list_tenants` relies on to tell records apart from the
    /// data of a tenant that happens to be named `tenants` (issue #271).
    #[test]
    fn tenant_registry_keys_are_direct_children_of_the_registry_prefix() {
        for base in [None, Some("hfs".to_string())] {
            let ks = S3Keyspace::new(base);
            let prefix = ks.tenant_registry_prefix();
            for id in ["acme", "a/b", "tenants", "resources", "__system__", ".."] {
                let key = ks.tenant_registry_key(id);
                let rest = key
                    .strip_prefix(&prefix)
                    .expect("registry key must sit under the registry prefix");
                assert!(
                    !rest.contains('/'),
                    "registry key for {id:?} must be a direct child, got {rest:?}"
                );
            }
        }
    }

    /// A tenant whose id names the registry segment still writes its data one
    /// or more segments deeper, so no key it can produce is mistaken for a
    /// registry record. This is the structural half of the #271 fix — it holds
    /// regardless of whether tenant-id validation ran.
    #[test]
    fn a_tenant_named_tenants_cannot_forge_a_registry_record() {
        let base = S3Keyspace::new(None);
        let prefix = base.tenant_registry_prefix();
        let hostile = base.with_tenant_prefix("tenants");

        let data_keys = [
            hostile.current_resource_key("Patient", "p1"),
            hostile.history_version_key("Patient", "p1", "1"),
            hostile.history_system_prefix(),
            hostile.resources_prefix(),
            hostile.history_root_prefix(),
        ];
        for key in data_keys {
            assert!(
                key.starts_with(&prefix),
                "precondition: hostile tenant data shares the registry prefix"
            );
            let rest = &key[prefix.len()..];
            assert!(
                rest.contains('/'),
                "tenant data must be nested deeper than a registry record, got {rest:?}"
            );
        }
    }

    /// Traversal in a tenant id cannot walk the registry key out of its
    /// namespace.
    ///
    /// What makes this safe is that `/` is escaped, so the id can never
    /// contribute a separator and the key always stays a single leaf under
    /// `tenants/`. A `..` surviving *within* that leaf is harmless: the leaf
    /// always has `.json` appended, so no path segment is ever exactly `.` or
    /// `..`, and S3 keys carry no path semantics anyway. Assert the property
    /// that actually holds — one leaf, no separator — rather than the absence of
    /// the characters.
    #[test]
    fn registry_key_cannot_be_escaped_by_traversal() {
        let ks = S3Keyspace::new(None);
        for id in ["..", ".", "../..", "a/../../b", "../../../etc/passwd"] {
            let key = ks.tenant_registry_key(id);
            let leaf = key
                .strip_prefix("tenants/")
                .unwrap_or_else(|| panic!("{id:?} escaped the registry namespace: {key}"));
            assert!(
                !leaf.contains('/'),
                "{id:?} injected a separator into the key: {key}"
            );
            // No path segment is ever a bare traversal token.
            for segment in key.split('/') {
                assert!(
                    segment != ".." && segment != ".",
                    "{id:?} produced a traversal segment: {key}"
                );
            }
        }
    }

    /// The legacy key shape is what pre-fix records were written under, and is
    /// read as a fallback. It must agree with the new shape for every id that
    /// `sanitize` did not rewrite, so the fallback is a no-op for them.
    #[test]
    fn legacy_registry_key_differs_only_for_ids_sanitize_rewrote() {
        let ks = S3Keyspace::new(None);
        for id in [
            "acme",
            "acme-research",
            "tenant_123",
            "acme.org",
            "__system__",
        ] {
            assert_eq!(
                ks.tenant_registry_key(id),
                ks.legacy_tenant_registry_key(id),
                "unaffected id {id:?} must keep its existing key"
            );
        }
        for id in ["a/b", "a\\b", "a b"] {
            assert_ne!(
                ks.tenant_registry_key(id),
                ks.legacy_tenant_registry_key(id),
                "previously-colliding id {id:?} must move to a new key"
            );
        }
    }
}
