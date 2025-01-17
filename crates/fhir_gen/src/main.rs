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
    let args = Args::parse();

    let output_dir = std::path::PathBuf::from("generated");
    if let Err(e) = fhir_gen::process_fhir_version(args.version, &output_dir) {
        eprintln!("Error processing FHIR version: {}", e);
        std::process::exit(1);
    }
}
