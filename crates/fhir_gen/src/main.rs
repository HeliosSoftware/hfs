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

    // Instead of println below, we should be calling
    match args.version {
        FhirVersion::R4 => println!("Processing R4"),
        FhirVersion::R4B => println!("Processing R4B"),
        FhirVersion::R5 => println!("Processing R5"),
        FhirVersion::R6 => println!("Processing R6"),
        FhirVersion::All => println!("Processing all versions"),
    }
}
