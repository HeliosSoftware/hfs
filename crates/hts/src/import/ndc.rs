//! FDA National Drug Code (NDC) Directory importer.
//!
//! Parses the FDA NDC Text Files distribution (`product.txt`) and imports all
//! active drug product codes, display names, and a product-type hierarchy into
//! the HTS normalized schema.
//!
//! # No license required
//!
//! The NDC Directory is published by the U.S. Food and Drug Administration (FDA),
//! a US federal government work in the public domain.  No registration or license
//! agreement is required.
//! Download from: <https://www.accessdata.fda.gov/cder/ndctext.zip>
//!
//! # File format
//!
//! The FDA distributes a ZIP (`ndctext.zip`) containing two tab-delimited files:
//!
//! - `product.txt` — one row per drug product (5-4 NDC code format)
//! - `package.txt` — one row per package (11-digit NDC code, not imported here)
//!
//! The first line of `product.txt` is a tab-separated header row that defines
//! column names.  This importer reads column indices from the header dynamically
//! so it is robust to future additions or reorderings.
//!
//! Key columns used:
//!
//! | Column | Description |
//! |--------|-------------|
//! | `PRODUCTNDC` | NDC product code in 5-4 format, e.g. `0002-1200` |
//! | `PROPRIETARYNAME` | Brand name, e.g. `Prozac` |
//! | `NONPROPRIETARYNAME` | Generic / active-ingredient name |
//! | `PRODUCTTYPENAME` | Product category used as a hierarchy grouper |
//! | `NDC_EXCLUDE_FLAG` | `E` = excluded from active directory; blank = active |
//!
//! # Hierarchy
//!
//! ```text
//! NDC  (virtual root)
//! ├── HUMAN PRESCRIPTION DRUG
//! │   ├── 0002-1200
//! │   └── …
//! ├── HUMAN OTC DRUG
//! │   └── …
//! └── …
//! ```
//!
//! Products with a missing or empty `PRODUCTTYPENAME` are hung directly off the
//! virtual root.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Canonical FHIR URI for the NDC code system.
const NDC_URL: &str = "http://hl7.org/fhir/sid/ndc";
/// Short machine name stored in the `code_systems.name` column.
const NDC_NAME: &str = "NDC";
/// Human-readable title for the `code_systems.title` column.
const NDC_TITLE: &str = "NDC (National Drug Code Directory)";
/// Version label.  The NDC Directory has no formal version number; the FDA
/// updates it continuously, so we label it `"current"`.
const NDC_VERSION: &str = "current";
/// Code of the virtual root concept.  Every product-type grouper hangs off
/// this root, giving the two-level hierarchy:
/// `NDC → <product-type> → <individual-product>`.
const ROOT_CODE: &str = "NDC";

// ── Data types ────────────────────────────────────────────────────────────────

/// A single parsed drug product, ready for DB insertion.
#[derive(Debug)]
struct NdcProduct {
    /// NDC product code in 5-4 format (labeler-product), e.g. `"0002-1200"`.
    code: String,
    /// Human-readable display built from the source columns:
    /// - `"Brand (Generic)"` when both names are present.
    /// - `"Brand"` when only the brand/proprietary name is present.
    /// - `"generic"` when only the non-proprietary name is present.
    /// - Empty string when neither name is present (recorded as a non-fatal
    ///   error in [`ImportStats::errors`]).
    display: String,
    /// Product-type name used as the mid-level hierarchy grouper,
    /// e.g. `"HUMAN PRESCRIPTION DRUG"`.  Empty when `PRODUCTTYPENAME` is
    /// blank; those products are hung directly off the virtual root.
    product_type: String,
}

