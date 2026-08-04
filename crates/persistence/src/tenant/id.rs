//! Tenant identifier type.
//!
//! This module defines the [`TenantId`] type, an opaque identifier for tenants
//! with support for hierarchical namespaces.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{StorageResult, TenantError};

/// The system tenant identifier, used for shared/global resources.
///
/// Resources stored under the system tenant are accessible to all tenants
/// (subject to permission checks). This is used for shared resources like
/// CodeSystems, ValueSets, and other terminology resources.
///
/// This id names an *internal* tenant: it is only ever legitimate when the
/// process itself authors it (via [`TenantId::system`] /
/// [`TenantContext::system`](crate::tenant::TenantContext::system)). It must
/// never be accepted from a request — see [`TenantId::parse`] and
/// [`ensure_mutable_tenant`], and issue #317.
pub const SYSTEM_TENANT: &str = "__system__";

/// Maximum accepted length of a caller-supplied tenant id.
///
/// Bounds the value that flows into storage keys, index terms and (for S3)
/// object key prefixes.
pub const MAX_TENANT_ID_LEN: usize = 128;

/// Tenant ids that are reserved for internal use and may never be supplied by
/// a caller.
///
/// [`SYSTEM_TENANT`] is the shared-tenant sentinel. The remainder name
/// control-plane namespaces in the S3 backend's keyspace (the sibling prefixes
/// of `tenants/`; see `S3Keyspace` and issue #271).
///
/// This is the single authority: the REST tenant extractor, the `/admin/tenants`
/// API, the web UI's tenant page, and the storage-layer tenant-lifecycle guard
/// all consult it, so a new reserved id cannot be added to one door and
/// forgotten on another.
pub const RESERVED_TENANT_IDS: &[&str] =
    &[SYSTEM_TENANT, "tenants", "resources", "history", "bulk"];

/// Why a caller-supplied tenant id was rejected by [`TenantId::parse`].
///
/// Deliberately carries no HTTP status: mapping a rejection onto a response
/// code depends on *which door* the id arrived at (a reserved id in a JWT claim
/// is a different statement from one in a URL), and that is the REST layer's
/// business. See `helios_rest::extractors::TenantExtractor`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TenantIdError {
    /// The id was empty.
    Empty,
    /// The id exceeded [`MAX_TENANT_ID_LEN`].
    TooLong {
        /// Actual length, in bytes.
        len: usize,
        /// The maximum permitted length.
        max: usize,
    },
    /// The id contained a character outside the permitted set.
    InvalidChar {
        /// The offending character.
        ch: char,
    },
    /// The id is reserved for internal use (see [`RESERVED_TENANT_IDS`]).
    Reserved {
        /// The rejected id.
        id: String,
    },
}

impl fmt::Display for TenantIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "tenant id must not be empty"),
            Self::TooLong { len, max } => {
                write!(f, "tenant id is {len} characters, maximum is {max}")
            }
            Self::InvalidChar { ch } => write!(
                f,
                "tenant id contains unsupported character {ch:?}; \
                 only letters, digits, '-', '_', '.', and '/' are allowed"
            ),
            Self::Reserved { id } => {
                write!(f, "tenant id {id:?} is reserved for internal use")
            }
        }
    }
}

impl std::error::Error for TenantIdError {}

/// Refuses a tenant-lifecycle mutation that targets a reserved tenant.
///
/// This is the storage-layer backstop for issue #317. The primary control is at
/// the REST ingress (`helios_rest::extractors::TenantExtractor`), but
/// `register_tenant` / `deregister_tenant` / `purge_tenant_data` take a bare
/// `&str` and are reachable from more than one handler — including, today, an
/// unauthenticated web-UI route. Every backend calls this at the top of those
/// three methods so no future call site can reach a destructive registry
/// operation on the shared tenant.
///
/// Read operations are deliberately *not* guarded: `get_tenant(SYSTEM_TENANT)`
/// truthfully returns `None` (the sentinel is never registered), and turning
/// that into an error would convert a benign lookup into a 500 while adding a
/// cleaner existence oracle rather than removing one.
pub fn ensure_mutable_tenant(id: &str) -> StorageResult<()> {
    if TenantId::is_reserved(id) {
        return Err(TenantError::InvalidTenant {
            tenant_id: TenantId::new(id),
        }
        .into());
    }
    Ok(())
}

