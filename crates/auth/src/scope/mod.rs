pub mod permissions;
pub mod smart_v2;

pub use permissions::SmartPermissions;
pub use smart_v2::{ResourceTypeSpec, ScopeContext, SmartScope};

/// A set of parsed SMART v2 scopes from a JWT token.
///
/// Retains the original raw scope tokens alongside the parsed resource scopes so
/// that *operation* scopes (e.g. `system/bulk-submit`), which do not match the
/// `context/resourceType.permissions` grammar and are therefore dropped by
/// [`SmartScope::parse`], can still be inspected — see [`ScopeSet::grants_operation`].
#[derive(Debug, Clone, Default)]
pub struct ScopeSet {
    scopes: Vec<SmartScope>,
    raw: Vec<String>,
}

impl ScopeSet {
    /// Create an empty scope set.
    pub fn empty() -> Self {
        Self {
            scopes: Vec::new(),
            raw: Vec::new(),
        }
    }

    /// Parse a space-delimited scope string (from JWT `scope` claim).
    ///
    /// Non-SMART scopes (e.g., `openid`, `profile`) are silently ignored for
    /// resource-permission checks but retained in [`ScopeSet::raw`].
    pub fn parse(scope_str: &str) -> Self {
        let raw: Vec<String> = scope_str.split_whitespace().map(str::to_string).collect();
        let scopes = raw.iter().filter_map(|s| SmartScope::parse(s)).collect();
        Self { scopes, raw }
    }

    /// Parse from an array of scope strings (from JWT `scp` claim, e.g., Okta).
    pub fn parse_array(scope_strs: &[String]) -> Self {
        let raw: Vec<String> = scope_strs.to_vec();
        let scopes = raw.iter().filter_map(|s| SmartScope::parse(s)).collect();
        Self { scopes, raw }
    }

