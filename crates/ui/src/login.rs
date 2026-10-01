//! Interactive browser login for the web UI (issue #1449).
//!
//! Three routes and one middleware:
//!
//! - `GET /ui/login` starts an Authorization Code + PKCE login: it remembers a
//!   pending login (CSRF `state` + PKCE verifier) under a short-lived cookie and
//!   redirects the browser to the IdP.
//! - `GET /ui/callback?code&state` completes it: checks `state`, exchanges the
//!   code, establishes the server-side session, sets the session cookie and
//!   lands on the page the user originally asked for.
//! - `POST /ui/logout` ends the session, clears the cookie and, when the IdP
//!   has an end-session endpoint, sends the browser there so the IdP session
//!   ends too.
//! - [`require_session`] runs on every UI page: a request with a valid session
//!   gets its [`helios_auth::Principal`] stamped (so per-user settings key on
//!   the signed-in identity); one without is sent to `/ui/login`.
//!
//! The tokens never reach the browser. The FHIR API authenticates the pages'
//! own browser-originated calls because the REST auth middleware turns the
//! same session cookie into the session's bearer (`AuthMiddlewareState::sessions`).
//!
//! The runtime is **process-wide**, installed once at startup with
//! [`set_interactive_login`] — the same shape as the dashboard refresh
//! cadences — because the session store is shared with the REST auth layer,
//! which is built before the UI is mounted. When nothing is installed the UI
//! behaves exactly as before: no login, no gate.

use std::sync::{Arc, RwLock};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use chrono::Utc;
use helios_auth::{AccessOutcome, PENDING_COOKIE, SESSION_COOKIE, SessionStore};
use serde::Deserialize;

use crate::WebState;

/// What the UI needs to drive the login flow.
#[derive(Clone)]
pub struct LoginRuntime {
    /// The session store shared with the REST auth middleware.
    pub sessions: Arc<SessionStore>,
}

impl std::fmt::Debug for LoginRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRuntime")
            .field("sessions", &self.sessions)
            .finish()
    }
}

static INTERACTIVE_LOGIN: RwLock<Option<LoginRuntime>> = RwLock::new(None);

/// Install (or replace) the process-wide interactive login. Called once from
/// the server's startup, after auth is initialized and before the UI is
/// mounted; the mount reads it into `WebState`.
pub fn set_interactive_login(runtime: LoginRuntime) {
    match INTERACTIVE_LOGIN.write() {
        Ok(mut guard) => *guard = Some(runtime),
        Err(poisoned) => *poisoned.into_inner() = Some(runtime),
    }
}

/// The installed runtime, if any.
pub(crate) fn installed() -> Option<Arc<LoginRuntime>> {
    INTERACTIVE_LOGIN
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .map(Arc::new)
}

/// Paths under `/ui` that must stay reachable without a session: the login
/// flow itself and the static assets the login-adjacent pages need.
fn is_open_path(path: &str) -> bool {
    matches!(path, "/ui/login" | "/ui/callback" | "/ui/logout") || path.starts_with("/ui/assets/")
}

/// Middleware: with a login installed, a UI page request must carry a valid
/// session. One that does gets the signed-in [`helios_auth::Principal`] in its
/// extensions; one that does not is redirected to `/ui/login` with the
/// requested path as `next`. Without a login installed this is a no-op.
pub(crate) async fn require_session(
    State(state): State<WebState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return next.run(request).await;
    };
    let path = request.uri().path().to_string();
    if !path.starts_with("/ui") || is_open_path(&path) {
        return next.run(request).await;
    }

    // Resolve the session the way the FHIR layer will a moment later: through
    // `access_token`, which refreshes a token about to lapse and drops a
    // session whose refresh is dead. Gating on a bare lookup let a page render
    // once with a session the API had just discarded — one stale page showing
    // a raw 401 before the next navigation redirected — so the gate now settles
    // the session's fate before anything renders.
    let session = match helios_auth::cookie_value(request.headers(), SESSION_COOKIE) {
        Some(id) => match login.sessions.access_token(&id).await {
            AccessOutcome::Token(_) => login.sessions.get(&id),
            AccessOutcome::NoSession => None,
        },
        None => None,
    };
    match session {
        Some(session) => {
            request.extensions_mut().insert(session_principal(&session));
            request.extensions_mut().insert(SignedIn(session.principal));
            next.run(request).await
        }
        None => {
            let next_path = request
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_else(|| "/ui".to_string());
            let target = format!(
                "/ui/login?{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("next", &next_path)
                    .finish()
            );
            // htmx fragment requests cannot follow a redirect into a full-page
            // login; tell htmx to navigate the whole window there instead.
            if request.headers().contains_key("hx-request") {
                return (
                    StatusCode::OK,
                    [(axum::http::HeaderName::from_static("hx-redirect"), target)],
                )
                    .into_response();
            }
            Redirect::to(&target).into_response()
        }
    }
}

