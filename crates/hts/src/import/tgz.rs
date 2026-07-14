//! HL7 FHIR NPM package (.tgz) importer.
//!
//! HL7 terminology packages downloaded from <https://terminology.hl7.org/en/downloads.html>
//! follow the FHIR NPM package spec: a gzip-compressed tar archive containing a
//! `package/` directory with one JSON file per resource (CodeSystem, ValueSet,
//! ConceptMap, …).  Resources are **not** pre-wrapped in a FHIR Bundle.

use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};
use tar::Archive;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;

// ── Shared resource collection ────────────────────────────────────────────────

/// Resources extracted from a `.tgz` archive, partitioned by type.
pub(crate) struct CollectedResources {
    pub code_systems: Vec<Value>,
    pub value_sets: Vec<Value>,
    pub concept_maps: Vec<Value>,
    pub parse_errors: Vec<String>,
    /// The `canonical` base declared in the package's `package.json`, e.g.
    /// `http://terminology.hl7.org` for `hl7.terminology`. Drives the
    /// owner-vs-copy classification in [`crate::import::bundle_parser::authority_rank_for`].
    /// `None` when the package omits it (then every resource is authoritative —
    /// fail-open, never worse than today).
    pub package_canonical: Option<String>,
}

/// Open a `.tgz` FHIR NPM package and collect all CodeSystem, ValueSet, and
/// ConceptMap resources into separate lists.
pub(crate) fn collect_tgz_resources(path: &Path) -> Result<CollectedResources, HtsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open {}: {e}", path.display())))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    let mut code_systems: Vec<Value> = Vec::new();
    let mut value_sets: Vec<Value> = Vec::new();
    let mut concept_maps: Vec<Value> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();
    let mut package_canonical: Option<String> = None;

    let entries = archive
        .entries()
        .map_err(|e| HtsError::InvalidRequest(format!("Failed to read tar archive: {e}")))?;

    for entry_result in entries {
        let mut entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                parse_errors.push(format!("Skipping unreadable tar entry: {e}"));
                continue;
            }
        };

        let entry_path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };

        let name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if !name.ends_with(".json") {
            continue;
        }
        if name == ".index.json" {
            continue;
        }

        let mut buf = Vec::new();
        if let Err(e) = entry.read_to_end(&mut buf) {
            parse_errors.push(format!("Cannot read {}: {e}", entry_path.display()));
            continue;
        }

        // `package/package.json` is the package manifest, not a FHIR resource.
        // It carries the `canonical` base that tells us which URLs this package
        // actually owns — the signal that separates an authoritative definition
        // from a re-published copy. Read it, then move on without importing it.
        if name == "package.json" {
            if let Ok(manifest) = serde_json::from_slice::<Value>(&buf)
                && let Some(canonical) = manifest["canonical"].as_str()
                && !canonical.is_empty()
            {
                // Prefer the top-level `package/package.json`; an example or
                // nested manifest must not override it.
                let is_root = entry_path.parent().and_then(|p| p.file_name())
                    == Some(std::ffi::OsStr::new("package"));
                if is_root || package_canonical.is_none() {
                    package_canonical = Some(canonical.trim_end_matches('/').to_owned());
                }
            }
            continue;
        }

        let resource: Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(e) => {
                parse_errors.push(format!("JSON parse error in {}: {e}", entry_path.display()));
                continue;
            }
        };

        match resource["resourceType"].as_str() {
            Some("CodeSystem") => code_systems.push(resource),
            Some("ValueSet") => value_sets.push(resource),
            Some("ConceptMap") => concept_maps.push(resource),
            Some(rt) => {
                tracing::debug!(
                    resource_type = rt,
                    file = %entry_path.display(),
                    "Skipping unsupported resource type"
                );
            }
            None => {}
        }
    }

    Ok(CollectedResources {
        code_systems,
        value_sets,
        concept_maps,
        parse_errors,
        package_canonical,
    })
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Extract and bulk-import a FHIR NPM terminology package through the given
/// backend. Works against either SQLite or PostgreSQL.
pub async fn import_tgz(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "hl7-npm";

    let path_owned = path.to_path_buf();
    let collected = tokio::task::spawn_blocking(move || collect_tgz_resources(&path_owned))
        .await
        .map_err(|e| HtsError::StorageError(format!("Archive reader panicked: {e}")))??;

    tracing::info!(
        code_systems = collected.code_systems.len(),
        value_sets = collected.value_sets.len(),
        concept_maps = collected.concept_maps.len(),
        parse_errors = collected.parse_errors.len(),
        package_canonical = collected.package_canonical.as_deref().unwrap_or("<none>"),
        "Extracted resources from package; starting import"
    );

    let package_canonical = collected.package_canonical.clone();

    let mut total = ImportStats::default();
    total.errors = collected.parse_errors;

    let batch_size = batch_size.max(1);

    for (resources, type_label) in [
        (&collected.code_systems, "CodeSystem"),
        (&collected.value_sets, "ValueSet"),
        (&collected.concept_maps, "ConceptMap"),
    ] {
        if resources.is_empty() {
            continue;
        }
        let num_batches = resources.len().div_ceil(batch_size);

        for (i, chunk) in resources.chunks(batch_size).enumerate() {
            let concepts_before = total.concepts;

            if dry_run {
                let batch = count_batch(chunk);
                total.code_systems += batch.code_systems;
                total.value_sets += batch.value_sets;
                total.concept_maps += batch.concept_maps;
                total.concepts += batch.concepts;
            } else {
                let bundle_bytes = make_bundle_bytes(chunk, package_canonical.as_deref());
                let stats = backend
                    .import_bundle(ctx, &bundle_bytes)
                    .await
                    .map_err(|e| {
                        HtsError::StorageError(format!("{type_label} batch import failed: {e}"))
                    })?;
                total.code_systems += stats.code_systems;
                total.value_sets += stats.value_sets;
                total.concept_maps += stats.concept_maps;
                total.concepts += stats.concepts;
                total.errors.extend(stats.errors);
            }

            let new_concepts = total.concepts - concepts_before;
            eprintln!(
                "[{FORMAT}] {type_label} batch {}/{num_batches} — +{new_concepts} concepts (total: {})",
                i + 1,
                total.concepts,
            );
        }
    }

    Ok(total)
}

