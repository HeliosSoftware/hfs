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
    /// The subscription was created in error and must never be acted on.
    ///
    /// Client-owned and terminal: the server never transitions *to* it and
    /// never transitions *out* of it. Present in the R4B/R5/R6 value set (and
    /// absent from R4's), but parsed for every version — an unknown code
    /// previously fell back to [`Requested`](Self::Requested), which made the
    /// engine handshake and *activate* a resource the client had explicitly
    /// retracted.
    EnteredInError,
}

impl SubscriptionStatusCode {
    /// Returns whether a transition from `self` to `target` is valid.
    ///
    /// Note that both [`Off`](Self::Off) and
    /// [`EnteredInError`](Self::EnteredInError) are terminal: no transition
    /// leaves them, so a subscription only returns to service when the client
    /// re-arms it by rewriting the resource (see
    /// [`SubscriptionManager::register`](crate::manager::SubscriptionManager::register)).
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

    /// Whether the server should stop acting on a subscription in this state.
    ///
    /// Terminal states are still *registered* (so `$status` keeps answering for
    /// them) but never dispatched to and never handshaken.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Off | Self::EnteredInError)
    }

    /// Parses a status string from a FHIR Subscription resource.
    pub fn from_fhir_str(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "active" => Some(Self::Active),
            "error" => Some(Self::Error),
            "off" => Some(Self::Off),
            "entered-in-error" => Some(Self::EnteredInError),
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
            Self::EnteredInError => "entered-in-error",
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

    /// `entered-in-error` is client-owned and inert: the server neither enters
    /// nor leaves it. Before it was parsed, `from_fhir_str` returned `None` and
    /// `register` fell back to `Requested`, so rehydration would handshake and
    /// *activate* a subscription the client had retracted.
    #[test]
    fn test_entered_in_error_is_inert() {
        use SubscriptionStatusCode::*;

        assert_eq!(
            SubscriptionStatusCode::from_fhir_str("entered-in-error"),
            Some(EnteredInError)
        );
        assert_eq!(EnteredInError.as_fhir_str(), "entered-in-error");

        for target in [Requested, Active, Error, Off, EnteredInError] {
            assert!(
                !EnteredInError.can_transition_to(target),
                "entered-in-error must not transition to {target}"
            );
            assert!(
                !target.can_transition_to(EnteredInError),
                "{target} must not transition to entered-in-error"
            );
        }
    }

    #[test]
    fn test_is_terminal() {
        use SubscriptionStatusCode::*;

        assert!(Off.is_terminal());
        assert!(EnteredInError.is_terminal());
        assert!(!Requested.is_terminal());
        assert!(!Active.is_terminal());
        // `error` is NOT terminal: a restart re-runs activation for it, which is
        // the only recovery path the server has today.
        assert!(!Error.is_terminal());
    }

    #[test]
    fn test_fhir_str_roundtrip() {
        for status in [
            SubscriptionStatusCode::Requested,
            SubscriptionStatusCode::Active,
            SubscriptionStatusCode::Error,
            SubscriptionStatusCode::Off,
            SubscriptionStatusCode::EnteredInError,
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
