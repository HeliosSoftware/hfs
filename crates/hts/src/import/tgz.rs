//! HL7 FHIR NPM package (.tgz) importer.
//!
//! HL7 terminology packages downloaded from <https://terminology.hl7.org/en/downloads.html>
//! follow the FHIR NPM package spec: a gzip-compressed tar archive containing a
//! `package/` directory with one JSON file per resource (CodeSystem, ValueSet,
//! ConceptMap, …).  Resources are **not** pre-wrapped in a FHIR Bundle.
//!
//! This module extracts those resources, groups them by type to preserve the
//! required import order (CodeSystems → ValueSets → ConceptMaps), and feeds
//! them in batches through the existing [`import_bundle_sync`] pipeline.
//!
//! [`import_bundle_sync`]: super::fhir_bundle::import_bundle_sync

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use serde_json::{Value, json};
use tar::Archive;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Shared resource collection ────────────────────────────────────────────────

/// Resources extracted from a `.tgz` archive, partitioned by type.
///
/// Backend-agnostic: consumed by both the SQLite and PostgreSQL import paths.
pub(crate) struct CollectedResources {
    pub code_systems: Vec<Value>,
    pub value_sets: Vec<Value>,
    pub concept_maps: Vec<Value>,
    /// Non-fatal errors encountered while reading or parsing archive entries.
    pub parse_errors: Vec<String>,
}

/// Open a `.tgz` FHIR NPM package and collect all CodeSystem, ValueSet, and
/// ConceptMap resources into separate lists.  Metadata files (`package.json`,
/// `.index.json`) are silently skipped.  Unreadable or malformed entries are
/// recorded in `parse_errors` and skipped.
pub(crate) fn collect_tgz_resources(path: &Path) -> Result<CollectedResources, HtsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open {}: {e}", path.display())))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    let mut code_systems: Vec<Value> = Vec::new();
    let mut value_sets: Vec<Value> = Vec::new();
    let mut concept_maps: Vec<Value> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();

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
        if name == "package.json" || name == ".index.json" {
            continue;
        }

        let mut buf = Vec::new();
        if let Err(e) = entry.read_to_end(&mut buf) {
            parse_errors.push(format!("Cannot read {}: {e}", entry_path.display()));
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
    })
}

// ── Public entry points ───────────────────────────────────────────────────────

/// Extract and bulk-import a FHIR NPM terminology package (SQLite backend).
///
/// Opens the `.tgz` at `path`, collects all `CodeSystem`, `ValueSet`, and
/// `ConceptMap` resources, then imports them in dependency order using the
/// existing `import_bundle_sync` pipeline.  Resources are processed in
/// batches of `batch_size` to bound peak memory.
///
/// Progress is written to **stderr** in the format:
/// `[hl7-npm] CodeSystem batch 1/3 — +47 concepts (total: 94)`
///
/// When `dry_run` is `true` the archive is fully parsed and counted but no
/// data is written to the database.  Concept counts in dry-run mode reflect
/// top-level `concept` array entries only (nested concepts are not traversed).
///
/// Returns cumulative [`ImportStats`] across all batches.
#[cfg(feature = "sqlite")]
pub fn import_tgz(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    use super::fhir_bundle::import_bundle_sync;

    const FORMAT: &str = "hl7-npm";

    let collected = collect_tgz_resources(path)?;

    tracing::info!(
        code_systems = collected.code_systems.len(),
        value_sets = collected.value_sets.len(),
        concept_maps = collected.concept_maps.len(),
        parse_errors = collected.parse_errors.len(),
        "Extracted resources from package; starting import"
    );

    let mut total = ImportStats::default();
    total.errors = collected.parse_errors;

    let batch_size = batch_size.max(1);

    import_type_batches(
        pool,
        &collected.code_systems,
        "CodeSystem",
        batch_size,
        dry_run,
        FORMAT,
        &mut total,
        import_bundle_sync,
    )?;
    import_type_batches(
        pool,
        &collected.value_sets,
        "ValueSet",
        batch_size,
        dry_run,
        FORMAT,
        &mut total,
        import_bundle_sync,
    )?;
    import_type_batches(
        pool,
        &collected.concept_maps,
        "ConceptMap",
        batch_size,
        dry_run,
        FORMAT,
        &mut total,
        import_bundle_sync,
    )?;

    Ok(total)
}

