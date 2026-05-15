//! FDA National Drug Code (NDC) Directory importer.
//!
//! Parses the FDA NDC Text Files distribution (`product.txt`) and imports all
//! active drug product codes, display names, and a product-type hierarchy into
//! the HTS normalized schema.
//!
//! # No license required
//!
//! The NDC Directory is published by the U.S. Food and Drug Administration (FDA),
//! a US federal government work in the public domain.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const NDC_URL: &str = "http://hl7.org/fhir/sid/ndc";
const NDC_ID: &str = "ndc";
const NDC_NAME: &str = "NDC";
const NDC_TITLE: &str = "NDC (National Drug Code Directory)";
const NDC_VERSION: &str = "current";
const ROOT_CODE: &str = "NDC";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct NdcProduct {
    code: String,
    display: String,
    product_type: String,
}

struct ColIdx {
    productndc: usize,
    proprietaryname: usize,
    nonproprietaryname: usize,
    producttypename: usize,
    ndc_exclude_flag: usize,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import the FDA NDC Directory (`product.txt`) through the given backend.
pub async fn import_ndc(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (products, errors) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<NdcProduct>, Vec<String>), HtsError> {
            let text = read_product_txt(&path_owned)?;
            parse_product_txt(&text)
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("NDC parser panicked: {e}")))??;

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

    let meta = CodeSystemMeta {
        id: NDC_ID,
        url: NDC_URL,
        version: Some(NDC_VERSION),
        name: Some(NDC_NAME),
        title: Some(NDC_TITLE),
        status: "active",
        content: "complete",
    };

    // Collect unique product-type grouper names.
    let product_types: BTreeSet<String> = products
        .iter()
        .filter(|p| !p.product_type.is_empty())
        .map(|p| p.product_type.clone())
        .collect();

    // Seed bundle: virtual root + grouper concepts.
    let root = BuilderConcept {
        code: ROOT_CODE,
        display: Some(NDC_TITLE),
        definition: Some("header"),
        ..Default::default()
    };
    let mut seed_concepts: Vec<BuilderConcept<'_>> = Vec::with_capacity(1 + product_types.len());
    seed_concepts.push(root);
    for pt in &product_types {
        seed_concepts.push(BuilderConcept {
            code: pt.as_str(),
            display: Some(pt.as_str()),
            definition: Some("header"),
            parent_code: Some(ROOT_CODE),
            ..Default::default()
        });
    }
    let seed = build_code_system_bundle(&meta, &seed_concepts);
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let total = products.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in products.chunks(batch_size).enumerate() {
        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .map(|p| BuilderConcept {
                code: &p.code,
                display: Some(p.display.as_str()).filter(|s| !s.is_empty()),
                parent_code: Some(if p.product_type.is_empty() {
                    ROOT_CODE
                } else {
                    p.product_type.as_str()
                }),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[ndc] batch {}/{total_batches} — +{} products (total: {})",
            batch_idx + 1,
            batch.len(),
            ((batch_idx + 1) * batch_size).min(total)
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

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

fn parse_product_txt(text: &str) -> Result<(Vec<NdcProduct>, Vec<String>), HtsError> {
    let mut lines = BufReader::new(text.as_bytes()).lines().enumerate();

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        let header = "PRODUCTID\tPRODUCTNDC\tPROPRIETARYNAME";
        let result = resolve_columns(header);
        assert!(result.is_err());
    }

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

    #[cfg(feature = "sqlite")]
    mod integration {
        use super::*;
        use crate::backends::SqliteTerminologyBackend;
        use std::io::Write;

        fn count(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
            backend
                .pool()
                .get()
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

        #[tokio::test]
        async fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            let stats = import_ndc(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "code_systems"), 0);
            assert_eq!(count(&backend, "concepts"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            let stats = import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "concepts"), 6);
            assert_eq!(count(&backend, "concept_hierarchy"), 5);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 6);
            assert_eq!(count(&backend, "concept_hierarchy"), 5);
        }

        #[tokio::test]
        async fn import_from_zip() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_zip_file();
            let stats = import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "concepts"), 6);
        }

        #[tokio::test]
        async fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            let stats = import_ndc(&backend, &ctx, f.path(), 1, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "concepts"), 6);
        }

        #[tokio::test]
        async fn missing_file_returns_error() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let result = import_ndc(
                &backend,
                &ctx,
                Path::new("/nonexistent/product.txt"),
                500,
                false,
            )
            .await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn lookup_known_code() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();

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

        #[tokio::test]
        async fn excluded_product_not_in_db() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file();
            import_ndc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();

            let conn = backend.pool().get().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM concepts WHERE code = ?1",
                    rusqlite::params!["9999-0001"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        }
    }
}
