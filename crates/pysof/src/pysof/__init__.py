"""pysof: Python wrapper for the Helios SOF (SQL on FHIR) toolkit.

This package provides Python bindings for the Rust `sof` crate, enabling
transformation of FHIR resources into tabular data using ViewDefinitions.

Public API:
    run_view_definition: Transform FHIR data using ViewDefinition
    run_view_definition_with_options: Transform with filtering/pagination
    validate_view_definition: Pre-validate ViewDefinition structure
    validate_bundle: Pre-validate Bundle structure
    get_supported_fhir_versions: List available FHIR versions
    parse_content_type: Parse MIME types to format strings

Exception hierarchy:
    SofError: Base exception for all pysof errors
    InvalidViewDefinitionError: ViewDefinition validation errors
    FhirPathError: FHIRPath expression evaluation errors
    SerializationError: JSON/data serialization errors
    UnsupportedContentTypeError: Unsupported output format errors
    CsvError: CSV generation errors
    IoError: File/IO related errors
"""

from typing import Any, cast

try:
    # Import the Rust extension module
    from pysof._pysof import (
        CsvError,
        FhirPathError,
        InvalidViewDefinitionError,
        IoError,
        SerializationError,
        # Exception classes
        SofError,
        UnsupportedContentTypeError,
        py_get_supported_fhir_versions,
        py_parse_content_type,
        py_run_view_definition,
        py_run_view_definition_with_options,
        py_validate_bundle,
        py_validate_view_definition,
    )

    # Create Python-friendly wrapper functions
    def run_view_definition(
        view: dict[str, Any],
        bundle: dict[str, Any],
        format: str,
        *,
        fhir_version: str = "R4",
    ) -> bytes:
        """Transform FHIR Bundle data using a ViewDefinition.

        Args:
            view: ViewDefinition resource as a Python dictionary
            bundle: FHIR Bundle resource as a Python dictionary
            format: Output format ("csv", "csv_with_header", "json", "ndjson", "parquet")
            fhir_version: FHIR version to use ("R4", "R4B", "R5", "R6"). Defaults to "R4"

        Returns:
            Transformed data in the requested format as bytes

        Raises:
            InvalidViewDefinitionError: ViewDefinition structure is invalid
            FhirPathError: FHIRPath expression evaluation failed
            SerializationError: JSON parsing/serialization failed
            UnsupportedContentTypeError: Unsupported output format
            CsvError: CSV generation failed
            IoError: I/O operation failed
        """
        return cast(bytes, py_run_view_definition(view, bundle, format, fhir_version))

    def run_view_definition_with_options(
        view: dict[str, Any],
        bundle: dict[str, Any],
        format: str,
        *,
        since: str | None = None,
        limit: int | None = None,
        page: int | None = None,
        fhir_version: str = "R4",
    ) -> bytes:
        """Transform FHIR Bundle data using a ViewDefinition with additional options.

        Args:
            view: ViewDefinition resource as a Python dictionary
            bundle: FHIR Bundle resource as a Python dictionary
            format: Output format ("csv", "csv_with_header", "json", "ndjson", "parquet")
            since: Filter resources modified after this ISO8601 datetime
            limit: Limit the number of results returned
            page: Page number for pagination (1-based)
            fhir_version: FHIR version to use ("R4", "R4B", "R5", "R6"). Defaults to "R4"

        Returns:
            Transformed data in the requested format as bytes

        Raises:
            InvalidViewDefinitionError: ViewDefinition structure is invalid
            FhirPathError: FHIRPath expression evaluation failed
            SerializationError: JSON parsing/serialization failed
            UnsupportedContentTypeError: Unsupported output format
            CsvError: CSV generation failed
            IoError: I/O operation failed
        """
        return cast(
            bytes,
            py_run_view_definition_with_options(
                view,
                bundle,
                format,
                since=since,
                limit=limit,
                page=page,
                fhir_version=fhir_version,
            ),
        )

    def validate_view_definition(
        view: dict[str, Any], *, fhir_version: str = "R4"
    ) -> bool:
        """Validate a ViewDefinition structure without executing it.

        Args:
            view: ViewDefinition resource as a Python dictionary
            fhir_version: FHIR version to use ("R4", "R4B", "R5", "R6"). Defaults to "R4"

        Returns:
            True if valid

        Raises:
            InvalidViewDefinitionError: ViewDefinition structure is invalid
            SerializationError: JSON parsing failed
        """
        return cast(bool, py_validate_view_definition(view, fhir_version))

    def validate_bundle(bundle: dict[str, Any], *, fhir_version: str = "R4") -> bool:
        """Validate a Bundle structure without executing transformations.

        Args:
            bundle: FHIR Bundle resource as a Python dictionary
            fhir_version: FHIR version to use ("R4", "R4B", "R5", "R6"). Defaults to "R4"

        Returns:
            True if valid

        Raises:
            SerializationError: JSON parsing failed
        """
        return cast(bool, py_validate_bundle(bundle, fhir_version))

    def parse_content_type(mime_type: str) -> str:
        """Parse MIME type string to format identifier.

        Args:
            mime_type: MIME type string (e.g., "text/csv", "application/json")

        Returns:
            Format identifier suitable for use with run_view_definition

        Raises:
            UnsupportedContentTypeError: Unknown or unsupported MIME type
        """
        return cast(str, py_parse_content_type(mime_type))

    def get_supported_fhir_versions() -> list[str]:
        """Get list of supported FHIR versions compiled into this build.

        Returns:
            List of supported FHIR version strings
        """
        return cast(list[str], py_get_supported_fhir_versions())

