//! FHIR version matching for NPM `package.json` `fhirVersions`.

use helios_fhir::FhirVersion;

/// Whether a package that declares `fhir_versions` is compatible with `version`.
///
/// An empty declaration is treated as compatible with every release (many
/// fixtures and older packages omit the field). Declared strings may be full
/// versions (`4.0.1`), MIME params (`4.0`), or short labels (`R4` / `r4`).
pub fn manifest_supports_fhir_version(fhir_versions: &[String], version: FhirVersion) -> bool {
    if fhir_versions.is_empty() {
        return true;
    }
    fhir_versions
        .iter()
        .any(|declared| declared_matches_version(declared, version))
}

fn declared_matches_version(declared: &str, version: FhirVersion) -> bool {
    let d = declared.trim();
    if d.eq_ignore_ascii_case(version.as_str()) {
        return true;
    }
    if d == version.as_mime_param() || d == version.full_version() {
        return true;
    }
    // Prefix match: "4.0.1" accepts MIME "4.0"; "5.0.0-snapshot1" accepts "5.0".
    let mime = version.as_mime_param();
    d.starts_with(mime)
        && d.as_bytes()
            .get(mime.len())
            .is_none_or(|b| *b == b'.' || *b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "R4")]
    fn empty_supports_all() {
        assert!(manifest_supports_fhir_version(&[], FhirVersion::R4));
    }

    #[test]
    #[cfg(feature = "R4")]
    fn r4_aliases() {
        for s in ["4.0.1", "4.0", "R4", "r4"] {
            assert!(
                manifest_supports_fhir_version(&[s.into()], FhirVersion::R4),
                "{s}"
            );
        }
        assert!(!manifest_supports_fhir_version(
            &["5.0.0".into()],
            FhirVersion::R4
        ));
    }
}
