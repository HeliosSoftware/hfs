pub mod initial_fhir_model;

use crate::initial_fhir_model::{Bundle, Resource};
use fhir::FhirVersion;
use initial_fhir_model::ElementDefinition;
use initial_fhir_model::StructureDefinition;
use serde_json::Result;
use std::fs::File;
use std::io::BufReader;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;

fn process_single_version(version: &FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let version_dir = resources_dir.join(version.as_str());
    // Create output directory if it doesn't exist
    std::fs::create_dir_all(output_path.as_ref())?;

    let version_path = output_path
        .as_ref()
        .join(&format!("{}.rs", version.as_str().to_lowercase()));

    // Create the version-specific output file with initial content
    std::fs::write(
        &version_path,
        "use serde::{Serialize, Deserialize};\nuse crate::{Element, DecimalElement};\n\n",
    )?;

    let mut all_resource_names = std::collections::HashSet::new(); // Use HashSet for automatic deduplication

    // Process all JSON files in the resources/{FhirVersion} directory
    visit_dirs(&version_dir)?
        .into_iter()
        .try_for_each::<_, io::Result<()>>(|file_path| {
            match parse_structure_definitions(&file_path) {
                // Pass a mutable reference to collect resource names
                Ok(bundle) => generate_code(bundle, &version_path, &mut all_resource_names)?,
                Err(e) => {
                    eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e)
                }
            }
            Ok(())
        })?;

    // --- Generate Resource enum once after processing all files ---
    // Use the collected resource names. Sort them for consistent output.
    let mut sorted_resource_names: Vec<String> = all_resource_names.into_iter().collect();
    sorted_resource_names.sort(); // Sort alphabetically
    let resource_enum_code = generate_resource_enum(sorted_resource_names);

    // Append the single Resource enum definition to the file
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&version_path)?;
    write!(file, "{}", resource_enum_code)?;
    // --- End Resource enum generation ---

    Ok(())
}

