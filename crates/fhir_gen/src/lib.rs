pub mod initial_fhir_model;

use crate::initial_fhir_model::{Bundle, ElementDefinitionType, Resource};
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

fn is_valid_structure_definition(def: &StructureDefinition) -> bool {
    (def.kind == "complex-type" || def.kind == "primitive-type" || def.kind == "resource")
        && def.derivation.as_deref() == Some("specialization")
        && def.r#abstract == false
}

fn generate_code(bundle: Bundle, output_path: impl AsRef<Path>) -> io::Result<()> {
    // First collect all ElementDefinitions across all StructureDefinitions
    // Also collect all Resource names
    let mut all_elements = Vec::new();
    let mut all_resources = Vec::new();

    if let Some(entries) = bundle.entry.as_ref() {
        // First pass: collect all elements
        for entry in entries {
            if let Some(resource) = &entry.resource {
                if let Resource::StructureDefinition(def) = resource {
                    if is_valid_structure_definition(def) {
                        if let Some(snapshot) = &def.snapshot {
                            if let Some(elements) = &snapshot.element {
                                all_elements.extend(elements.iter().map(|e| e));
                            }
                        }
                        if def.kind == "resource" && def.r#abstract == false {
                            all_resources.push(&def.name);
                        }
                    }
                }
            }
        }

        // Detect cycles using all collected elements
        let element_refs: Vec<&ElementDefinition> = all_elements;
        let cycles = detect_struct_cycles(&element_refs);

        // Second pass: generate code
        for entry in entries {
            if let Some(resource) = &entry.resource {
                match resource {
                    Resource::StructureDefinition(def) => {
                        if is_valid_structure_definition(def) {
                            let content = structure_definition_to_rust(def, &cycles);
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

        // Finally, generate the Resource enum
        if !all_resources.is_empty() {
            let resource_enum =
                generate_resource_enum(all_resources.iter().map(|s| s.to_string()).collect());
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(output_path.as_ref())?;
            writeln!(file, "{}", resource_enum)?;
        }
    }

    Ok(())
}

fn generate_resource_enum(resources: Vec<String>) -> String {
    let mut output = String::new();
    output.push_str("#[derive(Debug, Serialize, Deserialize)]\n");
    output.push_str("#[serde(tag = \"resourceType\")]\n");
    output.push_str("pub enum Resource {\n");

    for resource in resources {
        output.push_str(&format!("    {}({}),\n", resource, resource));
    }

    output.push_str("}\n\n");
    output
}

fn make_rust_safe(input: &str) -> String {
    let snake_case = input
        .chars()
        .enumerate()
        .fold(String::new(), |mut acc, (i, c)| {
            if i > 0 && c.is_uppercase() {
                acc.push('_');
            }
            acc.push(c.to_lowercase().next().unwrap());
            acc
        });

    match snake_case.as_str() {
        "type" | "use" | "abstract" | "for" | "ref" => format!("r#{}", snake_case),
        _ => snake_case,
    }
}

fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

fn structure_definition_to_rust(
    sd: &StructureDefinition,
    cycles: &std::collections::HashSet<(String, String)>,
) -> String {
    let mut output = String::new();
    // Process elements
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            let mut processed_types = std::collections::HashSet::new();
            process_elements(elements, &mut output, &mut processed_types, cycles);
        }
    }
    output
}

fn detect_struct_cycles(
    elements: &Vec<&ElementDefinition>,
) -> std::collections::HashSet<(String, String)> {
    let mut cycles = std::collections::HashSet::new();
    let mut graph: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    // Build direct dependencies where max=1
    for element in elements {
        if let Some(types) = &element.r#type {
            let path_parts: Vec<&str> = element.path.split('.').collect();
            if path_parts.len() > 1 {
                let from_type = path_parts[0].to_string();
                if !from_type.is_empty() && element.max.as_ref().map(|m| m.as_str()) == Some("1") {
                    for ty in types {
                        if !ty.code.contains('.') && from_type != ty.code {
                            graph
                                .entry(from_type.clone())
                                .or_default()
                                .push(ty.code.clone());
                        }
                    }
                }
            }
        }
    }

    // Find cycles between exactly two structs
    for (from_type, deps) in &graph {
        for to_type in deps {
            if let Some(back_deps) = graph.get(to_type) {
                if back_deps.contains(from_type) {
                    // We found a cycle between exactly two structs
                    cycles.insert((from_type.clone(), to_type.clone()));
                }
            }
        }
    }

    cycles
}

fn process_elements(
    elements: &[ElementDefinition],
    output: &mut String,
    processed_types: &mut std::collections::HashSet<String>,
    cycles: &std::collections::HashSet<(String, String)>,
) {
    // Group elements by their parent path
    let mut element_groups: std::collections::HashMap<String, Vec<&ElementDefinition>> =
        std::collections::HashMap::new();

    // First pass - collect all type names that will be generated
    for element in elements {
        let path_parts: Vec<&str> = element.path.split('.').collect();
        if path_parts.len() > 1 {
            let parent_path = path_parts[..path_parts.len() - 1].join(".");
            println!(
                "Parent path: {}, element.path: {}",
                parent_path, &element.path
            );
            element_groups.entry(parent_path).or_default().push(element);
        }
    }

    // Process each group
    for (path, group) in element_groups {
        let type_name = generate_type_name(&path);

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
                    generate_element_definition(element, &type_name, output, cycles, elements);
                } else {
                    let mut choice_fields: Vec<ElementDefinition> = vec![];
                    if let Some(types) = &element.r#type {
                        for choice_type in types {
                            let choice_type_type = capitalize_first_letter(&choice_type.code);
                            let mut new_choice_type = ElementDefinition::default();
                            new_choice_type.id = element.id.clone().map(|id| {
                                let mut s = id.trim_end_matches("[x]").to_string();
                                s.push_str(&choice_type_type);
                                s
                            });
                            new_choice_type.path =
                                element.path.clone().trim_end_matches("[x]").to_string();
                            new_choice_type.path.push_str(&choice_type_type);
                            new_choice_type.short = element.short.clone();
                            new_choice_type.definition = element.definition.clone();
                            new_choice_type.min = element.min;
                            new_choice_type.max = element.max.clone();
                            let edt = ElementDefinitionType::new(choice_type.code.clone());
                            new_choice_type.r#type = Some(vec![edt]);
                            choice_fields.push(new_choice_type);
                        }
                    }
                    for choice_field in choice_fields {
                        generate_element_definition(
                            &choice_field,
                            &type_name,
                            output,
                            cycles,
                            elements,
                        );
                    }
                }
            }
        }
        output.push_str("}\n\n");
    }
}

