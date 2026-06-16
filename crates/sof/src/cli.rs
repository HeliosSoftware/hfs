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
//! -s, --source <SOURCE>          Path or URL to FHIR data source (local paths or URLs)
//! -f, --format <FORMAT>          Output format (csv, json, ndjson, parquet) [default: csv]
//!     --no-headers               Exclude CSV headers (only for CSV format)
//! -o, --output <OUTPUT>          Output file path (defaults to stdout)
//!     --since <SINCE>            Filter resources modified after this time (RFC3339 format)
//!     --limit <LIMIT>            Limit the number of results (1-10000)
//! -t, --threads <THREADS>        Number of threads to use for parallel processing
//!     --fhir-version <VERSION>   FHIR version to use [default: R4]
//!     --terminology-server <URL> Terminology server URL for FHIRPath functions
//!                                [env: SOF_TERMINOLOGY_SERVER]
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
//! ### Use multiple threads for parallel processing
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json --threads 8
//! ```
//!
//! ### Combine filters
//! ```bash
//! sof-cli -v view_definition.json -b patient_bundle.json --since 2024-01-01T00:00:00Z --limit 50 --threads 4
//! ```
//!
//! ### Using source parameter for external data
//! ```bash
//! # Load data from a local file (relative path)
//! sof-cli -v view_definition.json -s ./output/fhir/Observation.ndjson
//!
//! # Load data from a local file (absolute path)
//! sof-cli -v view_definition.json -s /path/to/fhir-data.json
//!
//! # Load data from a local file (file:// URL)
//! sof-cli -v view_definition.json -s file:///path/to/fhir-data.json
//!
//! # Load data from HTTP URL
//! sof-cli -v view_definition.json -s https://example.com/fhir/Bundle/123
//!
//! # Combine source with bundle (merges both data sources)
//! sof-cli -v view_definition.json -s ./external-data.json -b local-bundle.json
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
    ChunkConfig, ContentType, ParquetOptions, ProcessingStats, RunOptions, SofBundle,
    SofViewDefinition,
    data_source::{DataSource, UniversalDataSource, parse_fhir_content},
    process_ndjson_chunked, process_ndjson_chunked_remote, run_view_definition_with_options,
    run_view_definition_with_options_remote,
};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sof-cli")]
#[command(about = "SQL-on-FHIR CLI tool for running ViewDefinition transformations")]
struct Args {
    /// Path to ViewDefinition JSON file (or use stdin if not provided)
    #[arg(long, short = 'v')]
    view: Option<PathBuf>,

    /// Path to FHIR Bundle JSON or NDJSON file (or use stdin if not provided)
    #[arg(
        long,
        short = 'b',
        help = "Path to FHIR data file. Can be a Bundle (JSON), single resource, array of resources, or NDJSON file. NDJSON files (.ndjson) are auto-detected."
    )]
    bundle: Option<PathBuf>,

    /// Path or URL to FHIR data source (local paths, file://, http://, https://, s3://, gs://, azure://)
    #[arg(
        long,
        short = 's',
        help = "Path or URL to FHIR data source. Supports:\n  - Local file paths (relative or absolute, e.g., ./data/bundle.json)\n  - file:// for local files\n  - http(s):// for web resources\n  - s3:// for AWS S3 (e.g., s3://bucket/path/to/bundle.json)\n  - gs:// for Google Cloud Storage (e.g., gs://bucket/path/to/bundle.json)\n  - azure:// for Azure Blob Storage (e.g., azure://container/path/to/bundle.json)\n\nCan be a Bundle, single resource, array of resources, or NDJSON file.\nNDJSON files (.ndjson) are auto-detected; multi-line content falls back to NDJSON if JSON parsing fails.\n\nCloud storage authentication:\n  - AWS S3: Set AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_DEFAULT_REGION\n  - S3-compatible (MinIO, Ceph, etc.): Also set AWS_ENDPOINT, and AWS_ALLOW_HTTP=true for HTTP endpoints\n  - GCS: Set GOOGLE_SERVICE_ACCOUNT or use Application Default Credentials\n  - Azure: Set AZURE_STORAGE_ACCOUNT, AZURE_STORAGE_ACCESS_KEY or use managed identity"
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

    /// Number of threads to use for parallel processing (default: system decides)
    #[arg(long, short = 't')]
    threads: Option<usize>,

    /// FHIR version to use for parsing resources
    #[arg(long, value_enum, default_value_t = FhirVersion::default_enabled())]
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

    /// Chunk size for NDJSON streaming (number of resources per chunk)
    #[arg(
        long,
        default_value = "1000",
        help = "Number of resources to process per chunk when streaming NDJSON files. Larger values use more memory but may improve throughput. Default: 1000 (~10MB memory usage per chunk)"
    )]
    chunk_size: usize,

    /// Skip invalid lines in NDJSON files instead of failing
    #[arg(
        long,
        help = "Continue processing when encountering invalid JSON lines in NDJSON files instead of returning an error"
    )]
    skip_invalid: bool,

    /// Terminology server URL for FHIRPath terminology functions (memberOf, subsumes, etc.)
    #[arg(long, env = "SOF_TERMINOLOGY_SERVER")]
    terminology_server: Option<String>,

    /// Enable remote resolution of `Reference.resolve()` against trusted servers.
    /// Off by default; requires --resolve-allowed-base-urls (or SOF_RESOLVE_ALLOWED_BASE_URLS).
    #[arg(long)]
    resolve_remote: bool,

    /// Comma-separated trusted base URLs that remote `resolve()` may fetch from,
    /// e.g. `https://fhir.example.org/r4,https://hapi.example.com/baseR4`.
    /// Overrides SOF_RESOLVE_ALLOWED_BASE_URLS. Other limits use SOF_RESOLVE_* env vars.
    #[arg(long)]
    resolve_allowed_base_urls: Option<String>,

    /// Allow allowlisted hostnames to resolve to private/internal addresses
    /// (RFC1918 / IPv6-ULA), e.g. for an internal load balancer such as Traefik.
    /// Loopback and link-local (cloud-metadata) addresses remain blocked.
    /// Overrides SOF_RESOLVE_ALLOW_PRIVATE_ADDRESSES.
    #[arg(long)]
    resolve_allow_private_addresses: bool,
}

