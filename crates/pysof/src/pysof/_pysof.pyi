"""Type stubs for the pysof Rust extension module."""

from typing import Any

# Module attributes
__version__: str

# Exception classes
class SofError(Exception): ...
class InvalidViewDefinitionError(SofError): ...
class FhirPathError(SofError): ...
class SerializationError(SofError): ...
class UnsupportedContentTypeError(SofError): ...
class CsvError(SofError): ...
class IoError(SofError): ...
class InvalidSourceError(SofError): ...
class SourceNotFoundError(SofError): ...
class SourceFetchError(SofError): ...
class SourceReadError(SofError): ...
class InvalidSourceContentError(SofError): ...
class UnsupportedSourceProtocolError(SofError): ...

# Remote resolve() configuration
class RemoteResolveConfig:
    def __init__(
        self,
        allowed_base_urls: list[str],
        *,
        enabled: bool = True,
        timeout_ms: int | None = None,
        max_fetches: int | None = None,
        max_depth: int | None = None,
        max_response_bytes: int | None = None,
        concurrency: int | None = None,
        allow_private_addresses: bool = False,
        cache_max_entries: int | None = None,
        bearer_tokens: dict[str, str] | None = None,
    ) -> None: ...
    @staticmethod
    def from_env() -> RemoteResolveConfig: ...
    def is_active(self) -> bool: ...

# Core functions
def py_run_view_definition(
    view: dict[str, Any],
    bundle: dict[str, Any],
    format: str,
    fhir_version: str,
) -> bytes: ...
def py_run_view_definition_with_options(
    view: dict[str, Any],
    bundle: dict[str, Any],
    format: str,
    *,
    since: str | None = None,
    limit: int | None = None,
    page: int | None = None,
    fhir_version: str = "R4",
) -> bytes: ...
def py_validate_view_definition(
    view: dict[str, Any],
    fhir_version: str,
) -> bool: ...
def py_validate_bundle(
    bundle: dict[str, Any],
    fhir_version: str,
) -> bool: ...
def py_parse_content_type(mime_type: str) -> str: ...
def py_get_supported_fhir_versions() -> list[str]: ...
def py_run_view_definition_with_options_remote(
    view: dict[str, Any],
    bundle: dict[str, Any],
    format: str,
    remote_config: RemoteResolveConfig,
    *,
    since: str | None = None,
    limit: int | None = None,
    page: int | None = None,
    fhir_version: str = "R4",
) -> bytes: ...
def py_process_ndjson_to_file_remote(
    view: dict[str, Any],
    input_path: str,
    output_path: str,
    format: str,
    remote_config: RemoteResolveConfig,
    *,
    chunk_size: int = 1000,
    skip_invalid: bool = False,
    fhir_version: str = "R4",
) -> dict[str, Any]: ...
