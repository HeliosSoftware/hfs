# Errors and validation issues in `fhir-validation`

This document describes how failures are represented in the validation engine, how they relate to FHIR `OperationOutcome`, and how terminology binding paths map structured failures into [`ValidationIssue`](src/core.rs) rows.

## Two layers: `ValidationError` vs `ValidationIssue`

| Concept | Type | Role |
|--------|------|------|
| **Pipeline / orchestration failure** | [`ValidationError`](src/error.rs) | Returned from `Result` APIs: FHIRPath evaluation, profile JSON extraction, remote `$validate-code` HTTP/parsing, and wrapping [`TerminologyValidationError`](../fhir/src/error.rs) when it must travel as `Err` (e.g. invalid input on a sync terminology path). Implements `std::error::Error` and `Display`. |
| **User-visible validation result** | [`ValidationIssue`](src/core.rs) | One row in the validator’s issue list; later mapped to `OperationOutcome.issue` (see [`issue_to_op_outcome`](src/issue_to_op_outcome.rs)). Not an `Error` type—this is the stable reporting DTO. |

Most validation flows **accumulate `ValidationIssue`** directly (invariants, bindings, structure). `ValidationError` appears where the code uses `Result` and the caller converts to issues (or propagates).

### `std::error::Error` source chain

- **`ValidationError::FhirPath`** and **`ValidationError::LocalTerminology`** propagate **`Error::source`** to the inner [`EvaluationError`](../fhirpath-support/src/lib.rs) or [`TerminologyValidationError`](../fhir/src/error.rs) for `anyhow`/`eyre`-style chains.
- **`ValidationError::InvalidRequest`** propagates **`Error::source`** to [`TerminologyRequestInvalid`](src/error.rs) (which implements [`std::error::Error`]).
- **`RemoteTerminology`**, **`InvalidStructureDefinition`**, and **`Internal`** do not currently expose a nested `Error` source (remote payloads are structured values; internal messages are stringly).

### Non-exhaustive public enums

[`ValidationError`](src/error.rs) and [`RemoteTerminologyError`](src/error.rs) are **`#[non_exhaustive]`**. Code **outside** this crate must match with a wildcard arm (or equivalent) so new variants can be added without breaking every downstream `match` in a minor release.

## `ValidationError` variants ([`error.rs`](src/error.rs))

- **`FhirPath`** — [`helios_fhirpath_support::EvaluationError`](../fhirpath-support/src/lib.rs) from expression evaluation.
- **`LocalTerminology(TerminologyValidationError)`** — Structured local terminology semantics from generated ValueSet helpers (same enum as `Result<(), TerminologyValidationError>` in `helios_fhir`). Used when that result must be surfaced as `Err(ValidationError::…)` (e.g. `InvalidInput` on [`LocalTerminologyService`](src/terminology/local.rs)).
- **`RemoteTerminology(RemoteTerminologyError)`** — Remote / protocol failures around `$validate-code`:
  - **`Upstream(TerminologyRemoteError)`** — HTTP status, server diagnostics, optional body from the terminology server (see [`types.rs`](src/terminology/types.rs)). Variant name avoids repeating “remote” next to [`RemoteTerminologyError`].
  - **`MalformedResponse(MalformedValidateCodeParameters)`** — Response JSON is not usable `$validate-code` `Parameters` (wrong `resourceType`, bad `parameter` array shape, or invalid / missing boolean `result`). See [`MalformedValidateCodeParameters`](src/error.rs); labels: [`malformed_validate_code_parameters_kind_label`](src/error.rs).
- **`InvalidRequest(TerminologyRequestInvalid)`** — Client-side request validation failed before any HTTP call (e.g. [`ValidateVsRequest::validate`](src/terminology/requests.rs)); carries a human-readable `message` string aligned with the underlying validation error.
- **`InvalidStructureDefinition(StructureDefinitionExtractMessage)`** — Profile extraction from `StructureDefinition` JSON failed; reasons are enumerated in [`structure_definition_extract.rs`](src/profile/structure_definition_extract.rs) (`Display` matches prior string diagnostics).
- **`Internal(String)`** — Catch-all for unexpected pipeline messages (same stable label as before: [`validation_error_kind_label`](src/error.rs) → `"other"`).

Stable labels for metrics and logging: [`validation_error_kind_label`](src/error.rs), [`remote_terminology_error_kind_label`](src/error.rs) (for malformed responses, this resolves to the fine-grained [`malformed_validate_code_parameters_kind_label`](src/error.rs)), and [`malformed_validate_code_parameters_kind_label`](src/error.rs) when you only have [`MalformedValidateCodeParameters`].

Optional structural access without a full `match`: [`ValidationError::as_invalid_request`](src/error.rs), [`ValidationError::as_remote_terminology`](src/error.rs), [`ValidationError::as_remote_malformed_parameters`](src/error.rs), [`RemoteTerminologyError::as_upstream`](src/error.rs), [`RemoteTerminologyError::as_malformed_parameters`](src/error.rs).

### Remote diagnostics helper

[`ValidationError::remote_binding_failure_diagnostics`](src/error.rs) takes a ValueSet URL and produces user-facing text for **`RemoteTerminology`** failures (ValueSet context for remote/malformed branches). For other [`ValidationError`] variants it falls back to [`Display`] (so **`InvalidRequest`** / **`LocalTerminology`** are not prefixed as “remote”). Binding code should prefer [`validation_error_to_issues`](src/binding/common.rs) when possible.

## Local terminology: `TerminologyValidationError`

