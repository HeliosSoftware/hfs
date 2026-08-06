use crate::packages::cache::PackageCache;
use crate::packages::error::PackageError;
use crate::packages::manifest::{PackageId, PackageManifest, PackageRef};
use crate::packages::version::manifest_supports_fhir_version;
use helios_fhir::FhirVersion;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

/// A package located in the cache, ready to materialize.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub id: PackageId,
    pub path: PathBuf,
    pub manifest: PackageManifest,
}

/// Resolve `roots` and their transitive `package.json` dependencies from
/// `cache` only (offline). Returns packages in **deps-first** topological
/// order (dependencies before dependents). Callers that need overlay
/// precedence (dependents win) should reverse this list.
///
/// When `fhir_version` is `Some`, every resolved package must declare a
/// compatible `fhirVersions` entry (or omit the field). Incompatible packages
/// yield [`PackageError::Resolve`].
pub fn resolve_packages(
    cache: &PackageCache,
    roots: &[PackageRef],
    fhir_version: Option<FhirVersion>,
) -> Result<Vec<ResolvedPackage>, PackageError> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    // Collect closure.
    let mut manifests: BTreeMap<PackageId, (PathBuf, PackageManifest)> = BTreeMap::new();
    let mut pending: Vec<PackageId> = roots.to_vec();
    while let Some(id) = pending.pop() {
        if manifests.contains_key(&id) {
            continue;
        }
        let path = cache.get(&id)?;
        let manifest = PackageManifest::load(&path.join("package.json"))?;
        if let Some(version) = fhir_version
            && !manifest_supports_fhir_version(&manifest.fhir_versions, version)
        {
            return Err(PackageError::Resolve(format!(
                "package {id} declares fhirVersions {:?} incompatible with {}",
                manifest.fhir_versions,
                version.full_version()
            )));
        }
        for dep in manifest.dependency_ids() {
            if !manifests.contains_key(&dep) {
                pending.push(dep);
            }
        }
        manifests.insert(id, (path, manifest));
    }

    // Edges: dependency -> dependent (dep must come first).
    let mut indegree: BTreeMap<PackageId, usize> =
        manifests.keys().cloned().map(|id| (id, 0)).collect();
    let mut outgoing: BTreeMap<PackageId, Vec<PackageId>> = BTreeMap::new();

    for (id, (_, manifest)) in &manifests {
        for dep in manifest.dependency_ids() {
            if !manifests.contains_key(&dep) {
                return Err(PackageError::NotInCache {
                    name: dep.name,
                    version: dep.version,
                });
            }
            *indegree.get_mut(id).expect("id present") += 1;
            outgoing.entry(dep).or_default().push(id.clone());
        }
    }

    let mut queue: VecDeque<PackageId> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();

    let mut ordered = Vec::with_capacity(manifests.len());
    let mut seen: BTreeSet<PackageId> = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        ordered.push(id.clone());
        if let Some(dependents) = outgoing.get(&id) {
            for dep_of in dependents {
                let d = indegree.get_mut(dep_of).expect("present");
                *d = d.saturating_sub(1);
                if *d == 0 {
                    queue.push_back(dep_of.clone());
                }
            }
        }
    }

    if ordered.len() != manifests.len() {
        return Err(PackageError::Resolve(
            "dependency cycle detected among cached packages".into(),
        ));
    }

    Ok(ordered
        .into_iter()
        .map(|id| {
            let (path, manifest) = manifests.remove(&id).expect("id in manifests");
            ResolvedPackage { id, path, manifest }
        })
        .collect())
}
