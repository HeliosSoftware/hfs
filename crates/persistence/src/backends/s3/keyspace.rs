//! S3 key construction for all FHIR storage namespaces.
//!
//! Keys are structured as hierarchical paths that encode the tenant prefix,
//! resource type, resource ID, version, and operation type. [`S3Keyspace`]
//! derives every key shape used by the backend from a common base prefix.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// File name of the layout marker object, directly under `history/`.
const LAYOUT_MARKER_FILE: &str = "_keyspace.json";

/// Number of hex characters in a V2 shard segment (`16^2 = 256` shards).
const SHARD_HEX_CHARS: usize = 2;

/// How a tenant's resource and history objects are laid out in S3.
///
/// The layout is fixed per tenant by a marker object
/// ([`S3Keyspace::layout_marker_key`]); it never changes once stamped, because
/// S3 is the system of record and a derivation change would orphan objects.
///
/// | | `V1` (flat, today) | `V2` (sharded) |
/// |---|---|---|
/// | current | `resources/{type}/{id}/current.json` | `resources/{type}/{h2}/{id}/current.json` |
/// | version | `resources/{type}/{id}/_history/{v}.json` | `resources/{type}/{h2}/{id}/_history/{v}.json` |
/// | type event | `history/type/{type}/{file}` | `history/type/{type}/{h2}/{file}` |
/// | system event | `history/system/{file}` | none — listed from `history/type/` |
///
/// `h2` is the first two lowercase hex characters of `sha256(id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyspaceLayout {
    /// Flat per-type prefixes plus a tenant-wide `history/system/` index.
    V1,
    /// Resources and type history sharded 256 ways by id hash; no system index.
    V2,
}

impl KeyspaceLayout {
    /// The number stored in the marker object.
    pub fn version(self) -> u8 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// Inverse of [`version`](Self::version); `None` for an unknown version.
    pub fn from_version(version: u64) -> Option<Self> {
        match version {
            1 => Some(Self::V1),
            2 => Some(Self::V2),
            _ => None,
        }
    }

    /// Parses `v1` / `v2`, case-insensitively and trimmed.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "v1" => Some(Self::V1),
            "v2" => Some(Self::V2),
            _ => None,
        }
    }

    /// Body of the marker object for this layout.
    pub fn marker_body(self) -> Vec<u8> {
        format!("{{\"layout\":{}}}", self.version()).into_bytes()
    }

    /// Reads a layout out of a marker object body. `None` when the body is not
    /// a JSON object with a known integer `layout`.
    pub fn from_marker_body(body: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        Self::from_version(value.get("layout")?.as_u64()?)
    }
}

/// The two-hex-character shard segment for `id` (V2 layout).
pub fn shard_segment(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let mut shard = hex_digest(&digest[..SHARD_HEX_CHARS / 2]);
    shard.truncate(SHARD_HEX_CHARS);
    shard
}