/// An opaque tenant identifier with hierarchical namespace support.
///
/// `TenantId` supports hierarchical organization using a `/` separator,
/// enabling nested tenant structures like `org/department/team`.
///
/// # Hierarchy
///
/// Tenant IDs can form a hierarchy:
/// - `acme` - Top-level tenant
/// - `acme/research` - Child tenant under acme
/// - `acme/research/oncology` - Further nested child
///
/// A parent tenant may have visibility into child tenant data depending
/// on the configured tenancy strategy and permissions.
///
/// # Examples
///
/// ```
/// use helios_persistence::tenant::TenantId;
///
/// let tenant = TenantId::new("acme/research");
/// assert_eq!(tenant.as_str(), "acme/research");
/// assert!(tenant.is_descendant_of(&TenantId::new("acme")));
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(String);

impl TenantId {
    /// Creates a new tenant ID from the given string.
    ///
    /// # Arguments
    ///
    /// * `id` - The tenant identifier string. Can include `/` for hierarchy.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let tenant = TenantId::new("my-tenant");
    /// let nested = TenantId::new("parent/child");
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Parses a **caller-supplied** tenant id, rejecting ids that are empty,
    /// over-long, outside the permitted character set, or reserved for internal
    /// use.
    ///
    /// Use this — never [`TenantId::new`] — for any id that originates outside
    /// the process: an `X-Tenant-ID` header, a `/{tenant}/…` URL prefix, a JWT
    /// tenant claim, or an admin-API request body. [`TenantId::new`] stays
    /// infallible for ids the process itself authored (constants, validated
    /// configuration, values read back from storage), which is why
    /// `TenantContext::system()` and the ~120 terminology-server call sites are
    /// unaffected.
    ///
    /// The permitted character set is `[A-Za-z0-9._/-]`, matching what the
    /// `/admin/tenants` provisioning API has always accepted. Note that `/` is
    /// meaningful — it is the hierarchy separator (see
    /// [`TenantId::is_descendant_of`]) — and that URL-path tenant *routing*
    /// accepts a narrower set still (no `.` or `/`), so a hierarchical tenant is
    /// addressable by header or JWT claim but not as a URL prefix. That
    /// long-standing divergence is documented, not introduced, here.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// assert!(TenantId::parse("acme").is_ok());
    /// assert!(TenantId::parse("acme/research").is_ok());
    /// assert!(TenantId::parse("__system__").is_err());
    /// assert!(TenantId::parse("bad tenant").is_err());
    /// ```
    pub fn parse(id: &str) -> Result<Self, TenantIdError> {
        if id.is_empty() {
            return Err(TenantIdError::Empty);
        }
        if id.len() > MAX_TENANT_ID_LEN {
            return Err(TenantIdError::TooLong {
                len: id.len(),
                max: MAX_TENANT_ID_LEN,
            });
        }
        if let Some(ch) = id
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
        {
            return Err(TenantIdError::InvalidChar { ch });
        }
        if Self::is_reserved(id) {
            return Err(TenantIdError::Reserved { id: id.to_string() });
        }
        Ok(Self(id.to_string()))
    }

    /// Returns `true` if `id` is reserved for internal use and must never be
    /// accepted from a caller.
    ///
    /// Matching is exact and case-sensitive, mirroring how the ids are compared
    /// everywhere else (tenant ids are opaque and case-sensitive throughout the
    /// storage layer, so a case-insensitive check here would reject ids that are
    /// in fact distinct tenants).
    ///
    /// Only the exact reserved strings are refused, not a `__`-prefixed
    /// namespace. A tenant id is a partition key in every backend and there is
    /// no rename operation, so banning a prefix would strand any deployment that
    /// already holds such a tenant — its data would keep serving while becoming
    /// permanently impossible to deregister or purge. Reserving the wider space
    /// is a follow-up that has to start on the provisioning path, not here.
    pub fn is_reserved(id: &str) -> bool {
        RESERVED_TENANT_IDS.contains(&id)
    }

