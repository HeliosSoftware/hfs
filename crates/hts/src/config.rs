use std::fmt;
use std::path::Path;

use clap::{Parser, Subcommand, ValueEnum};

// ── Top-level CLI ─────────────────────────────────────────────────────────────

/// Top-level CLI for the Helios Terminology Service.
///
/// When no subcommand is provided the server starts with default settings,
/// preserving backwards-compatible behaviour (`hts` == `hts serve`).
#[derive(Parser, Debug)]
#[command(
    name = "hts",
    about = "Helios Terminology Service — FHIR Terminology Operations",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the FHIR Terminology HTTP server (default when no subcommand given)
    Serve(HtsConfig),
    /// Bulk-import a terminology package from the filesystem
    Import(ImportArgs),
}

// ── Server config ─────────────────────────────────────────────────────────────

/// Configuration for the Helios Terminology Service HTTP server.
#[derive(Parser, Debug, Clone)]
pub struct HtsConfig {
    /// Server port
    #[arg(long, env = "HTS_SERVER_PORT", default_value = "8090")]
    pub port: u16,

    /// Server host to bind
    #[arg(long, env = "HTS_SERVER_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, env = "HTS_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Database URL (SQLite file path or PostgreSQL connection string)
    #[arg(long, env = "HTS_DATABASE_URL", default_value = "./data/hts.db")]
    pub database_url: String,

    /// Storage backend (sqlite | postgres)
    #[arg(long, env = "HTS_STORAGE_BACKEND", default_value = "sqlite")]
    pub storage_backend: String,

    /// Enable CORS
    #[arg(long, env = "HTS_ENABLE_CORS", default_value = "true")]
    pub enable_cors: bool,

    /// Allowed CORS origins (comma-separated)
    #[arg(long, env = "HTS_CORS_ORIGINS", default_value = "*")]
    pub cors_origins: String,

    /// Maximum number of codes allowed in a single ValueSet expansion.
    /// Requests that would exceed this limit receive HTTP 422 with issue
    /// code `too-costly`.
    #[arg(long, env = "HTS_MAX_EXPANSION_SIZE", default_value = "10000")]
    pub max_expansion_size: u32,
}

impl HtsConfig {
    /// Returns the socket address string for binding.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl Default for HtsConfig {
    fn default() -> Self {
        Self {
            port: 8090,
            host: "127.0.0.1".into(),
            log_level: "info".into(),
            database_url: "./data/hts.db".into(),
            storage_backend: "sqlite".into(),
            enable_cors: true,
            cors_origins: "*".into(),
            max_expansion_size: 10_000,
        }
    }
}

// ── Import format ─────────────────────────────────────────────────────────────

/// Terminology distribution format for `hts import`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportFormat {
    /// HL7 FHIR NPM package (.tgz) from terminology.hl7.org
    #[value(name = "hl7-npm")]
    Hl7Npm,
    /// SNOMED CT RF2 snapshot distribution (.zip)
    #[value(name = "snomed-rf2")]
    SnomedRf2,
    /// LOINC CSV distribution (.zip) from loinc.org
    #[value(name = "loinc")]
    Loinc,
    /// ICD-10-CM tabular XML from CMS
    #[value(name = "icd10-cm")]
    Icd10Cm,
    /// RxNorm RRF files from NLM
    #[value(name = "rxnorm")]
    Rxnorm,
}

impl fmt::Display for ImportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportFormat::Hl7Npm => write!(f, "hl7-npm"),
            ImportFormat::SnomedRf2 => write!(f, "snomed-rf2"),
            ImportFormat::Loinc => write!(f, "loinc"),
            ImportFormat::Icd10Cm => write!(f, "icd10-cm"),
            ImportFormat::Rxnorm => write!(f, "rxnorm"),
        }
    }
}

/// Auto-detect the import format from the file at `path`.
///
/// Detection rules (in order):
/// - `.tgz` / `.tar.gz` → [`ImportFormat::Hl7Npm`]
/// - `.xml` containing "tabular" in the filename → [`ImportFormat::Icd10Cm`]
/// - `.rrf` (case-insensitive) → [`ImportFormat::Rxnorm`]
/// - directory → [`ImportFormat::Rxnorm`]
/// - `.zip` → peeks into the archive to distinguish formats
/// - anything else → `None` (user must pass `--format`)
pub fn detect_format(path: &Path) -> Option<ImportFormat> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    if name.ends_with(".tgz") || name.ends_with(".tar.gz") {
        return Some(ImportFormat::Hl7Npm);
    }
    // Only detect ICD-10-CM for XML files that look like the CMS tabular file
    // (e.g. icd10cm_tabular_2025.xml). Generic .xml files require --format.
    if name.ends_with(".xml") && name.contains("tabular") {
        return Some(ImportFormat::Icd10Cm);
    }
    if name.ends_with(".rrf") {
        return Some(ImportFormat::Rxnorm);
    }
    if path.is_dir() {
        return Some(ImportFormat::Rxnorm);
    }
    if name.ends_with(".zip") {
        return detect_zip_format(path);
    }
    None
}

