use std::fmt;

/// Errors that can occur during authentication and authorization.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No Authorization header present.
    #[error("Missing Authorization header")]
    MissingToken,

    /// Authorization header is malformed.
    #[error("Invalid token format: {0}")]
    InvalidTokenFormat(String),

    /// Token has expired.
    #[error("Token expired")]
    TokenExpired,

    /// Token signature is invalid.
    #[error("Invalid signature")]
    InvalidSignature,

    /// JWT algorithm is not in the allowed list.
    #[error("Unsupported algorithm: {alg}")]
    UnsupportedAlgorithm {
        /// The algorithm that was rejected.
        alg: String,
    },

    /// Key ID from JWT header not found in JWKS.
    #[error("Unknown key ID: {kid}")]
    UnknownKid {
        /// The key ID that was not found.
        kid: String,
    },

    /// Token with this JTI has already been used.
    #[error("JTI replay detected: {jti}")]
    ReplayDetected {
        /// The replayed JWT ID.
        jti: String,
    },

    /// Authenticated principal lacks required scopes.
    #[error("Forbidden: insufficient scope for {operation} on {resource_type}")]
    Forbidden {
        /// The FHIR resource type.
        resource_type: String,
        /// The operation that was attempted.
        operation: String,
    },

    /// Failed to fetch JWKS from the endpoint.
    #[error("JWKS fetch error: {0}")]
    JwksFetchError(String),

    /// General token validation error.
    #[error("Token validation error: {0}")]
    ValidationError(String),

    /// Internal error in the auth subsystem.
    #[error("Internal auth error: {0}")]
    InternalError(String),
}

/// The kind of FHIR operation being performed, used for scope checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FhirOperation {
    /// Read a resource by ID.
    Read,
    /// Search for resources.
    Search,
    /// Create a new resource.
    Create,
    /// Update an existing resource.
    Update,
    /// Delete a resource.
    Delete,
}

impl fmt::Display for FhirOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FhirOperation::Read => write!(f, "read"),
            FhirOperation::Search => write!(f, "search"),
            FhirOperation::Create => write!(f, "create"),
            FhirOperation::Update => write!(f, "update"),
            FhirOperation::Delete => write!(f, "delete"),
        }
    }
}
