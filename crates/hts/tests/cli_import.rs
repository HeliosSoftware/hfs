//! Integration tests for the `hts import` CLI importers.
//!
//! These tests exercise each importer end-to-end using hand-crafted synthetic
//! fixture files, verifying that resources land correctly in the DB and that
//! `$lookup` returns the expected display names.
//!
//! **No real terminology data is used.**  All fixtures are minimal synthetic
//! examples that match the expected file format.

#[cfg(feature = "sqlite")]
mod import_tests {
    use helios_hts::backend::SqliteTerminologyBackend;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn count_rows(pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>, table: &str) -> i64 {
        let conn = pool.get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    // ── HL7 NPM .tgz ─────────────────────────────────────────────────────────

    /// Build a minimal HL7 FHIR NPM `.tgz` in memory.
    ///
    /// The archive contains three files:
    /// - `package/package.json`          (metadata — must be skipped)
    /// - `package/CodeSystem-example.json`
    /// - `package/ValueSet-example.json`
    fn build_minimal_npm_tgz() -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let code_system = serde_json::json!({
            "resourceType": "CodeSystem",
            "id": "example-cs",
            "url": "http://example.org/fhir/cs/test",
            "name": "TestCodeSystem",
            "status": "active",
            "content": "complete",
            "concept": [
                { "code": "A", "display": "Alpha" },
                { "code": "B", "display": "Beta" }
            ]
        })
        .to_string();

        let value_set = serde_json::json!({
            "resourceType": "ValueSet",
            "id": "example-vs",
            "url": "http://example.org/fhir/vs/test",
            "name": "TestValueSet",
            "status": "active",
            "compose": {
                "include": [{ "system": "http://example.org/fhir/cs/test" }]
            }
        })
        .to_string();

        let package_json = r#"{"name":"test.package","version":"1.0.0"}"#;

        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(gz);

        let mut add_entry = |name: &str, content: &str| {
            let bytes = content.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, bytes).unwrap();
        };

        add_entry("package/package.json", package_json);
        add_entry("package/CodeSystem-example.json", &code_system);
        add_entry("package/ValueSet-example.json", &value_set);

        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn import_tgz_imports_code_system_and_value_set() {
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tgz_bytes = build_minimal_npm_tgz();
        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        let stats = import_tgz(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1, "expected 1 CodeSystem");
        assert_eq!(stats.value_sets, 1, "expected 1 ValueSet");
        assert_eq!(stats.concepts, 2, "expected 2 concepts (A, B)");
        assert!(
            stats.errors.is_empty(),
            "unexpected errors: {:?}",
            stats.errors
        );
    }

    #[test]
    fn import_tgz_dry_run_writes_nothing() {
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tgz_bytes = build_minimal_npm_tgz();
        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        import_tgz(&pool, tmp.path(), 500, true).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_tgz_lookup_after_import() {
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tgz_bytes = build_minimal_npm_tgz();
        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        import_tgz(&pool, tmp.path(), 500, false).unwrap();

        // Verify concept A is in the DB with the correct display
        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = ?1 AND c.code = ?2",
                rusqlite::params!["http://example.org/fhir/cs/test", "A"],
                |row| row.get(0),
            )
            .expect("concept A not found after import");

        assert_eq!(display, "Alpha");
    }

    #[test]
    fn import_tgz_is_idempotent() {
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tgz_bytes = build_minimal_npm_tgz();
        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        import_tgz(&pool, tmp.path(), 500, false).unwrap();
        import_tgz(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 2);
    }

