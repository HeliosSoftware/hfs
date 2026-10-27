//! SQLite bindings for the backend-agnostic meta-parameter suite (#523).
//!
//! The scenario lives in `meta_params_suite.rs` and is shared verbatim with the
//! PostgreSQL test binary; this file only supplies the SQLite backend and the
//! `#[tokio::test]` wrapper.
//!
//! The sibling `meta_params_tests.rs` covers the same four parameters on the
//! query side only, on SQLite only (#474). It cannot catch an extraction bug:
//! from there, a filter that never runs and a filter that runs against a
//! missing index row are indistinguishable.

#![cfg(feature = "sqlite")]

use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

use super::meta_params_suite as suite;

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

#[tokio::test]
async fn meta_parameters_match_only_their_carrier() {
    let backend = super::make_sqlite_backend();
    suite::meta_parameters_match_only_their_carrier(&backend, &tenant("meta-params")).await;
}
