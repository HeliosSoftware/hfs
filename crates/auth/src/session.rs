//! Interactive browser login sessions for the web UI (issue #1449).
//!
//! HFS is not the authorization server: the browser is sent to the IdP
//! (Keycloak, Okta, Auth0, Entra ID — any OpenID Connect provider) with the
//! **Authorization Code + PKCE** grant, the IdP authenticates the user and
//! returns a `code`, and HFS exchanges it for tokens. The tokens never reach the
//! browser: they live in a server-side [`SessionStore`], referenced by an
//! `HttpOnly` cookie. The rest of the server then treats a request carrying a
//! valid session cookie exactly as if it had sent `Authorization: Bearer
//! <access_token>` — the auth middleware injects that header, so token
//! validation, scope authorization and audit run unchanged.
//!
//! This module holds everything both halves share: the UI crate drives the
//! login/callback/logout endpoints, the REST crate's auth middleware reads
//! sessions. Neither crate depends on the other, so the shared pieces live here.
//!
//! The store is in-process. A session therefore does not survive a restart and
//! is not visible to another node; a store-backed, cluster-safe session is a
//! follow-up.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::AuthError;

/// Name of the cookie carrying the session id.
pub const SESSION_COOKIE: &str = "hfs_session";

/// Name of the short-lived cookie tying a browser to its pending login
/// (the `state` + PKCE verifier waiting for the IdP to call back).
pub const PENDING_COOKIE: &str = "hfs_login";

/// How long a login may stay pending between the redirect to the IdP and the
/// callback before it is discarded.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// How long an idle session lives with no refresh token to renew it.
const SESSION_IDLE_TTL: Duration = Duration::from_secs(8 * 60 * 60);

/// Renew an access token this long before it actually expires, so a request
/// never goes out with a token about to lapse mid-flight.
const REFRESH_SKEW: Duration = Duration::from_secs(30);

/// The IdP client configuration the login flow drives.
#[derive(Debug, Clone)]
pub struct LoginConfig {
    /// OAuth client id registered at the IdP for the web UI (e.g. `hfs-web`).
    pub client_id: String,
    /// Client secret, for a confidential client. A public client (PKCE only)
    /// leaves this `None`.
    pub client_secret: Option<String>,
    /// The `redirect_uri` registered at the IdP — HFS's own `/ui/callback`.
    pub redirect_uri: String,
    /// Scopes requested at authorization (space separated).
    pub scopes: String,
    /// IdP authorization endpoint.
    pub authorization_endpoint: String,
    /// IdP token endpoint (code exchange and refresh).
    pub token_endpoint: String,
    /// IdP end-session (RP-initiated logout) endpoint, when it has one.
    pub end_session_endpoint: Option<String>,
    /// Whether the session cookie is marked `Secure`. Off only for plain-HTTP
    /// local development.
    pub cookie_secure: bool,
}

/// The identity a session was established for, from the ID token's claims.
#[derive(Debug, Clone)]
pub struct SessionPrincipal {
    /// `sub`.
    pub subject: String,
    /// `iss`.
    pub issuer: String,
    /// `name`, when the IdP sent one.
    pub name: Option<String>,
    /// `preferred_username`, when the IdP sent one.
    pub preferred_username: Option<String>,
    /// `email`, when the IdP sent one.
    pub email: Option<String>,
    /// `picture`, when the IdP sent one.
    pub picture: Option<String>,
}

impl SessionPrincipal {
    /// The name to show in the UI: `name`, else `preferred_username`, else
    /// `email`, else the bare subject.
    pub fn display(&self) -> &str {
        self.name
            .as_deref()
            .or(self.preferred_username.as_deref())
            .or(self.email.as_deref())
            .unwrap_or(&self.subject)
    }
}

/// One established login.
#[derive(Debug, Clone)]
pub struct Session {
    /// The session id — the cookie value. Opaque and random.
    pub id: String,
    /// Who is logged in.
    pub principal: SessionPrincipal,
    /// The bearer used for FHIR calls on this user's behalf.
    pub access_token: String,
    /// When `access_token` expires.
    pub access_expires_at: Instant,
    /// The refresh token, when the IdP issued one.
    pub refresh_token: Option<String>,
    /// The ID token, kept for RP-initiated logout (`id_token_hint`).
    pub id_token: Option<String>,
    /// Last time the session was used; idle sessions are dropped.
    pub last_seen: Instant,
    /// When the session was established.
    pub created_at: DateTime<Utc>,
}

