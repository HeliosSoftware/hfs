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

    Router::new()
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
        .with_state(state)
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
