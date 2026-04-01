use axum::{
    Router,
    routing::{get, post},
};
use std::time::Duration;
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::config::HtsConfig;
use crate::import::BundleImportBackend;
use crate::operations::closure::closure_handler;
use crate::operations::crud::{
    create_code_system, create_concept_map, create_value_set, delete_code_system,
    delete_concept_map, delete_value_set, read_code_system, read_concept_map, read_value_set,
    update_code_system, update_concept_map, update_value_set,
};
use crate::operations::expand::{expand_handler, get_expand_handler};
use crate::operations::health::health_handler;
use crate::operations::import_bundle::import_handler;
use crate::operations::lookup::{get_lookup_handler, lookup_handler};
use crate::operations::metadata::metadata_handler;
use crate::operations::subsumes::{get_subsumes_handler, subsumes_handler};
use crate::operations::translate::{get_translate_handler, translate_handler};
use crate::operations::validate_code::{
    get_validate_code_handler, get_vs_validate_code_handler, validate_code_handler,
    vs_validate_code_handler,
};
use crate::state::AppState;
use crate::traits::TerminologyBackend;

/// Build the Axum application router with all middleware and routes.
pub fn create_app<B>(config: &HtsConfig, state: AppState<B>) -> Router
where
    B: TerminologyBackend + BundleImportBackend + Clone,
{
    let cors = build_cors(config);

    Router::new()
        // ── Utility ──────────────────────────────────────────────────────────
        .route("/health", get(health_handler))
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
        // ── ConceptMap operations ─────────────────────────────────────────────
        .route(
            "/ConceptMap/$translate",
            get(get_translate_handler::<B>).post(translate_handler::<B>),
        )
        .route("/ConceptMap/$closure", post(closure_handler::<B>))
        // ── Resource CRUD ─────────────────────────────────────────────────────
        .route("/CodeSystem", post(create_code_system::<B>))
        .route(
            "/CodeSystem/{id}",
            get(read_code_system::<B>)
                .put(update_code_system::<B>)
                .delete(delete_code_system::<B>),
        )
        .route("/ValueSet", post(create_value_set::<B>))
        .route(
            "/ValueSet/{id}",
            get(read_value_set::<B>)
                .put(update_value_set::<B>)
                .delete(delete_value_set::<B>),
        )
        .route("/ConceptMap", post(create_concept_map::<B>))
        .route(
            "/ConceptMap/{id}",
            get(read_concept_map::<B>)
                .put(update_concept_map::<B>)
                .delete(delete_concept_map::<B>),
        )
        .with_state(state)
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
        let origins: Vec<axum::http::HeaderValue> = config
            .cors_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        CorsLayer::new().allow_origin(origins)
    }
}
