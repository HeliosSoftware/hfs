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
//! The store keeps sessions in process and, once a [`SessionPersistence`] is
//! attached, also in the primary store: a session established on one node is
//! resolved on any other (read-through) and survives a restart, while the
//! in-process map stays the fast path for the node that already knows the
//! session. With nothing attached the store is purely in-memory.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SecondsFormat, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
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
pub const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// How long an idle session lives with no refresh token to renew it.
pub const SESSION_IDLE_TTL: Duration = Duration::from_secs(8 * 60 * 60);

/// Renew an access token this long before it actually expires, so a request
/// never goes out with a token about to lapse mid-flight.
const REFRESH_SKEW: Duration = Duration::from_secs(30);

/// How often at most a session's `last_seen` is written through to the
/// attached persistence. Every request touches the in-process copy; the
/// store only needs to know within this window — so a stored session's
/// `last_seen` lags the real one by at most this much.
pub const TOUCH_WRITE_INTERVAL: Duration = Duration::from_secs(60);

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether the attached persistence holds it too. When it does, the
    /// store's row decides whether the login is still pending; when the
    /// write failed, this node's copy is all there is.
    persisted: bool,
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

/// A [`Session`] as it is kept outside the process: wall-clock instants in
/// place of [`Instant`], so another node, or this one after a restart, can
/// read it back. The tokens are stored as they are — they never leave the
/// server, and the IdP's own lifetimes bound them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSession {
    /// The session id — the cookie value.
    pub id: String,
    /// Who is logged in.
    pub principal: SessionPrincipal,
    /// The bearer used for FHIR calls on this user's behalf.
    pub access_token: String,
    /// When `access_token` expires.
    pub access_expires_at: DateTime<Utc>,
    /// The refresh token, when the IdP issued one.
    pub refresh_token: Option<String>,
    /// The ID token, kept for RP-initiated logout.
    pub id_token: Option<String>,
    /// Last time the session was used; the idle expiry counts from here.
    pub last_seen: DateTime<Utc>,
    /// When the session was established.
    pub created_at: DateTime<Utc>,
    /// The row's version at the store, for conditional writes. `0` for a
    /// session that has not been written yet. Not part of the document: the
    /// store keeps it beside the row.
    #[serde(skip)]
    pub version: i64,
}

impl PersistedSession {
    /// Snapshots `session` for the store, tagged with the row version the
    /// caller last saw.
    pub fn from_session(session: &Session, version: i64) -> Self {
        Self {
            id: session.id.clone(),
            principal: session.principal.clone(),
            access_token: session.access_token.clone(),
            access_expires_at: wall_clock(session.access_expires_at),
            refresh_token: session.refresh_token.clone(),
            id_token: session.id_token.clone(),
            last_seen: wall_clock(session.last_seen),
            created_at: session.created_at,
            version,
        }
    }

    /// Rebuilds the in-process [`Session`].
    pub fn into_session(self) -> Session {
        Session {
            id: self.id,
            principal: self.principal,
            access_token: self.access_token,
            access_expires_at: monotonic(self.access_expires_at),
            refresh_token: self.refresh_token,
            id_token: self.id_token,
            last_seen: monotonic(self.last_seen),
            created_at: self.created_at,
        }
    }

    /// When the store may discard this row: the idle expiry, counted from
    /// `last_seen`. The sweep key.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.last_seen + chrono_duration(SESSION_IDLE_TTL)
    }
}

/// A pending login as it is kept outside the process, so the IdP's callback
/// may land on a node other than the one that started the login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPending {
    /// The pending id — the [`PENDING_COOKIE`] value.
    pub id: String,
    /// CSRF `state` the IdP must echo back.
    pub state: String,
    /// PKCE verifier whose S256 challenge went out with the authorize request.
    pub code_verifier: String,
    /// Where to land after login.
    pub next: String,
    /// When the browser was sent to the IdP.
    pub started_at: DateTime<Utc>,
}

impl PersistedPending {
    /// When the store may discard this row. The sweep key.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.started_at + chrono_duration(PENDING_TTL)
    }
}

/// What a conditional [`SessionPersistence::save_session`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Written; the row is now at this version.
    Saved(i64),
    /// Not written: the row was not at the expected version. Another node
    /// changed it first; the caller re-reads and adopts that change.
    Conflict {
        /// The version the row is actually at (`0` when it does not exist).
        current: i64,
    },
}

/// Where a [`SessionStore`] keeps sessions and pending logins beyond the
/// process — a table in the primary store. Implemented per storage backend;
/// attached to the store with [`SessionStore::attach_persistence`].
///
/// A row is one JSON document plus a monotonic `version`, keyed by the opaque
/// session or pending id, with an `expires_at` for the sweep. Sessions and
/// pending logins never share an id (both are 32 random bytes), but an
/// implementation keeps them apart so a pending id is never read as a session.
#[async_trait]
pub trait SessionPersistence: Send + Sync {
    /// The session stored under `id`, with its current `version`.
    async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError>;

    /// Writes `session`. With `if_version = Some(v)` the write happens only
    /// when the row is at `v` (`0`: does not exist yet) and otherwise reports
    /// [`SaveOutcome::Conflict`]; `None` writes unconditionally.
    async fn save_session(
        &self,
        session: &PersistedSession,
        if_version: Option<i64>,
    ) -> Result<SaveOutcome, AuthError>;

    /// Removes the session under `id`; a missing row is not an error.
    async fn delete_session(&self, id: &str) -> Result<(), AuthError>;

    /// The pending login stored under `id`.
    async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError>;

    /// Writes `pending`.
    async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError>;

    /// Removes the pending login under `id`, reporting whether a row was
    /// there. That answer is what makes a login consumable exactly once
    /// across nodes: only the callback whose delete removed the row proceeds.
    async fn delete_pending(&self, id: &str) -> Result<bool, AuthError>;

    /// Drops every row whose `expires_at` is before `now`. Returns how many.
    async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError>;
}

