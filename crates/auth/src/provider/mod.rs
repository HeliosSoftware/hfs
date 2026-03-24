pub mod jwks_bearer;

use async_trait::async_trait;

use crate::error::AuthError;
use crate::principal::Principal;

/// Trait for authenticating incoming requests.
///
/// The auth middleware calls `authenticate()` with the raw `Authorization`
/// header value. Implementations validate the token and return a `Principal`
/// on success.
#[async_trait]
pub trait AuthProvider: Send + Sync + 'static {
    /// Authenticate from the Authorization header value (e.g., "Bearer <token>").
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError>;

    /// Returns the provider name for logging/diagnostics.
    fn name(&self) -> &str;
}
