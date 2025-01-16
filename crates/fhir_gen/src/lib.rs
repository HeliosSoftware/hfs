pub mod initial_fhir_model;

use serde_json::Result;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::initial_fhir_model::Bundle;
use std::io;

pub fn parse_structure_definitions<P: AsRef<Path>>(path: P) -> Result<Bundle> {
    let file = File::open(path).expect("File not found");
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}

pub fn generate_code(bundle: Bundle, output_path: impl AsRef<Path>) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use initial_fhir_model::Resource;
    use std::path::PathBuf;

    #[test]
    fn test_parse_structure_definitions() {
        let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");

        fn visit_dirs(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
            let mut json_files = Vec::new();
            if dir.is_dir() {
                for entry in std::fs::read_dir(dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() {
                        json_files.extend(visit_dirs(&path)?);
                    } else if let Some(ext) = path.extension() {
                        if ext == "json"
                            && !path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .contains("conceptmap")
                            && !path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .contains("valueset")
                        {
                            json_files.push(path);
                        }
                    }
                }
            }
            Ok(json_files)
        }

        let json_files = visit_dirs(&resources_dir).expect("Failed to read resource directory");
        assert!(
            !json_files.is_empty(),
            "No JSON files found in resources directory"
        );

        for file_path in json_files {
            println!("Testing file: {}", file_path.display());
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
