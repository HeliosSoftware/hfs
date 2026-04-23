//! **Framework** integration tests: engine behaviour that is not split into the versioned
//! `bindings_*` / `invariants_*` / `profiles_*` suites (cardinality, slicing, SD extract, FHIRPath
//! bulk helpers, terminology request/parse plumbing, error mapping, OperationOutcome details).
//!
//! R5-only modules are gated with `#[cfg(feature = "R5")]`. Run, for example:
//! `cargo test -p fhir-validation --features "R4,R5" --test framework`.
//!
//! HL7 example smoke / remote parity: **`examples_r5`**. Versioned profile/registry tests: **`profiles_r5`**.
pub mod common;
#[cfg(feature = "R5")]
#[path = "framework/validate_r5.rs"]
mod validate_r5;

#[path = "framework/cardinality.rs"]
mod cardinality;

#[cfg(feature = "R5")]
#[path = "framework/slicing.rs"]
mod slicing;

#[cfg(feature = "R5")]
#[path = "framework/extract.rs"]
mod extract;

#[cfg(feature = "R5")]
#[path = "framework/bulk_fhirpath.rs"]
mod bulk_fhirpath;

#[path = "framework/terminology_requests.rs"]
mod terminology_requests;

#[path = "framework/terminology_helpers.rs"]
mod terminology_helpers;

#[path = "framework/validation_error_mapping.rs"]
mod validation_error_mapping;

#[path = "framework/detail_code_operation_outcome.rs"]
mod detail_code_operation_outcome;
