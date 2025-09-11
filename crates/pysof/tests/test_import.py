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
    assert "v0" in status
    assert "scaffold" in status.lower()


def test_all_exports() -> None:
    """Test that __all__ is properly defined."""
    # In v0, __all__ should be empty since no public APIs are implemented yet
    assert isinstance(pysof.__all__, list)
    assert len(pysof.__all__) == 0


def test_docstring() -> None:
    """Test that module has proper docstring."""
    assert pysof.__doc__ is not None
    assert "SQL on FHIR" in pysof.__doc__
    assert "ViewDefinition" in pysof.__doc__
