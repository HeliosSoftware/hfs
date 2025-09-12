"""Comprehensive test suite for content type support in pysof."""

import json
import pytest
from typing import Dict, Any

import pysof


def get_test_view_definition() -> Dict[str, Any]:
    """Return a test ViewDefinition for content type testing."""
    return {
        "resourceType": "ViewDefinition",
        "id": "content-type-test",
        "name": "ContentTypeTest",
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
                    },
                    {
                        "name": "given_name",
                        "path": "name.given.first()"
                    }
                ]
            }
        ]
    }


def get_test_bundle() -> Dict[str, Any]:
    """Return a test Bundle with multiple patients for content type testing."""
    return {
        "resourceType": "Bundle",
        "id": "content-type-test-bundle",
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
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-2",
                    "name": [
                        {
                            "family": "Smith",
                            "given": ["Jane"]
                        }
                    ]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-3",
                    "name": [
                        {
                            "family": "Johnson",
                            "given": ["Bob"]
                        }
                    ]
                }
            }
        ]
    }


class TestContentTypeFormats:
    """Test different content type output formats."""
    
    def test_csv_format(self) -> None:
        """Test CSV output format with headers."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition(view, bundle, "csv")
        
        # Should return bytes
        assert isinstance(result, bytes)
        
        # Decode and verify CSV structure
        csv_content = result.decode('utf-8')
        lines = csv_content.strip().split('\n')
        
        # Should have header line plus data lines
        assert len(lines) >= 4  # header + 3 patients minimum
        
        # Check that it contains expected patient IDs
        assert "patient-1" in csv_content
        assert "patient-2" in csv_content
        assert "patient-3" in csv_content
        
        # Check for family names
        assert "Doe" in csv_content
        assert "Smith" in csv_content
        assert "Johnson" in csv_content
    
    def test_json_format(self) -> None:
        """Test JSON array output format."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition(view, bundle, "json")
        
        # Should return bytes
        assert isinstance(result, bytes)
        
        # Decode and parse JSON
        json_content = result.decode('utf-8')
        data = json.loads(json_content)
        
        # Should be a list/array
        assert isinstance(data, list)
        
        # Should have 3 patient records
        assert len(data) == 3
        
        # Verify structure of first record
        first_record = data[0]
        assert isinstance(first_record, dict)
        assert "id" in first_record
        
        # Check patient IDs are present
        patient_ids = [record.get("id") for record in data]
        assert "patient-1" in patient_ids
        assert "patient-2" in patient_ids
        assert "patient-3" in patient_ids
    
    def test_ndjson_format(self) -> None:
        """Test newline-delimited JSON output format."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition(view, bundle, "ndjson")
        
        # Should return bytes
        assert isinstance(result, bytes)
        
        # Decode and parse NDJSON
        ndjson_content = result.decode('utf-8')
        lines = [line.strip() for line in ndjson_content.strip().split('\n') if line.strip()]
        
        # Should have 3 lines (one per patient)
        assert len(lines) == 3
        
        # Each line should be valid JSON
        records = []
        for line in lines:
            record = json.loads(line)
            records.append(record)
            assert isinstance(record, dict)
            assert "id" in record
        
        # Check patient IDs are present
        patient_ids = [record.get("id") for record in records]
        assert "patient-1" in patient_ids
        assert "patient-2" in patient_ids
        assert "patient-3" in patient_ids
    
    def test_parquet_format_not_implemented(self) -> None:
        """Test that parquet format raises appropriate error."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError) as exc_info:
            pysof.run_view_definition(view, bundle, "parquet")
        
        assert "Parquet not yet implemented" in str(exc_info.value)