/// Column indices resolved dynamically from the header row of `product.txt`.
///
/// The FDA sometimes adds or reorders columns between releases.  Resolving
/// indices at parse time (rather than hard-coding them) makes the importer
/// robust to schema changes.
struct ColIdx {
    /// Column index of `PRODUCTNDC`.
    productndc: usize,
    /// Column index of `PROPRIETARYNAME` (brand name).
    proprietaryname: usize,
    /// Column index of `NONPROPRIETARYNAME` (generic / active-ingredient name).
    nonproprietaryname: usize,
    /// Column index of `PRODUCTTYPENAME` (hierarchy grouper).
    producttypename: usize,
    /// Column index of `NDC_EXCLUDE_FLAG` (`"E"` = excluded, blank = active).
    ndc_exclude_flag: usize,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import the FDA NDC Directory (`product.txt`) into the HTS database.
///
/// `path` may point to:
/// - The raw `product.txt` tab-delimited file, or
/// - A `.zip` archive containing a `product.txt` file (e.g. `ndctext.zip`).
///
/// Products marked `NDC_EXCLUDE_FLAG = "E"` are skipped.
#[cfg(feature = "sqlite")]
pub fn import_ndc(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let text = read_product_txt(path)?;
    let (products, errors) = parse_product_txt(&text)?;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = products.len() as u32;
        eprintln!(
            "[ndc] dry-run — {} products parsed, no DB writes",
            products.len()
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    // Virtual root concept.
    conn.execute(
        "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
         VALUES (?1, ?2, ?3, 'header')",
        rusqlite::params![system_id, ROOT_CODE, NDC_TITLE],
    )
    .map_err(|e| HtsError::Internal(format!("Insert virtual root: {e}")))?;

    // Collect unique product-type names so we can insert grouper concepts first.
    let product_types: BTreeSet<String> = products
        .iter()
        .filter(|p| !p.product_type.is_empty())
        .map(|p| p.product_type.clone())
        .collect();

    insert_groupers(&conn, &system_id, &product_types)?;

    let total = products.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in products.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;
        insert_hierarchy_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[ndc] batch {}/{total_batches} — +{} products (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

/// Read `product.txt` from a raw file or from a ZIP archive.
fn read_product_txt(path: &Path) -> Result<String, HtsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "zip" {
        read_product_txt_from_zip(path)
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display())))
    }
}

/// Extract `product.txt` from a ZIP archive (any path depth).
fn read_product_txt_from_zip(path: &Path) -> Result<String, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let idx = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .map(|e| {
                    let n = e.name().to_ascii_lowercase();
                    n == "product.txt" || n.ends_with("/product.txt")
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No 'product.txt' found inside ZIP '{}'",
                path.display()
            ))
        })?;

    let mut entry = archive
        .by_index(idx)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read product.txt from ZIP: {e}")))?;
    Ok(buf)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Resolve column indices from the `product.txt` header row.
///
/// Returns an error if any required column is absent.
fn resolve_columns(header: &str) -> Result<ColIdx, HtsError> {
    let cols: Vec<&str> = header.split('\t').collect();
    let find = |name: &str| -> Result<usize, HtsError> {
        cols.iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                HtsError::InvalidRequest(format!(
                    "product.txt header is missing required column '{name}'"
                ))
            })
    };

    Ok(ColIdx {
        productndc: find("PRODUCTNDC")?,
        proprietaryname: find("PROPRIETARYNAME")?,
        nonproprietaryname: find("NONPROPRIETARYNAME")?,
        producttypename: find("PRODUCTTYPENAME")?,
        ndc_exclude_flag: find("NDC_EXCLUDE_FLAG")?,
    })
}

/// Parse the tab-delimited `product.txt` content.
///
/// The first row must be the header.  Products flagged `NDC_EXCLUDE_FLAG = "E"`
/// are skipped.  Returns `(products, non_fatal_errors)`.
fn parse_product_txt(text: &str) -> Result<(Vec<NdcProduct>, Vec<String>), HtsError> {
    let mut lines = BufReader::new(text.as_bytes()).lines().enumerate();

    // Parse header row to resolve column indices.
    let (_, first) = lines
        .next()
        .ok_or_else(|| HtsError::InvalidRequest("product.txt is empty".to_string()))?;
    let header =
        first.map_err(|e| HtsError::InvalidRequest(format!("Cannot read header row: {e}")))?;
    let idx = resolve_columns(&header)?;

    let mut products = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (line_num, line_result) in lines {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }

        let cols: Vec<&str> = line.split('\t').collect();

        // Skip excluded entries.
        if cols
            .get(idx.ndc_exclude_flag)
            .map(|s| s.trim())
            .unwrap_or("")
            == "E"
        {
            continue;
        }

        let ndc = match cols.get(idx.productndc).map(|s| s.trim()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                errors.push(format!(
                    "line {}: missing PRODUCTNDC — skipped",
                    line_num + 1
                ));
                continue;
            }
        };

        let brand = cols
            .get(idx.proprietaryname)
            .map(|s| s.trim())
            .unwrap_or("")
            .to_string();
        let generic = cols
            .get(idx.nonproprietaryname)
            .map(|s| s.trim())
            .unwrap_or("")
            .to_string();

        let display = match (brand.is_empty(), generic.is_empty()) {
            (false, false) => format!("{brand} ({generic})"),
            (false, true) => brand,
            (true, false) => generic.clone(),
            (true, true) => {
                errors.push(format!(
                    "line {}: NDC {ndc} has no brand or generic name — imported with empty display",
                    line_num + 1
                ));
                String::new()
            }
        };

        let product_type = cols
            .get(idx.producttypename)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        products.push(NdcProduct {
            code: ndc,
            display,
            product_type,
        });
    }

    Ok((products, errors))
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Ensure the NDC `code_systems` row exists and return its `id`.
///
/// Uses `INSERT OR IGNORE` so that a second import run does not create a
/// duplicate row.  The `id` of the existing row is always returned via a
/// subsequent `SELECT`, so the generated UUID is discarded on the second call.
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', 'complete', ?6, ?6)",
        rusqlite::params![id, NDC_URL, NDC_VERSION, NDC_NAME, NDC_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    conn.query_row(
        "SELECT id FROM code_systems WHERE url = ?1",
        rusqlite::params![NDC_URL],
        |row| row.get(0),
    )
    .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))
}

