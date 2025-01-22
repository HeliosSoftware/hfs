pub mod initial_fhir_model;

use crate::initial_fhir_model::{Bundle, Resource};
use clap::ValueEnum;
use initial_fhir_model::ElementDefinition;
use initial_fhir_model::StructureDefinition;
use serde_json::Result;
use std::fs::File;
use std::io::BufReader;
use std::io::{self, Write};
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

impl ToString for FhirVersion {
    fn to_string(&self) -> String {
        match self {
            FhirVersion::R4 => "r4".to_string(),
            FhirVersion::R4B => "r4b".to_string(),
            FhirVersion::R5 => "r5".to_string(),
            FhirVersion::R6 => "r6".to_string(),
            FhirVersion::All => "all".to_string(),
        }
    }
}

fn process_single_version(version: &FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let version_dir = resources_dir.join(version.to_string());
    // Create output directory if it doesn't exist
    std::fs::create_dir_all(output_path.as_ref())?;

    let version_path = output_path
        .as_ref()
        .join(&format!("{}.rs", version.to_string()));

    // Create the version-specific output file with initial content
    std::fs::write(&version_path, "use serde::{Serialize, Deserialize};\n\n")?;

    // Process all JSON files in the resources/{FhirVersion} directory
    visit_dirs(&version_dir)?
        .into_iter()
        .try_for_each::<_, io::Result<()>>(|file_path| {
            match parse_structure_definitions(&file_path) {
                Ok(bundle) => generate_code(bundle, &version_path)?,
                Err(e) => {
                    eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e)
                }
            }
            Ok(())
        })?;

    Ok(())
}

pub fn process_fhir_version(version: FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
    let mut lib_content = String::new();

    match version {
        FhirVersion::All => {
            // Process each version separately
            for ver in [
                FhirVersion::R4,
                FhirVersion::R4B,
                FhirVersion::R5,
                FhirVersion::R6,
            ] {
                if let Err(e) = process_single_version(&ver, &output_path) {
                    eprintln!("Warning: Failed to process {:?}: {}", ver, e);
                }
                lib_content.push_str(&format!("pub mod {};\n", ver.to_string()));
            }
            std::fs::write(output_path.as_ref().join("lib.rs"), lib_content)?;
            Ok(())
        }
        specific_version => {
            lib_content.push_str(&format!("pub mod {};\n", specific_version.to_string()));
            std::fs::write(output_path.as_ref().join("lib.rs"), lib_content)?;
            process_single_version(&specific_version, output_path)
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
    // Process each entry in the bundle
    if let Some(entries) = _bundle.entry.as_ref() {
        for entry in entries {
            if let Some(resource) = &entry.resource {
                match resource {
                    Resource::StructureDefinition(def) => {
                        // Skip constraint derivations, only work with specializations and only process base types
                        if (def.kind == "complex-type" || def.kind == "primitive-type")
                            && def.derivation.as_deref() == Some("specialization")
                            && def.r#abstract == false
                        {
                            let content = structure_definition_to_rust(def);
                            // Append the content to the version-specific file
                            let mut file = std::fs::OpenOptions::new()
                                .create(true)
                                .write(true)
                                .append(true)
                                .open(output_path.as_ref())?;
                            writeln!(file, "{}", content)?;
                        }
                    }
                    Resource::SearchParameter(_param) => {
                        // TODO: Generate code for search parameter
                    }
                    Resource::OperationDefinition(_op) => {
                        // TODO: Generate code for operation definition
                    }
                    _ => {} // Skip other resource types for now
                }
            }
        }
    }

    Ok(())
}

fn make_rust_safe(input: &str) -> String {
    // Also handle the conversion to snake_case here AI!
    match input {
        "type" | "use" | "abstract" => format!("r#{}", input),
        _ => input.to_string(),
    }
}

fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

fn structure_definition_to_rust(sd: &StructureDefinition) -> String {
    let mut output = String::new();
    // Process elements
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            let mut processed_types = std::collections::HashSet::new();
            process_elements(elements, &mut output, &mut processed_types, &sd.name);
        }
    }
    output
}

