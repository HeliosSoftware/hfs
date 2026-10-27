//! JWKS endpoint for HFS's own bulk-submit signing key.
//!
//! Serves `/.well-known/bulk-submit-jwks.json` — the public key HFS uses when
//! signing outbound SMART Backend Services client assertions. Recipients register
//! this URL once instead of hand-carrying a PEM on every key rotation.

use axum::{Json, extract::State, response::IntoResponse};
use helios_persistence::core::ResourceStorage;

use crate::bulk_submit_oauth::derive_public_jwk;
use crate::state::AppState;

/// `GET /.well-known/bulk-submit-jwks.json`
///
/// Returns a JWK Set containing the server's current signing public key.
/// Responds with an empty `keys` array when no signing key is configured or
/// when the configured algorithm is not ES384.
pub async fn bulk_submit_jwks_handler<S>(State(state): State<AppState<S>>) -> impl IntoResponse
where
    S: ResourceStorage + Send + Sync,
{
    let config = state.bulk_submit_config();
    let keys = config
        .private_key
        .as_deref()
        .and_then(|pem| derive_public_jwk(pem, &config.signing_alg))
        .into_iter()
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "keys": keys }))
}
