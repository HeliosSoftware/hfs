"""Test basic package import and metadata."""

import pysof


def test_import() -> None:
    """Test that the package can be imported."""
    assert pysof is not None


def test_version() -> None:
    """Test that version is accessible and correctly formatted."""
    version = pysof.__version__
    assert isinstance(version, str)
    assert version == "0.1.0"


def test_get_version_function() -> None:
    """Test the get_version utility function."""
    version = pysof.get_version()
    assert isinstance(version, str)
    assert version == pysof.__version__


def test_get_status_function() -> None:
    """Test the get_status utility function."""
    status = pysof.get_status()
    assert isinstance(status, str)
    assert "v1" in status
    assert "rust" in status.lower()


def test_all_exports() -> None:
    """Test that __all__ is properly defined."""
    assert isinstance(pysof.__all__, list)
    # In v1, core APIs are exposed
    for name in [
        "run_view_definition",
        "run_view_definition_with_options",
        "validate_view_definition",
        "validate_bundle",
        "get_supported_fhir_versions",
        "parse_content_type",
        "SofError",
        "InvalidViewDefinitionError",
        "FhirPathError",
        "SerializationError",
        "UnsupportedContentTypeError",
        "CsvError",
        "IoError",
    ]:
        assert name in pysof.__all__


def test_docstring() -> None:
    """Test that module has proper docstring."""
    assert pysof.__doc__ is not None
    assert "SQL on FHIR" in pysof.__doc__
    assert "ViewDefinition" in pysof.__doc__
