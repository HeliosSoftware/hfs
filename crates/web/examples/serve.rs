//! Standalone runner for the `helios-web` proof of concept.
//!
//! ```bash
//! cargo run -p helios-web --example serve
//! # open http://127.0.0.1:8088/
//! ```
//!
//! This exists so the UI can be exercised in a browser without standing up the
//! full `hfs` binary. In production the router is mounted by `hfs`; see the
//! crate README.

use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let app = helios_web::router();
    let addr = "127.0.0.1:8088";
    let listener = TcpListener::bind(addr).await.expect("bind");
    println!("helios-web listening on http://{addr}/");
    axum::serve(listener, app).await.expect("serve");
}
