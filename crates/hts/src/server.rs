//! Axum application router for the Helios Terminology Server.
//!
//! [`create_app`] is the single entry point that wires every HTTP handler to
//! its route and attaches middleware.  It is called from `main.rs` during
//! server startup and from integration tests that use the crate as a library.
//!
//! ## Middleware stack (outermost → innermost)
//!
//! 1. **Tracing** — emits `tracing` spans for every request/response pair.
//! 2. **Timeout** — hard 30-second request deadline (non-configurable).
//! 3. **CORS** — configurable allowed origins; enabled by default with `*`.
//! 4. **Response compression** — gzip/deflate/br/zstd when the client sends
//!    `Accept-Encoding` (adds `Content-Encoding` + `Vary: Accept-Encoding`).
//! 5. **Request decompression** — bodies sent with `Content-Encoding` are
//!    decompressed before parsing; unsupported encodings get `415`.
//! 6. **Body-size limit** — `HTS_MAX_BODY_SIZE` (default 10 MiB), measured on
//!    the decompressed body.
//!
//! ## Route registration order
//!
//! Instance-level operation routes (e.g., `/CodeSystem/{id}/$lookup`) **must**
//! be registered before the bare `/{id}` CRUD routes.  Axum resolves routes in
//! registration order; placing the operation routes first prevents the CRUD
//! handler from capturing requests that end with an operation suffix.

