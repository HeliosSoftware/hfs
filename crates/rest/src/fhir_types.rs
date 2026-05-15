//! FHIR type utilities backed by the generated `Resource` enum.
//!
//! Resource type names are sourced from `helios_fhir::r{4,4b,5,6}::Resource` via
//! the `FhirResourceTypeProvider` trait — there is no hand-maintained list to
//! drift from `fhir-gen` output. Each per-version list is computed once on first
//! use and cached behind a `OnceLock` so request-time validation stays O(N) over
//! the cached `&'static [&'static str]`.

use helios_fhir::{FhirResourceTypeProvider, FhirVersion};
use std::sync::OnceLock;

#[cfg(feature = "R4")]
fn r4_resource_types() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(
            <helios_fhir::r4::Resource as FhirResourceTypeProvider>::get_resource_type_names,
        )
        .as_slice()
}

#[cfg(feature = "R4B")]
fn r4b_resource_types() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(
            <helios_fhir::r4b::Resource as FhirResourceTypeProvider>::get_resource_type_names,
        )
        .as_slice()
}

#[cfg(feature = "R5")]
fn r5_resource_types() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(
            <helios_fhir::r5::Resource as FhirResourceTypeProvider>::get_resource_type_names,
        )
        .as_slice()
}

#[cfg(feature = "R6")]
fn r6_resource_types() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(
            <helios_fhir::r6::Resource as FhirResourceTypeProvider>::get_resource_type_names,
        )
        .as_slice()
}

/// Returns resource type names for the build's default FHIR version.
///
/// The priority order when multiple versions are enabled is R4 > R4B > R5 > R6,
/// matching `FhirVersion::default()` (which is gated on `feature = "R4"`).
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::fhir_types::get_resource_type_names;
///
/// let types = get_resource_type_names();
/// assert!(types.contains(&"Patient"));
/// ```
pub fn get_resource_type_names() -> &'static [&'static str] {
    #[cfg(feature = "R4")]
    {
        return r4_resource_types();
    }

    #[cfg(all(feature = "R4B", not(feature = "R4")))]
    {
        return r4b_resource_types();
    }

    #[cfg(all(feature = "R5", not(any(feature = "R4", feature = "R4B"))))]
    {
        return r5_resource_types();
    }

    #[cfg(all(
        feature = "R6",
        not(any(feature = "R4", feature = "R4B", feature = "R5"))
    ))]
    {
        return r6_resource_types();
    }

    // No FHIR version enabled — code below assumes at least one is, per CLAUDE.md.
    #[allow(unreachable_code)]
    &[]
}

/// Checks if a resource type name is valid for any FHIR version enabled in this build.
///
/// The comparison is case-sensitive as per FHIR specification. A type is accepted
/// if it appears in the resource list of *any* enabled FHIR version, so that
/// multi-version builds (e.g. `R4,R4B`) accept resource types introduced in the
/// later version (e.g. `SubscriptionTopic`) when running with that version active.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::fhir_types::is_valid_resource_type;
///
/// assert!(is_valid_resource_type("Patient"));
/// assert!(!is_valid_resource_type("InvalidType"));
/// assert!(!is_valid_resource_type("patient")); // Case-sensitive
/// ```
pub fn is_valid_resource_type(type_name: &str) -> bool {
    #[cfg(feature = "R4")]
    if r4_resource_types().contains(&type_name) {
        return true;
    }
    #[cfg(feature = "R4B")]
    if r4b_resource_types().contains(&type_name) {
        return true;
    }
    #[cfg(feature = "R5")]
    if r5_resource_types().contains(&type_name) {
        return true;
    }
    #[cfg(feature = "R6")]
    if r6_resource_types().contains(&type_name) {
        return true;
    }
    false
}

/// Returns the FHIR version string for the enabled version.
///
/// This is useful for including in CapabilityStatements and other metadata.
pub fn get_fhir_version() -> &'static str {
    #[cfg(feature = "R4")]
    {
        return "4.0.1";
    }

    #[cfg(all(feature = "R4B", not(feature = "R4")))]
    {
        return "4.3.0";
    }

    #[cfg(all(feature = "R5", not(any(feature = "R4", feature = "R4B"))))]
    {
        return "5.0.0";
    }

    #[cfg(all(
        feature = "R6",
        not(any(feature = "R4", feature = "R4B", feature = "R5"))
    ))]
    {
        return "6.0.0-ballot";
    }

    #[allow(unreachable_code)]
    "unknown"
}

/// Returns resource type names for a specific FHIR version.
///
/// Falls back to the default version's list if the requested version is not
/// enabled in this build.
pub fn get_resource_type_names_for_version(version: FhirVersion) -> &'static [&'static str] {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => r4_resource_types(),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => r4b_resource_types(),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => r5_resource_types(),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => r6_resource_types(),
        #[allow(unreachable_patterns)]
        _ => get_resource_type_names(),
    }
}

/// Checks if a resource type name is valid for a specific FHIR version.
pub fn is_valid_resource_type_for_version(type_name: &str, version: FhirVersion) -> bool {
    get_resource_type_names_for_version(version).contains(&type_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_resource_type_names() {
        let types = get_resource_type_names();

        // Should have a reasonable number of types
        assert!(!types.is_empty());

        // Should include common types
        assert!(types.contains(&"Patient"));
        assert!(types.contains(&"Observation"));
        assert!(types.contains(&"Bundle"));
        assert!(types.contains(&"CapabilityStatement"));
    }

    #[test]
    fn test_is_valid_resource_type() {
        // Valid types
        assert!(is_valid_resource_type("Patient"));
        assert!(is_valid_resource_type("Observation"));
        assert!(is_valid_resource_type("Bundle"));

        // Invalid types
        assert!(!is_valid_resource_type("InvalidType"));
        assert!(!is_valid_resource_type(""));
        assert!(!is_valid_resource_type("patient")); // Case-sensitive
    }

    #[test]
    fn test_get_fhir_version() {
        let version = get_fhir_version();
        // Should be a valid version string
        assert!(!version.is_empty());
        assert!(version.contains('.'));
    }

    // Regression: in multi-version builds, types introduced in a later version
    // (e.g. SubscriptionTopic in R4B/R5/R6) must validate even when an earlier
    // version is also enabled. See HFS subscriptions smoke matrix.
    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn test_subscription_topic_valid_when_post_r4_enabled() {
        assert!(is_valid_resource_type("SubscriptionTopic"));
    }

    #[cfg(feature = "R4B")]
    #[test]
    fn test_per_version_lookup_returns_r4b_types() {
        let types = get_resource_type_names_for_version(FhirVersion::R4B);
        assert!(types.contains(&"SubscriptionTopic"));
    }

    #[cfg(feature = "R5")]
    #[test]
    fn test_per_version_lookup_returns_r5_types() {
        let types = get_resource_type_names_for_version(FhirVersion::R5);
        assert!(types.contains(&"SubscriptionTopic"));
    }

    #[cfg(feature = "R6")]
    #[test]
    fn test_per_version_lookup_returns_r6_types() {
        let types = get_resource_type_names_for_version(FhirVersion::R6);
        assert!(types.contains(&"SubscriptionTopic"));
    }
}
