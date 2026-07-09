//! User-identity extractor for per-user endpoints.
//!
//! [`UserKey`] derives a stable, opaque key identifying the caller, used to
//! scope the per-user settings store. It reads the authenticated
//! [`Principal`](helios_auth::Principal) injected into request extensions by the
//! auth middleware; when authentication is disabled (no principal present), it
//! falls back to a fixed local key so single-user / development deployments
//! still get a stable place to persist settings.

use axum::{extract::FromRequestParts, http::request::Parts};

/// The key used when no authenticated principal is present (auth disabled).
const LOCAL_USER_KEY: &str = "local|default";

/// A stable, opaque identifier for the calling user.
///
/// Derived as `"{issuer}|{subject}"` from the authenticated principal — the
/// issuer qualifies the subject so identifiers from different identity providers
/// cannot collide — or [`LOCAL_USER_KEY`] when auth is disabled.
#[derive(Debug, Clone)]
pub struct UserKey(pub String);

impl UserKey {
    /// Returns the user key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S> FromRequestParts<S> for UserKey
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let key = match parts.extensions.get::<helios_auth::Principal>() {
            Some(principal) => format!("{}|{}", principal.issuer(), principal.subject()),
            None => LOCAL_USER_KEY.to_string(),
        };
        Ok(UserKey(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use chrono::Utc;
    use helios_auth::Principal;
    use helios_auth::scope::ScopeSet;

    async fn extract(req: Request<()>) -> UserKey {
        let (mut parts, _) = req.into_parts();
        UserKey::from_request_parts(&mut parts, &())
            .await
            .expect("infallible")
    }

    #[tokio::test]
    async fn falls_back_to_local_key_without_principal() {
        let key = extract(Request::builder().body(()).unwrap()).await;
        assert_eq!(key.as_str(), "local|default");
    }

    #[tokio::test]
    async fn derives_issuer_qualified_key_from_principal() {
        let principal = Principal {
            subject: "user-123".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: None,
            scopes: ScopeSet::default(),
            jti: None,
            expires_at: Utc::now(),
            custom_claims: serde_json::Map::new(),
        };
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(principal);
        let key = extract(req).await;
        assert_eq!(key.as_str(), "https://idp.example.com|user-123");
    }
}
