//! Internal [`crate::ValidationIssue::code`] strings (validator categories before FHIR mapping),
//! plus a shared JSON key alias where it matches the same spelling.

pub const STRUCTURE: &str = "structure";
pub const VALUE: &str = "value";
/// JSON property name for primitive wrapper shapes such as `{"value": ...}`.
pub const FHIR_JSON_VALUE: &str = VALUE;
pub const TERMINOLOGY: &str = "terminology";
pub const INVARIANT: &str = "invariant";
pub const EXCEPTION: &str = "exception";