/// A session as this node holds it: the session itself, the store version it
/// was last seen at, and when its `last_seen` was last written through.
#[derive(Debug, Clone)]
struct Held {
    session: Session,
    version: i64,
    last_written: Instant,
}

/// What a conditional write-through found at the store.
enum WriteThrough {
    /// This node's copy is the store's now (or there is no store to agree with).
    Kept,
    /// Another node wrote first; its copy is now held here.
    Superseded(Session),
    /// The row is gone — logged out elsewhere. This node's copy is dropped too.
    Gone,
}

/// Server-side store of pending logins and established sessions.
pub struct SessionStore {
    config: LoginConfig,
    http: reqwest::Client,
    pending: RwLock<HashMap<String, PendingLogin>>,
    sessions: RwLock<HashMap<String, Held>>,
    persistence: RwLock<Option<Arc<dyn SessionPersistence>>>,
}

impl SessionStore {
    /// Creates an empty, in-process store for `config`.
    pub fn new(config: LoginConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            pending: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            persistence: RwLock::new(None),
        }
    }

    /// The client configuration this store drives.
    pub fn config(&self) -> &LoginConfig {
        &self.config
    }

    /// Backs the store with `persistence` from now on. Sessions and pending
    /// logins are written through to it and read through from it, so a
    /// session established on another node — or before a restart — resolves
    /// here. Attached late because the store exists before any storage
    /// backend does; every handle to the store sees the attachment.
    pub fn attach_persistence(&self, persistence: Arc<dyn SessionPersistence>) {
        *write(&self.persistence) = Some(persistence);
        tracing::info!(
            "web login sessions are store-backed: shared across nodes, kept over restarts"
        );
    }

    /// Whether a persistence is attached.
    pub fn is_persistent(&self) -> bool {
        read(&self.persistence).is_some()
    }

    fn persistence(&self) -> Option<Arc<dyn SessionPersistence>> {
        read(&self.persistence).clone()
    }

    /// Starts a login: mints `state` + a PKCE verifier, remembers them under a
    /// fresh pending id, and returns `(pending_id, authorize_url)`. The caller
    /// sets `pending_id` as the [`PENDING_COOKIE`] and redirects the browser
    /// to `authorize_url`.
    pub async fn begin(&self, next: &str) -> (String, String) {
        self.sweep().await;
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
        let mut pending = PendingLogin {
            state,
            code_verifier,
            next,
            started_at: Instant::now(),
            persisted: false,
        };
        if let Some(persistence) = self.persistence() {
            let persisted = PersistedPending {
                id: pending_id.clone(),
                state: pending.state.clone(),
                code_verifier: pending.code_verifier.clone(),
                next: pending.next.clone(),
                started_at: wall_clock(pending.started_at),
            };
            match persistence.save_pending(&persisted).await {
                Ok(()) => pending.persisted = true,
                Err(err) => tracing::warn!(
                    error = %err,
                    "could not persist the pending login; the callback must reach this node"
                ),
            }
        }
        write(&self.pending).insert(pending_id.clone(), pending);
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
        let pending = self.take_pending(pending_id).await?;
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
        let version = self.persist_new(&session).await;
        write(&self.sessions).insert(
            session.id.clone(),
            Held {
                session: session.clone(),
                version,
                last_written: Instant::now(),
            },
        );
        Ok((session, pending.next))
    }

    /// Adds an already-established session — for an embedder that obtains
    /// tokens by other means, and for tests that need a session without an
    /// IdP. The normal path is [`Self::complete`]. Held in process; an
    /// attached persistence learns of it on the first write-through.
    pub fn insert(&self, session: Session) {
        write(&self.sessions).insert(
            session.id.clone(),
            Held {
                session,
                version: 0,
                last_written: Instant::now(),
            },
        );
    }

    /// Looks up the session this node holds for a cookie value, without
    /// touching the IdP or the store. [`Self::access_token`] is what resolves
    /// a session this node has not seen yet.
    pub fn get(&self, session_id: &str) -> Option<Session> {
        let sessions = read(&self.sessions);
        let held = sessions.get(session_id)?;
        if held.session.last_seen.elapsed() > SESSION_IDLE_TTL {
            return None;
        }
        Some(held.session.clone())
    }

    /// Resolves a session cookie to a bearer for the current request,
    /// refreshing the access token when it is about to expire. An expired
    /// session with no working refresh token is dropped and reported as
    /// [`AccessOutcome::NoSession`], so the caller falls back to "not logged
    /// in" rather than forwarding a dead token. A session this node does not
    /// hold is looked up in the attached persistence and kept from then on.
    pub async fn access_token(&self, session_id: &str) -> AccessOutcome {
        let Some(session) = self.resolve(session_id).await else {
            return AccessOutcome::NoSession;
        };
        if is_fresh(&session) {
            self.touch(session_id).await;
            return AccessOutcome::Token(session.access_token);
        }
        match self
            .refresh(session_id, session.refresh_token.as_deref())
            .await
        {
            Ok(token) => AccessOutcome::Token(token),
            Err(err) => {
                tracing::info!(error = %err, "session refresh failed; dropping session");
                self.remove(session_id).await;
                AccessOutcome::NoSession
            }
        }
    }

    /// Ends a session. Returns what was needed for RP-initiated logout at the
    /// IdP (the end-session endpoint and the ID token hint), when configured.
    pub async fn logout(&self, session_id: &str) -> Option<(String, Option<String>)> {
        let removed = self.remove(session_id).await;
        let endpoint = self.config.end_session_endpoint.clone()?;
        Some((endpoint, removed.and_then(|s| s.id_token)))
    }

    /// Number of sessions this node holds — for tests and diagnostics.
    pub fn len(&self) -> usize {
        read(&self.sessions).len()
    }

    /// Whether this node holds no sessions.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The session for `session_id`: this node's copy, else the store's,
    /// which is then held here too.
    async fn resolve(&self, session_id: &str) -> Option<Session> {
        if let Some(session) = self.get(session_id) {
            return Some(session);
        }
        let persistence = self.persistence()?;
        let persisted = match persistence.load_session(session_id).await {
            Ok(persisted) => persisted?,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the session store; treating the cookie as unknown");
                return None;
            }
        };
        if persisted.expires_at() <= Utc::now() {
            return None;
        }
        Some(self.adopt(persisted))
    }

    /// Takes the pending login for `pending_id` out of wherever it is —
    /// this node's map, else the store — and consumes it. With a persistence
    /// attached, the store's row is the arbiter of "exactly once": the
    /// callback whose delete removed the row is the one that proceeds.
    async fn take_pending(&self, pending_id: &str) -> Result<PendingLogin, AuthError> {
        let not_pending = || AuthError::ValidationError("login is not pending".to_string());
        let local = write(&self.pending).remove(pending_id);
        let Some(persistence) = self.persistence() else {
            return local.ok_or_else(not_pending);
        };
        if let Some(local) = local.as_ref().filter(|local| !local.persisted) {
            return Ok(local.clone());
        }
        let remote = match persistence.load_pending(pending_id).await {
            Ok(remote) => remote,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the pending login from the store");
                None
            }
        };
        match persistence.delete_pending(pending_id).await {
            Ok(true) => {}
            Ok(false) => {
                if remote.is_none() && local.is_none() {
                    return Err(not_pending());
                }
                return Err(AuthError::ValidationError(
                    "login was already completed".to_string(),
                ));
            }
            Err(err) => {
                tracing::warn!(error = %err, "could not consume the pending login at the store");
                return local.ok_or_else(not_pending);
            }
        }
        Ok(local.unwrap_or_else(|| {
            let remote = remote.expect("a deleted row was loaded first");
            PendingLogin {
                state: remote.state,
                code_verifier: remote.code_verifier,
                next: remote.next,
                started_at: monotonic(remote.started_at),
                persisted: true,
            }
        }))
    }

    /// Holds `persisted` on this node, replacing whatever copy it had.
    fn adopt(&self, persisted: PersistedSession) -> Session {
        let version = persisted.version;
        let session = persisted.into_session();
        write(&self.sessions).insert(
            session.id.clone(),
            Held {
                session: session.clone(),
                version,
                last_written: Instant::now(),
            },
        );
        session
    }

    /// Writes a brand-new session through; returns the version it landed at
    /// (`0` when nothing is attached or the write failed, so a later write
    /// creates the row).
    async fn persist_new(&self, session: &Session) -> i64 {
        let Some(persistence) = self.persistence() else {
            return 0;
        };
        match persistence
            .save_session(&PersistedSession::from_session(session, 0), Some(0))
            .await
        {
            Ok(SaveOutcome::Saved(version)) => version,
            Ok(SaveOutcome::Conflict { current }) => {
                tracing::warn!(current, "a fresh session id already exists in the store");
                current
            }
            Err(err) => {
                tracing::warn!(error = %err, "could not persist the new session; it is held on this node only");
                0
            }
        }
    }

    /// Writes this node's copy of `session_id` through, conditional on the
    /// version it was last seen at. On a conflict another node changed the
    /// row first: a refresh, whose copy is adopted here so this node never
    /// overwrites newer tokens with older ones — or a logout, after which
    /// this node drops its copy too.
    async fn write_through(&self, session_id: &str) -> WriteThrough {
        let Some(persistence) = self.persistence() else {
            return WriteThrough::Kept;
        };
        let persisted = {
            let sessions = read(&self.sessions);
            let Some(held) = sessions.get(session_id) else {
                return WriteThrough::Gone;
            };
            PersistedSession::from_session(&held.session, held.version)
        };
        match persistence
            .save_session(&persisted, Some(persisted.version))
            .await
        {
            Ok(SaveOutcome::Saved(version)) => {
                if let Some(held) = write(&self.sessions).get_mut(session_id) {
                    held.version = version;
                    held.last_written = Instant::now();
                }
                WriteThrough::Kept
            }
            Ok(SaveOutcome::Conflict { .. }) => match persistence.load_session(session_id).await {
                Ok(Some(theirs)) => WriteThrough::Superseded(self.adopt(theirs)),
                Ok(None) => {
                    write(&self.sessions).remove(session_id);
                    WriteThrough::Gone
                }
                Err(err) => {
                    tracing::warn!(error = %err, "could not re-read the session after a write conflict");
                    WriteThrough::Kept
                }
            },
            Err(err) => {
                tracing::warn!(error = %err, "could not write the session through to the store");
                WriteThrough::Kept
            }
        }
    }

    async fn refresh(
        &self,
        session_id: &str,
        refresh_token: Option<&str>,
    ) -> Result<String, AuthError> {
        // Another node may have refreshed this session already; its tokens
        // are in the store. Using them spares the IdP a second refresh with a
        // token it may have rotated away.
        let mut refresh_token = refresh_token.map(str::to_string);
        if let Some(persistence) = self.persistence()
            && let Ok(Some(theirs)) = persistence.load_session(session_id).await
        {
            let theirs = self.adopt(theirs);
            if is_fresh(&theirs) {
                return Ok(theirs.access_token);
            }
            refresh_token = theirs.refresh_token;
        }
        let refresh_token = refresh_token.ok_or(AuthError::TokenExpired)?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", self.config.client_id.clone()),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let tokens = self.post_token(&form).await?;
        {
            let mut sessions = write(&self.sessions);
            let held = sessions
                .get_mut(session_id)
                .ok_or_else(|| AuthError::ValidationError("session vanished".to_string()))?;
            let session = &mut held.session;
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
        }
        // Two nodes refreshing at once: the one whose write lands second takes
        // the winner's tokens rather than handing out its own, now-superseded
        // ones.
        match self.write_through(session_id).await {
            WriteThrough::Kept => Ok(tokens.access_token),
            WriteThrough::Superseded(theirs) => Ok(theirs.access_token),
            WriteThrough::Gone => Err(AuthError::ValidationError(
                "session was ended elsewhere".to_string(),
            )),
        }
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

    /// Marks the session used. The in-process copy is updated on every call;
    /// the store sees it at most once per [`TOUCH_WRITE_INTERVAL`].
    async fn touch(&self, session_id: &str) {
        let due = {
            let mut sessions = write(&self.sessions);
            let Some(held) = sessions.get_mut(session_id) else {
                return;
            };
            held.session.last_seen = Instant::now();
            held.last_written.elapsed() >= TOUCH_WRITE_INTERVAL
        };
        if due && self.is_persistent() {
            self.write_through(session_id).await;
        }
    }

    /// Drops the session here and at the store. Returns what this node held,
    /// else what the store did, so a logout started on another node still
    /// finds the ID token hint.
    async fn remove(&self, session_id: &str) -> Option<Session> {
        let local = write(&self.sessions)
            .remove(session_id)
            .map(|held| held.session);
        let Some(persistence) = self.persistence() else {
            return local;
        };
        let remote = if local.is_none() {
            persistence
                .load_session(session_id)
                .await
                .ok()
                .flatten()
                .map(PersistedSession::into_session)
        } else {
            None
        };
        if let Err(err) = persistence.delete_session(session_id).await {
            tracing::warn!(error = %err, "could not remove the session from the store");
        }
        local.or(remote)
    }

    /// Drops what has expired: pending logins past their window and sessions
    /// idle past theirs, here and at the store.
    async fn sweep(&self) {
        write(&self.pending).retain(|_, p| p.started_at.elapsed() <= PENDING_TTL);
        write(&self.sessions)
            .retain(|_, held| held.session.last_seen.elapsed() <= SESSION_IDLE_TTL);
        if let Some(persistence) = self.persistence()
            && let Err(err) = persistence.sweep(Utc::now()).await
        {
            tracing::warn!(error = %err, "could not sweep expired logins from the store");
        }
    }
}

