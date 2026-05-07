//! Load [`ProfileRegistry`](crate::profile::profile_registry::ProfileRegistry) entries from
//! on-disk `StructureDefinition` JSON (single resource or Bundle).
//!
//! Used for **IG materialization**: expand NPM packages in CI, list canonical JSON paths in a
//! manifest, then call [`load_profile_registry_from_manifest`] at process startup.
//!
//! CapabilityStatement / ImplementationGuide resources do **not** become registry rows; publish a
//! manifest that lists only `StructureDefinition` JSON paths your deployment validates against.
//!
//! See also [`scan_ig_package_for_fhir_json`] and [`build_and_write_profile_manifest_for_ig`] to
//! **generate** a manifest from an expanded FHIR NPM / IG folder.
//!
//! Path layout is chosen with [`ProfileManifestPathStyle`]: **absolute** (default, independent of
//! CWD) or **relative to the manifest file’s parent** (portable in repos).

use crate::ValidationError;
use crate::profile::extract::extract_structure_definition_profile_from_json;
use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::types::ExtractedProfile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// How JSON path strings are written when building a manifest from a directory scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileManifestPathStyle {
    /// Canonical absolute paths (via [`std::fs::canonicalize`] when possible). Does not depend on
    /// the process working directory when loading.
    #[default]
    Absolute,
    /// Paths relative to the output manifest file’s **parent directory** (portable when the
    /// manifest and IG tree live under a common folder). Resolving listed files still depends on
    /// **loader CWD** unless you run from that parent or rewrite paths.
    RelativeToManifestParent,
}

/// Paths (relative or absolute) to JSON files. Each file is either a `StructureDefinition`
/// resource or a FHIR `Bundle` containing `StructureDefinition` entries.
///
/// [`code_system_files`] and [`value_set_files`] are recorded for **operators** (HTS import,
/// audits, split tooling). They are **ignored** by [`load_profile_registry_from_manifest`], which
/// only loads [`structure_definition_files`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileManifest {
    #[serde(default)]
    pub structure_definition_files: Vec<String>,
    /// Paths to standalone `CodeSystem` JSON (or mixed Bundles that contain CodeSystems).
    ///
    /// Not consumed by [`load_profile_registry_from_manifest`]; use HTS import or other
    /// terminology loading for these paths.
    #[serde(default)]
    pub code_system_files: Vec<String>,
    /// Paths to standalone `ValueSet` JSON (or mixed Bundles that contain ValueSets).
    ///
    /// Not consumed by [`load_profile_registry_from_manifest`].
    #[serde(default)]
    pub value_set_files: Vec<String>,
}

/// Discovered JSON resources under an expanded IG / NPM `package` tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScannedIgResources {
    pub structure_definition_paths: Vec<PathBuf>,
    pub code_system_paths: Vec<PathBuf>,
    pub value_set_paths: Vec<PathBuf>,
}

/// Recursively find `*.json` under `ig_root`, parse each file’s root `resourceType`, and classify
/// standalone resources. **`Bundle`** entries are inspected: a file may be listed in more than one
/// bucket if it is a collection Bundle containing multiple resource kinds.
///
/// Unknown or non-object JSON files are skipped (no error). Malformed files log behaviour: parse
/// failure skips the file.
pub fn scan_ig_package_for_fhir_json(
    ig_root: &Path,
) -> Result<ScannedIgResources, ValidationError> {
    if !ig_root.is_dir() {
        return Err(ValidationError::Internal(format!(
            "IG root is not a directory: {}",
            ig_root.display()
        )));
    }

    let mut json_files = Vec::new();
    walk_json_files(ig_root, &mut json_files).map_err(|e| {
        ValidationError::Internal(format!("failed walking IG tree {}: {e}", ig_root.display()))
    })?;
    json_files.sort();
    json_files.dedup();

    let mut out = ScannedIgResources::default();
    for path in json_files {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let (sd, cs, vs) = classify_fhir_json(&v);
        if sd {
            out.structure_definition_paths.push(path.clone());
        }
        if cs {
            out.code_system_paths.push(path.clone());
        }
        if vs {
            out.value_set_paths.push(path.clone());
        }
    }

    out.structure_definition_paths.sort();
    out.structure_definition_paths.dedup();
    out.code_system_paths.sort();
    out.code_system_paths.dedup();
    out.value_set_paths.sort();
    out.value_set_paths.dedup();

    Ok(out)
}

fn walk_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            walk_json_files(&p, out)?;
        } else if is_json_file(&p) {
            out.push(p);
        }
    }
    Ok(())
}

