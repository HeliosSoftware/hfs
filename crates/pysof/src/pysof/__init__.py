"""pysof: Python wrapper for the Helios SOF (SQL on FHIR) toolkit.

This package provides Python bindings for the Rust `sof` crate, enabling
transformation of FHIR resources into tabular data using ViewDefinitions.

Public API (v1 - planned):
    run_view_definition: Transform FHIR data using ViewDefinition
    run_view_definition_with_options: Transform with filtering/pagination
    validate_view_definition: Pre-validate ViewDefinition structure
    validate_bundle: Pre-validate Bundle structure
    get_supported_fhir_versions: List available FHIR versions
    
Exception hierarchy (v1 - planned):
    SofError: Base exception for all pysof errors
    InvalidViewDefinitionError: ViewDefinition validation errors
    FhirPathError: FHIRPath expression evaluation errors
    SerializationError: JSON/data serialization errors
    UnsupportedContentTypeError: Unsupported output format errors
    CsvError: CSV generation errors
    IoError: File/IO related errors
"""

from typing import List

__all__: List[str] = [
    # Core functions (v1 - not yet implemented)
    # "run_view_definition",
    # "run_view_definition_with_options", 
    # "validate_view_definition",
    # "validate_bundle",
    # "get_supported_fhir_versions",
    # "parse_content_type",
    
    # Exception classes (v1 - not yet implemented)
    # "SofError",
    # "InvalidViewDefinitionError", 
    # "FhirPathError",
    # "SerializationError",
    # "UnsupportedContentTypeError",
    # "CsvError",
    # "IoError",
]

__version__ = "0.1.0"


# Placeholder implementations for v0 (will be replaced with Rust bindings in v1)

def get_version() -> str:
    """Return the package version."""
    return __version__


def get_status() -> str:
    """Return the current implementation status."""
    return "v0: Python packaging scaffold. Rust bindings coming in v1."


