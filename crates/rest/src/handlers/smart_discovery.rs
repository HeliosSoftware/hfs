//! SMART on FHIR discovery endpoint.
//!
//! Serves the `/.well-known/smart-configuration` document.

use axum::{Json, extract::State, response::IntoResponse};
use helios_auth::SmartConfiguration;
use helios_persistence::core::ResourceStorage;

use crate::state::AppState;

/// Handler for `GET /.well-known/smart-configuration`.
///
/// Returns a SMART configuration document built from the server's
/// auth configuration. If auth is not configured, returns a minimal
/// document.
pub async fn smart_configuration_handler<S>(State(state): State<AppState<S>>) -> impl IntoResponse
where
    S: ResourceStorage + Send + Sync,
{
    let config = state.auth_config();
    let smart = SmartConfiguration::from_config(config);
    Json(smart.to_json())
}
