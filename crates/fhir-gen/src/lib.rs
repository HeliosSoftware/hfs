//! # FHIR Code Generator
//!
//! This crate provides functionality to generate Rust code from FHIR StructureDefinitions.
//! It transforms official FHIR specification JSON files into idiomatic Rust types with
//! proper serialization/deserialization support.

//!
//! ## Overview
//!
//! The code generator performs the following steps:
//! 1. Loads FHIR specification files from `resources/{VERSION}/`
//! 2. Parses StructureDefinitions using minimal bootstrap types
//! 3. Analyzes type hierarchies and detects circular dependencies
//! 4. Generates strongly-typed Rust structs and enums
//! 5. Outputs version-specific modules (e.g., `r4.rs`, `r5.rs`)
//!
//! ## Example Usage
//!
//! ```ignore
//! use helios_fhir_gen::process_fhir_version;
//! use helios_fhir::FhirVersion;
//! use std::path::PathBuf;
//!
//! let output_dir = PathBuf::from("output");
//!
//! // Generate code for R4
//! process_fhir_version(Some(FhirVersion::R4), &output_dir)?;
//!
//! // Generate code for all versions
//! process_fhir_version(None, &output_dir)?;
//! # Ok::<(), std::io::Error>(())
//! ```

pub mod initial_fhir_model;

use crate::initial_fhir_model::{Bundle, Resource};
use helios_fhir::FhirVersion;
use initial_fhir_model::ElementDefinition;
use initial_fhir_model::StructureDefinition;
use serde_json::Result;
use std::fs::File;
use std::io::BufReader;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;

/// Processes a single FHIR version and generates corresponding Rust code.
///
/// This function loads all JSON specification files for the given FHIR version,
/// parses the StructureDefinitions, and generates Rust code for all valid types.
///
/// # Arguments
///
/// * `version` - The FHIR version to process (R4, R4B, R5, or R6)
/// * `output_path` - Directory where the generated Rust files will be written
///
/// # Returns
///
/// Returns `Ok(())` on success, or an `io::Error` if file operations fail.
///
/// # Generated Output
///
/// Creates a single `.rs` file named after the version (e.g., `r4.rs`) containing:
/// - Type definitions for all FHIR resources and data types
/// - Choice type enums for polymorphic elements
/// - A unified Resource enum for all resource types
/// - Proper serialization/deserialization attributes
///
/// # Example
///
/// ```ignore
/// use helios_fhir_gen::process_single_version;
/// use helios_fhir::FhirVersion;
/// use std::path::PathBuf;
///
/// let output_dir = PathBuf::from("generated");
/// process_single_version(&FhirVersion::R4, &output_dir)?;
/// # Ok::<(), std::io::Error>(())
/// ```
fn process_single_version(version: &FhirVersion, output_path: impl AsRef<Path>) -> io::Result<()> {
    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let version_dir = resources_dir.join(version.as_str());
    // Create output directory if it doesn't exist
    std::fs::create_dir_all(output_path.as_ref())?;

    let version_path = output_path
        .as_ref()
        .join(format!("{}.rs", version.as_str().to_lowercase()));

    // Create the version-specific output file with initial content
    std::fs::write(
        &version_path,
        "// Generated documentation contains content from HL7 FHIR specifications\n// which may include HTML-like tags and bracket notations that are not actual HTML or links\n#![allow(rustdoc::broken_intra_doc_links)]\n#![allow(rustdoc::invalid_html_tags)]\n\nuse helios_fhir_macro::{FhirPath, FhirSerde};\nuse serde::{Deserialize, Serialize};\n\nuse crate::{DecimalElement, Element};\n\n",
    )?;

    // Collect all type hierarchy information across all bundles
    let mut global_type_hierarchy = std::collections::HashMap::new();
    let mut all_resources = Vec::new();
    let mut all_complex_types = Vec::new();

    // First pass: collect all bundles and extract global information
    let bundles: Vec<_> = visit_dirs(&version_dir)?
        .into_iter()
        .filter_map(|file_path| match parse_structure_definitions(&file_path) {
            Ok(bundle) => Some(bundle),
            Err(e) => {
                eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e);
                None
            }
        })
        .collect();

    // Extract global information from all bundles
    for bundle in &bundles {
        if let Some((hierarchy, resources, complex_types)) = extract_bundle_info(bundle) {
            global_type_hierarchy.extend(hierarchy);
            all_resources.extend(resources);
            all_complex_types.extend(complex_types);
        }
    }

    // Second pass: generate code for each bundle
    for bundle in bundles {
        generate_code(bundle, &version_path, false)?; // false = don't generate global constructs
    }

    // Generate global constructs once at the end
    generate_global_constructs(
        &version_path,
        &global_type_hierarchy,
        &all_resources,
        &all_complex_types,
    )?;

    Ok(())
}

/// Processes one or more FHIR versions and generates corresponding Rust code.
///
/// This is the main entry point for the code generation process. It can either
/// process a specific FHIR version or all available versions based on enabled features.
///
/// # Arguments
///
/// * `version` - Optional specific FHIR version to process. If `None`, processes all
///   versions that are enabled via Cargo features
/// * `output_path` - Directory where generated Rust files will be written
///
/// # Returns
///
/// Returns `Ok(())` on success. If processing multiple versions, continues even if
/// individual versions fail (with warnings), returning `Ok(())` as long as the
/// overall process completes.
///
/// # Feature Dependencies
///
/// The versions processed depend on which Cargo features are enabled:
/// - `R4` - FHIR Release 4 (default)
/// - `R4B` - FHIR Release 4B  
/// - `R5` - FHIR Release 5
/// - `R6` - FHIR Release 6
///
/// # Examples
///
/// ```ignore
/// use helios_fhir_gen::process_fhir_version;
/// use helios_fhir::FhirVersion;
/// use std::path::PathBuf;
///
/// let output_dir = PathBuf::from("crates/fhir/src");
///
/// // Process only R4
/// process_fhir_version(Some(FhirVersion::R4), &output_dir)?;
///
/// // Process all enabled versions
/// process_fhir_version(None, &output_dir)?;
/// # Ok::<(), std::io::Error>(())
/// ```
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

/// Recursively visits directories to find relevant JSON specification files.
///
/// This function traverses the resource directory structure and collects all JSON files
/// that contain FHIR definitions, while filtering out files that aren't needed for
/// code generation (like concept maps and value sets).
///
/// # Arguments
///
/// * `dir` - Root directory to search for JSON files
///
/// # Returns
///
/// Returns a vector of `PathBuf`s pointing to relevant JSON specification files,
/// or an `io::Error` if directory traversal fails.
///
/// # Filtering Logic
///
/// Only includes JSON files that:
/// - Have a `.json` extension
/// - Do not contain "conceptmap" in the filename
/// - Do not contain "valueset" in the filename
///
/// This filtering focuses the code generation on structural definitions rather
/// than terminology content.
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
                        if !filename.contains("conceptmap")
                            && !filename.contains("valueset")
                            && !filename.contains("bundle-entry")
                        {
                            json_files.push(path);
                        }
                    }
                }
            }
        }
    }
    Ok(json_files)
}

/// Parses a JSON file containing FHIR StructureDefinitions into a Bundle.
///
/// This function reads a JSON file and deserializes it into a FHIR Bundle containing
/// StructureDefinitions and other FHIR resources used for code generation.
///
/// # Arguments
///
/// * `path` - Path to the JSON file to parse
///
/// # Returns
///
/// Returns a `Bundle` on success, or a `serde_json::Error` if parsing fails.
///
/// # File Format
///
/// Expects JSON files in the standard FHIR Bundle format with entries containing
/// StructureDefinition resources, as provided by the official FHIR specification.
fn parse_structure_definitions<P: AsRef<Path>>(path: P) -> Result<Bundle> {
    let file = File::open(path).map_err(serde_json::Error::io)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}

/// Determines if a StructureDefinition should be included in code generation.
///
/// This function filters StructureDefinitions to only include those that represent
/// concrete types that should have Rust code generated for them.
///
/// # Arguments
///
/// * `def` - The StructureDefinition to evaluate
///
/// # Returns
///
/// Returns `true` if the StructureDefinition should be processed for code generation.
///
/// # Criteria
///
/// A StructureDefinition is considered valid if:
/// - Kind is "complex-type", "primitive-type", or "resource"
/// - Derivation is "specialization" (concrete implementations)
/// - Abstract is `false` (not an abstract base type)
fn is_valid_structure_definition(def: &StructureDefinition) -> bool {
    (def.kind == "complex-type" || def.kind == "primitive-type" || def.kind == "resource")
        && def.derivation.as_deref() == Some("specialization")
        && !def.r#abstract
}

