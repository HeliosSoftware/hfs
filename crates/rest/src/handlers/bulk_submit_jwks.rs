//! JWKS endpoint for HFS's own bulk-submit signing key.
//!
//! Serves `/.well-known/bulk-submit-jwks.json` — the public key HFS uses when
//! signing outbound SMART Backend Services client assertions. With an `ES384`
//! key a recipient can register this URL instead of being handed a PEM. With
//! `RS384` the set is empty and the key is still registered out of band.
//!
//! This is HFS's *own* key set, for other servers to verify what HFS sends. It
//! is not the identity provider's key set that `jwks_uri` in
//! `/.well-known/smart-configuration` points at, which validates tokens coming
//! in.

use axum::{Json, extract::State, response::IntoResponse};
use helios_persistence::core::ResourceStorage;

use crate::bulk_submit_oauth::derive_public_jwk;
use crate::state::AppState;

/// `GET /.well-known/bulk-submit-jwks.json`
///
/// Returns a JWK Set containing the server's current signing public key. The
/// endpoint needs no authentication (it is in the auth middleware's exempt
/// paths) and serves one key at most.
///
/// The key's `kid` is its RFC 7638 thumbprint, the same value both signing
/// sites put in the `kid` header of their client assertions.
///
/// Responds with an empty `keys` array when no signing key is configured or
/// when the configured algorithm is not ES384. In particular an `RS384`
/// deployment publishes nothing and its assertions carry no `kid`: RSA key
/// derivation was removed over RUSTSEC-2023-0071 (#560), so such a deployment
/// registers its public key with the recipient out of band.
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
