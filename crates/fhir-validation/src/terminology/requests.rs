//! Strongly typed terminology request models for remote validation operations.
//!
//! This module defines internal request shapes used by the validation engine
//! when interacting with terminology backends, especially for `$validate-code`.
//! It centralizes request construction, structural validation, and FHIR
//! `Parameters` serialization so that validator logic remains decoupled from
//! backend/client-specific transport details.
//!
//! Embedded FHIR resources (`valueSet`, `coding`, `codeableConcept`, `tx-resource`)
//! use [`serde_json::Value`] so one struct works across FHIR versions. Use
//! [`super::builder_r4`] / [`super::builder_r5`] to construct these from typed
//! generated models when available.
use helios_fhir::PrecisionDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidateVsRequest {
    /// Value set Canonical URL. The server must know the value set (e.g. it is defined explicitly in the server's value sets,
    /// or it is defined implicitly by some code system known to the server
    #[serde(rename = "url")]
    pub valueset_url: String,

    /// The context of the value set, so that the server can resolve this to a value set to validate against.
    /// The recommended format for this URI is [Structure Definition URL]#[name or path into structure definition]
    /// e.g. http://hl7.org/fhir/StructureDefinition/observation-hspc-height-hspcheight#Observation.interpretation.
    /// Other forms may be used but are not defined. This form is only usable if the terminology server also has access
    /// to the conformance registry that the server is using, but can be used to delegate the mapping from an application context to a binding at run-time
    pub context: Option<String>,

    /// The value set is provided directly as part of the request. Servers may choose not to accept value sets in this fashion.
    /// This parameter is used when the client wants the server to expand a value set that is not stored on the server
    #[serde(rename = "valueSet")]
    pub valueset: Option<Value>,

    /// The identifier that is used to identify a specific version of the value set to be used when validating the code.
    /// This is an arbitrary value managed by the value set author and is not expected to be globally unique.
    /// For example, it might be a timestamp (e.g. yyyymmdd) if a managed version is not available
    #[serde(rename = "valueSetVersion")]
    pub value_set_version: Option<String>,
    // exactly one of these should be set
    /// The code that is to be validated. If a code is provided, a system or a context must be provided
    /// (if a context is provided, then the server SHALL ensure that the code is not ambiguous without a system)
    pub code: Option<String>,

    /// The system for the code that is to be validated
    pub system: Option<String>,

    /// The version of the system, if one was provided in the source data. Note that this is a different parameter to system-version
    #[serde(rename = "systemVersion")]
    pub system_version: Option<String>,

    /// The display associated with the code, if provided. If a display is provided a code must be provided.
    /// If no display is provided, the server cannot validate the display value, but may choose to return a
    /// recommended display name using the display parameter in the outcome. Whether displays are case sensitive is code system dependent
    pub display: Option<String>,

    /// A coding to validate (JSON `Coding` object, any FHIR version).
    pub coding: Option<Value>,

    /// A full CodeableConcept to validate (JSON object, any FHIR version).
    #[serde(rename = "codeableConcept")]
    pub codeable_concept: Option<Value>,

    /// The date for which the validation should be checked. Normally, this is the current conditions (which is the default values)
    /// but under some circumstances, systems need to validate that a correct code was used at some point in the past.
    /// A typical example of this would be where code selection is constrained to the set of codes that were available when the patient was treated,
    /// not when the record is being edited. Note that which date is appropriate is a matter for implementation policy.
    pub date: Option<PrecisionDateTime>,

    /// If this parameter has a value of true or the parameter is omitted, the client is stating that the validation is being performed in a context where a concept designated as 'abstract' is appropriate/allowed to be used,
    /// and the server should regard abstract codes as valid. If this parameter is false, abstract codes are not considered to be valid.
    /// Note that 'abstract' is a property defined by many HL7 code systems that indicates that the concept is a logical grouping concept that is not intended to be used as a 'concrete' concept
    /// to in an actual patient/care/process record. This language is borrowed from object-orientated theory where 'abstract' entities are never instantiated. However in the general record and terminology eco-system,
    /// there are many contexts where it is appropriate to use these codes e.g. as decision making criterion, or when editing value sets themselves. This parameter allows a client to indicate to the server that it is working in such a context.
    #[serde(rename = "abstract")]
    pub abstract_ok: Option<bool>,

    /// Specifies the language for display validation. Note: the display value only needs to match 1 displayLanguage in order for the validate operation to return true.
    #[serde(rename = "displayLanguage")]
    pub display_language: Option<Vec<String>>,

    /// The supplement must be used when validating the code. Use of this parameter should result in $validate-code
    /// behaving the same way as if the supplements were included in the value set definition using the http://hl7.org/fhir/StructureDefinition/valueset-supplement
    #[serde(rename = "useSupplement")]
    pub use_supplement: Option<Vec<String>>,

    /// When the 'lenient-display-validation' parameter is true, an invalid display string will not cause the 'result' output parameter to be 'false'.
    /// If the 'lenient-display-validation' parameter is false or absent, then an invalid display will cause the 'result' output parameter to be 'false', i.e. the validation will fail.
    #[serde(rename = "lenient-display-validation")]
    pub lenient_display_validation: Option<bool>,

    /// When 'true', the server will not perform the additional validation tasks beyond validating membership in the value set
    /// (e.g. the server won't check displays, etc.)
    #[serde(rename = "valueset-membership-only")]
    pub valueset_membership_only: Option<bool>,

    /// If true, the terminology server is required to infer the system from evaluation of the value set definition.
    /// The inferSystem parameter is only to be used with the code parameter, and not with the coding nor codeableConcept parameters.
    #[serde(rename = "inferSystem")]
    pub infer_system: Option<bool>,

    /// Specifies a version to use for a system, if the value set does not specify which one to use.
    /// The format is the same as a canonical URL: [system]|[version] - e.g. http://loinc.org|2.56.
    /// Note that this is a different parameter to systemVersion
    #[serde(rename = "system-version")]
    pub system_version2: Option<Vec<String>>,

    /// Edge Case: Specifies a version to use for a system. If a value set specifies a different version, an error is returned instead of the expansion.
    /// The format is the same as a canonical URL: [system]|[version] - e.g. http://loinc.org|2.56
    #[serde(rename = "check-system-version")]
    pub check_system_version: Option<Vec<String>>,

    /// Specifies a version to use for a valueset, if the reference to the value set does not specify which version to use.
    /// The format is the same as a canonical URL: [system]|[version] - e.g. http://example.org/ValueSet/example|1.0.0.
    /// Note that this is similar to the force-system-version parameter but applied to valuesets
    #[serde(rename = "default-valueset-version")]
    pub default_valueset_version: Option<Vec<String>>,

    /// Edge Case: Specifies a version to use for a valueset. If a reference to a value set specifies a different version,
    /// an error is returned instead of the expansion. The format is the same as a canonical URL: [system]|[version] -
    /// e.g. http://example.org/ValueSet/example|1.0.0. Note that this is similar to the force-system-version parameter but applied to valuesets
    #[serde(rename = "check-valueset-version")]
    pub check_valueset_version: Option<Vec<String>>,

    /// Edge Case: Specifies a version to use for a valueset. This parameter overrides any specified version in the reference to the value set
    /// (and any it depends on). The format is the same as a canonical URL: [system]|[version] - e.g. http://example.org/ValueSet/example|1.0.0.
    /// Note that this has obvious safety issues, in that it may result in a value set expansion giving a different list of codes that is both wrong and unsafe,
    /// and implementers should only use this capability reluctantly. It primarily exists to deal with situations where specifications have fallen into decay as time passes.
    /// If the value is overridden, the version used SHALL explicitly be represented in the expansion parameters.
    /// Note that this is similar to the force-system-version parameter but applied to valuesets.
    #[serde(rename = "force-valueset-version")]
    pub force_valueset_version: Option<Vec<String>>,

    /// Specifies an library that provides expansion rules for the operation. The library has an extension expansionParameters that references a contained Parameters
    /// resource that contains additional $expand parameters. See the [CRMI specification description of manifests]https://hl7.org/fhir/uv/crmi/STU1/StructureDefinition-crmi-manifestlibrary.html)
    /// and CRMI expansion rules icon for a complete description of how manifest values are used to provide defaults for expansion parameters. Parameters specified directly in an $expand operation
    /// override behaviors specified by the manifest parameter.
    // backend-dependent / not yet interpreted locally
    #[serde(rename = "manifest")]
    pub manifest: Option<String>,

    /// One or more additional resources that are referred to from the value set provided with the $expand or $validate-code invocation.
    /// These may be additional value sets or code systems that the client believes will or may be necessary to perform the operation.
    /// Resources provided in this fashion are used preferentially to those known to the system, though servers may return an error if
    /// these resources are already known to the server (by URL and version) but differ from that information on the server.
    #[serde(rename = "tx-resource")]
    pub tx_resource: Option<Vec<Value>>,
}

