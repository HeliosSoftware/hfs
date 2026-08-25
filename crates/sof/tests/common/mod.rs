//! Common test utilities for server integration tests.
//!
//! Builds the same router the `sof-server` binary serves, so integration
//! tests exercise production wiring rather than a hand-maintained stub.

use axum_test::TestServer;

/// Create a test server instance backed by the production router.
pub async fn test_server() -> TestServer {
    TestServer::new(helios_sof::app::create_app()).expect("failed to create test server")
}
