use crate::error::{AuthError, FhirOperation};
use crate::principal::Principal;
use crate::scope::SmartPermissions;

/// How far an authorized request may reach.
///
/// Returned by [`SmartScopePolicy::check`]. A `user/` or `system/` scope
/// grants the operation on every resource of the type the tenant holds; a
/// `patient/` scope grants it only inside the launch patient's compartment
/// (SMART App Launch, "Patient-specific scopes"). The REST layer attaches the
/// grant to the request so handlers can confine searches, reads and writes
/// accordingly (#1618).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessScope {
    /// Granted by a `user/` or `system/` scope: not confined to a compartment.
    Unrestricted,
    /// Granted only by a `patient/` scope: confined to the Patient compartment
    /// of the launch patient with this logical id.
    PatientCompartment {
        /// The launch patient's logical id, from the token's `patient` claim.
        patient_id: String,
    },
}

impl AccessScope {
    /// The launch patient the grant is confined to, if it is.
    pub fn patient_id(&self) -> Option<&str> {
        match self {
            AccessScope::Unrestricted => None,
            AccessScope::PatientCompartment { patient_id } => Some(patient_id),
        }
    }

    /// True for a compartment-confined grant.
    pub fn is_patient_compartment(&self) -> bool {
        matches!(self, AccessScope::PatientCompartment { .. })
    }
}

/// Checks whether a Principal's SMART scopes authorize a FHIR operation.
pub struct SmartScopePolicy;

impl SmartScopePolicy {
    /// Check if the principal is authorized to perform the given operation
    /// on the given resource type.
    ///
    /// Returns the [`AccessScope`] of the grant, or `Err(AuthError::Forbidden)`
    /// when no scope grants it. The contexts are tried widest first: a `user/`
    /// or `system/` scope yields [`AccessScope::Unrestricted`] even when a
    /// `patient/` scope would also match. A grant that rests on a `patient/`
    /// scope alone needs a usable launch patient
    /// ([`LaunchContext::patient_id`](crate::LaunchContext::patient_id)); when
    /// the token has none, the scope is refused with
    /// [`AuthError::PatientContextRequired`] rather than widened to `user/`.
    pub fn check(
        principal: &Principal,
        resource_type: &str,
        operation: FhirOperation,
    ) -> Result<AccessScope, AuthError> {
        let permission = Self::operation_to_permission(operation);

        if principal
            .scopes
            .is_permitted_unrestricted(resource_type, permission)
        {
            return Ok(AccessScope::Unrestricted);
        }

        if principal
            .scopes
            .is_permitted_by_patient_scope(resource_type, permission)
        {
            return match principal
                .launch_context
                .as_ref()
                .and_then(|ctx| ctx.patient_id())
            {
                Some(patient_id) => Ok(AccessScope::PatientCompartment {
                    patient_id: patient_id.to_string(),
                }),
                None => Err(AuthError::PatientContextRequired {
                    resource_type: resource_type.to_string(),
                    operation: operation.to_string(),
                }),
            };
        }

        Err(AuthError::Forbidden {
            resource_type: resource_type.to_string(),
            operation: operation.to_string(),
        })
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
            ..Default::default()
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

    fn patient_principal(scope_str: &str, patient: Option<&str>) -> Principal {
        Principal {
            launch_context: patient.map(|p| crate::LaunchContext {
                patient: Some(p.to_string()),
                ..Default::default()
            }),
            ..make_principal(scope_str)
        }
    }

    #[test]
    fn test_user_and_system_grants_are_unrestricted() {
        for scope in ["user/Patient.rs", "system/Patient.rs", "system/*.cruds"] {
            let principal = make_principal(scope);
            assert_eq!(
                SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).unwrap(),
                AccessScope::Unrestricted,
                "{scope}"
            );
        }
    }

    #[test]
    fn test_patient_scope_with_launch_patient_is_compartment_confined() {
        let principal = patient_principal("patient/*.rs", Some("p1"));
        assert_eq!(
            SmartScopePolicy::check(&principal, "Observation", FhirOperation::Search).unwrap(),
            AccessScope::PatientCompartment {
                patient_id: "p1".to_string()
            }
        );
        // The `Patient/<id>` claim form is normalised to the bare id.
        let principal = patient_principal("patient/*.rs", Some("Patient/p1"));
        assert_eq!(
            SmartScopePolicy::check(&principal, "Observation", FhirOperation::Read)
                .unwrap()
                .patient_id(),
            Some("p1")
        );
    }

    #[test]
    fn test_patient_scope_without_launch_patient_is_refused_not_widened() {
        // No launch context at all.
        let principal = patient_principal("patient/*.rs", None);
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read),
            Err(AuthError::PatientContextRequired { .. })
        ));
        // An unusable claim (not a Patient id) is the same as none.
        let principal = patient_principal("patient/*.rs", Some("Group/g1"));
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read),
            Err(AuthError::PatientContextRequired { .. })
        ));
        // Encounter-only launch context does not name a patient either.
        let principal = Principal {
            launch_context: Some(crate::LaunchContext {
                encounter: Some("e1".to_string()),
                ..Default::default()
            }),
            ..make_principal("patient/*.rs")
        };
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read),
            Err(AuthError::PatientContextRequired { .. })
        ));
    }

    #[test]
    fn test_patient_scope_lacking_the_permission_is_plain_forbidden() {
        let principal = patient_principal("patient/Patient.r", Some("p1"));
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Create),
            Err(AuthError::Forbidden { .. })
        ));
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Observation", FhirOperation::Read),
            Err(AuthError::Forbidden { .. })
        ));
    }

    #[test]
    fn test_wider_context_wins_over_patient_scope() {
        // Observation is granted by user/, so it is unrestricted even though
        // patient/*.rs would also match; Patient is granted by patient/ only.
        let principal = patient_principal("user/Observation.rs patient/*.rs", Some("p1"));
        assert_eq!(
            SmartScopePolicy::check(&principal, "Observation", FhirOperation::Read).unwrap(),
            AccessScope::Unrestricted
        );
        assert_eq!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).unwrap(),
            AccessScope::PatientCompartment {
                patient_id: "p1".to_string()
            }
        );
        // Without a launch patient the user/ grant still works and the
        // patient/-only type is refused.
        let principal = patient_principal("user/Observation.rs patient/*.rs", None);
        assert!(SmartScopePolicy::check(&principal, "Observation", FhirOperation::Read).is_ok());
        assert!(matches!(
            SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read),
            Err(AuthError::PatientContextRequired { .. })
        ));
    }

    #[test]
    fn test_empty_scopes_deny_all() {
        let principal = make_principal("");
        assert!(SmartScopePolicy::check(&principal, "Patient", FhirOperation::Read).is_err());
    }
}
