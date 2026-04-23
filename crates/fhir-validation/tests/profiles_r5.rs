//! R5 **profile** validation: `meta.profile` + [`ProfileRegistry`], [`validate_profile`] on extracted
//! [`ExtractedProfile`] shapes, and element-scoped constraints (e.g. `cpb-12`-style).
//!
//! Uses local terminology only. Fixtures: `tests/fixtures/r5/profile/`; HL7 JSON:
//! `crates/fhir/tests/data/json/R5/`.
//!
//! Related integration crates: **`bindings_r5`**, **`invariants_r5`**, **`examples_r5`**, **`framework`**, **`r4_suite`**.

#![cfg(feature = "R5")]
#[path = "profiles_r5/capability_statement_cpb12.rs"]
mod capability_statement_cpb12;
pub mod common;
#[path = "profiles_r5/examples_atrius_registry.rs"]
mod examples_atrius_registry;
#[path = "profiles_r5/fixtures_declared_profile.rs"]
mod fixtures_declared_profile;
#[path = "profiles_r5/harness.rs"]
mod harness;
#[path = "profiles_r5/patient_validate_profile_root.rs"]
mod patient_validate_profile_root;
