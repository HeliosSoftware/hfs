//! S3 key construction for all FHIR storage namespaces.
//!
//! Keys are structured as hierarchical paths that encode the tenant prefix,
//! resource type, resource ID, version, and operation type. [`S3Keyspace`]
//! derives every key shape used by the backend from a common base prefix.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

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
    ///
    /// `file_url` names the manifest output file the line came from; see
    /// [`submit_file_segment`] for why it is part of the key.
    pub fn submit_raw_line_key(
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
            submitter,
            submission_id,
            "raw",
            manifest_id,
            &submit_file_segment(file_url),
            &format!("line-{}.ndjson", line),
        ])
    }

    /// Key for the processing result of a single NDJSON line.
    ///
    /// `file_url` names the manifest output file the line came from; see
    /// [`submit_file_segment`] for why it is part of the key.
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
            submitter,
            submission_id,
            "results",
            manifest_id,
            &submit_file_segment(file_url),
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
/// prefix that `load_entry_results` sweeps recursively.
fn submit_file_segment(file_url: Option<&str>) -> String {
    match file_url {
        Some(url) if !url.is_empty() => {
            let digest = Sha256::digest(url.as_bytes());
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        }
        _ => String::new(),
    }
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
        vec![
            ks.current_resource_key("Patient", "p1"),
            ks.history_version_key("Patient", "p1", "2"),
            ks.resource_type_prefix("Patient"),
            ks.history_system_prefix(),
            ks.submit_state_key("submitter", "sub-1"),
        ]
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
