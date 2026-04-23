//! **R4** integration tests: ValueSet binding units, `CodeableConcept` binding helpers, domain
//! resource invariants (`dom-*`), reference / contained rules, and patient fixtures.
//!
//! ```text
//! cargo test -p fhir-validation --features R4 --test r4_suite
//! ```

#![cfg(feature = "R4")]
#[path = "r4_suite/bindings_code.rs"]
mod bindings_code;
#[path = "r4_suite/bindings_codeable_concept.rs"]
mod bindings_codeable_concept;
pub mod common;
#[path = "r4_suite/domain_resource_invariants.rs"]
mod domain_resource_invariants;
#[path = "r4_suite/patient.rs"]
mod patient;
#[path = "r4_suite/reference_invariants.rs"]
mod reference_invariants;
