use clap::Parser;
use fhir_gen::FhirVersion;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// FHIR version to process
    #[arg(value_enum, default_value_t = FhirVersion::default())]
    version: FhirVersion,
}

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
            println!("  all  - Process all versions\n");
            println!("Usage example:");
            println!("  fhir_gen R5\n");
            e.exit();
        }
    };

    let output_dir = std::path::PathBuf::from("generated");
    if let Err(e) = fhir_gen::process_fhir_version(args.version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
