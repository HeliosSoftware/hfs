use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::types::ExtractedProfile;
use crate::service::{TerminologyService, TerminologyServiceSync};
use crate::{FhirPathEvaluator, Validator};
use helios_fhir::FhirVersion;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct ValidationContext<'a> {
    pub fhir_version: FhirVersion,
    pub validator: &'a Validator,
    pub terminology: Option<&'a dyn TerminologyServiceSync>,
    pub evaluator: &'a dyn FhirPathEvaluator,
    pub runtime_profile_registry: Option<&'a ProfileRegistry>,
    pub extracted_profile_map: &'a HashMap<String, ExtractedProfile>,
}
#[derive(Clone)]
pub struct AsyncValidationContext<'a> {
    pub fhir_version: FhirVersion,
    pub validator: &'a Validator,
    pub terminology: Option<&'a dyn TerminologyService>,
    pub evaluator: &'a dyn FhirPathEvaluator,
    pub runtime_profile_registry: Option<&'a ProfileRegistry>,
    pub extracted_profile_map: &'a HashMap<String, ExtractedProfile>,
}

#[derive(Debug, Clone, Default)]
pub struct ValidationState {
    pub recursion_depth: usize,
    pub active_profiles: HashSet<String>,
}