/// Every shard segment, `00`..`ff`, in lexicographic order.
fn all_shard_segments() -> impl Iterator<Item = String> {
    (0..=255u8).map(|byte| format!("{byte:02x}"))
}

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
    /// directory segment without changing the bucket. This is the *only* place
    /// a tenant id becomes a data-key location, reached through the single
    /// funnel `S3Backend::tenant_location`, so the two guarantees below are
    /// what tenant isolation on S3 rests on.
    ///
    /// # Guarantees
    ///
    /// 1. **Injective** — distinct ids yield distinct prefixes.
    /// 2. **Prefix-disjoint** — no tenant's *sweep* prefix
    ///    (`{prefix}/resources/`, `{prefix}/history/`, `{prefix}/bulk/`) is a
    ///    prefix of another tenant's keys.
    ///
    /// Both are needed, and neither implies the other. (1) alone would still let
    /// a tenant named `acme/resources` nest its whole keyspace inside the prefix
    /// `acme` sweeps; (2) alone would still let `/a` and `a` share one prefix.
    ///
    /// # Why this is not a plain escape of the whole id
    ///
    /// S3 is the **system of record**, not a derived index: there is no
    /// `$reindex`-equivalent that could rebuild objects at a new key. A
    /// derivation change does not move objects — it makes the old ones
    /// unreachable, and a later `purge_tenant_data` will not reach them either.
    /// So the derivation escapes exactly the ids that are *not already safe* and
    /// is the **identity** for every id that is:
    ///
    /// | tenant id | before | after |
    /// |---|---|---|
    /// | `acme`            | `acme`            | `acme` (unchanged) |
    /// | `acme/research`   | `acme/research`   | `acme/research` (unchanged) |
    /// | `__system__`      | `__system__`      | `__system__` (unchanged) |
    /// | `/acme`           | `acme` ⚠ shared   | `%2Facme` |
    /// | `acme/`           | `acme` ⚠ shared   | `acme%2F` |
    /// | `//acme`          | `acme` ⚠ shared   | `%2F%2Facme` |
    /// | `acme/resources`  | `acme/resources` ⚠ inside `acme`'s sweep | `acme%2Fresources` |
    ///
    /// Every id whose prefix moves is one whose objects are *currently*
    /// commingled with another tenant's, or sitting inside another tenant's
    /// purge radius. There is no coherent single-tenant dataset at the old
    /// location to preserve — relocating it **is** the remediation. Every id a
    /// deployment can already be storing safely keeps its exact prefix, so an
    /// upgrade moves nothing that was working. See issue #447.
    ///
    /// # Why the guarantees hold
    ///
    /// [`is_keyspace_safe`] rejects any id containing a character
    /// [`registry_object_id`] escapes (`%`, `\`, space) and any id whose
    /// structure is unsafe (an empty segment, or a sweep-root segment after the
    /// first) — and every such structural defect implies the id contains `/`.
    /// So an escaped prefix always contains `%`, and a safe prefix never can:
    /// the two ranges are disjoint, and each mapping is injective on its own
    /// domain, giving (1). For (2), an escaped prefix contains no `/` at all and
    /// so cannot end in `/{sweep-root}`, while a safe prefix is barred from a
    /// non-first sweep-root segment by construction.
    pub fn with_tenant_prefix(&self, tenant_id: &str) -> Self {
        let tenant = tenant_prefix_component(tenant_id);
        let merged = match &self.base_prefix {
            Some(base) => format!("{}/{}", base, tenant),
            None => tenant,
        };
        // `new`'s slash trim is a no-op on the tenant component by construction
        // (a safe id has no empty segment, an escaped one has no `/`), so it can
        // no longer erase the distinction between `a`, `/a`, `a/`, and `//a`.
        Self::new(Some(merged))
    }

    /// Prefix covering all current resource objects across all types.
    ///
    /// Resource and history keys below this prefix depend on the tenant's
    /// layout and are built by [`ResourceKeyspace`].
    pub fn resources_prefix(&self) -> String {
        self.join(&["resources/"])
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

    /// Key of the per-tenant keyspace layout marker (see [`KeyspaceLayout`]).
    ///
    /// It lives under `history/` so a tenant sweep of that prefix reaches it
    /// and no `resources/` probe ever sees it. `purge_tenant_data` must skip it:
    /// a tenant's layout is immutable once stamped.
    pub fn layout_marker_key(&self) -> String {
        self.join(&["history", LAYOUT_MARKER_FILE])
    }

    /// Binds this keyspace to a resolved [`KeyspaceLayout`], yielding the only
    /// type that can build resource and history keys.
    pub fn with_layout(&self, layout: KeyspaceLayout) -> ResourceKeyspace {
        ResourceKeyspace {
            keyspace: self.clone(),
            layout,
        }
    }

    /// Key for the JSON state object of a bulk submission.
    pub fn submit_state_key(&self, submitter: &str, submission_id: &str) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "state.json",
        ])
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
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "manifests",
            &format!("{}.json", submit_segment(manifest_id)),
        ])
    }

    /// Key for a single raw NDJSON line within a submission manifest.
    ///
    /// `file_url` names the manifest output file the line came from; see
    /// [`submit_file_segment`] for why it is part of the key.
    /// Key for the raw NDJSON archive of one ingest batch (chunk) of a file.
    ///
    /// One object holds every line of the batch, keyed by the batch's first
    /// line number so successive chunks of the same file (each a separate
    /// `process_entries` call) never collide (#1429). `file_url` names the
    /// manifest output file the lines came from; see [`submit_file_segment`]
    /// for why it is part of the key (two files' line-1 batches must differ).
    pub fn submit_raw_batch_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        file_url: Option<&str>,
        first_line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "raw",
            &submit_segment(manifest_id),
            &submit_file_segment(file_url),
            &format!("batch-{}.ndjson", first_line),
        ])
    }

    /// Key for the processing result of a single NDJSON line — the legacy
    /// per-line receipt shape, which the ingest no longer writes (#1429) but
    /// the readers still accept; kept so the tests can plant one.
    ///
    /// `file_url` names the manifest output file the line came from; see
    /// [`submit_file_segment`] for why it is part of the key.
    #[cfg(test)]
    pub fn submit_result_line_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        file_url: Option<&str>,
        line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "results",
            &submit_segment(manifest_id),
            &submit_file_segment(file_url),
            &format!("line-{}.json", line),
        ])
    }

    /// Key for one ingest batch's coalesced receipts — every entry result the
    /// batch produced, in a single object rather than one per line (#1429).
    ///
    /// Sits under the same `results/<manifest>/` prefix that
    /// [`Self::submit_result_line_key`] writes and that the receipt readers
    /// sweep, nested by file and keyed by the batch's first line exactly like
    /// the raw archive and the change log, so batches of one file never
    /// collide and two files' batches stay apart (#457).
    pub fn submit_result_batch_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        file_url: Option<&str>,
        first_line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "results",
            &submit_segment(manifest_id),
            &submit_file_segment(file_url),
            &format!("batch-{:020}.json", first_line),
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
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "changes",
            &format!("{}.json", submit_segment(change_id)),
        ])
    }

    /// Key for one ingest batch's coalesced change log — every change the batch
    /// recorded, in a single object rather than one per resource (#1429).
    ///
    /// Sits under the same `changes/` prefix that [`Self::submit_change_key`]
    /// writes and that `load_changes` lists, but nested by manifest and file so
    /// batches never collide: like the raw archive and the entry receipts, the
    /// key is discriminated by `file_url` (line numbers restart per file, see
    /// [`submit_file_segment`]) and keyed by the batch's first line.
    pub fn submit_change_batch_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        file_url: Option<&str>,
        first_line: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "changes",
            &submit_segment(manifest_id),
            &submit_file_segment(file_url),
            &format!("batch-{}.json", first_line),
        ])
    }

    /// Key for one finalized status-manifest artifact row of a submission.
    ///
    /// The identity is `(submitter, submission_id, manifest_id, file_type,
    /// resource_type, part_index, fencing_token)`, matching the manifest-aware
    /// `bulk_submit_files` uniqueness the SQL backends carry. The fencing token
    /// is in the key so a worker that re-writes an artifact after reclaiming a
    /// manifest replaces its own row and never collides with the row a
    /// *previous* lease holder left behind.
    #[allow(clippy::too_many_arguments)] // The complete persisted artifact identity.
    pub fn submit_file_key(
        &self,
        submitter: &str,
        submission_id: &str,
        manifest_id: &str,
        file_type: &str,
        resource_type: Option<&str>,
        part_index: u32,
        fencing_token: u64,
    ) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "files",
            &format!(
                "{}.json",
                submit_file_object_id(
                    submitter,
                    submission_id,
                    manifest_id,
                    file_type,
                    resource_type,
                    part_index,
                    fencing_token,
                )
            ),
        ])
    }

    /// Prefix covering the status-manifest artifact rows of a submission.
    pub fn submit_files_prefix(&self, submitter: &str, submission_id: &str) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "files/",
        ])
    }

    /// Prefix covering all objects belonging to a single submission.
    pub fn submit_prefix(&self, submitter: &str, submission_id: &str) -> String {
        self.join(&[
            "bulk",
            "submit",
            &submit_segment(submitter),
            &submit_segment(submission_id),
            "/",
        ])
    }

    /// Key for one entry in the cross-tenant `$bulk-submit` worker index.
    ///
    /// `namespace` is `queue` (manifests awaiting or under a worker lease),
    /// `tokens` (poll token → submission), or `submissions` (TTL sweep records).
    /// `object_id` **must** be an opaque, injective digest — see
    /// [`submit_index_object_id`] — never a raw submitter, submission id, or
    /// poll token: all three are client-supplied, and `sanitize` is lossy, so two
    /// submissions could otherwise collide on one index entry and a poll token
    /// could be made to resolve to another submitter's submission.
    ///
    /// Like [`user_settings_key`](Self::user_settings_key) this is built from the
    /// *base* keyspace, not a tenant's: the worker claims manifests and resolves
    /// poll tokens with no tenant in hand, so the index must span tenants. The
    /// same structural argument applies — index objects sit under a `queue/`,
    /// `tokens/`, or `submissions/` segment of `_system.bulk-submit/`, whereas
    /// every tenant-scoped key lives under a `resources/`, `history/`, or `bulk/`
    /// Key for one provider-side Bulk Submit submission (#772). Tenant-scoped:
    /// it lives under the tenant's `bulk/` segment beside the consumer-side
    /// submit state, so tenant deletion and IAM prefixes cover it.
    pub fn bulk_provider_key(&self, object_id: &str) -> String {
        self.join(&["bulk", "provider-submissions", &format!("{object_id}.json")])
    }

    /// Prefix holding every provider-side submission of the tenant.
    pub fn bulk_provider_prefix(&self) -> String {
        self.join(&["bulk", "provider-submissions", "/"])
    }

    /// Key of the tenant's SQL-on-FHIR terminology registry (#2185): which
    /// generation object holds each ValueSet/ConceptMap relation, and which
    /// Libraries reference it.
    ///
    /// Tenant-scoped under the `bulk/` sweep root, like
    /// [`bulk_provider_key`](Self::bulk_provider_key), rather than under a new
    /// root: adding a root to `SWEEP_ROOT_SEGMENTS` would change which tenant
    /// ids [`with_tenant_prefix`](Self::with_tenant_prefix) escapes, and so
    /// move objects S3 is the system of record for.
    pub fn sof_terminology_registry_key(&self) -> String {
        self.join(&["bulk", "sof-terminology", "registry.json"])
    }

    /// Key of one terminology generation object. `generation` is a UUID
    /// (path-safe by construction).
    pub fn sof_terminology_generation_key(&self, generation: &str) -> String {
        self.join(&[
            "bulk",
            "sof-terminology",
            "generations",
            &format!("{generation}.json"),
        ])
    }

    /// Prefix holding all of the tenant's terminology objects (tenant purge).
    pub fn sof_terminology_prefix(&self) -> String {
        self.join(&["bulk", "sof-terminology", "/"])
    }

    /// sub-prefix of its tenant segment, so even a tenant named
    /// `_system.bulk-submit` cannot reach one. (`_system.bulk-submit` is also in
    /// [`RESERVED_TENANT_SEGMENTS`](crate::tenant::RESERVED_TENANT_SEGMENTS), as
    /// defence in depth rather than as the proof.)
    pub fn submit_index_key(&self, namespace: &str, object_id: &str) -> String {
        self.join(&[
            "_system.bulk-submit",
            namespace,
            &format!("{object_id}.json"),
        ])
    }

    /// Prefix covering one namespace of the cross-tenant worker index.
    pub fn submit_index_prefix(&self, namespace: &str) -> String {
        self.join(&["_system.bulk-submit", &format!("{namespace}/")])
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
    /// Do **not** weaken this to "tenant IDs cannot contain `.`". That was never
    /// reliably true, and since issue #385 it is plainly false: the canonical
    /// charset ([`TenantId::parse`](crate::tenant::TenantId::parse)) permits `.`
    /// on every ingress, so `_system.user-settings` is a *well-formed* tenant id.
    /// It is refused only because it is listed in
    /// [`RESERVED_TENANT_SEGMENTS`](crate::tenant::RESERVED_TENANT_SEGMENTS) —
    /// and that list is a guardrail, not this namespace's safety proof.
    ///
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

    /// Object key for one web UI login session or pending login (#1481):
    /// `kind` is `session` or `pending`, `id` the opaque random id (URL-safe
    /// base64, so path-safe and fixed-length by construction).
    pub fn login_session_key(&self, kind: &str, id: &str) -> String {
        self.join(&["_system.login-sessions", kind, &format!("{id}.json")])
    }

    /// Prefix covering every login object of one `kind`, for the sweep.
    pub fn login_sessions_prefix(&self, kind: &str) -> String {
        self.join(&["_system.login-sessions", &format!("{kind}/")])
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

/// Resource and history key builder bound to a tenant's resolved
/// [`KeyspaceLayout`].
///
/// Obtained only through [`S3Keyspace::with_layout`], which the backend calls
/// after reading the tenant's marker, so a resource or history key cannot be
/// built without the layout having been resolved. Under `V1` every method
/// produces exactly the key the plain [`S3Keyspace`] method of the same name
/// does.
#[derive(Debug, Clone)]
pub struct ResourceKeyspace {
    keyspace: S3Keyspace,
    layout: KeyspaceLayout,
}

impl ResourceKeyspace {
    /// The layout these keys follow.
    pub fn layout(&self) -> KeyspaceLayout {
        self.layout
    }

    /// The layout-independent keyspace (marker, bulk, registry, sweep roots).
    pub fn base(&self) -> &S3Keyspace {
        &self.keyspace
    }

    /// Key for the mutable "current" pointer of a resource.
    pub fn current_resource_key(&self, resource_type: &str, id: &str) -> String {
        match self.layout {
            KeyspaceLayout::V1 => {
                self.keyspace
                    .join(&["resources", resource_type, id, "current.json"])
            }
            KeyspaceLayout::V2 => self.keyspace.join(&[
                "resources",
                resource_type,
                &shard_segment(id),
                id,
                "current.json",
            ]),
        }
    }

    /// Immutable key for a specific historical version of a resource.
    pub fn history_version_key(&self, resource_type: &str, id: &str, version_id: &str) -> String {
        match self.layout {
            KeyspaceLayout::V1 => self.keyspace.join(&[
                "resources",
                resource_type,
                id,
                "_history",
                &format!("{}.json", version_id),
            ]),
            KeyspaceLayout::V2 => self.keyspace.join(&[
                "resources",
                resource_type,
                &shard_segment(id),
                id,
                "_history",
                &format!("{}.json", version_id),
            ]),
        }
    }

    /// Prefix covering all history version objects for one resource.
    pub fn history_versions_prefix(&self, resource_type: &str, id: &str) -> String {
        match self.layout {
            KeyspaceLayout::V1 => {
                self.keyspace
                    .join(&["resources", resource_type, id, "_history/"])
            }
            KeyspaceLayout::V2 => self.keyspace.join(&[
                "resources",
                resource_type,
                &shard_segment(id),
                id,
                "_history/",
            ]),
        }
    }

    /// Prefix covering every object of a resource type, in either layout.
    pub fn resource_type_prefix(&self, resource_type: &str) -> String {
        self.keyspace.join(&["resources", resource_type, "/"])
    }

    /// The prefixes a type-wide listing can fan out over: the type prefix
    /// itself under `V1`, one prefix per shard (`00`..`ff`, in order) under
    /// `V2`. Concatenating the listings in this order equals a single listing
    /// of [`resource_type_prefix`](Self::resource_type_prefix).
    pub fn resource_shard_prefixes(&self, resource_type: &str) -> Vec<String> {
        match self.layout {
            KeyspaceLayout::V1 => vec![self.resource_type_prefix(resource_type)],
            KeyspaceLayout::V2 => all_shard_segments()
                .map(|shard| {
                    self.keyspace
                        .join(&["resources", resource_type, &shard, "/"])
                })
                .collect(),
        }
    }

    /// Prefix covering all current objects across all types (layout-independent).
    pub fn resources_prefix(&self) -> String {
        self.keyspace.resources_prefix()
    }

    /// Prefix covering all history objects (layout-independent).
    pub fn history_root_prefix(&self) -> String {
        self.keyspace.history_root_prefix()
    }

    /// Key for a type-level history index event.
    pub fn history_type_event_key(
        &self,
        resource_type: &str,
        timestamp: DateTime<Utc>,
        id: &str,
        version_id: &str,
        suffix: &str,
    ) -> String {
        let file = format!(
            "{}_{}_{}_{}.json",
            timestamp.timestamp_millis(),
            sanitize(id),
            version_id,
            suffix
        );
        match self.layout {
            KeyspaceLayout::V1 => self
                .keyspace
                .join(&["history", "type", resource_type, &file]),
            KeyspaceLayout::V2 => {
                self.keyspace
                    .join(&["history", "type", resource_type, &shard_segment(id), &file])
            }
        }
    }

    /// Key for a system-level history index event; `None` under `V2`, which
    /// has no system index (system history is listed from `history/type/`).
    pub fn history_system_event_key(
        &self,
        resource_type: &str,
        timestamp: DateTime<Utc>,
        id: &str,
        version_id: &str,
        suffix: &str,
    ) -> Option<String> {
        match self.layout {
            KeyspaceLayout::V1 => Some(self.keyspace.join(&[
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
            ])),
            KeyspaceLayout::V2 => None,
        }
    }

    /// Prefix covering all type-level history events of a resource type.
    pub fn history_type_prefix(&self, resource_type: &str) -> String {
        self.keyspace.join(&["history", "type", resource_type, "/"])
    }

    /// Per-shard prefixes for type-level history (see
    /// [`resource_shard_prefixes`](Self::resource_shard_prefixes)).
    pub fn history_type_shard_prefixes(&self, resource_type: &str) -> Vec<String> {
        match self.layout {
            KeyspaceLayout::V1 => vec![self.history_type_prefix(resource_type)],
            KeyspaceLayout::V2 => all_shard_segments()
                .map(|shard| {
                    self.keyspace
                        .join(&["history", "type", resource_type, &shard, "/"])
                })
                .collect(),
        }
    }

    /// The one type-history prefix that can hold events of resource `id`: its
    /// shard under `V2`, the whole type prefix under `V1`.
    pub fn history_type_shard_prefix_for(&self, resource_type: &str, id: &str) -> String {
        match self.layout {
            KeyspaceLayout::V1 => self.history_type_prefix(resource_type),
            KeyspaceLayout::V2 => {
                self.keyspace
                    .join(&["history", "type", resource_type, &shard_segment(id), "/"])
            }
        }
    }

    /// Prefix covering the type-level history of every type (`history/type/`).
    /// Under `V2` this is what system history is served from.
    pub fn history_types_root_prefix(&self) -> String {
        self.keyspace.join(&["history", "type/"])
    }

    /// Prefix covering system-level history events; `None` under `V2`.
    pub fn history_system_prefix(&self) -> Option<String> {
        match self.layout {
            KeyspaceLayout::V1 => Some(self.keyspace.join(&["history", "system/"])),
            KeyspaceLayout::V2 => None,
        }
    }
}

const SUBMIT_FILE_KEY_VERSION: u8 = 1;

/// Builds the opaque object id for one manifest-aware submit artifact row.
///
/// Raw ids and optional resource types are serialized as an injective typed
/// JSON tuple — `None` and `Some("")` remain distinct — then SHA-256 digested.
/// The digest is collision-resistant, not mathematically injective, but it keeps
/// hostile slashes out of S3 paths while preserving the stable `files/` prefix.
fn submit_file_object_id(
    submitter: &str,
    submission_id: &str,
    manifest_id: &str,
    file_type: &str,
    resource_type: Option<&str>,
    part_index: u32,
    fencing_token: u64,
) -> String {
    let payload = serde_json::to_vec(&(
        SUBMIT_FILE_KEY_VERSION,
        submitter,
        submission_id,
        manifest_id,
        file_type,
        resource_type,
        part_index,
        fencing_token,
    ))
    .expect("canonical submit-file key payload must be valid JSON");
    let digest = Sha256::digest(&payload);
    format!("{SUBMIT_FILE_KEY_VERSION}-{}", hex_digest(&digest))
}

fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
/// Escapes one client-supplied `$bulk-submit` identifier (the submitter,
/// the submission id, a manifest or change id) into a single S3 key segment.
///
/// These keys establish the submission's identity and ownership, so the
/// mapping must be injective: `%` is escaped first and then the characters a
/// key path cannot carry — `/` (which would split the segment; a URL system
/// such as `http://example.org|smoke` also puts a `//` in the key, which
/// MinIO refuses outright), `\`, and control characters. A segment that is
/// only dots is escaped too, since `.` and `..` are path components S3
/// stores refuse. Everything else, including `|`, `:` and Unicode, is kept,
/// so an identifier with none of those characters produces the same key it
/// always did.
fn submit_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            '\\' => out.push_str("%5C"),
            c if c.is_control() => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    if !out.is_empty() && out.bytes().all(|b| b == b'.') {
        return out.replace('.', "%2E");
    }
    out
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ' ' => '_',
            _ => c,
        })
        .collect()
}

