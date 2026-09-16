//! Build identity: the version (and, when known, git commit) this server was
//! compiled from.
//!
//! Surfaced on the two places a tester or operator looks for it — `hfs
//! --version` and `CapabilityStatement.software` — so a build can be recorded
//! from the artifact under test rather than from shell history (#992).
//! `/health` reports the same [`PKG_VERSION`].

/// Product name advertised as `CapabilityStatement.software.name`.
pub const SOFTWARE_NAME: &str = "Helios FHIR Server";

/// Crate version (`CARGO_PKG_VERSION`). Every workspace crate shares the
/// workspace version, so this is also the `hfs` binary's version.
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Abbreviated git commit the build was cut from, or `""` when the build
/// script could not determine one (source tarball with no `.git`, no `git` on
/// `PATH`). Set by `build.rs`; overridable at build time via `HFS_GIT_SHA`.
const GIT_SHA_RAW: &str = env!("HFS_BUILD_GIT_SHA");

/// What `--version` prints after the binary name: `0.2.1 (git 1a2b3c4d5)`, or
/// just `0.2.1` when the commit is unknown.
///
/// Built with `concat!` so it is a `&'static str` usable in clap's
/// `#[command(version = ...)]`; the build script emits the parenthesised
/// suffix (or an empty string) as `HFS_BUILD_VERSION_SUFFIX`.
pub const VERSION_STRING: &str =
    concat!(env!("CARGO_PKG_VERSION"), env!("HFS_BUILD_VERSION_SUFFIX"));

/// Extension URL carrying the git commit on `CapabilityStatement.software`.
///
/// `software.version` stays the plain package version (what a client compares
/// against a release number); the commit rides alongside it as an extension,
/// only when known.
pub const GIT_SHA_EXTENSION_URL: &str =
    "http://heliossoftware.com/fhir/StructureDefinition/capabilitystatement-software-git-sha";

/// The git commit the build was cut from, when the build script could
/// determine one.
pub fn git_sha() -> Option<&'static str> {
    (!GIT_SHA_RAW.is_empty()).then_some(GIT_SHA_RAW)
}

/// `CapabilityStatement.software` for this build.
pub fn capability_software() -> serde_json::Value {
    let mut software = serde_json::json!({
        "name": SOFTWARE_NAME,
        "version": PKG_VERSION,
    });
    if let Some(sha) = git_sha() {
        software["extension"] = serde_json::json!([{
            "url": GIT_SHA_EXTENSION_URL,
            "valueString": sha,
        }]);
    }
    software
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_starts_with_package_version() {
        assert!(
            VERSION_STRING.starts_with(PKG_VERSION),
            "{VERSION_STRING:?} must start with {PKG_VERSION:?}"
        );
        assert!(!PKG_VERSION.is_empty());
    }

    #[test]
    fn version_string_carries_git_sha_exactly_when_known() {
        match git_sha() {
            Some(sha) => {
                assert!(!sha.is_empty());
                assert_eq!(VERSION_STRING, format!("{PKG_VERSION} (git {sha})"));
            }
            None => assert_eq!(VERSION_STRING, PKG_VERSION),
        }
    }

    #[test]
    fn capability_software_names_the_build() {
        let software = capability_software();
        assert_eq!(software["name"], SOFTWARE_NAME);
        assert_eq!(software["version"], PKG_VERSION);
        match git_sha() {
            Some(sha) => {
                assert_eq!(software["extension"][0]["url"], GIT_SHA_EXTENSION_URL);
                assert_eq!(software["extension"][0]["valueString"], sha);
            }
            None => assert!(software.get("extension").is_none()),
        }
    }
}
