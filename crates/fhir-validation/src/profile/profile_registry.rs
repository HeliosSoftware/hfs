//! In-memory **canonical URL → extracted profile** store for runtime validation.
//!
//! Keys are [`ExtractedProfile::url`] (the `StructureDefinition.url` string). Pass
//! [`ProfileRegistry`] by reference on [`crate::validation_context::ValidationContext`] /
//! [`crate::validation_context::AsyncValidationContext`] so that:
//!
//! - [`crate::profile::validate::validate_declared_profiles`] can resolve `meta.profile` claims.
//! - Nested `type.profile` validation can load referenced profiles without HTTP.
//! - Base-definition resolution can consult the registry before optional network fetch
//!   (when enabled).
//!
//! Population is **caller-defined**: load IGs at startup, bundle known `StructureDefinition`
//! JSON, etc. The registry does not fetch or parse on its own.

use crate::profile::types::ExtractedProfile;
use std::collections::HashMap;

/// Map of profile canonical URLs to pre-extracted [`ExtractedProfile`] definitions.
#[derive(Debug, Clone, Default)]
pub struct ProfileRegistry {
    profiles: HashMap<String, ExtractedProfile>,
}

impl ProfileRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a profile keyed by [`ExtractedProfile::url`]. Returns the previous entry, if any.
    pub fn insert(&mut self, profile: ExtractedProfile) -> Option<ExtractedProfile> {
        self.profiles.insert(profile.url.clone(), profile)
    }

    /// Borrow the underlying map for bulk inspection or serialization.
    pub fn as_map(&self) -> &HashMap<String, ExtractedProfile> {
        &self.profiles
    }

    /// Lookup by exact canonical URL string.
    pub fn get(&self, url: &str) -> Option<&ExtractedProfile> {
        self.profiles.get(url)
    }

    /// Returns whether `url` is a known key (same as `get(url).is_some()`).
    pub fn contains(&self, url: &str) -> bool {
        self.profiles.contains_key(url)
    }

    /// Number of registered profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Returns true if no profiles are registered.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Iterate URL / profile pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ExtractedProfile)> {
        self.profiles.iter()
    }
}