except ImportError as e:
    # Fallback for when the Rust extension is not available
    import warnings

    warnings.warn(
        f"Rust extension module not available: {e}. Using placeholder functions.",
        ImportWarning,
        stacklevel=2,
    )

    # Define placeholder exception classes
    class SofError(Exception):  # type: ignore[no-redef]
        """Base exception for all pysof errors"""

        pass

    class InvalidViewDefinitionError(SofError):  # type: ignore[no-redef]
        """ViewDefinition validation errors"""

        pass

    class FhirPathError(SofError):  # type: ignore[no-redef]
        """FHIRPath expression evaluation errors"""

        pass

    class SerializationError(SofError):  # type: ignore[no-redef]
        """JSON/data serialization errors"""

        pass

    class UnsupportedContentTypeError(SofError):  # type: ignore[no-redef]
        """Unsupported output format errors"""

        pass

    class CsvError(SofError):  # type: ignore[no-redef]
        """CSV generation errors"""

        pass

    class IoError(SofError):  # type: ignore[no-redef]
        """File/IO related errors"""

        pass

    # Define placeholder functions
    def run_view_definition(
        view: dict[str, Any],
        bundle: dict[str, Any],
        format: str,
        *,
        fhir_version: str = "R4",
    ) -> bytes:
        raise NotImplementedError("Rust extension module not available")

    def run_view_definition_with_options(
        view: dict[str, Any],
        bundle: dict[str, Any],
        format: str,
        *,
        since: str | None = None,
        limit: int | None = None,
        page: int | None = None,
        fhir_version: str = "R4",
    ) -> bytes:
        raise NotImplementedError("Rust extension module not available")

    def validate_view_definition(
        view: dict[str, Any], *, fhir_version: str = "R4"
    ) -> bool:
        raise NotImplementedError("Rust extension module not available")

    def validate_bundle(bundle: dict[str, Any], *, fhir_version: str = "R4") -> bool:
        raise NotImplementedError("Rust extension module not available")

    def parse_content_type(mime_type: str) -> str:
        raise NotImplementedError("Rust extension module not available")

    def get_supported_fhir_versions() -> list[str]:
        raise NotImplementedError("Rust extension module not available")


__all__: list[str] = [
    # Core functions
    "run_view_definition",
    "run_view_definition_with_options",
    "validate_view_definition",
    "validate_bundle",
    "get_supported_fhir_versions",
    "parse_content_type",
    # Exception classes
    "SofError",
    "InvalidViewDefinitionError",
    "FhirPathError",
    "SerializationError",
    "UnsupportedContentTypeError",
    "CsvError",
    "IoError",
]

__version__ = "0.1.0"


def get_version() -> str:
    """Return the package version."""
    return __version__


def get_status() -> str:
    """Return the current implementation status."""
    return "v1: Rust bindings available with full SOF transformation capabilities."