fn generate_element_definition(
    element: &ElementDefinition,
    type_name: &String,
    output: &mut String,
    cycles: &std::collections::HashSet<(String, String)>,
    elements: &[ElementDefinition],
) {
    if let Some(field_name) = element.path.split('.').last() {
        let rust_field_name = make_rust_safe(field_name);

        if field_name != rust_field_name {
            output.push_str(&format!("    #[serde(rename = \"{}\")]\n", field_name));
        }

        let ty = element.r#type.as_ref().and_then(|t| t.first());
        if ty.is_none() {
            if let Some(content_ref) = &element.content_reference {
                if content_ref.starts_with('#') {
                    let ref_id = &content_ref[1..];
                    if let Some(referenced_element) = elements
                        .iter()
                        .find(|e| e.id.as_ref().map_or(false, |id| id == ref_id))
                    {
                        if let Some(ref_ty) =
                            referenced_element.r#type.as_ref().and_then(|t| t.first())
                        {
                            let is_array = element.max.as_deref() == Some("*");
                            let base_type = match ref_ty.code.as_str() {
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
                                    &generate_type_name(&referenced_element.path)
                                }
                                _ => &capitalize_first_letter(&ref_ty.code),
                            };

                            let type_str = if is_array {
                                format!("Option<Vec<{}>>", base_type)
                            } else if element.min.unwrap_or(0) == 0 {
                                format!("Option<{}>", base_type)
                            } else {
                                base_type.to_string()
                            };

                            output
                                .push_str(&format!("    pub {}: {},\n", rust_field_name, type_str));
                            return;
                        }
                    }
                }
            }
            // If we get here, either there was no content_reference or we couldn't resolve it
            output.push_str(&format!("    pub {}: Option<String>,\n", rust_field_name));
            return;
        }
        {
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
                "Element" | "BackboneElement" => &generate_type_name(&element.path),
                _ => &capitalize_first_letter(&ty.code),
            };

            let base_type = if let Some(content_ref) = &element.content_reference {
                if content_ref.starts_with('#') {
                    generate_type_name(&content_ref[1..])
                } else {
                    base_type.to_string()
                }
            } else {
                base_type.to_string()
            };

            let mut type_str = if field_name.ends_with("[x]") {
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

            // Add Box<> to break cycles (only to the "to" type in the cycle)
            if let Some(field_type) = element.r#type.as_ref().and_then(|t| t.first()) {
                let from_type = element.path.split('.').next().unwrap_or("");
                if !from_type.is_empty() {
                    for (cycle_from, cycle_to) in cycles.iter() {
                        if cycle_from == from_type && cycle_to == &field_type.code {
                            // Add Box<> around the type, preserving Option if present
                            if type_str.starts_with("Option<") {
                                type_str =
                                    format!("Option<Box<{}>>", &type_str[7..type_str.len() - 1]);
                            } else {
                                type_str = format!("Box<{}>", type_str);
                            }
                            break;
                        }
                    }
                }
            }
            output.push_str(&format!("    pub {}: {},\n", rust_field_name, type_str));
        }
    }
}