    /// Returns the system tenant ID.
    ///
    /// The system tenant is used for shared resources that should be
    /// accessible across all tenants.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let system = TenantId::system();
    /// assert!(system.is_system());
    /// ```
    pub fn system() -> Self {
        Self(SYSTEM_TENANT.to_string())
    }

    /// Returns the tenant ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` if this is the system tenant.
    pub fn is_system(&self) -> bool {
        self.0 == SYSTEM_TENANT
    }

    /// Returns `true` if this tenant is a descendant of the given ancestor.
    ///
    /// A tenant is a descendant if its ID starts with the ancestor's ID
    /// followed by a `/` separator.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let parent = TenantId::new("acme");
    /// let child = TenantId::new("acme/research");
    /// let grandchild = TenantId::new("acme/research/oncology");
    ///
    /// assert!(child.is_descendant_of(&parent));
    /// assert!(grandchild.is_descendant_of(&parent));
    /// assert!(grandchild.is_descendant_of(&child));
    /// assert!(!parent.is_descendant_of(&child));
    /// ```
    pub fn is_descendant_of(&self, ancestor: &TenantId) -> bool {
        if self.0 == ancestor.0 {
            return false; // A tenant is not a descendant of itself
        }
        self.0.starts_with(&ancestor.0) && self.0[ancestor.0.len()..].starts_with('/')
    }

    /// Returns `true` if this tenant is an ancestor of the given descendant.
    ///
    /// This is the inverse of [`is_descendant_of`](Self::is_descendant_of).
    pub fn is_ancestor_of(&self, descendant: &TenantId) -> bool {
        descendant.is_descendant_of(self)
    }

    /// Returns the parent tenant ID, if this is a nested tenant.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let child = TenantId::new("acme/research");
    /// assert_eq!(child.parent(), Some(TenantId::new("acme")));
    ///
    /// let root = TenantId::new("acme");
    /// assert_eq!(root.parent(), None);
    /// ```
    pub fn parent(&self) -> Option<TenantId> {
        self.0.rfind('/').map(|idx| TenantId::new(&self.0[..idx]))
    }

    /// Returns the depth of this tenant in the hierarchy.
    ///
    /// A root tenant has depth 0, its direct children have depth 1, etc.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// assert_eq!(TenantId::new("acme").depth(), 0);
    /// assert_eq!(TenantId::new("acme/research").depth(), 1);
    /// assert_eq!(TenantId::new("acme/research/oncology").depth(), 2);
    /// ```
    pub fn depth(&self) -> usize {
        self.0.matches('/').count()
    }

    /// Returns an iterator over all ancestor tenant IDs, from immediate parent to root.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let tenant = TenantId::new("acme/research/oncology");
    /// let ancestors: Vec<_> = tenant.ancestors().collect();
    /// assert_eq!(ancestors.len(), 2);
    /// assert_eq!(ancestors[0].as_str(), "acme/research");
    /// assert_eq!(ancestors[1].as_str(), "acme");
    /// ```
    pub fn ancestors(&self) -> impl Iterator<Item = TenantId> + '_ {
        TenantAncestorIterator {
            current: self.clone(),
        }
    }

    /// Returns the root tenant ID (the first segment of the hierarchy).
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let tenant = TenantId::new("acme/research/oncology");
    /// assert_eq!(tenant.root().as_str(), "acme");
    /// ```
    pub fn root(&self) -> TenantId {
        match self.0.find('/') {
            Some(idx) => TenantId::new(&self.0[..idx]),
            None => self.clone(),
        }
    }

    /// Creates a child tenant ID by appending a segment.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::TenantId;
    ///
    /// let parent = TenantId::new("acme");
    /// let child = parent.child("research");
    /// assert_eq!(child.as_str(), "acme/research");
    /// ```
    pub fn child(&self, segment: &str) -> TenantId {
        TenantId::new(format!("{}/{}", self.0, segment))
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TenantId({})", self.0)
    }
}