/// A login that has been started (the browser was sent to the IdP) but not
/// yet completed. Consumed exactly once by the callback.
#[derive(Debug, Clone)]
struct PendingLogin {
    /// CSRF `state` the IdP must echo back.
    state: String,
    /// PKCE verifier whose S256 challenge went out with the authorize request.
    code_verifier: String,
    /// Where to land after login — a UI path the user originally asked for.
    next: String,
    started_at: Instant,
}

/// What the middleware learns when it resolves a session cookie.
#[derive(Debug)]
pub enum AccessOutcome {
    /// A usable bearer for this request.
    Token(String),
    /// No such session (never existed, expired, or logged out).
    NoSession,
}

/// The token endpoint's answer to a code exchange or a refresh.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Server-side store of pending logins and established sessions.
pub struct SessionStore {
    config: LoginConfig,
    http: reqwest::Client,
    pending: RwLock<HashMap<String, PendingLogin>>,
    sessions: RwLock<HashMap<String, Session>>,
}

impl SessionStore {
    /// Creates an empty store for `config`.
    pub fn new(config: LoginConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            pending: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// The client configuration this store drives.
    pub fn config(&self) -> &LoginConfig {
        &self.config
    }

    /// Starts a login: mints `state` + a PKCE verifier, remembers them under a
    /// fresh pending id, and returns `(pending_id, authorize_url)`. The caller
    /// sets `pending_id` as the [`PENDING_COOKIE`] and redirects the browser
    /// to `authorize_url`.
    pub fn begin(&self, next: &str) -> (String, String) {
        self.sweep_pending();
        let pending_id = random_token(32);
        let state = random_token(32);
        let code_verifier = random_token(64);
        let challenge = code_challenge_s256(&code_verifier);

        let authorize_url = build_authorize_url(
            &self.config.authorization_endpoint,
            &self.config.client_id,
            &self.config.redirect_uri,
            &self.config.scopes,
            &state,
            &challenge,
        );

        let next = if is_safe_next(next) {
            next.to_string()
        } else {
            "/ui".to_string()
        };
        write(&self.pending).insert(
            pending_id.clone(),
            PendingLogin {
                state,
                code_verifier,
                next,
                started_at: Instant::now(),
            },
        );
        (pending_id, authorize_url)
    }

    /// Completes a login from the IdP callback. Consumes the pending login
    /// (a second callback with the same pending id is rejected), checks
    /// `state`, exchanges `code` with the PKCE verifier, and establishes the
    /// session. Returns the new session and the `next` path to land on.
    pub async fn complete(
        &self,
        pending_id: &str,
        state: &str,
        code: &str,
    ) -> Result<(Session, String), AuthError> {
        let pending = write(&self.pending)
            .remove(pending_id)
            .ok_or_else(|| AuthError::ValidationError("login is not pending".to_string()))?;
        if pending.started_at.elapsed() > PENDING_TTL {
            return Err(AuthError::ValidationError(
                "login attempt expired".to_string(),
            ));
        }
        if !constant_time_eq(pending.state.as_bytes(), state.as_bytes()) {
            return Err(AuthError::ValidationError("state mismatch".to_string()));
        }

        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", self.config.redirect_uri.clone()),
            ("client_id", self.config.client_id.clone()),
            ("code_verifier", pending.code_verifier.clone()),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let tokens = self.post_token(&form).await?;

        let id_claims = tokens
            .id_token
            .as_deref()
            .and_then(unverified_claims)
            .unwrap_or(Value::Null);
        // The ID token is what names the user; a token endpoint that sent none
        // (a plain OAuth server) still yields an access token whose `sub`
        // identifies the caller.
        let access_claims = unverified_claims(&tokens.access_token).unwrap_or(Value::Null);
        let claims = if id_claims.is_null() {
            &access_claims
        } else {
            &id_claims
        };
        let principal = principal_from_claims(claims)?;

        let session = Session {
            id: random_token(32),
            principal,
            access_token: tokens.access_token,
            access_expires_at: expiry(tokens.expires_in),
            refresh_token: tokens.refresh_token,
            id_token: tokens.id_token,
            last_seen: Instant::now(),
            created_at: Utc::now(),
        };
        write(&self.sessions).insert(session.id.clone(), session.clone());
        Ok((session, pending.next))
    }

    /// Adds an already-established session — for an embedder that obtains
    /// tokens by other means, and for tests that need a session without an
    /// IdP. The normal path is [`Self::complete`].
    pub fn insert(&self, session: Session) {
        write(&self.sessions).insert(session.id.clone(), session);
    }

    /// Looks up the session for a cookie value without touching the IdP.
    pub fn get(&self, session_id: &str) -> Option<Session> {
        let sessions = read(&self.sessions);
        let session = sessions.get(session_id)?;
        if session.last_seen.elapsed() > SESSION_IDLE_TTL {
            return None;
        }
        Some(session.clone())
    }

    /// Resolves a session cookie to a bearer for the current request,
    /// refreshing the access token when it is about to expire. An expired
    /// session with no working refresh token is dropped and reported as
    /// [`AccessOutcome::NoSession`], so the caller falls back to "not logged
    /// in" rather than forwarding a dead token.
    pub async fn access_token(&self, session_id: &str) -> AccessOutcome {
        let Some(session) = self.get(session_id) else {
            return AccessOutcome::NoSession;
        };
        let fresh = session
            .access_expires_at
            .saturating_duration_since(Instant::now())
            > REFRESH_SKEW;
        if fresh {
            self.touch(session_id);
            return AccessOutcome::Token(session.access_token);
        }
        match self
            .refresh(session_id, session.refresh_token.as_deref())
            .await
        {
            Ok(token) => AccessOutcome::Token(token),
            Err(err) => {
                tracing::info!(error = %err, "session refresh failed; dropping session");
                self.remove(session_id);
                AccessOutcome::NoSession
            }
        }
    }

    /// Ends a session. Returns what was needed for RP-initiated logout at the
    /// IdP (the end-session endpoint and the ID token hint), when configured.
    pub fn logout(&self, session_id: &str) -> Option<(String, Option<String>)> {
        let removed = self.remove(session_id);
        let endpoint = self.config.end_session_endpoint.clone()?;
        Some((endpoint, removed.and_then(|s| s.id_token)))
    }

    /// Number of live sessions — for tests and diagnostics.
    pub fn len(&self) -> usize {
        read(&self.sessions).len()
    }

    /// Whether there are no live sessions.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    async fn refresh(
        &self,
        session_id: &str,
        refresh_token: Option<&str>,
    ) -> Result<String, AuthError> {
        let refresh_token = refresh_token.ok_or(AuthError::TokenExpired)?.to_string();
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", self.config.client_id.clone()),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let tokens = self.post_token(&form).await?;
        let mut sessions = write(&self.sessions);
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| AuthError::ValidationError("session vanished".to_string()))?;
        session.access_token = tokens.access_token.clone();
        session.access_expires_at = expiry(tokens.expires_in);
        // An IdP that rotates refresh tokens sends a new one; one that does not
        // keeps the old one valid.
        if tokens.refresh_token.is_some() {
            session.refresh_token = tokens.refresh_token;
        }
        if tokens.id_token.is_some() {
            session.id_token = tokens.id_token;
        }
        session.last_seen = Instant::now();
        Ok(tokens.access_token)
    }

    async fn post_token(&self, form: &[(&str, String)]) -> Result<TokenResponse, AuthError> {
        let response = self
            .http
            .post(&self.config.token_endpoint)
            .form(form)
            .send()
            .await
            .map_err(|e| AuthError::InternalError(format!("token endpoint unreachable: {e}")))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error_description")
                        .or_else(|| v.get("error"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(AuthError::ValidationError(format!(
                "token endpoint answered {status}: {detail}"
            )));
        }
        serde_json::from_str(&body).map_err(|e| {
            AuthError::ValidationError(format!("token endpoint sent an unreadable response: {e}"))
        })
    }

    fn touch(&self, session_id: &str) {
        if let Some(session) = write(&self.sessions).get_mut(session_id) {
            session.last_seen = Instant::now();
        }
    }

    fn remove(&self, session_id: &str) -> Option<Session> {
        write(&self.sessions).remove(session_id)
    }

    fn sweep_pending(&self) {
        write(&self.pending).retain(|_, p| p.started_at.elapsed() <= PENDING_TTL);
    }
}

