//! FHIRPath CLI executable
//!
//! This binary provides command-line access to FHIRPath expression evaluation.
//! It allows users to evaluate FHIRPath expressions against FHIR resources,
//! with support for variables, context expressions, and debugging features.
//!
//! See the cli module documentation for detailed usage information.

use clap::Parser;
use helios_fhirpath::cli::{Args, run_cli};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    run_cli(args)?;
    Ok(())
}
