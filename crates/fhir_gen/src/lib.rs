pub mod initial_fhir_model;

use crate::initial_fhir_model::Bundle;
use clap::ValueEnum;
use serde_json::Result;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, ValueEnum)]
pub enum FhirVersion {
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

pub fn process_fhir_version(version: FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");

    match version {
        FhirVersion::All => {
            // Process each version separately
            for ver in [
                FhirVersion::R4,
                FhirVersion::R4B,
                FhirVersion::R5,
                FhirVersion::R6,
            ] {
                if let Err(e) = process_fhir_version(ver.clone(), &output_path) {
                    eprintln!("Warning: Failed to process {:?}: {}", ver, e);
                }
            }
            Ok(())
        }
        specific_version => {
            let version_dir = match specific_version {
                FhirVersion::R4 => resources_dir.join("R4"),
                FhirVersion::R4B => resources_dir.join("R4B"),
                FhirVersion::R5 => resources_dir.join("R5"),
                FhirVersion::R6 => resources_dir.join("build"),
                FhirVersion::All => unreachable!(),
            };

            if !version_dir.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "FHIR resources directory not found for version {:?}: {}",
                        specific_version,
                        version_dir.display()
                    ),
                ));
            }

            // Create output directory if it doesn't exist
            std::fs::create_dir_all(&output_path)?;

            // Process all JSON files in the version directory
            visit_dirs(&version_dir)?
                .into_iter()
                .try_for_each::<_, io::Result<()>>(|file_path| {
                    match parse_structure_definitions(&file_path) {
                        Ok(bundle) => generate_code(bundle, &output_path)?,
                        Err(e) => {
                            eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e)
                        }
                    }
                    Ok(())
                })?;
            Ok(())
        }
    }
}

fn visit_dirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut json_files = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                json_files.extend(visit_dirs(&path)?);
            } else if let Some(ext) = path.extension() {
                if ext == "json" {
                    if let Some(filename) = path.file_name() {
                        let filename = filename.to_string_lossy();
                        if !filename.contains("conceptmap") && !filename.contains("valueset") {
                            json_files.push(path);
                        }
                    }
                }
            }
        }
    }
    Ok(json_files)
}

fn parse_structure_definitions<P: AsRef<Path>>(path: P) -> Result<Bundle> {
    let file = File::open(path).map_err(|e| serde_json::Error::io(e))?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}

fn generate_code(_bundle: Bundle, output_path: impl AsRef<Path>) -> io::Result<()> {
    // Create the output directory if it doesn't exist
    let output_path = output_path.as_ref();
    std::fs::create_dir_all(output_path)?;

    // TODO: Generate actual code from the bundle
    // For now just write a placeholder file to verify the path works
    let placeholder_path = output_path.join("placeholder.rs");
    std::fs::write(placeholder_path, "// TODO: Generated code will go here\n")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use initial_fhir_model::Resource;
    use std::path::PathBuf;

    // Add a test for process_fhir_version AI!

    #[test]
    fn test_parse_structure_definitions() {
        let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
        let json_files = visit_dirs(&resources_dir).expect("Failed to read resource directory");
        assert!(
            !json_files.is_empty(),
            "No JSON files found in resources directory"
        );

        for file_path in json_files {
            match parse_structure_definitions(&file_path) {
                Ok(bundle) => {
                    // Verify that we have something
                    assert!(
                        !bundle.entry.is_none(),
                        "Bundle entry is None for file: {}",
                        file_path.display()
                    );

                    // Verify we have the expected type definitions
                    assert!(
                        bundle.entry.unwrap().iter().any(|e| {
                            if let Some(resource) = &e.resource {
                                matches!(
                                    resource,
                                    Resource::StructureDefinition(_)
                                        | Resource::SearchParameter(_)
                                        | Resource::OperationDefinition(_)
                                )
                            } else {
                                false
                            }
                        }),
                        "No expected resource types found in file: {}",
                        file_path.display()
                    );
                }
                Err(e) => {
                    panic!("Failed to parse bundle {}: {:?}", file_path.display(), e);
                }
            }
        }
    }
}
