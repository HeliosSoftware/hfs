//! Post-boot builder for the generation-2 `search_index` indexes (#1059, #1084).

use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How `SearchIndexBuilder` runs relative to boot. Read from
/// `HFS_MONGODB_INDEX_BUILD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexBuildMode {
    /// Spawn the builder after boot and return immediately (default).
    #[default]
    Background,
    /// Await the builder before boot completes. Tests and small databases.
    Inline,
    /// Inspect and warn only. The operator builds out of band.
    Off,
}

impl FromStr for IndexBuildMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "background" => Ok(Self::Background),
            "inline" => Ok(Self::Inline),
            "off" => Ok(Self::Off),
            other => Err(format!(
                "HFS_MONGODB_INDEX_BUILD must be one of background, inline, off; got {other:?}"
            )),
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_case_insensitively() {
        assert_eq!(
            "background".parse::<IndexBuildMode>(),
            Ok(IndexBuildMode::Background)
        );
        assert_eq!(
            " Inline ".parse::<IndexBuildMode>(),
            Ok(IndexBuildMode::Inline)
        );
        assert_eq!("OFF".parse::<IndexBuildMode>(), Ok(IndexBuildMode::Off));
        assert!("sometimes".parse::<IndexBuildMode>().is_err());
    }

    #[test]
    fn default_is_background() {
        assert_eq!(IndexBuildMode::default(), IndexBuildMode::Background);
    }
}
