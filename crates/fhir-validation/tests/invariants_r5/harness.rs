//! Shared helpers for the `invariants_r5` integration test crate.

use fhir_validation::R5FhirPathEvaluator;
use helios_fhir::FhirResource;

pub fn r5_evaluator_for(resource: &FhirResource) -> R5FhirPathEvaluator {
    let FhirResource::R5(r) = resource else {
        panic!("expected R5 FhirResource");
    };
    R5FhirPathEvaluator::new((**r).clone())
}
