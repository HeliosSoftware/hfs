use super::permissions::SmartPermissions;
use std::fmt;

/// The access context of a SMART scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScopeContext {
    /// System-level access (backend services, no user context).
    System,
    /// User-level access (scoped to the authenticated user).
    User,
    /// Patient-level access (scoped to a specific patient).
    Patient,
}

impl fmt::Display for ScopeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeContext::System => write!(f, "system"),
            ScopeContext::User => write!(f, "user"),
            ScopeContext::Patient => write!(f, "patient"),
        }
    }
}

/// The resource type specifier in a SMART scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceTypeSpec {
    /// A specific FHIR resource type (e.g., "Patient").
    Specific(String),
    /// Wildcard — applies to all resource types.
    Wildcard,
}

impl fmt::Display for ResourceTypeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceTypeSpec::Specific(t) => write!(f, "{}", t),
            ResourceTypeSpec::Wildcard => write!(f, "*"),
        }
    }
}

/// A single parsed SMART v2 scope.
///
/// Follows the format: `context/resourceType.permissions`
///
/// Examples:
/// - `system/Patient.rs` — system-level read+search on Patient
/// - `system/*.cruds` — system-level full CRUD+search on all types
/// - `user/Observation.r` — user-level read on Observation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmartScope {
    /// The access context.
    pub context: ScopeContext,
    /// The resource type or wildcard.
    pub resource_type: ResourceTypeSpec,
    /// The granted permissions.
    pub permissions: SmartPermissions,
}

impl SmartScope {
    /// Parse a single SMART v2 scope string.
    ///
    /// Expected format: `context/resourceType.permissions`
    /// Returns `None` if the string is malformed.
    pub fn parse(scope_str: &str) -> Option<Self> {
        // Split into context and rest: "system/Patient.rs"
        let (context_str, rest) = scope_str.split_once('/')?;

        let context = match context_str {
            "system" => ScopeContext::System,
            "user" => ScopeContext::User,
            "patient" => ScopeContext::Patient,
            _ => return None,
        };

        // Split rest into resource type and permissions: "Patient.rs"
        let (resource_str, perm_str) = rest.split_once('.')?;

        if resource_str.is_empty() || perm_str.is_empty() {
            return None;
        }

        let resource_type = if resource_str == "*" {
            ResourceTypeSpec::Wildcard
        } else {
            ResourceTypeSpec::Specific(resource_str.to_string())
        };

        let permissions = SmartPermissions::from_permission_str(perm_str)?;

        Some(SmartScope {
            context,
            resource_type,
            permissions,
        })
    }

    /// Check if this scope grants the given permission on the given resource type.
    pub fn permits(&self, resource_type: &str, permission: SmartPermissions) -> bool {
        let type_matches = match &self.resource_type {
            ResourceTypeSpec::Wildcard => true,
            ResourceTypeSpec::Specific(t) => t == resource_type,
        };

        type_matches && self.permissions.contains(permission)
    }
}

impl fmt::Display for SmartScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}.{}",
            self.context, self.resource_type, self.permissions
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_system_patient_rs() {
        let scope = SmartScope::parse("system/Patient.rs").unwrap();
        assert_eq!(scope.context, ScopeContext::System);
        assert_eq!(
            scope.resource_type,
            ResourceTypeSpec::Specific("Patient".to_string())
        );
        assert_eq!(
            scope.permissions,
            SmartPermissions::READ | SmartPermissions::SEARCH
        );
    }

    #[test]
    fn test_parse_wildcard_cruds() {
        let scope = SmartScope::parse("system/*.cruds").unwrap();
        assert_eq!(scope.context, ScopeContext::System);
        assert_eq!(scope.resource_type, ResourceTypeSpec::Wildcard);
        assert!(scope.permissions.contains(SmartPermissions::CREATE));
        assert!(scope.permissions.contains(SmartPermissions::READ));
        assert!(scope.permissions.contains(SmartPermissions::UPDATE));
        assert!(scope.permissions.contains(SmartPermissions::DELETE));
        assert!(scope.permissions.contains(SmartPermissions::SEARCH));
    }

    #[test]
    fn test_parse_user_context() {
        let scope = SmartScope::parse("user/Observation.r").unwrap();
        assert_eq!(scope.context, ScopeContext::User);
        assert_eq!(
            scope.resource_type,
            ResourceTypeSpec::Specific("Observation".to_string())
        );
        assert_eq!(scope.permissions, SmartPermissions::READ);
    }

    #[test]
    fn test_parse_patient_context() {
        let scope = SmartScope::parse("patient/MedicationRequest.crud").unwrap();
        assert_eq!(scope.context, ScopeContext::Patient);
    }

    #[test]
    fn test_parse_invalid_scopes() {
        assert!(SmartScope::parse("").is_none());
        assert!(SmartScope::parse("system").is_none());
        assert!(SmartScope::parse("system/").is_none());
        assert!(SmartScope::parse("system/Patient").is_none());
        assert!(SmartScope::parse("system/.rs").is_none());
        assert!(SmartScope::parse("system/Patient.").is_none());
        assert!(SmartScope::parse("system/Patient.xyz").is_none());
        assert!(SmartScope::parse("unknown/Patient.rs").is_none());
        assert!(SmartScope::parse("foo/bar/baz").is_none());
    }

    #[test]
    fn test_permits() {
        let scope = SmartScope::parse("system/Patient.rs").unwrap();
        assert!(scope.permits("Patient", SmartPermissions::READ));
        assert!(scope.permits("Patient", SmartPermissions::SEARCH));
        assert!(!scope.permits("Patient", SmartPermissions::CREATE));
        assert!(!scope.permits("Observation", SmartPermissions::READ));
    }

    #[test]
    fn test_permits_wildcard() {
        let scope = SmartScope::parse("system/*.rs").unwrap();
        assert!(scope.permits("Patient", SmartPermissions::READ));
        assert!(scope.permits("Observation", SmartPermissions::SEARCH));
        assert!(!scope.permits("Patient", SmartPermissions::DELETE));
    }

    #[test]
    fn test_display_roundtrip() {
        let original = "system/Patient.rs";
        let scope = SmartScope::parse(original).unwrap();
        assert_eq!(format!("{}", scope), original);
    }
}
