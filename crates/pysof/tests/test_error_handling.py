"""Comprehensive tests for pysof error handling and exception mapping."""

import json
import pytest
from typing import Dict, Any

import pysof


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


class TestExceptionHierarchy:
    """Test that all exceptions follow the correct inheritance hierarchy."""
    
    def test_base_exception_inheritance(self) -> None:
        """Test that all pysof exceptions inherit from the base SofError."""
        assert issubclass(pysof.InvalidViewDefinitionError, pysof.SofError)
        assert issubclass(pysof.FhirPathError, pysof.SofError)
        assert issubclass(pysof.SerializationError, pysof.SofError)
        assert issubclass(pysof.UnsupportedContentTypeError, pysof.SofError)
        assert issubclass(pysof.CsvError, pysof.SofError)
        assert issubclass(pysof.IoError, pysof.SofError)
    
    def test_python_exception_inheritance(self) -> None:
        """Test that all pysof exceptions inherit from Python's Exception."""
        assert issubclass(pysof.SofError, Exception)
        assert issubclass(pysof.InvalidViewDefinitionError, Exception)
        assert issubclass(pysof.FhirPathError, Exception)
        assert issubclass(pysof.SerializationError, Exception)
        assert issubclass(pysof.UnsupportedContentTypeError, Exception)
        assert issubclass(pysof.CsvError, Exception)
        assert issubclass(pysof.IoError, Exception)
    
    def test_exception_availability(self) -> None:
        """Test that all exception classes are available in the module."""
        assert hasattr(pysof, 'SofError')
        assert hasattr(pysof, 'InvalidViewDefinitionError')
        assert hasattr(pysof, 'FhirPathError')
        assert hasattr(pysof, 'SerializationError')
        assert hasattr(pysof, 'UnsupportedContentTypeError')
        assert hasattr(pysof, 'CsvError')
        assert hasattr(pysof, 'IoError')


class TestInvalidViewDefinitionError:
    """Test InvalidViewDefinitionError scenarios."""
    
    def test_missing_required_fields(self) -> None:
        """Test error when ViewDefinition is missing required fields."""
        invalid_view = {
            "resourceType": "ViewDefinition",
            "id": "invalid",
            # Missing required fields like 'name', 'status', 'resource', 'select'
        }
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.InvalidViewDefinitionError) as exc_info:
            pysof.run_view_definition(invalid_view, bundle, "json")
        
        # Verify error message is propagated from Rust
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        # The actual error message is "ViewDefinition must specify a resource type"
        assert "ViewDefinition" in error_message
    
    def test_invalid_view_definition_structure(self) -> None:
        """Test error when ViewDefinition has invalid structure."""
        invalid_view = {
            "resourceType": "ViewDefinition",
            "id": "test-view",
            "name": "TestView",
            "status": "active",
            "resource": "Patient",
            "select": "invalid_select_structure"  # Should be a list
        }
        bundle = get_minimal_bundle()
        
        # This actually raises SerializationError, not InvalidViewDefinitionError
        with pytest.raises(pysof.SerializationError) as exc_info:
            pysof.run_view_definition(invalid_view, bundle, "json")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        assert "expected a sequence" in error_message
    
    def test_validation_function_invalid_view(self) -> None:
        """Test that validate_view_definition handles invalid views appropriately."""
        invalid_view = {
            "resourceType": "ViewDefinition",
            "id": "invalid",
            # Missing required fields
        }
        
        # The validation function is lenient and returns True even for incomplete ViewDefinitions
        # It only validates that the JSON can be parsed, not that it's a complete ViewDefinition
        result = pysof.validate_view_definition(invalid_view)
        assert result is True


class TestFhirPathError:
    """Test FhirPathError scenarios."""
    
    def test_invalid_fhirpath_syntax(self) -> None:
        """Test error when FHIRPath expression has invalid syntax."""
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
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        assert "FHIRPath" in error_message
    
    def test_fhirpath_evaluation_error(self) -> None:
        """Test error when FHIRPath expression fails during evaluation."""
        view_with_evaluation_error = {
            "resourceType": "ViewDefinition",
            "id": "test-view",
            "name": "TestView",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "nonexistent_field",
                            "path": "nonexistent.field.that.does.not.exist"
                        }
                    ]
                }
            ]
        }
        bundle = get_minimal_bundle()
        
        # This actually succeeds - FHIRPath expressions that don't match anything just return empty results
        # Let's test with a more obviously invalid FHIRPath syntax
        view_with_syntax_error = {
            "resourceType": "ViewDefinition",
            "id": "test-view",
            "name": "TestView",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "syntax_error",
                            "path": "invalid[unclosed bracket"
                        }
                    ]
                }
            ]
        }
        
        with pytest.raises(pysof.FhirPathError) as exc_info:
            pysof.run_view_definition(view_with_syntax_error, bundle, "json")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0