fn generate_type_name(path: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    if !parts.is_empty() {
        let mut result = String::from(parts[0]);
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
        let result = String::from("Empty path provided to generate_type_name");
        result
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
    fn test_detect_struct_cycles() {
        let elements = vec![
            ElementDefinition {
                path: "Identifier".to_string(),
                ..Default::default()
            },
            ElementDefinition {
                path: "Identifier.assigner".to_string(),
                r#type: Some(vec![initial_fhir_model::ElementDefinitionType::new(
                    "Reference".to_string(),
                )]),
                max: Some("1".to_string()),
                ..Default::default()
            },
            ElementDefinition {
                path: "Reference".to_string(),
                ..Default::default()
            },
            ElementDefinition {
                path: "Reference.identifier".to_string(),
                r#type: Some(vec![initial_fhir_model::ElementDefinitionType::new(
                    "Identifier".to_string(),
                )]),
                max: Some("1".to_string()),
                ..Default::default()
            },
            ElementDefinition {
                path: "Patient".to_string(),
                r#type: Some(vec![initial_fhir_model::ElementDefinitionType::new(
                    "Resource".to_string(),
                )]),
                ..Default::default()
            },
            ElementDefinition {
                path: "Extension".to_string(),
                ..Default::default()
            },
            ElementDefinition {
                path: "Extension.extension".to_string(),
                r#type: Some(vec![initial_fhir_model::ElementDefinitionType::new(
                    "Extension".to_string(),
                )]),
                max: Some("*".to_string()),
                ..Default::default()
            },
            ElementDefinition {
                path: "Base64Binary".to_string(),
                ..Default::default()
            },
            ElementDefinition {
                path: "Base64Binary.extension".to_string(),
                r#type: Some(vec![initial_fhir_model::ElementDefinitionType::new(
                    "Extension".to_string(),
                )]),
                max: Some("*".to_string()),
                ..Default::default()
            },
        ];

        let element_refs: Vec<&ElementDefinition> = elements.iter().collect();
        let cycles = detect_struct_cycles(&element_refs);

        // Should detect the Identifier <-> Reference cycle with both sides have max="1"
        // cardinality
        assert!(
            cycles.contains(&("Identifier".to_string(), "Reference".to_string()))
                || cycles.contains(&("Reference".to_string(), "Identifier".to_string()))
        );

        // Should not detect Patient -> Resource as a cycle (one-way dependency)
        assert!(!cycles.contains(&("Patient".to_string(), "Resource".to_string())));
        assert!(!cycles.contains(&("Resource".to_string(), "Patient".to_string())));

        // Should also not detect self cycles - these are ok
        assert!(!cycles.contains(&("Extension".to_string(), "Extension".to_string())));

        // This is ok too because it is a one to many relationship.
        assert!(!cycles.contains(&("Base64Binary".to_string(), "Extension".to_string())));
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
