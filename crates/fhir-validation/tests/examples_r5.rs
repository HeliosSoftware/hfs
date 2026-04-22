//! HL7-style **R5 example JSON** checks: local smoke validation and optional remote `$validate` parity.
//!
//! - **`smoke`**: `validate_resource` completes; issues are well-formed (no tight semantic parity).
//! - **`online_parity`**: `#[ignore]` by default; compares severity counts vs a public FHIR server when
//!   `FHIR_ONLINE_VALIDATOR_BASE_URL` is set.
//!
//! Related crates: **`bindings_r5`**, **`invariants_r5`**, **`profiles_r5`**, **`framework`**, **`r4_suite`**.

#![cfg(feature = "R5")]
pub mod common;
#[path = "examples_r5/online_parity.rs"]
mod online_parity;
#[path = "examples_r5/smoke.rs"]
mod smoke;
