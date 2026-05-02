//! Subscription status state machine.

use std::fmt;

/// The possible states of a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubscriptionStatusCode {
    /// The client has requested the subscription, and the server has not yet
    /// set it to be active.
    Requested,
    /// The subscription is active and notifications will be sent.
    Active,
    /// The server has an error executing the notification or detecting status changes.
    Error,
    /// The subscription has been turned off by the server or client.
    Off,
}

impl SubscriptionStatusCode {
    /// Returns whether a transition from `self` to `target` is valid.
    pub fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            // Requested can become Active (handshake succeeded) or Error or Off.
            (Self::Requested, Self::Active)
                | (Self::Requested, Self::Error)
                | (Self::Requested, Self::Off)
                // Active can become Error (delivery failure) or Off (client/server deactivates).
                | (Self::Active, Self::Error)
                | (Self::Active, Self::Off)
                // Error can recover to Active (after successful retry) or be turned off.
                | (Self::Error, Self::Active)
                | (Self::Error, Self::Off)
        )
    }

    /// Parses a status string from a FHIR Subscription resource.
    pub fn from_fhir_str(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "active" => Some(Self::Active),
            "error" => Some(Self::Error),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// Returns the FHIR string representation.
    pub fn as_fhir_str(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Active => "active",
            Self::Error => "error",
            Self::Off => "off",
        }
    }
}

impl fmt::Display for SubscriptionStatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_fhir_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        use SubscriptionStatusCode::*;

        // From Requested
        assert!(Requested.can_transition_to(Active));
        assert!(Requested.can_transition_to(Error));
        assert!(Requested.can_transition_to(Off));

        // From Active
        assert!(Active.can_transition_to(Error));
        assert!(Active.can_transition_to(Off));

        // From Error
        assert!(Error.can_transition_to(Active));
        assert!(Error.can_transition_to(Off));
    }

    #[test]
    fn test_invalid_transitions() {
        use SubscriptionStatusCode::*;

        // Cannot go backward.
        assert!(!Active.can_transition_to(Requested));
        assert!(!Error.can_transition_to(Requested));
        assert!(!Off.can_transition_to(Requested));

        // Off is terminal (cannot transition anywhere).
        assert!(!Off.can_transition_to(Active));
        assert!(!Off.can_transition_to(Error));
        assert!(!Off.can_transition_to(Off));

        // Cannot stay in same state.
        assert!(!Requested.can_transition_to(Requested));
        assert!(!Active.can_transition_to(Active));
    }

    #[test]
    fn test_fhir_str_roundtrip() {
        for status in [
            SubscriptionStatusCode::Requested,
            SubscriptionStatusCode::Active,
            SubscriptionStatusCode::Error,
            SubscriptionStatusCode::Off,
        ] {
            let s = status.as_fhir_str();
            let parsed = SubscriptionStatusCode::from_fhir_str(s).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_display() {
        assert_eq!(SubscriptionStatusCode::Active.to_string(), "active");
        assert_eq!(SubscriptionStatusCode::Error.to_string(), "error");
    }

    #[test]
    fn test_from_fhir_str_invalid() {
        assert!(SubscriptionStatusCode::from_fhir_str("invalid").is_none());
        assert!(SubscriptionStatusCode::from_fhir_str("").is_none());
    }
}
