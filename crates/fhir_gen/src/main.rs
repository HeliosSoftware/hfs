use clap::Parser;
use fhir::FhirVersion;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, disable_version_flag = true)]
struct Args {
    /// FHIR version to process
    #[arg(value_enum)]
    version: Option<FhirVersion>,

    /// Process all versions
    #[arg(long, short, conflicts_with = "version")]
    all: bool,
}

fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            println!("FHIR Generator - Process FHIR definitions\n");
            println!("Available versions:");
            println!("  r4   - FHIR Release 4 (default)");
            println!("  r4b  - FHIR Release 4B");
            println!("  r5   - FHIR Release 5");
            println!("  r6   - FHIR Release 6");
            println!("  --all  - Process all versions\n");
            println!("Usage examples:");
            println!("  fhir_gen r5");
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

    if let Err(e) = fhir_gen::process_fhir_version(version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