Defined in [`crates/fhir/src/error.rs`](../fhir/src/error.rs). Generated ValueSet code returns `Result<(), TerminologyValidationError>` for `validate_code` / `validate_coding` style checks. Variants include unknown code, not in value set, wrong display, missing system, remote validation required, etc.

Binding validation maps terminal local failures through [`local_error_to_issues`](src/binding/common.rs) (respects binding strength and [`Validator`](src/core.rs) policy). When membership is checked via [`TerminologyService::member_of`](src/terminology/service.rs), structured failures can be carried on [`TerminologyMembershipOutcome::local_failure`](src/terminology/types.rs) so display mismatches route through the same mapping as full local validation.

## Remote terminology: `RemoteTerminologyError` and HTTP

[`TerminologyRemoteError`](src/terminology/types.rs) holds optional HTTP status, structured diagnostics (e.g. parsed from `OperationOutcome` in the body), and raw body text. Helpers in [`terminology/helpers.rs`](src/terminology/helpers.rs) parse client error strings and `$validate-code` `Parameters` JSON.

## Unified mapping: `ValidationError` → `Vec<ValidationIssue>` on binding paths

There is **no** context-free `From<ValidationError> for ValidationIssue`: issue shaping needs the binding path, ValueSet URL, strength, and validator policy.

For ValueSet binding and `member_of` outcomes, use:

- **[`TerminologyIssueContext`](src/binding/common.rs)** — Holds `validator`, `fhir_path`, `valueset_url`, and `binding` [`BindingStrength`](../fhir-validation-types/src/lib.rs).
- **[`validation_error_to_issues`](src/binding/common.rs)** or **[`ValidationError::to_binding_issues`](src/binding/common.rs)** — Dispatches:
  - **`LocalTerminology`** → [`local_error_to_issues`](src/binding/common.rs) (structured, strength-aware).
  - **`RemoteTerminology`** → single [`terminology_validation_issue`](src/binding/common.rs) with [`remote_binding_failure_diagnostics`](src/error.rs).
  - **`InvalidRequest`** — Single [`terminology_validation_issue`](src/binding/common.rs) with the request validation message.
  - **`FhirPath` / `InvalidStructureDefinition` / `Other`** — Mapped to exception / structure / terminology issues so the `Err` branch always yields actionable issues (rare on pure `member_of` paths).

[`BindingCheckContextSync::terminology_issue_context`](src/binding/common.rs) and the async variant build a [`TerminologyIssueContext`] from the current binding check.

[`remote_result_to_issues`](src/binding/common.rs) uses `validation_error_to_issues` for the `Err(ValidationError)` branch of `member_of`.

## Production operations

These concerns are primarily owned by **callers** (HTTP clients, servers, logging pipelines), but they interact directly with this crate’s types:

| Topic | Guidance |
|--------|----------|
| **HTTP client** | Use [`RemoteTerminologyService::with_client`](src/terminology/service.rs) with a **production** `reqwest::Client`: set **timeouts**, **TLS**, **connection pooling**, **authentication**, and **proxies** as required. The default `new()` helper is oriented toward development. |
| **Retries** | Retry only **idempotent** failures (e.g. transient 5xx, network) **in the HTTP client layer**. Do **not** blindly retry 4xx responses from `$validate-code` (they may be validation semantics, not transport). |
| **PII / logs** | [`TerminologyRemoteError`](src/terminology/types.rs) may include **raw response bodies**; treat as sensitive. **Redact or truncate** bodies in production logs. [`ValidationIssue::diagnostics`](src/core.rs) may surface server text to clients—**API layers** should decide what to expose externally vs. keep internal-only. |
| **Observability** | [`TerminologyService::member_of`](src/terminology/service.rs) and [`LocalTerminologyService`](src/terminology/local.rs) emit **`tracing::warn!`** on `Err` with `valueset_url` and `error_kind` (and `remote_detail` for remote failures). Use these fields in metrics or log aggregation without logging full bodies when possible. |

## Boundary: validation vs business rules

`fhir-validation` is intentionally scoped to **FHIR semantic validation**:

- base spec structure/invariants/bindings
- profile constraints (`meta.profile`, `type.profile`, `baseDefinition` recursion policy)
- optional structural add-ons (strict unknown JSON keys, base snapshot min/max checks)
- optional questionnaire conformance checks when the caller supplies `Questionnaire`

The following stay in an **outer application layer** (server/business middleware), not this crate:

- authorization and tenant policy decisions
- duplicate detection / idempotency policy
- workflow/state-machine constraints outside FHIR cardinality/invariants
- cross-resource referential policy that depends on persistence, not the submitted payload alone

Recommended contract: run `fhir-validation` first to produce `ValidationIssue` / `OperationOutcome`; then run business checks and map failures to additional operation outcomes or application-specific errors.

## Crate re-exports

The crate root re-exports [`ValidationError`](src/lib.rs), [`RemoteTerminologyError`](src/lib.rs), [`MalformedValidateCodeParameters`](src/lib.rs), [`TerminologyRequestInvalid`](src/lib.rs), [`TerminologyIssueContext`](src/lib.rs), [`validation_error_to_issues`](src/lib.rs), [`validation_error_kind_label`](src/lib.rs), [`remote_terminology_error_kind_label`](src/lib.rs), and [`malformed_validate_code_parameters_kind_label`](src/lib.rs) for convenience.

## Related

- [`FhirValidation_Architecture.md`](FhirValidation_Architecture.md) — High-level validation pipeline.
- [`issue_to_op_outcome.rs`](src/issue_to_op_outcome.rs) — `ValidationIssue` → `OperationOutcome`.
