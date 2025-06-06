use clap::Parser;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};

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

    // Parse ViewDefinition (assume R4 for now, could be enhanced to auto-detect)
    let view_definition: SofViewDefinition = {
        #[cfg(feature = "R4")]
        {
            let vd: fhir::r4::ViewDefinition = serde_json::from_str(&view_content)?;
            SofViewDefinition::R4(vd)
        }
        #[cfg(not(feature = "R4"))]
        {
            return Err("R4 feature not enabled".into());
        }
    };

    // Parse Bundle (assume R4 for now, could be enhanced to auto-detect)
    let bundle: SofBundle = {
        #[cfg(feature = "R4")]
        {
            let b: fhir::r4::Bundle = serde_json::from_str(&bundle_content)?;
            SofBundle::R4(b)
        }
        #[cfg(not(feature = "R4"))]
        {
            return Err("R4 feature not enabled".into());
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