/// Builds the remote-resolution config from env vars, with CLI flags taking
/// precedence (CLI > env > default).
fn build_remote_resolve_config(args: &Args) -> helios_sof::RemoteResolveConfig {
    let mut config = helios_sof::RemoteResolveConfig::from_env();
    if args.resolve_remote {
        config.enabled = true;
    }
    if let Some(csv) = &args.resolve_allowed_base_urls {
        config.allowed_base_urls = helios_sof::parse_allowed_base_urls(csv);
    }
    if args.resolve_allow_private_addresses {
        config.allow_private_addresses = true;
    }
    config
}

/// Normalize a source path to a URL.
///
/// This function converts local file paths (relative or absolute) to file:// URLs,
/// while leaving existing URLs (http://, https://, s3://, etc.) unchanged.
///
/// # Examples
/// - `./data/bundle.json` -> `file:///absolute/path/to/data/bundle.json`
/// - `/absolute/path/bundle.json` -> `file:///absolute/path/bundle.json`
/// - `file:///path/bundle.json` -> `file:///path/bundle.json` (unchanged)
/// - `https://example.com/bundle.json` -> `https://example.com/bundle.json` (unchanged)
fn normalize_source_path(source: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Check if this is already a URL with a scheme
    if source.contains("://") {
        return Ok(source.to_string());
    }

    // Treat as a file path - convert to absolute and create file:// URL
    let path = PathBuf::from(source);
    let absolute_path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };

    // Canonicalize to resolve . and .. components (but don't fail if file doesn't exist yet)
    let canonical_path = absolute_path
        .canonicalize()
        .unwrap_or_else(|_| absolute_path.clone());

    // Convert to file:// URL
    // On Windows, paths like C:\foo need to become file:///C:/foo
    // On Unix, paths like /foo become file:///foo
    #[cfg(windows)]
    let url = {
        let path_str = canonical_path.to_string_lossy();
        // Replace backslashes with forward slashes for URL format
        let normalized = path_str.replace('\\', "/");
        format!("file:///{}", normalized)
    };
    #[cfg(not(windows))]
    let url = format!("file://{}", canonical_path.display());

    Ok(url)
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

    // Propagate SOF_TERMINOLOGY_SERVER to FHIRPATH_TERMINOLOGY_SERVER so that any
    // FHIRPath evaluation (memberOf, subsumes, etc.) delegates terminology
    // operations to the configured HTS instance.
    if let Some(ref ts_url) = args.terminology_server {
        if std::env::var("FHIRPATH_TERMINOLOGY_SERVER").is_err() {
            // SAFETY: called before any threads are spawned by the tokio runtime.
            unsafe {
                std::env::set_var("FHIRPATH_TERMINOLOGY_SERVER", ts_url);
            }
        }
    }

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

    // Check if we should use streaming mode for NDJSON
    // Streaming is used when:
    // 1. --bundle is provided with a .ndjson file extension
    // 2. --source is not also provided (no bundle merging needed)
    // 3. Output format is not Parquet (doesn't support streaming)
    let use_streaming = args
        .bundle
        .as_ref()
        .is_some_and(|p| p.to_string_lossy().to_lowercase().ends_with(".ndjson"))
        && args.source.is_none();

    // Determine content type early (needed for streaming check)
    let content_type = if args.format == "csv" {
        if args.no_headers {
            ContentType::Csv
        } else {
            ContentType::CsvWithHeader
        }
    } else {
        ContentType::from_string(&args.format)?
    };

    // Use streaming path for NDJSON files
    if use_streaming && content_type != ContentType::Parquet {
        let bundle_path = args.bundle.as_ref().unwrap();
        let file = File::open(bundle_path)?;
        let reader = BufReader::new(file);

        let chunk_config = ChunkConfig {
            chunk_size: args.chunk_size,
            skip_invalid_lines: args.skip_invalid,
        };

        // Remote `resolve()` (trusted-server prefetch) for the streaming path uses
        // a per-stream shared cache; inactive config takes the plain sync path.
        let remote_config = build_remote_resolve_config(&args);

        // Create output writer
        let stats: ProcessingStats = match &args.output {
            Some(path) => {
                let file = File::create(path)?;
                let mut writer = BufWriter::new(file);
                if remote_config.is_active() {
                    process_ndjson_chunked_remote(
                        view_definition,
                        reader,
                        &mut writer,
                        content_type,
                        chunk_config,
                        &remote_config,
                    )
                    .await?
                } else {
                    process_ndjson_chunked(
                        view_definition,
                        reader,
                        &mut writer,
                        content_type,
                        chunk_config,
                    )?
                }
            }
            None => {
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                if remote_config.is_active() {
                    process_ndjson_chunked_remote(
                        view_definition,
                        reader,
                        &mut handle,
                        content_type,
                        chunk_config,
                        &remote_config,
                    )
                    .await?
                } else {
                    process_ndjson_chunked(
                        view_definition,
                        reader,
                        &mut handle,
                        content_type,
                        chunk_config,
                    )?
                }
            }
        };

        eprintln!(
            "Processed {} resources in {} chunks, {} output rows",
            stats.resources_processed, stats.chunks_processed, stats.output_rows
        );

        return Ok(());
    }

    // Non-streaming path: load all data into memory
    // Load data from source if provided
    let source_bundle = if let Some(source) = &args.source {
        let data_source = UniversalDataSource::new();
        // Convert relative/absolute file paths to file:// URLs
        let source_url = normalize_source_path(source)?;
        Some(data_source.load(&source_url).await?)
    } else {
        None
    };

    // Read Bundle from file if provided
    // Use parse_fhir_content to support both JSON and NDJSON
    let file_bundle = if let Some(bundle_path) = &args.bundle {
        let bundle_content = fs::read_to_string(bundle_path)?;
        let bundle_path_str = bundle_path.to_string_lossy();
        Some(parse_fhir_content(&bundle_content, &bundle_path_str)?)
    } else {
        None
    };

    // Determine the final bundle based on available sources
    let bundle: SofBundle = match (source_bundle, file_bundle) {
        // Only source provided
        (Some(bundle), None) => bundle,

        // Only file bundle provided (already parsed with NDJSON support)
        (None, Some(bundle)) => bundle,

        // Both source and file provided - merge them
        (Some(source_bundle), Some(file_bundle)) => {
            // Merge the bundles - source data comes first
            merge_bundles(source_bundle, file_bundle)?
        }

        // This shouldn't happen due to validation above
        (None, None) => unreachable!("No data source provided"),
    };

    // content_type already determined above before streaming check

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
    if let (ContentType::Parquet, Some(_), Some(output)) =
        (&content_type, &args.max_file_size, &args.output)
    {
        // Process and write Parquet files with splitting
        write_parquet_with_splitting(view_definition, bundle, output, options)?;
    } else {
        // Standard processing for all other cases
        let remote_config = build_remote_resolve_config(&args);
        let result = if remote_config.is_active() {
            run_view_definition_with_options_remote(
                view_definition,
                bundle,
                content_type,
                options,
                &remote_config,
            )
            .await?
        } else {
            run_view_definition_with_options(view_definition, bundle, content_type, options)?
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_source_path_url_unchanged() {
        // URLs should remain unchanged
        assert_eq!(
            normalize_source_path("file:///path/to/file.json").unwrap(),
            "file:///path/to/file.json"
        );
        assert_eq!(
            normalize_source_path("http://example.com/bundle.json").unwrap(),
            "http://example.com/bundle.json"
        );
        assert_eq!(
            normalize_source_path("https://example.com/bundle.json").unwrap(),
            "https://example.com/bundle.json"
        );
        assert_eq!(
            normalize_source_path("s3://bucket/path/bundle.json").unwrap(),
            "s3://bucket/path/bundle.json"
        );
    }

    #[test]
    fn test_normalize_source_path_absolute_path() {
        // Absolute paths should be converted to file:// URLs
        // Use a platform-appropriate absolute path
        #[cfg(windows)]
        let test_path = "C:\\tmp\\test.json";
        #[cfg(not(windows))]
        let test_path = "/tmp/test.json";

        let result = normalize_source_path(test_path).unwrap();
        assert!(
            result.starts_with("file:///"),
            "Expected file:/// prefix, got: {}",
            result
        );
        // On macOS, /tmp may be symlinked to /private/tmp
        assert!(
            result.contains("tmp") && result.contains("test.json"),
            "Expected path to contain tmp and test.json, got: {}",
            result
        );
    }

    #[test]
    fn test_normalize_source_path_relative_path() {
        // Relative paths should be converted to absolute file:// URLs
        let result = normalize_source_path("./test.json").unwrap();
        assert!(
            result.starts_with("file:///"),
            "Expected file:/// prefix, got: {}",
            result
        );
        // Should contain test.json somewhere in the path
        assert!(
            result.contains("test.json"),
            "Expected path to contain test.json, got: {}",
            result
        );
    }
}
