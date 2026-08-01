use crate::packages::error::PackageError;
use crate::packages::manifest::{PackageId, PackageManifest};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tar::Archive;

/// Curated on-disk FHIR NPM package cache.
///
/// Layout: `{root}/{name}/{version}/` with `package.json` at that root.
/// Resolution is offline — [`Self::get`] fails if the package is absent.
#[derive(Debug, Clone)]
pub struct PackageCache {
    root: PathBuf,
}

impl PackageCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory for `id`, whether or not it exists yet.
    pub fn package_dir(&self, id: &PackageId) -> PathBuf {
        self.root.join(&id.name).join(&id.version)
    }

    /// Return the expanded package directory if present and valid.
    pub fn get(&self, id: &PackageId) -> Result<PathBuf, PackageError> {
        let dir = self.package_dir(id);
        let manifest_path = dir.join("package.json");
        if !manifest_path.is_file() {
            return Err(PackageError::NotInCache {
                name: id.name.clone(),
                version: id.version.clone(),
            });
        }
        let manifest = PackageManifest::load(&manifest_path)?;
        if manifest.name != id.name || manifest.version != id.version {
            return Err(PackageError::Manifest(format!(
                "cache entry {} has package.json id {}@{} (expected {id})",
                dir.display(),
                manifest.name,
                manifest.version
            )));
        }
        Ok(dir)
    }

    /// Extract a FHIR NPM `.tgz` into the cache. Returns the package id from
    /// `package.json`. Writes a `.sha256` sidecar of the source archive.
    pub fn ensure_from_tgz(&self, tgz: &Path) -> Result<PackageId, PackageError> {
        if !tgz.is_file() {
            return Err(PackageError::Invalid(format!(
                "tarball not found: {}",
                tgz.display()
            )));
        }

        let sha = hash_file(tgz)?;
        let staging = self.root.join(".staging").join(&sha);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|e| PackageError::io(&staging, e))?;
        }
        fs::create_dir_all(&staging).map_err(|e| PackageError::io(&staging, e))?;

        extract_tgz(tgz, &staging)?;
        let package_root = find_package_root(&staging)?;
        let manifest = PackageManifest::load(&package_root.join("package.json"))?;
        let id = manifest.id();
        let dest = self.package_dir(&id);

        if dest.exists() {
            fs::remove_dir_all(&dest).map_err(|e| PackageError::io(&dest, e))?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| PackageError::io(parent, e))?;
        }
        // Move contents of package_root into dest.
        fs::rename(&package_root, &dest).or_else(|_| {
            copy_dir_all(&package_root, &dest)?;
            fs::remove_dir_all(&package_root).map_err(|e| PackageError::io(&package_root, e))
        })?;

        let _ = fs::remove_dir_all(self.root.join(".staging"));

        let sidecar = dest.join(".sha256");
        fs::write(&sidecar, format!("{sha}\n")).map_err(|e| PackageError::io(&sidecar, e))?;

        Ok(id)
    }

    /// Copy an already-expanded package directory (must contain `package.json`)
    /// into the cache.
    pub fn ensure_from_dir(&self, dir: &Path) -> Result<PackageId, PackageError> {
        let package_root = find_package_root(dir)?;
        let manifest = PackageManifest::load(&package_root.join("package.json"))?;
        let id = manifest.id();
        let dest = self.package_dir(&id);
        if dest.exists() {
            fs::remove_dir_all(&dest).map_err(|e| PackageError::io(&dest, e))?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| PackageError::io(parent, e))?;
        }
        copy_dir_all(&package_root, &dest)?;
        Ok(id)
    }
}

fn hash_file(path: &Path) -> Result<String, PackageError> {
    let mut file = File::open(path).map_err(|e| PackageError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|e| PackageError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract_tgz(tgz: &Path, dest: &Path) -> Result<(), PackageError> {
    let file = File::open(tgz).map_err(|e| PackageError::io(tgz, e))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| PackageError::Invalid(format!("failed to extract {}: {e}", tgz.display())))?;
    Ok(())
}

/// Prefer `dir/package/package.json` (NPM layout), else `dir/package.json`.
fn find_package_root(dir: &Path) -> Result<PathBuf, PackageError> {
    let nested = dir.join("package");
    if nested.join("package.json").is_file() {
        return Ok(nested);
    }
    if dir.join("package.json").is_file() {
        return Ok(dir.to_path_buf());
    }
    // After unpack, the archive may have a single top-level folder.
    if dir.is_dir() {
        let mut children = fs::read_dir(dir)
            .map_err(|e| PackageError::io(dir, e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        if children.len() == 1 {
            let child = children.pop().expect("one child");
            if child.join("package.json").is_file() {
                return Ok(child);
            }
            if child.join("package").join("package.json").is_file() {
                return Ok(child.join("package"));
            }
        }
    }
    Err(PackageError::Manifest(format!(
        "no package.json under {}",
        dir.display()
    )))
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), PackageError> {
    fs::create_dir_all(dst).map_err(|e| PackageError::io(dst, e))?;
    for entry in fs::read_dir(src).map_err(|e| PackageError::io(src, e))? {
        let entry = entry.map_err(|e| PackageError::io(src, e))?;
        let ty = entry.file_type().map_err(|e| PackageError::io(src, e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| PackageError::io(&from, e))?;
        }
    }
    Ok(())
}

/// Used by tests / materialize helpers when writing small files.
#[allow(dead_code)]
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), PackageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| PackageError::io(parent, e))?;
    }
    let mut f = File::create(path).map_err(|e| PackageError::io(path, e))?;
    f.write_all(bytes).map_err(|e| PackageError::io(path, e))?;
    Ok(())
}
