#!/usr/bin/env bash
#
# download-bundled-terminologies.sh
#
# Downloads every terminology listed as "Bundled" in crates/hts/README.md.
# Versions are pinned below — bump them when refreshing. Each block links to
# the upstream landing page so future maintainers can find the next version.
#
# Usage:
#   ./download-bundled-terminologies.sh [OUTPUT_DIR]
#
#   OUTPUT_DIR defaults to ./terminology-data
#
# After download, import with e.g.:
#   hts import ./terminology-data/hl7.terminology.r4-7.1.0.tgz
#   hts import ./terminology-data/icd10cm-table-and-index-2026.zip
#   ...
#
# Licensing: none of these files require registration, but each carries its
# own license (see the README "Terminology Support" section). Make sure you
# comply with attribution requirements before redistributing.

set -euo pipefail

OUTPUT_DIR="${1:-./terminology-data}"
mkdir -p "$OUTPUT_DIR"

# ---------- pinned versions ----------------------------------------------------
THO_VERSION="7.1.0"           # https://terminology.hl7.org/en/downloads.html
ICD10CM_YEAR="2026"           # https://www.cdc.gov/nchs/icd/icd-10-cm/files.html (FY, effective Oct 1 prior year)
ICD9CM_VERSION="v32"          # https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles (retired 2015, final release)
UCUM_VERSION="v2.2"           # https://github.com/ucum-org/ucum/releases
NCIT_VERSION="26.03e"         # https://evs.nci.nih.gov/evs-download/thesaurus-downloads
MESH_YEAR="2026"              # https://www.nlm.nih.gov/databases/download/mesh.html
NUCC_VERSION="251"            # https://www.nucc.org/index.php/code-sets-mainmenu-41/provider-taxonomy-mainmenu-40/csv-mainmenu-57 (25.1 == filename suffix 251)
# NDC has no explicit version — FDA publishes a rolling "current" ZIP.
# We use a stable filename so refreshes overwrite in place; git history
# records when the checked-in copy was last refreshed.
NDC_TAG="current"

# ---------- helpers ------------------------------------------------------------
RESULTS=()

download() {
    local name="$1" url="$2" out="$3"
    local path="$OUTPUT_DIR/$out"
    echo
    echo "=== $name ==="
    echo "  url: $url"
    echo "  out: $path"
    if [[ -f "$path" ]]; then
        echo "  [skip] already exists"
        RESULTS+=("SKIP  $name -> $out")
        return 0
    fi
    if curl --fail --location --progress-bar --retry 3 --retry-delay 2 \
            --output "$path.part" "$url"; then
        mv "$path.part" "$path"
        RESULTS+=("OK    $name -> $out")
    else
        rm -f "$path.part"
        echo "  [FAIL] download failed"
        RESULTS+=("FAIL  $name -> $url")
        return 1
    fi
}

try_download() {
    # Non-fatal wrapper — lets the script continue past a broken source.
    download "$@" || true
}

# ---------- 1. HL7 FHIR Core (THO) --------------------------------------------
# Landing page: https://terminology.hl7.org/en/downloads.html
# Direct tarballs served by the FHIR package registry. Master is FHIR-R5-native
# canonical content; .r4/.r5 are version-specific repackagings for R4/R5 tooling.
try_download "HL7 THO (master)" \
    "https://packages.fhir.org/hl7.terminology/${THO_VERSION}" \
    "hl7.terminology-${THO_VERSION}.tgz"

try_download "HL7 THO (R4)" \
    "https://packages.fhir.org/hl7.terminology.r4/${THO_VERSION}" \
    "hl7.terminology.r4-${THO_VERSION}.tgz"

try_download "HL7 THO (R5)" \
    "https://packages.fhir.org/hl7.terminology.r5/${THO_VERSION}" \
    "hl7.terminology.r5-${THO_VERSION}.tgz"

# ---------- 2. ICD-10-CM (FY26) -----------------------------------------------
# Landing page: https://www.cdc.gov/nchs/icd/icd-10-cm/files.html
# The "table and index" ZIP contains icd10cm_tabular_YYYY.xml.
try_download "ICD-10-CM FY${ICD10CM_YEAR}" \
    "https://ftp.cdc.gov/pub/Health_Statistics/NCHS/Publications/ICD10CM/${ICD10CM_YEAR}/icd10cm-table%20and%20index-${ICD10CM_YEAR}.zip" \
    "icd10cm-table-and-index-${ICD10CM_YEAR}.zip"