fn is_json_file(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// Classify a parsed JSON value. Returns flags `(has_structure_definition, has_code_system, has_value_set)`.
fn classify_fhir_json(v: &serde_json::Value) -> (bool, bool, bool) {
    match v.get("resourceType").and_then(|x| x.as_str()) {
        Some("StructureDefinition") => (true, false, false),
        Some("CodeSystem") => (false, true, false),
        Some("ValueSet") => (false, false, true),
        Some("Bundle") => classify_bundle(v),
        _ => (false, false, false),
    }
}

fn classify_bundle(v: &serde_json::Value) -> (bool, bool, bool) {
    let mut sd = false;
    let mut cs = false;
    let mut vs = false;
    let Some(entries) = v.get("entry").and_then(|e| e.as_array()) else {
        return (false, false, false);
    };
    for entry in entries {
        let Some(res) = entry.get("resource") else {
            continue;
        };
        match res.get("resourceType").and_then(|x| x.as_str()) {
            Some("StructureDefinition") => sd = true,
            Some("CodeSystem") => cs = true,
            Some("ValueSet") => vs = true,
            _ => {}
        }
    }
    (sd, cs, vs)
}

/// Build a [`ProfileManifest`] from a scan using [`ProfileManifestPathStyle`].
///
/// For [`ProfileManifestPathStyle::RelativeToManifestParent`], pass `manifest_out` (the path you
/// will write the manifest to); its parent becomes the base for relative paths. With
/// [`ProfileManifestPathStyle::Absolute`], `manifest_out` may be `None`.
pub fn profile_manifest_from_scan_with_style(
    scan: &ScannedIgResources,
    path_style: ProfileManifestPathStyle,
    manifest_out: Option<&Path>,
) -> Result<ProfileManifest, ValidationError> {
    match path_style {
        ProfileManifestPathStyle::Absolute => Ok(profile_manifest_from_scan_absolute(scan)),
        ProfileManifestPathStyle::RelativeToManifestParent => {
            let manifest_out = manifest_out.ok_or_else(|| {
                ValidationError::Internal(
                    "manifest_out is required when path_style is RelativeToManifestParent"
                        .to_string(),
                )
            })?;
            profile_manifest_from_scan_relative(scan, manifest_out)
        }
    }
}

/// Same as [`profile_manifest_from_scan_with_style`] with [`ProfileManifestPathStyle::Absolute`].
pub fn profile_manifest_from_scan(scan: &ScannedIgResources) -> ProfileManifest {
    profile_manifest_from_scan_absolute(scan)
}

fn profile_manifest_from_scan_absolute(scan: &ScannedIgResources) -> ProfileManifest {
    ProfileManifest {
        structure_definition_files: scan
            .structure_definition_paths
            .iter()
            .map(|p| manifest_absolute_path_string(p.as_path()))
            .collect(),
        code_system_files: scan
            .code_system_paths
            .iter()
            .map(|p| manifest_absolute_path_string(p.as_path()))
            .collect(),
        value_set_files: scan
            .value_set_paths
            .iter()
            .map(|p| manifest_absolute_path_string(p.as_path()))
            .collect(),
    }
}

fn profile_manifest_from_scan_relative(
    scan: &ScannedIgResources,
    manifest_out: &Path,
) -> Result<ProfileManifest, ValidationError> {
    let base_dir = manifest_out.parent().unwrap_or_else(|| Path::new("."));
    let base_canon = fs::canonicalize(base_dir).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to canonicalize manifest parent '{}': {e}",
            base_dir.display()
        ))
    })?;

    let rel = |p: &Path| -> Result<String, ValidationError> {
        let p = fs::canonicalize(p).map_err(|e| {
            ValidationError::Internal(format!(
                "failed to canonicalize resource path '{}': {e}",
                p.display()
            ))
        })?;
        Ok(path_as_posix_string(
            &make_path_relative_to(&base_canon, &p).unwrap_or(p),
        ))
    };

    Ok(ProfileManifest {
        structure_definition_files: scan
            .structure_definition_paths
            .iter()
            .map(|p| rel(p))
            .collect::<Result<Vec<_>, _>>()?,
        code_system_files: scan
            .code_system_paths
            .iter()
            .map(|p| rel(p))
            .collect::<Result<Vec<_>, _>>()?,
        value_set_files: scan
            .value_set_paths
            .iter()
            .map(|p| rel(p))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn manifest_absolute_path_string(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

/// Strip `base` as a logical prefix from `path` when they share a root; otherwise walk up with `..`.
fn make_path_relative_to(base: &Path, path: &Path) -> Option<PathBuf> {
    let base_v: Vec<Component<'_>> = base
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    let path_v: Vec<Component<'_>> = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();

    let mut i = 0usize;
    while i < base_v.len() && i < path_v.len() && base_v[i] == path_v[i] {
        i += 1;
    }

    if i == base_v.len() {
        let mut out = PathBuf::new();
        for c in &path_v[i..] {
            out.push(c.as_os_str());
        }
        return if out.as_os_str().is_empty() {
            Some(PathBuf::from("."))
        } else {
            Some(out)
        };
    }

    let up = base_v.len() - i;
    let mut out = PathBuf::new();
    for _ in 0..up {
        out.push("..");
    }
    for c in &path_v[i..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(out)
    }
}

fn path_as_posix_string(p: &Path) -> String {
    p.iter()
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Write manifest JSON (pretty-printed UTF-8).
pub fn write_profile_manifest_to_file(
    manifest: &ProfileManifest,
    manifest_path: &Path,
) -> Result<(), ValidationError> {
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            ValidationError::Internal(format!(
                "failed creating manifest parent '{}': {e}",
                parent.display()
            ))
        })?;
    }
    let text = serde_json::to_string_pretty(manifest)
        .map_err(|e| ValidationError::Internal(format!("failed serializing manifest JSON: {e}")))?;
    fs::write(manifest_path, text).map_err(|e| {
        ValidationError::Internal(format!(
            "failed writing manifest '{}': {e}",
            manifest_path.display()
        ))
    })?;
    Ok(())
}

/// Scan `ig_root`, build a [`ProfileManifest`], and write `manifest_out`.
///
/// [`path_style`](ProfileManifestPathStyle) selects absolute paths vs paths relative to
/// `manifest_out`’s parent directory.
pub fn build_and_write_profile_manifest_for_ig(
    ig_root: &Path,
    manifest_out: &Path,
    path_style: ProfileManifestPathStyle,
) -> Result<ProfileManifest, ValidationError> {
    let scan = scan_ig_package_for_fhir_json(ig_root)?;
    let manifest = profile_manifest_from_scan_with_style(&scan, path_style, Some(manifest_out))?;
    write_profile_manifest_to_file(&manifest, manifest_out)?;
    Ok(manifest)
}

/// Load and merge all profiles from [`ProfileManifest::structure_definition_files`].
pub fn load_profile_registry_from_manifest(
    manifest: &ProfileManifest,
) -> Result<ProfileRegistry, ValidationError> {
    let mut registry = ProfileRegistry::new();
    for path in &manifest.structure_definition_files {
        merge_structure_definition_file_into_registry(Path::new(path), &mut registry)?;
    }
    Ok(registry)
}

/// Read a manifest JSON file (`structure_definition_files` array) and build a registry.
pub fn load_profile_registry_from_manifest_file(
    path: &Path,
) -> Result<ProfileRegistry, ValidationError> {
    let text = fs::read_to_string(path).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to read profile manifest '{}': {e}",
            path.display()
        ))
    })?;
    let manifest: ProfileManifest = serde_json::from_str(&text).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to parse profile manifest '{}': {e}",
            path.display()
        ))
    })?;
    load_profile_registry_from_manifest(&manifest)
}

