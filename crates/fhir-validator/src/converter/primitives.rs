//! Primitive-type StructureDefinitions → leaf schemas.
//!
//! A primitive SD becomes `{kind: "primitive-type"}` plus the value regex
//! extracted from its `<type>.value` element
//! (`type[0].extension[url=.../regex].valueString`). No `base` is emitted:
//! pulling `Element` into every primitive's cooperative set would be wasted
//! work — the `_field` sidecar machinery owns the Element part instead.
//!
//! There is deliberately no hardcoded fallback regex table: the spec files
//! carry the regexes, and a wrong guess is worse than the JSON type-class
//! check the engine applies from the primitive's name anyway. A missing
//! regex just produces a warning.

use super::Ed;
use crate::schema::FhirSchema;

pub(super) fn convert_primitive(
    url: String,
    name: Option<String>,
    sd_type: &str,
    derivation: Option<String>,
    elements: &[Ed],
    warnings: &mut Vec<String>,
) -> FhirSchema {
    let value_path = format!("{sd_type}.value");
    let regex = elements
        .iter()
        .find(|ed| ed.path == value_path)
        .and_then(|ed| ed.types.first())
        .and_then(|t| t.regex())
        .map(str::to_string);
    if regex.is_none() {
        warnings.push(format!("primitive '{sd_type}': no value regex found"));
    }

    FhirSchema {
        url: Some(url),
        name,
        kind: Some(crate::schema::kind::PRIMITIVE_TYPE.to_string()),
        derivation,
        type_: Some(sd_type.to_string()),
        regex,
        ..Default::default()
    }
}
