//! Tenant identifier type.
//!
//! This module defines the [`TenantId`] type, an opaque identifier for tenants
//! with support for hierarchical namespaces.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The system tenant identifier, used for shared/global resources.
///
/// Resources stored under the system tenant are accessible to all tenants
/// (subject to permission checks). This is used for shared resources like
/// CodeSystems, ValueSets, and other terminology resources.
pub const SYSTEM_TENANT: &str = "__system__";

/// Maximum length of a tenant id, in bytes.
///
/// 64 rather than something larger for two reasons. It is what the REST
/// tenant-routing surfaces have always enforced, so it is the widest bound that
/// every *routable* tenant already satisfies. And the Elasticsearch backend's
/// index-name derivation escapes unsafe bytes as three-byte sequences against a
/// 255-byte Elasticsearch limit; 64 × 3 = 192 leaves room for the index prefix
/// and resource type, whereas a 128-byte id could not fit.
pub const MAX_TENANT_ID_LEN: usize = 64;

/// Segments a tenant id may not contain, in any position.
///
/// These name control-plane namespaces inside a storage backend's keyspace.
/// The check is **per segment**, not on the whole id, because the hierarchy
/// separator is what makes them dangerous: on S3 a tenant's data lives under
/// `{tenant}/resources/…` and `{tenant}/history/…`, so tenant `a/resources`
/// stores at `a/resources/resources/…` — inside the prefix tenant `a` scans and
/// purges. `S3Backend::purge_tenant_data("a")` would delete it. Rejecting the
/// segment anywhere makes that shape unconstructible.
///
/// `__system__` is listed here too so `a/__system__` cannot be used to smuggle
/// the shared-tenant sentinel past a check that only compares the whole id.
///
/// Kept in sync with `backends::s3::keyspace::S3Keyspace`, which owns
/// `tenants/`, `resources/`, `history/`, `bulk/`, and `_system.user-settings/`.
/// Each of those is *also* safe structurally — a tenant so named writes to a
/// `resources/`/`history/` **sub**-prefix, which can never equal a control-plane
/// leaf — so this list is defence in depth, not the proof. Do not delete the
/// structural arguments in `S3Keyspace` on the strength of it.
pub const RESERVED_TENANT_SEGMENTS: &[&str] = &[
    SYSTEM_TENANT,
    "tenants",
    "resources",
    "history",
    "bulk",
    "_system.user-settings",
    // Relative-path segments. No backend resolves `..`, but a tenant id flows
    // into object keys and filesystem-shaped paths, where a normalising
    // intermediary would make `a/../b` and `b` the same location.
    ".",
    "..",
];

/// Why a string is not a valid tenant id.
///
/// Returned by [`TenantId::parse`], the canonical validating constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantIdError {
    /// The id was empty.
    Empty,
    /// The id exceeded [`MAX_TENANT_ID_LEN`] bytes.
    TooLong {
        /// The offending length, in bytes.
        len: usize,
    },
    /// The id contained a character outside the permitted set.
    InvalidCharacter {
        /// The first offending character.
        found: char,
    },
    /// The id had a leading `/`, a trailing `/`, or an empty segment (`a//b`).
    EmptySegment,
    /// A segment named a reserved control-plane namespace.
    ReservedSegment {
        /// The offending segment.
        segment: String,
    },
}

impl fmt::Display for TenantIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "tenant id must not be empty"),
            Self::TooLong { len } => write!(
                f,
                "tenant id is {len} bytes; the maximum is {MAX_TENANT_ID_LEN}"
            ),
            Self::InvalidCharacter { found } => write!(
                f,
                "tenant id contains {found:?}; only letters, digits, '-', '_', '.', \
                 and '/' (the hierarchy separator) are permitted"
            ),
            Self::EmptySegment => write!(
                f,
                "tenant id must not begin or end with '/' or contain an empty segment"
            ),
            Self::ReservedSegment { segment } => write!(
                f,
                "tenant id segment '{segment}' is reserved for internal use"
            ),
        }
    }
}