// ── Batch helpers ─────────────────────────────────────────────────────────────

/// Count resources in a batch without writing to the database (dry-run).
fn count_batch(resources: &[Value]) -> ImportStats {
    let mut stats = ImportStats::default();
    for r in resources {
        match r["resourceType"].as_str() {
            Some("CodeSystem") => {
                stats.code_systems += 1;
                stats.concepts += r["concept"].as_array().map(|a| a.len() as u32).unwrap_or(0);
            }
            Some("ValueSet") => stats.value_sets += 1,
            Some("ConceptMap") => stats.concept_maps += 1,
            _ => {}
        }
    }
    stats
}

/// Wrap a slice of resource `Value`s in a synthetic FHIR Bundle and serialize to JSON bytes.
fn make_bundle_bytes(resources: &[Value], package_canonical: Option<&str>) -> Vec<u8> {
    let entries: Vec<Value> = resources.iter().map(|r| json!({ "resource": r })).collect();
    let mut bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": entries
    });
    // Transient provenance on the wrapper only — see `SOURCE_CANONICAL_KEY`.
    if let Some(canonical) = package_canonical {
        bundle[crate::import::bundle_parser::SOURCE_CANONICAL_KEY] = json!(canonical);
    }
    serde_json::to_vec(&bundle).expect("in-memory bundle serialization cannot fail")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use tar::Builder;
    use tempfile::NamedTempFile;

    fn sample_code_system() -> Value {
        json!({
            "resourceType": "CodeSystem",
            "id": "test-cs",
            "url": "http://hts.test/cs",
            "version": "1.0",
            "name": "TestCS",
            "status": "active",
            "content": "complete",
            "concept": [
                {"code": "A", "display": "Alpha"},
                {"code": "B", "display": "Beta"},
                {"code": "C", "display": "Gamma"}
            ]
        })
    }

    fn sample_value_set() -> Value {
        json!({
            "resourceType": "ValueSet",
            "id": "test-vs",
            "url": "http://hts.test/vs",
            "version": "1.0",
            "name": "TestVS",
            "status": "active"
        })
    }

    fn make_test_tgz(resources: &[Value]) -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".tgz").unwrap();
        let enc = GzEncoder::new(tmp.reopen().unwrap(), Compression::fast());
        let mut tar = Builder::new(enc);

        for (i, resource) in resources.iter().enumerate() {
            let rt = resource["resourceType"].as_str().unwrap_or("Unknown");
            let path = format!("package/{rt}-{i}.json");
            let bytes = serde_json::to_vec(resource).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, &path, bytes.as_slice())
                .unwrap();
        }
        tar.finish().unwrap();
        tmp
    }

    fn count_rows(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    }

    #[test]
    fn count_batch_counts_resources_and_top_level_concepts() {
        let resources = vec![sample_code_system(), sample_value_set()];
        let stats = count_batch(&resources);
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 0);
        assert_eq!(stats.concepts, 3);
    }

    #[test]
    fn count_batch_empty_slice() {
        let stats = count_batch(&[]);
        assert_eq!(stats.code_systems, 0);
        assert_eq!(stats.concepts, 0);
    }

    #[test]
    fn count_batch_code_system_without_concept_array() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "url": "http://hts.test/empty",
            "content": "not-present"
        });
        let stats = count_batch(&[cs]);
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 0);
    }

    #[tokio::test]
    async fn import_tgz_dry_run_returns_correct_counts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        let stats = import_tgz(&backend, &ctx, tgz.path(), 500, true)
            .await
            .expect("dry-run should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 0);
        assert_eq!(stats.concepts, 3);
    }

    #[tokio::test]
    async fn import_tgz_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        import_tgz(&backend, &ctx, tgz.path(), 500, true)
            .await
            .expect("dry-run should succeed");

        assert_eq!(count_rows(&backend, "code_systems"), 0);
        assert_eq!(count_rows(&backend, "value_sets"), 0);
        assert_eq!(count_rows(&backend, "concepts"), 0);
    }

    #[tokio::test]
    async fn import_tgz_live_writes_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        let stats = import_tgz(&backend, &ctx, tgz.path(), 500, false)
            .await
            .expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concepts, 3);

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "value_sets"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 3);
    }

    #[tokio::test]
    async fn import_tgz_skips_package_metadata_files() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let tmp = NamedTempFile::with_suffix(".tgz").unwrap();
        {
            let enc = GzEncoder::new(tmp.reopen().unwrap(), Compression::fast());
            let mut tar = Builder::new(enc);

            for (name, content) in [
                ("package/package.json", b"{\"name\":\"test\"}" as &[u8]),
                ("package/.index.json", b"{\"index\":[]}" as &[u8]),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, content).unwrap();
            }

            let cs = serde_json::to_vec(&sample_code_system()).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(cs.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "package/CodeSystem-test.json", cs.as_slice())
                .unwrap();
            tar.finish().unwrap();
        }

        let stats = import_tgz(&backend, &ctx, tmp.path(), 500, true)
            .await
            .unwrap();
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.errors.len(), 0);
    }

    /// Build a `.tgz` carrying a `package/package.json` that declares `canonical`,
    /// exactly as a real FHIR NPM package does.
    fn make_test_tgz_with_canonical(
        resources: &[Value],
        name: &str,
        canonical: &str,
    ) -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".tgz").unwrap();
        let enc = GzEncoder::new(tmp.reopen().unwrap(), Compression::fast());
        let mut tar = Builder::new(enc);

        let manifest =
            serde_json::to_vec(&json!({"name": name, "version": "1.0.0", "canonical": canonical}))
                .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "package/package.json", manifest.as_slice())
            .unwrap();

        for (i, resource) in resources.iter().enumerate() {
            let rt = resource["resourceType"].as_str().unwrap_or("Unknown");
            let bytes = serde_json::to_vec(resource).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("package/{rt}-{i}.json"),
                bytes.as_slice(),
            )
            .unwrap();
        }
        tar.finish().unwrap();
        tmp
    }

    #[test]
    fn authority_rank_marks_foreign_canonicals_as_copies() {
        use crate::import::bundle_parser::{
            AUTHORITY_FOREIGN_COPY, AUTHORITY_OWNER, authority_rank_for,
        };

        // The package owns the URL it publishes.
        assert_eq!(
            authority_rank_for(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                Some("http://terminology.hl7.org")
            ),
            AUTHORITY_OWNER
        );
        // hl7.fhir.r4.core re-publishing a THO canonical: a copy.
        assert_eq!(
            authority_rank_for(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                Some("http://hl7.org/fhir")
            ),
            AUTHORITY_FOREIGN_COPY
        );
        // ...but it IS authoritative for its own canonical base.
        assert_eq!(
            authority_rank_for(
                "http://hl7.org/fhir/CodeSystem/example",
                Some("http://hl7.org/fhir")
            ),
            AUTHORITY_OWNER
        );
        // No package (REST write, $import-bundle, native importer): authoritative.
        assert_eq!(
            authority_rank_for("http://terminology.hl7.org/CodeSystem/x", None),
            AUTHORITY_OWNER
        );
        // A package that declares no canonical fails open, never worse than before.
        assert_eq!(
            authority_rank_for("http://anything/at/all", Some("")),
            AUTHORITY_OWNER
        );
    }

    /// Regression test for issue #200.
    ///
    /// `hl7.fhir.r4.core` re-ships THO CodeSystems stamped with the *FHIR release*
    /// version (`4.0.1`) and truncated to a single concept, while the THO package
    /// publishes the complete definition as version `1.0.0`. Both rows are
    /// `content: complete` and both carry concepts, so no intrinsic property of
    /// the rows separates them — and `4.0.1` outsorts `1.0.0` under any version
    /// ordering, lexicographic or semver. Only provenance can decide, and the
    /// bare (no-version) call must land on the complete THO row.
    ///
    /// Imports the *core-like* package first, mirroring the real bootstrap order
    /// (the directory is walked alphabetically, so `hl7.fhir.r4.core-*.tgz` lands
    /// before `hl7.terminology-*.tgz`) — the outcome must not depend on it.
    #[tokio::test]
    async fn bare_validate_code_prefers_owning_package_over_republished_copy() {
        use crate::traits::CodeSystemOperations;
        use crate::types::ValidateCodeRequest;

        const URL: &str = "http://terminology.hl7.org/CodeSystem/audit-event-type";

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let core_copy = json!({
            "resourceType": "CodeSystem", "id": "audit-event-type", "url": URL,
            "version": "4.0.1", "name": "AuditEventID", "status": "active",
            "content": "complete",
            "concept": [{"code": "rest", "display": "RESTful Operation"}]
        });
        let tho_original = json!({
            "resourceType": "CodeSystem", "id": "audit-event-type", "url": URL,
            "version": "1.0.0", "name": "AuditEventID", "status": "active",
            "content": "complete",
            "concept": [
                {"code": "rest", "display": "RESTful Operation"},
                {"code": "hl7-v2", "display": "HL7v2 Message"},
                {"code": "hl7-v3", "display": "HL7v3 Message"},
                {"code": "document", "display": "Document"},
                {"code": "object", "display": "Object"}
            ]
        });

        let core = make_test_tgz_with_canonical(
            std::slice::from_ref(&core_copy),
            "hl7.fhir.r4.core",
            "http://hl7.org/fhir",
        );
        let tho = make_test_tgz_with_canonical(
            std::slice::from_ref(&tho_original),
            "hl7.terminology",
            "http://terminology.hl7.org",
        );

        import_tgz(&backend, &ctx, core.path(), 500, false)
            .await
            .expect("core package import should succeed");
        import_tgz(&backend, &ctx, tho.path(), 500, false)
            .await
            .expect("THO package import should succeed");

        // Both versions coexist — the copy is ranked, not discarded, so an
        // explicit `version=4.0.1` request can still reach it.
        assert_eq!(count_rows(&backend, "code_systems"), 2);

        // The bug: `object` exists only in the complete THO 1.0.0 definition.
        let resp = CodeSystemOperations::validate_code(
            &backend,
            &ctx,
            ValidateCodeRequest {
                system: Some(URL.into()),
                code: "object".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(
            resp.result,
            "bare $validate-code must resolve to the complete THO definition, \
             not the truncated hl7.fhir.r4.core copy; got: {:?}",
            resp.message
        );
        assert_eq!(resp.cs_version.as_deref(), Some("1.0.0"));

        // A genuine miss must name the version it ACTUALLY validated against.
        // The diagnostic used to re-query by URL with its own ordering, so it
        // reported whichever version string sorted highest ("4.0.1") even though
        // validation had run against a different row.
        let miss = CodeSystemOperations::validate_code(
            &backend,
            &ctx,
            ValidateCodeRequest {
                system: Some(URL.into()),
                code: "no-such-code".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!miss.result);
        let msg = miss.message.unwrap_or_default();
        assert!(
            msg.contains("1.0.0") && !msg.contains("4.0.1"),
            "a miss must report the resolved version (1.0.0), not the highest \
             version string on the URL (4.0.1); got: {msg}"
        );

        // An explicit pin to the copy still resolves to the copy, and correctly
        // reports that `object` is not in it. The copy is ranked, not hidden.
        let pinned = CodeSystemOperations::validate_code(
            &backend,
            &ctx,
            ValidateCodeRequest {
                system: Some(URL.into()),
                code: "object".into(),
                version: Some("4.0.1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!pinned.result, "version=4.0.1 pins the truncated copy");
        assert!(
            pinned.message.unwrap_or_default().contains("4.0.1"),
            "a miss must report the version it actually validated against"
        );
    }

    /// Import order must not decide the winner. The real bootstrap walks the
    /// terminology directory alphabetically, which is what let the copy win in
    /// the first place; reversing the order must change nothing.
    #[tokio::test]
    async fn owning_package_wins_regardless_of_import_order() {
        use crate::traits::CodeSystemOperations;
        use crate::types::ValidateCodeRequest;

        const URL: &str = "http://terminology.hl7.org/CodeSystem/audit-event-type";

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let core_copy = json!({
            "resourceType": "CodeSystem", "id": "audit-event-type", "url": URL,
            "version": "4.0.1", "status": "active", "content": "complete",
            "concept": [{"code": "rest"}]
        });
        let tho_original = json!({
            "resourceType": "CodeSystem", "id": "audit-event-type", "url": URL,
            "version": "1.0.0", "status": "active", "content": "complete",
            "concept": [{"code": "rest"}, {"code": "object"}]
        });

        let core = make_test_tgz_with_canonical(
            std::slice::from_ref(&core_copy),
            "hl7.fhir.r4.core",
            "http://hl7.org/fhir",
        );
        let tho = make_test_tgz_with_canonical(
            std::slice::from_ref(&tho_original),
            "hl7.terminology",
            "http://terminology.hl7.org",
        );

        // THO first, core second — the reverse of the alphabetical bootstrap order.
        import_tgz(&backend, &ctx, tho.path(), 500, false)
            .await
            .unwrap();
        import_tgz(&backend, &ctx, core.path(), 500, false)
            .await
            .unwrap();

        let resp = CodeSystemOperations::validate_code(
            &backend,
            &ctx,
            ValidateCodeRequest {
                system: Some(URL.into()),
                code: "object".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            resp.result,
            "the owning package must win no matter which package was imported last"
        );
    }

    #[tokio::test]
    async fn import_tgz_invalid_path_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let result = import_tgz(
            &backend,
            &ctx,
            Path::new("/nonexistent/path.tgz"),
            500,
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_tgz_batching_preserves_all_resources() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let resources: Vec<Value> = (0..5)
            .map(|i| {
                json!({
                    "resourceType": "CodeSystem",
                    "id": format!("cs-{i}"),
                    "url": format!("http://hts.test/cs-{i}"),
                    "version": "1.0",
                    "name": format!("CS{i}"),
                    "status": "active",
                    "content": "complete",
                    "concept": [{"code": format!("C{i}"), "display": format!("Concept {i}")}]
                })
            })
            .collect();

        let tgz = make_test_tgz(&resources);
        let stats = import_tgz(&backend, &ctx, tgz.path(), 2, false)
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 5);
        assert_eq!(stats.concepts, 5);
        assert_eq!(count_rows(&backend, "code_systems"), 5);
    }
}
