//! FHIR NPM / IG package materialization.
//!
//! #232 already makes package overlays the native validation shape: convert
//! StructureDefinitions to FHIR Schemas and push a [`SchemaRegistry`] layer
//! over the embedded core pack. This module is **materialization proper** —
//! loading full IG/NPM packages from a curated on-disk cache, resolving
//! `package.json` dependencies against that cache only, and producing
//! registry layers for [`CompositeResolver`](crate::CompositeResolver).
//!
//! ## Cache layout
//!
//! ```text
//! {cache}/{package-name}/{version}/
//!   package.json
//!   StructureDefinition-….json
//!   …
//!   .sha256          # optional integrity of the source .tgz
//! ```
//!
//! Packages are expanded with the FHIR NPM `package/` prefix stripped so
//! `package.json` sits at the version directory root.
//!
//! Populate the cache from **any local source** via
//! [`PackageCache::ensure_from_path`] (`.tgz`, expanded dir, or IG publisher
//! `output/` which selects `package.tgz`). HTTP(S) seeding is handled by the
//! HFS REST layer (`HFS_FHIR_PACKAGE_SOURCES`) so this crate stays
//! filesystem-only at validate time. The cache's `.staging/` directory is
//! only a temporary unpack workspace — not a package source.
//!
//! ## Abstract StructureDefinitions
//!
//! `Element`, `BackboneElement`, `Resource`, and `DomainResource` are skipped
//! during materialization. They are FHIR infrastructure roots, not useful
//! profile targets, and including them can abort converters that require
//! `derivation` / `baseDefinition`.
//!
//! ## Terminology
//!
//! CodeSystem / ValueSet resources found in a package are discovered but
//! **not** loaded into the schema registry — import those via HTS.

mod cache;
mod error;
mod manifest;
mod materialize;
mod resolve;
mod scan;
mod version;

pub use cache::PackageCache;
pub use error::PackageError;
pub use manifest::{PackageId, PackageManifest, PackageRef};
pub use materialize::{MaterializeReport, materialize_package};
pub use resolve::{ResolvedPackage, resolve_packages};
pub use scan::{ScannedPackage, scan_package_dir};
pub use version::manifest_supports_fhir_version;

/// Install `path` into `cache` then return the package id.
///
/// See [`PackageCache::ensure_from_path`] for accepted layouts (`.tgz`,
/// expanded package dir, IG publisher `output/`).
pub fn ensure_package_path(cache: &PackageCache, path: &Path) -> Result<PackageId, PackageError> {
    cache.ensure_from_path(path)
}

use crate::resolver::SchemaRegistry;
use helios_fhir::FhirVersion;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Resolve roots against `cache`, materialize each package into a schema
/// registry, and return layers in **CompositeResolver order** (earlier wins):
/// configured roots first (config order), then transitive dependencies
/// (dependents before deeper deps), so a root IG overrides dependency
/// profiles with the same canonical URL.
///
/// When `fhir_version` is `Some`, packages whose `fhirVersions` are
/// incompatible with that release are rejected ([`PackageError::Resolve`]).
pub fn materialize_package_layers(
    cache: &PackageCache,
    roots: &[PackageRef],
    fhir_version: Option<FhirVersion>,
) -> Result<Vec<(PackageRef, Arc<SchemaRegistry>, MaterializeReport)>, PackageError> {
    let resolved = resolve_packages(cache, roots, fhir_version)?;
    // resolve_packages returns deps-first topo order; reverse for overlay
    // precedence (dependents / roots win over dependencies).
    let mut layers = Vec::with_capacity(resolved.len());
    for pkg in resolved.into_iter().rev() {
        let (registry, report) = materialize_package(&pkg.path)?;
        layers.push((pkg.id, Arc::new(registry), report));
    }
    Ok(layers)
}

/// Materialize package layers once, then partition them by every version in
/// `versions`. Packages with an empty `fhirVersions` list appear under every
/// requested version; otherwise only matching versions receive the layer.
///
/// Root packages that do not support `default_version` fail with
/// [`PackageError::Resolve`] so a misconfigured IG cannot boot silently.
pub fn materialize_package_layers_by_version(
    cache: &PackageCache,
    roots: &[PackageRef],
    default_version: FhirVersion,
    versions: &[FhirVersion],
) -> Result<HashMap<FhirVersion, Vec<Arc<SchemaRegistry>>>, PackageError> {
    // Resolve without a version filter so multi-version servers can keep
    // packages that only apply to a subset of enabled releases.
    let resolved = resolve_packages(cache, roots, None)?;

    for root in roots {
        let Some(pkg) = resolved.iter().find(|p| p.id == *root) else {
            continue;
        };
        if !manifest_supports_fhir_version(&pkg.manifest.fhir_versions, default_version) {
            return Err(PackageError::Resolve(format!(
                "package {} declares fhirVersions {:?} incompatible with default FHIR version {}",
                pkg.id,
                pkg.manifest.fhir_versions,
                default_version.full_version()
            )));
        }
    }

    // Materialize in dependents-first overlay order once.
    let mut materialized: Vec<(PackageRef, Arc<SchemaRegistry>, PackageManifest)> =
        Vec::with_capacity(resolved.len());
    for pkg in resolved.into_iter().rev() {
        let (registry, _report) = materialize_package(&pkg.path)?;
        materialized.push((pkg.id, Arc::new(registry), pkg.manifest));
    }

    let mut out: HashMap<FhirVersion, Vec<Arc<SchemaRegistry>>> = HashMap::new();
    for &version in versions {
        let mut layers = Vec::new();
        for (_id, registry, manifest) in &materialized {
            if manifest_supports_fhir_version(&manifest.fhir_versions, version) {
                layers.push(Arc::clone(registry));
            }
        }
        out.insert(version, layers);
    }
    Ok(out)
}

/// Convenience: ensure a `.tgz` is in the cache, then materialize that single
/// package (no dependency walk).
pub fn materialize_tgz(
    cache: &PackageCache,
    tgz: &Path,
) -> Result<(PackageRef, SchemaRegistry, MaterializeReport), PackageError> {
    let id = cache.ensure_from_tgz(tgz)?;
    let path = cache.get(&id)?;
    let (registry, report) = materialize_package(&path)?;
    Ok((id, registry, report))
}