/// Checks if a StructureDefinition represents a FHIR primitive type.
///
/// Primitive types are handled differently in code generation, typically being
/// mapped to Rust primitive types or type aliases rather than full structs.
///
/// # Arguments
///
/// * `def` - The StructureDefinition to check
///
/// # Returns
///
/// Returns `true` if this is a primitive type definition.
fn is_primitive_type(def: &StructureDefinition) -> bool {
    def.kind == "primitive-type"
}

type BundleInfo = (
    std::collections::HashMap<String, String>,
    Vec<String>,
    Vec<String>,
);

/// Extracts type hierarchy and resource information from a bundle
fn extract_bundle_info(bundle: &Bundle) -> Option<BundleInfo> {
    let mut type_hierarchy = std::collections::HashMap::new();
    let mut resources = Vec::new();
    let mut complex_types = Vec::new();

    if let Some(entries) = bundle.entry.as_ref() {
        for entry in entries {
            if let Some(resource) = &entry.resource {
                if let Resource::StructureDefinition(def) = resource {
                    if is_valid_structure_definition(def) {
                        // Extract type hierarchy from baseDefinition
                        if let Some(base_def) = &def.base_definition {
                            if let Some(parent) = base_def.split('/').next_back() {
                                type_hierarchy.insert(def.name.clone(), parent.to_string());
                            }
                        }

                        if def.kind == "resource" && !def.r#abstract {
                            resources.push(def.name.clone());
                        } else if def.kind == "complex-type" && !def.r#abstract {
                            complex_types.push(def.name.clone());
                        }
                    }
                }
            }
        }
    }

    Some((type_hierarchy, resources, complex_types))
}

/// Generates global constructs (Resource enum, type hierarchy, etc.) once at the end
fn generate_global_constructs(
    output_path: impl AsRef<Path>,
    type_hierarchy: &std::collections::HashMap<String, String>,
    all_resources: &[String],
    all_complex_types: &[String],
) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_path.as_ref())?;

    // Generate the Resource enum
    if !all_resources.is_empty() {
        let resource_enum = generate_resource_enum(all_resources.to_vec());
        write!(file, "{}", resource_enum)?;

        // Add From<T> implementations for base types
        writeln!(
            file,
            "// --- From<T> Implementations for Element<T, Extension> ---"
        )?;
        writeln!(file, "impl From<bool> for Element<bool, Extension> {{")?;
        writeln!(file, "    fn from(value: bool) -> Self {{")?;
        writeln!(file, "        Self {{")?;
        writeln!(file, "            value: Some(value),")?;
        writeln!(file, "            ..Default::default()")?;
        writeln!(file, "        }}")?;
        writeln!(file, "    }}")?;
        writeln!(file, "}}")?;

        writeln!(
            file,
            "impl From<std::primitive::i32> for Element<std::primitive::i32, Extension> {{"
        )?;
        writeln!(file, "    fn from(value: std::primitive::i32) -> Self {{")?;
        writeln!(file, "        Self {{")?;
        writeln!(file, "            value: Some(value),")?;
        writeln!(file, "            ..Default::default()")?;
        writeln!(file, "        }}")?;
        writeln!(file, "    }}")?;
        writeln!(file, "}}")?;

        writeln!(
            file,
            "impl From<std::string::String> for Element<std::string::String, Extension> {{"
        )?;
        writeln!(file, "    fn from(value: std::string::String) -> Self {{")?;
        writeln!(file, "        Self {{")?;
        writeln!(file, "            value: Some(value),")?;
        writeln!(file, "            ..Default::default()")?;
        writeln!(file, "        }}")?;
        writeln!(file, "    }}")?;
        writeln!(file, "}}")?;
        writeln!(file, "// --- End From<T> Implementations ---")?;
    }

    // Generate type hierarchy module
    if !type_hierarchy.is_empty() {
        let type_hierarchy_module = generate_type_hierarchy_module(type_hierarchy);
        write!(file, "{}", type_hierarchy_module)?;
    }

    // Generate ComplexTypes struct and FhirComplexTypeProvider implementation
    if !all_complex_types.is_empty() {
        writeln!(file, "\n// --- Complex Types Provider ---")?;
        writeln!(file, "/// Marker struct for complex type information")?;
        writeln!(file, "pub struct ComplexTypes;")?;
        writeln!(
            file,
            "\nimpl crate::FhirComplexTypeProvider for ComplexTypes {{"
        )?;
        writeln!(
            file,
            "    fn get_complex_type_names() -> Vec<&'static str> {{"
        )?;
        writeln!(file, "        vec![")?;
        for complex_type in all_complex_types {
            writeln!(file, "            \"{}\",", complex_type)?;
        }
        writeln!(file, "        ]")?;
        writeln!(file, "    }}")?;
        writeln!(file, "}}")?;
    }

    Ok(())
}

