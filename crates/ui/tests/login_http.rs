//! The interactive browser login (issue #1449) over the mounted UI router.
//!
//! No IdP runs here: `/ui/login` only builds the authorize redirect, and the
//! callback/gate/logout paths are exercised up to — not through — the token
//! exchange. The runtime is process-wide, so every test installs the same one
//! before mounting; the "no login installed" behaviour is covered separately
//! in `login_absent_http.rs` (its own process).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use helios_auth::{LoginConfig, Session, SessionPrincipal, SessionStore};
use tower::ServiceExt;

/// The login runtime is process-wide and `mount` captures it at construction,
/// so a test that installs a store and then seeds it must not interleave with
/// another test's install. Every test holds this for its whole body.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn login_config() -> LoginConfig {
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

/// Installs a fresh session store as the process-wide login and mounts the
/// UI on it. Returns the store so a test can seed sessions.
fn app() -> (Router, Arc<SessionStore>) {
    let sessions = Arc::new(SessionStore::new(login_config()));
    helios_ui::set_interactive_login(helios_ui::LoginRuntime {
        sessions: Arc::clone(&sessions),
    });
    let router = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: false,
            configured: false,
            model: "test-model".to_string(),
        },
        None,
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );
    (router, sessions)
}

fn seeded_session(sessions: &SessionStore) -> Session {
    let session = Session {
        id: "sess-test-1".to_string(),
        principal: SessionPrincipal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com/realms/fhir".to_string(),
            name: Some("Demo User".to_string()),
            preferred_username: Some("demo".to_string()),
            email: Some("demo@example.org".to_string()),
            picture: None,
        },
        access_token: "access-token-1".to_string(),
        access_expires_at: Instant::now() + Duration::from_secs(300),
        refresh_token: None,
        id_token: Some("id.token.hint".to_string()),
        last_seen: Instant::now(),
        created_at: chrono::Utc::now(),
    };
    sessions.insert(session.clone());
    session
}

fn set_cookies(response: &axum::http::Response<Body>) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn login_redirects_to_the_idp_with_pkce_and_sets_the_pending_cookie() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/login?next=%2Fui%2Fresources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(
        location.starts_with("https://idp.example.com/auth?"),
        "{location}"
    );
    assert!(location.contains("client_id=hfs-web"), "{location}");
    assert!(location.contains("response_type=code"), "{location}");
    assert!(
        location.contains("code_challenge_method=S256"),
        "{location}"
    );
    assert!(location.contains("state="), "{location}");
    let cookies = set_cookies(&response);
    let pending = cookies
        .iter()
        .find(|c| c.starts_with("hfs_login="))
        .expect("pending-login cookie");
    assert!(pending.contains("HttpOnly"), "{pending}");
    assert!(pending.contains("SameSite=Lax"), "{pending}");
    assert!(pending.contains("Secure"), "{pending}");
}

#[tokio::test]
async fn a_page_without_a_session_is_sent_to_login_with_next() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/resources?type=Patient")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/ui/login?next=%2Fui%2Fresources%3Ftype%3DPatient"
    );
}

#[tokio::test]
async fn an_htmx_fragment_without_a_session_gets_hx_redirect_instead_of_302() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/resources")
                .header("hx-request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["hx-redirect"],
        "/ui/login?next=%2Fui%2Fresources"
    );
}