pub fn process_fhir_version(
    version: Option<FhirVersion>,
    output_path: impl AsRef<Path>,
) -> io::Result<()> {
    match version {
        None => {
            // Process all versions
            for ver in [
                #[cfg(feature = "R4")]
                FhirVersion::R4,
                #[cfg(feature = "R4B")]
                FhirVersion::R4B,
                #[cfg(feature = "R5")]
                FhirVersion::R5,
                #[cfg(feature = "R6")]
                FhirVersion::R6,
            ] {
                if let Err(e) = process_single_version(&ver, &output_path) {
                    eprintln!("Warning: Failed to process {:?}: {}", ver, e);
                }
            }
            Ok(())
        }
        Some(specific_version) => process_single_version(&specific_version, output_path),
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

fn is_primitive_type(def: &StructureDefinition) -> bool {
    def.kind == "primitive-type"
}

fn generate_code(
    bundle: Bundle,
    output_path: impl AsRef<Path>,
    resource_names: &mut std::collections::HashSet<String>, // Add parameter to collect resource names
) -> io::Result<()> {
    // Signature already returns io::Result<()>
    let mut all_elements: Vec<&ElementDefinition> = Vec::new(); // Store references
    let mut generated_structs_code = String::new(); // Accumulate struct code

    if let Some(entries) = bundle.entry.as_ref() {
        // First pass: collect elements for ALL valid types
        for entry in entries {
            if let Some(resource) = &entry.resource {
                if let Resource::StructureDefinition(def) = resource {
                    // Consider all valid structure definitions
                    if is_valid_structure_definition(def) {
                        if let Some(snapshot) = &def.snapshot {
                            if let Some(elements) = &snapshot.element {
                                // Store references to elements
                                all_elements.extend(elements.iter());
                            }
                        }
                        // We will handle the Resource enum generation separately below
                    }
                }
            }
        }

        // Detect cycles using the collected element references
        let element_refs: Vec<&ElementDefinition> = all_elements.clone(); // Clone references for cycle detection
        let cycles = detect_struct_cycles(&element_refs);

        // Second pass: generate struct/type code for ALL valid types
        for entry in entries {
            if let Some(resource) = &entry.resource {
                match resource {
                    Resource::StructureDefinition(def) => {
                        // Generate code for all valid structure definitions
                        if is_valid_structure_definition(def) {
                            // Collect resource names (if it's a resource type)
                            if def.kind == "resource" {
                                resource_names.insert(def.name.clone());
                            }
                            // Pass references to all elements for context
                            let content = structure_definition_to_rust(def, &cycles, &all_elements);
                            generated_structs_code.push_str(&content); // Append to buffer
                            generated_structs_code.push('\n'); // Add newline between definitions
                        }
                    }
                    // Skip SearchParameter, OperationDefinition, etc.
                    _ => {}
                }
            }
        }

        // --- REMOVED Resource enum generation from here ---

        // Append the generated struct/type code for this bundle to the file
        let mut file = std::fs::OpenOptions::new()
            .append(true) // Append to the file opened in process_single_version
            .open(output_path.as_ref())?;
        write!(file, "{}", generated_structs_code)?; // Write the accumulated struct code
    }

    Ok(()) // Return Ok(()) as per the new signature
}

fn generate_resource_enum(resources: Vec<String>) -> String {
    let mut output = String::new();
    // Add PartialEq here
    output.push_str("#[derive(Debug, Serialize, Deserialize, PartialEq)]\n");
    output.push_str("#[serde(tag = \"resourceType\")]\n");
    output.push_str("pub enum Resource {\n");

    for resource in resources {
        // Apply Box for recursive variants like Bundle and Parameters
        if resource == "Bundle" || resource == "Parameters" {
            output.push_str(&format!("    {}(Box<{}>),\n", resource, resource));
        } else {
            output.push_str(&format!("    {}({}),\n", resource, resource));
        }
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
        "type" | "use" | "abstract" | "for" | "ref" | "const" => format!("r#{}", snake_case),
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
    all_allowed_elements: &[&ElementDefinition], // Pass all relevant elements for context
) -> String {
    let mut output = String::new();
    let mut backbone_structs_code = String::new(); // Buffer for backbone structs

    // Handle primitive types differently
    if is_primitive_type(sd) {
        return generate_primitive_type(sd);
    }

    // Process elements for complex types and resources
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            // --- First Pass: Generate BackboneElement Structs ---
            let mut processed_backbone_paths = std::collections::HashSet::new();
            for element in elements.iter() {
                // Identify potential BackboneElement definitions (path has parts, type is BackboneElement or Element)
                // and it's not the root element itself.
                let path_parts: Vec<&str> = element.path.split('.').collect();
                if path_parts.len() > 1 && path_parts[0] == sd.name { // Belongs to the current resource/type
                    if let Some(types) = &element.r#type {
                        if types.iter().any(|t| t.code == "BackboneElement" || t.code == "Element") && element.content_reference.is_none() {
                            // Check if it's a definition point for a backbone element (often has children)
                            let has_children = elements.iter().any(|child_el| {
                                child_el.path.starts_with(&format!("{}.", element.path))
                            });

                            if has_children && !processed_backbone_paths.contains(&element.path) {
                                let backbone_type_name = generate_type_name(&element.path);
                                let mut backbone_output = String::new();
                                // Generate the struct for this backbone element
                                // We need a dummy StructureDefinition-like object or adapt process_struct_elements
                                let backbone_sd = StructureDefinition {
                                    name: backbone_type_name.clone(),
                                    kind: "complex-type".to_string(), // Assume backbone is complex-type
                                    r#abstract: false,
                                    derivation: Some("specialization".to_string()), // Assume specialization
                                    snapshot: Some(initial_fhir_model::StructureDefinitionSnapshot {
                                        element: Some(elements.iter().filter(|e| e.path.starts_with(&format!("{}.", element.path)) || e.path == element.path).cloned().collect()), // Pass relevant elements
                                    }),
                                    ..Default::default() // Add other necessary fields if needed
                                };
                                process_struct_elements(&backbone_sd, elements, &mut backbone_output, cycles, all_allowed_elements);
                                backbone_structs_code.push_str(&backbone_output);
                                processed_backbone_paths.insert(element.path.clone());
                            }
                        }
                    }
                }
            }
            // Prepend backbone structs before the main struct
            output.push_str(&backbone_structs_code);

            // --- Second Pass: Generate the Main Struct ---
            // Pass all_allowed_elements for context lookups (e.g., contentReference).
            process_struct_elements(sd, elements, &mut output, cycles, all_allowed_elements);
        }
    }
    output
}

