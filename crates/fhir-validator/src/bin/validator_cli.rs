//! validator-cli — validate a FHIR resource from a file or stdin against the
//! embedded core schemas (and optional profiles), following the
//! fhirpath-cli / sof-cli pattern.
//!
//! ```text
//! validator-cli patient.json
//! cat patient.json | validator-cli --fhir-version R4 --output json
//! validator-cli patient.json --profile http://example.org/StructureDefinition/my-patient \
//!     --profile-file my-profile-sd.json
//! ```
//!
//! Exit codes: 0 = valid (no error-severity issues), 1 = validation errors,
//! 2 = usage/input failure.

use clap::Parser;
use helios_fhir::FhirVersion;
use helios_fhir_validator::converter::convert;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{
    CompositeResolver, SchemaRegistry, Severity, UnknownProfilePolicy, ValidationOptions, Validator,
};
use std::fs;
use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "validator-cli",
    about = "Validate a FHIR resource against the core specification and optional profiles",
    version
)]
struct Args {
    /// Resource file to validate (JSON). Reads stdin when omitted.
    file: Option<String>,

    /// FHIR version to validate against.
    #[arg(long, value_enum, default_value_t = FhirVersion::R4)]
    fhir_version: FhirVersion,

    /// Additional profile canonical(s) to validate against (repeatable).
    /// The profile must resolve — from --profile-file or the core pack.
    #[arg(long = "profile")]
    profiles: Vec<String>,

    /// StructureDefinition file(s) to convert and make resolvable
    /// (repeatable). Use together with --profile, or let meta.profile pick
    /// them up.
    #[arg(long = "profile-file")]
    profile_files: Vec<String>,

    /// Ignore the profiles the resource claims in meta.profile.
    #[arg(long)]
    no_meta_profiles: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Output::Text)]
    output: Output,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
enum Output {
    Text,
    Json,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let raw = match &args.file {
        Some(path) => match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {path}: {e}");
                return ExitCode::from(2);
            }
        },
        None => {
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("error: cannot read stdin: {e}");
                return ExitCode::from(2);
            }
            buf
        }
    };
    let resource: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: input is not valid JSON: {e}");
            return ExitCode::from(2);
        }
    };

    // Core pack, optionally overlaid with converted profile files.
    let core = core_registry(args.fhir_version);
    let resolver: Arc<dyn helios_fhir_validator::SchemaResolver> = if args.profile_files.is_empty()
    {
        core
    } else {
        let mut overlay = SchemaRegistry::new();
        for path in &args.profile_files {
            let sd: serde_json::Value = match fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: cannot load profile {path}: {e}");
                    return ExitCode::from(2);
                }
            };
            match convert(&sd) {
                Ok(conversion) => {
                    for w in &conversion.warnings {
                        eprintln!("warning: {path}: {w}");
                    }
                    if !overlay.insert(conversion.schema) {
                        eprintln!("error: profile {path} has neither url nor name");
                        return ExitCode::from(2);
                    }
                }
                Err(e) => {
                    eprintln!("error: profile {path} failed to convert: {e}");
                    return ExitCode::from(2);
                }
            }
        }
        Arc::new(CompositeResolver::new(vec![Arc::new(overlay), core]))
    };

    let validator = Validator::new(resolver);
    let opts = ValidationOptions {
        profiles: args.profiles.clone(),
        use_meta_profiles: !args.no_meta_profiles,
        unknown_profile: UnknownProfilePolicy::Warn,
    };
    let outcome = validator.validate_sync(&resource, &opts);

    let error_count = outcome
        .errors
        .iter()
        .filter(|e| e.severity == Severity::Error)
        .count();

    match args.output {
        Output::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "valid": error_count == 0,
                    "errors": outcome.errors,
                }))
                .expect("report serializes")
            );
        }
        Output::Text => {
            if outcome.errors.is_empty() {
                println!("valid: no issues");
            } else {
                for e in &outcome.errors {
                    let sev = if e.severity == Severity::Error {
                        "error"
                    } else {
                        "warning"
                    };
                    let kind = serde_json::to_value(e.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    println!("{sev} [{kind}] {}: {}", e.path, e.message);
                }
                println!(
                    "{} issue(s), {} error(s)",
                    outcome.errors.len(),
                    error_count
                );
            }
        }
    }

    if error_count > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
