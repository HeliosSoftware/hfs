//! FHIR NPM package cache, dependency resolution, and materialization.

use flate2::Compression;
use flate2::write::GzEncoder;
use helios_fhir::FhirVersion;
use helios_fhir_validator::{
    CompositeResolver, PackageCache, PackageRef, SchemaResolver, ValidationOptions, Validator,
    materialize_package, materialize_package_layers, packs, resolve_packages,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::{Builder, Header};

fn write_tgz(path: &Path, files: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let enc = GzEncoder::new(file, Compression::default());
    let mut archive = Builder::new(enc);
    for (name, data) in files {
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, *name, *data).unwrap();
    }
    let enc = archive.into_inner().unwrap();
    enc.finish().unwrap();
}

fn minimal_patient_profile_sd() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/fhir/StructureDefinition/example-patient",
        "name": "ExamplePatient",
        "status": "active",
        "kind": "resource",
        "abstract": false,
        "type": "Patient",
        "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
        "derivation": "constraint",
        "differential": {
            "element": [
                {
                    "id": "Patient",
                    "path": "Patient",
                    "min": 0,
                    "max": "*"
                },
                {
                    "id": "Patient.active",
                    "path": "Patient.active",
                    "min": 1,
                    "max": "1"
                }
            ]
        }
    }))
    .unwrap()
}

fn dep_extension_sd() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "resourceType": "StructureDefinition",
        "url": "http://example.org/fhir/StructureDefinition/example-ext",
        "name": "ExampleExt",
        "status": "active",
        "kind": "complex-type",
        "abstract": false,
        "type": "Extension",
        "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Extension",
        "derivation": "constraint",
        "differential": {
            "element": [
                { "id": "Extension", "path": "Extension", "min": 0, "max": "*" },
                {
                    "id": "Extension.url",
                    "path": "Extension.url",
                    "fixedUri": "http://example.org/fhir/StructureDefinition/example-ext"
                }
            ]
        }
    }))
    .unwrap()
}

fn seed_dep_and_root(cache_root: &Path) -> PathBuf {
    let tgz_dir = cache_root.join("_tgz");
    fs::create_dir_all(&tgz_dir).unwrap();

    let dep_tgz = tgz_dir.join("dep.tgz");
    write_tgz(
        &dep_tgz,
        &[
            (
                "package/package.json",
                br#"{
                  "name": "example.dep",
                  "version": "1.0.0",
                  "fhirVersions": ["4.0.1"],
                  "dependencies": {}
                }"#,
            ),
            (
                "package/StructureDefinition-example-ext.json",
                &dep_extension_sd(),
            ),
        ],
    );

    let root_tgz = tgz_dir.join("root.tgz");
    write_tgz(
        &root_tgz,
        &[
            (
                "package/package.json",
                br#"{
                  "name": "example.ig",
                  "version": "1.0.0",
                  "fhirVersions": ["4.0.1"],
                  "dependencies": { "example.dep": "1.0.0" }
                }"#,
            ),
            (
                "package/StructureDefinition-example-patient.json",
                &minimal_patient_profile_sd(),
            ),
            // Poison abstract root — must be skipped, not abort materialize.
            (
                "package/StructureDefinition-Element.json",
                br#"{
                  "resourceType": "StructureDefinition",
                  "url": "http://hl7.org/fhir/StructureDefinition/Element",
                  "name": "Element",
                  "status": "active",
                  "kind": "complex-type",
                  "abstract": true,
                  "type": "Element",
                  "differential": { "element": [] }
                }"#,
            ),
        ],
    );

    let cache = PackageCache::new(cache_root);
    cache.ensure_from_tgz(&dep_tgz).unwrap();
    cache.ensure_from_tgz(&root_tgz).unwrap();
    tgz_dir
}