    #[test]
    fn import_tgz_skips_non_resource_files() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        // Archive with package metadata only — no FHIR resources
        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(gz);
        let content = b"{}";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "package/package.json", &content[..])
            .unwrap();
        let gz = tar.into_inner().unwrap();
        let tgz_bytes = gz.finish().unwrap();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let stats = import_tgz(&pool, tmp.path(), 500, false).unwrap();
        assert_eq!(stats.code_systems, 0);
        assert_eq!(stats.concepts, 0);
    }

    // ── ICD-10-CM end-to-end (no license needed) ──────────────────────────────

    const ICD10_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ICD10CM.tabular>
  <chapter>
    <name>I</name>
    <desc>Certain infectious and parasitic diseases</desc>
    <section id="A00-A09">
      <desc>Intestinal infectious diseases</desc>
      <diag>
        <name>A00</name>
        <desc>Cholera</desc>
        <diag><name>A00.0</name><desc>Cholera due to Vibrio cholerae 01, biovar cholerae</desc></diag>
        <diag><name>A00.9</name><desc>Cholera, unspecified</desc></diag>
      </diag>
    </section>
  </chapter>
</ICD10CM.tabular>"#;

    #[test]
    fn import_icd10_end_to_end() {
        use helios_hts::import::icd10_cm::import_icd10_cm;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(ICD10_XML.as_bytes()).unwrap();

        let stats = import_icd10_cm(&pool, tmp.path(), 500, false).unwrap();

        // virtual root + 1 chapter + 1 section + 1 header + 2 billable = 6
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 6);
        assert!(stats.errors.is_empty());

        // Verify a billable leaf is queryable
        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://hl7.org/fhir/sid/icd-10-cm' AND c.code = 'A00.9'",
                [],
                |row| row.get(0),
            )
            .expect("A00.9 not found");
        assert_eq!(display, "Cholera, unspecified");
    }

    // ── Error message quality ─────────────────────────────────────────────────

    #[test]
    fn import_tgz_bad_json_records_filename_in_error() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(gz);

        // A file that looks like a FHIR resource but has invalid JSON
        let bad = b"{ this is not json }";
        let mut header = tar::Header::new_gnu();
        header.set_size(bad.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "package/CodeSystem-bad.json", &bad[..])
            .unwrap();

        let gz = tar.into_inner().unwrap();
        let tgz_bytes = gz.finish().unwrap();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let stats = import_tgz(&pool, tmp.path(), 500, false).unwrap();

        // Import should succeed (non-fatal) and record an error with the filename
        assert_eq!(stats.errors.len(), 1, "expected exactly 1 non-fatal error");
        let err = &stats.errors[0];
        assert!(
            err.contains("CodeSystem-bad.json"),
            "error should mention the filename, got: {err}"
        );
    }

    /// Gap 9 — verifies the non-fatal error path: `stats.has_errors()` is true
    /// when a `.tgz` contains a mix of valid and invalid resources, and the
    /// valid resource is still imported.  The exit-code mapping (`Ok(2)`)
    /// is exercised via `run_import` internally; this test confirms the
    /// condition that triggers it.
    #[test]
    fn import_tgz_non_fatal_errors_reflected_in_stats() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use helios_hts::import::tgz::import_tgz;
        use std::io::Write;

        let code_system = serde_json::json!({
            "resourceType": "CodeSystem",
            "id": "cs-ok",
            "url": "http://example.org/cs/ok",
            "name": "OkCS",
            "status": "active",
            "content": "complete",
            "concept": [{ "code": "X", "display": "Xray" }]
        })
        .to_string();

        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(gz);

        let mut add = |name: &str, content: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, content).unwrap();
        };

        add("package/CodeSystem-ok.json", code_system.as_bytes());
        add("package/CodeSystem-bad.json", b"{ not valid json }");

        let gz = tar.into_inner().unwrap();
        let tgz_bytes = gz.finish().unwrap();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".tgz").unwrap();
        tmp.write_all(&tgz_bytes).unwrap();

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let stats = import_tgz(&pool, tmp.path(), 500, false).unwrap();

        // Valid resource imported despite the bad neighbour
        assert_eq!(
            stats.code_systems, 1,
            "valid CodeSystem must still be imported"
        );
        assert_eq!(stats.concepts, 1);

        // Non-fatal error recorded — this is what drives exit code 2 in run_import
        assert!(stats.has_errors(), "expected has_errors() == true");
        assert_eq!(stats.errors.len(), 1);
    }

    // ── --verbose flag (config smoke test) ───────────────────────────────────

    #[test]
    fn import_args_verbose_flag_defaults_to_false() {
        use clap::Parser;
        use helios_hts::config::ImportArgs;

        // Simulate `hts import /tmp/x.tgz` with no --verbose
        let args = ImportArgs::try_parse_from(["import", "/tmp/x.tgz"]).unwrap();
        assert!(!args.verbose);
    }

    #[test]
    fn import_args_verbose_flag_can_be_set() {
        use clap::Parser;
        use helios_hts::config::ImportArgs;

        let args = ImportArgs::try_parse_from(["import", "/tmp/x.tgz", "--verbose"]).unwrap();
        assert!(args.verbose);
    }

    // ── dry-run gate (all formats) ────────────────────────────────────────────

    #[test]
    fn import_args_dry_run_defaults_to_false() {
        use clap::Parser;
        use helios_hts::config::ImportArgs;

        let args = ImportArgs::try_parse_from(["import", "/tmp/x.tgz"]).unwrap();
        assert!(!args.dry_run);
    }

    #[test]
    fn import_args_dry_run_flag_can_be_set() {
        use clap::Parser;
        use helios_hts::config::ImportArgs;

        let args = ImportArgs::try_parse_from(["import", "/tmp/x.tgz", "--dry-run"]).unwrap();
        assert!(args.dry_run);
    }

    // ── batch_size=0 guard ────────────────────────────────────────────────────

    #[test]
    fn import_icd10_batch_size_zero_does_not_panic() {
        use helios_hts::import::icd10_cm::import_icd10_cm;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(ICD10_XML.as_bytes()).unwrap();

        // batch_size=0 must not panic; the .max(1) guard clamps it to 1
        let stats = import_icd10_cm(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 6);
    }

    #[test]
    fn import_rxnorm_batch_size_zero_does_not_panic() {
        use helios_hts::import::rxnorm_rrf::import_rxnorm_rrf;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
            .unwrap()
            .write_all(
                b"1049502|ENG|P|L1|PF|S1|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n",
            )
            .unwrap();
        std::fs::File::create(dir.path().join("RXNREL.RRF"))
            .unwrap()
            .write_all(b"")
            .unwrap();

        let stats = import_rxnorm_rrf(&pool, dir.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 1);
    }

    // ── --format override ─────────────────────────────────────────────────────

    #[test]
    fn import_args_format_override_takes_precedence() {
        use clap::Parser;
        use helios_hts::config::{ImportArgs, ImportFormat};

        // The file has a .tgz extension but --format overrides to loinc
        let args =
            ImportArgs::try_parse_from(["import", "/tmp/data.tgz", "--format", "loinc"]).unwrap();

        assert!(matches!(args.format, Some(ImportFormat::Loinc)));
    }

    #[test]
    fn import_args_format_all_values_parse() {
        use clap::Parser;
        use helios_hts::config::{ImportArgs, ImportFormat};

        let cases = [
            ("hl7-npm", ImportFormat::Hl7Npm),
            ("snomed-rf2", ImportFormat::SnomedRf2),
            ("loinc", ImportFormat::Loinc),
            ("icd10-cm", ImportFormat::Icd10Cm),
            ("rxnorm", ImportFormat::Rxnorm),
        ];

        for (flag_val, expected) in cases {
            let args =
                ImportArgs::try_parse_from(["import", "/tmp/x", "--format", flag_val]).unwrap();
            assert!(
                std::mem::discriminant(&args.format.unwrap()) == std::mem::discriminant(&expected),
                "failed for {flag_val}"
            );
        }
    }
}
