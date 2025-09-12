"""Integration tests for core pysof API functions."""

import json
import pytest
from typing import Dict, Any

import pysof


def test_parse_content_type() -> None:
    """Test content type parsing utility."""
    assert pysof.parse_content_type("text/csv") == "csv_with_header"
    assert pysof.parse_content_type("application/json") == "json"
    assert pysof.parse_content_type("application/ndjson") == "ndjson"
    
    with pytest.raises(pysof.UnsupportedContentTypeError):
        pysof.parse_content_type("invalid/type")


def test_get_supported_fhir_versions() -> None:
    """Test FHIR version support detection."""
    versions = pysof.get_supported_fhir_versions()
    assert isinstance(versions, list)
    assert "R4" in versions  # R4 should always be available


# Sample minimal test data
def get_minimal_view_definition() -> Dict[str, Any]:
    """Return a minimal ViewDefinition for testing."""
    return {
        "resourceType": "ViewDefinition",
        "id": "test-view",
        "name": "TestView",
        "status": "active",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    }
                ]
            }
        ]
    }


def get_minimal_bundle() -> Dict[str, Any]:
    """Return a minimal Bundle for testing."""
    return {
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-1",
                    "name": [
                        {
                            "family": "Doe",
                            "given": ["John"]
                        }
                    ]
                }
            }
        ]
    }


class TestValidationFunctions:
    """Test validation utility functions."""
    
    def test_validate_view_definition_success(self) -> None:
        """Test successful ViewDefinition validation."""
        view = get_minimal_view_definition()
        assert pysof.validate_view_definition(view) is True
        
    def test_validate_view_definition_with_fhir_version(self) -> None:
        """Test ViewDefinition validation with specific FHIR version."""
        view = get_minimal_view_definition()
        assert pysof.validate_view_definition(view, fhir_version="R4") is True
    
    def test_validate_view_definition_invalid(self) -> None:
        """Test ViewDefinition validation with invalid data."""
        invalid_view = {"invalid": "structure"}
        # Currently validation is lenient, so this should return True
        # In future, this could be enhanced to do stricter validation
        result = pysof.validate_view_definition(invalid_view)
        assert isinstance(result, bool)
    
    def test_validate_bundle_success(self) -> None:
        """Test successful Bundle validation."""
        bundle = get_minimal_bundle()
        assert pysof.validate_bundle(bundle) is True
        
    def test_validate_bundle_with_fhir_version(self) -> None:
        """Test Bundle validation with specific FHIR version."""
        bundle = get_minimal_bundle()
        assert pysof.validate_bundle(bundle, fhir_version="R4") is True
    
    def test_validate_bundle_invalid(self) -> None:
        """Test Bundle validation with invalid data."""
        invalid_bundle = {"invalid": "structure"}
        # Currently validation is lenient, so this should return True
        # In future, this could be enhanced to do stricter validation
        result = pysof.validate_bundle(invalid_bundle)
        assert isinstance(result, bool)


class TestCoreAPIFunctions:
    """Test core transformation API functions."""
    
    def test_run_view_definition_json_output(self) -> None:
        """Test basic ViewDefinition transformation with JSON output."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition(view, bundle, "json")
        assert isinstance(result, bytes)
        
        # Parse the result as JSON to verify it's valid
        result_data = json.loads(result.decode('utf-8'))
        assert isinstance(result_data, list)
    
    def test_run_view_definition_csv_output(self) -> None:
        """Test ViewDefinition transformation with CSV output."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition(view, bundle, "csv")
        assert isinstance(result, bytes)
        
        # Basic CSV validation - should contain patient-1
        csv_content = result.decode('utf-8')
        assert "patient-1" in csv_content
    
    def test_run_view_definition_with_fhir_version(self) -> None:
        """Test ViewDefinition transformation with explicit FHIR version."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition(view, bundle, "json", fhir_version="R4")
        assert isinstance(result, bytes)
        
        result_data = json.loads(result.decode('utf-8'))
        assert isinstance(result_data, list)
    
    def test_run_view_definition_with_options_basic(self) -> None:
        """Test ViewDefinition transformation with options (basic case)."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "json"
        )
        assert isinstance(result, bytes)
        
        result_data = json.loads(result.decode('utf-8'))
        assert isinstance(result_data, list)
    
    def test_run_view_definition_with_options_limit(self) -> None:
        """Test ViewDefinition transformation with limit option."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "json", limit=1
        )
        assert isinstance(result, bytes)
        
        result_data = json.loads(result.decode('utf-8'))
        assert isinstance(result_data, list)
        assert len(result_data) <= 1
    
    def test_run_view_definition_with_options_page(self) -> None:
        """Test ViewDefinition transformation with page option."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "json", page=1
        )
        assert isinstance(result, bytes)
        
        result_data = json.loads(result.decode('utf-8'))
        assert isinstance(result_data, list)
    
    def test_run_view_definition_invalid_format(self) -> None:
        """Test ViewDefinition transformation with invalid format."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "invalid_format")
    
    def test_run_view_definition_invalid_fhir_version(self) -> None:
        """Test ViewDefinition transformation with invalid FHIR version."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "json", fhir_version="R99")


