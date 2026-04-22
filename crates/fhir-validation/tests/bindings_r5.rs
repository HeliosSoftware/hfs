//! R5 ValueSet binding integration tests.
//!
//! Covers binding targets used by the validator: primitive `code`, `string`, `uri`, `Coding`,
//! `CodeableConcept`, and `CodeableReference` (R5).
//!
//! - **Local terminology**: [`LocalTerminologyService`] (safe for CI without a terminology server).
//! - **Remote terminology**: several `#[tokio::test]` cases call [`RemoteTerminologyService`] against
//!   a real FHIR server (e.g. local [Snowstorm](https://github.com/IHTSDO/snowstorm) on
//!   `http://localhost:8080/fhir`). Set `FHIR_TERMINOLOGY_BASE_URL` to override the default base URL
//!   in [`crate::harness::remote_terminology_for_tests`]. Without a reachable server, those tests fail fast.
//!
//! Example JSON primarily lives under `crates/fhir/tests/data/json/R5/`; a few focused fixtures
//! under `tests/fixtures/r5/` cover types not present in that corpus (e.g. `StructureDefinition`
//! language, `Slot.serviceType`).
//!
//! Related integration crates: **`invariants_r5`**, **`profiles_r5`**, **`examples_r5`**, **`framework`**, **`r4_suite`**.

#![cfg(feature = "R5")]
pub mod common;
// Integration-test submodules live under `tests/` by default; keep sources in `tests/bindings_r5/`.
#[path = "bindings_r5/harness.rs"]
mod harness;
#[path = "bindings_r5/remote_terminology.rs"]
mod remote_terminology;
#[path = "bindings_r5/resource_examples.rs"]
mod resource_examples;
#[path = "bindings_r5/resource_fixtures.rs"]
mod resource_fixtures;
#[path = "bindings_r5/smoke_curated.rs"]
mod smoke_curated;
#[path = "bindings_r5/unit_binding_handlers.rs"]
mod unit_binding_handlers;