impl std::fmt::Debug for SessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStore")
            .field("client_id", &self.config.client_id)
            .field("sessions", &self.len())
            .finish()
    }
}

/// Fetches the IdP's OpenID Connect discovery document and fills in the
/// endpoints a [`LoginConfig`] is missing. Endpoints already set (from
/// explicit configuration) win; the document only supplies the rest.
pub async fn discover_endpoints(
    issuer: &str,
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    end_session_endpoint: Option<String>,
) -> Result<(String, String, Option<String>), AuthError> {
    if let (Some(auth), Some(token)) = (&authorization_endpoint, &token_endpoint) {
        return Ok((auth.clone(), token.clone(), end_session_endpoint));
    }
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let doc: Value = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| AuthError::InternalError(format!("OIDC discovery at {url} failed: {e}")))?
        .error_for_status()
        .map_err(|e| AuthError::InternalError(format!("OIDC discovery at {url} failed: {e}")))?
        .json()
        .await
        .map_err(|e| {
            AuthError::InternalError(format!("OIDC discovery at {url} unreadable: {e}"))
        })?;
    let pick = |explicit: Option<String>, key: &str| -> Option<String> {
        explicit.or_else(|| doc.get(key).and_then(Value::as_str).map(str::to_string))
    };
    let auth = pick(authorization_endpoint, "authorization_endpoint")
        .ok_or_else(|| AuthError::InternalError(format!("{url} has no authorization_endpoint")))?;
    let token = pick(token_endpoint, "token_endpoint")
        .ok_or_else(|| AuthError::InternalError(format!("{url} has no token_endpoint")))?;
    let end_session = pick(end_session_endpoint, "end_session_endpoint");
    Ok((auth, token, end_session))
}