impl std::error::Error for TenantIdError {}

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
    /// Creates a tenant ID **without validating it**.
    ///
    /// This is the constructor for *trusted reconstruction*: rebuilding a
    /// `TenantId` from a value that was already validated when it entered the
    /// system — a `tenant_id` column read back out of a row, a value round-tripped
    /// through a `TenantContext`, a literal in a test. It stores the string
    /// verbatim and cannot fail.
    ///
    /// Use [`parse`](Self::parse) for anything derived from a request: an
    /// `X-Tenant-ID` header, a URL path prefix, a JWT claim, an admin-API body.
    /// Those are the paths that must not admit an id the storage backends cannot
    /// keep distinct.
    ///
    /// It stays infallible deliberately. Making it fallible would force an
    /// `unwrap()` at every trusted reconstruction site, which converts a
    /// data-integrity problem into a panic without catching anything new — the
    /// value is already in the database by then.
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

    /// Parses and validates a tenant ID — the canonical constructor for any id
    /// that arrives from outside the server.
    ///
    /// # The charset
    ///
    /// A valid tenant id is 1..=[`MAX_TENANT_ID_LEN`] bytes drawn from ASCII
    /// letters, ASCII digits, `-`, `_`, `.`, and `/`. `/` separates hierarchy
    /// levels (see [`parent`](Self::parent)), so it may not lead, trail, or
    /// repeat, and no segment may name a [reserved namespace](RESERVED_TENANT_SEGMENTS).
    ///
    /// Case is **preserved**, not folded: `ACME` and `acme` are two different
    /// tenants and every backend keeps them apart. Folding would be a silent
    /// data-visibility change for any deployment already using a mixed-case id.
    ///
    /// # Why this exists
    ///
    /// Before this, four surfaces validated tenant ids and all four disagreed —
    /// the header and URL extractors on one charset, the admin API on a wider
    /// one, the JWT claim on nothing at all — and `new` imposed no constraint
    /// beneath them. Backends were left to defend individually, which is how the
    /// Elasticsearch case-collision (issue #384) became reachable. One
    /// definition here lets a backend state a precondition instead of deriving
    /// its own (issue #385).
    ///
    /// What this does *not* cover, deliberately: whether an id shadows a FHIR
    /// resource type or a reserved route (`metadata`, `console`, …). That is a
    /// property of the REST routing surface, not of storage, and depends on the
    /// configured FHIR version — so it is layered on top in `helios-rest`, not
    /// duplicated here.
    ///
    /// # Examples
    ///
    /// ```
    /// use helios_persistence::tenant::{TenantId, TenantIdError};
    ///
    /// assert_eq!(TenantId::parse("acme").unwrap().as_str(), "acme");
    /// assert_eq!(TenantId::parse("acme/research").unwrap().depth(), 1);
    /// // Case is preserved, so these are distinct tenants.
    /// assert_ne!(TenantId::parse("ACME").unwrap(), TenantId::parse("acme").unwrap());
    ///
    /// assert!(matches!(TenantId::parse(""), Err(TenantIdError::Empty)));
    /// assert!(matches!(
    ///     TenantId::parse("acme corp"),
    ///     Err(TenantIdError::InvalidCharacter { found: ' ' })
    /// ));
    /// // Would otherwise land inside the prefix tenant `acme` scans and purges.
    /// assert!(matches!(
    ///     TenantId::parse("acme/resources"),
    ///     Err(TenantIdError::ReservedSegment { .. })
    /// ));
    /// ```
    pub fn parse(id: &str) -> Result<Self, TenantIdError> {
        if id.is_empty() {
            return Err(TenantIdError::Empty);
        }
        // Bytes, not chars: the length that matters is what reaches a storage
        // key. The charset check below rejects every non-ASCII byte anyway, so
        // the two measures coincide for accepted ids — this only makes the
        // rejection of an over-long non-ASCII id report a truthful length.
        if id.len() > MAX_TENANT_ID_LEN {
            return Err(TenantIdError::TooLong { len: id.len() });
        }
        if let Some(found) = id
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
        {
            return Err(TenantIdError::InvalidCharacter { found });
        }
        // `split('/')` yields an empty piece for a leading `/`, a trailing `/`,
        // and each `//`, so one emptiness check covers all three shapes.
        for segment in id.split('/') {
            if segment.is_empty() {
                return Err(TenantIdError::EmptySegment);
            }
            if RESERVED_TENANT_SEGMENTS.contains(&segment) {
                return Err(TenantIdError::ReservedSegment {
                    segment: segment.to_string(),
                });
            }
        }
        Ok(Self(id.to_string()))
    }

    /// Returns `true` if this id satisfies [`parse`](Self::parse)'s rules.
    ///
    /// For auditing ids that entered through [`new`](Self::new) — a value read
    /// back from storage that predates validation, say. Prefer `parse` when you
    /// are accepting an id, so the caller gets a reason rather than a bool.
    pub fn is_canonical(&self) -> bool {
        Self::parse(&self.0).is_ok()
    }

    /// Returns the system tenant ID.
    ///
    /// The system tenant is used for shared resources that should be
    /// accessible across all tenants.
    ///
    /// This is trusted internal construction, so it bypasses
    /// [`parse`](Self::parse) — which rejects `__system__` precisely so that no
    /// client-supplied id can ever name it. [`is_canonical`](Self::is_canonical)
    /// is therefore `false` for the system tenant, and that is correct: it means
    /// "not a value a client may assert", not "malformed".
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

