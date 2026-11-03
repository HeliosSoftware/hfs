//! Identity of the agent recorded on an `AuditEvent`.
//!
//! IHE BALP identifies the user by `agent.who.identifier` (the token subject,
//! scoped by its issuer). A bare `sub` is not a FHIR reference, so it must not
//! go in `who.reference`; that is reserved for a literal reference such as the
//! SMART `fhirUser` claim (#1515).

use crate::fhir_model::{Identifier, Reference};
use crate::helpers::{fhir_string, uri};

/// Who performed an audited action.
///
/// Set this in request extensions (e.g., from the auth middleware) so the
/// audit middleware can record who performed the action. This decouples the
/// audit crate from the auth crate's `Principal` type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditAgent {
    /// Subject identifier, normally the token `sub` claim. Recorded as
    /// `who.identifier.value`.
    pub subject: String,
    /// Issuer that scopes [`subject`](Self::subject), normally the token
    /// `iss` claim. Recorded as `who.identifier.system`.
    pub issuer: Option<String>,
    /// Literal FHIR reference to the agent, e.g. the SMART `fhirUser` claim
    /// (`Practitioner/123`). Recorded as `who.reference`.
    pub reference: Option<String>,
    /// Human-readable name, e.g. `preferred_username`. Recorded as
    /// `who.display`.
    pub display: Option<String>,
}

impl AuditAgent {
    /// An agent known only by its subject identifier.
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            ..Default::default()
        }
    }

    /// Classifies a single "who" string: a literal reference
    /// (`Practitioner/123`, an absolute URL, a `urn:`) becomes
    /// `who.reference`, anything else is treated as a subject identifier.
    pub fn from_who(who: &str) -> Self {
        if looks_like_reference(who) {
            Self {
                reference: Some(who.to_string()),
                ..Default::default()
            }
        } else {
            Self::new(who)
        }
    }

    /// The `AuditEvent.agent.who` value, or `None` when nothing identifies
    /// the agent.
    pub(crate) fn to_reference(&self) -> Option<Reference> {
        let non_empty =
            |s: &Option<String>| s.as_deref().filter(|v| !v.is_empty()).map(str::to_owned);
        let identifier = (!self.subject.is_empty()).then(|| {
            Box::new(Identifier {
                system: non_empty(&self.issuer).map(uri),
                value: Some(fhir_string(self.subject.clone())),
                ..Default::default()
            })
        });
        let reference = non_empty(&self.reference).map(fhir_string);
        let display = non_empty(&self.display).map(fhir_string);
        if identifier.is_none() && reference.is_none() && display.is_none() {
            return None;
        }
        Some(Reference {
            reference,
            identifier,
            display,
            ..Default::default()
        })
    }
}

/// Whether `value` is usable as `Reference.reference`: a relative
/// `Type/id` reference, an absolute URL, or a URN.
fn looks_like_reference(value: &str) -> bool {
    if value.starts_with("urn:") || value.starts_with("http://") || value.starts_with("https://") {
        return true;
    }
    let Some((resource_type, id)) = value.split_once('/') else {
        return false;
    };
    resource_type.starts_with(|c: char| c.is_ascii_uppercase())
        && resource_type.chars().all(|c| c.is_ascii_alphanumeric())
        && !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(s: &Option<crate::fhir_model::String>) -> Option<&str> {
        s.as_ref().and_then(|v| v.value.as_deref())
    }

    #[test]
    fn bare_subject_goes_to_identifier_not_reference() {
        let who = AuditAgent::from_who("937f5c4f-1b2c-4d5e-8f90-123456789abc")
            .to_reference()
            .unwrap();
        assert!(who.reference.is_none());
        let identifier = who.identifier.unwrap();
        assert_eq!(
            value(&identifier.value),
            Some("937f5c4f-1b2c-4d5e-8f90-123456789abc")
        );
        assert!(identifier.system.is_none());
    }

    #[test]
    fn literal_reference_goes_to_reference() {
        for r in [
            "Practitioner/dr-smith",
            "Device/hfs",
            "https://fhir.example.org/Practitioner/1",
            "urn:uuid:937f5c4f-1b2c-4d5e-8f90-123456789abc",
        ] {
            let who = AuditAgent::from_who(r).to_reference().unwrap();
            assert_eq!(value(&who.reference), Some(r), "{r}");
            assert!(who.identifier.is_none(), "{r}");
        }
    }

    #[test]
    fn subject_with_slash_that_is_not_a_reference_stays_an_identifier() {
        let who = AuditAgent::from_who("auth0|abc/def")
            .to_reference()
            .unwrap();
        assert!(who.reference.is_none());
    }

    #[test]
    fn full_identity_fills_every_field() {
        let who = AuditAgent {
            subject: "937f5c4f".into(),
            issuer: Some("https://idp.example.org/realms/fhir".into()),
            reference: Some("Practitioner/123".into()),
            display: Some("demo".into()),
        }
        .to_reference()
        .unwrap();
        assert_eq!(value(&who.reference), Some("Practitioner/123"));
        assert_eq!(value(&who.display), Some("demo"));
        let identifier = who.identifier.unwrap();
        assert_eq!(
            identifier.system.as_ref().and_then(|s| s.value.as_deref()),
            Some("https://idp.example.org/realms/fhir")
        );
        assert_eq!(value(&identifier.value), Some("937f5c4f"));
    }

    #[test]
    fn empty_agent_has_no_who() {
        assert!(AuditAgent::default().to_reference().is_none());
    }
}
