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
//! -s, --source <SOURCE>          URL or path to FHIR data source (file://, http://, https://)
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
//! ### Using source parameter for external data
//! ```bash
//! # Load data from a local file
//! sof-cli -v view_definition.json -s file:///path/to/fhir-data.json
//!
//! # Load data from HTTP URL
//! sof-cli -v view_definition.json -s https://example.com/fhir/Bundle/123
//!
//! # Combine source with bundle (merges both data sources)
//! sof-cli -v view_definition.json -s file:///external-data.json -b local-bundle.json
//! ```
//!
//! ## Input Requirements
//!
//! - **ViewDefinition**: A FHIR ViewDefinition resource that defines the SQL transformation
//! - **Data Source**: Either:
//!   - **Bundle**: A FHIR Bundle resource containing the resources to be transformed
//!   - **Source**: A URL or file path to FHIR data (can be Bundle, single resource, or array)
//!   - Both Bundle and Source can be provided (data will be merged)
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
    ContentType, ParquetOptions, RunOptions, SofBundle, SofViewDefinition,
    data_source::{DataSource, UniversalDataSource},
    run_view_definition_with_options,
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

    /// URL or path to FHIR data source (file://, http://, https://, s3://, gs://, azure://)
    #[arg(
        long,
        short = 's',
        help = "URL or path to FHIR data source. Supports:\n  - file:// for local files\n  - http(s):// for web resources\n  - s3:// for AWS S3 (e.g., s3://bucket/path/to/bundle.json)\n  - gs:// for Google Cloud Storage (e.g., gs://bucket/path/to/bundle.json)\n  - azure:// for Azure Blob Storage (e.g., azure://container/path/to/bundle.json)\nCan be a Bundle, single resource, or array of resources.\n\nCloud storage authentication:\n  - AWS S3: Set AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION\n  - GCS: Set GOOGLE_SERVICE_ACCOUNT or use Application Default Credentials\n  - Azure: Set AZURE_STORAGE_ACCOUNT, AZURE_STORAGE_ACCESS_KEY or use managed identity"
    )]
    source: Option<String>,

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

    /// Parquet row group size in MB (default: 256MB, range: 64-1024MB)
    #[arg(
        long,
        value_parser = clap::value_parser!(u32).range(64..=1024),
        default_value = "256",
        help = "Target row group size in MB for Parquet files. Larger values improve compression and columnar efficiency but require more memory."
    )]
    parquet_row_group_size: u32,

    /// Parquet page size in KB (default: 1024KB, range: 64-8192KB)
    #[arg(
        long,
        value_parser = clap::value_parser!(u32).range(64..=8192),
        default_value = "1024",
        help = "Target page size in KB for Parquet files. Smaller pages allow more fine-grained reading, larger pages have less overhead."
    )]
    parquet_page_size: u32,

    /// Parquet compression algorithm
    #[arg(
        long,
        default_value = "snappy",
        value_parser = ["none", "snappy", "gzip", "lz4", "brotli", "zstd"],
        help = "Compression algorithm for Parquet files. Options: none, snappy (default, fast), gzip (compatible), lz4 (fastest), brotli (best ratio), zstd (balanced)"
    )]
    parquet_compression: String,

    /// Maximum file size for output files (in MB, only applies to Parquet format)
    #[arg(
        long,
        value_parser = clap::value_parser!(u32).range(10..=10000),
        help = "Maximum file size in MB for Parquet files. When exceeded, creates multiple numbered files (e.g., output_001.parquet, output_002.parquet). Range: 10-10000MB"
    )]
    max_file_size: Option<u32>,
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
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Check that we have at least a view definition
    if args.view.is_none() {
        return Err(
            "ViewDefinition is required. Please provide a path to ViewDefinition JSON file.".into(),
        );
    }

    // Check that we have either bundle or source for data
    if args.bundle.is_none() && args.source.is_none() {
        return Err(
            "No data source provided. Please provide either --bundle or --source parameter.".into(),
        );
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

    // Load data from source if provided
    let source_bundle = if let Some(source) = &args.source {
        let data_source = UniversalDataSource::new();
        Some(data_source.load(source).await?)
    } else {
        None
    };

    // Read Bundle from file if provided
    let file_bundle = if let Some(bundle_path) = &args.bundle {
        Some(bundle_path)
    } else {
        None
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

    // Determine the final bundle based on available sources
    let bundle: SofBundle = match (source_bundle, file_bundle) {
        // Only source provided
        (Some(bundle), None) => bundle,

        // Only file bundle provided
        (None, Some(bundle_path)) => {
            let bundle_content = fs::read_to_string(bundle_path)?;
            match args.fhir_version {
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
            }
        }

        // Both source and file provided - merge them
        (Some(source_bundle), Some(bundle_path)) => {
            let bundle_content = fs::read_to_string(bundle_path)?;

            // Parse the file bundle
            let file_bundle = match args.fhir_version {
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

            // Merge the bundles - source data comes first
            merge_bundles(source_bundle, file_bundle)?
        }

        // This shouldn't happen due to validation above
        (None, None) => unreachable!("No data source provided"),
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
    let mut options = RunOptions {
        since,
        limit,
        page: None,            // CLI doesn't support page parameter yet
        parquet_options: None, // Will be set if using parquet format
    };

    // Configure parquet options if using parquet format
    if content_type == ContentType::Parquet {
        options.parquet_options = Some(ParquetOptions {
            row_group_size_mb: args.parquet_row_group_size,
            page_size_kb: args.parquet_page_size,
            compression: args.parquet_compression.clone(),
            max_file_size_mb: args.max_file_size,
        });
    }

    // For Parquet with max_file_size, we need special handling
    if content_type == ContentType::Parquet && args.max_file_size.is_some() && args.output.is_some()
    {
        // Process and write Parquet files with splitting
        write_parquet_with_splitting(view_definition, bundle, &args.output.unwrap(), options)?;
    } else {
        // Standard processing for all other cases
        let result =
            run_view_definition_with_options(view_definition, bundle, content_type, options)?;

        // Output result
        match args.output {
            Some(path) => fs::write(path, result)?,
            None => {
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                io::Write::write_all(&mut handle, &result)?;
            }
        }
    }

    Ok(())
}

/// Merge two bundles by extracting and combining their resources
fn merge_bundles(
    source_bundle: SofBundle,
    file_bundle: SofBundle,
) -> Result<SofBundle, Box<dyn std::error::Error>> {
    // Extract all resources from both bundles
    let mut all_resources = Vec::new();

    // Extract from source bundle first (takes precedence)
    match source_bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
    }

    // Extract from file bundle
    match &file_bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        all_resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        all_resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        all_resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        all_resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
    }

    // Create a new bundle with all resources, matching the file bundle's version
    match file_bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(_) => {
            let bundle_json = serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": all_resources.into_iter().map(|r| {
                    serde_json::json!({"resource": r})
                }).collect::<Vec<_>>()
            });
            let bundle = serde_json::from_value(bundle_json)?;
            Ok(SofBundle::R4(bundle))
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(_) => {
            let bundle_json = serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": all_resources.into_iter().map(|r| {
                    serde_json::json!({"resource": r})
                }).collect::<Vec<_>>()
            });
            let bundle = serde_json::from_value(bundle_json)?;
            Ok(SofBundle::R4B(bundle))
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(_) => {
            let bundle_json = serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": all_resources.into_iter().map(|r| {
                    serde_json::json!({"resource": r})
                }).collect::<Vec<_>>()
            });
            let bundle = serde_json::from_value(bundle_json)?;
            Ok(SofBundle::R5(bundle))
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(_) => {
            let bundle_json = serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": all_resources.into_iter().map(|r| {
                    serde_json::json!({"resource": r})
                }).collect::<Vec<_>>()
            });
            let bundle = serde_json::from_value(bundle_json)?;
            Ok(SofBundle::R6(bundle))
        }
    }
}

