//! Helpers to embed **R5** generated types into [`ValidateVsRequest`](super::requests::ValidateVsRequest)
//! as [`serde_json::Value`] for `$validate-code` `Parameters` serialization.
//!
//! Requires the `R5` crate feature.

use super::requests::ValidateVsRequest;
use helios_fhir::r5::{CodeSystem, CodeableConcept, Coding, ValueSet};
use serde::Serialize;
use serde_json::Value;

fn to_json<T: Serialize + ?Sized>(v: &T) -> Result<Value, serde_json::Error> {
    serde_json::to_value(v)
}

/// Serialize an R5 `ValueSet` into the `valueSet` parameter resource.
pub fn set_valueset(req: &mut ValidateVsRequest, vs: &ValueSet) -> Result<(), serde_json::Error> {
    req.valueset = Some(to_json(vs)?);
    Ok(())
}

/// Serialize an R5 `Coding` into the `coding` parameter.
pub fn set_coding(req: &mut ValidateVsRequest, c: &Coding) -> Result<(), serde_json::Error> {
    req.coding = Some(to_json(c)?);
    Ok(())
}

/// Serialize an R5 `CodeableConcept` into the `codeableConcept` parameter.
pub fn set_codeable_concept(
    req: &mut ValidateVsRequest,
    cc: &CodeableConcept,
) -> Result<(), serde_json::Error> {
    req.codeable_concept = Some(to_json(cc)?);
    Ok(())
}

/// Append R5 `CodeSystem` / `ValueSet` resources as `tx-resource` parameters.
pub fn push_tx_resource_value_set(
    req: &mut ValidateVsRequest,
    vs: &ValueSet,
) -> Result<(), serde_json::Error> {
    let v = to_json(vs)?;
    req.tx_resource.get_or_insert_with(Vec::new).push(v);
    Ok(())
}

/// Append R5 `CodeSystem` / `ValueSet` resources as `tx-resource` parameters.
pub fn push_tx_resource_code_system(
    req: &mut ValidateVsRequest,
    cs: &CodeSystem,
) -> Result<(), serde_json::Error> {
    let v = to_json(cs)?;
    req.tx_resource.get_or_insert_with(Vec::new).push(v);
    Ok(())
}
