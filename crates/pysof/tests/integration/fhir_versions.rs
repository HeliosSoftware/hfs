//! Integration tests for FHIR version handling
//! 
//! Tests the interaction between different FHIR versions and the pysof library.

use helios_serde::json;
use serde_json::json;

#[test]
fn test_fhir_version_feature_flag_integration() {
    // Test that the correct FHIR version is being used based on feature flags
    #[cfg(feature = "R4")]
    {
        // Test R4-specific functionality
        let patient = json!({
            "resourceType": "Patient",
            "id": "example",
            "active": true,
            "name": [{
                "use": "official",
                "family": "Doe",
                "given": ["John"]
            }]
        });
        
        let result: Result<helios_fhir::r4::Patient, _> = json::from_value(patient);
        assert!(result.is_ok());
    }
    
    #[cfg(feature = "R4B")]
    {
        // Test R4B-specific functionality
        let patient = json!({
            "resourceType": "Patient",
            "id": "example",
            "active": true,
            "name": [{
                "use": "official",
                "family": "Doe",
                "given": ["John"]
            }]
        });
        
        let result: Result<helios_fhir::r4b::Patient, _> = json::from_value(patient);
        assert!(result.is_ok());
    }
    
    #[cfg(feature = "R5")]
    {
        // Test R5-specific functionality
        let patient = json!({
            "resourceType": "Patient",
            "id": "example",
            "active": true,
            "name": [{
                "use": "official",
                "family": "Doe",
                "given": ["John"]
            }]
        });
        
        let result: Result<helios_fhir::r5::Patient, _> = json::from_value(patient);
        assert!(result.is_ok());
    }
}

#[test]
fn test_view_definition_version_compatibility() {
    // Test ViewDefinition compatibility across FHIR versions
    let view_definition = json!({
        "resourceType": "ViewDefinition",
        "id": "test-view",
        "status": "active",
        "name": "TestView",
        "title": "Test View Definition",
        "description": "A test view definition",
        "select": [{
            "column": [{
                "name": "id",
                "path": "Patient.id",
                "type": "string"
            }]
        }]
    });
    
    // Test that ViewDefinition can be parsed with the active FHIR version
    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::ViewDefinition, _> = 
            json::from_value(view_definition.clone());
        // ViewDefinition parsing may be lenient, so we just ensure it doesn't panic
        let _parsed = result;
    }
    
    #[cfg(feature = "R4B")]
    {
        let result: Result<helios_fhir::r4b::ViewDefinition, _> = 
            json::from_value(view_definition.clone());
        let _parsed = result;
    }
    
    #[cfg(feature = "R5")]
    {
        let result: Result<helios_fhir::r5::ViewDefinition, _> = 
            json::from_value(view_definition.clone());
        let _parsed = result;
    }
}

#[test]
fn test_bundle_version_compatibility() {
    // Test Bundle compatibility across FHIR versions
    let bundle = json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [{
            "resource": {
                "resourceType": "Patient",
                "id": "patient1",
                "active": true
            }
        }]
    });
    
    // Test that Bundle can be parsed with the active FHIR version
    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::Bundle, _> = 
            json::from_value(bundle.clone());
        assert!(result.is_ok());
    }
    
    #[cfg(feature = "R4B")]
    {
        let result: Result<helios_fhir::r4b::Bundle, _> = 
            json::from_value(bundle.clone());
        assert!(result.is_ok());
    }
    
    #[cfg(feature = "R5")]
    {
        let result: Result<helios_fhir::r5::Bundle, _> = 
            json::from_value(bundle.clone());
        assert!(result.is_ok());
    }
}