use crate::operations::batch::batch_handler;
use crate::operations::batch_validate::vs_batch_validate_handler;
use axum::extract::DefaultBodyLimit;
use axum::{
    Router,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, decompression::RequestDecompressionLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config::HtsConfig;
use crate::import::BundleImportBackend;
use crate::operations::closure::closure_handler;
use crate::operations::crud::{
    create_code_system, create_concept_map, create_value_set, delete_code_system,
    delete_concept_map, delete_value_set, read_code_system, read_concept_map, read_value_set,
    update_code_system, update_concept_map, update_value_set,
};
use crate::operations::expand::{
    expand_by_id_post, expand_handler, get_expand_by_id, get_expand_handler,
};
use crate::operations::health::health_handler;
use crate::operations::import_bundle::import_handler;
use crate::operations::lookup::{
    get_lookup_by_id, get_lookup_handler, lookup_by_id_post, lookup_handler,
};
use crate::operations::metadata::metadata_handler;
use crate::operations::search::{search_code_systems, search_concept_maps, search_value_sets};
use crate::operations::subsumes::{get_subsumes_handler, subsumes_handler};
use crate::operations::translate::{
    get_translate_by_id, get_translate_handler, translate_by_id_post, translate_handler,
};
use crate::operations::validate_code::{
    get_validate_code_handler, get_vs_validate_by_id, get_vs_validate_code_handler,
    validate_code_handler, vs_validate_by_id_post, vs_validate_code_handler,
};
use crate::state::AppState;
use crate::traits::TerminologyBackend;

/// Redirects the bare root URL to the HTS UI home (`/ui/hts`).
///
/// Mounted by [`create_app`] only when `config.ui_enabled` is true, so a UI-off
/// deployment keeps axum's default 405 on `GET /` instead of pointing operators
/// at a 404. Returns a 308 (permanent) mirroring the trailing-slash
/// canonicalization pattern in `helios-hts-ui`'s `home.rs`; the sibling
/// `POST /` FHIR batch route on the same path is unaffected because axum merges
/// compatible method routers per path.
async fn root_redirect() -> Response {
    Redirect::permanent("/ui/hts").into_response()
}

/// Build the Axum application router with all middleware and routes.
///
/// Accepts the runtime [`HtsConfig`] (for CORS and other settings) and the
/// shared [`AppState`] (backend + pools).  Returns a fully configured Axum
/// [`Router`] ready to be bound to a TCP listener.
///
/// # Panics
///
/// Does not panic.  Invalid CORS origin strings are silently dropped because
/// [`CorsLayer`] validates origins lazily at request time.
pub fn create_app<B>(config: &HtsConfig, state: AppState<B>) -> Router
where
    B: TerminologyBackend + BundleImportBackend + Clone,
{
    let cors = build_cors(config);

    // Optional HTS administrative UI (crates/hts-ui) mounted at `/ui` so
    // routes resolve as `/ui/hts`, `/ui/hts/assets/*`, etc. The crate owns
    // the `/hts` prefix internally so this mount point stays `/ui`, the same
    // place HFS mounts its own UI. On by default; opt out with
    // `HTS_UI_ENABLED=false`.
    //
    // Upstream URL policy (design doc §7 degraded state contract):
    //   1. `HTS_UI_UPSTREAM_URL` when set — lets a developer point the UI at
    //      a remote HTS without a rebuild;
    //   2. otherwise loopback to *this* binary's `127.0.0.1:{port}`. Not
    //      `config.host` because that may be `0.0.0.0` for external binds;
    //      the loopback client always uses the loopback interface.
    //
    // Bundled data footprint powers the dashboard's "Bundled data: X MB"
    // tile. `None` when no `HTS_BOOTSTRAP_DIR` was configured — the tile
    // then shows an em-dash rather than a misleading zero.
    let hts_ui = if config.ui_enabled {
        let upstream_url = std::env::var("HTS_UI_UPSTREAM_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
        let upstream = helios_hts_ui::UpstreamClient::new(upstream_url).unwrap_or_else(|err| {
            // reqwest's builder fails only under very degenerate conditions
            // (e.g. no TLS backend, which cannot happen here — we use
            // default_features = false with no TLS feature). Log loudly if
            // it ever does and fall back to a client aimed at a closed loopback
            // port so the dashboard degrades cleanly instead of the whole
            // binary crashing.
            tracing::error!(
                ?err,
                "hts-ui: upstream client build failed; UI will render only the degraded banner"
            );
            helios_hts_ui::UpstreamClient::new("http://127.0.0.1:1")
                .expect("closed loopback URL should always parse")
        });
        let bundled_data_bytes = if config.bootstrap_dir.is_empty() {
            None
        } else {
            match dir_size_bytes(std::path::Path::new(&config.bootstrap_dir)) {
                Ok(bytes) => Some(bytes),
                Err(err) => {
                    tracing::warn!(
                        dir = %config.bootstrap_dir,
                        error = %err,
                        "hts-ui: could not compute bootstrap dir size; the tile will read `—`"
                    );
                    None
                }
            }
        };
        let ui_state = std::sync::Arc::new(helios_hts_ui::HtsUiState {
            fhir_version: FHIR_VERSION_LABEL,
            version: env!("CARGO_PKG_VERSION"),
            upstream,
            bundled_data_bytes,
            // Rolling `/metrics` samples for the Home request-rate chart.
            // Starts empty; the Home page's own 15 s poll fills it.
            metrics_ring: Default::default(),
        });
        Some(helios_hts_ui::router(ui_state))
    } else {
        None
    };

    let router = Router::new()
        // ── Batch / transaction ───────────────────────────────────────────────
        .route("/", post(batch_handler::<B>))
        // ── Utility ──────────────────────────────────────────────────────────
        .route("/health", get(health_handler::<B>))
        // ── Capabilities ─────────────────────────────────────────────────────
        .route("/metadata", get(metadata_handler::<B>))
        // ── CodeSystem operations ─────────────────────────────────────────────
        .route(
            "/CodeSystem/$lookup",
            get(get_lookup_handler::<B>).post(lookup_handler::<B>),
        )
        .route(
            "/CodeSystem/$validate-code",
            get(get_validate_code_handler::<B>).post(validate_code_handler::<B>),
        )
        .route(
            "/CodeSystem/$subsumes",
            get(get_subsumes_handler::<B>).post(subsumes_handler::<B>),
        )
        // ── Bundle import ─────────────────────────────────────────────────────
        .route("/import", post(import_handler::<B>))
        // ── ValueSet operations ───────────────────────────────────────────────
        .route(
            "/ValueSet/$expand",
            get(get_expand_handler::<B>).post(expand_handler::<B>),
        )
        .route(
            "/ValueSet/$validate-code",
            get(get_vs_validate_code_handler::<B>).post(vs_validate_code_handler::<B>),
        )
        .route(
            "/ValueSet/$batch-validate-code",
            post(vs_batch_validate_handler::<B>),
        )
        // ── ConceptMap operations ─────────────────────────────────────────────
        .route(
            "/ConceptMap/$translate",
            get(get_translate_handler::<B>).post(translate_handler::<B>),
        )
        .route("/ConceptMap/$closure", post(closure_handler::<B>))
        // ── Resource CRUD + Search ────────────────────────────────────────────────
        // GET /CodeSystem searches; POST /CodeSystem creates.
        .route(
            "/CodeSystem",
            get(search_code_systems::<B>).post(create_code_system::<B>),
        )
        // Instance-level CodeSystem operations MUST be registered before /{id} CRUD.
        .route(
            "/CodeSystem/{id}/$lookup",
            get(get_lookup_by_id::<B>).post(lookup_by_id_post::<B>),
        )
        .route(
            "/CodeSystem/{id}",
            get(read_code_system::<B>)
                .put(update_code_system::<B>)
                .delete(delete_code_system::<B>),
        )
        // GET /ValueSet searches; POST /ValueSet creates.
        .route(
            "/ValueSet",
            get(search_value_sets::<B>).post(create_value_set::<B>),
        )
        // Instance-level operations MUST be registered before /{id} so Axum
        // matches /ValueSet/abc/$expand before /ValueSet/{id}.
        .route(
            "/ValueSet/{id}/$expand",
            get(get_expand_by_id::<B>).post(expand_by_id_post::<B>),
        )
        .route(
            "/ValueSet/{id}/$validate-code",
            get(get_vs_validate_by_id::<B>).post(vs_validate_by_id_post::<B>),
        )
        .route(
            "/ValueSet/{id}",
            get(read_value_set::<B>)
                .put(update_value_set::<B>)
                .delete(delete_value_set::<B>),
        )
        // GET /ConceptMap searches; POST /ConceptMap creates.
        .route(
            "/ConceptMap",
            get(search_concept_maps::<B>).post(create_concept_map::<B>),
        )
        // Instance-level ConceptMap operations.
        .route(
            "/ConceptMap/{id}/$translate",
            get(get_translate_by_id::<B>).post(translate_by_id_post::<B>),
        )
        .route(
            "/ConceptMap/{id}",
            get(read_concept_map::<B>)
                .put(update_concept_map::<B>)
                .delete(delete_concept_map::<B>),
        )
        .with_state(state);

    // Merge the optional UI router before wrapping the whole app in the
    // observability / cors / timeout / trace layers so the UI benefits from
    // the same shared middleware stack (metrics + trace spans).
    //
    // `GET /` -> 308 `/ui/hts`: reviewer-requested landing so browsers hitting
    // the bare root URL end up on the HTS UI home instead of seeing axum's
    // "405 Method Not Allowed" for the POST-only FHIR batch endpoint.
    // Registered inside the `ui_enabled` branch so a UI-off deployment keeps
    // the current 405 behavior (redirecting to `/ui/hts` when that route is
    // not mounted would send operators to a 404). `Redirect::permanent`
    // (308) is the idiomatic choice mirrored from `hts-ui/src/home.rs`
    // trailing-slash canonicalization; the existing `POST /` batch handler
    // is unaffected because axum's `Router::route` merges compatible method
    // routers on the same path.
    let router = if let Some(ui) = hts_ui {
        router.route("/", get(root_redirect)).nest("/ui", ui)
    } else {
        router
    };

    router
        // Raise the body-size limit from axum's 2 MiB default to the
        // configured ceiling. The decompression layer below replaces the
        // request body before extractors read it, so this limit applies to
        // the *decompressed* bytes — a small highly-compressed payload
        // cannot bypass `HTS_MAX_BODY_SIZE`.
        .layer(DefaultBodyLimit::max(config.max_body_size))
        // Decompress request bodies sent with `Content-Encoding` (gzip,
        // deflate, br, zstd); unsupported encodings get 415. Compress
        // responses when the client sends `Accept-Encoding`.
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
        // Observability: `/metrics` (state-free) + per-request metrics/trace span.
        .merge(helios_observability::metrics::router())
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ))
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(TraceLayer::new_for_http())
}

