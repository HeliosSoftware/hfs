//! Which resource types the caller's scopes let `$everything` return.
//!
//! The authorization middleware admits the operation on a `Patient` read
//! grant alone. SMART scopes are granted per resource type, so the walk
//! itself keeps only the compartment member types the token may search and
//! the supporting resource types it may read, as `Patient/$export` does.

use helios_auth::{FhirOperation, Principal, SmartScopePolicy};
use helios_fhir::FhirVersion;

use super::params::EverythingParams;
use crate::error::{RestError, RestResult};

/// Per-type grants of the principal behind a `$everything` request. With no
/// principal (authentication off) every type is permitted.
#[derive(Clone)]
pub(crate) struct TypeGrants {
    principal: Option<Principal>,
}

impl TypeGrants {
    pub(crate) fn for_principal(principal: Option<&Principal>) -> Self {
        Self {
            principal: principal.cloned(),
        }
    }

    /// Everything permitted: the authentication-off posture.
    #[cfg(test)]
    pub(crate) fn all() -> Self {
        Self { principal: None }
    }

    /// A compartment member type the walk may search.
    pub(crate) fn permits_member(&self, resource_type: &str) -> bool {
        self.permits(resource_type, FhirOperation::Search)
    }

    /// A supporting resource type the walk may read.
    pub(crate) fn permits_supporting(&self, resource_type: &str) -> bool {
        self.permits(resource_type, FhirOperation::Read)
    }

    fn permits(&self, resource_type: &str, operation: FhirOperation) -> bool {
        match &self.principal {
            None => true,
            Some(p) => SmartScopePolicy::check(p, resource_type, operation).is_ok(),
        }
    }
}

/// Every Patient-compartment member type of `version`, in table order.
pub(crate) fn member_types(version: FhirVersion) -> impl Iterator<Item = &'static str> {
    crate::fhir_types::get_resource_type_names_for_version(version)
        .iter()
        .copied()
        .filter(move |t| {
            *t != "Patient"
                && !helios_fhir::get_compartment_params(version, "Patient", t).is_empty()
        })
}

/// Narrows the member types the walk visits to the ones `grants` permits.
///
/// An explicit `_type` naming a member type the token may not search is
/// refused outright, as an explicit ask. Without `_type`, the walk is
/// narrowed silently and the omitted member types are returned so the
/// response can say so.
pub(crate) fn narrow_member_types(
    params: &mut EverythingParams,
    version: FhirVersion,
    grants: &TypeGrants,
) -> RestResult<Vec<String>> {
    match &params.types {
        Some(wanted) => {
            let members: Vec<&str> = member_types(version).collect();
            if let Some(refused) = wanted
                .iter()
                .find(|t| members.contains(&t.as_str()) && !grants.permits_member(t))
            {
                return Err(RestError::Forbidden {
                    message: format!(
                        "$everything was asked for {refused}, which the token's scopes do not grant"
                    ),
                });
            }
            Ok(Vec::new())
        }
        None => {
            let (permitted, omitted): (Vec<&str>, Vec<&str>) =
                member_types(version).partition(|t| grants.permits_member(t));
            if omitted.is_empty() {
                return Ok(Vec::new());
            }
            params.types = Some(permitted.into_iter().map(String::from).collect());
            Ok(omitted.into_iter().map(String::from).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_auth::ScopeSet;

    fn principal(scopes: &str) -> Principal {
        Principal {
            subject: "s".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            ..Default::default()
        }
    }

    fn params_with(types: Option<&[&str]>) -> EverythingParams {
        let pairs: Vec<(String, String)> = types
            .map(|ts| vec![("_type".to_string(), ts.join(","))])
            .unwrap_or_default();
        EverythingParams::from_pairs(&pairs, FhirVersion::R4, 100).expect("valid params")
    }

    #[test]
    fn no_principal_permits_everything() {
        let grants = TypeGrants::all();
        assert!(grants.permits_member("Observation"));
        assert!(grants.permits_supporting("Practitioner"));
        let mut p = params_with(None);
        assert!(
            narrow_member_types(&mut p, FhirVersion::R4, &grants)
                .unwrap()
                .is_empty()
        );
        assert!(p.types.is_none());
    }

    #[test]
    fn member_types_follow_search_grants_and_supporting_types_read_grants() {
        let p = principal("user/Patient.rs user/Observation.rs user/Practitioner.r");
        let grants = TypeGrants::for_principal(Some(&p));
        assert!(grants.permits_member("Observation"));
        assert!(!grants.permits_member("Condition"));
        assert!(grants.permits_supporting("Practitioner"));
        assert!(!grants.permits_supporting("Organization"));
        // A read-only grant on a member type does not let the walk search it.
        let p = principal("user/Patient.rs user/Condition.r");
        assert!(!TypeGrants::for_principal(Some(&p)).permits_member("Condition"));
    }

    #[test]
    fn a_wildcard_permits_every_member() {
        for scopes in ["system/*.rs", "user/*.rs"] {
            let p = principal(scopes);
            let grants = TypeGrants::for_principal(Some(&p));
            let mut params = params_with(None);
            let omitted = narrow_member_types(&mut params, FhirVersion::R4, &grants).unwrap();
            assert!(omitted.is_empty(), "{scopes}: {omitted:?}");
            assert!(params.types.is_none(), "{scopes}");
        }
    }

    #[test]
    fn without_type_the_walk_is_narrowed_and_the_omissions_reported() {
        let p = principal("user/Patient.rs user/Observation.rs");
        let grants = TypeGrants::for_principal(Some(&p));
        let mut params = params_with(None);
        let omitted = narrow_member_types(&mut params, FhirVersion::R4, &grants).unwrap();
        assert_eq!(
            params.types.as_deref(),
            Some(&["Observation".to_string()][..])
        );
        assert!(omitted.contains(&"Condition".to_string()), "{omitted:?}");
        assert!(omitted.contains(&"Encounter".to_string()), "{omitted:?}");
        assert!(!omitted.contains(&"Observation".to_string()), "{omitted:?}");
        assert!(!omitted.contains(&"Patient".to_string()), "{omitted:?}");

        // Nothing but the Patient: every member type is omitted.
        let p = principal("user/Patient.rs");
        let grants = TypeGrants::for_principal(Some(&p));
        let mut params = params_with(None);
        let omitted = narrow_member_types(&mut params, FhirVersion::R4, &grants).unwrap();
        assert_eq!(params.types.as_deref(), Some(&[][..]));
        assert_eq!(omitted.len(), member_types(FhirVersion::R4).count());
    }

    #[test]
    fn an_explicit_type_outside_the_grants_is_refused() {
        let p = principal("user/Patient.rs user/Observation.rs");
        let grants = TypeGrants::for_principal(Some(&p));

        let mut params = params_with(Some(&["Observation", "Condition"]));
        let err = narrow_member_types(&mut params, FhirVersion::R4, &grants).unwrap_err();
        assert!(
            matches!(err, RestError::Forbidden { ref message } if message.contains("Condition")),
            "{err:?}"
        );

        let mut params = params_with(Some(&["Observation"]));
        assert!(
            narrow_member_types(&mut params, FhirVersion::R4, &grants)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            params.types.as_deref(),
            Some(&["Observation".to_string()][..])
        );
    }
}
