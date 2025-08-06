//! # SQL-on-FHIR CLI Tool
//!
//! This module provides a command-line interface for the [SQL-on-FHIR
//! specification](https://sql-on-fhir.org/ig/latest),
//! allowing users to execute ViewDefinition transformations on FHIR Bundle resources
//! and output the results in various formats.
//!
//! ## Overview
//!
//! The CLI tool accepts FHIR ViewDefinition and Bundle resources as input (either from
//! files or stdin) and applies the SQL-on-FHIR transformation to produce structured
//! output in formats like CSV, JSON, or other supported content types.
//!
//! ## Command Line Options
//!
//! ```text
//! -v, --view <VIEW>              Path to ViewDefinition JSON file (or use stdin if not provided)
//! -b, --bundle <BUNDLE>          Path to FHIR Bundle JSON file (or use stdin if not provided)
//! -f, --format <FORMAT>          Output format (csv, json, ndjson, parquet) [default: csv]
//!     --no-headers               Exclude CSV headers (only for CSV format)
//! -o, --output <OUTPUT>          Output file path (defaults to stdout)
//!     --since <SINCE>            Filter resources modified after this time (RFC3339 format)
//!     --limit <LIMIT>            Limit the number of results (1-10000)
//!     --fhir-version <VERSION>   FHIR version to use [default: R4]
//! -h, --help                     Print help
//!
//! * Additional FHIR versions (R4B, R5, R6) available when compiled with corresponding features
//! ```
//!
//! ## Usage Examples
//!
//! ### Basic usage with files
//! ```bash
//! sof-cli --view view_definition.json --bundle patient_bundle.json --format csv
//! ```
//!
//! ### Using stdin for ViewDefinition
//! ```bash
//! cat view_definition.json | sof-cli --bundle patient_bundle.json --format csv
//! ```
//!
//! ### Output to file (CSV includes headers by default)
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json -f csv -o output.csv
//! ```
//!
//! ### CSV output without headers
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json -f csv --no-headers -o output.csv
//! ```
//!
//! ### Using stdin (ViewDefinition from stdin, Bundle from file)
//! ```bash
//! cat view_definition.json | sof-cli --bundle patient_bundle.json
//! ```
//!
//! ### JSON output format
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json -f json
//! ```
//!
//! ### Filter by modification time
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json --since 2024-01-01T00:00:00Z
//! ```
//!
//! ### Limit number of results
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json --limit 100
//! ```
//!
//! ### Combine filters
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json --since 2024-01-01T00:00:00Z --limit 50
//! ```
//!
//! ## Input Requirements
//!
//! - **ViewDefinition**: A FHIR ViewDefinition resource that defines the SQL transformation
//! - **Bundle**: A FHIR Bundle resource containing the resources to be transformed
//! - At least one of ViewDefinition or Bundle must be provided as a file path (not both from stdin)
//!
//! ## Supported Output Formats
//!
//! - `csv` - Comma-separated values (includes headers by default, use --no-headers to exclude)
//! - `json` - JSON format
//! - Other formats supported by the ContentType enum
//!
//! ## FHIR Version Support
//!
//! The CLI defaults to FHIR R4. To use other FHIR versions, the crate must be
//! compiled with the corresponding feature flags:
//! - R4 (default, always available)
//! - R4B (requires `--features R4B` at compile time)
//! - R5 (requires `--features R5` at compile time)
//! - R6 (requires `--features R6` at compile time)
//!
//! When compiled with additional features, use `--fhir-version` to specify the version.

use chrono::{DateTime, Utc};
use clap::Parser;
use helios_fhir::FhirVersion;
use helios_sof::{
    ContentType, RunOptions, SofBundle, SofViewDefinition, run_view_definition_with_options,
};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sof-cli")]
#[command(about = "SQL-on-FHIR CLI tool for running ViewDefinition transformations")]
struct Args {
    /// Path to ViewDefinition JSON file (or use stdin if not provided)
    #[arg(long, short = 'v')]
    view: Option<PathBuf>,

    /// Path to FHIR Bundle JSON file (or use stdin if not provided)
    #[arg(long, short = 'b')]
    bundle: Option<PathBuf>,

