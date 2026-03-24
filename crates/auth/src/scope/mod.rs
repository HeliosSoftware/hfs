pub mod permissions;
pub mod smart_v2;

pub use permissions::SmartPermissions;
pub use smart_v2::{ResourceTypeSpec, ScopeContext, SmartScope};

/// A set of parsed SMART v2 scopes from a JWT token.
#[derive(Debug, Clone, Default)]
pub struct ScopeSet {
    scopes: Vec<SmartScope>,
}

impl ScopeSet {
    /// Create an empty scope set.
    pub fn empty() -> Self {
        Self { scopes: Vec::new() }
    }

    /// Parse a space-delimited scope string (from JWT `scope` claim).
    ///
    /// Non-SMART scopes (e.g., `openid`, `profile`) are silently ignored.
    pub fn parse(scope_str: &str) -> Self {
        let scopes = scope_str
            .split_whitespace()
            .filter_map(SmartScope::parse)
            .collect();
        Self { scopes }
    }

    /// Parse from an array of scope strings (from JWT `scp` claim, e.g., Okta).
    pub fn parse_array(scope_strs: &[String]) -> Self {
        let scopes = scope_strs
            .iter()
            .filter_map(|s| SmartScope::parse(s))
            .collect();
        Self { scopes }
    }

    /// Check if any scope grants the given permission on the given resource type.
    pub fn is_permitted(&self, resource_type: &str, permission: SmartPermissions) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope.permits(resource_type, permission))
    }

    /// Returns the parsed scopes.
    pub fn scopes(&self) -> &[SmartScope] {
        &self.scopes
    }

    /// Returns true if no SMART scopes were parsed.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
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
}
