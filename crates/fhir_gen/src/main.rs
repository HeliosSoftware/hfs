use clap::Parser;
use fhir::FhirVersion;
use fhir_gen::GeneratorVersion;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, disable_version_flag = true)]
struct Args {
    /// FHIR version to process
    #[arg(value_enum, default_value_t = GeneratorVersion::default())]
    version: GeneratorVersion,
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
            println!("  all  - Process all versions\n");
            println!("Usage example:");
            println!("  fhir_gen r5\n");
            e.exit();
        }
    };

    let output_dir = std::path::PathBuf::from("crates/fhir/src");

    if let Err(e) = fhir_gen::process_fhir_version(args.version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
