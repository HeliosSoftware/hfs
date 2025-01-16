use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, ValueEnum)]
enum FhirVersion {
    R4,
    R4B,
    R5,
    R6,
    All,
}

impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// FHIR version to process
    #[arg(value_enum, default_value_t = FhirVersion::default())]
    version: FhirVersion,
}

fn main() {
    let args = Args::parse();

    match args.version {
        FhirVersion::R4 => println!("Processing R4"),
        FhirVersion::R4B => println!("Processing R4B"),
        FhirVersion::R5 => println!("Processing R5"),
        FhirVersion::R6 => println!("Processing R6"),
        FhirVersion::All => println!("Processing all versions"),
    }

    println!("Hello world!");
}