/// Escapes a manifest output file URL into one S3 key segment.
///
/// Line numbers restart at 1 in every output file of a manifest, so the file a
/// line came from is part of that entry's identity. Without this segment the
/// second file's line 1 silently overwrites the first file's (issue #457 — the
/// SQL backends carry the same discriminator in the `bulk_entry_results`
/// primary key, where the collision surfaces as a UNIQUE violation instead).
///
/// Hashed rather than escaped, for the reasons `settings_object_id` documents:
/// the URL comes from a submitted manifest, is unbounded in length, and is full
/// of `/` — so escaping it would risk both a lossy collision (the very bug this
/// fixes) and a key that walks out of the manifest's prefix. The digest is over
/// raw bytes, so no Unicode normalisation is applied: two URLs differing only in
/// normalisation form get two prefixes, which is the fail-safe direction.
///
/// `None` — a `process_entries` call made outside any manifest file — yields an
/// empty segment, which [`S3Keyspace::join`] drops, reproducing the pre-#457
/// flat layout. That is deliberate on two counts: it mirrors the SQL backends'
/// `''` default for the same case, and it leaves entry results written before
/// this change readable, since they still sit under the `results/<manifest>/`
/// prefix that the receipt readers sweep recursively.
fn submit_file_segment(file_url: Option<&str>) -> String {
    match file_url {
        Some(url) if !url.is_empty() => {
            let digest = Sha256::digest(url.as_bytes());
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        }
        _ => String::new(),
    }
}