    /// Output format (csv, json, ndjson, parquet)
    #[arg(
        long,
        short = 'f',
        default_value = "csv",
        help = "Output format. Valid values: csv (default, includes headers), json, ndjson, parquet"
    )]
    format: String,

    /// Exclude CSV headers (only for CSV format, headers are included by default)
    #[arg(long)]
    no_headers: bool,

    /// Output file path (defaults to stdout)
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,

    /// Filter resources modified after this time (RFC3339 format, e.g., 2024-01-01T00:00:00Z)
    #[arg(long)]
    since: Option<String>,

    /// Limit the number of results (1-10000)
    #[arg(long)]
    limit: Option<usize>,

    /// FHIR version to use for parsing resources
    #[arg(long, value_enum, default_value_t = FhirVersion::R4)]
    fhir_version: FhirVersion,
}

/// Main entry point for the SQL-on-FHIR CLI application.
///
/// This function orchestrates the entire CLI workflow:
/// 1. Parses command-line arguments
/// 2. Validates input constraints (stdin usage)
/// 3. Reads ViewDefinition and Bundle data from files or stdin
/// 4. Parses the JSON data into appropriate FHIR version-specific types
/// 5. Executes the SQL-on-FHIR transformation
/// 6. Outputs the results to stdout or a specified file
///
/// # Returns
///
/// Returns `Ok(())` on successful execution, or an error if any step fails.
/// Common error scenarios include:
/// - Invalid command-line arguments
/// - File I/O errors (missing files, permission issues)
/// - JSON parsing errors (malformed FHIR resources)
/// - SQL-on-FHIR transformation errors
/// - Output writing errors
///
/// # Errors
///
/// This function will return an error if:
/// - Both ViewDefinition and Bundle are attempted to be read from stdin
/// - Input files cannot be read or parsed
/// - The FHIR version feature is not enabled for the specified version
/// - The transformation fails due to invalid ViewDefinition or Bundle content
/// - Output cannot be written to the specified location
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Check that we don't try to read both from stdin
    if args.view.is_none() && args.bundle.is_none() {
        return Err("Cannot read both ViewDefinition and Bundle from stdin. Please provide at least one file path.".into());
    }

    // Read ViewDefinition
    let view_content = match &args.view {
        Some(path) => fs::read_to_string(path)?,
        None => {
            let mut buffer = String::new();
            io::stdin().read_to_string(&mut buffer)?;
            buffer
        }
    };

    // Read Bundle
    let bundle_content = match &args.bundle {
        Some(path) => fs::read_to_string(path)?,
        None => {
            let mut buffer = String::new();
            io::stdin().read_to_string(&mut buffer)?;
            buffer
        }
    };

    // Parse ViewDefinition based on specified FHIR version
    let view_definition: SofViewDefinition = match args.fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let vd: helios_fhir::r4::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R4(vd)
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let vd: helios_fhir::r4b::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R4B(vd)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let vd: helios_fhir::r5::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R5(vd)
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let vd: helios_fhir::r6::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R6(vd)
        }
    };

    // Parse Bundle based on specified FHIR version
    let bundle: SofBundle = match args.fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let b: helios_fhir::r4::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R4(b)
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let b: helios_fhir::r4b::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R4B(b)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let b: helios_fhir::r5::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R5(b)
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let b: helios_fhir::r6::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R6(b)
        }
    };

    // Determine content type
    let content_type = if args.format == "csv" {
        if args.no_headers {
            ContentType::Csv
        } else {
            ContentType::CsvWithHeader
        }
    } else {
        ContentType::from_string(&args.format)?
    };

    // Parse and validate the since parameter
    let since = if let Some(since_str) = &args.since {
        match DateTime::parse_from_rfc3339(since_str) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(_) => {
                return Err(format!(
                    "Invalid --since parameter: '{}'. Must be RFC3339 format (e.g., 2024-01-01T00:00:00Z)",
                    since_str
                ).into());
            }
        }
    } else {
        None
    };

    // Validate the limit parameter
    let limit = if let Some(c) = args.limit {
        if c == 0 {
            return Err("--limit parameter must be greater than 0".into());
        }
        if c > 10000 {
            return Err("--limit parameter cannot exceed 10000".into());
        }
        Some(c)
    } else {
        None
    };

    // Build run options
    let options = RunOptions {
        since,
        limit,
        page: None, // CLI doesn't support page parameter yet
    };

    // Run the transformation
    let result = run_view_definition_with_options(view_definition, bundle, content_type, options)?;

    // Output result
    match args.output {
        Some(path) => fs::write(path, result)?,
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            io::Write::write_all(&mut handle, &result)?;
        }
    }

    Ok(())
}