/// The signed-in user, stamped on the request by [`require_session`] for
/// templates and handlers that show identity (the user menu, #738).
#[derive(Debug, Clone)]
pub struct SignedIn(pub helios_auth::SessionPrincipal);

fn session_principal(session: &helios_auth::Session) -> helios_auth::Principal {
    helios_auth::Principal {
        subject: session.principal.subject.clone(),
        issuer: session.principal.issuer.clone(),
        tenant_id: None,
        scopes: helios_auth::ScopeSet::empty(),
        jti: None,
        // The UI never authorizes by scope or expiry itself — the FHIR layer
        // does, on the bearer the session stands for — so this is only the
        // identity that keys per-user settings.
        expires_at: Utc::now() + chrono::Duration::hours(1),
        custom_claims: serde_json::Map::new(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LoginQuery {
    #[serde(default)]
    next: Option<String>,
}

/// `GET /ui/login` — start the login and send the browser to the IdP.
pub(crate) async fn login(
    State(state): State<WebState>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let next = query.next.as_deref().unwrap_or("/ui");
    let (pending_id, authorize_url) = login.sessions.begin(next).await;
    tracing::info!(next = %next, "web login started; redirecting to the identity provider");
    let secure = login.sessions.config().cookie_secure;
    let mut response = Redirect::to(&authorize_url).into_response();
    append_cookie(
        &mut response,
        &cookie(PENDING_COOKIE, &pending_id, Some(600), secure),
    );
    response
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// `GET /ui/callback` — the IdP sends the browser back here with the code.
pub(crate) async fn callback(
    State(state): State<WebState>,
    Query(query): Query<CallbackQuery>,
    request: Request,
) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let secure = login.sessions.config().cookie_secure;

    if let Some(error) = query.error {
        let detail = query.error_description.unwrap_or_default();
        return login_failed(
            StatusCode::BAD_REQUEST,
            &format!("the identity provider refused the login: {error} {detail}"),
            secure,
        );
    }
    let (Some(code), Some(state_param)) = (query.code, query.state) else {
        return login_failed(
            StatusCode::BAD_REQUEST,
            "the callback carried no code or state",
            secure,
        );
    };
    let Some(pending_id) = helios_auth::cookie_value(request.headers(), PENDING_COOKIE) else {
        return login_failed(
            StatusCode::BAD_REQUEST,
            "no login is pending in this browser — start again from /ui/login",
            secure,
        );
    };

    match login
        .sessions
        .complete(&pending_id, &state_param, &code)
        .await
    {
        Ok((session, next)) => {
            tracing::info!(
                subject = %session.principal.subject,
                user = %session.principal.display(),
                "web login completed"
            );
            let mut response = Redirect::to(&next).into_response();
            append_cookie(
                &mut response,
                &cookie(SESSION_COOKIE, &session.id, None, secure),
            );
            append_cookie(&mut response, &cookie(PENDING_COOKIE, "", Some(0), secure));
            response
        }
        Err(err) => login_failed(StatusCode::BAD_REQUEST, &err.to_string(), secure),
    }
}

/// `POST /ui/logout` — end the session; then the IdP's session too, when it
/// offers RP-initiated logout.
pub(crate) async fn logout(State(state): State<WebState>, request: Request) -> Response {
    let Some(login) = state.login.as_ref() else {
        return not_configured();
    };
    let secure = login.sessions.config().cookie_secure;
    let session_id = helios_auth::cookie_value(request.headers(), SESSION_COOKIE);
    let end_session = match session_id.as_deref() {
        Some(id) => login.sessions.logout(id).await,
        None => None,
    };

    let post_logout = format!("{}/ui", state.public_base_url.trim_end_matches('/'));
    let end_session_requested = end_session.is_some();
    let target = match end_session {
        Some((endpoint, id_token_hint)) => {
            let mut query = form_urlencoded::Serializer::new(String::new());
            query.append_pair("post_logout_redirect_uri", &post_logout);
            query.append_pair("client_id", &login.sessions.config().client_id);
            if let Some(hint) = id_token_hint {
                query.append_pair("id_token_hint", &hint);
            }
            let separator = if endpoint.contains('?') { '&' } else { '?' };
            format!("{endpoint}{separator}{}", query.finish())
        }
        None => "/ui/login".to_string(),
    };
    tracing::info!(
        idp_logout = end_session_requested,
        "web logout; session cleared"
    );
    let mut response = Redirect::to(&target).into_response();
    append_cookie(&mut response, &cookie(SESSION_COOKIE, "", Some(0), secure));
    response
}

fn not_configured() -> Response {
    (
        StatusCode::NOT_FOUND,
        "interactive login is not configured on this server",
    )
        .into_response()
}

fn login_failed(status: StatusCode, detail: &str, secure: bool) -> Response {
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("Login failed: {detail}"),
    )
        .into_response();
    append_cookie(&mut response, &cookie(PENDING_COOKIE, "", Some(0), secure));
    response
}

/// A `Set-Cookie` value. `max_age` of `Some(0)` deletes the cookie; `None`
/// makes it a session cookie (cleared when the browser closes).
fn cookie(name: &str, value: &str, max_age: Option<u64>, secure: bool) -> String {
    let mut parts = vec![
        format!("{name}={value}"),
        "Path=/".to_string(),
        "HttpOnly".to_string(),
        "SameSite=Lax".to_string(),
    ];
    if secure {
        parts.push("Secure".to_string());
    }
    if let Some(max_age) = max_age {
        parts.push(format!("Max-Age={max_age}"));
    }
    parts.join("; ")
}

fn append_cookie(response: &mut Response, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_paths_bypass_the_session_gate() {
        assert!(is_open_path("/ui/login"));
        assert!(is_open_path("/ui/callback"));
        assert!(is_open_path("/ui/logout"));
        assert!(is_open_path("/ui/assets/app.css"));
        assert!(!is_open_path("/ui"));
        assert!(!is_open_path("/ui/resources"));
    }

    #[test]
    fn session_cookie_is_httponly_lax_and_secure_by_default() {
        let c = cookie(SESSION_COOKIE, "abc", None, true);
        assert_eq!(c, "hfs_session=abc; Path=/; HttpOnly; SameSite=Lax; Secure");
        let dev = cookie(SESSION_COOKIE, "abc", None, false);
        assert!(!dev.contains("Secure"));
        let cleared = cookie(SESSION_COOKIE, "", Some(0), true);
        assert!(cleared.ends_with("Max-Age=0"));
    }
}

/// The `Authorization` value a self-call made on this request's behalf should
/// carry when the browser sent none of its own: the signed-in user's session
/// bearer (issue #1480). Resolved through the same `access_token` path the
/// REST middleware uses — a token about to lapse is refreshed, a session whose
/// refresh is dead is dropped — so a page's server-side call runs with exactly
/// the credential its browser-side calls run with. `None` when no login is
/// installed or the request carries no valid session; the caller then falls
/// back to the process's outbound service credential as before.
pub(crate) async fn session_authorization(
    state: &WebState,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    let login = state.login.as_ref()?;
    let id = helios_auth::cookie_value(headers, SESSION_COOKIE)?;
    match login.sessions.access_token(&id).await {
        AccessOutcome::Token(token) => Some(format!("Bearer {token}")),
        AccessOutcome::NoSession => None,
    }
}

/// A [`crate::Caller`] for this request: the browser's own `Authorization`
/// when it sent one, else the signed-in session's bearer (#1480), else none —
/// in which case the conformance source applies the outbound service
/// credential. The `$sql-export` self-calls run as the user who clicked.
pub(crate) async fn caller_for(
    state: &WebState,
    headers: &axum::http::HeaderMap,
    tenant: &str,
) -> crate::Caller {
    let mut caller = crate::Caller::from_request(headers, tenant);
    if caller.authorization.is_none() {
        caller.authorization = session_authorization(state, headers).await;
    }
    caller
}

/// The credential a page's own server-side self-call carries under the
/// interactive login (#1480): the browser's `Authorization` when it sent one,
/// else the signed-in session's bearer, else nothing (the outbound service
/// credential applies downstream).
#[cfg(test)]
mod selfcall_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::http::{HeaderMap, HeaderValue, header};
    use helios_auth::{LoginConfig, Session, SessionPrincipal, SessionStore};

    use super::*;

    const TOKEN: &str = "access-token-1";

    fn store() -> Arc<SessionStore> {
        let store = Arc::new(SessionStore::new(LoginConfig {
            client_id: "hfs-web".to_string(),
            client_secret: None,
            redirect_uri: "http://localhost:8080/ui/callback".to_string(),
            scopes: "openid".to_string(),
            authorization_endpoint: "https://idp.example.com/auth".to_string(),
            token_endpoint: "https://idp.example.com/token".to_string(),
            end_session_endpoint: None,
            cookie_secure: true,
        }));
        store.insert(Session {
            id: "sess-1".to_string(),
            principal: SessionPrincipal {
                subject: "demo-sub".to_string(),
                issuer: "https://idp.example.com".to_string(),
                name: Some("Demo User".to_string()),
                preferred_username: None,
                email: None,
                picture: None,
            },
            access_token: TOKEN.to_string(),
            access_expires_at: Instant::now() + Duration::from_secs(300),
            refresh_token: None,
            id_token: None,
            last_seen: Instant::now(),
            created_at: Utc::now(),
        });
        store
    }

    /// A real `WebState`, with or without a login installed.
    fn web_state(sessions: Option<Arc<SessionStore>>) -> WebState {
        let source: Arc<dyn crate::ConformanceSource> = Arc::new(
            crate::StaticConformanceSource::from_data_dir(std::path::Path::new("../../data")),
        );
        WebState {
            version: "9.9.9",
            sp_catalog: Arc::new(crate::search_params::SpCatalog::new(source.clone())),
            nl: Arc::new(crate::NlSearch::default()),
            compartments: Arc::new(crate::compartments::CompartmentCatalog::new(source.clone())),
            conformance: source,
            tenants: None,
            provisioning: Default::default(),
            data_dir: None,
            public_base_url: "http://localhost:8080".to_string(),
            self_base_url: "http://localhost:8080".to_string(),
            outbound_auth: Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            tenant_path_routing: false,
            fhir_version: helios_fhir::FhirVersion::R4,
            default_tenant: "default".to_string(),
            terminology: None,
            settings: None,
            bulk_provider: None,
            write_observer: None,
            patient_name_search: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            login: sessions.map(|sessions| Arc::new(LoginRuntime { sessions })),
        }
    }

    fn cookie(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[tokio::test]
    async fn session_authorization_is_the_session_bearer_only_for_a_valid_cookie() {
        let state = web_state(Some(store()));
        assert_eq!(
            session_authorization(&state, &cookie("hfs_lang=es; hfs_session=sess-1")).await,
            Some(format!("Bearer {TOKEN}"))
        );
        assert_eq!(session_authorization(&state, &HeaderMap::new()).await, None);
        assert_eq!(
            session_authorization(&state, &cookie("hfs_session=nope")).await,
            None
        );
        // No login installed: the cookie means nothing.
        let no_login = web_state(None);
        assert_eq!(
            session_authorization(&no_login, &cookie("hfs_session=sess-1")).await,
            None
        );
    }

    #[tokio::test]
    async fn caller_for_prefers_the_browser_header_then_the_session_then_nothing() {
        let state = web_state(Some(store()));

        let mut both = cookie("hfs_session=sess-1");
        both.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer browser-token"),
        );
        let caller = caller_for(&state, &both, "clinic-a").await;
        assert_eq!(
            caller.authorization.as_deref(),
            Some("Bearer browser-token")
        );
        assert_eq!(caller.tenant, "clinic-a");

        let caller = caller_for(&state, &cookie("hfs_session=sess-1"), "clinic-a").await;
        assert_eq!(
            caller.authorization.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );

        let caller = caller_for(&state, &HeaderMap::new(), "clinic-a").await;
        assert_eq!(caller.authorization, None);
    }

    async fn built_headers(state: &WebState, headers: &HeaderMap) -> reqwest::header::HeaderMap {
        let request = reqwest::Client::new().get("http://127.0.0.1:1/Patient/$export");
        crate::bulk_export::forward_identity(state, request, headers, "clinic-a", "aud")
            .await
            .expect("credential resolved")
            .build()
            .expect("request builds")
            .headers()
            .clone()
    }

    #[tokio::test]
    async fn forward_identity_runs_the_self_call_as_the_signed_in_user() {
        let state = web_state(Some(store()));

        // Session cookie, no browser header: the session's bearer goes on.
        let sent = built_headers(&state, &cookie("hfs_session=sess-1")).await;
        assert_eq!(
            sent.get("authorization").and_then(|v| v.to_str().ok()),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(
            sent.get("x-tenant-id").and_then(|v| v.to_str().ok()),
            Some("clinic-a")
        );

        // The browser's own header still wins, verbatim.
        let mut both = cookie("hfs_session=sess-1");
        both.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer browser-token"),
        );
        let sent = built_headers(&state, &both).await;
        assert_eq!(
            sent.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer browser-token")
        );

        // No session: the outbound provider (a no-op here) is all there is.
        let sent = built_headers(&state, &HeaderMap::new()).await;
        assert!(sent.get("authorization").is_none());
    }

    async fn import_headers(
        state: &WebState,
        submission: &crate::bulk_import::Submission,
        headers: &HeaderMap,
    ) -> reqwest::header::HeaderMap {
        let request = reqwest::Client::new().post("http://localhost:8080/$bulk-submit");
        crate::bulk_import::authorize_self_call(state, submission, headers, request, "aud")
            .await
            .expect("credential resolved")
            .build()
            .expect("request builds")
            .headers()
            .clone()
    }

    #[tokio::test]
    async fn the_import_page_submits_to_this_server_as_the_signed_in_user_too() {
        let state = web_state(Some(store()));
        let to_self = crate::bulk_import::Submission {
            auth: "none".to_string(),
            recipient_base_url: "http://localhost:8080".to_string(),
            ..Default::default()
        };

        // Same order as every other page: the session's bearer goes on.
        let sent = import_headers(&state, &to_self, &cookie("hfs_session=sess-1")).await;
        assert_eq!(
            sent.get("authorization").and_then(|v| v.to_str().ok()),
            Some(format!("Bearer {TOKEN}").as_str())
        );

        // The browser's own header still wins.
        let mut both = cookie("hfs_session=sess-1");
        both.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer browser-token"),
        );
        let sent = import_headers(&state, &to_self, &both).await;
        assert_eq!(
            sent.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer browser-token")
        );

        // Another server never sees this server's user or service token.
        let elsewhere = crate::bulk_import::Submission {
            auth: "none".to_string(),
            recipient_base_url: "https://recipient.example".to_string(),
            ..Default::default()
        };
        let sent = import_headers(&state, &elsewhere, &cookie("hfs_session=sess-1")).await;
        assert!(sent.get("authorization").is_none());
    }
}
