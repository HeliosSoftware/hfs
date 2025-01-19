pub mod initial_fhir_model;

use crate::initial_fhir_model::{Bundle, Resource};
use clap::ValueEnum;
use initial_fhir_model::StructureDefinition;
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

impl ToString for FhirVersion {
    fn to_string(&self) -> String {
        match self {
            FhirVersion::R4 => "R4".to_string(),
            FhirVersion::R4B => "R4B".to_string(),
            FhirVersion::R5 => "R5".to_string(),
            FhirVersion::R6 => "R6".to_string(),
            FhirVersion::All => "ALL".to_string(),
        }
    }
}

impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

fn process_single_version(
    version: &FhirVersion,
    base_output_path: impl AsRef<Path>,
) -> io::Result<()> {
    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");

    let generated_path = base_output_path.as_ref().join("generated");
    let (version_dir, output_path) = match version {
        FhirVersion::R4 => (resources_dir.join("R4"), generated_path.join("R4")),
        FhirVersion::R4B => (resources_dir.join("R4B"), generated_path.join("R4B")),
        FhirVersion::R5 => (resources_dir.join("R5"), generated_path.join("R5")),
        FhirVersion::R6 => (resources_dir.join("build"), generated_path.join("R6")),
        FhirVersion::All => return Ok(()), // Skip All variant
    };

    if !version_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "FHIR resources directory not found for version {:?}: {}",
                version,
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
                Ok(bundle) => generate_code(bundle, &output_path, version)?,
                Err(e) => {
                    eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e)
                }
            }
            Ok(())
        })?;
    Ok(())
}

pub fn process_fhir_version(version: FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
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
            }
            Ok(())
        }
        specific_version => process_single_version(&specific_version, output_path),
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