#[tokio::test]
async fn a_valid_session_cookie_passes_the_gate() {
    let _serial = SERIAL.lock().await;
    let (app, sessions) = app();
    let session = seeded_session(&sessions);
    let response = app
        .oneshot(
            Request::get("/ui/status")
                .header(header::COOKIE, format!("hfs_session={}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_session_cookie_is_treated_as_signed_out() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/status")
                .header(header::COOKIE, "hfs_session=not-a-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/ui/login?next=%2Fui%2Fstatus"
    );
}

#[tokio::test]
async fn callback_without_a_pending_login_is_rejected() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/callback?code=abc&state=xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_with_a_state_mismatch_is_rejected_and_the_pending_login_is_consumed() {
    let _serial = SERIAL.lock().await;
    let (app, sessions) = app();
    let (pending_id, _) = sessions.begin("/ui");
    let request = || {
        Request::get("/ui/callback?code=abc&state=wrong")
            .header(header::COOKIE, format!("hfs_login={pending_id}"))
            .body(Body::empty())
            .unwrap()
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // The pending cookie is cleared so the browser starts over cleanly.
    assert!(
        set_cookies(&response)
            .iter()
            .any(|c| c.starts_with("hfs_login=;") && c.contains("Max-Age=0")),
        "{:?}",
        set_cookies(&response)
    );
    // And a replay of the same pending id no longer finds a login to attack.
    let replay = app.oneshot(request()).await.unwrap();
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_reporting_an_idp_error_is_rejected() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    let response = app
        .oneshot(
            Request::get("/ui/callback?error=access_denied&error_description=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn logout_clears_the_session_and_sends_the_browser_to_the_idp_end_session() {
    let _serial = SERIAL.lock().await;
    let (app, sessions) = app();
    let session = seeded_session(&sessions);
    let response = app
        .oneshot(
            Request::post("/ui/logout")
                .header(header::COOKIE, format!("hfs_session={}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(
        location.starts_with("https://idp.example.com/logout?"),
        "{location}"
    );
    assert!(
        location.contains("id_token_hint=id.token.hint"),
        "{location}"
    );
    assert!(
        location.contains("post_logout_redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fui"),
        "{location}"
    );
    assert!(
        set_cookies(&response)
            .iter()
            .any(|c| c.starts_with("hfs_session=;") && c.contains("Max-Age=0"))
    );
    assert!(sessions.get(&session.id).is_none(), "session is gone");
}

#[tokio::test]
async fn the_login_routes_and_assets_stay_reachable_without_a_session() {
    let _serial = SERIAL.lock().await;
    let (app, _) = app();
    for path in [
        "/ui/login",
        "/ui/callback?code=a&state=b",
        "/ui/assets/app.css",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(
            response
                .headers()
                .get(header::LOCATION)
                .map(|v| v.to_str().unwrap()),
            Some("/ui/login?next=%2Fui%2Flogin"),
            "{path} must not be gated behind itself"
        );
    }
}

#[tokio::test]
async fn a_session_whose_token_is_dead_is_dropped_at_the_gate_not_after_a_stale_render() {
    let _serial = SERIAL.lock().await;
    let (app, sessions) = app();
    // Expired access token and no refresh token: the FHIR layer would drop it
    // on the page's first API call. The gate must settle that before the page
    // renders, so the user is sent to log in rather than shown a stale page
    // with a raw 401 in it.
    sessions.insert(Session {
        id: "sess-dead".to_string(),
        principal: SessionPrincipal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com/realms/fhir".to_string(),
            name: None,
            preferred_username: None,
            email: None,
            picture: None,
        },
        access_token: "expired".to_string(),
        access_expires_at: Instant::now() - Duration::from_secs(1),
        refresh_token: None,
        id_token: None,
        last_seen: Instant::now(),
        created_at: chrono::Utc::now(),
    });
    let response = app
        .oneshot(
            Request::get("/ui/status")
                .header(header::COOKIE, "hfs_session=sess-dead")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/ui/login?next=%2Fui%2Fstatus"
    );
    assert!(
        sessions.get("sess-dead").is_none(),
        "the dead session is gone"
    );
}

#[tokio::test]
async fn a_signed_in_page_shows_the_user_in_the_account_menu_with_sign_out() {
    let _serial = SERIAL.lock().await;
    let (app, sessions) = app();
    let session = seeded_session(&sessions);
    let response = app
        .oneshot(
            Request::get("/ui")
                .header(header::COOKIE, format!("hfs_session={}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        html.contains("<div class=\"user-menu__name\">Demo User</div>"),
        "display name from the session's claims"
    );
    assert!(
        html.contains("<div class=\"user-menu__hint\">demo@example.org</div>"),
        "email as the secondary line"
    );
    // The partial renders the initials on their own indented line inside the
    // avatar, so compare with the whitespace removed.
    let compact: String = html.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains(">DU</summary>"), "initials in the avatar");
    assert!(
        html.contains("<form class=\"user-menu__out-form\" method=\"post\" action=\"/ui/logout\">"),
        "Sign out posts to /ui/logout"
    );
}
