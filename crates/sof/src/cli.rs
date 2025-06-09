use clap::Parser;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};
use fhir::FhirVersion;

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

    /// Output format
    #[arg(long, short = 'f', default_value = "csv")]
    format: String,

    /// Include CSV headers (only for CSV format)
    #[arg(long)]
    headers: bool,

    /// Output file path (defaults to stdout)
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,

    /// FHIR version to use for parsing resources
    #[arg(long, value_enum, default_value_t = FhirVersion::R4)]
    fhir_version: FhirVersion,
}

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
            let vd: fhir::r4::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R4(vd)
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let vd: fhir::r4b::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R4B(vd)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let vd: fhir::r5::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R5(vd)
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let vd: fhir::r6::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R6(vd)
        }
    };

    // Parse Bundle based on specified FHIR version
    let bundle: SofBundle = match args.fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let b: fhir::r4::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R4(b)
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let b: fhir::r4b::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R4B(b)
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let b: fhir::r5::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R5(b)
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let b: fhir::r6::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R6(b)
        }
    };

    // Determine content type
    let content_type = if args.format == "csv" {
        if args.headers {
            ContentType::CsvWithHeader
        } else {
            ContentType::Csv
        }
    } else {
        ContentType::from_string(&args.format)?
    };

    // Run the transformation
    let result = run_view_definition(view_definition, bundle, content_type)?;

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
