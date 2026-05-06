//! R5 mirrors of validation add-on integration tests.
//!
//! ```text
//! cargo test -p fhir-validation --features R5 --test addons_r5
//! ```

#![cfg(feature = "R5")]

#[path = "addons_r5/strict_properties.rs"]
mod strict_properties;

#[path = "addons_r5/base_cardinality.rs"]
mod base_cardinality;
