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
) -> io::Result<()> { // Signature already returns io::Result<()>
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

    // Handle primitive types differently
    if is_primitive_type(sd) {
        // Ensure primitive types are handled even if not explicitly in allowed_types set initially
        // as they are fundamental.
        return generate_primitive_type(sd);
    }

    // Process elements for complex types and resources
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            // We are generating code for a specific StructureDefinition (sd),
            // so we don't need the processed_types check here as generate_code filters calls.
            // We pass all_allowed_elements for context lookups (e.g., contentReference).
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

    // Add cycle from Bundle to Resource since Bundle.issues contains Resources (an specially generated enum) beginning in R5
    if elements
        .iter()
        .any(|e| e.id.as_ref().map_or(false, |id| id == "Bundle.issues"))
    {
        cycles.insert(("Bundle".to_string(), "Resource".to_string()));
    }

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

    // Filter elements that belong directly to this type (path starts with type_name and has one dot)
    // or are the base element itself (path == type_name).
    // Also handle BackboneElement which might have paths like "Patient.contact.address"
    // where "Patient.contact" is the BackboneElement.
    let direct_elements: Vec<&ElementDefinition> = elements
        .iter()
        .filter(|e| {
            let path_parts: Vec<&str> = e.path.split('.').collect();
            // Check if the element path starts with the type name and has exactly one more part,
            // OR if the element path *is* the type name (for the root element definition, though we often skip it).
            // We need to handle cases like "Patient.identifier" (direct child of Patient)
            // and potentially nested elements defined within the same StructureDefinition,
            // like "Patient.contact.gender" if Patient.contact is a BackboneElement defined inline.
            // The filtering logic here assumes elements are ordered such that parents appear before children.
            // A robust approach might involve building a tree, but filtering by path prefix length is simpler.

            // Include the root element definition itself (e.g., "Patient") - needed for metadata but not fields.
            if path_parts.len() == 1 && path_parts[0] == type_name.as_str() {
                 return true; // Keep the root element for potential metadata access if needed later
            }
            // Include direct children (e.g., "Patient.identifier")
            if path_parts.len() > 1 && path_parts[0] == type_name.as_str() {
                // Check if it's a direct child or a child of an inline BackboneElement
                // This logic might need refinement depending on how BackboneElements are structured.
                // For now, let's assume direct children are what we primarily need for struct fields.
                // We filter out the root element later when generating fields.
                return true;
            }
            false
        })
        .collect();

    // Filter out the root element itself when generating fields
    let field_elements: Vec<&ElementDefinition> = direct_elements
        .iter()
        .filter(|e| e.path != *type_name) // Exclude the root element
        .cloned() // Clone the references
        .collect();


    let choice_fields: Vec<_> = field_elements // Use field_elements here
        .iter()
        .filter(|e| e.path.ends_with("[x]"))
        .cloned() // Clone references
        .collect();
    let mut processed_choice_enums = std::collections::HashSet::new(); // Track enums generated for this struct

    for choice in &choice_fields { // Iterate over references
        let base_name = choice
            .path
            .split('.')
            .last()
            .unwrap()
            .trim_end_matches("[x]");

        let enum_name = format!(
            "{}{}",
            capitalize_first_letter(type_name),
            capitalize_first_letter(base_name)
        );
        // Ensure choice enum names are distinct by adding a suffix
        let enum_name = format!("{}_Choice", enum_name);


        // Skip if we've already generated this enum for this struct
        if processed_choice_enums.contains(&enum_name) {
            continue;
        }
        processed_choice_enums.insert(enum_name.clone());

        // Add documentation comment for the enum
        output.push_str(&format!(
            "/// Choice of types for the {}[x] field in {}\n",
            base_name,
            capitalize_first_letter(type_name)
        ));

        output.push_str("#[derive(Debug, Serialize, Deserialize)]\n"); // Keep Serialize, Deserialize for choice enums
        output.push_str("#[serde(rename_all = \"camelCase\")]\n");
        output.push_str(&format!("pub enum {} {{\n", enum_name));

        if let Some(types) = &choice.r#type {
            for ty in types {
                let type_code = capitalize_first_letter(&ty.code);
                let rename_value = format!("{}{}", base_name, type_code);

                // Add documentation for each variant
                output.push_str(&format!(
                    "    /// Variant accepting the {} type.\n",
                    type_code
                ));
                output.push_str(&format!("    #[serde(rename = \"{}\")]\n", rename_value));
                // Assume the type itself (e.g., String, Reference) is generated elsewhere
                output.push_str(&format!("    {}({}),\n", type_code, type_code));
            }
        }
        output.push_str("}\n\n");
    }

    // Only add FhirSerde derive for complex-types and resources (structs)
    if struct_def.kind == "complex-type" || struct_def.kind == "resource" {
         // Add standard derives first, then FhirSerde
         output.push_str("#[derive(Debug, Serialize, Deserialize, PartialEq)]\n");
    } else {
         // For other kinds like BackboneElement (if treated differently) or potentially others
         // just add standard derives. Enums are handled separately.
         output.push_str("#[derive(Debug, Serialize, Deserialize, PartialEq)]\n");
    }
    output.push_str(&format!(
        "pub struct {} {{\n",
        capitalize_first_letter(type_name)
    ));

    // Keep track of fields added to avoid duplicates (e.g., from slicing or choice types)
    let mut added_fields = std::collections::HashSet::new();

    for element in &field_elements { // Use field_elements here
        // Skip elements that don't directly define a field (e.g., intermediate BackboneElement paths)
        // A simple check is if the path has more parts than the base type name path.
        let base_path_parts = type_name.split('.').count();
        let element_path_parts = element.path.split('.').count();

        // Only process elements that are direct children or grandchildren (for simple cases)
        // This might need adjustment for deeply nested BackboneElements.
        // We primarily want elements like "Patient.identifier" or "Patient.contact.gender".
        // Elements like "Patient.contact" itself (if it's a BackboneElement) are handled by their type definition.
        if element_path_parts <= base_path_parts {
             continue; // Skip elements that are not fields of the current struct
        }


        if let Some(field_name) = element.path.split('.').last() {
            // Use the cleaned field name (without [x]) for tracking duplicates
            let clean_field_name = field_name.trim_end_matches("[x]");
            if !added_fields.contains(clean_field_name) {
                generate_element_definition(
                    element,
                    type_name,
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
                capitalize_first_letter(type_name),
                capitalize_first_letter(base_name)
            );
            // For choice fields, we use flatten instead of rename
            serde_attrs.clear(); // Clear any previous attributes
            serde_attrs.push("flatten".to_string());
            format!("Option<{}>", enum_name)
        } else if is_array {
            serde_attrs.push("skip_serializing_if = \"Option::is_none\"".to_string());
            format!("Option<Vec<{}>>", base_type)
        } else if element.min.unwrap_or(0) == 0 {
            serde_attrs.push("skip_serializing_if = \"Option::is_none\"".to_string());
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
                            type_str = format!("Option<Box<{}>>", &type_str[7..type_str.len() - 1]);
                        } else {
                            type_str = format!("Box<{}>", type_str);
                        }
                        break;
                    }
                }
            }
        }

        // Output consolidated serde attributes if any exist
        if !serde_attrs.is_empty() {
            output.push_str(&format!("    #[serde({})]\n", serde_attrs.join(", ")));
        }

        // For choice fields, strip the [x] from the field name
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