# ---------- 3. ICD-9-CM (retired 2015, final release) -------------------------
# Landing page: https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles
# ZIP contains CMS32_DESC_LONG_DX.txt and related pipe-delimited files.
try_download "ICD-9-CM ${ICD9CM_VERSION}" \
    "https://www.cms.gov/Medicare/Coding/ICD9ProviderDiagnosticCodes/Downloads/ICD-9-CM-${ICD9CM_VERSION}-master-descriptions.zip" \
    "ICD-9-CM-${ICD9CM_VERSION}-master-descriptions.zip"

# ---------- 4. UCUM ------------------------------------------------------------
# Already bundled inside the HL7 THO tarball (see step 1). Importing THO
# covers this — no separate download needed. Left in the script as a
# reference for customers who want to import UCUM without THO; uncomment
# and delete the THO UCUM entries to use it standalone.
echo
echo "=== UCUM ==="
echo "  [bundled] included in hl7.terminology.*.tgz — no separate file"
RESULTS+=("SKIP  UCUM -> bundled in THO")
# Landing page: https://github.com/ucum-org/ucum/releases
# try_download "UCUM ${UCUM_VERSION}" \
#     "https://raw.githubusercontent.com/ucum-org/ucum/${UCUM_VERSION}/ucum-essence.xml" \
#     "ucum-essence-${UCUM_VERSION}.xml"

# ---------- 5. NCI Thesaurus (NCIt) -------------------------------------------
# Landing page: https://evs.nci.nih.gov/evs-download/thesaurus-downloads
try_download "NCIt ${NCIT_VERSION}" \
    "https://evs.nci.nih.gov/ftp1/NCI_Thesaurus/Thesaurus_${NCIT_VERSION}.FLAT.zip" \
    "Thesaurus_${NCIT_VERSION}.FLAT.zip"

# ---------- 6. MeSH ------------------------------------------------------------
# Landing page: https://www.nlm.nih.gov/databases/download/mesh.html
# We grab the compressed descriptor file; the uncompressed desc${YEAR}.xml is
# ~300 MB which exceeds GitHub's 100 MB per-file push limit. The MeSH importer
# accepts the .zip directly. Qualifiers and supplementaries are available at
# the same URL stem if you need them:
#   https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/qual${MESH_YEAR}.zip
#   https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/supp${MESH_YEAR}.zip
try_download "MeSH ${MESH_YEAR}" \
    "https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/desc${MESH_YEAR}.zip" \
    "desc${MESH_YEAR}.zip"

# ---------- 7. DICOM -----------------------------------------------------------
# Landing page: https://www.dicomstandard.org/current
# DICOM Part 16 is published as PDF/HTML/DOCX/XML only — there is no official
# CSV export of the code tables. The hts dicom importer expects a CSV that
# you generate yourself from Part 16 XML. Skipping the automated download;
# see the README "DICOM" section for guidance.
echo
echo "=== DICOM ==="
echo "  [manual] no direct CSV download — see crates/hts/README.md#dicom"
RESULTS+=("SKIP  DICOM -> manual CSV export required")

# ---------- 8. HL7 v2 tables ---------------------------------------------------
# Already bundled inside the HL7 THO tarball (see step 1). Importing THO
# covers this — no separate download needed.
echo
echo "=== HL7 v2 tables ==="
echo "  [bundled] included in hl7.terminology.*.tgz — no separate file"
RESULTS+=("SKIP  HL7 v2 tables -> bundled in THO")

# ---------- 9. NUCC Provider Taxonomy -----------------------------------------
# Landing page: https://www.nucc.org/index.php/code-sets-mainmenu-41/provider-taxonomy-mainmenu-40/csv-mainmenu-57
try_download "NUCC Provider Taxonomy ${NUCC_VERSION}" \
    "https://www.nucc.org/images/stories/CSV/nucc_taxonomy_${NUCC_VERSION}.csv" \
    "nucc_taxonomy_${NUCC_VERSION}.csv"

# ---------- 10. NDC ------------------------------------------------------------
# Landing page: https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory
# FDA only publishes a rolling "current" ZIP — no version in the URL. The
# NDC_TAG suffix above records when this copy was fetched.
try_download "NDC (FDA, ${NDC_TAG})" \
    "https://www.accessdata.fda.gov/cder/ndctext.zip" \
    "ndctext-${NDC_TAG}.zip"

# ---------- summary ------------------------------------------------------------
echo
echo "============================================================"
echo "Download summary ($OUTPUT_DIR)"
echo "============================================================"
for line in "${RESULTS[@]}"; do
    echo "  $line"
done

# Exit non-zero if anything actually failed (SKIP and OK are both fine).
if printf '%s\n' "${RESULTS[@]}" | grep -q '^FAIL'; then
    exit 1
fi
