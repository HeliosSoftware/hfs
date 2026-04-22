#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_upper_case_globals)]
#![allow(unused_mut)]
use crate as fhir_validation;
use helios_fhir::r5::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fhir-validation-gen/generated/r5/all.rs"
));