fn generate_primitive_type(sd: &StructureDefinition) -> String {
    let type_name = &sd.name;
    let mut output = String::new();

    // Determine the value type based on the primitive type
    let value_type = match type_name.as_str() {
        "boolean" => "bool",
        "integer" | "positiveInt" | "unsignedInt" => "std::primitive::i32",
        "decimal" => "std::primitive::f64",
        "integer64" => "std::primitive::i64",
        "string" => "std::string::String",
        "code" => "std::string::String",
        "base64Binary" => "std::string::String",
        "canonical" => "std::string::String",
        "id" => "std::string::String",
        "oid" => "std::string::String",
        "uri" => "std::string::String",
        "url" => "std::string::String",
        "uuid" => "std::string::String",
        "markdown" => "std::string::String",
        "xhtml" => "std::string::String",
        "date" => "std::string::String",
        "dateTime" | "instant" | "time" => "std::string::String",
        _ => "std::string::String",
    };

    // Generate a type alias using Element<T, Extension> or DecimalElement<Extension> for decimal type
    if type_name == "decimal" {
        output.push_str("pub type Decimal = DecimalElement<Extension>;\n");
    } else {
        output.push_str(&format!(
            "pub type {} = Element<{}, Extension>;\n",
            capitalize_first_letter(type_name),
            value_type
        ));
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

    // Explicitly add known cycles that might not be detected by the simple logic above
    // (e.g., involving the generated Resource enum or specific FHIR patterns)
    cycles.insert(("Bundle".to_string(), "Resource".to_string()));
    cycles.insert(("Resource".to_string(), "Bundle".to_string())); // Ensure bidirectional check

    cycles
}

// Renamed from process_elements to avoid confusion, processes elements for a *single* struct/type
fn process_struct_elements(
    struct_def: &StructureDefinition, // The definition of the struct being generated
    elements: &[ElementDefinition],   // Elements belonging to this struct definition
    output: &mut String,
    cycles: &std::collections::HashSet<(String, String)>,
    all_allowed_elements: &[&ElementDefinition], // All elements from allowed types for context
) {
    let type_name = &struct_def.name; // Use the struct name directly
    let base_path = type_name; // For top-level types, base path is the type name
    let base_path_parts_count = base_path.split('.').count();

    // Filter elements that are direct children of the current struct/backbone element.
    let direct_elements: Vec<&ElementDefinition> = elements
        .iter()
        .filter(|e| {
            e.path.starts_with(&format!("{}.", base_path)) // Must be under the current path
            && e.path.split('.').count() == base_path_parts_count + 1 // Must be exactly one level deeper
        })
        .collect();

    // Generate Choice Enums first (needed by fields)
    let choice_fields: Vec<_> = direct_elements
        .iter()
        .filter(|e| e.path.ends_with("[x]"))
        .cloned()
        .collect();
    let mut processed_choice_enums = std::collections::HashSet::new();

    for choice in &choice_fields {
        let base_name = choice.path.split('.').last().unwrap().trim_end_matches("[x]");
        // Use the *struct_def name* (which could be Patient or PatientContact) for the enum prefix
        let enum_name = format!(
            "{}{}",
            capitalize_first_letter(type_name), // Use current struct/backbone name
            capitalize_first_letter(base_name)
        );
        let enum_name = format!("{}Choice", enum_name);

        if processed_choice_enums.contains(&enum_name) {
            continue;
        }
        processed_choice_enums.insert(enum_name.clone());

        output.push_str(&format!(
            "/// Choice of types for the {}[x] field in {}\n",
            base_name,
            capitalize_first_letter(type_name) // Use current struct/backbone name
        ));
        output.push_str("#[derive(Debug, Serialize, Deserialize, PartialEq)]\n");
        output.push_str("#[serde(rename_all = \"camelCase\")]\n");
        output.push_str(&format!("pub enum {} {{\n", enum_name));

        if let Some(types) = &choice.r#type {
            for ty in types {
                let type_code = capitalize_first_letter(&ty.code);
                let rename_value = format!("{}{}", base_name, type_code);
                output.push_str(&format!(
                    "    /// Variant accepting the {} type.\n",
                    type_code
                ));
                output.push_str(&format!("    #[serde(rename = \"{}\")]\n", rename_value));
                output.push_str(&format!("    {}({}),\n", type_code, type_code));
            }
        }
        output.push_str("}\n\n");
    }

    // Generate the struct definition
    output.push_str("#[derive(Debug, Serialize, Deserialize, PartialEq)]\n");
    output.push_str(&format!(
        "pub struct {} {{\n",
        capitalize_first_letter(type_name) // Use current struct/backbone name
    ));

    let mut added_fields = std::collections::HashSet::new();
    for element in &direct_elements { // Iterate over direct children only
        if let Some(field_name) = element.path.split('.').last() {
            let clean_field_name = field_name.trim_end_matches("[x]");
            if !added_fields.contains(clean_field_name) {
                generate_element_definition(
                    element,
                    type_name, // Pass the current struct/backbone name
                    output,
                    cycles,
                    all_allowed_elements,
                );
                added_fields.insert(clean_field_name.to_string());
            }
        }
    }
    output.push_str("}\n\n");
}