/// Extract and bulk-import a FHIR NPM terminology package (PostgreSQL backend).
///
/// Async counterpart to [`import_tgz`].  Reads the archive with
/// [`collect_tgz_resources`] (blocking, run via `spawn_blocking`), then
/// imports each batch by calling [`BundleImportBackend::import_bundle`] on
/// the provided backend.
///
/// When `dry_run` is `true` the archive is parsed and counted but no data is
/// written to the database.
#[cfg(feature = "postgres")]
pub async fn import_tgz_pg(
    backend: &crate::backend::postgres::PostgresTerminologyBackend,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    use helios_persistence::tenant::TenantContext;

    const FORMAT: &str = "hl7-npm";

    // Archive I/O is blocking — run off the async executor.
    let path_owned = path.to_path_buf();
    let collected = tokio::task::spawn_blocking(move || collect_tgz_resources(&path_owned))
        .await
        .map_err(|e| HtsError::StorageError(format!("Archive reader panicked: {e}")))??;

    tracing::info!(
        code_systems = collected.code_systems.len(),
        value_sets = collected.value_sets.len(),
        concept_maps = collected.concept_maps.len(),
        parse_errors = collected.parse_errors.len(),
        "Extracted resources from package; starting PostgreSQL import"
    );

    let mut total = ImportStats::default();
    total.errors = collected.parse_errors;

    let batch_size = batch_size.max(1);

    let ctx = TenantContext::system();

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
                use crate::import::BundleImportBackend;
                let bundle_bytes = make_bundle_bytes(chunk);
                let stats = backend
                    .import_bundle(&ctx, &bundle_bytes)
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

/// Send `resources` through `import_fn` in slices of at most `batch_size`,
/// emitting a progress line to stderr after each batch.
///
/// When `dry_run` is `true` the import function is **not** called; resources
/// are counted locally instead.
#[cfg(feature = "sqlite")]
#[allow(clippy::too_many_arguments)]
fn import_type_batches(
    pool: &Pool<SqliteConnectionManager>,
    resources: &[Value],
    type_label: &str,
    batch_size: usize,
    dry_run: bool,
    format_label: &str,
    total: &mut ImportStats,
    import_fn: fn(&Pool<SqliteConnectionManager>, &[u8]) -> Result<ImportStats, HtsError>,
) -> Result<(), HtsError> {
    if resources.is_empty() {
        return Ok(());
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
            let bundle_bytes = make_bundle_bytes(chunk);
            match import_fn(pool, &bundle_bytes) {
                Ok(stats) => {
                    total.code_systems += stats.code_systems;
                    total.value_sets += stats.value_sets;
                    total.concept_maps += stats.concept_maps;
                    total.concepts += stats.concepts;
                    total.errors.extend(stats.errors);
                }
                Err(e) => return Err(e),
            }
        }

        let new_concepts = total.concepts - concepts_before;
        eprintln!(
            "[{format_label}] {type_label} batch {}/{num_batches} — +{new_concepts} concepts (total: {})",
            i + 1,
            total.concepts,
        );
    }

    Ok(())
}

/// Count resources in a batch without writing to the database (dry-run).
///
/// Concept count reflects top-level `concept` array entries in CodeSystems only;
/// nested concepts are not traversed.
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
fn make_bundle_bytes(resources: &[Value]) -> Vec<u8> {
    let entries: Vec<Value> = resources.iter().map(|r| json!({ "resource": r })).collect();
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": entries
    });
    serde_json::to_vec(&bundle).expect("in-memory bundle serialization cannot fail")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backend::SqliteTerminologyBackend;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use tar::Builder;
    use tempfile::NamedTempFile;

    // ── Fixtures ──────────────────────────────────────────────────────────────

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

    /// Build a minimal `.tgz` with the given resources as `package/<Type>-N.json` entries.
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

    /// Count rows in a table using a connection from the pool.
    fn count_rows(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
        let conn = pool.get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    }

    // ── count_batch unit tests ─────────────────────────────────────────────────

    #[test]
    fn count_batch_counts_resources_and_top_level_concepts() {
        let resources = vec![sample_code_system(), sample_value_set()];
        let stats = count_batch(&resources);
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 0);
        assert_eq!(stats.concepts, 3); // 3 top-level concepts in sample_code_system
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

    // ── import_tgz behavioural tests ──────────────────────────────────────────

    #[test]
    fn import_tgz_dry_run_returns_correct_counts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        let stats = import_tgz(&pool, tgz.path(), 500, true).expect("dry-run should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 0);
        assert_eq!(stats.concepts, 3); // 3 top-level concepts
    }

    #[test]
    fn import_tgz_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        import_tgz(&pool, tgz.path(), 500, true).expect("dry-run should succeed");

        // Nothing should have been written to the database
        assert_eq!(
            count_rows(&pool, "code_systems"),
            0,
            "dry-run must not write CodeSystems"
        );
        assert_eq!(
            count_rows(&pool, "value_sets"),
            0,
            "dry-run must not write ValueSets"
        );
        assert_eq!(
            count_rows(&pool, "concepts"),
            0,
            "dry-run must not write concepts"
        );
    }

    #[test]
    fn import_tgz_live_writes_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let tgz = make_test_tgz(&[sample_code_system(), sample_value_set()]);

        let stats = import_tgz(&pool, tgz.path(), 500, false).expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concepts, 3);

        assert_eq!(
            count_rows(&pool, "code_systems"),
            1,
            "CodeSystem should be in DB"
        );
        assert_eq!(
            count_rows(&pool, "value_sets"),
            1,
            "ValueSet should be in DB"
        );
        assert_eq!(count_rows(&pool, "concepts"), 3, "concepts should be in DB");
    }

    #[test]
    fn import_tgz_skips_package_metadata_files() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tmp = NamedTempFile::with_suffix(".tgz").unwrap();
        {
            let enc = GzEncoder::new(tmp.reopen().unwrap(), Compression::fast());
            let mut tar = Builder::new(enc);

            // Metadata files that must be ignored
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

            // The real resource
            let cs = serde_json::to_vec(&sample_code_system()).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(cs.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "package/CodeSystem-test.json", cs.as_slice())
                .unwrap();
            tar.finish().unwrap();
        }

        let stats = import_tgz(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(
            stats.code_systems, 1,
            "only the CodeSystem resource should be counted"
        );
        assert_eq!(stats.errors.len(), 0, "no errors expected");
    }

    #[test]
    fn import_tgz_invalid_path_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let result = import_tgz(&pool, Path::new("/nonexistent/path.tgz"), 500, false);
        assert!(result.is_err(), "missing file should produce an error");
    }

    #[test]
    fn import_tgz_batching_preserves_all_resources() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        // 5 code systems, batch_size=2 → 3 batches
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
        let stats = import_tgz(&pool, tgz.path(), 2, false).unwrap();

        assert_eq!(stats.code_systems, 5);
        assert_eq!(stats.concepts, 5);
        assert_eq!(count_rows(&pool, "code_systems"), 5);
    }
}
