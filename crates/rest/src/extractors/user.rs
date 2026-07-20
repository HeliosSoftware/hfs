//! User-identity extractor for per-user endpoints.
//!
//! [`UserKey`] derives a stable, opaque key identifying the caller, used to
//! scope the per-user settings store. It reads the authenticated
//! [`Principal`](helios_auth::Principal) injected into request extensions by the
//! auth middleware; when authentication is disabled (no principal present), it
//! falls back to a fixed local key so single-user / development deployments
//! still get a stable place to persist settings.
//!
//! # Why the encoding is length-prefixed
//!
//! The key must be *injective*: two distinct principals must never derive the
//! same string, because that string is the primary key of the settings document
//! on every backend. The obvious `format!("{iss}|{sub}")` is not injective —
//! `(iss = "https://idp", sub = "a|b")` and `(iss = "https://idp|a", sub = "b")`
//! both render as `https://idp|a|b`, so the two principals would share (read
//! *and* overwrite) one document. Subjects containing `|` are not exotic:
//! Auth0 and Cognito emit identifiers such as `google-oauth2|1076…`.
//!
//! Prefixing the issuer with its byte length removes the ambiguity for *any*
//! byte content, without reserving a character that either component might
//! contain. A NUL separator would also work in theory but is unusable in
//! practice: PostgreSQL rejects `U+0000` in `TEXT`, and `user_key` is a
//! `TEXT PRIMARY KEY` there.
//!
//! See issue #270.

use axum::{extract::FromRequestParts, http::request::Parts};

/// Tag for keys derived from an authenticated principal.
const PRINCIPAL_TAG: &str = "u2";

/// The complete key used when no authenticated principal is present (auth
/// disabled). It carries no length-prefixed body, so no principal-derived key
/// can ever equal it.
const LOCAL_KEY: &str = "l2:";

/// The pre-#270 auth-disabled key, migrated from on first access.
const LEGACY_LOCAL_KEY: &str = "local|default";

/// A stable, opaque identifier for the calling user.
///
/// Derived from the authenticated principal as
/// `u2:{issuer_len}:{issuer}:{subject}`, or the fixed key `l2:` when auth is
/// disabled.
///
/// The fields are deliberately private and there is no public constructor
/// taking a string, no `From<String>`, no `FromStr` and no
/// `Deserialize`: the only two ways to obtain a `UserKey` are
/// [`from_principal`](Self::from_principal) and [`local`](Self::local), so a
/// caller cannot hand-assemble a key that collides with another user's.
///
/// This is a deliberate divergence from `TenantId`, which *is* freely
/// constructible because tenant identifiers legitimately arrive from
/// configuration and request headers. A user key has exactly two legitimate
/// origins; please don't add more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKey {
    key: String,
    legacy: Option<String>,
}

impl UserKey {
    /// Derives the key for an authenticated principal.
    pub fn from_principal(principal: &helios_auth::Principal) -> Self {
        let (issuer, subject) = (principal.issuer(), principal.subject());
        Self {
            key: encode(issuer, subject),
            legacy: legacy_key(issuer, subject),
        }
    }

    /// The key used when authentication is disabled.
    pub fn local() -> Self {
        Self {
            key: LOCAL_KEY.to_string(),
            legacy: Some(LEGACY_LOCAL_KEY.to_string()),
        }
    }

    /// Returns the user key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.key
    }

    /// The pre-#270 key (`"{issuer}|{subject}"`) holding this user's settings,
    /// when that key is provably unambiguous.
    ///
    /// `None` when either component contains the `|` delimiter. In that case the
    /// legacy key may be shared with a *different* principal, and its document
    /// may already hold commingled data; silently promoting it to the new
    /// (correct) key would launder one user's settings into another's. Those
    /// callers correctly start from an empty document.
    ///
    /// Used only to migrate a document written before this encoding existed;
    /// see `handlers::user_settings::adopt_legacy_document`.
    pub fn legacy_key(&self) -> Option<&str> {
        self.legacy.as_deref()
    }
}

/// Encodes `(issuer, subject)` injectively.
///
/// The issuer is length-prefixed (in bytes), so the decoder — and therefore any
/// collision — is unambiguous regardless of what either component contains. The
/// subject needs no prefix: it is the entire remainder of the string.
fn encode(issuer: &str, subject: &str) -> String {
    format!("{PRINCIPAL_TAG}:{}:{issuer}:{subject}", issuer.len())
}