/// Insert product-type grouper concepts and their edges to the virtual root.
///
/// Groupers are inserted before the product batch loop so that the hierarchy
/// edges for individual products (`grouper → product`) can reference rows that
/// already exist.  `INSERT OR IGNORE` makes this idempotent across re-imports.
#[cfg(feature = "sqlite")]
fn insert_groupers(
    conn: &rusqlite::Connection,
    system_id: &str,
    product_types: &BTreeSet<String>,
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin grouper transaction: {e}")))?;

    for pt in product_types {
        tx.execute(
            "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, 'header')",
            rusqlite::params![system_id, pt, pt],
        )
        .map_err(|e| HtsError::Internal(format!("Insert grouper '{pt}': {e}")))?;

        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, ROOT_CODE, pt],
        )
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy root->{pt}: {e}")))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit groupers: {e}")))
}

/// Insert (or replace) one batch of drug-product concepts inside a single
/// SQLite transaction.
///
/// `INSERT OR REPLACE` handles re-imports by overwriting any previously stored
/// display value for the same `(system_id, code)` pair.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[NdcProduct],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin concept transaction: {e}")))?;

    for p in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display) VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, p.code, p.display],
        )
        .map_err(|e| HtsError::Internal(format!("Insert concept '{}': {e}", p.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit concepts: {e}")))
}

/// Insert hierarchy edges for one batch of drug products inside a single
/// SQLite transaction.
///
/// Each product gets one edge: `grouper → product` (or `ROOT → product` when
/// `product_type` is empty).  `INSERT OR IGNORE` ensures idempotency.
#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[NdcProduct],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for p in batch {
        let parent: &str = if p.product_type.is_empty() {
            ROOT_CODE
        } else {
            &p.product_type
        };
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent, p.code],
        )
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy {parent}->{}: {e}", p.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_columns ───────────────────────────────────────────────────────

    fn sample_header() -> &'static str {
        "PRODUCTID\tPRODUCTNDC\tPRODUCTTYPENAME\tPROPRIETARYNAME\tPROPRIETARYNAMESUFFIX\t\
         NONPROPRIETARYNAME\tDOSAGEFORMNAME\tROUTENAME\tSTARTMARKETINGDATE\tENDMARKETINGDATE\t\
         MARKETINGCATEGORYNAME\tAPPLICATIONNUMBER\tLABELERNAME\tSUBSTANCENAME\t\
         ACTIVE_NUMERATOR_STRENGTH\tACTIVE_INGRED_UNIT\tPHARM_CLASSES\tDEASCHEDULE\t\
         NDC_EXCLUDE_FLAG\tLISTINGRECORDCERTIFIEDTHROUGH"
    }

    #[test]
    fn resolve_columns_finds_standard_header() {
        let idx = resolve_columns(sample_header()).unwrap();
        assert_eq!(idx.productndc, 1);
        assert_eq!(idx.producttypename, 2);
        assert_eq!(idx.proprietaryname, 3);
        assert_eq!(idx.nonproprietaryname, 5);
        assert_eq!(idx.ndc_exclude_flag, 18);
    }

    #[test]
    fn resolve_columns_is_case_insensitive() {
        let header = "productid\tproductndc\tproducttypename\tproprietaryname\tproprietarynamesuffix\t\
                      nonproprietaryname\tdosageformname\troutename\tstartmarketingdate\tendmarketingdate\t\
                      marketingcategoryname\tapplicationnumber\tlabelername\tsubstancename\t\
                      active_numerator_strength\tactive_ingred_unit\tpharm_classes\tdeaschedule\t\
                      ndc_exclude_flag\tlistingrecordcertifiedthrough";
        let idx = resolve_columns(header).unwrap();
        assert_eq!(idx.productndc, 1);
    }

    #[test]
    fn resolve_columns_missing_required_column_returns_error() {
        let header = "PRODUCTID\tPRODUCTNDC\tPROPRIETARYNAME"; // missing PRODUCTTYPENAME etc.
        let result = resolve_columns(header);
        assert!(result.is_err());
    }

    // ── parse_product_txt ─────────────────────────────────────────────────────

    fn sample_product_txt() -> String {
        format!(
            "{header}\n\
             0_0002-1200_001\t0002-1200\tHUMAN PRESCRIPTION DRUG\tProzac\t\tfluoxetine hydrochloride\tCAPSULE\tORAL\t19870101\t\tNDA\tNDA018936\tEli Lilly\tfluoxetine hydrochloride\t10\tmg/1\t\t\t\t\n\
             0_0002-3233_001\t0002-3233\tHUMAN PRESCRIPTION DRUG\tHumulin N\t\tinsulin isophane\tINJECTION\tSUBCUTANEOUS\t19821001\t\tNDA\tNDA018781\tEli Lilly\tinsulin isophane, human\t100\t[iU]/mL\t\t\t\t\n\
             0_0067-2000_001\t0067-2000\tHUMAN OTC DRUG\tTylenol\t\tacetaminophen\tTABLET\tORAL\t19800101\t\tOTC monograph final\t\tJohnson & Johnson\tacetaminophen\t500\tmg/1\t\t\t\t\n\
             0_9999-0001_001\t9999-0001\tHUMAN PRESCRIPTION DRUG\tExcluded Drug\t\texcluded\tCAPSULE\tORAL\t20000101\t\tNDA\tNDA999999\tAcme\texcluded\t10\tmg/1\t\t\tE\t\n",
            header = sample_header()
        )
    }

    #[test]
    fn parse_skips_excluded_products() {
        let (products, errors) = parse_product_txt(&sample_product_txt()).unwrap();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        // 3 active + 1 excluded → 3
        assert_eq!(products.len(), 3);
        assert!(!products.iter().any(|p| p.code == "9999-0001"));
    }

    #[test]
    fn parse_display_combines_brand_and_generic() {
        let (products, _) = parse_product_txt(&sample_product_txt()).unwrap();
        let prozac = products.iter().find(|p| p.code == "0002-1200").unwrap();
        assert_eq!(prozac.display, "Prozac (fluoxetine hydrochloride)");
    }

    #[test]
    fn parse_product_type_grouper_is_populated() {
        let (products, _) = parse_product_txt(&sample_product_txt()).unwrap();
        let otc = products.iter().find(|p| p.code == "0067-2000").unwrap();
        assert_eq!(otc.product_type, "HUMAN OTC DRUG");
    }

    #[test]
    fn parse_empty_input_returns_error() {
        let result = parse_product_txt("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_required_column_returns_error() {
        let bad = "PRODUCTID\tPRODUCTNDC\n0001\t1234-5678\n";
        let result = parse_product_txt(bad);
        assert!(result.is_err());
    }

    // ── Integration (SQLite) ──────────────────────────────────────────────────

    #[cfg(feature = "sqlite")]
    mod integration {
        use super::*;
        use crate::backends::SqliteTerminologyBackend;
        use std::io::Write;

        fn count(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
            pool.get()
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }

        fn make_txt_file() -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
            f.write_all(sample_product_txt().as_bytes()).unwrap();
            f
        }

        fn make_zip_file() -> tempfile::NamedTempFile {
            let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
            {
                let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
                zip.start_file("product.txt", zip::write::FileOptions::default())
                    .unwrap();
                zip.write_all(sample_product_txt().as_bytes()).unwrap();
                zip.finish().unwrap();
            }
            tmp
        }

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            let stats = import_ndc(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3); // 3 active products
            assert_eq!(count(backend.pool(), "code_systems"), 0);
            assert_eq!(count(backend.pool(), "concepts"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            let stats = import_ndc(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // virtual root + 2 product-type groupers + 3 products = 6
            assert_eq!(count(backend.pool(), "concepts"), 6);
            // 2 grouper edges (root→type) + 3 product edges (type→product) = 5
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 5);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            import_ndc(backend.pool(), f.path(), 500, false).unwrap();
            import_ndc(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 6);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 5);
        }

        #[test]
        fn import_from_zip() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_zip_file();
            let stats = import_ndc(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "concepts"), 6);
        }

        #[test]
        fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            let stats = import_ndc(backend.pool(), f.path(), 1, false).unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "concepts"), 6);
        }

        #[test]
        fn missing_file_returns_error() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let result = import_ndc(
                backend.pool(),
                Path::new("/nonexistent/product.txt"),
                500,
                false,
            );
            assert!(result.is_err());
        }

        #[test]
        fn lookup_known_code() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            import_ndc(backend.pool(), f.path(), 500, false).unwrap();

            let conn = backend.pool().get().unwrap();
            let display: String = conn
                .query_row(
                    "SELECT display FROM concepts WHERE code = ?1",
                    rusqlite::params!["0002-1200"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(display, "Prozac (fluoxetine hydrochloride)");
        }

        #[test]
        fn excluded_product_not_in_db() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file();
            import_ndc(backend.pool(), f.path(), 500, false).unwrap();

            let conn = backend.pool().get().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM concepts WHERE code = ?1",
                    rusqlite::params!["9999-0001"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "excluded product must not be imported");
        }
    }
}
