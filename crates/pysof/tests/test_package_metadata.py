"""Test package metadata and structure."""

import sys
from pathlib import Path


def test_package_structure() -> None:
    """Test that package has expected structure."""
    import pysof
    
    # Test that package is installed/importable
    assert hasattr(pysof, "__version__")
    assert hasattr(pysof, "__all__")
    assert hasattr(pysof, "get_version")
    assert hasattr(pysof, "get_status")


def test_python_version_compatibility() -> None:
    """Test that we're running on Python 3.11+ as expected."""
    assert sys.version_info >= (3, 11), f"Expected Python 3.11+, got {sys.version_info}"


def test_module_file_location() -> None:
    """Test that module is loaded from expected location."""
    import pysof
    
    # Should be loaded from src/pysof/__init__.py
    module_file = Path(pysof.__file__) if pysof.__file__ else None
    assert module_file is not None
    assert module_file.name == "__init__.py"
    assert module_file.parent.name == "pysof"