/// The value of `cookie_name` in a request's `Cookie` header(s), if present.
pub fn cookie_value(headers: &http::HeaderMap, cookie_name: &str) -> Option<String> {
    headers
        .get_all(http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let pair = pair.trim();
            let value = pair.strip_prefix(cookie_name)?.strip_prefix('=')?;
            Some(value.to_string())
        })
}

/// Whether the browser says this request is cross-site. Browsers send
/// `Sec-Fetch-Site` on every request; a value of `cross-site` means another
/// origin initiated it, in which case a session cookie must not be turned into
/// a bearer even if the browser attached it (`SameSite=Lax` already withholds
/// it on cross-site sub-requests; this is defense in depth).
pub fn is_cross_site(headers: &http::HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("cross-site"))
}

/// A URL-safe random token of `bytes` random bytes.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// PKCE `S256` challenge for `verifier`: base64url(SHA-256(verifier)), no
/// padding (RFC 7636 §4.2).
pub fn code_challenge_s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn build_authorize_url(
    endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("scope", scopes)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}{}", query.finish())
}

/// Only a same-site UI path may be a post-login destination — never an
/// absolute URL, so a crafted login link cannot bounce the user elsewhere.
fn is_safe_next(next: &str) -> bool {
    next.starts_with("/ui") && !next.starts_with("//") && !next.contains("://")
}

/// The claims of a JWT, **without verifying its signature**. Only used on
/// tokens received directly from the token endpoint over TLS in a response to
/// our own request, where the transport is what authenticates them; a token
/// presented by a client is never read this way.
fn unverified_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn principal_from_claims(claims: &Value) -> Result<SessionPrincipal, AuthError> {
    let string = |key: &str| claims.get(key).and_then(Value::as_str).map(str::to_string);
    let subject = string("sub")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AuthError::ValidationError("token has no sub".to_string()))?;
    Ok(SessionPrincipal {
        subject,
        issuer: string("iss").unwrap_or_default(),
        name: string("name"),
        preferred_username: string("preferred_username"),
        email: string("email"),
        picture: string("picture"),
    })
}