/// Recursively size a directory in bytes. Small utility used at HTS-UI
/// mount time to compute the "Bundled data: X MiB" dashboard tile.
///
/// Returns `Err` on the first I/O failure so operator-facing errors don't
/// mix "partial walk succeeded" with "walk failed" — callers log and fall
/// back to `None` for the tile.
fn dir_size_bytes(dir: &std::path::Path) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                stack.push(entry?.path());
            }
        } else if meta.is_file() {
            total = total.saturating_add(meta.len());
        }
        // Symlinks and other non-file entries: skip silently — the bootstrap
        // directory contract is "regular files under a directory tree".
    }
    Ok(total)
}

/// Compile-time FHIR version label rendered in the HTS UI topbar.
///
/// The `hts` binary is built for exactly one FHIR version (features are
/// mutually exclusive in this crate's build matrix). R4 is the workspace
/// default; the CI matrix and Docker images set exactly one of the four.
#[cfg(feature = "R4")]
const FHIR_VERSION_LABEL: &str = "R4";
#[cfg(all(feature = "R4B", not(feature = "R4")))]
const FHIR_VERSION_LABEL: &str = "R4B";
#[cfg(all(feature = "R5", not(feature = "R4"), not(feature = "R4B")))]
const FHIR_VERSION_LABEL: &str = "R5";
#[cfg(all(
    feature = "R6",
    not(feature = "R4"),
    not(feature = "R4B"),
    not(feature = "R5")
))]
const FHIR_VERSION_LABEL: &str = "R6";
#[cfg(not(any(feature = "R4", feature = "R4B", feature = "R5", feature = "R6")))]
const FHIR_VERSION_LABEL: &str = "R4";

fn build_cors(config: &HtsConfig) -> CorsLayer {
    if !config.enable_cors {
        return CorsLayer::new();
    }

    if config.cors_origins.trim() == "*" {
        CorsLayer::permissive()
    } else {
        use axum::http::{Method, header};

        let origins: Vec<axum::http::HeaderValue> = config
            .cors_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        // With explicit origins the browser enforces the allow-lists below;
        // include `Content-Encoding` so clients can send compressed bodies
        // and `Accept-Language` so they can request designation languages.
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                header::CONTENT_TYPE,
                header::ACCEPT,
                header::ACCEPT_LANGUAGE,
                header::AUTHORIZATION,
                header::CONTENT_ENCODING,
            ])
    }
}
