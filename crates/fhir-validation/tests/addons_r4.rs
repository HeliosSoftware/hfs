//! Integration tests for validation add-ons: strict JSON properties + base snapshot cardinality.
//!
//! ```text
//! cargo test -p fhir-validation --features R4 --test addons_r4
//! ```

#![cfg(feature = "R4")]

pub mod common;

#[path = "addons_r4/strict_properties.rs"]
mod strict_properties;

#[path = "addons_r4/base_cardinality.rs"]
mod base_cardinality;
