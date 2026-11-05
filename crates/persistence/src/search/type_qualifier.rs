//! The `:[type]` qualifier rule, in one place.
//!
//! `subject:Patient=123` restricts a reference parameter to one target type.
//! [`SearchModifier::parse`] reads *any* capitalised suffix as such a
//! qualifier, so every parser of search parameter names has to check the name
//! against real resource types: `subject:Bogus` is no more a modifier than
//! `subject:bogus`. The REST query builder (direct search, chains, `_has`) and
//! the conditional-criteria builder of this crate both go through
//! [`ResourceTypeScope`], so the rule, its case-sensitivity and its error text
//! cannot drift apart (#1366).

use std::sync::OnceLock;

use helios_fhir::{FhirResourceTypeProvider, FhirVersion};

use crate::types::SearchModifier;

/// The resource types a `:[type]` qualifier may name: those of the FHIR version
/// the search runs against, or — for a caller that cannot say — those of every
/// FHIR version enabled in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceTypeScope(Option<FhirVersion>);

impl ResourceTypeScope {
    /// The resource types of `fhir_version`.
    pub const fn version(fhir_version: FhirVersion) -> Self {
        Self(Some(fhir_version))
    }

    /// The resource types of every enabled FHIR version. Only for a caller
    /// that has no version to judge against: on a multi-version build it
    /// accepts a type the search's own version does not have.
    pub const fn any_enabled() -> Self {
        Self(None)
    }

    /// The version this scope judges against, if it judges against one.
    pub fn fhir_version(self) -> Option<FhirVersion> {
        self.0
    }

    /// Whether `type_name` is, case-sensitively, a resource type in scope.
    pub fn contains(self, type_name: &str) -> bool {
        self.lists().any(|names| names.contains(&type_name))
    }

    /// Parses a `:suffix` as a search modifier, or as the `:[type]` qualifier
    /// of a reference parameter — which must name a resource type in scope.
    ///
    /// Whether the modifier suits the parameter's *type* (a `:[type]`
    /// qualifier is only defined for references) is
    /// [`validate_modifier`](super::validate_modifier)'s job.
    pub fn parse_modifier(self, suffix: &str) -> Option<SearchModifier> {
        match SearchModifier::parse(suffix)? {
            SearchModifier::Type(t) if !self.contains(&t) => None,
            modifier => Some(modifier),
        }
    }

    /// The error text for a `:suffix` on `param_name` that
    /// [`parse_modifier`](Self::parse_modifier) refused.
    pub fn unknown_modifier_message(self, suffix: &str, param_name: &str) -> String {
        format!(
            "unknown search modifier ':{suffix}' on parameter '{param_name}'; it is neither a \
             search modifier nor a resource type{}{}",
            self.version_clause(),
            self.case_hint(suffix)
        )
    }

    /// Names the FHIR version a `:[type]` qualifier was judged against, when
    /// it was judged against one: ` of FHIR R4`.
    pub fn version_clause(self) -> String {
        self.0.map(|v| format!(" of FHIR {v}")).unwrap_or_default()
    }

    /// Modifiers and resource type names are case-sensitive (`name:EXACT` and
    /// `subject:patient` are neither). When an unknown `:suffix` is one of
    /// them in a different case, this is the clause that says so.
    pub fn case_hint(self, suffix: &str) -> String {
        let lower = suffix.to_lowercase();
        let intended = if lower != suffix && SearchModifier::parse(&lower).is_some() {
            Some(lower)
        } else {
            self.lists()
                .flat_map(|names| names.iter())
                .find(|t| **t != suffix && t.eq_ignore_ascii_case(suffix))
                .map(|t| t.to_string())
        };
        match intended {
            Some(i) => format!(" (modifiers and resource type names are case-sensitive: ':{i}'?)"),
            None => String::new(),
        }
    }

    /// The name tables in scope. A version this build has no table for is
    /// judged against every enabled version rather than against nothing.
    fn lists(self) -> impl Iterator<Item = &'static [&'static str]> {
        let of_version = self.0.and_then(resource_type_names);
        let all = of_version.is_none().then(|| {
            FhirVersion::enabled_versions()
                .iter()
                .filter_map(|v| resource_type_names(*v))
        });
        of_version.into_iter().chain(all.into_iter().flatten())
    }
}

/// The resource type names of `version`, or `None` when this crate was built
/// without that version's feature.
fn resource_type_names(version: FhirVersion) -> Option<&'static [&'static str]> {
    fn cached<R: FhirResourceTypeProvider>(
        cache: &'static OnceLock<Vec<&'static str>>,
    ) -> &'static [&'static str] {
        cache.get_or_init(R::get_resource_type_names).as_slice()
    }

    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
            Some(cached::<helios_fhir::r4::Resource>(&NAMES))
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
            Some(cached::<helios_fhir::r4b::Resource>(&NAMES))
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
            Some(cached::<helios_fhir::r5::Resource>(&NAMES))
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
            Some(cached::<helios_fhir::r6::Resource>(&NAMES))
        }
        // helios-fhir can have a version enabled (by another crate in the
        // build) that this crate's own features do not name.
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_type_qualifier_must_name_a_resource_type_case_sensitively() {
        for scope in [
            ResourceTypeScope::any_enabled(),
            ResourceTypeScope::version(FhirVersion::default_enabled()),
        ] {
            assert_eq!(
                scope.parse_modifier("Patient"),
                Some(SearchModifier::Type("Patient".to_string()))
            );
            assert_eq!(scope.parse_modifier("exact"), Some(SearchModifier::Exact));
            assert_eq!(scope.parse_modifier("Bogus"), None);
            assert_eq!(scope.parse_modifier("patient"), None);
            assert_eq!(scope.parse_modifier("PATIENT"), None);
            assert_eq!(scope.parse_modifier("EXACT"), None);
        }
    }

    #[test]
    fn the_message_names_the_version_and_the_probable_spelling() {
        let version = FhirVersion::default_enabled();
        let message =
            ResourceTypeScope::version(version).unknown_modifier_message("patient", "subject");
        assert!(
            message.contains(&format!("nor a resource type of FHIR {version}")),
            "{message}"
        );
        assert!(message.contains("case-sensitive: ':Patient'?"), "{message}");

        let message = ResourceTypeScope::any_enabled().unknown_modifier_message("EXACT", "name");
        assert!(!message.contains("of FHIR"), "{message}");
        assert!(message.contains("case-sensitive: ':exact'?"), "{message}");

        let message = ResourceTypeScope::any_enabled().unknown_modifier_message("Bogus", "subject");
        assert!(!message.contains("case-sensitive"), "{message}");
    }

    /// Only a multi-version build can tell a scoped check from an unscoped one.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn a_type_of_another_enabled_version_is_out_of_scope() {
        // ActorDefinition is new in R5; DocumentManifest did not survive R4B.
        let (r4, r5) = (
            ResourceTypeScope::version(FhirVersion::R4),
            ResourceTypeScope::version(FhirVersion::R5),
        );
        assert!(r5.contains("ActorDefinition") && !r4.contains("ActorDefinition"));
        assert!(r4.contains("DocumentManifest") && !r5.contains("DocumentManifest"));
        assert_eq!(r4.parse_modifier("ActorDefinition"), None);
        assert!(
            ResourceTypeScope::any_enabled()
                .parse_modifier("ActorDefinition")
                .is_some()
        );
        assert!(
            r4.unknown_modifier_message("ActorDefinition", "subject")
                .contains("nor a resource type of FHIR R4")
        );
    }
}