fn is_fresh(session: &Session) -> bool {
    session
        .access_expires_at
        .saturating_duration_since(Instant::now())
        > REFRESH_SKEW
}

/// The wall-clock time `instant` corresponds to, for storing.
fn wall_clock(instant: Instant) -> DateTime<Utc> {
    let now = Instant::now();
    if instant >= now {
        Utc::now() + chrono_duration(instant - now)
    } else {
        Utc::now() - chrono_duration(now - instant)
    }
}

/// The monotonic instant `at` corresponds to, for a stored time read back.
fn monotonic(at: DateTime<Utc>) -> Instant {
    let now = Utc::now();
    let now_instant = Instant::now();
    if at >= now {
        now_instant + (at - now).to_std().unwrap_or_default()
    } else {
        now_instant
            .checked_sub((now - at).to_std().unwrap_or_default())
            .unwrap_or(now_instant)
    }
}

fn chrono_duration(duration: Duration) -> chrono::TimeDelta {
    chrono::TimeDelta::from_std(duration).unwrap_or(chrono::TimeDelta::MAX)
}

/// A store's fixed-width RFC 3339 rendering of `at` (millisecond precision,
/// `Z`), so rows sort chronologically as text.
pub fn store_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
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
/// explicit configuration) win; the document only supplies the rest — the
/// document is consulted whenever *any* of the three is unset, so an explicit
/// authorize + token pair (the usual `HFS_SMART_*` setup) still gets the
/// end-session endpoint that RP-initiated logout needs. Without it a logout
/// only clears HFS's cookie and the IdP's own SSO session signs the user
/// straight back in on the next page.
///
/// When the document cannot be fetched but authorize + token are explicit,
/// login is still possible, so that degrades to a warning with no end-session
/// endpoint rather than a startup failure; with nothing explicit there is
/// nothing to log in with, and it is an error.
pub async fn discover_endpoints(
    issuer: &str,
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    end_session_endpoint: Option<String>,
) -> Result<(String, String, Option<String>), AuthError> {
    if let (Some(auth), Some(token), Some(end)) = (
        &authorization_endpoint,
        &token_endpoint,
        &end_session_endpoint,
    ) {
        return Ok((auth.clone(), token.clone(), Some(end.clone())));
    }
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let doc = match fetch_discovery(&url).await {
        Ok(doc) => doc,
        Err(err) => {
            if let (Some(auth), Some(token)) = (&authorization_endpoint, &token_endpoint) {
                tracing::warn!(
                    error = %err,
                    "OIDC discovery failed; logging in with the configured endpoints, but the \
                     end-session endpoint is unknown — Sign out will not end the IdP session. \
                     Set HFS_SMART_END_SESSION_ENDPOINT."
                );
                return Ok((auth.clone(), token.clone(), None));
            }
            return Err(err);
        }
    };
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

async fn fetch_discovery(url: &str) -> Result<Value, AuthError> {
    reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| AuthError::InternalError(format!("OIDC discovery at {url} failed: {e}")))?
        .error_for_status()
        .map_err(|e| AuthError::InternalError(format!("OIDC discovery at {url} failed: {e}")))?
        .json()
        .await
        .map_err(|e| AuthError::InternalError(format!("OIDC discovery at {url} unreadable: {e}")))
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

    #[tokio::test]
    async fn begin_mints_state_and_challenge_into_the_authorize_url() {
        let store = SessionStore::new(config());
        let (pending_id, url) = store.begin("/ui/resources").await;
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
        let (pending_id, _) = store.begin("/ui").await;
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

    #[tokio::test]
    async fn unknown_session_yields_nothing_and_logout_reports_the_end_session_endpoint() {
        let store = SessionStore::new(config());
        assert!(store.get("missing").is_none());
        let (endpoint, hint) = store
            .logout("missing")
            .await
            .expect("end-session configured");
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

/// Tests that need a token endpoint: the code exchange, refresh, and OpenID
/// discovery are exercised against a local mock server.
#[cfg(test)]
mod token_endpoint_tests {
    use std::time::{Duration, Instant};

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn config_for(server: &MockServer) -> LoginConfig {
        LoginConfig {
            client_id: "hfs-web".to_string(),
            client_secret: None,
            redirect_uri: "http://localhost:8080/ui/callback".to_string(),
            scopes: "openid profile email".to_string(),
            authorization_endpoint: format!("{}/auth", server.uri()),
            token_endpoint: format!("{}/token", server.uri()),
            end_session_endpoint: None,
            cookie_secure: true,
        }
    }

    /// An unsigned JWT whose payload carries `claims` — enough for the store,
    /// which reads a token it received from the token endpoint unverified.
    fn jwt_with(claims: serde_json::Value) -> String {
        format!(
            "eyJhbGciOiJub25lIn0.{}.sig",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    fn seeded(id: &str) -> Session {
        Session {
            id: id.to_string(),
            principal: SessionPrincipal {
                subject: "demo-sub".to_string(),
                issuer: "https://idp".to_string(),
                name: None,
                preferred_username: None,
                email: None,
                picture: None,
            },
            access_token: "at-old".to_string(),
            access_expires_at: Instant::now() + Duration::from_secs(300),
            refresh_token: Some("rt-1".to_string()),
            id_token: None,
            last_seen: Instant::now(),
            created_at: Utc::now(),
        }
    }

    fn state_of(authorize_url: &str) -> String {
        authorize_url
            .split("state=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .expect("state in the authorize url")
            .to_string()
    }

    #[tokio::test]
    async fn complete_exchanges_the_code_with_pkce_and_establishes_the_session() {
        let server = MockServer::start().await;
        let id_token = jwt_with(json!({
            "sub": "demo-sub", "iss": "https://idp", "name": "Demo User",
            "preferred_username": "demo", "email": "demo@example.org"
        }));
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains("code_verifier="))
            .and(body_string_contains("client_id=hfs-web"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-1", "refresh_token": "rt-1", "id_token": id_token,
                "expires_in": 300, "token_type": "Bearer"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = SessionStore::new(config_for(&server));
        let (pending_id, authorize_url) = store.begin("/ui/resources").await;
        let state = state_of(&authorize_url);

        let (session, next) = store
            .complete(&pending_id, &state, "the-code")
            .await
            .expect("code exchange succeeds");
        assert_eq!(next, "/ui/resources");
        assert_eq!(session.access_token, "at-1");
        assert_eq!(session.refresh_token.as_deref(), Some("rt-1"));
        assert_eq!(session.principal.subject, "demo-sub");
        assert_eq!(session.principal.display(), "Demo User");
        assert_eq!(session.principal.email.as_deref(), Some("demo@example.org"));
        assert!(store.get(&session.id).is_some(), "the session is stored");
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn a_rejected_exchange_surfaces_the_idp_error_and_leaves_no_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "Code not valid"
            })))
            .mount(&server)
            .await;
        let store = SessionStore::new(config_for(&server));
        let (pending_id, authorize_url) = store.begin("/ui").await;
        let state = state_of(&authorize_url);
        let err = store
            .complete(&pending_id, &state, "used")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Code not valid"), "{err}");
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn an_access_token_about_to_expire_is_refreshed_in_place() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=rt-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2", "refresh_token": "rt-2", "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;
        let store = SessionStore::new(config_for(&server));
        let mut session = seeded("s1");
        session.access_expires_at = Instant::now();
        store.insert(session);

        match store.access_token("s1").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-2"),
            other => panic!("expected a refreshed token, got {other:?}"),
        }
        let stored = store.get("s1").expect("still stored");
        assert_eq!(stored.access_token, "at-2");
        assert_eq!(
            stored.refresh_token.as_deref(),
            Some("rt-2"),
            "rotated token kept"
        );
    }

    #[tokio::test]
    async fn a_fresh_access_token_is_returned_without_touching_the_idp() {
        let server = MockServer::start().await;
        let store = SessionStore::new(config_for(&server));
        store.insert(seeded("fresh"));
        match store.access_token("fresh").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-old"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dead_refresh_drops_the_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "Session not active"
            })))
            .mount(&server)
            .await;
        let store = SessionStore::new(config_for(&server));
        let mut session = seeded("dead");
        session.access_expires_at = Instant::now();
        store.insert(session);

        assert!(matches!(
            store.access_token("dead").await,
            AccessOutcome::NoSession
        ));
        assert!(store.get("dead").is_none(), "dropped");

        let mut none = seeded("norefresh");
        none.access_expires_at = Instant::now();
        none.refresh_token = None;
        store.insert(none);
        assert!(matches!(
            store.access_token("norefresh").await,
            AccessOutcome::NoSession
        ));
        assert!(store.get("norefresh").is_none());
    }

    #[tokio::test]
    async fn discovery_fills_only_what_explicit_configuration_left_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "authorization_endpoint": format!("{}/discovered-auth", server.uri()),
                "token_endpoint": format!("{}/discovered-token", server.uri()),
                "end_session_endpoint": format!("{}/discovered-logout", server.uri()),
            })))
            .mount(&server)
            .await;

        let (auth, token, end) = discover_endpoints(
            &server.uri(),
            Some("https://explicit/auth".to_string()),
            Some("https://explicit/token".to_string()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(auth, "https://explicit/auth");
        assert_eq!(token, "https://explicit/token");
        assert_eq!(
            end.as_deref(),
            Some(format!("{}/discovered-logout", server.uri()).as_str())
        );

        let (auth, token, end) = discover_endpoints(&server.uri(), None, None, None)
            .await
            .unwrap();
        assert!(auth.ends_with("/discovered-auth"));
        assert!(token.ends_with("/discovered-token"));
        assert!(end.unwrap().ends_with("/discovered-logout"));
    }

    #[tokio::test]
    async fn everything_explicit_skips_discovery_entirely() {
        let server = MockServer::start().await;
        let (auth, token, end) = discover_endpoints(
            &server.uri(),
            Some("https://explicit/auth".to_string()),
            Some("https://explicit/token".to_string()),
            Some("https://explicit/logout".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(auth, "https://explicit/auth");
        assert_eq!(token, "https://explicit/token");
        assert_eq!(end.as_deref(), Some("https://explicit/logout"));
    }

    #[tokio::test]
    async fn unreachable_discovery_degrades_to_the_explicit_endpoints_without_end_session() {
        let server = MockServer::start().await;
        let (auth, token, end) = discover_endpoints(
            &server.uri(),
            Some("https://explicit/auth".to_string()),
            Some("https://explicit/token".to_string()),
            None,
        )
        .await
        .expect("login still configurable from the explicit endpoints");
        assert_eq!(auth, "https://explicit/auth");
        assert_eq!(token, "https://explicit/token");
        assert!(end.is_none(), "end-session stays unknown");

        assert!(
            discover_endpoints(&server.uri(), None, None, None)
                .await
                .is_err()
        );
    }
}

/// Tests with a [`SessionPersistence`] attached: what another node, or this
/// one after a restart, sees. The persistence is an in-memory table with the
/// same versioning a real backend has; `Racing` interposes a second node's
/// write between a read and a conditional write.
#[cfg(test)]
mod persistence_tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[derive(Default)]
    struct Memory {
        sessions: Mutex<HashMap<String, PersistedSession>>,
        pending: Mutex<HashMap<String, PersistedPending>>,
        saves: AtomicUsize,
    }

    impl Memory {
        fn lock<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
            lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    #[async_trait]
    impl SessionPersistence for Memory {
        async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError> {
            Ok(Self::lock(&self.sessions).get(id).cloned())
        }

        async fn save_session(
            &self,
            session: &PersistedSession,
            if_version: Option<i64>,
        ) -> Result<SaveOutcome, AuthError> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            let mut sessions = Self::lock(&self.sessions);
            let current = sessions.get(&session.id).map(|s| s.version).unwrap_or(0);
            if let Some(expected) = if_version
                && expected != current
            {
                return Ok(SaveOutcome::Conflict { current });
            }
            let mut stored = session.clone();
            stored.version = current + 1;
            sessions.insert(session.id.clone(), stored);
            Ok(SaveOutcome::Saved(current + 1))
        }

        async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
            Self::lock(&self.sessions).remove(id);
            Ok(())
        }

        async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError> {
            Ok(Self::lock(&self.pending).get(id).cloned())
        }

        async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError> {
            Self::lock(&self.pending).insert(pending.id.clone(), pending.clone());
            Ok(())
        }

        async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
            Ok(Self::lock(&self.pending).remove(id).is_some())
        }

        async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
            let mut removed = 0;
            Self::lock(&self.sessions).retain(|_, s| {
                let keep = s.expires_at() >= now;
                removed += u64::from(!keep);
                keep
            });
            Self::lock(&self.pending).retain(|_, p| {
                let keep = p.expires_at() >= now;
                removed += u64::from(!keep);
                keep
            });
            Ok(removed)
        }
    }

    /// Before the first conditional session write, another node writes the
    /// same session with `at-theirs`, so that write conflicts.
    struct Racing {
        inner: Arc<Memory>,
        armed: AtomicBool,
    }

    #[async_trait]
    impl SessionPersistence for Racing {
        async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError> {
            self.inner.load_session(id).await
        }

        async fn save_session(
            &self,
            session: &PersistedSession,
            if_version: Option<i64>,
        ) -> Result<SaveOutcome, AuthError> {
            if self.armed.swap(false, Ordering::SeqCst) {
                let mut theirs = session.clone();
                theirs.access_token = "at-theirs".to_string();
                theirs.refresh_token = Some("rt-theirs".to_string());
                theirs.access_expires_at = Utc::now() + chrono::TimeDelta::minutes(5);
                self.inner.save_session(&theirs, None).await?;
            }
            self.inner.save_session(session, if_version).await
        }

        async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
            self.inner.delete_session(id).await
        }

        async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError> {
            self.inner.load_pending(id).await
        }

        async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError> {
            self.inner.save_pending(pending).await
        }

        async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
            self.inner.delete_pending(id).await
        }

        async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
            self.inner.sweep(now).await
        }
    }

    /// A store that is down.
    struct Down;

    #[async_trait]
    impl SessionPersistence for Down {
        async fn load_session(&self, _: &str) -> Result<Option<PersistedSession>, AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn save_session(
            &self,
            _: &PersistedSession,
            _: Option<i64>,
        ) -> Result<SaveOutcome, AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn delete_session(&self, _: &str) -> Result<(), AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn load_pending(&self, _: &str) -> Result<Option<PersistedPending>, AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn save_pending(&self, _: &PersistedPending) -> Result<(), AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn delete_pending(&self, _: &str) -> Result<bool, AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }

        async fn sweep(&self, _: DateTime<Utc>) -> Result<u64, AuthError> {
            Err(AuthError::InternalError("down".to_string()))
        }
    }

    fn config_for(server: &MockServer) -> LoginConfig {
        LoginConfig {
            client_id: "hfs-web".to_string(),
            client_secret: None,
            redirect_uri: "http://localhost:8080/ui/callback".to_string(),
            scopes: "openid profile email".to_string(),
            authorization_endpoint: format!("{}/auth", server.uri()),
            token_endpoint: format!("{}/token", server.uri()),
            end_session_endpoint: Some(format!("{}/logout", server.uri())),
            cookie_secure: true,
        }
    }

    /// A store on one node, sharing `persistence` with its siblings.
    fn node(server: &MockServer, persistence: Arc<dyn SessionPersistence>) -> SessionStore {
        let store = SessionStore::new(config_for(server));
        store.attach_persistence(persistence);
        store
    }

    fn jwt_with(claims: serde_json::Value) -> String {
        format!(
            "eyJhbGciOiJub25lIn0.{}.sig",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    fn state_of(authorize_url: &str) -> String {
        authorize_url
            .split("state=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .expect("state in the authorize url")
            .to_string()
    }

    fn stored(id: &str, access_token: &str, expires_in: Duration) -> PersistedSession {
        let now = Utc::now();
        PersistedSession {
            id: id.to_string(),
            principal: SessionPrincipal {
                subject: "demo-sub".to_string(),
                issuer: "https://idp".to_string(),
                name: Some("Demo User".to_string()),
                preferred_username: None,
                email: None,
                picture: None,
            },
            access_token: access_token.to_string(),
            access_expires_at: now + chrono_duration(expires_in),
            refresh_token: Some("rt-1".to_string()),
            id_token: Some("id-1".to_string()),
            last_seen: now,
            created_at: now,
            version: 0,
        }
    }

    async fn mount_code_exchange(server: &MockServer, access_token: &str) {
        let id_token =
            jwt_with(json!({"sub": "demo-sub", "iss": "https://idp", "name": "Demo User"}));
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token, "refresh_token": "rt-1", "id_token": id_token,
                "expires_in": 300, "token_type": "Bearer"
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_login_started_on_one_node_completes_on_another_exactly_once() {
        let server = MockServer::start().await;
        mount_code_exchange(&server, "at-1").await;
        let persistence = Arc::new(Memory::default());
        let a = node(&server, persistence.clone());
        let b = node(&server, persistence.clone());

        let (pending_id, authorize_url) = a.begin("/ui/resources").await;
        let state = state_of(&authorize_url);
        assert!(
            persistence
                .load_pending(&pending_id)
                .await
                .unwrap()
                .is_some()
        );

        let (session, next) = b
            .complete(&pending_id, &state, "the-code")
            .await
            .expect("the callback lands on the other node");
        assert_eq!(next, "/ui/resources");
        assert_eq!(session.access_token, "at-1");
        assert!(
            persistence
                .load_pending(&pending_id)
                .await
                .unwrap()
                .is_none()
        );

        // The node that started it still holds a copy, but the store already
        // consumed the login: a replay there is refused too.
        let err = a
            .complete(&pending_id, &state, "the-code")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already completed"), "{err}");
        assert!(a.get(&session.id).is_none(), "not on A yet");
        assert_eq!(
            persistence
                .load_session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .version,
            1
        );
    }

    #[tokio::test]
    async fn a_session_established_elsewhere_is_resolved_here_and_then_held() {
        let server = MockServer::start().await;
        let persistence = Arc::new(Memory::default());
        persistence
            .save_session(&stored("s1", "at-1", Duration::from_secs(300)), None)
            .await
            .unwrap();
        let here = node(&server, persistence.clone());
        assert!(here.is_empty());

        match here.access_token("s1").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-1"),
            other => panic!("{other:?}"),
        }
        let held = here.get("s1").expect("held after the read-through");
        assert_eq!(held.principal.display(), "Demo User");
        assert_eq!(held.id_token.as_deref(), Some("id-1"));

        // Every later request is served from this node's copy: no writes for
        // a fresh token inside the touch window.
        let saves = persistence.saves.load(Ordering::SeqCst);
        for _ in 0..3 {
            assert!(matches!(
                here.access_token("s1").await,
                AccessOutcome::Token(_)
            ));
        }
        assert_eq!(persistence.saves.load(Ordering::SeqCst), saves);

        // A restart: a fresh store over the same persistence still knows it.
        let restarted = node(&server, persistence.clone());
        assert!(matches!(
            restarted.access_token("s1").await,
            AccessOutcome::Token(_)
        ));
    }

    #[tokio::test]
    async fn an_idle_session_in_the_store_is_not_resurrected() {
        let server = MockServer::start().await;
        let persistence = Arc::new(Memory::default());
        let mut idle = stored("idle", "at-1", Duration::from_secs(300));
        idle.last_seen = Utc::now() - chrono::TimeDelta::hours(9);
        persistence.save_session(&idle, None).await.unwrap();
        let here = node(&server, persistence);
        assert!(matches!(
            here.access_token("idle").await,
            AccessOutcome::NoSession
        ));
        assert!(here.get("idle").is_none());
    }

    #[tokio::test]
    async fn a_refresh_done_by_another_node_is_adopted_without_asking_the_idp() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let persistence = Arc::new(Memory::default());
        persistence
            .save_session(&stored("s1", "at-old", Duration::from_secs(5)), None)
            .await
            .unwrap();
        let here = node(&server, persistence.clone());
        // Warm this node with the about-to-expire copy.
        assert!(here.get("s1").is_none());
        let _ = here.resolve("s1").await.expect("read through");

        // The other node refreshes first.
        let mut theirs = stored("s1", "at-theirs", Duration::from_secs(300));
        theirs.version = 1;
        persistence.save_session(&theirs, Some(1)).await.unwrap();

        match here.access_token("s1").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-theirs"),
            other => panic!("{other:?}"),
        }
        assert_eq!(here.get("s1").unwrap().access_token, "at-theirs");
    }

    #[tokio::test]
    async fn a_refresh_that_loses_the_race_hands_out_the_winners_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-mine", "refresh_token": "rt-mine", "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;
        let memory = Arc::new(Memory::default());
        memory
            .save_session(&stored("s1", "at-old", Duration::from_secs(5)), None)
            .await
            .unwrap();
        let racing = Arc::new(Racing {
            inner: memory.clone(),
            armed: AtomicBool::new(true),
        });
        let here = node(&server, racing);

        match here.access_token("s1").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-theirs"),
            other => panic!("{other:?}"),
        }
        let held = here.get("s1").unwrap();
        assert_eq!(held.access_token, "at-theirs");
        assert_eq!(held.refresh_token.as_deref(), Some("rt-theirs"));
        assert_eq!(
            memory
                .load_session("s1")
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "at-theirs",
            "the loser's tokens never reached the store"
        );
    }

    #[tokio::test]
    async fn a_refresh_that_wins_is_written_through_at_the_next_version() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2", "refresh_token": "rt-2", "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;
        let persistence = Arc::new(Memory::default());
        persistence
            .save_session(&stored("s1", "at-old", Duration::from_secs(5)), None)
            .await
            .unwrap();
        let here = node(&server, persistence.clone());
        match here.access_token("s1").await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-2"),
            other => panic!("{other:?}"),
        }
        let row = persistence.load_session("s1").await.unwrap().unwrap();
        assert_eq!(row.version, 2);
        assert_eq!(row.access_token, "at-2");
        assert_eq!(row.refresh_token.as_deref(), Some("rt-2"));
    }

    #[tokio::test]
    async fn logout_removes_the_session_from_the_store_and_finds_the_id_token_there() {
        let server = MockServer::start().await;
        let persistence = Arc::new(Memory::default());
        persistence
            .save_session(&stored("s1", "at-1", Duration::from_secs(300)), None)
            .await
            .unwrap();
        let here = node(&server, persistence.clone());
        let (endpoint, hint) = here.logout("s1").await.expect("end-session configured");
        assert!(endpoint.ends_with("/logout"));
        assert_eq!(hint.as_deref(), Some("id-1"));
        assert!(persistence.load_session("s1").await.unwrap().is_none());
        let elsewhere = node(&server, persistence);
        assert!(matches!(
            elsewhere.access_token("s1").await,
            AccessOutcome::NoSession
        ));
    }

    #[tokio::test]
    async fn a_logout_elsewhere_ends_the_session_here_at_the_next_write() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2", "expires_in": 300
            })))
            .mount(&server)
            .await;
        let persistence = Arc::new(Memory::default());
        persistence
            .save_session(&stored("s1", "at-old", Duration::from_secs(5)), None)
            .await
            .unwrap();
        let here = node(&server, persistence.clone());
        let _ = here.resolve("s1").await.expect("read through");
        // The user signs out on another node; this node still holds a copy
        // and, its token about to lapse, tries to refresh.
        persistence.delete_session("s1").await.unwrap();
        assert!(matches!(
            here.access_token("s1").await,
            AccessOutcome::NoSession
        ));
        assert!(here.get("s1").is_none());
    }

    #[tokio::test]
    async fn a_pending_login_the_store_never_saw_still_completes_here() {
        let server = MockServer::start().await;
        mount_code_exchange(&server, "at-1").await;
        let memory = Arc::new(Memory::default());
        // The store is down while the login starts…
        let here = SessionStore::new(config_for(&server));
        here.attach_persistence(Arc::new(Down));
        let (pending_id, authorize_url) = here.begin("/ui").await;
        let state = state_of(&authorize_url);
        // …and back when the callback arrives: this node's copy is all there
        // is, and the store's "no such row" must not refuse the login.
        here.attach_persistence(memory.clone());
        let (session, _) = here
            .complete(&pending_id, &state, "the-code")
            .await
            .expect("completes from this node's copy");
        assert_eq!(session.access_token, "at-1");
        assert!(memory.load_session(&session.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_store_that_is_down_leaves_this_node_working_on_its_own() {
        let server = MockServer::start().await;
        mount_code_exchange(&server, "at-1").await;
        let here = node(&server, Arc::new(Down));
        let (pending_id, authorize_url) = here.begin("/ui").await;
        let state = state_of(&authorize_url);
        let (session, _) = here
            .complete(&pending_id, &state, "the-code")
            .await
            .expect("login completes from this node's copy");
        match here.access_token(&session.id).await {
            AccessOutcome::Token(token) => assert_eq!(token, "at-1"),
            other => panic!("{other:?}"),
        }
        here.logout(&session.id).await;
        assert!(here.get(&session.id).is_none());
    }

    #[tokio::test]
    async fn begin_sweeps_what_has_expired_from_the_store() {
        let server = MockServer::start().await;
        let persistence = Arc::new(Memory::default());
        let mut old = PersistedPending {
            id: "old".to_string(),
            state: "s".to_string(),
            code_verifier: "v".to_string(),
            next: "/ui".to_string(),
            started_at: Utc::now(),
        };
        old.started_at -= chrono::TimeDelta::minutes(11);
        persistence.save_pending(&old).await.unwrap();
        let here = node(&server, persistence.clone());
        here.begin("/ui").await;
        assert!(persistence.load_pending("old").await.unwrap().is_none());
    }

    #[test]
    fn a_session_survives_the_trip_through_its_persisted_shape() {
        let session = Session {
            id: "s1".to_string(),
            principal: SessionPrincipal {
                subject: "sub".to_string(),
                issuer: "iss".to_string(),
                name: None,
                preferred_username: Some("demo".to_string()),
                email: None,
                picture: None,
            },
            access_token: "at".to_string(),
            access_expires_at: Instant::now() + Duration::from_secs(300),
            refresh_token: Some("rt".to_string()),
            id_token: None,
            last_seen: Instant::now() - Duration::from_secs(60),
            created_at: Utc::now(),
        };
        let persisted = PersistedSession::from_session(&session, 3);
        let json = serde_json::to_string(&persisted).unwrap();
        assert!(
            !json.contains("\"version\""),
            "the version is the row's, not the document's"
        );
        let back: PersistedSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, 0);
        assert_eq!(back.principal, session.principal);
        let restored = back.into_session();
        let expires_in = restored
            .access_expires_at
            .saturating_duration_since(Instant::now());
        assert!(expires_in > Duration::from_secs(298) && expires_in <= Duration::from_secs(300));
        assert!(restored.last_seen.elapsed() >= Duration::from_secs(59));
        assert_eq!(restored.refresh_token.as_deref(), Some("rt"));
    }
}