fn expiry(expires_in: Option<u64>) -> Instant {
    Instant::now() + Duration::from_secs(expires_in.unwrap_or(300))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LoginConfig {
        LoginConfig {
            client_id: "hfs-web".to_string(),
            client_secret: None,
            redirect_uri: "http://localhost:8080/ui/callback".to_string(),
            scopes: "openid profile email".to_string(),
            authorization_endpoint: "https://idp.example.com/auth".to_string(),
            token_endpoint: "https://idp.example.com/token".to_string(),
            end_session_endpoint: Some("https://idp.example.com/logout".to_string()),
            cookie_secure: true,
        }
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636_vector() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge_s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn begin_mints_state_and_challenge_into_the_authorize_url() {
        let store = SessionStore::new(config());
        let (pending_id, url) = store.begin("/ui/resources");
        assert!(!pending_id.is_empty());
        assert!(url.starts_with("https://idp.example.com/auth?"));
        assert!(url.contains("client_id=hfs-web"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fui%2Fcallback"));
        assert!(url.contains("state="));
        assert!(url.contains("code_challenge="));
    }

    #[tokio::test]
    async fn callback_with_unknown_pending_id_is_rejected() {
        let store = SessionStore::new(config());
        let err = store.complete("nope", "s", "c").await.unwrap_err();
        assert!(err.to_string().contains("not pending"), "{err}");
    }

    #[tokio::test]
    async fn callback_with_wrong_state_is_rejected_and_consumes_the_pending_login() {
        let store = SessionStore::new(config());
        let (pending_id, _) = store.begin("/ui");
        let err = store
            .complete(&pending_id, "not-the-state", "code")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("state mismatch"), "{err}");
        // Consumed: a second attempt no longer finds it (no replay).
        let err = store.complete(&pending_id, "x", "code").await.unwrap_err();
        assert!(err.to_string().contains("not pending"), "{err}");
    }

    #[test]
    fn unsafe_next_paths_fall_back_to_the_home_page() {
        assert!(is_safe_next("/ui"));
        assert!(is_safe_next("/ui/resources?type=Patient"));
        assert!(!is_safe_next("//evil.example.com"));
        assert!(!is_safe_next("https://evil.example.com/ui"));
        assert!(!is_safe_next("/Patient"));
    }

    #[test]
    fn cookie_value_finds_the_named_cookie_among_others() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::COOKIE,
            "hfs_lang=es; hfs_session=abc123; other=x".parse().unwrap(),
        );
        assert_eq!(
            cookie_value(&headers, SESSION_COOKIE).as_deref(),
            Some("abc123")
        );
        assert_eq!(cookie_value(&headers, PENDING_COOKIE), None);
    }

    #[test]
    fn cross_site_is_detected_from_sec_fetch_site() {
        let mut headers = http::HeaderMap::new();
        assert!(!is_cross_site(&headers));
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert!(!is_cross_site(&headers));
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(is_cross_site(&headers));
    }

    #[test]
    fn unknown_session_yields_nothing_and_logout_reports_the_end_session_endpoint() {
        let store = SessionStore::new(config());
        assert!(store.get("missing").is_none());
        let (endpoint, hint) = store.logout("missing").expect("end-session configured");
        assert_eq!(endpoint, "https://idp.example.com/logout");
        assert!(hint.is_none());
    }

    #[test]
    fn principal_display_prefers_name_then_username_then_email_then_subject() {
        let mut p = SessionPrincipal {
            subject: "sub-1".to_string(),
            issuer: "iss".to_string(),
            name: None,
            preferred_username: None,
            email: None,
            picture: None,
        };
        assert_eq!(p.display(), "sub-1");
        p.email = Some("d@example.org".to_string());
        assert_eq!(p.display(), "d@example.org");
        p.preferred_username = Some("demo".to_string());
        assert_eq!(p.display(), "demo");
        p.name = Some("Demo User".to_string());
        assert_eq!(p.display(), "Demo User");
    }
}