impl FromStr for TenantId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(TenantId::new(s))
    }
}

impl From<&str> for TenantId {
    fn from(s: &str) -> Self {
        TenantId::new(s)
    }
}

impl From<String> for TenantId {
    fn from(s: String) -> Self {
        TenantId::new(s)
    }
}

impl AsRef<str> for TenantId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Iterator over ancestor tenant IDs.
struct TenantAncestorIterator {
    current: TenantId,
}

impl Iterator for TenantAncestorIterator {
    type Item = TenantId;

    fn next(&mut self) -> Option<Self::Item> {
        let parent = self.current.parent()?;
        self.current = parent.clone();
        Some(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_id_creation() {
        let tenant = TenantId::new("my-tenant");
        assert_eq!(tenant.as_str(), "my-tenant");
    }

    #[test]
    fn test_system_tenant() {
        let system = TenantId::system();
        assert!(system.is_system());
        assert_eq!(system.as_str(), SYSTEM_TENANT);
    }

    #[test]
    fn test_hierarchy_descendant() {
        let parent = TenantId::new("acme");
        let child = TenantId::new("acme/research");
        let grandchild = TenantId::new("acme/research/oncology");
        let unrelated = TenantId::new("other");

        assert!(child.is_descendant_of(&parent));
        assert!(grandchild.is_descendant_of(&parent));
        assert!(grandchild.is_descendant_of(&child));
        assert!(!parent.is_descendant_of(&child));
        assert!(!child.is_descendant_of(&unrelated));
        assert!(!parent.is_descendant_of(&parent)); // Not descendant of self
    }

    #[test]
    fn test_hierarchy_ancestor() {
        let parent = TenantId::new("acme");
        let child = TenantId::new("acme/research");

        assert!(parent.is_ancestor_of(&child));
        assert!(!child.is_ancestor_of(&parent));
    }

    #[test]
    fn test_parent() {
        let root = TenantId::new("acme");
        let child = TenantId::new("acme/research");
        let grandchild = TenantId::new("acme/research/oncology");

        assert_eq!(root.parent(), None);
        assert_eq!(child.parent(), Some(TenantId::new("acme")));
        assert_eq!(grandchild.parent(), Some(TenantId::new("acme/research")));
    }

    #[test]
    fn test_depth() {
        assert_eq!(TenantId::new("acme").depth(), 0);
        assert_eq!(TenantId::new("acme/research").depth(), 1);
        assert_eq!(TenantId::new("acme/research/oncology").depth(), 2);
    }

    #[test]
    fn test_ancestors() {
        let tenant = TenantId::new("acme/research/oncology");
        let ancestors: Vec<_> = tenant.ancestors().collect();

        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0].as_str(), "acme/research");
        assert_eq!(ancestors[1].as_str(), "acme");
    }

    #[test]
    fn test_root() {
        assert_eq!(TenantId::new("acme").root().as_str(), "acme");
        assert_eq!(TenantId::new("acme/research").root().as_str(), "acme");
        assert_eq!(
            TenantId::new("acme/research/oncology").root().as_str(),
            "acme"
        );
    }

    #[test]
    fn test_child() {
        let parent = TenantId::new("acme");
        let child = parent.child("research");
        assert_eq!(child.as_str(), "acme/research");
    }

