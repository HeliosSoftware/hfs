//! R5 **invariant** (`ConstraintDefinition`) and element-constraint integration tests.
//!
//! - **Resource invariants**: generated FHIRPath rules on resource types (e.g. `dom-3`, `ref-2`,
//!   `ele-1`).
//!
//! Profile / `StructureDefinition` constraint validation lives in **`profiles_r5`**.
//!
//! All tests here use **local** terminology only; invariants do not call remote terminology.
//!
//! HL7 examples: `crates/fhir/tests/data/json/R5/`. Targeted invalid shapes:
//! `tests/fixtures/r5/`.
//!
//! Related integration crates: **`bindings_r5`**, **`profiles_r5`**, **`examples_r5`**, **`framework`**, **`r4_suite`**.

#![cfg(feature = "R5")]
pub mod common;
#[path = "invariants_r5/element_constraints.rs"]
mod element_constraints;
#[path = "invariants_r5/harness.rs"]
mod harness;
#[path = "invariants_r5/observation_fixture.rs"]
mod observation_fixture;
#[path = "invariants_r5/patient_contained_reference.rs"]
mod patient_contained_reference;
#[path = "invariants_r5/resource_examples.rs"]
mod resource_examples;
#[path = "invariants_r5/smoke_curated.rs"]
mod smoke_curated;