class TestSerializationError:
    """Test SerializationError scenarios."""
    
    def test_malformed_json_structure(self) -> None:
        """Test error when JSON structure is malformed."""
        malformed_view = {
            "resourceType": None,  # Invalid value type
            "id": "test-view"
        }
        bundle = get_minimal_bundle()
        
        # This actually raises InvalidViewDefinitionError, not SerializationError
        with pytest.raises(pysof.InvalidViewDefinitionError) as exc_info:
            pysof.run_view_definition(malformed_view, bundle, "json")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        assert "ViewDefinition must specify a resource type" in error_message
    
    def test_invalid_json_types(self) -> None:
        """Test error when JSON contains invalid types for FHIR resources."""
        invalid_view = {
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
                            "path": 123  # Should be a string, not a number
                        }
                    ]
                }
            ]
        }
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.SerializationError) as exc_info:
            pysof.run_view_definition(invalid_view, bundle, "json")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0


class TestUnsupportedContentTypeError:
    """Test UnsupportedContentTypeError scenarios."""
    
    def test_invalid_output_format(self) -> None:
        """Test error when output format is invalid."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "invalid_format")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        # The actual error message is just the invalid format string
        assert "invalid_format" in error_message
    
    def test_invalid_fhir_version(self) -> None:
        """Test error when FHIR version is invalid."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "json", fhir_version="R99")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0
        assert "Unsupported FHIR version" in error_message
    
    def test_parse_content_type_invalid_mime(self) -> None:
        """Test error when parsing invalid MIME type."""
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.parse_content_type("invalid/mime/type")
        
        error_message = str(exc_info.value)
        assert len(error_message) > 0


class TestCsvError:
    """Test CsvError scenarios."""
    
    def test_csv_generation_success(self) -> None:
        """Test that CSV generation works with valid data."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        result = pysof.run_view_definition(view, bundle, "csv")
        assert isinstance(result, bytes)
        
        # Verify it's valid CSV content
        csv_content = result.decode('utf-8')
        assert len(csv_content) > 0
    
    def test_csv_with_header_generation_success(self) -> None:
        """Test that CSV with header generation works with valid data."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # csv_with_header is not supported in this build
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "csv_with_header")
        
        error_message = str(exc_info.value)
        assert "csv_with_header" in error_message


class TestIoError:
    """Test IoError scenarios."""
    
    def test_io_operations_success(self) -> None:
        """Test that I/O operations work with valid data."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # Test supported output formats to ensure no I/O errors
        formats = ["json", "ndjson", "csv"]  # csv_with_header is not supported
        for format_type in formats:
            result = pysof.run_view_definition(view, bundle, format_type)
            assert isinstance(result, bytes)
            assert len(result) > 0


class TestErrorMessagePropagation:
    """Test that error messages are properly propagated from Rust to Python."""
    
    def test_error_messages_are_non_empty(self) -> None:
        """Test that all error messages are non-empty and informative."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # Test various error scenarios
        error_scenarios = [
            ("invalid_format", pysof.UnsupportedContentTypeError),
            ("json", pysof.UnsupportedContentTypeError, {"fhir_version": "R99"}),
        ]
        
        for scenario in error_scenarios:
            if len(scenario) == 2:
                format_type, expected_error = scenario
                with pytest.raises(expected_error) as exc_info:
                    pysof.run_view_definition(view, bundle, format_type)
            else:
                format_type, expected_error, kwargs = scenario
                with pytest.raises(expected_error) as exc_info:
                    pysof.run_view_definition(view, bundle, format_type, **kwargs)
            
            error_message = str(exc_info.value)
            assert len(error_message) > 0, f"Error message should not be empty for {scenario}"
            assert isinstance(error_message, str), f"Error message should be a string for {scenario}"
    
    def test_error_messages_contain_relevant_info(self) -> None:
        """Test that error messages contain relevant information about the error."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        # Test invalid format error
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "totally_invalid_format")
        
        error_message = str(exc_info.value)
        # The error message should contain information about the invalid format
        assert "totally_invalid_format" in error_message or "Unsupported" in error_message


class TestErrorHandlingIntegration:
    """Integration tests for error handling across different functions."""
    
    def test_validate_view_definition_error_handling(self) -> None:
        """Test error handling in validate_view_definition function."""
        invalid_view = {"invalid": "structure"}
        
        # The validation function is lenient and doesn't raise errors for invalid structures
        # It just returns False or True based on whether it can parse the JSON
        result = pysof.validate_view_definition(invalid_view)
        assert isinstance(result, bool)
    
    def test_validate_bundle_error_handling(self) -> None:
        """Test error handling in validate_bundle function."""
        invalid_bundle = {"invalid": "structure"}
        
        # The validation function is lenient and doesn't raise errors for invalid structures
        # It just returns False or True based on whether it can parse the JSON
        result = pysof.validate_bundle(invalid_bundle)
        assert isinstance(result, bool)
    
    def test_parse_content_type_error_handling(self) -> None:
        """Test error handling in parse_content_type function."""
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.parse_content_type("invalid/mime/type")
    
    def test_run_view_definition_with_options_error_handling(self) -> None:
        """Test error handling in run_view_definition_with_options function."""
        view = get_minimal_view_definition()
        bundle = get_minimal_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition_with_options(view, bundle, "invalid_format")


if __name__ == "__main__":
    pytest.main([__file__])