/// Peek into a ZIP to identify the terminology distribution format.
///
/// Checks entry names in order:
/// - Contains `concept_full` or `description_full` (RF2) → SNOMED RF2
/// - Ends with `loinctable.csv` → LOINC
/// - Ends with `rxnconso.rrf` → RxNorm
/// - Ends with `.xml` and contains `tabular` → ICD-10-CM
/// - Otherwise → `None` (user must pass `--format`)
fn detect_zip_format(path: &Path) -> Option<ImportFormat> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;

    for i in 0..zip.len() {
        let entry = zip.by_index(i).ok()?;
        let entry_name = entry.name().to_lowercase();
        if entry_name.contains("concept_full") || entry_name.contains("description_full") {
            return Some(ImportFormat::SnomedRf2);
        }
        if entry_name.ends_with("loinctable.csv") {
            return Some(ImportFormat::Loinc);
        }
        if entry_name.ends_with("rxnconso.rrf") {
            return Some(ImportFormat::Rxnorm);
        }
        if entry_name.ends_with(".xml") && entry_name.contains("tabular") {
            return Some(ImportFormat::Icd10Cm);
        }
    }
    None
}

// ── Import args ───────────────────────────────────────────────────────────────

/// Arguments for `hts import`.
#[derive(Parser, Debug)]
pub struct ImportArgs {
    /// Path to the terminology package file or directory
    pub path: std::path::PathBuf,

    /// Terminology distribution format. Auto-detected from the file when omitted.
    /// Required for .zip files (cannot distinguish SNOMED from LOINC by extension alone).
    #[arg(long, value_enum)]
    pub format: Option<ImportFormat>,

    /// Database URL (SQLite file path or PostgreSQL connection string)
    #[arg(long, env = "HTS_DATABASE_URL", default_value = "./data/hts.db")]
    pub database_url: String,

    /// Storage backend to import into (`sqlite` or `postgres`)
    #[arg(long, env = "HTS_STORAGE_BACKEND", default_value = "sqlite")]
    pub storage_backend: String,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, env = "HTS_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Number of resources per import batch (controls peak memory usage)
    #[arg(long, default_value = "500")]
    pub batch_size: usize,

    /// Parse and count resources without writing anything to the database
    #[arg(long)]
    pub dry_run: bool,

    /// Emit per-batch progress details to stderr during import
    #[arg(long)]
    pub verbose: bool,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tgz_extension() {
        assert_eq!(
            detect_format(Path::new("hl7.terminology.r4-6.0.0.tgz")),
            Some(ImportFormat::Hl7Npm)
        );
    }

    #[test]
    fn detect_tar_gz_extension() {
        assert_eq!(
            detect_format(Path::new("package.tar.gz")),
            Some(ImportFormat::Hl7Npm)
        );
    }

    #[test]
    fn detect_xml_tabular_returns_icd10() {
        assert_eq!(
            detect_format(Path::new("icd10cm_tabular_2025.xml")),
            Some(ImportFormat::Icd10Cm)
        );
    }

    #[test]
    fn detect_xml_generic_returns_none() {
        // A non-tabular XML (FHIR resource, CDA, etc.) must not auto-detect as ICD-10-CM
        assert_eq!(detect_format(Path::new("patient.xml")), None);
        assert_eq!(detect_format(Path::new("bundle.xml")), None);
    }

    #[test]
    fn detect_rrf_extension() {
        assert_eq!(
            detect_format(Path::new("RXNCONSO.RRF")),
            Some(ImportFormat::Rxnorm)
        );
    }

    #[test]
    fn detect_rrf_lowercase() {
        assert_eq!(
            detect_format(Path::new("rxnconso.rrf")),
            Some(ImportFormat::Rxnorm)
        );
    }

    #[test]
    fn detect_unknown_returns_none() {
        assert_eq!(detect_format(Path::new("terms.csv")), None);
        assert_eq!(detect_format(Path::new("data.json")), None);
        assert_eq!(detect_format(Path::new("archive.7z")), None);
    }

    #[test]
    fn detect_directory_returns_rxnorm() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_format(dir.path()), Some(ImportFormat::Rxnorm));
    }

    #[test]
    fn detect_zip_snomed() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();
            zip.start_file(
                "SnomedCT/Snapshot/Terminology/Concept_Full_INT_20240101.txt",
                opts,
            )
            .unwrap();
            zip.write_all(b"dummy").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(detect_format(tmp.path()), Some(ImportFormat::SnomedRf2));
    }

    #[test]
    fn detect_zip_loinc() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();
            zip.start_file("LoincTable.csv", opts).unwrap();
            zip.write_all(b"dummy").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(detect_format(tmp.path()), Some(ImportFormat::Loinc));
    }

    #[test]
    fn detect_zip_unknown_returns_none() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();
            zip.start_file("readme.txt", opts).unwrap();
            zip.write_all(b"dummy").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(detect_format(tmp.path()), None);
    }

    #[test]
    fn detect_zip_rxnorm() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();
            zip.start_file("rrf/RXNCONSO.RRF", opts).unwrap();
            zip.write_all(b"dummy").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(detect_format(tmp.path()), Some(ImportFormat::Rxnorm));
    }

    #[test]
    fn detect_zip_icd10_tabular_xml() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();
            zip.start_file("icd10cm_tabular_2025.xml", opts).unwrap();
            zip.write_all(b"<ICD10CM.tabular/>").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(detect_format(tmp.path()), Some(ImportFormat::Icd10Cm));
    }

    #[test]
    fn import_format_display() {
        assert_eq!(ImportFormat::Hl7Npm.to_string(), "hl7-npm");
        assert_eq!(ImportFormat::SnomedRf2.to_string(), "snomed-rf2");
        assert_eq!(ImportFormat::Loinc.to_string(), "loinc");
        assert_eq!(ImportFormat::Icd10Cm.to_string(), "icd10-cm");
        assert_eq!(ImportFormat::Rxnorm.to_string(), "rxnorm");
    }
}