fn generate_code(
    _bundle: Bundle,
    output_path: impl AsRef<Path>,
    version: &FhirVersion,
) -> io::Result<()> {
    // Create the output directory if it doesn't exist
    let output_path = output_path.as_ref();
    std::fs::create_dir_all(output_path)?;

    let mut generated_modules = Vec::new();

    // Process each entry in the bundle
    if let Some(entries) = _bundle.entry.as_ref() {
        for entry in entries {
            if let Some(resource) = &entry.resource {
                match resource {
                    Resource::StructureDefinition(def) => {
                        // Only process complex-type and primitive-type definitions
                        if def.kind == "complex-type" || def.kind == "primitive-type" {
                            if let Some(id) = &def.id {
                                let file_name = format!("{}.rs", id);
                                let file_path = output_path.join(&file_name);
                                std::fs::write(file_path, structure_definition_to_rust_file(def))?;
                                generated_modules.push(id);
                            }
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

    // Generate lib.rs with module declarations and use statements
    let mut lib_content = String::new();
    lib_content.push_str(&format!("mod {};\n\n", version.to_string()));

    println!("Generated modules size: {}", generated_modules.len());
    if !generated_modules.is_empty() {
        println!(
            "First few modules: {:?}",
            &generated_modules[..generated_modules.len().min(3)]
        );
    }

    // generated_modules has a non zero length, but lib_content.push_str("hi"); is not getting
    // called AI!

    // Add use statements
    for module in generated_modules {
        let use_statement = format!("pub use {}::*;\n", module);
        lib_content.push_str(&use_statement);
        lib_content.push_str("hi");
    }
    lib_content.push_str("hi2");

    // Write lib.rs
    std::fs::write(output_path.join("lib.rs"), lib_content)?;

    Ok(())
}

fn make_rust_safe(input: &str) -> String {
    match input {
        "type" | "use" | "abstract" => format!("r#{}", input),
        _ => input.to_string(),
    }
}

fn structure_definition_to_rust_file(sd: &StructureDefinition) -> String {
    let mut output = String::new();
    let mut enums_to_add = Vec::new();

    // Add imports
    output.push_str("use serde::{Serialize, Deserialize};\n\n");

    // Add struct definition
    output.push_str("#[derive(Debug, Serialize, Deserialize)]\n");
    output.push_str(&format!("pub struct {} {{\n", sd.name));

    // Add fields from snapshot or differential
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            for element in elements.iter().skip(1) {
                // Skip first element as it's the root
                if let Some(ty) = element.r#type.as_ref().and_then(|t| t.first()) {
                    let field_name = element.path.split('.').last().unwrap_or("unknown");
                    let rust_field_name = make_rust_safe(field_name);

                    // Handle array types
                    let is_array = element.max.as_deref() == Some("*");
                    let base_type = match ty.code.as_str() {
                        "http://hl7.org/fhirpath/System.Boolean" => "bool",
                        "http://hl7.org/fhirpath/System.String" => "String",
                        "http://hl7.org/fhirpath/System.Integer" => "i32",
                        "http://hl7.org/fhirpath/System.Long" => "i64",
                        "http://hl7.org/fhirpath/System.Decimal" => "f64",
                        "http://hl7.org/fhirpath/System.DateTime" => "String",
                        "http://hl7.org/fhirpath/System.Time" => "String",
                        "http://hl7.org/fhirpath/System.Quantity" => "String",
                        _ => ty.code.as_str(), // Use the type name directly for complex types
                    };

                    // Check if this is a choice type field
                    if element.path.ends_with("[x]") {
                        // Generate enum name from base path
                        let base_path = element.path.trim_end_matches("[x]");

                        // Generate enum name from struct name and field
                        let enum_name = format!(
                            "{}{}",
                            sd.name,
                            base_path
                                .split('.')
                                .last()
                                .unwrap_or("Unknown")
                                .chars()
                                .next()
                                .unwrap()
                                .to_uppercase()
                                .chain(
                                    base_path
                                        .split('.')
                                        .last()
                                        .unwrap_or("Unknown")
                                        .chars()
                                        .skip(1)
                                )
                                .collect::<String>()
                        );

                        let mut enum_def = String::new();
                        enum_def.push_str("#[derive(Debug, Serialize, Deserialize)]\n");
                        enum_def.push_str("#[serde(rename_all = \"camelCase\")]\n");
                        enum_def.push_str(&format!("pub enum {} {{\n", enum_name));

                        if let Some(types) = &element.r#type {
                            for ty in types {
                                let variant_name = ty
                                    .code
                                    .chars()
                                    .next()
                                    .unwrap()
                                    .to_uppercase()
                                    .chain(ty.code.chars().skip(1))
                                    .collect::<String>();
                                let type_str = match ty.code.as_str() {
                                    "string" => "String",
                                    "boolean" => "bool",
                                    "integer" => "i32",
                                    "decimal" => "String",
                                    "Reference" => "Reference",
                                    _ => &ty.code,
                                };
                                enum_def
                                    .push_str(&format!("    {}({}),\n", variant_name, type_str));
                            }
                        }
                        enum_def.push_str("}\n\n");

                        // Store enum definition to be added after struct
                        enums_to_add.push(enum_def);

                        // Add the field using the enum type
                        let field_name = base_path.split('.').last().unwrap_or("unknown");
                        let rust_field_name = make_rust_safe(field_name);
                        let type_str = if element.min.unwrap_or(0) == 0 {
                            format!("Option<{}>", enum_name)
                        } else {
                            enum_name
                        };
                        output.push_str(&format!("    pub {}: {},\n", rust_field_name, type_str));
                    } else {
                        // Add field with proper optionality and array handling
                        if field_name != rust_field_name {
                            output.push_str("    #[serde(rename = \"");
                            output.push_str(field_name);
                            output.push_str("\")]\n");
                        }

                        let type_str = if is_array {
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
    }

    output.push_str("}\n\n");

    // Add all the enums after the struct definition
    for enum_def in enums_to_add {
        output.push_str(&enum_def);
    }

    output
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

        // Verify placeholder file was created
        let placeholder_path = temp_dir.join("placeholder.rs");
        assert!(placeholder_path.exists());

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