fn generate_element_definition(
    element: &ElementDefinition,
    type_name: &String, // Name of the struct this field belongs to
    output: &mut String,
    cycles: &std::collections::HashSet<(String, String)>,
    all_allowed_elements: &[&ElementDefinition], // Pass all relevant elements for context lookups
) {
    if let Some(field_name) = element.path.split('.').last() {
        let rust_field_name = make_rust_safe(field_name);

        let mut serde_attrs = Vec::new();
        // Handle field renaming, ensuring we don't add duplicate rename attributes
        if field_name != rust_field_name {
            // For choice fields, use the name without [x]
            if field_name.ends_with("[x]") {
                serde_attrs.push(format!(
                    "rename = \"{}\"",
                    field_name.trim_end_matches("[x]")
                ));
            } else {
                serde_attrs.push(format!("rename = \"{}\"", field_name));
            }
        }

        let ty = match element.r#type.as_ref().and_then(|t| t.first()) {
            Some(ty) => ty,
            None => {
                if let Some(content_ref) = &element.content_reference {
                    let ref_id = &content_ref[1..]; // e.g., "Patient.contact.name"
                                                    // Search in all allowed elements passed for context
                    if let Some(referenced_element) = all_allowed_elements
                        .iter()
                        .find(|e| e.id.as_ref().map_or(false, |id| id == ref_id))
                    {
                        // Use the type from the referenced element
                        if let Some(ref_ty) =
                            referenced_element.r#type.as_ref().and_then(|t| t.first())
                        {
                            ref_ty
                        } else {
                            return;
                        }
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            }
        };
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
            // Use generate_type_name for BackboneElement/Element to get specific names like PatientContact
            "Element" | "BackboneElement" => &generate_type_name(&element.path),
            _ => &capitalize_first_letter(&ty.code), // Default: Capitalize the FHIR type code
        };

        let mut base_type = if let Some(content_ref) = &element.content_reference {
            if content_ref.starts_with('#') {
                // If it's a contentReference, generate the type name from the reference path
                // Find the element definition by ID (which should match the content_ref without '#')
                let ref_id = &content_ref[1..];
                all_allowed_elements
                    .iter()
                    .find(|e| e.id.as_ref().map_or(false, |id| id == ref_id))
                    .map(|referenced_element| generate_type_name(&referenced_element.path))
                    .unwrap_or_else(|| {
                        eprintln!("Warning: Could not resolve contentReference: {}", content_ref);
                        "UnknownType".to_string() // Fallback or error
                    })
            } else {
                base_type_raw.to_string() // Should not happen for contentReference?
            }
        } else {
            base_type_raw.to_string()
        };

        // Ensure primitive types use the generated type alias (e.g., String, Boolean)
        // Check if base_type matches a known primitive alias we generate
        match base_type.as_str() {
            "Base64Binary" | "Boolean" | "Canonical" | "Code" | "Date" | "DateTime" |
            "Decimal" | "Id" | "Instant" | "Integer" | "Markdown" | "Oid" |
            "PositiveInt" | "String" | "Time" | "UnsignedInt" | "Uri" | "Url" |
            "Uuid" | "Xhtml" => {
                // It's a primitive type alias, keep it as is.
            }
            _ => {
                // If it's not a primitive alias, assume it's a complex type/resource/backbone element name.
                // No change needed here, base_type should already be the correct struct name.
            }
        }


        let mut type_str = if field_name.ends_with("[x]") {
            let base_name = field_name.trim_end_matches("[x]");
            let enum_name = format!(
                "{}Choice",
                format!(
                    "{}{}",
                    capitalize_first_letter(type_name), // Use parent struct name
                    capitalize_first_letter(base_name)
                )
            );
            serde_attrs.clear();
            serde_attrs.push("flatten".to_string());
            format!("Option<{}>", enum_name)
        } else if is_array {
            serde_attrs.push("skip_serializing_if = \"Option::is_none\"".to_string());
            format!("Option<Vec<{}>>", base_type)
        } else if element.min.unwrap_or(0) == 0 {
            serde_attrs.push("skip_serializing_if = \"Option::is_none\"".to_string());
            format!("Option<{}>", base_type)
        } else {
            base_type.clone() // Clone base_type here
        };

        // Add Box<> to break cycles
        let from_struct_name = type_name;
        let field_type_name = &base_type; // Use the potentially generated backbone name

        if cycles.contains(&(from_struct_name.clone(), field_type_name.clone())) ||
           cycles.contains(&(field_type_name.clone(), from_struct_name.clone()))
        {
            if type_str.starts_with("Option<Vec<") {
                let inner = &type_str[12..type_str.len() - 2];
                type_str = format!("Option<Vec<Box<{}>>>", inner);
            } else if type_str.starts_with("Option<") {
                let inner = &type_str[7..type_str.len() - 1];
                type_str = format!("Option<Box<{}>>", inner);
            } else if type_str.starts_with("Vec<") {
                let inner = &type_str[4..type_str.len() - 1];
                type_str = format!("Vec<Box<{}>>", inner);
            } else {
                type_str = format!("Box<{}>", type_str);
            }
        } else if from_struct_name == "Parameters" && field_name == "resource" {
             // Explicitly Box Parameters.resource
            type_str = format!("Option<Box<{}>>", base_type); // base_type should be "Resource"
        } else if from_struct_name == "Bundle" && field_name == "resource" {
             // Explicitly Box Bundle.entry.resource
            type_str = format!("Option<Box<{}>>", base_type); // base_type should be "Resource"
        }


        if !serde_attrs.is_empty() {
            output.push_str(&format!("    #[serde({})]\n", serde_attrs.join(", ")));
        }

        let clean_field_name = if rust_field_name.ends_with("[x]") {
            rust_field_name.trim_end_matches("[x]").to_string()
        } else {
            rust_field_name
        };

        output.push_str(&format!("    pub {}: {},\n", clean_field_name, type_str));
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
        assert!(process_fhir_version(Some(FhirVersion::R4), &temp_dir).is_ok());

        // Verify files were created
        assert!(temp_dir.join("r4.rs").exists());

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