/// The pre-#270 encoding, offered for migration only when it is provably
/// unambiguous — that is, when neither component contains the delimiter, so the
/// single `|` in the result can only have come from this principal.
fn legacy_key(issuer: &str, subject: &str) -> Option<String> {
    if issuer.contains('|') || subject.contains('|') {
        return None;
    }
    Some(format!("{issuer}|{subject}"))
}

impl<S> FromRequestParts<S> for UserKey
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(match parts.extensions.get::<helios_auth::Principal>() {
            Some(principal) => UserKey::from_principal(principal),
            None => UserKey::local(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use chrono::Utc;
    use helios_auth::Principal;
    use helios_auth::scope::ScopeSet;

    fn principal(issuer: &str, subject: &str) -> Principal {
        Principal {
            subject: subject.to_string(),
            issuer: issuer.to_string(),
            tenant_id: None,
            scopes: ScopeSet::default(),
            jti: None,
            expires_at: Utc::now(),
            custom_claims: serde_json::Map::new(),
        }
    }

    async fn extract(req: Request<()>) -> UserKey {
        let (mut parts, _) = req.into_parts();
        UserKey::from_request_parts(&mut parts, &())
            .await
            .expect("infallible")
    }

    #[tokio::test]
    async fn falls_back_to_local_key_without_principal() {
        let key = extract(Request::builder().body(()).unwrap()).await;
        assert_eq!(key.as_str(), "l2:");
    }

    #[tokio::test]
    async fn derives_issuer_qualified_key_from_principal() {
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut()
            .insert(principal("https://idp.example.com", "user-123"));
        let key = extract(req).await;
        assert_eq!(key.as_str(), "u2:23:https://idp.example.com:user-123");
    }

    /// The regression this encoding exists to prevent (#270): under the old
    /// `"{iss}|{sub}"` format these two principals derived the *same* key and
    /// therefore shared one settings document.
    #[test]
    fn colliding_principals_derive_distinct_keys() {
        let a = UserKey::from_principal(&principal("https://idp", "a|b"));
        let b = UserKey::from_principal(&principal("https://idp|a", "b"));

        // Both still render identically under the legacy encoding …
        assert_eq!(
            format!("{}|{}", "https://idp", "a|b"),
            format!("{}|{}", "https://idp|a", "b"),
        );
        // … but not under this one.
        assert_ne!(a, b);
    }

    /// A subject containing the legacy delimiter is routine (Auth0, Cognito).
    #[test]
    fn subject_containing_delimiter_round_trips() {
        let key = UserKey::from_principal(&principal("https://idp", "google-oauth2|1076"));
        assert_eq!(key.as_str(), "u2:11:https://idp:google-oauth2|1076");
    }

    /// No principal-derived key can equal the auth-disabled key, whatever the
    /// token claims say — the tags differ and `u2:` is always followed by digits.
    #[test]
    fn principal_cannot_forge_the_local_key() {
        for (iss, sub) in [("l2", ""), ("", ""), ("local", "default"), ("l2:", "")] {
            assert_ne!(
                UserKey::from_principal(&principal(iss, sub)),
                UserKey::local()
            );
        }
    }

    /// An empty issuer (a token with no `iss`, accepted when no issuer is
    /// pinned) must still not collide with a different principal.
    #[test]
    fn empty_issuer_is_unambiguous() {
        let a = UserKey::from_principal(&principal("", "alice"));
        let b = UserKey::from_principal(&principal("", "bob"));
        let c = UserKey::from_principal(&principal("a", "lice"));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.as_str(), "u2:0::alice");
    }

    /// Multi-byte issuers are length-prefixed in *bytes*, matching `str::len`.
    #[test]
    fn multibyte_issuer_uses_byte_length() {
        let key = UserKey::from_principal(&principal("héllo", "s"));
        assert_eq!(key.as_str(), "u2:6:héllo:s");
    }

    #[test]
    fn legacy_key_is_offered_only_when_unambiguous() {
        assert_eq!(
            UserKey::from_principal(&principal("https://idp", "user-1")).legacy_key(),
            Some("https://idp|user-1"),
        );
        // Ambiguous: the legacy document may be shared with another principal,
        // so it must not be promoted to the new key.
        assert_eq!(
            UserKey::from_principal(&principal("https://idp", "a|b")).legacy_key(),
            None
        );
        assert_eq!(
            UserKey::from_principal(&principal("https://idp|a", "b")).legacy_key(),
            None
        );
    }

    #[test]
    fn local_key_migrates_from_its_own_legacy_form() {
        assert_eq!(UserKey::local().legacy_key(), Some("local|default"));
    }
}