    #[test]
    fn test_serde_roundtrip() {
        let tenant = TenantId::new("acme/research");
        let json = serde_json::to_string(&tenant).unwrap();
        assert_eq!(json, "\"acme/research\"");

        let parsed: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tenant);
    }

    #[test]
    fn test_from_string() {
        let tenant: TenantId = "my-tenant".into();
        assert_eq!(tenant.as_str(), "my-tenant");

        let tenant2: TenantId = String::from("my-tenant").into();
        assert_eq!(tenant2.as_str(), "my-tenant");
    }

    // ── `parse` — the validating constructor for caller-supplied ids (#317) ──

    #[test]
    fn test_parse_accepts_ordinary_ids() {
        for id in [
            "acme",
            "tenant-123",
            "my_tenant",
            "ABC123",
            "a.b",
            "acme/research",
        ] {
            assert!(TenantId::parse(id).is_ok(), "{id} should parse");
        }
    }

    /// The whole point of #317: the shared-tenant sentinel must never survive
    /// parsing of a caller-supplied id.
    #[test]
    fn test_parse_rejects_system_tenant() {
        assert_eq!(
            TenantId::parse(SYSTEM_TENANT),
            Err(TenantIdError::Reserved {
                id: SYSTEM_TENANT.to_string()
            })
        );
    }

    #[test]
    fn test_parse_rejects_every_reserved_id() {
        for id in RESERVED_TENANT_IDS {
            assert!(
                TenantId::parse(id).is_err(),
                "reserved id {id} must not parse"
            );
            assert!(TenantId::is_reserved(id));
        }
    }

    /// Reservation is exact, not a `__` namespace ban — a deployment that
    /// already holds a `__`-prefixed tenant must keep working, because tenant id
    /// is a partition key with no rename. Guards against someone "hardening"
    /// this into a prefix check without a migration story.
    #[test]
    fn test_parse_reserves_exact_ids_not_the_underscore_namespace() {
        assert!(TenantId::parse("__system").is_ok());
        assert!(TenantId::parse("__system__x").is_ok());
        assert!(TenantId::parse("__legacy").is_ok());
        // Single-underscore tenants are a supported, separately-tested feature
        // of URL-path routing (`/_x/Patient/123`).
        assert!(TenantId::parse("_x").is_ok());
    }

    /// Tenant ids are opaque and case-sensitive in the storage layer, so a
    /// differently-cased id is a genuinely different tenant, not an evasion.
    #[test]
    fn test_parse_reservation_is_case_sensitive() {
        assert!(TenantId::parse("__SYSTEM__").is_ok());
        assert!(!TenantId::is_reserved("__SYSTEM__"));
    }

    #[test]
    fn test_parse_rejects_empty_overlong_and_bad_charset() {
        assert_eq!(TenantId::parse(""), Err(TenantIdError::Empty));

        let long = "a".repeat(MAX_TENANT_ID_LEN + 1);
        assert_eq!(
            TenantId::parse(&long),
            Err(TenantIdError::TooLong {
                len: MAX_TENANT_ID_LEN + 1,
                max: MAX_TENANT_ID_LEN,
            })
        );
        assert!(TenantId::parse(&"a".repeat(MAX_TENANT_ID_LEN)).is_ok());

        for (id, ch) in [("bad tenant", ' '), ("a:b", ':'), ("a\0b", '\0')] {
            assert_eq!(TenantId::parse(id), Err(TenantIdError::InvalidChar { ch }));
        }
    }

    /// `new` must stay infallible: `TenantContext::system()` and the ~120
    /// terminology-server call sites depend on constructing the sentinel
    /// in-process.
    #[test]
    fn test_new_still_builds_the_system_tenant() {
        assert_eq!(TenantId::new(SYSTEM_TENANT).as_str(), SYSTEM_TENANT);
        assert!(TenantId::system().is_system());
    }

    // ── `ensure_mutable_tenant` — the storage-layer backstop ──

    #[test]
    fn test_ensure_mutable_tenant_refuses_reserved_ids() {
        assert!(ensure_mutable_tenant("acme").is_ok());
        assert!(ensure_mutable_tenant("acme/research").is_ok());
        for id in RESERVED_TENANT_IDS {
            assert!(
                ensure_mutable_tenant(id).is_err(),
                "lifecycle mutation on reserved id {id} must be refused"
            );
        }
    }
}