impl ValidateVsRequest {
    pub fn validate(&self) -> Result<(), String> {
        let n = self.code.is_some() as u8
            + self.coding.is_some() as u8
            + self.codeable_concept.is_some() as u8;

        if n != 1 {
            return Err("Exactly one of code, coding, or codeable_concept must be set".into());
        }
        if self.code.is_some()
            && self.system.is_none()
            && self.context.is_none()
            && self.infer_system != Some(true)
        {
            return Err(
                "When code is provided, either system, context, or infer_system=true should be set"
                    .into(),
            );
        }

        Ok(())
    }

    pub fn to_parameters_json(&self) -> serde_json::Value {
        let mut params = Vec::new();

        params.push(serde_json::json!({
            "name": "url",
            "valueUri": self.valueset_url
        }));

        if let Some(v) = &self.context {
            params.push(serde_json::json!({
                "name": "context",
                "valueUri": v
            }));
        }

        if let Some(v) = &self.valueset {
            params.push(serde_json::json!({
                "name": "valueSet",
                "resource": v
            }));
        }

        if let Some(v) = &self.value_set_version {
            params.push(serde_json::json!({
                "name": "valueSetVersion",
                "valueString": v
            }));
        }

        if let Some(v) = &self.code {
            params.push(serde_json::json!({
                "name": "code",
                "valueCode": v
            }));
        }

        if let Some(v) = &self.system {
            params.push(serde_json::json!({
                "name": "system",
                "valueUri": v
            }));
        }

        if let Some(v) = &self.system_version {
            params.push(serde_json::json!({
                "name": "systemVersion",
                "valueString": v
            }));
        }

        if let Some(v) = &self.display {
            params.push(serde_json::json!({
                "name": "display",
                "valueString": v
            }));
        }

        if let Some(v) = &self.coding {
            params.push(serde_json::json!({
                "name": "coding",
                "valueCoding": v
            }));
        }

        if let Some(v) = &self.codeable_concept {
            params.push(serde_json::json!({
                "name": "codeableConcept",
                "valueCodeableConcept": v
            }));
        }

        if let Some(v) = &self.date {
            params.push(serde_json::json!({
                "name": "date",
                "valueDateTime": v
            }));
        }

        if let Some(v) = self.abstract_ok {
            params.push(serde_json::json!({
                "name": "abstract",
                "valueBoolean": v
            }));
        }

        if let Some(v) = self.infer_system {
            params.push(serde_json::json!({
                "name": "inferSystem",
                "valueBoolean": v
            }));
        }

        if let Some(v) = self.lenient_display_validation {
            params.push(serde_json::json!({
                "name": "lenient-display-validation",
                "valueBoolean": v
            }));
        }

        if let Some(v) = self.valueset_membership_only {
            params.push(serde_json::json!({
                "name": "valueset-membership-only",
                "valueBoolean": v
            }));
        }

        if let Some(langs) = &self.display_language {
            for lang in langs {
                params.push(serde_json::json!({
                    "name": "displayLanguage",
                    "valueCode": lang
                }));
            }
        }

        if let Some(supps) = &self.use_supplement {
            for supp in supps {
                params.push(serde_json::json!({
                    "name": "useSupplement",
                    "valueCode": supp
                }));
            }
        }

        if let Some(v) = &self.system_version2 {
            for s in v {
                params.push(serde_json::json!({
                    "name": "system-version",
                    "valueCanonical": s
                }));
            }
        }

        if let Some(v) = &self.check_system_version {
            for s in v {
                params.push(serde_json::json!({
                    "name": "check-system-version",
                    "valueCanonical": s
                }));
            }
        }

        if let Some(v) = &self.default_valueset_version {
            for s in v {
                params.push(serde_json::json!({
                    "name": "default-valueset-version",
                    "valueCanonical": s
                }));
            }
        }

        if let Some(v) = &self.check_valueset_version {
            for s in v {
                params.push(serde_json::json!({
                    "name": "check-valueset-version",
                    "valueCanonical": s
                }));
            }
        }

        if let Some(values) = &self.force_valueset_version {
            for s in values {
                params.push(serde_json::json!({
                    "name": "force-valueset-version",
                    "valueCanonical": s
                }));
            }
        }

        if let Some(v) = &self.manifest {
            params.push(serde_json::json!({
                "name": "manifest",
                "valueCanonical": v
            }));
        }

        if let Some(resources) = &self.tx_resource {
            for resource in resources {
                params.push(serde_json::json!({
                    "name": "tx-resource",
                    "resource": resource
                }));
            }
        }

        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": params
        })
    }
}