fn process_elements(
    elements: &[ElementDefinition],
    output: &mut String,
    processed_types: &mut std::collections::HashSet<String>,
    base_name: &str,
) {
    // Group elements by their parent path
    let mut element_groups: std::collections::HashMap<String, Vec<&ElementDefinition>> =
        std::collections::HashMap::new();

    // First pass - collect all type names that will be generated
    for element in elements {
        let path_parts: Vec<&str> = element.path.split('.').collect();
        let parent_path = if path_parts.len() == 1 {
            path_parts[0].to_string()
        } else {
            path_parts[..path_parts.len() - 1].join(".")
        };
        element_groups.entry(parent_path).or_default().push(element);
    }

    // Process each group
    for (path, group) in element_groups {
        let type_name = generate_type_name(&path, base_name);

        // Skip if we've already processed this type
        if processed_types.contains(&type_name) {
            continue;
        }

        processed_types.insert(type_name.clone());

        // Process choice types first
        let choice_fields: Vec<_> = group.iter().filter(|e| e.path.ends_with("[x]")).collect();
        for choice in choice_fields {
            let enum_name = format!(
                "{}{}",
                type_name,
                choice
                    .path
                    .split('.')
                    .last()
                    .unwrap()
                    .trim_end_matches("[x]")
                    .chars()
                    .next()
                    .unwrap()
                    .to_uppercase()
                    .chain(
                        choice
                            .path
                            .split('.')
                            .last()
                            .unwrap()
                            .trim_end_matches("[x]")
                            .chars()
                            .skip(1)
                    )
                    .collect::<String>()
            );

            // Skip if we've already processed this enum
            if processed_types.contains(&enum_name) {
                continue;
            }
            processed_types.insert(enum_name.clone());
            output.push_str("#[derive(Debug, Serialize, Deserialize)]\n");
            output.push_str("#[serde(rename_all = \"camelCase\")]\n");
            output.push_str(&format!("pub enum {} {{\n", enum_name));

            if let Some(types) = &choice.r#type {
                for ty in types {
                    output.push_str(&format!(
                        "    {}({}),\n",
                        capitalize_first_letter(&ty.code),
                        capitalize_first_letter(&ty.code)
                    ));
                }
            }
            output.push_str("}\n\n");
        }

        // Generate struct
        output.push_str("#[derive(Debug, Serialize, Deserialize)]\n");
        output.push_str(&format!(
            "pub struct {} {{\n",
            capitalize_first_letter(&type_name)
        ));

        for element in &group {
            if let Some(field_name) = element.path.split('.').last() {
                if !field_name.contains("[x]") {
                    let rust_field_name = make_rust_safe(field_name);

                    if field_name != rust_field_name {
                        output.push_str(&format!("    #[serde(rename = \"{}\")]\n", field_name));
                    }

                    if let Some(ty) = element.r#type.as_ref().and_then(|t| t.first()) {
                        let is_array = element.max.as_deref() == Some("*");
                        let base_type = match ty.code.as_str() {
                            // https://build.fhir.org/fhirpath.html#types
                            "http://hl7.org/fhirpath/System.Boolean" => "bool",
                            "http://hl7.org/fhirpath/System.String" => "std::string::String",
                            "http://hl7.org/fhirpath/System.Integer" => "std::primitive::i32",
                            "http://hl7.org/fhirpath/System.Long" => "std::primitive::i64",
                            "http://hl7.org/fhirpath/System.Decimal" => "std::primitive::f64",
                            "http://hl7.org/fhirpath/System.Date" => "std::string::String",
                            "http://hl7.org/fhirpath/System.DateTime" => "std::string::String",
                            "http://hl7.org/fhirpath/System.Time" => "std::string::String",
                            "http://hl7.org/fhirpath/System.Quantity" => "std::string::String",
                            "Element" | "BackboneElement" => {
                                &generate_type_name(&element.path, &base_name)
                            }
                            _ => &capitalize_first_letter(&ty.code),
                        };

                        let type_str = if field_name.ends_with("[x]") {
                            let base_name = field_name.trim_end_matches("[x]");
                            let enum_name = format!(
                                "{}{}",
                                type_name,
                                base_name
                                    .chars()
                                    .next()
                                    .unwrap()
                                    .to_uppercase()
                                    .chain(base_name.chars().skip(1))
                                    .collect::<String>()
                            );
                            format!("Option<{}>", enum_name)
                        } else if is_array {
                            format!("Option<Vec<{}>>", base_type)
                        } else if element.min.unwrap_or(0) == 0 {
                            format!("Option<{}>", base_type)
                        } else {
                            base_type.to_string()
                        };

                        output.push_str(&format!("    pub {}: {},\n", rust_field_name, type_str));
                    }
                }
            }
        }
        output.push_str("}\n\n");
    }
}

fn generate_type_name(path: &str, base_name: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    if parts[0] == base_name {
        // For nested types, combine all parts including the base name
        let mut result = base_name.to_string();
        for part in &parts[1..] {
            result.push_str(
                &part
                    .chars()
                    .next()
                    .unwrap()
                    .to_uppercase()
                    .chain(part.chars().skip(1))
                    .collect::<String>(),
            );
        }
        result
    } else {
        // For root type, use as is with capitalized first letter
        path.chars()
            .next()
            .unwrap_or_default()
            .to_uppercase()
            .chain(path.chars().skip(1))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use initial_fhir_model::Resource;
    use std::path::PathBuf;

    #[test]
    fn test_process_fhir_version() {
        // Create a temporary directory for test output
        let temp_dir = std::env::temp_dir().join("fhir_gen_test");
        std::fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");

        // Test processing R4 version
        assert!(process_fhir_version(FhirVersion::R4, &temp_dir).is_ok());

        // Verify files were created
        assert!(temp_dir.join("r4.rs").exists());
        assert!(temp_dir.join("lib.rs").exists());

        // Clean up
        std::fs::remove_dir_all(&temp_dir).expect("Failed to clean up temp directory");
    }

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
                    if bundle.entry.is_none() {
                        println!(
                            "Warning: Bundle entry is None for file: {}",
                            file_path.display()
                        );
                        continue;
                    }

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