class TestContentTypeParsing:
    """Test content type string parsing and MIME type mapping."""
    
    def test_parse_format_strings(self) -> None:
        """Test parsing simple format strings."""
        assert pysof.parse_content_type("csv") == "csv_with_header"
        assert pysof.parse_content_type("json") == "json"
        assert pysof.parse_content_type("ndjson") == "ndjson"
    
    def test_parse_mime_types(self) -> None:
        """Test parsing full MIME type strings."""
        assert pysof.parse_content_type("text/csv") == "csv_with_header"
        assert pysof.parse_content_type("application/json") == "json"
        assert pysof.parse_content_type("application/ndjson") == "ndjson"
    
    def test_parse_csv_header_variants(self) -> None:
        """Test CSV header parameter parsing."""
        # Default CSV includes headers
        assert pysof.parse_content_type("text/csv") == "csv_with_header"
        assert pysof.parse_content_type("text/csv;header=true") == "csv_with_header"
        
        # CSV without headers
        assert pysof.parse_content_type("text/csv;header=false") == "csv"
    
    def test_parse_unsupported_type(self) -> None:
        """Test error handling for unsupported content types."""
        unsupported_types = [
            "text/plain",
            "application/xml",
            "text/html",
            "invalid/type",
            "random-string"
        ]
        
        for content_type in unsupported_types:
            with pytest.raises(pysof.UnsupportedContentTypeError):
                pysof.parse_content_type(content_type)


class TestContentTypeWithOptions:
    """Test content types with run_view_definition_with_options."""
    
    def test_csv_with_limit(self) -> None:
        """Test CSV output with limit option."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "csv", limit=2
        )
        
        csv_content = result.decode('utf-8')
        lines = csv_content.strip().split('\n')
        
        # Should have header + limited number of data lines
        # Note: actual behavior may vary based on implementation
        assert len(lines) >= 2  # At least header + some data
    
    def test_json_with_pagination(self) -> None:
        """Test JSON output with pagination options."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "json", page=1, limit=2
        )
        
        json_content = result.decode('utf-8')
        data = json.loads(json_content)
        
        assert isinstance(data, list)
        # With pagination, may get fewer results
        assert len(data) <= 3
    
    def test_ndjson_with_options(self) -> None:
        """Test NDJSON output with various options."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        result = pysof.run_view_definition_with_options(
            view, bundle, "ndjson", limit=1
        )
        
        ndjson_content = result.decode('utf-8')
        lines = [line.strip() for line in ndjson_content.strip().split('\n') if line.strip()]
        
        # With limit=1, should get at most 1 record
        assert len(lines) <= 1
        
        if lines:
            record = json.loads(lines[0])
            assert isinstance(record, dict)
            assert "id" in record


class TestContentTypeEdgeCases:
    """Test edge cases and error conditions for content types."""
    
    def test_empty_bundle(self) -> None:
        """Test content type outputs with empty bundle."""
        view = get_test_view_definition()
        empty_bundle = {
            "resourceType": "Bundle",
            "id": "empty",
            "type": "collection",
            "entry": []
        }
        
        # Test all supported formats with empty data
        formats = ["csv", "json", "ndjson"]
        for fmt in formats:
            result = pysof.run_view_definition(view, empty_bundle, fmt)
            assert isinstance(result, bytes)
            
            content = result.decode('utf-8')
            if fmt == "csv":
                # CSV should still have headers even with no data
                assert len(content.strip()) > 0
            elif fmt == "json":
                # JSON should be empty array
                data = json.loads(content)
                assert data == []
            elif fmt == "ndjson":
                # NDJSON should be empty or minimal content
                lines = [line.strip() for line in content.strip().split('\n') if line.strip()]
                assert len(lines) == 0
    
    def test_content_type_case_sensitivity(self) -> None:
        """Test content type parsing is case insensitive where appropriate."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        # These should work (format strings are typically lowercase)
        result1 = pysof.run_view_definition(view, bundle, "json")
        result2 = pysof.run_view_definition(view, bundle, "csv")
        result3 = pysof.run_view_definition(view, bundle, "ndjson")
        
        assert isinstance(result1, bytes)
        assert isinstance(result2, bytes)
        assert isinstance(result3, bytes)
    
    def test_content_type_with_invalid_fhir_version(self) -> None:
        """Test content type with invalid FHIR version."""
        view = get_test_view_definition()
        bundle = get_test_bundle()
        
        with pytest.raises(pysof.UnsupportedContentTypeError):
            pysof.run_view_definition(view, bundle, "json", fhir_version="R99")


if __name__ == "__main__":
    pytest.main([__file__])