#[test]
fn cache_ensure_from_tgz_and_get() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    let tgz = tmp.path().join("pkg.tgz");
    write_tgz(
        &tgz,
        &[
            (
                "package/package.json",
                br#"{"name":"solo.pkg","version":"0.1.0","dependencies":{}}"#,
            ),
            (
                "package/StructureDefinition-example-patient.json",
                &minimal_patient_profile_sd(),
            ),
        ],
    );
    let id = cache.ensure_from_tgz(&tgz).unwrap();
    assert_eq!(id.to_string(), "solo.pkg@0.1.0");
    let dir = cache.get(&id).unwrap();
    assert!(dir.join("package.json").is_file());
    assert!(dir.join(".sha256").is_file());
}

#[test]
fn resolve_requires_deps_in_cache() {
    let tmp = tempfile::tempdir().unwrap();
    seed_dep_and_root(tmp.path());
    let cache = PackageCache::new(tmp.path());

    let missing = resolve_packages(
        &cache,
        &[PackageRef::parse("example.ig@1.0.0").unwrap()],
    );
    // dep was seeded — should succeed with dep then root
    let ok = missing.unwrap();
    assert_eq!(ok.len(), 2);
    assert_eq!(ok[0].id.to_string(), "example.dep@1.0.0");
    assert_eq!(ok[1].id.to_string(), "example.ig@1.0.0");

    // Fresh cache with only root → missing dep
    let tmp2 = tempfile::tempdir().unwrap();
    let cache2 = PackageCache::new(tmp2.path());
    let tgz = tmp2.path().join("root.tgz");
    write_tgz(
        &tgz,
        &[(
            "package/package.json",
            br#"{
              "name": "example.ig",
              "version": "1.0.0",
              "dependencies": { "example.dep": "1.0.0" }
            }"#,
        )],
    );
    cache2.ensure_from_tgz(&tgz).unwrap();
    let err = resolve_packages(
        &cache2,
        &[PackageRef::parse("example.ig@1.0.0").unwrap()],
    )
    .unwrap_err();
    assert!(err.to_string().contains("example.dep@1.0.0"), "{err}");
}

#[test]
fn materialize_skips_abstract_and_inserts_profile() {
    let tmp = tempfile::tempdir().unwrap();
    seed_dep_and_root(tmp.path());
    let cache = PackageCache::new(tmp.path());
    let dir = cache
        .get(&PackageRef::parse("example.ig@1.0.0").unwrap())
        .unwrap();
    let (registry, report) = materialize_package(&dir).unwrap();
    assert!(report.skipped_abstract >= 1);
    assert!(report.inserted >= 1);
    assert!(
        registry
            .resolve("http://example.org/fhir/StructureDefinition/example-patient")
            .is_some()
    );
}

#[test]
fn package_layers_overlay_core_for_meta_profile() {
    let tmp = tempfile::tempdir().unwrap();
    seed_dep_and_root(tmp.path());
    let cache = PackageCache::new(tmp.path());
    let layers = materialize_package_layers(
        &cache,
        &[PackageRef::parse("example.ig@1.0.0").unwrap()],
    )
    .unwrap();
    // Overlay order: root then dep (dependents first).
    assert_eq!(layers[0].0.to_string(), "example.ig@1.0.0");
    assert_eq!(layers[1].0.to_string(), "example.dep@1.0.0");

    let core = packs::core_registry(FhirVersion::R4);
    let mut resolvers: Vec<Arc<dyn helios_fhir_validator::SchemaResolver>> = layers
        .into_iter()
        .map(|(_, reg, _)| reg as Arc<dyn helios_fhir_validator::SchemaResolver>)
        .collect();
    resolvers.push(core);
    let resolver = Arc::new(CompositeResolver::new(resolvers));
    let validator = Validator::new(resolver);

    // Missing required Patient.active from the example profile.
    let resource = json!({
        "resourceType": "Patient",
        "meta": {
            "profile": ["http://example.org/fhir/StructureDefinition/example-patient"]
        }
    });
    let outcome = validator.validate_sync(&resource, &ValidationOptions::default());
    assert!(
        outcome.errors.iter().any(|e| e.message.contains("active")
            || e.path.contains("active")
            || e.message.to_lowercase().contains("required")),
        "expected required active issue, got {:?}",
        outcome.errors
    );
}
