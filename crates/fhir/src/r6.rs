// This is a template r6.rs file - it gets copied to r6.rs by build.rs and is only sufficient for the
// initial R6 feature build before downloading the spec.  It will be overwritten by fhir_gen if the R6 feature is specified (or --all)
use fhir_macro::{FhirPath, FhirSerde};
use serde::{Deserialize, Serialize};
use crate::Element;
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Extension {}
pub type Code = Element<std::string::String, Extension>;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
#[serde(tag = "resourceType")]
pub enum Resource {}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
pub struct ViewDefinition {
    pub select: Option<Vec<ViewDefinitionSelect>>,
    pub resource: Code 
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
pub struct ViewDefinitionSelect {
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
pub struct Bundle {}
