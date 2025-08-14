//! FHIRPath server executable
//!
//! This binary provides an HTTP server for FHIRPath expression evaluation,
//! compatible with fhirpath-lab and other tools that follow the server-api.md
//! specification.
//!
//! The server accepts FHIR Parameters resources containing expressions and
//! resources, evaluates them, and returns results in a standardized format.
//!
//! See the server module documentation for configuration options.

use clap::Parser;
use helios_fhirpath::server::{ServerArgs, ServerConfig, run_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = ServerArgs::parse();
    let config = ServerConfig::from(args);
    run_server(config).await?;
    Ok(())
}
