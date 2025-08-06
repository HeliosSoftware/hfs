//! # FHIR Generator CLI
//!
//! Command-line interface for generating Rust code from FHIR specifications.
//! This binary provides a simple way to process one or more FHIR versions
//! and generate corresponding Rust type definitions.
//!
//! ## Usage
//!
//! ```bash
//! # Generate code for default version (R4)
//! fhir_gen
//!
//! # Generate code for a specific version
//! fhir_gen R5
//!
//! # Generate code for all versions
//! fhir_gen --all
//! ```
//!
//! ## Output
//!
//! Generated Rust files are written to `crates/fhir/src/` with version-specific
//! names (e.g., `r4.rs`, `r5.rs`).

use clap::Parser;
use helios_fhir::FhirVersion;

/// Command-line arguments for the FHIR code generator.
///
/// This structure defines the available command-line options for controlling
/// which FHIR versions to process and how to process them.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, disable_version_flag = true)]
struct Args {
    /// FHIR version to process (R4, R4B, R5, or R6).
    /// If not specified along with --all, defaults to processing all enabled versions.
    #[arg(value_enum)]
    version: Option<FhirVersion>,

    /// Process all versions that are enabled via Cargo features.
    /// This flag conflicts with specifying a specific version.
    #[arg(long, short, conflicts_with = "version")]
    all: bool,
}

/// Main entry point for the FHIR code generator.
///
/// Parses command-line arguments and invokes the appropriate code generation
/// functions based on the specified FHIR version(s).
///
/// # Process
///
/// 1. **Argument Parsing**: Processes command-line arguments with helpful error messages
/// 2. **Version Selection**: Determines which FHIR version(s) to process
/// 3. **Code Generation**: Calls the library function to generate Rust code
/// 4. **Error Handling**: Provides clear error messages and appropriate exit codes
///
/// # Exit Codes
///
/// - `0`: Success
/// - `1`: Error during code generation process
///
/// # Output Directory
///
/// Generated files are always written to `crates/fhir/src/` relative to the
/// current working directory.
fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            println!("FHIR Generator - Process FHIR definitions\n");
            println!("Available versions:");
            println!("  R4   - FHIR Release 4 (default)");
            println!("  R4B  - FHIR Release 4B");
            println!("  R5   - FHIR Release 5");
            println!("  R6   - FHIR Release 6");
            println!("  --all  - Process all versions\n");
            println!("Usage examples:");
            println!("  fhir_gen R5");
            println!("  fhir_gen --all\n");
            e.exit();
        }
    };

    let output_dir = std::path::PathBuf::from("crates/fhir/src");

    // If --all flag is used or no version is specified, process all versions
    let version = if args.all || args.version.is_none() {
        None
    } else {
        args.version
    };

    if let Err(e) = helios_fhir_gen::process_fhir_version(version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