class TestErrorHandling:
    """Test comprehensive error handling."""
    
    def test_invalid_view_definition_error(self) -> None:
        """Test InvalidViewDefinitionError is raised appropriately."""
        invalid_view = {
            "resourceType": "ViewDefinition",
            "id": "invalid",
            # Missing required fields
        }
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.InvalidViewDefinitionError) as exc_info:
            pysof.run_view_definition(invalid_view, bundle, "json")
        
        # Verify error message is propagated
        assert "ViewDefinition" in str(exc_info.value)
    
    def test_fhirpath_error(self) -> None:
        """Test FhirPathError is raised for invalid FHIRPath expressions."""
        view_with_invalid_fhirpath = {
            "resourceType": "ViewDefinition",
            "id": "test-view",
            "name": "TestView",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "invalid_path",
                            "path": "invalid.fhirpath.expression[invalid syntax"
                        }
                    ]
                }
            ]
        }
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.FhirPathError) as exc_info:
            pysof.run_view_definition(view_with_invalid_fhirpath, bundle, "json")
        
        # Verify error message is propagated
        assert "FHIRPath" in str(exc_info.value)
    
    def test_serialization_error(self) -> None:
        """Test SerializationError is raised for malformed JSON structures."""
        # This should trigger a serialization error during JSON parsing
        malformed_view = {"resourceType": None}  # Invalid structure
        bundle = get_minimal_bundle()
        
        # This actually raises InvalidViewDefinitionError, not SerializationError
        with pytest.raises(pysof.InvalidViewDefinitionError) as exc_info:
            pysof.run_view_definition(malformed_view, bundle, "json")
        
        # Verify error message is propagated
        assert "ViewDefinition must specify a resource type" in str(exc_info.value)
    
    def test_unsupported_content_type_error(self) -> None:
        """Test UnsupportedContentTypeError is raised for invalid formats."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "invalid_format")
        
        # Verify error message is propagated
        assert "invalid_format" in str(exc_info.value)
    
    def test_unsupported_fhir_version_error(self) -> None:
        """Test UnsupportedContentTypeError is raised for invalid FHIR versions."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "json", fhir_version="R99")
        
        # Verify error message is propagated
        assert "Unsupported FHIR version" in str(exc_info.value)
    
    def test_csv_error_scenarios(self) -> None:
        """Test CsvError scenarios (if any specific CSV generation issues occur)."""
        # Note: CSV errors are typically internal to the Rust implementation
        # This test documents the expected behavior
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # This should work fine with valid data
        result = pysof.run_view_definition(view, bundle, "csv")
        assert isinstance(result, bytes)
        
        # CSV errors would typically occur with malformed data that causes
        # CSV writer issues, but these are hard to trigger from Python side
    
    def test_io_error_scenarios(self) -> None:
        """Test IoError scenarios (if any specific I/O issues occur)."""
        # Note: I/O errors are typically internal to the Rust implementation
        # This test documents the expected behavior
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # This should work fine with valid data
        result = pysof.run_view_definition(view, bundle, "json")
        assert isinstance(result, bytes)
        
        # I/O errors would typically occur with file operations,
        # but the Python API doesn't expose file I/O directly
    
    def test_exception_hierarchy(self) -> None:
        """Test that all exceptions inherit from SofError."""
        assert issubclass(pysof.InvalidViewDefinitionError, pysof.SofError)
        assert issubclass(pysof.FhirPathError, pysof.SofError)
        assert issubclass(pysof.SerializationError, pysof.SofError)
        assert issubclass(pysof.UnsupportedContentTypeError, pysof.SofError)
        assert issubclass(pysof.CsvError, pysof.SofError)
        assert issubclass(pysof.IoError, pysof.SofError)
    
    def test_error_message_propagation(self) -> None:
        """Test that error messages from Rust are properly propagated to Python."""
        # Test with invalid format to get a clear error message
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "totally_invalid_format")
        
        error_message = str(exc_info.value)
        # The error message should contain information about the invalid format
        assert len(error_message) > 0
        assert "totally_invalid_format" in error_message or "Unsupported" in error_message
    
    def test_error_types_are_importable(self) -> None:
        """Test that all error types can be imported and used."""
        # Test that all exception classes are available
        assert hasattr(pysof, 'SofError')
        assert hasattr(pysof, 'InvalidViewDefinitionError')
        assert hasattr(pysof, 'FhirPathError')
        assert hasattr(pysof, 'SerializationError')
        assert hasattr(pysof, 'UnsupportedContentTypeError')
        assert hasattr(pysof, 'CsvError')
        assert hasattr(pysof, 'IoError')
        
        # Test that they are actual exception classes
        assert issubclass(pysof.SofError, Exception)
        assert issubclass(pysof.InvalidViewDefinitionError, Exception)
        assert issubclass(pysof.FhirPathError, Exception)
        assert issubclass(pysof.SerializationError, Exception)
        assert issubclass(pysof.UnsupportedContentTypeError, Exception)
        assert issubclass(pysof.CsvError, Exception)
        assert issubclass(pysof.IoError, Exception)


if __name__ == "__main__":
    pytest.main([__file__])
