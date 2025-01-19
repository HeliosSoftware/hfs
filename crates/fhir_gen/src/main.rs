use clap::Parser;
use fhir_gen::FhirVersion;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, disable_version_flag = true)]
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

    // This path needs to be relative to the root of the project, and not this module AI!
    let output_dir =
        std::path::PathBuf::from("crates/fhir/src").join(args.version.to_string().to_lowercase());

    if let Err(e) = fhir_gen::process_fhir_version(args.version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