/// Digests the identifying parts of a cross-tenant `$bulk-submit` index entry
/// into one opaque, injective key segment.
///
/// Every input here is client-supplied — the submitter, the submission id, the
/// manifest id, and a poll token — so this hashes rather than escapes, for the
/// same reason `settings_object_id` does: a lossy mapping would let two
/// submissions share one queue entry, or let a crafted poll token resolve to
/// another submitter's submission.
///
/// Parts are joined with `\u{1}`, a byte no caller can supply through a URL path
/// or JSON string identifier, so `["a", "b/c"]` and `["a/b", "c"]` digest
/// differently.
pub fn submit_index_object_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0x01]);
        }
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Escapes a tenant id into a single, injective S3 key segment.
///
/// Unlike [`sanitize`], distinct ids always produce distinct segments, so this
/// is safe for a key that establishes identity. `%` is escaped first so the
/// mapping stays reversible and no other escape can be forged; `/`, `\`, and
/// space — the characters `sanitize` collapses — follow.
///
/// Used for the tenant registry key and, for the ids [`is_keyspace_safe`]
/// rejects, for the tenant data prefix as well — see
/// [`S3Keyspace::with_tenant_prefix`] for why the latter is conditional.
///
/// Percent-encoding rather than a hash (as `settings_object_id` uses) because a
/// tenant id is validated and non-secret, and keeping the bucket greppable by
/// tenant is worth more here than fixed-length keys. Ids reaching this point are
/// length-capped upstream, so S3's 1024-byte key limit is not a concern.
///
/// Only `/` is reachable through the canonical charset
/// ([`TenantId::parse`](crate::tenant::TenantId::parse)) — `\`, space, and `%`
/// are rejected on every ingress since issue #385 — so in practice this changes
/// the key of exactly the hierarchical ids that were previously colliding. The
/// `\`/space/`%` arms are kept anyway: this function's contract is to be
/// injective for *any* input, so it does not inherit the validator's bounds.
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

