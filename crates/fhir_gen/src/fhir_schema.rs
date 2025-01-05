use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a FHIR Schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FHIRSchema {
    /// URL used to reference the profile
    pub url: String,

    /// Package meta information
    #[serde(rename = "package-meta")]
    pub package_meta : PackageMeta,

    /// Resource type being constrained
    #[serde(rename = "type")]
    pub resource_type: String,

    /// Machine readable name of profile
    pub name: String,

    /// Type of derivation: "specialization" or "constraint"
    pub derivation: DerivationType,

    /// Base profile URL from which schema inherits (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,

    /// Array of excluded elements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Vec<String>>,

    /// Array of required elements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,

    /// Nested elements definitions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<HashMap<String, Element>>,

    /// Constraints on the schema
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<Constraint>>,

    /// Extensions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<Extension>>,
}

/// Represents Package Metadata information
/// Represents an element in the schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub path: String
}

/// Type of schema derivation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DerivationType {
    Specialization,
    Constraint,
}

/// Represents an element in the schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    /// Whether this is an array type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<bool>,

    /// Whether this is a scalar type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scalar: Option<bool>,

    /// Minimum number of items (for arrays)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i32>,

    /// Maximum number of items (for arrays)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i32>,

    /// What this element is a choice of
    #[serde(rename = "choiceOf", skip_serializing_if = "Option::is_none")]
    pub choice_of: Option<String>,

    /// Available choices for this element
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,

    /// Elements that must be present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,

    /// Elements that must be absent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Vec<String>>,

    /// References to other elements
    #[serde(rename = "elementReference", skip_serializing_if = "Option::is_none")]
    pub element_reference: Option<Vec<String>>,

    /// Element type
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub element_type: Option<String>,

    /// Nested elements definitions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<HashMap<String, Element>>,

    /// Fixed value that must match exactly
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<serde_json::Value>,

    /// Pattern value that must be contained
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<serde_json::Value>,

    /// Allowed reference targets
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refers: Option<Vec<String>>,

    /// Whether this element changes resource interpretation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<bool>,

    /// Whether this element must be supported by implementations
    #[serde(rename = "mustSupport", skip_serializing_if = "Option::is_none")]
    pub must_support: Option<bool>,

    /// Whether this element should be included in search summaries
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<bool>,

    /// Value set binding information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
}

/// Represents a terminology binding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    /// URL of the value set
    #[serde(rename = "valueSet")]
    pub value_set: String,

    /// Binding strength (required, extensible, preferred, example)
    pub strength: String,

    /// Code systems included in the value set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codesystems: Option<Vec<String>>,
}

/// Represents a constraint in the schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    // Add constraint properties as needed
    // This will need to be expanded based on constraint documentation
}

/// Represents an extension in the schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    // Add extension properties as needed
    // This will need to be expanded based on extension documentation
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    #[test]
    fn test_ndjson_roundtrip() {
        let file = File::open("resources/1.0.0_hl7.fhir.r4.core#4.0.1_package.ndjson").unwrap();
        let reader = BufReader::new(file);
        
        // Read all schemas from the file
        let schemas: Vec<FHIRSchema> = reader
            .lines()
            .skip(2) // Skip header lines
            .map(|line| {
                let line = line.unwrap();
                serde_json::from_str(&line).unwrap()
            })
            .collect();
        
        // Serialize back to NDJSON format
        let mut output = Vec::new();
        for schema in &schemas {
            let json = serde_json::to_string(&schema).unwrap();
            output.push(json);
        }
        
        // Read original file as string for comparison
        let original = std::fs::read_to_string("resources/1.0.0_hl7.fhir.r4.core#4.0.1_package.ndjson").unwrap();
        let original_lines: Vec<_> = original.lines().skip(2).collect();
        
        // Compare line by line
        assert_eq!(output.len(), original_lines.len(), "Number of records should match");
        
        for (i, (orig, processed)) in original_lines.iter().zip(output.iter()).enumerate() {
            let orig_value: serde_json::Value = serde_json::from_str(orig).unwrap();
            let processed_value: serde_json::Value = serde_json::from_str(processed).unwrap();
            assert_eq!(orig_value, processed_value, "Record {} should match", i);
        }
    }

    #[test]
    fn test_schema_serialization() {
        let schema = FHIRSchema {
            url: "http://example.com/patient".to_string(),
            package_meta: PackageMeta {
                name: "hl7.fhir.r4.core".to_string(),
                version: "4.0.1".to_string(),
                path: "hl7.fhir.r4.core#4.0.1".to_string(),
            },
            base: Some("http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient|6.0.0".to_string()),
            resource_type: "Patient".to_string(),
            name: "ExamplePatient".to_string(),
            derivation: DerivationType::Constraint,
            excluded: None,
            required: None,
            elements: None,
            constraints: None,
            extensions: None,
        };

        let json = serde_json::to_value(&schema).unwrap();
        assert_eq!(json["url"], "http://example.com/patient");
        assert_eq!(json["type"], "Patient");
        assert_eq!(json["name"], "ExamplePatient");
        assert_eq!(json["derivation"], "constraint");
    }
}
