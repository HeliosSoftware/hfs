//! Build a [`ProfileManifest`](fhir_validation::ProfileManifest) JSON file from an expanded FHIR NPM / IG folder.
//!
//! Usage:
//! ```text
//! cargo run -p fhir-validation --example build_ig_profile_manifest -- \
//!   /path/to/package /path/to/profile-manifest.json [--absolute|--relative]
//! ```
//!
//! Defaults to **`--absolute`**. Use **`--relative`** for paths relative to the manifest file’s
//! parent directory (better for committing next to the IG tree; loaders must resolve paths from
//! the right working directory).
//!
//! The first path is the IG root (typically the `package` directory inside an unpacked `.tgz`, or
//! the folder that contains `StructureDefinition/`, `CodeSystem/`, and `ValueSet/`).

use std::path::Path;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let ig_root = args.next().expect(
        "usage: build_ig_profile_manifest <ig_root> <manifest_out.json> [--absolute|--relative]",
    );
    let manifest_out = args.next().expect(
        "usage: build_ig_profile_manifest <ig_root> <manifest_out.json> [--absolute|--relative]",
    );

    let mut style = fhir_validation::ProfileManifestPathStyle::Absolute;
    for a in args {
        match a.to_string_lossy().as_ref() {
            "--relative" => {
                style = fhir_validation::ProfileManifestPathStyle::RelativeToManifestParent
            }
            "--absolute" => style = fhir_validation::ProfileManifestPathStyle::Absolute,
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let ig_root = Path::new(&ig_root);
    let manifest_out = Path::new(&manifest_out);

    match fhir_validation::build_and_write_profile_manifest_for_ig(ig_root, manifest_out, style) {
        Ok(m) => {
            eprintln!(
                "Wrote {} (style={}, SD: {}, CodeSystem: {}, ValueSet: {})",
                manifest_out.display(),
                match style {
                    fhir_validation::ProfileManifestPathStyle::Absolute => "absolute",
                    fhir_validation::ProfileManifestPathStyle::RelativeToManifestParent => {
                        "relative_to_manifest_parent"
                    }
                },
                m.structure_definition_files.len(),
                m.code_system_files.len(),
                m.value_set_files.len()
            );
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
