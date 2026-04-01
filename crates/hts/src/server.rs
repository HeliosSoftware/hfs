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
use crate::operations::expand::expand_handler;
use crate::operations::health::health_handler;
use crate::operations::import_bundle::import_handler;
use crate::operations::lookup::lookup_handler;
use crate::operations::metadata::metadata_handler;
use crate::operations::subsumes::subsumes_handler;
use crate::operations::translate::translate_handler;
use crate::operations::validate_code::{validate_code_handler, vs_validate_code_handler};
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
        // ── Phase 10: TerminologyCapabilities ─────────────────────────────────
        .route("/metadata", get(metadata_handler::<B>))
        // ── Phase 4: CodeSystem operations ───────────────────────────────────
        .route("/CodeSystem/$lookup", post(lookup_handler::<B>))
        .route(
            "/CodeSystem/$validate-code",
            post(validate_code_handler::<B>),
        )
        // ── Phase 5: $subsumes ───────────────────────────────────────────────
        .route("/CodeSystem/$subsumes", post(subsumes_handler::<B>))
        // ── Phase 6: FHIR Bundle import ───────────────────────────────────────
        .route("/import", post(import_handler::<B>))
        // ── Phase 7: ValueSet operations ──────────────────────────────────────
        .route("/ValueSet/$expand", post(expand_handler::<B>))
        .route(
            "/ValueSet/$validate-code",
            post(vs_validate_code_handler::<B>),
        )
        // ── Phase 8: ConceptMap operations ────────────────────────────────────
        .route("/ConceptMap/$translate", post(translate_handler::<B>))
        .route("/ConceptMap/$closure", post(closure_handler::<B>))
        // ── Phase 9: Resource CRUD API ────────────────────────────────────────
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