/// Generates Rust code from a Bundle of FHIR StructureDefinitions.
///
/// This is the main code generation function that processes all StructureDefinitions
/// in a Bundle and writes the corresponding Rust code to a file.
///
/// # Arguments
///
/// * `bundle` - FHIR Bundle containing StructureDefinitions and other resources
/// * `output_path` - Path to the output Rust file
///
/// # Returns
///
/// Returns `Ok(())` on success, or an `io::Error` if file operations fail.
///
/// # Process Overview
///
/// 1. **First Pass**: Collects all ElementDefinitions and detects circular dependencies
/// 2. **Second Pass**: Generates Rust code for each valid StructureDefinition
/// 3. **Final Step**: Generates a unified Resource enum and helper implementations
///
/// # Generated Code Includes
///
/// - Struct definitions for complex types and resources
/// - Enum definitions for choice types (polymorphic elements)
/// - A Resource enum containing all resource types
/// - From<T> implementations for primitive type conversions
/// - Proper derive macros for serialization and FHIR-specific functionality
fn generate_code(
    bundle: Bundle,
    output_path: impl AsRef<Path>,
    _generate_globals: bool,
) -> io::Result<()> {
    // First collect all ElementDefinitions across all StructureDefinitions
    let mut all_elements = Vec::new();

    if let Some(entries) = bundle.entry.as_ref() {
        // First pass: collect all elements
        for entry in entries {
            if let Some(resource) = &entry.resource {
                if let Resource::StructureDefinition(def) = resource {
                    if is_valid_structure_definition(def) {
                        if let Some(snapshot) = &def.snapshot {
                            if let Some(elements) = &snapshot.element {
                                all_elements.extend(elements.iter());
                            }
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
                                .append(true)
                                .open(output_path.as_ref())?;
                            write!(file, "{}", content)?;
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

/// Generates a Rust enum containing all FHIR resource types.
///
/// This function creates a single enum that can represent any FHIR resource,
/// using serde's tag-based deserialization to automatically route JSON to
/// the correct variant based on the "resourceType" field.
///
/// # Arguments
///
/// * `resources` - Vector of resource type names to include in the enum
///
/// # Returns
///
/// Returns a string containing the Rust enum definition.
///
/// # Generated Features
///
/// - Tagged enum with `#[serde(tag = "resourceType")]` for automatic routing
/// - All standard derives for functionality and compatibility
/// - Each variant contains the corresponding resource struct
fn generate_resource_enum(resources: Vec<String>) -> String {
    let mut output = String::new();
    // Add Clone to the derives for the Resource enum
    output.push_str("#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]\n");
    output.push_str("#[serde(tag = \"resourceType\")]\n");
    output.push_str("pub enum Resource {\n");

    for resource in resources {
        output.push_str(&format!("    {}({}),\n", resource, resource));
    }

    output.push_str("}\n\n");
    output
}

/// Generates a module containing type hierarchy information extracted from FHIR specifications.
///
/// This function creates a module with functions to query type relationships at runtime,
/// allowing the code to understand FHIR type inheritance without hard-coding.
///
/// # Arguments
///
/// * `type_hierarchy` - HashMap mapping type names to their parent types
///
/// # Returns
///
/// Returns a string containing the type hierarchy module definition.
fn generate_type_hierarchy_module(
    type_hierarchy: &std::collections::HashMap<String, String>,
) -> String {
    let mut output = String::new();

    output.push_str("\n// --- Type Hierarchy Module ---\n");
    output.push_str("/// Type hierarchy information extracted from FHIR specifications\n");
    output.push_str("pub mod type_hierarchy {\n");
    output.push_str("    use std::collections::HashMap;\n");
    output.push_str("    use std::sync::OnceLock;\n\n");

    // Generate the static HashMap
    output.push_str("    /// Maps FHIR type names to their parent types\n");
    output.push_str("    static TYPE_PARENTS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();\n\n");

    output
        .push_str("    fn get_type_parents() -> &'static HashMap<&'static str, &'static str> {\n");
    output.push_str("        TYPE_PARENTS.get_or_init(|| {\n");
    output.push_str("            let mut m = HashMap::new();\n");

    // Sort entries for consistent output
    let mut sorted_entries: Vec<_> = type_hierarchy.iter().collect();
    sorted_entries.sort_by_key(|(k, _)| k.as_str());

    for (child, parent) in sorted_entries {
        output.push_str(&format!(
            "            m.insert(\"{}\", \"{}\");\n",
            child, parent
        ));
    }

    output.push_str("            m\n");
    output.push_str("        })\n");
    output.push_str("    }\n\n");

    // Generate helper functions
    output.push_str("    /// Checks if a type is a subtype of another type\n");
    output.push_str("    pub fn is_subtype_of(child: &str, parent: &str) -> bool {\n");
    output.push_str("        // Direct match\n");
    output.push_str("        if child.eq_ignore_ascii_case(parent) {\n");
    output.push_str("            return true;\n");
    output.push_str("        }\n\n");
    output.push_str("        // Walk up the type hierarchy\n");
    output.push_str("        let mut current = child;\n");
    output.push_str("        while let Some(&parent_type) = get_type_parents().get(current) {\n");
    output.push_str("            if parent_type.eq_ignore_ascii_case(parent) {\n");
    output.push_str("                return true;\n");
    output.push_str("            }\n");
    output.push_str("            current = parent_type;\n");
    output.push_str("        }\n");
    output.push_str("        false\n");
    output.push_str("    }\n\n");

    output.push_str("    /// Gets the parent type of a given type\n");
    output.push_str("    pub fn get_parent_type(type_name: &str) -> Option<&'static str> {\n");
    output.push_str("        get_type_parents().get(type_name).copied()\n");
    output.push_str("    }\n\n");

    output.push_str("    /// Gets all subtypes of a given parent type\n");
    output.push_str("    pub fn get_subtypes(parent: &str) -> Vec<&'static str> {\n");
    output.push_str("        get_type_parents().iter()\n");
    output.push_str("            .filter_map(|(child, p)| {\n");
    output.push_str("                if p.eq_ignore_ascii_case(parent) {\n");
    output.push_str("                    Some(*child)\n");
    output.push_str("                } else {\n");
    output.push_str("                    None\n");
    output.push_str("                }\n");
    output.push_str("            })\n");
    output.push_str("            .collect()\n");
    output.push_str("    }\n");

    output.push_str("}\n\n");
    output
}

/// Converts a FHIR field name to a valid Rust identifier.
///
/// This function transforms FHIR field names into valid Rust identifiers by:
/// - Converting camelCase to snake_case
/// - Escaping Rust keywords with the `r#` prefix
///
/// # Arguments
///
/// * `input` - The original FHIR field name
///
/// # Returns
///
/// Returns a string that is a valid Rust identifier.
///
/// # Examples
///
/// ```ignore
/// # use helios_fhir_gen::make_rust_safe;
/// assert_eq!(make_rust_safe("birthDate"), "birth_date");
/// assert_eq!(make_rust_safe("type"), "r#type");
/// assert_eq!(make_rust_safe("abstract"), "r#abstract");
/// ```
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
        "type" | "use" | "abstract" | "for" | "ref" | "const" | "where" => {
            format!("r#{}", snake_case)
        }
        _ => snake_case,
    }
}

/// Capitalizes the first letter of a string.
///
/// This utility function is used to convert FHIR type names to proper Rust
/// type names that follow PascalCase conventions.
///
/// # Arguments
///
/// * `s` - The string to capitalize
///
/// # Returns
///
/// Returns a new string with the first character capitalized.
///
/// # Examples
///
/// ```ignore
/// # use helios_fhir_gen::capitalize_first_letter;
/// assert_eq!(capitalize_first_letter("patient"), "Patient");
/// assert_eq!(capitalize_first_letter("humanName"), "HumanName");
/// ```
fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

/// Escapes markdown text for use in Rust doc comments.
///
/// This function escapes special characters that could interfere with
/// Rust's doc comment parsing.
///
/// # Arguments
///
/// * `text` - The markdown text to escape
///
/// # Returns
///
/// Returns the escaped text safe for use in doc comments.
fn escape_doc_comment(text: &str) -> String {
    // First, normalize all line endings to \n and remove bare CRs
    let normalized = text
        .replace("\r\n", "\n") // Convert Windows line endings
        .replace('\r', "\n"); // Convert bare CRs to newlines

    let mut result = String::new();
    let mut in_code_block = false;

    // Process each line
    for line in normalized.lines() {
        let trimmed_line = line.trim();

        // Check for code block markers
        if trimmed_line == "```" {
            if in_code_block {
                // This is a closing ```
                result.push_str("```\n");
                in_code_block = false;
            } else {
                // This is an opening ```
                result.push_str("```text\n");
                in_code_block = true;
            }
            continue;
        }

        // Apply standard replacements
        let processed = line
            .replace("*/", "*\\/")
            .replace("/*", "/\\*")
            // Fix common typos in FHIR spec
            .replace("(aka \"privacy tags\".", "(aka \"privacy tags\").")
            .replace("(aka \"tagged\")", "(aka 'tagged')")
            // Escape comparison operators that look like quote markers to clippy
            .replace(" <=", " \\<=")
            .replace(" >=", " \\>=")
            .replace("(<=", "(\\<=")
            .replace("(>=", "(\\>=");

        result.push_str(&processed);
        result.push('\n');
    }

    // Clean up excessive blank lines and trailing whitespace
    result = result.replace("\n\n\n", "\n\n");
    result.trim_end().to_string()
}

/// Formats text content for use in Rust doc comments, handling proper indentation.
///
/// This function ensures that multi-line content is properly formatted for Rust doc
/// comments, including handling bullet points and numbered lists that need continuation indentation.
///
/// # Arguments
///
/// * `text` - The text to format
/// * `in_list` - Whether we're currently in a list context
///
/// # Returns
///
/// Returns formatted lines ready for doc comment output.
fn format_doc_content(text: &str, in_list: bool) -> Vec<String> {
    let mut output = Vec::new();
    let mut in_list_item = false;

    for line in text.split('\n') {
        let trimmed = line.trim_start();

        // Check if this is a list item (bullet, numbered, or dash)
        let is_bullet = trimmed.starts_with("* ") && !in_list;
        let is_dash = trimmed.starts_with("- ") && !in_list;
        let is_numbered = !in_list && {
            // Match patterns like "1) ", "2. ", "10) ", etc.
            if let Some(first_space) = trimmed.find(' ') {
                let prefix = &trimmed[..first_space];
                // Check if it ends with ) or . and starts with a number
                (prefix.ends_with(')') || prefix.ends_with('.'))
                    && prefix.chars().next().is_some_and(|c| c.is_numeric())
            } else {
                false
            }
        };

        if is_bullet || is_numbered || is_dash {
            in_list_item = true;
            output.push(line.to_string());
        } else if in_list_item {
            // We're in a list item context
            if line.trim().is_empty() {
                // Empty line ends the list item
                output.push(String::new());
                in_list_item = false;
            } else if trimmed.starts_with("* ")
                || trimmed.starts_with("- ")
                || (trimmed.find(' ').is_some_and(|idx| {
                    let prefix = &trimmed[..idx];
                    (prefix.ends_with(')') || prefix.ends_with('.'))
                        && prefix.chars().next().is_some_and(|c| c.is_numeric())
                }))
            {
                // New list item
                output.push(line.to_string());
            } else {
                // Continuation line - needs to be indented
                let content = line.trim();
                if !content.is_empty() {
                    // For numbered lists like "1) text", indent to align with text
                    // For bullet/dash lists, use 2 spaces
                    let indent = if let Some(prev_line) = output.last() {
                        let prev_trimmed = prev_line.trim_start();
                        if let Some(space_pos) = prev_trimmed.find(' ') {
                            let prefix = &prev_trimmed[..space_pos];
                            if (prefix.ends_with(')') || prefix.ends_with('.'))
                                && prefix.chars().next().is_some_and(|c| c.is_numeric())
                            {
                                // It's a numbered list - use 3 spaces for safety
                                "   ".to_string()
                            } else {
                                "  ".to_string()
                            }
                        } else {
                            "  ".to_string()
                        }
                    } else {
                        "  ".to_string()
                    };
                    output.push(format!("{}{}", indent, content));
                }
            }
        } else {
            // Not in a list item - regular line
            output.push(line.to_string());
        }
    }

    output
}

/// Formats cardinality information into human-readable text.
///
/// # Arguments
///
/// * `min` - Minimum cardinality (0 or 1)
/// * `max` - Maximum cardinality ("1", "*", or a specific number)
///
/// # Returns
///
/// Returns a formatted string describing the cardinality.
fn format_cardinality(min: Option<u32>, max: Option<&str>) -> String {
    let min_val = min.unwrap_or(0);
    let max_val = max.unwrap_or("1");

    match (min_val, max_val) {
        (0, "1") => "Optional (0..1)".to_string(),
        (1, "1") => "Required (1..1)".to_string(),
        (0, "*") => "Optional, Multiple (0..*)".to_string(),
        (1, "*") => "Required, Multiple (1..*)".to_string(),
        (min, max) => format!("{min}..{max}"),
    }
}

/// Formats constraint information for documentation.
///
/// # Arguments
///
/// * `constraints` - Vector of ElementDefinitionConstraint
///
/// # Returns
///
/// Returns formatted constraint documentation.
fn format_constraints(constraints: &[initial_fhir_model::ElementDefinitionConstraint]) -> String {
    if constraints.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("/// ## Constraints\n");

    for constraint in constraints {
        let escaped_human = escape_doc_comment(&constraint.human);

        // Handle multi-line constraint descriptions
        let human_lines: Vec<&str> = escaped_human.split('\n').collect();

        if human_lines.len() == 1 {
            // Single line - output as before
            output.push_str(&format!(
                "/// - **{}**: {} ({})\n",
                constraint.key, escaped_human, constraint.severity
            ));
        } else {
            // Multi-line - format the first line with key and severity
            output.push_str(&format!(
                "/// - **{}**: {} ({})\n",
                constraint.key, human_lines[0], constraint.severity
            ));

            // Add subsequent lines with proper indentation
            for line in &human_lines[1..] {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    output.push_str(&format!("///   {}\n", trimmed));
                }
            }
        }

        if let Some(expr) = &constraint.expression {
            output.push_str(&format!(
                "///   Expression: `{}`\n",
                escape_doc_comment(expr)
            ));
        }
    }

    output
}

/// Formats example values for documentation.
///
/// # Arguments
///
/// * `examples` - Vector of ElementDefinitionExample
///
/// # Returns
///
/// Returns formatted example documentation.
fn format_examples(examples: &[initial_fhir_model::ElementDefinitionExample]) -> String {
    if examples.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("/// ## Examples\n");

    for example in examples {
        output.push_str(&format!(
            "/// - {}: {:?}\n",
            escape_doc_comment(&example.label),
            example.value
        ));
    }

    output
}

/// Formats binding information for documentation.
///
/// # Arguments
///
/// * `binding` - Optional ElementDefinitionBinding
///
/// # Returns
///
/// Returns formatted binding documentation.
fn format_binding(binding: Option<&initial_fhir_model::ElementDefinitionBinding>) -> String {
    if let Some(b) = binding {
        let mut output = String::new();
        output.push_str("/// ## Binding\n");

        output.push_str(&format!("/// - **Strength**: {}\n", b.strength));

        if let Some(desc) = &b.description {
            let escaped_desc = escape_doc_comment(desc);
            let desc_lines: Vec<&str> = escaped_desc.split('\n').collect();

            if desc_lines.len() == 1 {
                // Single line - output as before
                output.push_str(&format!("/// - **Description**: {}\n", escaped_desc));
            } else {
                // Multi-line - format the first line with "Description:"
                output.push_str(&format!("/// - **Description**: {}\n", desc_lines[0]));

                // Add subsequent lines with proper indentation
                for line in &desc_lines[1..] {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        output.push_str(&format!("///   {}\n", trimmed));
                    }
                }
            }
        }

        if let Some(vs) = &b.value_set {
            output.push_str(&format!("/// - **ValueSet**: {}\n", vs));
        }

        output
    } else {
        String::new()
    }
}

/// Generates documentation comments for a FHIR struct/type from its StructureDefinition.
///
/// This function extracts type-level documentation from a StructureDefinition.
///
/// # Arguments
///
/// * `sd` - The StructureDefinition to document
///
/// # Returns
///
/// Returns a string containing formatted Rust doc comments for the type.
fn generate_struct_documentation(sd: &StructureDefinition) -> String {
    let mut output = String::new();

    // Type name
    output.push_str(&format!(
        "/// FHIR {} type\n",
        capitalize_first_letter(&sd.name)
    ));

    // Description
    if let Some(desc) = &sd.description {
        if !desc.is_empty() {
            output.push_str("/// \n");
            let escaped_desc = escape_doc_comment(desc);
            let formatted_lines = format_doc_content(&escaped_desc, false);

            // Split long descriptions into multiple lines
            for line in formatted_lines {
                if line.is_empty() {
                    output.push_str("/// \n");
                } else if line.len() <= 77 {
                    // Line fits, output as is
                    output.push_str("/// ");
                    output.push_str(&line);
                    output.push('\n');
                } else {
                    // Need to wrap - use word boundaries
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    let mut current_line = String::new();

                    // Check if this line needs indentation
                    // Either it's already indented (continuation) or it's a list item
                    let trimmed_line = line.trim_start();
                    let is_list_item = trimmed_line.starts_with("* ")
                        || trimmed_line.starts_with("- ")
                        || trimmed_line.find(' ').is_some_and(|idx| {
                            let prefix = &trimmed_line[..idx];
                            (prefix.ends_with(')') || prefix.ends_with('.'))
                                && prefix.chars().next().is_some_and(|c| c.is_numeric())
                        });

                    // Determine if this is a numbered list that needs more indentation
                    let is_numbered_list = trimmed_line.find(' ').is_some_and(|idx| {
                        let prefix = &trimmed_line[..idx];
                        (prefix.ends_with(')') || prefix.ends_with('.'))
                            && prefix.chars().next().is_some_and(|c| c.is_numeric())
                    });

                    let indent = if line.starts_with("   ") {
                        "   " // Already has 3 spaces
                    } else if line.starts_with("  ") {
                        "  " // Already has 2 spaces
                    } else if is_numbered_list {
                        // For numbered lists, use 3 spaces for continuation lines
                        "   "
                    } else if is_list_item {
                        // For bullet/dash lists, use 2 spaces
                        "  "
                    } else {
                        ""
                    };

                    // For list items, we don't want to indent the first line
                    let first_line_indent = if is_list_item { "" } else { indent };

                    for word in words.iter() {
                        if current_line.is_empty() {
                            // First word - include indent if needed (but not for bullet points)
                            current_line = if !first_line_indent.is_empty() {
                                format!("{}{}", first_line_indent, word)
                            } else {
                                word.to_string()
                            };
                        } else if current_line.len() + 1 + word.len() <= 77 {
                            current_line.push(' ');
                            current_line.push_str(word);
                        } else {
                            // Output the current line
                            output.push_str("/// ");
                            output.push_str(&current_line);
                            output.push('\n');
                            // Start new line with this word, always use indent for continuations
                            current_line = if !indent.is_empty() {
                                format!("{}{}", indent, word)
                            } else {
                                word.to_string()
                            };
                        }
                    }

                    // Output any remaining content
                    if !current_line.is_empty() {
                        output.push_str("/// ");
                        output.push_str(&current_line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    // Purpose
    if let Some(purpose) = &sd.purpose {
        if !purpose.is_empty() {
            output.push_str("/// \n");
            output.push_str("/// ## Purpose\n");
            let escaped_purpose = escape_doc_comment(purpose);
            let formatted_lines = format_doc_content(&escaped_purpose, false);

            for line in formatted_lines {
                if line.is_empty() {
                    output.push_str("/// \n");
                } else {
                    output.push_str(&format!("/// {}\n", line));
                }
            }
        }
    }

    // Kind and base
    output.push_str("/// \n");
    output.push_str(&format!(
        "/// ## Type: {} type\n",
        capitalize_first_letter(&sd.kind)
    ));

    if sd.r#abstract {
        output.push_str("/// Abstract type (cannot be instantiated directly)\n");
    }

    if let Some(base) = &sd.base_definition {
        output.push_str(&format!("/// Base type: {}\n", base));
    }

    // Status and version
    output.push_str("/// \n");
    output.push_str(&format!("/// ## Status: {}\n", sd.status));

    // FHIR version
    if let Some(version) = &sd.fhir_version {
        output.push_str(&format!("/// FHIR Version: {}\n", version));
    }

    // URL reference
    output.push_str("/// \n");
    output.push_str(&format!("/// See: [{}]({})\n", sd.name, sd.url));

    output
}

/// Generates comprehensive documentation comments for a FHIR element.
///
/// This function extracts all available documentation from an ElementDefinition
/// and formats it into structured Rust doc comments.
///
/// # Arguments
///
/// * `element` - The ElementDefinition to document
///
/// # Returns
///
/// Returns a string containing formatted Rust doc comments.
/// IMPORTANT: Every line in the returned string MUST start with "///"
fn generate_element_documentation(element: &ElementDefinition) -> String {
    let mut output = String::new();

    // Short description (primary doc comment)
    if let Some(short) = &element.short {
        output.push_str(&format!("/// {}\n", escape_doc_comment(short)));
    }

    // Full definition
    if let Some(definition) = &element.definition {
        if !definition.is_empty() {
            output.push_str("/// \n");
            let escaped_definition = escape_doc_comment(definition);
            let formatted_lines = format_doc_content(&escaped_definition, false);

            // Process each formatted line
            for line in formatted_lines {
                if line.is_empty() {
                    output.push_str("/// \n");
                } else if line.len() <= 77 {
                    // Line fits, output as is
                    output.push_str("/// ");
                    output.push_str(&line);
                    output.push('\n');
                } else {
                    // Need to wrap - use word boundaries
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    let mut current_line = String::new();

                    // Check if this line needs indentation
                    // Either it's already indented (continuation) or it's a list item
                    let trimmed_line = line.trim_start();
                    let is_list_item = trimmed_line.starts_with("* ")
                        || trimmed_line.starts_with("- ")
                        || trimmed_line.find(' ').is_some_and(|idx| {
                            let prefix = &trimmed_line[..idx];
                            (prefix.ends_with(')') || prefix.ends_with('.'))
                                && prefix.chars().next().is_some_and(|c| c.is_numeric())
                        });

                    // Determine if this is a numbered list that needs more indentation
                    let is_numbered_list = trimmed_line.find(' ').is_some_and(|idx| {
                        let prefix = &trimmed_line[..idx];
                        (prefix.ends_with(')') || prefix.ends_with('.'))
                            && prefix.chars().next().is_some_and(|c| c.is_numeric())
                    });

                    let indent = if line.starts_with("   ") {
                        "   " // Already has 3 spaces
                    } else if line.starts_with("  ") {
                        "  " // Already has 2 spaces
                    } else if is_numbered_list {
                        // For numbered lists, use 3 spaces for continuation lines
                        "   "
                    } else if is_list_item {
                        // For bullet/dash lists, use 2 spaces
                        "  "
                    } else {
                        ""
                    };

                    // For list items, we don't want to indent the first line
                    let first_line_indent = if is_list_item { "" } else { indent };

                    for word in words.iter() {
                        if current_line.is_empty() {
                            // First word - include indent if needed (but not for bullet points)
                            current_line = if !first_line_indent.is_empty() {
                                format!("{}{}", first_line_indent, word)
                            } else {
                                word.to_string()
                            };
                        } else if current_line.len() + 1 + word.len() <= 77 {
                            current_line.push(' ');
                            current_line.push_str(word);
                        } else {
                            // Output the current line
                            output.push_str("/// ");
                            output.push_str(&current_line);
                            output.push('\n');
                            // Start new line with this word, always use indent for continuations
                            current_line = if !indent.is_empty() {
                                format!("{}{}", indent, word)
                            } else {
                                word.to_string()
                            };
                        }
                    }

                    // Output any remaining content
                    if !current_line.is_empty() {
                        output.push_str("/// ");
                        output.push_str(&current_line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    // Requirements
    if let Some(requirements) = &element.requirements {
        if !requirements.is_empty() {
            output.push_str("/// \n");
            output.push_str("/// ## Requirements\n");
            let escaped_requirements = escape_doc_comment(requirements);
            let formatted_lines = format_doc_content(&escaped_requirements, false);

            for line in formatted_lines {
                if line.is_empty() {
                    output.push_str("/// \n");
                } else if line.len() <= 77 {
                    // Line fits, output as is
                    output.push_str("/// ");
                    output.push_str(&line);
                    output.push('\n');
                } else {
                    // Need to wrap - use word boundaries
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    let mut current_line = String::new();

                    // Check if this line needs indentation
                    // Either it's already indented (continuation) or it's a list item
                    let trimmed_line = line.trim_start();
                    let is_list_item = trimmed_line.starts_with("* ")
                        || trimmed_line.starts_with("- ")
                        || trimmed_line.find(' ').is_some_and(|idx| {
                            let prefix = &trimmed_line[..idx];
                            (prefix.ends_with(')') || prefix.ends_with('.'))
                                && prefix.chars().next().is_some_and(|c| c.is_numeric())
                        });

                    // Determine if this is a numbered list that needs more indentation
                    let is_numbered_list = trimmed_line.find(' ').is_some_and(|idx| {
                        let prefix = &trimmed_line[..idx];
                        (prefix.ends_with(')') || prefix.ends_with('.'))
                            && prefix.chars().next().is_some_and(|c| c.is_numeric())
                    });

                    let indent = if line.starts_with("   ") {
                        "   " // Already has 3 spaces
                    } else if line.starts_with("  ") {
                        "  " // Already has 2 spaces
                    } else if is_numbered_list {
                        // For numbered lists, use 3 spaces for continuation lines
                        "   "
                    } else if is_list_item {
                        // For bullet/dash lists, use 2 spaces
                        "  "
                    } else {
                        ""
                    };

                    // For list items, we don't want to indent the first line
                    let first_line_indent = if is_list_item { "" } else { indent };

                    for word in words.iter() {
                        if current_line.is_empty() {
                            // First word - include indent if needed (but not for bullet points)
                            current_line = if !first_line_indent.is_empty() {
                                format!("{}{}", first_line_indent, word)
                            } else {
                                word.to_string()
                            };
                        } else if current_line.len() + 1 + word.len() <= 77 {
                            current_line.push(' ');
                            current_line.push_str(word);
                        } else {
                            // Output the current line
                            output.push_str("/// ");
                            output.push_str(&current_line);
                            output.push('\n');
                            // Start new line with this word, always use indent for continuations
                            current_line = if !indent.is_empty() {
                                format!("{}{}", indent, word)
                            } else {
                                word.to_string()
                            };
                        }
                    }

                    // Output any remaining content
                    if !current_line.is_empty() {
                        output.push_str("/// ");
                        output.push_str(&current_line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    // Implementation comments
    if let Some(comment) = &element.comment {
        if !comment.is_empty() {
            output.push_str("/// \n");
            output.push_str("/// ## Implementation Notes\n");
            let escaped_comment = escape_doc_comment(comment);
            let formatted_lines = format_doc_content(&escaped_comment, false);

            for line in formatted_lines {
                if line.is_empty() {
                    output.push_str("/// \n");
                } else if line.len() <= 77 {
                    // Line fits, output as is
                    output.push_str("/// ");
                    output.push_str(&line);
                    output.push('\n');
                } else {
                    // Need to wrap - use word boundaries
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    let mut current_line = String::new();

                    // Check if this line needs indentation
                    // Either it's already indented (continuation) or it's a list item
                    let trimmed_line = line.trim_start();
                    let is_list_item = trimmed_line.starts_with("* ")
                        || trimmed_line.starts_with("- ")
                        || trimmed_line.find(' ').is_some_and(|idx| {
                            let prefix = &trimmed_line[..idx];
                            (prefix.ends_with(')') || prefix.ends_with('.'))
                                && prefix.chars().next().is_some_and(|c| c.is_numeric())
                        });

                    // Determine if this is a numbered list that needs more indentation
                    let is_numbered_list = trimmed_line.find(' ').is_some_and(|idx| {
                        let prefix = &trimmed_line[..idx];
                        (prefix.ends_with(')') || prefix.ends_with('.'))
                            && prefix.chars().next().is_some_and(|c| c.is_numeric())
                    });

                    let indent = if line.starts_with("   ") {
                        "   " // Already has 3 spaces
                    } else if line.starts_with("  ") {
                        "  " // Already has 2 spaces
                    } else if is_numbered_list {
                        // For numbered lists, use 3 spaces for continuation lines
                        "   "
                    } else if is_list_item {
                        // For bullet/dash lists, use 2 spaces
                        "  "
                    } else {
                        ""
                    };

                    // For list items, we don't want to indent the first line
                    let first_line_indent = if is_list_item { "" } else { indent };

                    for word in words.iter() {
                        if current_line.is_empty() {
                            // First word - include indent if needed (but not for bullet points)
                            current_line = if !first_line_indent.is_empty() {
                                format!("{}{}", first_line_indent, word)
                            } else {
                                word.to_string()
                            };
                        } else if current_line.len() + 1 + word.len() <= 77 {
                            current_line.push(' ');
                            current_line.push_str(word);
                        } else {
                            // Output the current line
                            output.push_str("/// ");
                            output.push_str(&current_line);
                            output.push('\n');
                            // Start new line with this word, always use indent for continuations
                            current_line = if !indent.is_empty() {
                                format!("{}{}", indent, word)
                            } else {
                                word.to_string()
                            };
                        }
                    }

                    // Output any remaining content
                    if !current_line.is_empty() {
                        output.push_str("/// ");
                        output.push_str(&current_line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    // Cardinality
    let cardinality = format_cardinality(element.min, element.max.as_deref());
    output.push_str("/// \n");
    output.push_str(&format!("/// ## Cardinality: {}\n", cardinality));

    // Special semantics
    let mut special_semantics = Vec::new();

    if element.is_modifier == Some(true) {
        let mut modifier_text = "Modifier element".to_string();
        if let Some(reason) = &element.is_modifier_reason {
            modifier_text.push_str(&format!(" - {}", escape_doc_comment(reason)));
        }
        special_semantics.push(modifier_text);
    }

    if element.is_summary == Some(true) {
        special_semantics.push("Included in summary".to_string());
    }

    if element.must_support == Some(true) {
        special_semantics.push("Must be supported".to_string());
    }

    if let Some(meaning) = &element.meaning_when_missing {
        special_semantics.push(format!("When missing: {}", escape_doc_comment(meaning)));
    }

    if let Some(order) = &element.order_meaning {
        special_semantics.push(format!("Order meaning: {}", escape_doc_comment(order)));
    }

    if !special_semantics.is_empty() {
        output.push_str("/// \n");
        output.push_str("/// ## Special Semantics\n");
        for semantic in special_semantics {
            output.push_str(&format!("/// - {}\n", semantic));
        }
    }

    // Constraints
    if let Some(constraints) = &element.constraint {
        let constraint_doc = format_constraints(constraints);
        if !constraint_doc.is_empty() {
            output.push_str("/// \n");
            output.push_str(&constraint_doc);
        }
    }

    // Examples
    if let Some(examples) = &element.example {
        let example_doc = format_examples(examples);
        if !example_doc.is_empty() {
            output.push_str("/// \n");
            output.push_str(&example_doc);
        }
    }

    // Binding
    let binding_doc = format_binding(element.binding.as_ref());
    if !binding_doc.is_empty() {
        output.push_str("/// \n");
        output.push_str(&binding_doc);
    }

    // Aliases
    if let Some(aliases) = &element.alias {
        if !aliases.is_empty() {
            output.push_str("/// \n");
            output.push_str("/// ## Aliases\n");

            // Handle aliases that might contain newlines
            let all_aliases = aliases.join(", ");
            let escaped_aliases = escape_doc_comment(&all_aliases);

            // Split on newlines and ensure each line has the /// prefix
            for line in escaped_aliases.split('\n') {
                if line.trim().is_empty() {
                    output.push_str("/// \n");
                } else {
                    output.push_str(&format!("/// {}\n", line));
                }
            }
        }
    }

    // Conditions
    if let Some(conditions) = &element.condition {
        if !conditions.is_empty() {
            output.push_str("/// \n");
            output.push_str("/// ## Conditions\n");
            output.push_str(&format!("/// Used when: {}\n", conditions.join(", ")));
        }
    }

    // Validate that all non-empty lines have the /// prefix
    let validated_output = output.lines()
        .enumerate()
        .map(|(i, line)| {
            if line.trim().is_empty() {
                "/// ".to_string()
            } else if line.starts_with("///") {
                line.to_string()
            } else {
                // This should never happen, but if it does, add the prefix
                eprintln!("ERROR in generate_element_documentation for {}: Line {} missing /// prefix: {}", 
                    &element.path, i, line);
                format!("/// {}", line)
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    if !validated_output.is_empty() && !validated_output.ends_with('\n') {
        format!("{}\n", validated_output)
    } else {
        validated_output
    }
}

/// Converts a FHIR StructureDefinition to Rust code.
///
/// This function is the main entry point for converting a single StructureDefinition
/// into its corresponding Rust representation, handling both primitive and complex types.
///
/// # Arguments
///
/// * `sd` - The StructureDefinition to convert
/// * `cycles` - Set of detected circular dependencies that need special handling
///
/// # Returns
///
/// Returns a string containing the generated Rust code for this structure.
///
/// # Type Handling
///
/// - **Primitive types**: Generates type aliases using `Element<T, Extension>`
/// - **Complex types**: Generates full struct definitions with all fields
/// - **Resources**: Generates structs that can be included in the Resource enum
fn structure_definition_to_rust(
    sd: &StructureDefinition,
    cycles: &std::collections::HashSet<(String, String)>,
) -> String {
    let mut output = String::new();

    // Handle primitive types differently
    if is_primitive_type(sd) {
        return generate_primitive_type(sd);
    }

    // Generate struct documentation for the main type
    let struct_doc = generate_struct_documentation(sd);

    // Process elements for complex types and resources
    if let Some(snapshot) = &sd.snapshot {
        if let Some(elements) = &snapshot.element {
            let mut processed_types = std::collections::HashSet::new();
            // Find the root element to get its documentation
            let root_element_doc = elements
                .iter()
                .find(|e| e.path == sd.name)
                .map(generate_element_documentation)
                .unwrap_or_default();

            process_elements(
                elements,
                &mut output,
                &mut processed_types,
                cycles,
                &sd.name,
                if !struct_doc.is_empty() {
                    Some(&struct_doc)
                } else if !root_element_doc.is_empty() {
                    Some(&root_element_doc)
                } else {
                    None
                },
            );
        }
    }
    output
}

/// Generates Rust type aliases for FHIR primitive types.
///
/// FHIR primitive types are mapped to appropriate Rust types and wrapped in
/// the `Element<T, Extension>` container to handle FHIR's extension mechanism.
///
/// # Arguments
///
/// * `sd` - The StructureDefinition for the primitive type
///
/// # Returns
///
/// Returns a string containing the type alias definition.
///
/// # Type Mappings
///
/// - `boolean` → `Element<bool, Extension>`
/// - `integer` → `Element<i32, Extension>`
/// - `decimal` → `DecimalElement<Extension>` (special handling for precision)
/// - `string`/`code`/`uri` → `Element<String, Extension>`
/// - Date/time types → `Element<PrecisionDate/DateTime/Time, Extension>` (precision-aware types)
///
/// # Note
///
/// This function must be kept in sync with `extract_inner_element_type` in
/// `fhir_macro/src/lib.rs` to ensure consistent type handling.
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
        "date" => "crate::PrecisionDate",
        "dateTime" => "crate::PrecisionDateTime",
        "instant" => "crate::PrecisionInstant",
        "time" => "crate::PrecisionTime",
        _ => "std::string::String",
    };

    // Add type-specific documentation
    match type_name.as_str() {
        "boolean" => {
            output.push_str("/// FHIR primitive type for boolean values (true/false)\n");
        }
        "integer" => {
            output.push_str("/// FHIR primitive type for whole number values\n");
        }
        "positiveInt" => {
            output.push_str("/// FHIR primitive type for positive whole number values (> 0)\n");
        }
        "unsignedInt" => {
            output
                .push_str("/// FHIR primitive type for non-negative whole number values (>= 0)\n");
        }
        "decimal" => {
            output
                .push_str("/// FHIR primitive type for decimal numbers with arbitrary precision\n");
        }
        "string" => {
            output.push_str("/// FHIR primitive type for character sequences\n");
        }
        "code" => {
            output.push_str("/// FHIR primitive type for coded values drawn from a defined set\n");
        }
        "uri" => {
            output
                .push_str("/// FHIR primitive type for Uniform Resource Identifiers (RFC 3986)\n");
        }
        "url" => {
            output.push_str("/// FHIR primitive type for Uniform Resource Locators\n");
        }
        "canonical" => {
            output.push_str(
                "/// FHIR primitive type for canonical URLs that reference FHIR resources\n",
            );
        }
        "base64Binary" => {
            output.push_str("/// FHIR primitive type for base64-encoded binary data\n");
        }
        "date" => {
            output.push_str("/// FHIR primitive type for date values (year, month, day)\n");
        }
        "dateTime" => {
            output.push_str("/// FHIR primitive type for date and time values\n");
        }
        "instant" => {
            output.push_str(
                "/// FHIR primitive type for instant in time values (to millisecond precision)\n",
            );
        }
        "time" => {
            output.push_str("/// FHIR primitive type for time of day values\n");
        }
        "id" => {
            output.push_str("/// FHIR primitive type for logical IDs within FHIR resources\n");
        }
        "oid" => {
            output.push_str("/// FHIR primitive type for Object Identifiers (OIDs)\n");
        }
        "uuid" => {
            output.push_str("/// FHIR primitive type for Universally Unique Identifiers (UUIDs)\n");
        }
        "markdown" => {
            output.push_str("/// FHIR primitive type for markdown-formatted text\n");
        }
        "xhtml" => {
            output
                .push_str("/// FHIR primitive type for XHTML-formatted text with limited subset\n");
        }
        _ => {
            output.push_str(&format!(
                "/// FHIR primitive type {}\n",
                capitalize_first_letter(type_name)
            ));
        }
    }

    // Add description if available
    if let Some(desc) = &sd.description {
        if !desc.is_empty() {
            output.push_str("/// \n");
            output.push_str(&format!("/// {}\n", escape_doc_comment(desc)));
        }
    }

    // Add reference to the spec
    output.push_str(&format!("/// \n/// See: [{}]({})\n", sd.name, sd.url));

    // Generate a type alias using Element<T, Extension> or DecimalElement<Extension> for decimal type
    if type_name == "decimal" {
        output.push_str("pub type Decimal = DecimalElement<Extension>;\n\n");
    } else {
        output.push_str(&format!(
            "pub type {} = Element<{}, Extension>;\n\n",
            capitalize_first_letter(type_name),
            value_type
        ));
        // REMOVED From<T> generation from here to avoid conflicts
    }

    output
}

/// Detects circular dependencies between FHIR types.
///
/// This function analyzes ElementDefinitions to find circular references between
/// types where both directions have a cardinality of 1 (max="1"). Such cycles
/// would cause infinite-sized structs in Rust, so they need to be broken with
/// `Box<T>` pointers.
///
/// # Arguments
///
/// * `elements` - All ElementDefinitions to analyze for cycles
///
/// # Returns
///
/// Returns a set of tuples representing detected cycles. Each tuple contains
/// the two type names that form a cycle.
///
/// # Cycle Detection Logic
///
/// 1. Builds a dependency graph of type relationships with max="1"
/// 2. Finds bidirectional dependencies (A → B and B → A)
/// 3. Adds special cases like Bundle → Resource for known problematic cycles
///
/// # Example
///
/// If `Identifier` has a field of type `Reference` and `Reference` has a field
/// of type `Identifier`, both with max="1", this creates a cycle that must be
/// broken by boxing one of the references.
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
                if !from_type.is_empty() && element.max.as_deref() == Some("1") {
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
        .any(|e| e.id.as_ref().is_some_and(|id| id == "Bundle.issues"))
    {
        cycles.insert(("Bundle".to_string(), "Resource".to_string()));
    }

    cycles
}

/// Processes ElementDefinitions to generate Rust struct and enum definitions.
///
/// This function groups related ElementDefinitions by their parent path and generates
/// the corresponding Rust types, including handling of choice types (polymorphic elements).
///
/// # Arguments
///
/// * `elements` - Slice of ElementDefinitions to process
/// * `output` - Mutable string to append generated code to
/// * `processed_types` - Set tracking which types have already been generated
/// * `cycles` - Set of detected circular dependencies requiring Box<T> handling
/// * `root_type_name` - The name of the root type (e.g., "Patient")
/// * `root_doc` - Optional documentation for the root type
///
/// # Process Overview
///
/// 1. **Grouping**: Groups elements by their parent path (e.g., "Patient.name")
/// 2. **Choice Types**: Generates enums for choice elements ending in "\[x\]"
/// 3. **Structs**: Generates struct definitions with all fields
/// 4. **Deduplication**: Ensures each type is only generated once
///
/// # Generated Code Features
///
/// - Derives for Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default
/// - Choice type enums with proper serde renaming
/// - Cycle-breaking with Box<T> where needed
/// - Optional wrapping for elements with min=0
fn process_elements(
    elements: &[ElementDefinition],
    output: &mut String,
    processed_types: &mut std::collections::HashSet<String>,
    cycles: &std::collections::HashSet<(String, String)>,
    root_type_name: &str,
    root_doc: Option<&str>,
) {
    // Group elements by their parent path
    let mut element_groups: std::collections::HashMap<String, Vec<&ElementDefinition>> =
        std::collections::HashMap::new();

    // First pass - collect all type names that will be generated
    for element in elements {
        let path_parts: Vec<&str> = element.path.split('.').collect();
        if path_parts.len() > 1 {
            let parent_path = path_parts[..path_parts.len() - 1].join(".");
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
            let base_name = choice
                .path
                .rsplit('.')
                .next()
                .unwrap()
                .trim_end_matches("[x]");

            let enum_name = format!(
                "{}{}",
                capitalize_first_letter(&type_name),
                capitalize_first_letter(base_name)
            );

            // Skip if we've already processed this enum
            if processed_types.contains(&enum_name) {
                continue;
            }
            processed_types.insert(enum_name.clone());

            // Add documentation comment for the enum
            output.push_str(&format!(
                "/// Choice of types for the {}\\[x\\] field in {}\n",
                base_name,
                capitalize_first_letter(&type_name)
            ));

            // Generate enum derives - Add Clone, PartialEq, Eq to all enums
            let enum_derives = ["Debug", "Clone", "PartialEq", "Eq", "FhirSerde", "FhirPath"];
            output.push_str(&format!("#[derive({})]\n", enum_derives.join(", ")));

            // Add choice element attribute to mark this as a choice type
            output.push_str(&format!(
                "#[fhir_choice_element(base_name = \"{}\")]\n",
                base_name
            ));

            // Add other serde attributes and enum definition
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
                    output.push_str(&format!(
                        "    #[fhir_serde(rename = \"{}\")]\n",
                        rename_value
                    ));
                    output.push_str(&format!("    {}({}),\n", type_code, type_code));
                }
            }
            output.push_str("}\n\n");
        }

        // Collect all choice element fields for this struct
        let choice_element_fields: Vec<String> = group
            .iter()
            .filter(|e| e.path.ends_with("[x]"))
            .filter_map(|e| e.path.rsplit('.').next())
            .map(|name| name.trim_end_matches("[x]").to_string())
            .collect();

        // Add struct documentation
        if path == *root_type_name {
            // This is the root type, use the provided documentation
            if let Some(doc) = root_doc {
                output.push_str(doc);
            }
        } else {
            // For nested types, try to find the documentation from the element
            if let Some(type_element) = elements.iter().find(|e| e.path == path) {
                let doc = generate_element_documentation(type_element);
                if !doc.is_empty() {
                    output.push_str(&doc);
                }
            } else {
                // Generate a basic doc comment
                output.push_str(&format!(
                    "/// {} sub-type\n",
                    capitalize_first_letter(&type_name)
                ));
            }
        }

        // Generate struct derives - Add Clone, PartialEq, Eq to all structs
        let derives = [
            "Debug",
            "Clone",
            "PartialEq",
            "Eq",
            "FhirSerde",
            "FhirPath",
            "Default",
        ];
        output.push_str(&format!("#[derive({})]\n", derives.join(", ")));

        // Add fhir_resource attribute if there are choice elements
        if !choice_element_fields.is_empty() {
            let choice_elements_str = choice_element_fields.join(",");
            output.push_str(&format!(
                "#[fhir_resource(choice_elements = \"{}\")]\n",
                choice_elements_str
            ));
        }

        // Add other serde attributes and struct definition
        output.push_str(&format!(
            "pub struct {} {{\n",
            capitalize_first_letter(&type_name)
        ));

        for element in &group {
            if let Some(field_name) = element.path.rsplit('.').next() {
                if !field_name.contains("[x]") {
                    generate_element_definition(element, &type_name, output, cycles, elements);
                } else {
                    // For choice types, we've already created an enum, so we just need to add the field
                    // that uses that enum type. We don't need to expand each choice type into separate fields.
                    generate_element_definition(element, &type_name, output, cycles, elements);
                }
            }
        }
        output.push_str("}\n\n");
    }
}

/// Generates a Rust field definition from a FHIR ElementDefinition.
///
/// This function converts a single FHIR element into a Rust struct field,
/// handling type mapping, cardinality, choice types, and circular references.
///
/// # Arguments
///
/// * `element` - The ElementDefinition to convert
/// * `type_name` - Name of the parent type containing this element
/// * `output` - Mutable string to append the field definition to
/// * `cycles` - Set of circular dependencies requiring Box<T> handling
/// * `elements` - All elements (used for resolving content references)
///
/// # Field Generation Features
///
/// - **Type Mapping**: Maps FHIR types to appropriate Rust types
/// - **Cardinality**: Wraps in `Option<T>` for min=0, `Vec<T>` for max="*"
/// - **Choice Types**: Uses generated enum types for polymorphic elements
/// - **Cycle Breaking**: Adds `Box<T>` for circular references
/// - **Serde Attributes**: Adds rename and flatten attributes as needed
/// - **Content References**: Resolves `#id` references to other elements
fn generate_element_definition(
    element: &ElementDefinition,
    type_name: &str,
    output: &mut String,
    cycles: &std::collections::HashSet<(String, String)>,
    elements: &[ElementDefinition],
) {
    if let Some(field_name) = element.path.rsplit('.').next() {
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
                    let ref_id = extract_content_reference_id(content_ref);
                    if let Some(referenced_element) = elements
                        .iter()
                        .find(|e| e.id.as_ref().is_some_and(|id| id == ref_id))
                    {
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
            "http://hl7.org/fhirpath/System.String" => "String",
            "http://hl7.org/fhirpath/System.Integer" => "std::primitive::i32",
            "http://hl7.org/fhirpath/System.Long" => "std::primitive::i64",
            "http://hl7.org/fhirpath/System.Decimal" => "std::primitive::f64",
            "http://hl7.org/fhirpath/System.Date" => "std::string::String",
            "http://hl7.org/fhirpath/System.DateTime" => "std::string::String",
            "http://hl7.org/fhirpath/System.Time" => "std::string::String",
            "http://hl7.org/fhirpath/System.Quantity" => "std::string::String",
            "Element" | "BackboneElement" => &generate_type_name(&element.path),
            // Fix for R6 TestPlan: replace Base with BackboneElement
            // See https://github.com/HeliosSoftware/hfs/issues/11
            "Base" if element.path.contains("TestPlan") => &generate_type_name(&element.path),
            _ => &capitalize_first_letter(&ty.code),
        };

        let base_type = if let Some(content_ref) = &element.content_reference {
            let ref_id = extract_content_reference_id(content_ref);
            if !ref_id.is_empty() {
                generate_type_name(ref_id)
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
                            type_str = format!("Option<Box<{}>>", &type_str[7..type_str.len() - 1]);
                        } else {
                            type_str = format!("Box<{}>", type_str);
                        }
                        break;
                    }
                }
            }
        }

        // Generate documentation for this field
        let doc_comment = generate_element_documentation(element);
        if !doc_comment.is_empty() {
            // Debug: Check for any issues
            if doc_comment
                .lines()
                .any(|line| !line.trim().is_empty() && !line.starts_with("//"))
            {
                eprintln!("\n=== WARNING: Found doc comment with lines missing /// prefix ===");
                eprintln!("Field: {}", element.path);
                eprintln!("Doc comment has {} lines", doc_comment.lines().count());
                for (i, line) in doc_comment.lines().enumerate() {
                    if !line.trim().is_empty() && !line.starts_with("//") {
                        eprintln!("  Line {}: Missing prefix: {:?}", i, line);
                    }
                }
                eprintln!("==================================================\n");
            }

            // Indent all doc comments with 4 spaces
            for line in doc_comment.lines() {
                // Ensure every line is a proper doc comment
                if line.trim().is_empty() {
                    output.push_str("    /// \n");
                } else if line.starts_with("///") {
                    output.push_str(&format!("    {}\n", line));
                } else {
                    // This line doesn't have a doc comment prefix - this is a bug!
                    eprintln!("WARNING: Doc comment line without /// prefix: {}", line);
                    output.push_str(&format!("    /// {}\n", line));
                }
            }
        }

        // Output consolidated serde attributes if any exist
        if !serde_attrs.is_empty() {
            output.push_str(&format!("    #[fhir_serde({})]\n", serde_attrs.join(", ")));
        }

        // For choice fields, strip the [x] from the field name
        let clean_field_name = if rust_field_name.ends_with("[x]") {
            rust_field_name.trim_end_matches("[x]").to_string()
        } else {
            rust_field_name
        };

        // Check if the line would be too long (rustfmt's default max line width is 100)
        // Account for "    pub " (8 chars) + ": " (2 chars) + "," (1 char) = 11 extra chars
        let line_length = 8 + clean_field_name.len() + 2 + type_str.len() + 1;

        if line_length > 100 {
            // For Option<Vec<...>>, rustfmt prefers a specific format
            if type_str.starts_with("Option<Vec<") && type_str.ends_with(">>") {
                // Extract the inner type
                let inner_type = &type_str[11..type_str.len() - 2];
                output.push_str(&format!(
                    "    pub {}: Option<\n        Vec<{}>,\n    >,\n",
                    clean_field_name, inner_type
                ));
            } else if type_str.starts_with("Option<") && type_str.ends_with(">") {
                // For other Option<...> types that are too long
                let inner_type = &type_str[7..type_str.len() - 1];
                output.push_str(&format!(
                    "    pub {}:\n        Option<{}>,\n",
                    clean_field_name, inner_type
                ));
            } else {
                // Break other long type declarations across multiple lines
                output.push_str(&format!(
                    "    pub {}:\n        {},\n",
                    clean_field_name, type_str
                ));
            }
        } else {
            output.push_str(&format!("    pub {}: {},\n", clean_field_name, type_str));
        }
    }
}

/// Extracts the element ID from a contentReference value.
///
/// This function handles both local contentReferences (starting with #) and
/// URL-based contentReferences that include a fragment after #.
///
/// # Arguments
///
/// * `content_ref` - The contentReference value from a FHIR ElementDefinition
///
/// # Returns
///
/// Returns the element ID portion of the contentReference.
///
/// # Examples
///
/// - "#Patient.name" → "Patient.name"
/// - "https://sql-on-fhir.org/ig/StructureDefinition/ViewDefinition#ViewDefinition.select" → "ViewDefinition.select"
/// - "invalid-ref" → ""
fn extract_content_reference_id(content_ref: &str) -> &str {
    if let Some(fragment_start) = content_ref.find('#') {
        let fragment = &content_ref[fragment_start + 1..];
        if !fragment.is_empty() { fragment } else { "" }
    } else {
        ""
    }
}

/// Generates a Rust type name from a FHIR element path.
///
/// This function converts dotted FHIR paths into appropriate Rust type names
/// using PascalCase conventions.
///
/// # Arguments
///
/// * `path` - The FHIR element path (e.g., "Patient.name.given")
///
/// # Returns
///
/// Returns a PascalCase type name suitable for Rust.
///
/// # Examples
///
/// - "Patient" → "Patient"
/// - "Patient.name" → "PatientName"
/// - "Observation.value.quantity" → "ObservationValueQuantity"
///
/// # Note
///
/// The first path segment becomes the base name, and subsequent segments
/// are capitalized and concatenated to create a compound type name.
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
        String::from("Empty path provided to generate_type_name")
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
