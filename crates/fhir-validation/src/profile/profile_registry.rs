use crate::profile::types::ExtractedProfile;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ProfileRegistry {
    profiles: HashMap<String, ExtractedProfile>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, profile: ExtractedProfile) -> Option<ExtractedProfile> {
        self.profiles.insert(profile.url.clone(), profile)
    }

    pub fn as_map(&self) -> &HashMap<String, ExtractedProfile> {
        &self.profiles
    }

    pub fn get(&self, url: &str) -> Option<&ExtractedProfile> {
        self.profiles.get(url)
    }

    pub fn contains(&self, url: &str) -> bool {
        self.profiles.contains_key(url)
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ExtractedProfile)> {
        self.profiles.iter()
    }
}
