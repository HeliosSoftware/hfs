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
    use helios_hts::backends::SqliteTerminologyBackend;

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
            ("icd9-cm", ImportFormat::Icd9Cm),
            ("rxnorm", ImportFormat::Rxnorm),
            ("ucum", ImportFormat::Ucum),
            ("nci-thesaurus", ImportFormat::NciThesaurus),
            ("mesh", ImportFormat::Mesh),
            ("dicom", ImportFormat::Dicom),
            ("hl7-v2-tables", ImportFormat::Hl7V2Tables),
            ("nucc", ImportFormat::Nucc),
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

    // ── UCUM end-to-end ───────────────────────────────────────────────────────

    const UCUM_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="http://unitsofmeasure.org/ucum-essence" version="2.2">
  <prefix Code="k" CODE="K"><name>kilo</name></prefix>
  <base-unit Code="m" CODE="M" isMetric="yes"><name>meter</name></base-unit>
  <unit Code="[lb_av]" CODE="[LB_AV]" isMetric="no"><name>pound</name></unit>
</root>"#;

    #[test]
    fn import_ucum_end_to_end() {
        use helios_hts::import::ucum::import_ucum;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(UCUM_XML.as_bytes()).unwrap();

        let stats = import_ucum(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert!(stats.errors.is_empty());

        // Verify a specific code is queryable
        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://unitsofmeasure.org' AND c.code = 'm'",
                [],
                |row| row.get(0),
            )
            .expect("meter not found");
        assert_eq!(display, "meter");
    }

    #[test]
    fn import_ucum_dry_run_writes_nothing() {
        use helios_hts::import::ucum::import_ucum;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(UCUM_XML.as_bytes()).unwrap();

        let stats = import_ucum(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_ucum_idempotent() {
        use helios_hts::import::ucum::import_ucum;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(UCUM_XML.as_bytes()).unwrap();

        import_ucum(&pool, tmp.path(), 500, false).unwrap();
        import_ucum(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        // virtual root + 3 unit codes
        assert_eq!(count_rows(&pool, "concepts"), 4);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 3);
    }

    #[test]
    fn import_ucum_batch_size_zero_does_not_panic() {
        use helios_hts::import::ucum::import_ucum;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(UCUM_XML.as_bytes()).unwrap();

        let stats = import_ucum(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 3);
    }

    // ── NCI Thesaurus end-to-end ──────────────────────────────────────────────

    /// Minimal NCI Thesaurus flat-file fixture (tab-delimited).
    /// Columns: code, concept name, parents (pipe-sep), synonyms, definition, display_name, ...
    const NCI_TXT: &str = "C12345\tRoot Concept\t\t\tA root concept.\tRoot Concept\n\
C67890\tChild Concept\tC12345\t\tA child concept.\tChild Concept\n\
C11111\tAnother Root\t\t\tAnother root.\tAnother Root\n";

    #[test]
    fn import_nci_thesaurus_end_to_end() {
        use helios_hts::import::nci_thesaurus::import_nci_thesaurus;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(NCI_TXT.as_bytes()).unwrap();

        let stats = import_nci_thesaurus(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert!(stats.errors.is_empty());

        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://ncicb.nci.nih.gov/xml/owl/EVS/Thesaurus.owl' \
                 AND c.code = 'C67890'",
                [],
                |row| row.get(0),
            )
            .expect("C67890 not found");
        assert_eq!(display, "Child Concept");
    }

    #[test]
    fn import_nci_thesaurus_dry_run_writes_nothing() {
        use helios_hts::import::nci_thesaurus::import_nci_thesaurus;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(NCI_TXT.as_bytes()).unwrap();

        import_nci_thesaurus(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_nci_thesaurus_idempotent() {
        use helios_hts::import::nci_thesaurus::import_nci_thesaurus;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(NCI_TXT.as_bytes()).unwrap();

        import_nci_thesaurus(&pool, tmp.path(), 500, false).unwrap();
        import_nci_thesaurus(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 3);
    }

    #[test]
    fn import_nci_thesaurus_batch_size_zero_does_not_panic() {
        use helios_hts::import::nci_thesaurus::import_nci_thesaurus;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(NCI_TXT.as_bytes()).unwrap();

        let stats = import_nci_thesaurus(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 3);
    }

    // ── DICOM end-to-end ──────────────────────────────────────────────────────

    const DICOM_CSV: &str = "\
CodeValue,CodingSchemeDesignator,CodeMeaning\n\
121049,DCM,Image Position (Patient)\n\
121050,DCM,Observer Context\n\
121058,DCM,Procedure reported\n\
";

    #[test]
    fn import_dicom_end_to_end() {
        use helios_hts::import::dicom::import_dicom;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(DICOM_CSV.as_bytes()).unwrap();

        let stats = import_dicom(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert!(stats.errors.is_empty());

        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://dicom.nema.org/resources/ontology/DCM' \
                 AND c.code = '121050'",
                [],
                |row| row.get(0),
            )
            .expect("121050 not found");
        assert_eq!(display, "Observer Context");
    }

    #[test]
    fn import_dicom_dry_run_writes_nothing() {
        use helios_hts::import::dicom::import_dicom;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(DICOM_CSV.as_bytes()).unwrap();

        import_dicom(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_dicom_idempotent() {
        use helios_hts::import::dicom::import_dicom;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(DICOM_CSV.as_bytes()).unwrap();

        import_dicom(&pool, tmp.path(), 500, false).unwrap();
        import_dicom(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 4); // virtual root + 3
    }

    #[test]
    fn import_dicom_batch_size_zero_does_not_panic() {
        use helios_hts::import::dicom::import_dicom;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(DICOM_CSV.as_bytes()).unwrap();

        let stats = import_dicom(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 3);
    }

    // ── HL7 v2 tables end-to-end ──────────────────────────────────────────────

    const HL7V2_XML: &str = r#"<?xml version="1.0"?>
<HL7Tables>
  <HL7Table id="0001" name="Administrative Sex">
    <tableEntry code="F" displayName="Female"/>
    <tableEntry code="M" displayName="Male"/>
    <tableEntry code="O" displayName="Other"/>
  </HL7Table>
</HL7Tables>"#;

    #[test]
    fn import_hl7_v2_tables_end_to_end() {
        use helios_hts::import::hl7_v2_tables::import_hl7_v2_tables;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(HL7V2_XML.as_bytes()).unwrap();

        let stats = import_hl7_v2_tables(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert!(stats.errors.is_empty());

        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://terminology.hl7.org/CodeSystem/v2-0001' \
                 AND c.code = 'F'",
                [],
                |row| row.get(0),
            )
            .expect("code F not found");
        assert_eq!(display, "Female");
    }

    #[test]
    fn import_hl7_v2_tables_dry_run_writes_nothing() {
        use helios_hts::import::hl7_v2_tables::import_hl7_v2_tables;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(HL7V2_XML.as_bytes()).unwrap();

        import_hl7_v2_tables(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_hl7_v2_tables_idempotent() {
        use helios_hts::import::hl7_v2_tables::import_hl7_v2_tables;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(HL7V2_XML.as_bytes()).unwrap();

        import_hl7_v2_tables(&pool, tmp.path(), 500, false).unwrap();
        import_hl7_v2_tables(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        // virtual root (v2-0001) + 3 real codes = 4
        assert_eq!(count_rows(&pool, "concepts"), 4);
    }

    #[test]
    fn import_hl7_v2_tables_batch_size_zero_does_not_panic() {
        use helios_hts::import::hl7_v2_tables::import_hl7_v2_tables;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        tmp.write_all(HL7V2_XML.as_bytes()).unwrap();

        let stats = import_hl7_v2_tables(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 3);
    }

    // ── NUCC end-to-end ───────────────────────────────────────────────────────

    const NUCC_CSV: &str = "\
Code,Grouping,Classification,Specialization,Definition,Notes,,,,,Display Name\n\
101Y00000X,Behavioral Health & Social Service Providers,Counselor,,A provider who...,,,,,Counselor\n\
101YA0400X,Behavioral Health & Social Service Providers,Counselor,Addiction (Substance Use Disorder),Addiction Counselor,,,,,Addiction Counselor\n\
207R00000X,Allopathic & Osteopathic Physicians,Internal Medicine,,An internal medicine physician,,,,,Internist\n\
";

    #[test]
    fn import_nucc_end_to_end() {
        use helios_hts::import::nucc::import_nucc;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(NUCC_CSV.as_bytes()).unwrap();

        let stats = import_nucc(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 3);
        assert!(stats.errors.is_empty());

        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://nucc.org/provider-taxonomy' \
                 AND c.code = '207R00000X'",
                [],
                |row| row.get(0),
            )
            .expect("207R00000X not found");
        assert_eq!(display, "Internist");
    }

    #[test]
    fn import_nucc_dry_run_writes_nothing() {
        use helios_hts::import::nucc::import_nucc;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(NUCC_CSV.as_bytes()).unwrap();

        import_nucc(&pool, tmp.path(), 500, true).unwrap();
        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_nucc_idempotent() {
        use helios_hts::import::nucc::import_nucc;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(NUCC_CSV.as_bytes()).unwrap();

        import_nucc(&pool, tmp.path(), 500, false).unwrap();
        import_nucc(&pool, tmp.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        // 1 virtual root (NUCC) + 2 synthetic groupings + 2 synthetic classifications + 3 real codes = 8
        assert_eq!(count_rows(&pool, "concepts"), 8);
    }

    #[test]
    fn import_nucc_batch_size_zero_does_not_panic() {
        use helios_hts::import::nucc::import_nucc;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(NUCC_CSV.as_bytes()).unwrap();

        let stats = import_nucc(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 3);
    }

    // ── ICD-9-CM end-to-end ───────────────────────────────────────────────────

    /// Minimal pipe-delimited ICD-9-CM fixture with two categories, one
    /// subcategory, and one sub-subcategory.
    const ICD9_TXT: &str = "\
001|Cholera\n\
0010|Cholera due to vibrio cholerae\n\
00100|Cholera due to vibrio cholerae\n\
002|Typhoid and paratyphoid fevers\n\
";

    #[test]
    fn import_icd9_end_to_end() {
        use helios_hts::import::icd9_cm::import_icd9_cm;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(ICD9_TXT.as_bytes()).unwrap();

        let stats = import_icd9_cm(&pool, tmp.path(), 500, false).unwrap();

        // 4 concepts from the file
        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 4);
        assert!(stats.errors.is_empty());

        // Verify a leaf code is queryable with the correct display code (dot inserted)
        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT c.display FROM concepts c \
                 JOIN code_systems cs ON cs.id = c.system_id \
                 WHERE cs.url = 'http://hl7.org/fhir/sid/icd-9-cm' AND c.code = '001.00'",
                [],
                |row| row.get(0),
            )
            .expect("001.00 not found");
        assert_eq!(display, "Cholera due to vibrio cholerae");
    }

    #[test]
    fn import_icd9_batch_size_zero_does_not_panic() {
        use helios_hts::import::icd9_cm::import_icd9_cm;
        use std::io::Write;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        tmp.write_all(ICD9_TXT.as_bytes()).unwrap();

        // batch_size=0 must not panic; the .max(1) guard clamps it to 1
        let stats = import_icd9_cm(&pool, tmp.path(), 0, false).unwrap();
        assert_eq!(stats.concepts, 4);
    }
}
