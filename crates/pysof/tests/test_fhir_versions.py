"""Test suite for FHIR version support in pysof."""

import pytest
from typing import Dict, Any

import pysof


def get_test_view_definition() -> Dict[str, Any]:
    """Return a test ViewDefinition for FHIR version testing."""
    return {
        "resourceType": "ViewDefinition",
        "id": "fhir-version-test",
        "name": "FhirVersionTest",
        "status": "active",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    },
                    {
                        "name": "family_name", 
                        "path": "name.family"
                    }
                ]
            }
        ]
    }


def get_test_bundle() -> Dict[str, Any]:
    """Return a test Bundle for FHIR version testing."""
    return {
        "resourceType": "Bundle",
        "id": "fhir-version-test-bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-patient",
                    "name": [
                        {
                            "family": "TestFamily",
                            "given": ["TestGiven"]
                        }
                    ]
                }
            }
        ]
    }


class TestFhirVersionParameter:
    """Test FHIR version parameter support in API functions."""
    
    def test_default_fhir_version(self) -> None:
        """Test default FHIR version (R4) without explicit parameter."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        # Should work with default R4
        result = pysof.run_view_definition(view, bundle, "json")
        assert isinstance(result, bytes)
        assert len(result) > 0
    
    def test_explicit_r4_version(self) -> None:
        """Test explicit R4 FHIR version parameter."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition(view, bundle, "json", fhir_version="R4")
        assert isinstance(result, bytes)
        assert len(result) > 0
    
    def test_fhir_version_with_options(self) -> None:
        """Test FHIR version parameter with run_view_definition_with_options."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "json", 
            fhir_version="R4",
            limit=10
        )
        assert isinstance(result, bytes)
        assert len(result) > 0
    
    def test_fhir_version_consistency(self) -> None:
        """Test that default and explicit R4 produce same results."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result_default = pysof.run_view_definition(view, bundle, "json")
        result_explicit = pysof.run_view_definition(view, bundle, "json", fhir_version="R4")
        
        # Should produce identical results
        assert result_default == result_explicit


class TestFhirVersionValidation:
    """Test FHIR version validation and error handling."""
    
    def test_view_definition_validation_with_version(self) -> None:
        """Test ViewDefinition validation with FHIR version."""
        view = get_test_view_definition()
        
        # Should validate successfully with R4
        assert pysof.validate_view_definition(view, fhir_version="R4") is True
        
        # Default version should also work
        assert pysof.validate_view_definition(view) is True
    
    def test_bundle_validation_with_version(self) -> None:
        """Test Bundle validation with FHIR version."""
        bundle = get_test_bundle()
        
        # Should validate successfully with R4
        assert pysof.validate_bundle(bundle, fhir_version="R4") is True
        
        # Default version should also work
        assert pysof.validate_bundle(bundle) is True
    
    def test_invalid_fhir_version_rejection(self) -> None:
        """Test that invalid FHIR versions are properly rejected."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        invalid_versions = ["R99", "R1", "R10", "FHIR4", "4", "r4", "R4b"]
        
        for invalid_version in invalid_versions:
            with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
                pysof.run_view_definition(view, bundle, "json", fhir_version=invalid_version)
            
            assert "Unsupported FHIR version" in str(exc_info.value)
            assert invalid_version in str(exc_info.value)
    
    def test_invalid_version_with_options(self) -> None:
        """Test invalid FHIR version with run_view_definition_with_options."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition_with_options(
                view, bundle, "json", 
                fhir_version="InvalidVersion",
                limit=5
            )
        
        assert "Unsupported FHIR version: InvalidVersion" in str(exc_info.value)
    
    def test_invalid_version_validation(self) -> None:
        """Test that validation functions reject invalid FHIR versions."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.validate_view_definition(view, fhir_version="R99")
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.validate_bundle(bundle, fhir_version="R99")


class TestSupportedFhirVersions:
    """Test FHIR version detection and feature compilation support."""
    
    def test_get_supported_versions(self) -> None:
        """Test get_supported_fhir_versions utility function."""
        versions = pysof.get_supported_fhir_versions()
        
        # Should return a list
        assert isinstance(versions, list)
        
        # Should always include R4
        assert "R4" in versions
        
        # Should only contain valid FHIR version strings
        valid_versions = {"R4", "R4B", "R5", "R6"}
        for version in versions:
            assert version in valid_versions
    
    def test_supported_version_usage(self) -> None:
        """Test that all supported versions can be used successfully."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        supported_versions = pysof.get_supported_fhir_versions()
        
        for version in supported_versions:
            # Each supported version should work
            result = pysof.run_view_definition(view, bundle, "json", fhir_version=version)
            assert isinstance(result, bytes)
            assert len(result) > 0
    
    def test_version_compatibility_note(self) -> None:
        """Test version compatibility based on feature compilation."""
        versions = pysof.get_supported_fhir_versions()
        
        # In current build, should at least have R4
        assert len(versions) >= 1
        assert "R4" in versions
        
        # Additional versions depend on compile-time features
        # This test documents the behavior but doesn't enforce specific versions
        # since they depend on how the library was compiled
        
        print(f"Available FHIR versions in this build: {versions}")


class TestFhirVersionEdgeCases:
    """Test edge cases and boundary conditions for FHIR version support."""
    
    def test_empty_fhir_version_string(self) -> None:
        """Test behavior with empty FHIR version string."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "json", fhir_version="")
    
    def test_none_fhir_version(self) -> None:
        """Test that None fhir_version uses default."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        # This should use the default version (R4)
        # Note: This test might not be applicable if the API doesn't accept None
        # but documents expected behavior
        result_default = pysof.run_view_definition(view, bundle, "json")
        
        # Should work the same as explicit R4
        result_explicit = pysof.run_view_definition(view, bundle, "json", fhir_version="R4")
        assert result_default == result_explicit
    
    def test_fhir_version_case_sensitivity(self) -> None:
        """Test that FHIR version parameter is case sensitive."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        # Lowercase should fail
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "json", fhir_version="r4")
        
        # Mixed case should fail
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "json", fhir_version="R4b")
    
    def test_fhir_version_with_different_content_types(self) -> None:
        """Test FHIR version parameter works with all content types."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        content_types = ["csv", "json", "ndjson"]
        
        for content_type in content_types:
            result = pysof.run_view_definition(
                view, bundle, content_type, fhir_version="R4"
            )
            assert isinstance(result, bytes)
            assert len(result) > 0


if __name__ == "__main__":
    pytest.main([__file__])