/// Validating. `str::parse` is where a Rust reader expects a fallible,
/// checked conversion, so it routes through [`TenantId::parse`].
///
/// The `From` impls below are deliberately *not* validating — they mirror
/// [`TenantId::new`], the unchecked trusted-reconstruction constructor, because
/// an infallible `From` has nowhere to report a rejection. The asymmetry is the
/// point: `"acme".parse::<TenantId>()` checks, `TenantId::from("acme")` does not.
impl FromStr for TenantId {
    type Err = TenantIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TenantId::parse(s)
    }
}

/// Unchecked — see the note on [`FromStr`]'s impl above.
impl From<&str> for TenantId {
    fn from(s: &str) -> Self {
        TenantId::new(s)
    }
}

/// Unchecked — see the note on [`FromStr`]'s impl above.
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

    // ── The canonical validator (issue #385) ────────────────────────────────

    #[test]
    fn parse_accepts_every_shape_the_routing_surfaces_already_accepted() {
        // The union of what `resolver.rs`, `tenant_prefix.rs`, and the admin API
        // accepted before this validator existed. Anything here that started
        // failing would orphan a deployment's data, since no rename exists.
        for id in [
            "acme",
            "tenant-123",
            "my_tenant",
            "ABC123",
            "tenant.example",
            "acme/research",
            "acme/research/oncology",
            "a",
        ] {
            assert!(
                TenantId::parse(id).is_ok(),
                "{id:?} was accepted before this validator and must stay valid"
            );
        }
        // Exactly at the cap, which both routing validators already enforced.
        let at_cap = "a".repeat(MAX_TENANT_ID_LEN);
        assert!(TenantId::parse(&at_cap).is_ok());
    }

    #[test]
    fn parse_preserves_case_so_mixed_case_tenants_stay_distinct() {
        // Folding case would silently merge two tenants that every backend
        // currently keeps apart — a data-visibility change, not a fix. The
        // Elasticsearch collision (#384) is closed by making *its* derivation
        // injective, not by narrowing the charset here.
        let upper = TenantId::parse("ACME").expect("uppercase is valid");
        let lower = TenantId::parse("acme").expect("lowercase is valid");
        assert_eq!(upper.as_str(), "ACME");
        assert_ne!(upper, lower);
    }

    #[test]
    fn parse_rejects_empty_and_over_long() {
        assert_eq!(TenantId::parse(""), Err(TenantIdError::Empty));

        let long = "a".repeat(MAX_TENANT_ID_LEN + 1);
        assert_eq!(
            TenantId::parse(&long),
            Err(TenantIdError::TooLong {
                len: MAX_TENANT_ID_LEN + 1
            })
        );
    }

    #[test]
    fn parse_rejects_characters_outside_the_charset() {
        // Whitespace and the shapes that break a storage key or a URL. `%` and
        // `+` matter specifically: both are escape introducers in a backend
        // keyspace encoding, so letting one through as a literal would make the
        // escape forgeable.
        for (id, found) in [
            ("acme corp", ' '),
            ("acme\tcorp", '\t'),
            ("acme:corp", ':'),
            ("acme%2Fcorp", '%'),
            ("acme+corp", '+'),
            ("acme\\corp", '\\'),
            ("acme?x", '?'),
            ("acme#x", '#'),
            ("acme*", '*'),
            ("acmé", 'é'),
        ] {
            assert_eq!(
                TenantId::parse(id),
                Err(TenantIdError::InvalidCharacter { found }),
                "{id:?} must be rejected"
            );
        }
    }

    #[test]
    fn parse_rejects_malformed_hierarchy() {
        for id in ["/acme", "acme/", "acme//research", "/", "//"] {
            assert_eq!(
                TenantId::parse(id),
                Err(TenantIdError::EmptySegment),
                "{id:?} must be rejected"
            );
        }
    }

    /// Guards the S3 cross-tenant *destructive* collision that motivated the
    /// per-segment check.
    ///
    /// On S3 a tenant's data lives under `{tenant}/resources/…` and
    /// `{tenant}/history/…`. Tenant `acme/resources` therefore stores at
    /// `acme/resources/resources/…`, which is inside the `acme/resources/`
    /// prefix that tenant `acme` lists — and that
    /// `S3Backend::purge_tenant_data("acme")` deletes. The admin API's old
    /// whole-id reserved list did not catch this because the id is not equal to
    /// a reserved name, it merely contains one as a segment.
    #[test]
    fn parse_rejects_reserved_segments_in_any_position() {
        for id in [
            "resources",
            "acme/resources",
            "acme/history/x",
            "tenants",
            "acme/bulk",
            "__system__",
            "acme/__system__",
            "..",
            "acme/../evil",
            ".",
            "acme/./x",
        ] {
            assert!(
                matches!(
                    TenantId::parse(id),
                    Err(TenantIdError::ReservedSegment { .. })
                ),
                "{id:?} must be rejected as reserved, got {:?}",
                TenantId::parse(id)
            );
        }

        // A reserved word is only reserved as a whole segment — it must stay
        // usable as a substring, or we would reject ordinary names.
        assert!(TenantId::parse("resources-team").is_ok());
        assert!(TenantId::parse("acme/resources-archive").is_ok());
        assert!(TenantId::parse("prehistory").is_ok());
    }

    #[test]
    fn parse_is_injective_over_accepted_ids() {
        // `parse` stores the input verbatim — it normalises nothing. That is
        // what lets a backend treat the validated id as the identity it keys on.
        for id in [
            "acme",
            "ACME",
            "AcMe",
            "acme.corp",
            "acme_corp",
            "acme-corp",
        ] {
            assert_eq!(TenantId::parse(id).unwrap().as_str(), id);
        }
    }

    #[test]
    fn system_tenant_is_trusted_construction_not_a_parsable_id() {
        // `system()` bypasses `parse` by design; `parse` rejects the sentinel so
        // no client-supplied id can name it.
        let system = TenantId::system();
        assert!(system.is_system());
        assert!(
            !system.is_canonical(),
            "the sentinel is reachable only through trusted construction"
        );
        assert!(TenantId::parse(SYSTEM_TENANT).is_err());
    }

    #[test]
    fn from_str_validates_but_from_does_not() {
        // The deliberate asymmetry documented on the impls.
        assert!("acme corp".parse::<TenantId>().is_err());
        assert!("acme".parse::<TenantId>().is_ok());
        // `From` mirrors the unchecked `new`, so it still accepts anything.
        assert_eq!(TenantId::from("acme corp").as_str(), "acme corp");
    }

    #[test]
    fn is_canonical_flags_legacy_ids_reconstructed_through_new() {
        // `new` is unchecked, so a value that predates the validator round-trips
        // out of storage unchanged; `is_canonical` is how an operator finds it.
        assert!(TenantId::new("acme").is_canonical());
        assert!(!TenantId::new("acme corp").is_canonical());
        assert!(!TenantId::new("a".repeat(MAX_TENANT_ID_LEN + 1)).is_canonical());
    }

    #[test]
    fn error_messages_name_the_reason() {
        // These strings reach the client in a 400/403 body, so they have to be
        // actionable rather than a bare "invalid".
        assert!(
            TenantId::parse("acme corp")
                .unwrap_err()
                .to_string()
                .contains('\'')
        );
        assert!(
            TenantId::parse(&"a".repeat(100))
                .unwrap_err()
                .to_string()
                .contains("100")
        );
        assert!(
            TenantId::parse("acme/resources")
                .unwrap_err()
                .to_string()
                .contains("resources")
        );
    }
}