/// Path segments the keyspace reserves *beneath* a tenant prefix.
///
/// These are the roots every tenant-scoped key hangs off, and — the reason they
/// matter here — the prefixes `S3Backend::purge_tenant_data` sweeps and
/// `list_current_keys` enumerates. A tenant whose id carried one of these as a
/// later segment would place its entire keyspace *inside* another tenant's
/// radius: tenant `acme` sweeps `acme/resources/`, which contains every object
/// of a tenant literally named `acme/resources` — a cross-tenant purge, and a
/// cross-tenant read, since `list_current_keys` selects on `/current.json`
/// anywhere under the prefix.
///
/// `tenants` and `_system.user-settings` are deliberately absent: those live on
/// the *base* keyspace, never under a tenant prefix, and are already protected
/// structurally by the direct-child filter in `list_tenants` and the digest leaf
/// in `user_settings_key` (see [`S3Keyspace::tenant_registry_prefix`], #271).
/// Adding them here would relocate a top-level tenant named `tenants` for no
/// gain.
const SWEEP_ROOT_SEGMENTS: [&str; 3] = ["resources", "history", "bulk"];

/// Prefix component standing in for the empty tenant id.
///
/// `registry_object_id("")` is `""`, which would collapse the tenant prefix into
/// the *base* keyspace — the one namespace no tenant may occupy. A bare `%` is
/// used instead: [`registry_object_id`] emits `%` only as the first byte of a
/// three-byte escape, so no non-empty id can produce it, and [`is_keyspace_safe`]
/// bars `%` from every unescaped prefix. The empty id is unreachable through
/// every ingress today; this exists so the derivation is total and disjoint
/// without depending on that.
const EMPTY_TENANT_PREFIX: &str = "%";

/// Whether `tenant_id` can be used verbatim as its tenant prefix component.
///
/// This is deliberately the **complement of "currently broken"**: an id is unsafe
/// exactly when using it raw would either collide with another id's prefix or
/// nest inside another tenant's sweep radius. Keeping the predicate that tight is
/// what makes [`S3Keyspace::with_tenant_prefix`] an identity for every id a
/// deployment can already be storing safely — see that function for why moving
/// objects on a system of record is the cost that governs this design.
///
/// Do **not** widen this into a general tenant-id validator. It answers one
/// question — "is this id safe as an S3 key prefix?" — and nothing here may
/// depend on ingress validation having run: `purge_tenant_data` constructs its
/// `TenantId` directly from an operator-supplied path segment, with no validator
/// in the path. (The canonical ingress validator is issue #385 / PR #450; it
/// bounds the input, it does not make a lossy mapping safe.)
fn is_keyspace_safe(tenant_id: &str) -> bool {
    // The empty id has no prefix of its own; see `EMPTY_TENANT_PREFIX`.
    if tenant_id.is_empty() {
        return false;
    }
    // Exactly the characters `registry_object_id` escapes. Barring them from the
    // unescaped range is what keeps the two ranges disjoint, and hence the whole
    // derivation injective.
    if tenant_id.contains(['%', '\\', ' ']) {
        return false;
    }

    let mut segments = tenant_id.split('/');
    // An empty first segment is a leading `/` — the many-to-one case issue #447
    // is named for (`a`, `/a`, `a/`, `//a` all trimmed to `a`).
    let Some(first) = segments.next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    for segment in segments {
        // Empty: a trailing or repeated `/`.
        if segment.is_empty() {
            return false;
        }
        // A sweep root anywhere but the first segment nests this tenant inside
        // the parent id's purge/list radius. The first segment is exempt: it
        // sits directly under the base prefix, where nothing sweeps a bare
        // `resources/`, so a top-level tenant named `resources` is harmless and
        // must not be relocated.
        if SWEEP_ROOT_SEGMENTS.contains(&segment) {
            return false;
        }
    }
    true
}

