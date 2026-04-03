use crate::error::{AuthError, FhirOperation};
use crate::principal::Principal;
use crate::scope::SmartPermissions;

/// Checks whether a Principal's SMART scopes authorize a FHIR operation.
pub struct SmartScopePolicy;

impl SmartScopePolicy {
    /// Check if the principal is authorized to perform the given operation
    /// on the given resource type.
    ///
    /// Returns `Ok(())` if authorized, or `Err(AuthError::Forbidden)` if not.
    pub fn check(
        principal: &Principal,
        resource_type: &str,
        operation: FhirOperation,
    ) -> Result<(), AuthError> {
        let permission = Self::operation_to_permission(operation);

        if principal.scopes.is_permitted(resource_type, permission) {
            Ok(())
        } else {
            Err(AuthError::Forbidden {
                resource_type: resource_type.to_string(),
                operation: operation.to_string(),
            })
        }
    }

    /// Map a FHIR operation to the corresponding SMART permission bit.
    fn operation_to_permission(operation: FhirOperation) -> SmartPermissions {
        match operation {
            FhirOperation::Read => SmartPermissions::READ,
            FhirOperation::Search => SmartPermissions::SEARCH,
            FhirOperation::Create => SmartPermissions::CREATE,
            FhirOperation::Update => SmartPermissions::UPDATE,
            FhirOperation::Delete => SmartPermissions::DELETE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeSet;
    use chrono::Utc;

    fn make_principal(scope_str: &str) -> Principal {
        Principal {
            subject: "test-client".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            scopes: ScopeSet::parse(scope_str),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    #[test]
    fn test_read_permitted() {
        let principal = make_principal("system/Patient.rs");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).is_ok());
    }

    #[test]
    fn test_search_permitted() {
        let principal = make_principal("system/Patient.rs");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Search).is_ok());
    }

    #[test]
    fn test_create_denied() {
        let principal = make_principal("system/Patient.rs");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Create).is_err());
    }

    #[test]
    fn test_wrong_resource_type() {
        let principal = make_principal("system/Patient.rs");
        assert!(SmartScopePolicy::check(&principal, "Observation", FhirOperation::Read).is_err());
    }

    #[test]
    fn test_wildcard_full_access() {
        let principal = make_principal("system/*.cruds");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Create).is_ok());
        assert!(SmartScopePolicy::check(&principal, "Observation", FhirOperation::Delete).is_ok());
        assert!(SmartScopePolicy::check(&principal, "Condition", FhirOperation::Search).is_ok());
    }

    #[test]
    fn test_multiple_scopes() {
        let principal = make_principal("system/Patient.rs system/Observation.crud");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).is_ok());
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Create).is_err());
        assert!(SmartScopePolicy::check(&principal, "Observation", FhirOperation::Create).is_ok());
        assert!(SmartScopePolicy::check(&principal, "Observation", FhirOperation::Search).is_err());
    }

    #[test]
    fn test_empty_scopes_deny_all() {
        let principal = make_principal("");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).is_err());
    }
}