    /// Check if any scope grants the given permission on the given resource type.
    pub fn is_permitted(&self, resource_type: &str, permission: SmartPermissions) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope.permits(resource_type, permission))
    }

    /// Check whether this set holds a **system-context, all-resource** scope
    /// (`system/*.<perm>`) granting the given permission.
    ///
    /// Unlike [`is_permitted`](Self::is_permitted), this requires the scope's
    /// context to be [`ScopeContext::System`] **and** its resource specifier to
    /// be [`ResourceTypeSpec::Wildcard`] — a `user/*` or `patient/*` token, or a
    /// system token scoped to a single resource type (`system/Patient.rs`), never
    /// satisfies it.
    ///
    /// This is the gate for backend-service / administrative endpoints that span
    /// every tenant (e.g. the cross-tenant console metrics), which no per-user or
    /// per-patient app — however broad its wildcard — should be able to reach.
    pub fn has_system_scope(&self, permission: SmartPermissions) -> bool {
        self.scopes.iter().any(|scope| {
            scope.context == ScopeContext::System
                && matches!(scope.resource_type, ResourceTypeSpec::Wildcard)
                && scope.permissions.contains(permission)
        })
    }

    /// Returns the parsed scopes.
    pub fn scopes(&self) -> &[SmartScope] {
        &self.scopes
    }

    /// Returns the raw, unparsed scope tokens exactly as presented in the token.
    pub fn raw(&self) -> &[String] {
        &self.raw
    }

    /// Returns true if no SMART scopes were parsed.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// Returns true if any parsed scope is a system-level wildcard (`system/*.<perms>`).
    ///
    /// Used as the ownership-bypass / broad-grant signal for bulk operations.
    pub fn has_system_wildcard(&self) -> bool {
        self.scopes.iter().any(|s| {
            s.context == ScopeContext::System
                && matches!(s.resource_type, ResourceTypeSpec::Wildcard)
        })
    }

    /// Returns true if the token grants the named system-level *operation* scope.
    ///
    /// Matches the literal raw scope `system/{name}` (e.g. `system/bulk-submit`)
    /// or any system-level wildcard scope (`system/*.<perms>`), which is treated
    /// as granting all operations.
    pub fn grants_operation(&self, name: &str) -> bool {
        let target = format!("system/{name}");
        self.raw.iter().any(|s| s == &target) || self.has_system_wildcard()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_space_delimited() {
        let set = ScopeSet::parse("system/Patient.rs system/Observation.r openid profile");
        assert_eq!(set.scopes().len(), 2);
        assert!(set.is_permitted("Patient", SmartPermissions::READ));
        assert!(set.is_permitted("Patient", SmartPermissions::SEARCH));
        assert!(set.is_permitted("Observation", SmartPermissions::READ));
        assert!(!set.is_permitted("Observation", SmartPermissions::SEARCH));
    }

    #[test]
    fn test_parse_array() {
        let scopes = vec![
            "system/Patient.rs".to_string(),
            "system/*.crud".to_string(),
            "openid".to_string(),
        ];
        let set = ScopeSet::parse_array(&scopes);
        assert_eq!(set.scopes().len(), 2);
        assert!(set.is_permitted("Patient", SmartPermissions::READ));
        // Wildcard scope grants CRUD on everything
        assert!(set.is_permitted("Condition", SmartPermissions::CREATE));
    }

    #[test]
    fn test_empty_scope() {
        let set = ScopeSet::parse("");
        assert!(set.is_empty());
        assert!(!set.is_permitted("Patient", SmartPermissions::READ));
    }

    #[test]
    fn test_wildcard_scope() {
        let set = ScopeSet::parse("system/*.cruds");
        assert!(set.is_permitted("Patient", SmartPermissions::CREATE));
        assert!(set.is_permitted("Observation", SmartPermissions::DELETE));
        assert!(set.is_permitted("Condition", SmartPermissions::SEARCH));
    }

    #[test]
    fn test_non_smart_scopes_ignored() {
        let set = ScopeSet::parse("openid profile email launch/patient");
        assert!(set.is_empty());
    }

    #[test]
    fn test_grants_operation_literal() {
        let set = ScopeSet::parse("openid system/bulk-submit profile");
        // Operation scope is dropped from parsed resource scopes but retained raw.
        assert!(set.is_empty());
        assert!(set.grants_operation("bulk-submit"));
        assert!(!set.grants_operation("export"));
        assert!(!set.has_system_wildcard());
    }

    #[test]
    fn test_grants_operation_via_wildcard() {
        let set = ScopeSet::parse("system/*.cruds");
        assert!(set.has_system_wildcard());
        // A system wildcard is treated as granting any operation.
        assert!(set.grants_operation("bulk-submit"));
    }

    #[test]
    fn test_grants_operation_array_claim() {
        let set = ScopeSet::parse_array(&[
            "system/bulk-submit".to_string(),
            "system/Patient.rs".to_string(),
        ]);
        assert!(set.grants_operation("bulk-submit"));
        assert!(set.is_permitted("Patient", SmartPermissions::READ));
        assert!(!set.has_system_wildcard());
    }

    #[test]
    fn test_raw_retained() {
        let set = ScopeSet::parse("system/Patient.rs system/bulk-submit");
        assert_eq!(set.raw().len(), 2);
    }

    #[test]
    fn test_has_system_scope_grants_for_system_wildcard() {
        assert!(ScopeSet::parse("system/*.r").has_system_scope(SmartPermissions::READ));
        assert!(ScopeSet::parse("system/*.rs").has_system_scope(SmartPermissions::READ));
        assert!(ScopeSet::parse("system/*.cruds").has_system_scope(SmartPermissions::READ));
        // The permission bit must actually be present.
        assert!(!ScopeSet::parse("system/*.cud").has_system_scope(SmartPermissions::READ));
    }

    #[test]
    fn test_has_system_scope_rejects_non_system_context() {
        // This is the exact class of token that leaks today: a broad, wildcard,
        // read-capable scope that is NOT system-context. It must be rejected.
        assert!(!ScopeSet::parse("patient/*.rs").has_system_scope(SmartPermissions::READ));
        assert!(!ScopeSet::parse("user/*.cruds").has_system_scope(SmartPermissions::READ));
    }

    #[test]
    fn test_has_system_scope_rejects_non_wildcard_system() {
        // System context but scoped to a single resource type is not system-wide.
        assert!(!ScopeSet::parse("system/Patient.rs").has_system_scope(SmartPermissions::READ));
    }

    #[test]
    fn test_has_system_scope_finds_grant_among_many() {
        let set = ScopeSet::parse("patient/Patient.rs user/Observation.r system/*.r");
        assert!(set.has_system_scope(SmartPermissions::READ));
    }
}