/// Derives the tenant's prefix component: the id itself when
/// [`is_keyspace_safe`], otherwise one opaque [`registry_object_id`] segment.
fn tenant_prefix_component(tenant_id: &str) -> String {
    if is_keyspace_safe(tenant_id) {
        return tenant_id.to_string();
    }
    let escaped = registry_object_id(tenant_id);
    if escaped.is_empty() {
        return EMPTY_TENANT_PREFIX.to_string();
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tenant prefix derivation (issue #447) ──────────────────────────────
    //
    // `with_tenant_prefix` is the single derivation every S3 data key passes
    // through, so these are property assertions over an adversarial corpus
    // rather than example-by-example checks: the properties are what tenant
    // isolation rests on, and a new id shape should be added to the corpus
    // rather than given its own test.

    /// Ids that collided under the old `trim_matches('/')`, ids that nested
    /// inside another tenant's sweep radius, ordinary ids that must not move,
    /// and forged-escape attempts.
    const TENANT_ID_CORPUS: &[&str] = &[
        // Ordinary ids — the derivation must be the identity for these.
        "default",
        "acme",
        "__system__",
        "acme-research",
        "tenant_123",
        "acme.org",
        "acme/research",
        "acme/research/oncology",
        // Slash padding: all of these trimmed to `acme` before the fix.
        "/acme",
        "acme/",
        "//acme",
        "acme//",
        "/acme/",
        "acme//research",
        // Nested inside a parent tenant's purge/list radius.
        "acme/resources",
        "acme/history",
        "acme/bulk",
        "acme/research/resources",
        // Top-level namespace names: harmless (nothing sweeps a bare
        // `resources/`), so these must NOT be relocated.
        "resources",
        "history",
        "bulk",
        "tenants",
        // Characters the escape itself uses — a literal must not be able to
        // forge another id's escaped form.
        "%2Facme",
        "acme%2Fresearch",
        "acme%25",
        "acme\\research",
        "acme research",
        // Degenerate.
        "",
    ];

    /// The subset of the corpus that must round-trip unchanged. Anything here
    /// that started moving would be silent data loss on upgrade: S3 is the
    /// system of record and there is no `$reindex` to rebuild objects at a new
    /// key. `__system__` in particular holds every shared terminology resource.
    const UNCHANGED_TENANT_IDS: &[&str] = &[
        "default",
        "acme",
        "__system__",
        "acme-research",
        "tenant_123",
        "acme.org",
        "acme/research",
        "acme/research/oncology",
        "resources",
        "history",
        "bulk",
        "tenants",
    ];

    /// Every key a tenant can write, for prefix-containment checks.
    fn sample_keys(ks: &S3Keyspace) -> Vec<String> {
        let mut keys = vec![ks.submit_state_key("submitter", "sub-1")];
        for layout in [KeyspaceLayout::V1, KeyspaceLayout::V2] {
            let rk = ks.with_layout(layout);
            keys.push(rk.current_resource_key("Patient", "p1"));
            keys.push(rk.history_version_key("Patient", "p1", "2"));
            keys.push(rk.resource_type_prefix("Patient"));
            keys.push(rk.history_type_event_key("Patient", event_time(), "p1", "2", "s"));
            keys.extend(rk.history_system_prefix());
        }
        keys.push(ks.layout_marker_key());
        keys
    }

    /// The prefixes a tenant sweeps on purge / enumerates on list. Anything of
    /// another tenant's underneath one of these is a cross-tenant purge and a
    /// cross-tenant read.
    fn sweep_prefixes(ks: &S3Keyspace) -> Vec<String> {
        vec![
            ks.resources_prefix(),
            ks.history_root_prefix(),
            ks.submit_root_prefix(),
        ]
    }

    /// Guarantee 1: distinct tenant ids never share a prefix.
    ///
    /// Before the fix `trim_matches('/')` made this many-to-one — `acme`,
    /// `/acme`, `acme/` and `//acme` all resolved to `acme`, so the registry
    /// held four tenants while the keyspace held one (issue #447).
    #[test]
    fn tenant_prefix_derivation_is_injective() {
        for base in [None, Some("hfs".to_string())] {
            let ks = S3Keyspace::new(base.clone());
            for (i, x) in TENANT_ID_CORPUS.iter().enumerate() {
                for y in TENANT_ID_CORPUS.iter().skip(i + 1) {
                    assert_ne!(
                        ks.with_tenant_prefix(x).resources_prefix(),
                        ks.with_tenant_prefix(y).resources_prefix(),
                        "tenants {x:?} and {y:?} share a data prefix (base={base:?})"
                    );
                }
            }
        }
    }

    /// Guarantee 2: no tenant's sweep prefix covers another tenant's keys.
    ///
    /// Injectivity alone does not give this. Tenant `acme/resources` has a
    /// prefix all its own, yet every one of its objects used to sit under
    /// `acme/resources/` — exactly what `purge_tenant_data("acme")` deletes and
    /// `list_current_keys` enumerates.
    #[test]
    fn no_tenant_sweep_prefix_covers_another_tenants_keys() {
        for base in [None, Some("hfs".to_string())] {
            let ks = S3Keyspace::new(base.clone());
            for x in TENANT_ID_CORPUS {
                for y in TENANT_ID_CORPUS {
                    if x == y {
                        continue;
                    }
                    let victim = ks.with_tenant_prefix(y);
                    for prefix in sweep_prefixes(&ks.with_tenant_prefix(x)) {
                        for key in sample_keys(&victim) {
                            assert!(
                                !key.starts_with(&prefix),
                                "tenant {x:?} sweeping {prefix:?} would reach \
                                 tenant {y:?}'s key {key:?} (base={base:?})"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The derivation is the identity for every id that is already safe, so an
    /// upgrade relocates no object that was working. This is the constraint that
    /// ruled out escaping the whole id unconditionally.
    #[test]
    fn safe_tenant_ids_keep_their_existing_prefix() {
        for base in [None, Some("hfs".to_string())] {
            let ks = S3Keyspace::new(base.clone());
            for id in UNCHANGED_TENANT_IDS {
                let expected = match &base {
                    Some(b) => format!("{b}/{id}/resources/"),
                    None => format!("{id}/resources/"),
                };
                assert_eq!(
                    ks.with_tenant_prefix(id).resources_prefix(),
                    expected,
                    "id {id:?} must keep its pre-fix prefix (base={base:?})"
                );
            }
        }
    }

    /// The ids that *do* move, and where to. These are exactly the ids whose
    /// objects are currently commingled with another tenant's or sitting inside
    /// another tenant's purge radius, so relocating them is the remediation —
    /// there is no coherent single-tenant dataset at the old location.
    #[test]
    fn unsafe_tenant_ids_move_to_one_escaped_segment() {
        let ks = S3Keyspace::new(None);
        for (id, expected) in [
            ("/acme", "%2Facme"),
            ("acme/", "acme%2F"),
            ("//acme", "%2F%2Facme"),
            ("acme/resources", "acme%2Fresources"),
            ("acme//research", "acme%2F%2Fresearch"),
            ("acme\\research", "acme%5Cresearch"),
            ("acme research", "acme%20research"),
            ("%2Facme", "%252Facme"),
            ("", "%"),
        ] {
            assert_eq!(
                ks.with_tenant_prefix(id).resources_prefix(),
                format!("{expected}/resources/"),
                "id {id:?}"
            );
        }
    }

    /// The escape must not be forgeable: a tenant whose id is the literal text
    /// `%2Facme` must not land where `/acme` lands. `%` is escaped first, which
    /// is what makes the mapping reversible.
    #[test]
    fn an_escaped_prefix_cannot_be_forged_by_a_literal() {
        let ks = S3Keyspace::new(None);
        assert_ne!(
            ks.with_tenant_prefix("/acme").resources_prefix(),
            ks.with_tenant_prefix("%2Facme").resources_prefix(),
        );
        assert_ne!(
            ks.with_tenant_prefix("acme/resources").resources_prefix(),
            ks.with_tenant_prefix("acme%2Fresources").resources_prefix(),
        );
    }

    /// The empty id must not collapse into the base keyspace, where the tenant
    /// registry and the user-settings store live.
    #[test]
    fn the_empty_tenant_id_does_not_become_the_base_keyspace() {
        for base in [None, Some("hfs".to_string())] {
            let ks = S3Keyspace::new(base);
            assert_ne!(
                ks.with_tenant_prefix("").resources_prefix(),
                ks.resources_prefix(),
            );
            assert_ne!(
                ks.with_tenant_prefix("").tenant_registry_prefix(),
                ks.tenant_registry_prefix(),
            );
        }
    }

    /// The headline scenario from issue #447, stated as the operator sees it:
    /// purging `/acme` must not touch `acme`'s resources or its version history.
    #[test]
    fn purging_a_slash_padded_id_no_longer_reaches_the_bare_id() {
        let ks = S3Keyspace::new(None);
        let padded = ks.with_tenant_prefix("/acme");
        let bare = ks.with_tenant_prefix("acme");

        for prefix in sweep_prefixes(&padded) {
            for key in sample_keys(&bare) {
                assert!(
                    !key.starts_with(&prefix),
                    "purging `/acme` ({prefix}) would delete `acme`'s {key}"
                );
            }
        }
    }

    /// `is_keyspace_safe` is the whole rule; assert it directly so a change to
    /// the predicate has to confront each clause.
    #[test]
    fn keyspace_safety_predicate_clauses() {
        // Ordinary and hierarchical ids are safe.
        assert!(is_keyspace_safe("acme"));
        assert!(is_keyspace_safe("acme/research"));
        assert!(is_keyspace_safe("__system__"));
        // Empty segments — leading, trailing, repeated.
        assert!(!is_keyspace_safe("/acme"));
        assert!(!is_keyspace_safe("acme/"));
        assert!(!is_keyspace_safe("//acme"));
        assert!(!is_keyspace_safe("acme//research"));
        assert!(!is_keyspace_safe(""));
        // Characters the escape uses.
        assert!(!is_keyspace_safe("acme%25"));
        assert!(!is_keyspace_safe("acme\\research"));
        assert!(!is_keyspace_safe("acme research"));
        // Sweep roots after the first segment.
        for ns in SWEEP_ROOT_SEGMENTS {
            assert!(!is_keyspace_safe(&format!("acme/{ns}")));
            assert!(!is_keyspace_safe(&format!("acme/research/{ns}")));
            // ...but harmless as the first segment: nothing sweeps a bare
            // `resources/`, so a top-level tenant so named must not move.
            assert!(is_keyspace_safe(ns));
            assert!(is_keyspace_safe(&format!("{ns}/acme")));
        }
    }

    /// The `acme/resources` overlap (issue #385) is now closed at *both* layers,
    /// and neither layer is redundant.
    ///
    /// Upstream, `TenantId::parse`'s per-segment reserved list means such an id
    /// can no longer be minted. Downstream, this file's derivation escapes it, so
    /// the keyspace does not overlap even for an id already sitting in a registry
    /// — which matters, because `purge_tenant_data` builds its `TenantId` from an
    /// operator-supplied path segment with no validator in the way (issue #447).
    ///
    /// Asserting both halves together is what keeps either from being quietly
    /// dropped on the grounds that "the other layer covers it".
    #[test]
    fn hierarchical_id_naming_a_keyspace_namespace_cannot_overlap_its_parent() {
        use crate::tenant::TenantId;

        let base = S3Keyspace::new(None);
        let parent = base.with_tenant_prefix("acme");
        let nested = base.with_tenant_prefix("acme/resources");

        // The keyspace layer: nothing the nested tenant writes sits under a
        // prefix the parent sweeps or enumerates.
        for prefix in sweep_prefixes(&parent) {
            for key in sample_keys(&nested) {
                assert!(
                    !key.starts_with(&prefix),
                    "expected {key} to sit outside {prefix}"
                );
            }
        }

        // The validator layer: the id cannot be minted in the first place.
        assert!(
            TenantId::parse("acme/resources").is_err(),
            "the reserved segment is what keeps the id unmintable"
        );
        // The parent itself stays perfectly valid.
        assert!(TenantId::parse("acme").is_ok());
    }

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

        let v1 = hostile.with_layout(KeyspaceLayout::V1);
        let v2 = hostile.with_layout(KeyspaceLayout::V2);
        let data_keys = [
            v1.current_resource_key("Patient", "p1"),
            v1.history_version_key("Patient", "p1", "1"),
            v1.history_system_prefix().unwrap(),
            v2.current_resource_key("Patient", "p1"),
            hostile.layout_marker_key(),
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

    /// #1556: the submitter is `system|value`, so a URL system used to put
    /// `//` (and nested path segments) into every key of the submission.
    #[test]
    fn submit_keys_escape_client_identifiers_injectively() {
        let ks = S3Keyspace::new(None);
        let key = ks.submit_state_key("http://example.org|smoke", "smoke-24");
        assert_eq!(
            key,
            "bulk/submit/http:%2F%2Fexample.org|smoke/smoke-24/state.json"
        );
        assert!(!key.contains("//"), "{key}");
        assert_eq!(
            ks.submit_state_key("smoke", "s-1"),
            "bulk/submit/smoke/s-1/state.json"
        );

        // Two pairs that a raw join would collapse onto one prefix.
        assert_ne!(ks.submit_prefix("a/b", "c"), ks.submit_prefix("a", "b/c"));
        assert_ne!(ks.submit_prefix("a%2Fb", "c"), ks.submit_prefix("a/b", "c"));
        assert!(
            ks.submit_prefix("a/b", "c")
                .starts_with("bulk/submit/a%2Fb/c/")
        );

        for id in ["..", ".", "a\\b", "x\u{1}y"] {
            let key = ks.submit_state_key("s", id);
            let segment = key.split('/').nth(3).unwrap();
            assert!(
                !segment.contains('\\') && segment != "." && segment != "..",
                "{key}"
            );
        }
        assert_eq!(submit_segment(".."), "%2E%2E");
        assert_eq!(submit_segment("a.b"), "a.b");

        let manifest = ks.submit_manifest_key("http://example.org|smoke", "s-1", "m/1");
        assert!(manifest.ends_with("/manifests/m%2F1.json"), "{manifest}");
        assert!(!manifest.contains("//"), "{manifest}");
    }

    #[test]
    fn submit_file_keys_distinguish_manifest_and_nullable_resource() {
        let ks = S3Keyspace::new(None);
        let none = ks.submit_file_key("s", "job", "m-a", "output", None, 0, 1);
        let empty = ks.submit_file_key("s", "job", "m-a", "output", Some(""), 0, 1);
        let other_manifest = ks.submit_file_key("s", "job", "m-b", "output", None, 0, 1);

        assert_ne!(none, empty);
        assert_ne!(none, other_manifest);
        for key in [none, empty, other_manifest] {
            assert!(key.starts_with("bulk/submit/s/job/files/"));
        }
    }

    fn event_time() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_700_000_000_000).unwrap()
    }

    /// V1 keys are frozen: S3 is the system of record, so any change here makes
    /// existing objects unreachable. These literals must never be edited.
    #[test]
    fn v1_resource_and_history_keys_are_pinned() {
        let flat = S3Keyspace::new(Some("hfs".to_string())).with_tenant_prefix("acme");
        let ks = flat.with_layout(KeyspaceLayout::V1);
        assert_eq!(
            ks.current_resource_key("Patient", "p1"),
            "hfs/acme/resources/Patient/p1/current.json"
        );
        assert_eq!(
            ks.history_version_key("Patient", "p1", "2"),
            "hfs/acme/resources/Patient/p1/_history/2.json"
        );
        assert_eq!(
            ks.history_versions_prefix("Patient", "p1"),
            "hfs/acme/resources/Patient/p1/_history/"
        );
        assert_eq!(
            ks.resource_type_prefix("Patient"),
            "hfs/acme/resources/Patient/"
        );
        assert_eq!(
            ks.history_type_event_key("Patient", event_time(), "p 1", "2", "sfx"),
            "hfs/acme/history/type/Patient/1700000000000_p_1_2_sfx.json"
        );
        assert_eq!(
            ks.history_system_event_key("Patient", event_time(), "p 1", "2", "sfx")
                .as_deref(),
            Some("hfs/acme/history/system/1700000000000_Patient_p_1_2_sfx.json")
        );
        assert_eq!(
            ks.history_type_prefix("Patient"),
            "hfs/acme/history/type/Patient/"
        );
        assert_eq!(
            ks.history_system_prefix().as_deref(),
            Some("hfs/acme/history/system/")
        );
        assert_eq!(ks.resource_shard_prefixes("Patient").len(), 1);
        assert_eq!(
            ks.resource_shard_prefixes("Patient")[0],
            ks.resource_type_prefix("Patient")
        );
        assert_eq!(
            ks.history_type_shard_prefix_for("Patient", "p1"),
            ks.history_type_prefix("Patient")
        );
    }

    #[test]
    fn shard_segment_is_the_first_two_hex_chars_of_sha256() {
        // printf p1 | sha256sum -> f6...; abc -> ba...; ab -> fb...
        assert_eq!(shard_segment("p1"), "f6");
        assert_eq!(shard_segment("abc"), "ba");
        assert_eq!(shard_segment("ab"), "fb");
    }

    #[test]
    fn v2_resource_keys_insert_the_shard_between_type_and_id() {
        let ks = S3Keyspace::new(None)
            .with_tenant_prefix("acme")
            .with_layout(KeyspaceLayout::V2);
        assert_eq!(
            ks.current_resource_key("Patient", "p1"),
            "acme/resources/Patient/f6/p1/current.json"
        );
        assert_eq!(
            ks.history_version_key("Patient", "p1", "2"),
            "acme/resources/Patient/f6/p1/_history/2.json"
        );
        assert_eq!(
            ks.history_versions_prefix("Patient", "p1"),
            "acme/resources/Patient/f6/p1/_history/"
        );
        // The type prefix still covers every shard.
        assert!(
            ks.current_resource_key("Patient", "p1")
                .starts_with(&ks.resource_type_prefix("Patient"))
        );
    }

    #[test]
    fn v2_history_is_sharded_by_type_and_has_no_system_index() {
        let ks = S3Keyspace::new(None)
            .with_tenant_prefix("acme")
            .with_layout(KeyspaceLayout::V2);
        let key = ks.history_type_event_key("Patient", event_time(), "p1", "2", "sfx");
        assert_eq!(
            key,
            "acme/history/type/Patient/f6/1700000000000_p1_2_sfx.json"
        );
        assert!(key.starts_with(&ks.history_type_shard_prefix_for("Patient", "p1")));
        assert!(key.starts_with(&ks.history_type_prefix("Patient")));
        assert!(key.starts_with(&ks.history_types_root_prefix()));
        assert_eq!(ks.history_types_root_prefix(), "acme/history/type/");
        assert_eq!(
            ks.history_system_event_key("Patient", event_time(), "p1", "2", "sfx"),
            None
        );
        assert_eq!(ks.history_system_prefix(), None);
    }

    #[test]
    fn v2_shard_prefixes_are_256_ordered_and_partition_the_type() {
        let ks = S3Keyspace::new(None)
            .with_tenant_prefix("acme")
            .with_layout(KeyspaceLayout::V2);
        for prefixes in [
            ks.resource_shard_prefixes("Patient"),
            ks.history_type_shard_prefixes("Patient"),
        ] {
            assert_eq!(prefixes.len(), 256);
            assert!(prefixes.windows(2).all(|w| w[0] < w[1]));
            assert!(prefixes.iter().all(|p| p.ends_with('/')));
        }
        let first = &ks.resource_shard_prefixes("Patient")[0];
        assert_eq!(first, "acme/resources/Patient/00/");
        let last = ks.history_type_shard_prefixes("Patient").pop().unwrap();
        assert_eq!(last, "acme/history/type/Patient/ff/");
        // Every V2 object key falls under exactly one shard prefix.
        let key = ks.current_resource_key("Patient", "p1");
        let hits = ks
            .resource_shard_prefixes("Patient")
            .iter()
            .filter(|p| key.starts_with(p.as_str()))
            .count();
        assert_eq!(hits, 1);
    }

    #[test]
    fn layout_marker_sits_under_history_and_outside_resources() {
        let ks = S3Keyspace::new(Some("hfs".to_string())).with_tenant_prefix("acme");
        let key = ks.layout_marker_key();
        assert_eq!(key, "hfs/acme/history/_keyspace.json");
        assert!(key.starts_with(&ks.history_root_prefix()));
        assert!(!key.starts_with(&ks.resources_prefix()));
        let v1 = ks.with_layout(KeyspaceLayout::V1);
        assert!(!key.starts_with(&v1.history_system_prefix().unwrap()));
        assert!(!key.starts_with(&v1.history_type_prefix("Patient")));
        assert!(!key.starts_with(&v1.history_types_root_prefix()));
    }

    #[test]
    fn layout_marker_body_round_trips_and_refuses_unknown_versions() {
        for layout in [KeyspaceLayout::V1, KeyspaceLayout::V2] {
            assert_eq!(
                KeyspaceLayout::from_marker_body(&layout.marker_body()),
                Some(layout)
            );
        }
        assert_eq!(KeyspaceLayout::from_marker_body(b"{\"layout\":3}"), None);
        assert_eq!(KeyspaceLayout::from_marker_body(b"{}"), None);
        assert_eq!(KeyspaceLayout::from_marker_body(b"not json"), None);
        assert_eq!(KeyspaceLayout::parse(" V2 "), Some(KeyspaceLayout::V2));
        assert_eq!(KeyspaceLayout::parse("v3"), None);
    }

    #[test]
    fn the_marker_and_v2_keys_stay_inside_the_tenants_sweep_prefixes() {
        for id in TENANT_ID_CORPUS {
            let ks = S3Keyspace::new(None).with_tenant_prefix(id);
            let v2 = ks.with_layout(KeyspaceLayout::V2);
            let sweeps = sweep_prefixes(&ks);
            for key in [
                ks.layout_marker_key(),
                v2.current_resource_key("Patient", "p1"),
                v2.history_type_event_key("Patient", event_time(), "p1", "1", "s"),
            ] {
                assert!(sweeps.iter().any(|p| key.starts_with(p.as_str())), "{key}");
            }
        }
    }
}