/// Write Parquet files with splitting when max_file_size is exceeded
fn write_parquet_with_splitting(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
    output_path: &PathBuf,
    options: RunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    use helios_sof::format_parquet_multi_file;

    // Get the max file size in bytes
    let max_file_size_bytes = options
        .parquet_options
        .as_ref()
        .and_then(|opts| opts.max_file_size_mb)
        .map(|mb| mb as usize * 1024 * 1024)
        .unwrap_or(usize::MAX); // No limit if not specified

    // Process the ViewDefinition to get the result
    let processed_result = helios_sof::process_view_definition(view_definition, bundle)?;

    // Generate Parquet files
    let file_buffers = format_parquet_multi_file(
        processed_result,
        options.parquet_options.as_ref(),
        max_file_size_bytes,
    )?;

    // Determine file naming pattern
    let (base_path, extension) = if let Some(ext) = output_path.extension() {
        let base = output_path.with_extension("");
        (base, format!(".{}", ext.to_string_lossy()))
    } else {
        (output_path.clone(), ".parquet".to_string())
    };

    // Write files
    if file_buffers.len() == 1 {
        // Single file - use the original name
        fs::write(output_path, &file_buffers[0])?;
        println!("Wrote {} bytes to {:?}", file_buffers[0].len(), output_path);
    } else {
        // Multiple files - use numbered naming
        for (i, buffer) in file_buffers.iter().enumerate() {
            let file_path = if i == 0 {
                // First file keeps the base name
                PathBuf::from(format!("{}{}", base_path.display(), extension))
            } else {
                // Subsequent files get numbered
                PathBuf::from(format!("{}_{:03}{}", base_path.display(), i + 1, extension))
            };

            fs::write(&file_path, buffer)?;
            println!("Wrote {} bytes to {:?}", buffer.len(), file_path);
        }
        println!("Created {} Parquet files", file_buffers.len());
    }

    Ok(())
}