fn merge_structure_definition_file_into_registry(
    path: &Path,
    registry: &mut ProfileRegistry,
) -> Result<(), ValidationError> {
    let text = fs::read_to_string(path).map_err(|e| {
        ValidationError::Internal(format!(
            "failed to read StructureDefinition file '{}': {e}",
            path.display()
        ))
    })?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        ValidationError::Internal(format!("failed to parse JSON '{}': {e}", path.display()))
    })?;

    match v.get("resourceType").and_then(|x| x.as_str()) {
        Some("StructureDefinition") => {
            insert_extracted_profile(registry, &v)?;
        }
        Some("Bundle") => {
            let Some(entries) = v.get("entry").and_then(|e| e.as_array()) else {
                return Err(ValidationError::Internal(format!(
                    "Bundle '{}' has no entry array",
                    path.display()
                )));
            };
            for entry in entries {
                let Some(res) = entry.get("resource") else {
                    continue;
                };
                if res.get("resourceType").and_then(|x| x.as_str()) != Some("StructureDefinition") {
                    continue;
                }
                insert_extracted_profile(registry, res)?;
            }
        }
        Some(other) => {
            return Err(ValidationError::Internal(format!(
                "expected StructureDefinition or Bundle in '{}', got resourceType={other}",
                path.display()
            )));
        }
        None => {
            return Err(ValidationError::Internal(format!(
                "missing resourceType in '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}

fn insert_extracted_profile(
    registry: &mut ProfileRegistry,
    sd_json: &serde_json::Value,
) -> Result<(), ValidationError> {
    let profile: ExtractedProfile = extract_structure_definition_profile_from_json(sd_json)?;
    registry.insert(profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loads_minimal_bundle_with_structure_definition() {
        let dir = std::env::temp_dir().join(format!("fv_manifest_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bundle_path = dir.join("bundle.json");
        let sd: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/r4/profiles/StructureDefinition-Patient.json"
        ))
        .unwrap();
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{ "resource": sd }]
        });
        std::fs::write(&bundle_path, serde_json::to_string(&bundle).unwrap()).unwrap();

        let mf_path = dir.join("manifest.json");
        std::fs::write(
            &mf_path,
            serde_json::to_string(&serde_json::json!({
                "structure_definition_files": [bundle_path.to_str().unwrap()]
            }))
            .unwrap(),
        )
        .unwrap();

        let reg = load_profile_registry_from_manifest_file(&mf_path).expect("load");
        assert!(!reg.is_empty());
    }

    #[test]
    fn scan_classifies_standalone_resources() {
        let dir = std::env::temp_dir().join(format!("fv_ig_scan_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("package/StructureDefinition")).unwrap();
        std::fs::create_dir_all(dir.join("package/CodeSystem")).unwrap();
        std::fs::create_dir_all(dir.join("package/ValueSet")).unwrap();
        let sd = json!({"resourceType":"StructureDefinition","url":"http://example.org/sd","kind":"resource","derivation":"constraint","type":"Patient","differential":{"element":[]}});
        std::fs::write(
            dir.join("package/StructureDefinition/sd.json"),
            serde_json::to_string(&sd).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("package/CodeSystem/cs.json"),
            serde_json::to_string(&json!({"resourceType":"CodeSystem","url":"http://example.org/cs","status":"active","content":"complete","concept":[]})).unwrap(),
        ).unwrap();
        std::fs::write(
            dir.join("package/ValueSet/vs.json"),
            serde_json::to_string(
                &json!({"resourceType":"ValueSet","url":"http://example.org/vs","status":"active"}),
            )
            .unwrap(),
        )
        .unwrap();

        let scan = scan_ig_package_for_fhir_json(&dir).expect("scan");
        assert_eq!(scan.structure_definition_paths.len(), 1);
        assert_eq!(scan.code_system_paths.len(), 1);
        assert_eq!(scan.value_set_paths.len(), 1);
    }

    #[test]
    fn build_manifest_writes_three_groups() {
        let dir = std::env::temp_dir().join(format!("fv_ig_build_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("package")).unwrap();
        std::fs::write(
            dir.join("package/x.json"),
            serde_json::to_string(
                &json!({"resourceType":"ValueSet","url":"http://x","status":"active"}),
            )
            .unwrap(),
        )
        .unwrap();
        let mf = dir.join("profile-manifest.json");
        let manifest =
            build_and_write_profile_manifest_for_ig(&dir, &mf, ProfileManifestPathStyle::Absolute)
                .expect("build");
        assert_eq!(manifest.value_set_files.len(), 1);
        assert!(manifest.structure_definition_files.is_empty());
        let text = std::fs::read_to_string(&mf).unwrap();
        assert!(text.contains("value_set_files"));
        assert!(
            Path::new(&manifest.value_set_files[0]).is_absolute(),
            "expected absolute path in manifest, got {:?}",
            manifest.value_set_files[0]
        );
    }

    #[test]
    fn build_manifest_relative_paths_under_flag() {
        let dir = std::env::temp_dir().join(format!("fv_ig_build_rel_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("package")).unwrap();
        std::fs::write(
            dir.join("package/y.json"),
            serde_json::to_string(
                &json!({"resourceType":"ValueSet","url":"http://y","status":"active"}),
            )
            .unwrap(),
        )
        .unwrap();
        let mf = dir.join("out/profile-manifest.json");
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let manifest = build_and_write_profile_manifest_for_ig(
            &dir,
            &mf,
            ProfileManifestPathStyle::RelativeToManifestParent,
        )
        .expect("build");
        assert_eq!(manifest.value_set_files.len(), 1);
        assert!(
            !Path::new(&manifest.value_set_files[0]).is_absolute(),
            "expected relative path, got {:?}",
            manifest.value_set_files[0]
        );
        assert!(
            manifest.value_set_files[0]
                .replace('\\', "/")
                .contains("package/"),
            "{:?}",
            manifest.value_set_files[0]
        );
    }
}
