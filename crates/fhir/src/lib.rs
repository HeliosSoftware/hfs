pub mod r4;
pub mod r4b;
pub mod r5;
pub mod r6;

/// Represents a generic FHIR resource from any version
#[derive(Debug, Clone)]
pub enum GenericResource {
    /// FHIR Release 4 resource
    R4(r4::Resource),
    /// FHIR Release 4B resource
    R4B(r4b::Resource),
    /// FHIR Release 5 resource
    R5(r5::Resource),
    /// FHIR Release 6 resource
    R6(r6::Resource),
}

impl GenericResource {
    /// Returns the FHIR version of this resource
    pub fn version(&self) -> FhirVersion {
        match self {
            GenericResource::R4(_) => FhirVersion::R4,
            GenericResource::R4B(_) => FhirVersion::R4B,
            GenericResource::R5(_) => FhirVersion::R5,
            GenericResource::R6(_) => FhirVersion::R6,
        }
    }
    
    /// Returns the resource type name as a string
    pub fn resource_type(&self) -> &'static str {
        match self {
            GenericResource::R4(res) => match res {
                r4::Resource::Account(_) => "Account",
                r4::Resource::ActivityDefinition(_) => "ActivityDefinition",
                r4::Resource::AdverseEvent(_) => "AdverseEvent",
                r4::Resource::AllergyIntolerance(_) => "AllergyIntolerance",
                r4::Resource::Appointment(_) => "Appointment",
                r4::Resource::AppointmentResponse(_) => "AppointmentResponse",
                r4::Resource::AuditEvent(_) => "AuditEvent",
                r4::Resource::Basic(_) => "Basic",
                r4::Resource::Binary(_) => "Binary",
                // Add other R4 resource types as needed
                _ => "Unknown",
            },
            GenericResource::R4B(res) => match res {
                r4b::Resource::Account(_) => "Account",
                r4b::Resource::ActivityDefinition(_) => "ActivityDefinition",
                r4b::Resource::AdministrableProductDefinition(_) => "AdministrableProductDefinition",
                r4b::Resource::AdverseEvent(_) => "AdverseEvent",
                r4b::Resource::AllergyIntolerance(_) => "AllergyIntolerance",
                r4b::Resource::Appointment(_) => "Appointment",
                r4b::Resource::AppointmentResponse(_) => "AppointmentResponse",
                r4b::Resource::AuditEvent(_) => "AuditEvent",
                r4b::Resource::Basic(_) => "Basic",
                // Add other R4B resource types as needed
                _ => "Unknown",
            },
            GenericResource::R5(res) => match res {
                r5::Resource::Account(_) => "Account",
                r5::Resource::ActivityDefinition(_) => "ActivityDefinition",
                r5::Resource::ActorDefinition(_) => "ActorDefinition",
                r5::Resource::AdministrableProductDefinition(_) => "AdministrableProductDefinition",
                r5::Resource::AdverseEvent(_) => "AdverseEvent",
                r5::Resource::AllergyIntolerance(_) => "AllergyIntolerance",
                r5::Resource::Appointment(_) => "Appointment",
                r5::Resource::AppointmentResponse(_) => "AppointmentResponse",
                r5::Resource::ArtifactAssessment(_) => "ArtifactAssessment",
                // Add other R5 resource types as needed
                _ => "Unknown",
            },
            GenericResource::R6(res) => "Unknown", // Expand as R6 resources are defined
        }
    }
    
    /// Attempts to convert the resource to JSON
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        match self {
            GenericResource::R4(res) => serde_json::to_value(res),
            GenericResource::R4B(res) => serde_json::to_value(res),
            GenericResource::R5(res) => serde_json::to_value(res),
            GenericResource::R6(res) => serde_json::to_value(res),
        }
    }
}

/// Represents a FHIR specification version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FhirVersion {
    /// FHIR Release 4
    R4,
    /// FHIR Release 4B
    R4B,
    /// FHIR Release 5
    R5,
    /// FHIR Release 6
    R6,
}

impl FhirVersion {
    /// Returns the string representation of the FHIR version
    pub fn as_str(&self) -> &'static str {
        match self {
            FhirVersion::R4 => "r4",
            FhirVersion::R4B => "r4b",
            FhirVersion::R5 => "r5",
            FhirVersion::R6 => "r6",
        }
    }
}

impl std::fmt::Display for FhirVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

// Implement ValueEnum for FhirVersion to support clap
impl clap::ValueEnum for FhirVersion {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            FhirVersion::R4,
            FhirVersion::R4B,
            FhirVersion::R5,
            FhirVersion::R6,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

/// Accesses an attribute from a FHIR resource using a simple path expression
/// 
/// This is a simplified accessor that handles basic dot notation paths like "Patient.id"
/// For more complex FHIRPath expressions, use a full FHIRPath evaluator
pub fn access_attribute(
    resource: &GenericResource, 
    path: &str
) -> Result<Option<serde_json::Value>, String> {
    // Convert the resource to JSON for simplified access
    let json = resource.to_json()
        .map_err(|e| format!("Failed to convert resource to JSON: {}", e))?;
    
    // Split the path into segments
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return Err("Empty path".to_string());
    }
    
    // The first segment should match the resource type
    let resource_type = segments[0];
    if resource_type != resource.resource_type() {
        return Err(format!(
            "Resource type mismatch: expected {}, got {}", 
            resource_type, 
            resource.resource_type()
        ));
    }
    
    // Navigate through the JSON object following the path
    let mut current = &json;
    for segment in segments.iter().skip(1) {
        match current {
            serde_json::Value::Object(map) => {
                // Try both the original segment and the camelCase version
                // (FHIR uses camelCase in JSON)
                if let Some(value) = map.get(*segment) {
                    current = value;
                } else {
                    // Convert snake_case to camelCase for field names
                    let camel_case = to_camel_case(segment);
                    if let Some(value) = map.get(&camel_case) {
                        current = value;
                    } else {
                        // Not found
                        return Ok(None);
                    }
                }
            },
            serde_json::Value::Array(arr) => {
                // If we hit an array, we can't continue with simple path navigation
                return Err(format!("Array encountered at '{}', use FHIRPath for complex paths", segment));
            },
            _ => {
                // We've hit a primitive value but still have path segments
                return Err(format!("Cannot navigate further from primitive value at '{}'", segment));
            }
        }
    }
    
    Ok(Some(current.clone()))
}

/// Converts a snake_case string to camelCase
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    
    result
}
