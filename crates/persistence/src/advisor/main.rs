//! Configuration Advisor CLI
//!
//! This binary provides an HTTP server for analyzing and optimizing
//! composite storage configurations.
//!
//! # Usage
//!
//! ```bash
//! # Run with default settings
//! config-advisor
//!
//! # Run with custom port
//! ADVISOR_PORT=9000 config-advisor
//!
//! # Run with custom host
//! ADVISOR_HOST=0.0.0.0 ADVISOR_PORT=8081 config-advisor
//! ```
//!
//! # Environment Variables
//!
//! - `ADVISOR_HOST` - Host to bind to (default: 127.0.0.1)
//! - `ADVISOR_PORT` - Port to bind to (default: 8081)
//! - `ADVISOR_ENABLE_CORS` - Enable CORS (default: true)
//! - `ADVISOR_TIMEOUT` - Request timeout in seconds (default: 30)
//!
//! # API Endpoints
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/health` | GET | Health check |
//! | `/backends` | GET | List available backend types |
//! | `/backends/:kind` | GET | Get capabilities for a backend type |
//! | `/analyze` | POST | Analyze a configuration |
//! | `/validate` | POST | Validate a configuration |
//! | `/suggest` | POST | Get optimization suggestions |
//! | `/simulate` | POST | Simulate query routing |

use helios_persistence::advisor::{AdvisorConfig, AdvisorServer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The advisor is configured purely from the environment and has no
    // argument parser; `--version` is the one flag every Helios binary
    // answers, so a tester can record which build is running (#992).
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("config-advisor {}", helios_persistence::VERSION);
        return Ok(());
    }

    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,helios_persistence=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration from environment
    let config = AdvisorConfig::from_env();

    tracing::info!(
        "Configuration Advisor starting on {}:{}",
        config.host,
        config.port
    );

    // Print available endpoints
    println!("\nAvailable endpoints:");
    println!("  GET  /health              - Health check");
    println!("  GET  /backends            - List backend types");
    println!("  GET  /backends/:kind      - Backend capabilities");
    println!("  POST /analyze             - Analyze configuration");
    println!("  POST /validate            - Validate configuration");
    println!("  POST /suggest             - Get suggestions");
    println!("  POST /simulate            - Simulate query routing");
    println!();

    // Create and run server
    let server = AdvisorServer::new(config);

    // Handle Ctrl+C gracefully
    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        tracing::info!("Shutdown signal received, stopping server...");
    };

    server.run_with_shutdown(shutdown_signal).await?;

    tracing::info!("Server stopped");
    Ok(())
}
