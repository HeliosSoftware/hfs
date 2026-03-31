//! Convenience constructors for FHIR types.
//!
//! The generated FHIR structs wrap every primitive in `Element<V, E>`, making
//! programmatic construction verbose.  This module provides terse helpers that
//! produce valid FHIR types with minimal ceremony.

use chrono::Utc;
use helios_fhir::r4::{
    Boolean, Canonical, Code, CodeableConcept, Coding, Instant, Reference, String as FhirString,
    Uri,
};
use helios_fhir::{Element, PrecisionInstant};

// ── Primitive helpers ────────────────────────────────────────────────────────

/// Create a `Code` element from a string value.
pub fn code(value: impl Into<String>) -> Code {
    let s: String = value.into();
    Element {
        value: Some(s),
        id: None,
        extension: None,
    }
}

/// Create a `Uri` element from a string value.
pub fn uri(value: impl Into<String>) -> Uri {
    let s: String = value.into();
    Element {
        value: Some(s),
        id: None,
        extension: None,
    }
}

/// Create a `Canonical` element from a URL string.
pub fn canonical(value: impl Into<String>) -> Canonical {
    let s: String = value.into();
    Element {
        value: Some(s),
        id: None,
        extension: None,
    }
}

/// Create a `Boolean` element.
pub fn boolean(value: bool) -> Boolean {
    Element {
        value: Some(value),
        id: None,
        extension: None,
    }
}

/// Create an `Instant` element set to the current UTC time.
pub fn instant_now() -> Instant {
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    Element {
        value: PrecisionInstant::parse(&now),
        id: None,
        extension: None,
    }
}

/// Create a FHIR `String` element (`Element<String, Extension>`) from a Rust string.
///
/// Many FHIR struct fields use `r4::String` which is `Element<std::string::String, Extension>`,
/// not `std::string::String`. This helper bridges the gap.
pub fn fhir_string(value: impl Into<std::string::String>) -> FhirString {
    Element {
        value: Some(value.into()),
        id: None,
        extension: None,
    }
}

// ── Composite helpers ────────────────────────────────────────────────────────

/// Create a `Coding` with system and code.
pub fn coding(system: &str, code_value: &str) -> Coding {
    Coding {
        system: Some(uri(system)),
        code: Some(code(code_value)),
        ..Default::default()
    }
}

/// Create a `Coding` with system, code, and display text.
pub fn coding_display(system: &str, code_value: &str, display: &str) -> Coding {
    Coding {
        system: Some(uri(system)),
        code: Some(code(code_value)),
        display: Some(fhir_string(display)),
        ..Default::default()
    }
}

/// Create a `Reference` from a reference string (e.g. `"Patient/123"`).
pub fn reference(value: &str) -> Reference {
    Reference {
        reference: Some(fhir_string(value)),
        ..Default::default()
    }
}

/// Create a `CodeableConcept` wrapping a single `Coding`.
pub fn codeable_concept(c: Coding) -> CodeableConcept {
    CodeableConcept {
        coding: Some(vec![c]),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_element() {
        let c = code("R");
        assert_eq!(c.value.as_deref(), Some("R"));
        assert!(c.id.is_none());
        assert!(c.extension.is_none());
    }

    #[test]
    fn test_uri_element() {
        let u = uri("http://example.com");
        assert_eq!(u.value.as_deref(), Some("http://example.com"));
    }

    #[test]
    fn test_canonical_element() {
        let c =
            canonical("https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read");
        assert!(
            c.value
                .as_deref()
                .unwrap()
                .starts_with("https://profiles.ihe.net")
        );
    }

    #[test]
    fn test_boolean_element() {
        let t = boolean(true);
        assert_eq!(t.value, Some(true));
        let f = boolean(false);
        assert_eq!(f.value, Some(false));
    }

    #[test]
    fn test_instant_now_produces_valid_timestamp() {
        let inst = instant_now();
        assert!(inst.value.is_some());
        let s = inst.value.unwrap().to_string();
        // Should be ISO 8601 with Z suffix
        assert!(s.ends_with('Z'), "Expected Z suffix, got: {s}");
        assert!(s.contains('T'), "Expected T separator, got: {s}");
    }

    #[test]
    fn test_coding_sets_system_and_code() {
        let c = coding("http://example.com/cs", "abc");
        assert_eq!(
            c.system.as_ref().and_then(|s| s.value.as_deref()),
            Some("http://example.com/cs")
        );
        assert_eq!(
            c.code.as_ref().and_then(|s| s.value.as_deref()),
            Some("abc")
        );
        assert!(c.display.is_none());
    }

    #[test]
    fn test_coding_display() {
        let c = coding_display("http://example.com/cs", "abc", "ABC Label");
        assert_eq!(
            c.display.as_ref().and_then(|s| s.value.as_deref()),
            Some("ABC Label")
        );
    }

    #[test]
    fn test_reference_sets_reference_field() {
        let r = reference("Patient/123");
        assert_eq!(
            r.reference.as_ref().and_then(|s| s.value.as_deref()),
            Some("Patient/123")
        );
        assert!(r.display.is_none());
    }

    #[test]
    fn test_codeable_concept_wraps_coding() {
        let c = coding("sys", "code");
        let cc = codeable_concept(c);
        assert_eq!(cc.coding.as_ref().map(|v| v.len()), Some(1));
    }
}
