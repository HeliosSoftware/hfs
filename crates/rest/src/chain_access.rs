//! Authorization of the hops of a chained or `_has` search (#2501).
//!
//! The authorization middleware judges a search by its base type only, and
//! [`PatientCompartment`] confines its base query. A chained parameter
//! (`Observation?subject:Patient.name=…`) or a reverse chain
//! (`Patient?_has:Observation:subject:code=…`) also searches other types —
//! one search per hop, built by the chain resolver — and decides which base
//! resources match by what those searches find. Left unjudged, a hop runs
//! over the whole tenant: a `user/Patient.rs` token learns which patients
//! have an Observation with a given code, and a `patient/` token learns, one
//! value per request, whether another patient's resource names its launch
//! patient.
//!
//! [`ChainAccess`] judges every hop as the same token's direct search on the
//! hop's type would be judged:
//!
//! - a type the token may not search refuses the whole search with `403`;
//! - a type granted only by a `patient/` scope is confined to the launch
//!   patient's compartment ([`PatientCompartment::confine_query`]): a member
//!   type gets the compartment filter, `Patient` is pinned to the launch
//!   patient, and a type outside the compartment (`Practitioner`,
//!   `Organization`) is refused;
//! - a type granted by a `user/` or `system/` scope runs as before.
//!
//! An untyped hop over a polymorphic reference has several candidate types;
//! a refused candidate contributes nothing and the search is refused only when
//! every candidate is (see [`ChainHopGuard`]).
//!
//! Without a [`Principal`] (authentication disabled) nothing is judged.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use helios_auth::{AccessScope, FhirOperation, Principal, SmartScopePolicy};
use helios_fhir::FhirVersion;
use helios_persistence::error::{SearchError, StorageError, StorageResult};
use helios_persistence::search::ChainHopGuard;
use helios_persistence::types::SearchQuery;

use crate::error::RestError;
use crate::patient_compartment::PatientCompartment;

/// The token a chained or `_has` search's hops are judged against.
///
/// As an extractor it never fails; it reads the [`Principal`] the
/// authentication middleware stored in the request extensions.
#[derive(Debug, Clone, Default)]
pub struct ChainAccess(Option<Principal>);

impl ChainAccess {
    /// Hops are not judged: authentication is disabled.
    pub fn unrestricted() -> Self {
        Self(None)
    }

    /// Hops are judged against `principal`'s scopes, when there is one.
    pub fn for_principal(principal: Option<&Principal>) -> Self {
        Self(principal.cloned())
    }

    /// The principal whose grants judge the hops, or `None` when every hop is
    /// permitted (authentication off).
    pub fn principal(&self) -> Option<&Principal> {
        self.0.as_ref()
    }

    /// The guard to hand the chain resolver for a search in `version`, or
    /// `None` when hops are not judged.
    pub fn guard(&self, version: FhirVersion) -> Option<ChainHopAccess<'_>> {
        self.0
            .as_ref()
            .map(|principal| ChainHopAccess { principal, version })
    }
}

impl<S> FromRequestParts<S> for ChainAccess
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::for_principal(parts.extensions.get::<Principal>()))
    }
}

/// [`ChainHopGuard`] for one token and FHIR version.
pub struct ChainHopAccess<'a> {
    principal: &'a Principal,
    version: FhirVersion,
}

impl ChainHopGuard for ChainHopAccess<'_> {
    fn confine_hop(&self, query: &mut SearchQuery) -> StorageResult<()> {
        let resource_type = query.resource_type.clone();
        let refused = |reason: String| {
            StorageError::Search(SearchError::ChainHopRefused {
                message: format!(
                    "a chained or _has search through {resource_type} is not permitted: {reason}"
                ),
            })
        };
        match SmartScopePolicy::check(self.principal, &resource_type, FhirOperation::Search) {
            Ok(AccessScope::Unrestricted) => Ok(()),
            Ok(AccessScope::PatientCompartment { patient_id }) => {
                PatientCompartment::confined_to(patient_id)
                    .confine_query(self.version, query)
                    .map_err(|e| match e {
                        RestError::Forbidden { message } => refused(message),
                        other => refused(other.to_string()),
                    })
            }
            Err(e) => Err(refused(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_auth::{LaunchContext, ScopeSet};

    fn principal(scopes: &str, patient: Option<&str>) -> Principal {
        Principal {
            subject: "client".to_string(),
            scopes: ScopeSet::parse(scopes),
            launch_context: patient.map(|p| LaunchContext {
                patient: Some(p.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn judge(access: &ChainAccess, resource_type: &str) -> StorageResult<SearchQuery> {
        let mut query = SearchQuery::new(resource_type);
        if let Some(guard) = access.guard(FhirVersion::R4) {
            guard.confine_hop(&mut query)?;
        }
        Ok(query)
    }

    fn is_refused(result: StorageResult<SearchQuery>) -> bool {
        matches!(
            result,
            Err(StorageError::Search(SearchError::ChainHopRefused { .. }))
        )
    }

    #[test]
    fn without_a_principal_nothing_is_judged() {
        let access = ChainAccess::unrestricted();
        let query = judge(&access, "Practitioner").unwrap();
        assert!(query.parameters.is_empty() && query.compartment.is_none());
    }

    #[test]
    fn a_type_the_token_cannot_search_is_refused() {
        let access = ChainAccess::for_principal(Some(&principal("user/Patient.rs", None)));
        assert!(judge(&access, "Patient").is_ok());
        assert!(is_refused(judge(&access, "Observation")));
    }

    #[test]
    fn a_patient_scope_confines_each_hop() {
        let access = ChainAccess::for_principal(Some(&principal("patient/*.rs", Some("p1"))));
        let observations = judge(&access, "Observation").unwrap();
        let membership = observations.compartment.expect("compartment filter");
        assert_eq!(membership.reference, "Patient/p1");

        let patients = judge(&access, "Patient").unwrap();
        assert_eq!(patients.parameters[0].name, "_id");
        assert_eq!(patients.parameters[0].values[0].value, "p1");

        assert!(is_refused(judge(&access, "Practitioner")));
    }

    #[test]
    fn a_wider_grant_leaves_its_types_unconfined() {
        let access = ChainAccess::for_principal(Some(&principal(
            "user/Practitioner.rs patient/*.rs",
            Some("p1"),
        )));
        let practitioners = judge(&access, "Practitioner").unwrap();
        assert!(practitioners.parameters.is_empty() && practitioners.compartment.is_none());
        assert!(judge(&access, "Observation").unwrap().compartment.is_some());
    }
}
