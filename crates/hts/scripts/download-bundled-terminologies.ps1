<#
.SYNOPSIS
    Downloads every terminology listed as "Bundled" in crates/hts/README.md.

.DESCRIPTION
    Versions are pinned below — bump them when refreshing. Each block links
    to the upstream landing page so future maintainers can find the next
    version. Keep this script in sync with its Bash counterpart,
    download-bundled-terminologies.sh.

.PARAMETER OutputDir
    Directory to download into. Defaults to .\terminology-data.

.EXAMPLE
    .\download-bundled-terminologies.ps1
    .\download-bundled-terminologies.ps1 -OutputDir D:\hts\terminology

.NOTES
    After download, import with e.g.:
        hts import .\terminology-data\hl7.terminology.r4-7.1.0.tgz
        hts import .\terminology-data\icd10cm-table-and-index-2026.zip

    Licensing: none of these files require registration, but each carries
    its own license (see the README "Terminology Support" section). Make
    sure you comply with attribution requirements before redistributing.
#>

[CmdletBinding()]
param(
    [string]$OutputDir = "./terminology-data"
)

$ErrorActionPreference = 'Stop'
# Suppressing the progress bar speeds up Invoke-WebRequest by orders of magnitude.
$ProgressPreference = 'SilentlyContinue'

if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
}

# ---------- pinned versions ----------------------------------------------------
$ThoVersion     = '7.1.0'       # https://terminology.hl7.org/en/downloads.html
$Icd10CmYear    = '2026'        # https://www.cdc.gov/nchs/icd/icd-10-cm/files.html (FY, effective Oct 1 prior year)
$Icd9CmVersion  = 'v32'         # https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles (retired 2015, final release)
$UcumVersion    = 'v2.2'        # https://github.com/ucum-org/ucum/releases
$NcitVersion    = '26.03e'      # https://evs.nci.nih.gov/evs-download/thesaurus-downloads
$MeshYear       = '2026'        # https://www.nlm.nih.gov/databases/download/mesh.html
$NuccVersion    = '251'         # https://www.nucc.org/index.php/code-sets-mainmenu-41/provider-taxonomy-mainmenu-40/csv-mainmenu-57 (25.1 == filename suffix 251)
# NDC has no explicit version — FDA publishes a rolling "current" ZIP.
# We use a stable filename so refreshes overwrite in place; git history
# records when the checked-in copy was last refreshed.
$NdcTag         = 'current'

# Commonly-used FHIR IG packages (latest versions). These bring in large
# sets of ValueSets and CodeSystems that downstream implementers frequently
# need alongside THO. Check packages.fhir.org/<name> for newer releases
# when refreshing.
$R4CoreVersion   = '4.0.1'      # https://packages.fhir.org/hl7.fhir.r4.core (R4 is frozen at 4.0.1)
$UsCoreVersion   = '8.0.1'      # https://packages.fhir.org/hl7.fhir.us.core
$VsacVersion     = '0.17.0'     # https://packages.fhir.org/us.nlm.vsac
$UvIpsVersion    = '2.0.0'      # https://packages.fhir.org/hl7.fhir.uv.ips
$PhinvadsVersion = '0.12.0'     # https://packages.fhir.org/us.cdc.phinvads

# ---------- helpers ------------------------------------------------------------
$Results = [System.Collections.Generic.List[string]]::new()

function Invoke-Download {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$OutFile
    )

    $path = Join-Path $OutputDir $OutFile
    Write-Host ""
    Write-Host "=== $Name ==="
    Write-Host "  url: $Url"
    Write-Host "  out: $path"

    if (Test-Path $path) {
        Write-Host "  [skip] already exists"
        $Results.Add("SKIP  $Name -> $OutFile") | Out-Null
        return
    }

    $tmp = "$path.part"
    $attempts = 0
    $maxAttempts = 3
    while ($attempts -lt $maxAttempts) {
        $attempts++
        try {
            Invoke-WebRequest -Uri $Url -OutFile $tmp -UseBasicParsing -MaximumRedirection 10
            Move-Item -Force $tmp $path
            $Results.Add("OK    $Name -> $OutFile") | Out-Null
            return
        } catch {
            Write-Warning "  attempt $attempts failed: $($_.Exception.Message)"
            if (Test-Path $tmp) { Remove-Item -Force $tmp }
            if ($attempts -lt $maxAttempts) {
                Start-Sleep -Seconds 2
            }
        }
    }
    $Results.Add("FAIL  $Name -> $Url") | Out-Null
}

# ---------- 1. HL7 FHIR Core (THO) --------------------------------------------
# Landing page: https://terminology.hl7.org/en/downloads.html
# Direct tarballs served by the FHIR package registry. Master is FHIR-R5-native
# canonical content; .r4/.r5 are version-specific repackagings for R4/R5 tooling.
Invoke-Download 'HL7 THO (master)' `
    "https://packages.fhir.org/hl7.terminology/$ThoVersion" `
    "hl7.terminology-$ThoVersion.tgz"

Invoke-Download 'HL7 THO (R4)' `
    "https://packages.fhir.org/hl7.terminology.r4/$ThoVersion" `
    "hl7.terminology.r4-$ThoVersion.tgz"

Invoke-Download 'HL7 THO (R5)' `
    "https://packages.fhir.org/hl7.terminology.r5/$ThoVersion" `
    "hl7.terminology.r5-$ThoVersion.tgz"

# ---------- 1b. Common FHIR IG packages ---------------------------------------
# Widely-used FHIR IGs whose ValueSets and CodeSystems reference THO and the
# licensed terminologies. All are freely redistributable under the HL7 FHIR
# License (with attribution).
Invoke-Download "HL7 FHIR R4 core $R4CoreVersion" `
    "https://packages.fhir.org/hl7.fhir.r4.core/$R4CoreVersion" `
    "hl7.fhir.r4.core-$R4CoreVersion.tgz"

Invoke-Download "HL7 FHIR US Core $UsCoreVersion" `
    "https://packages.fhir.org/hl7.fhir.us.core/$UsCoreVersion" `
    "hl7.fhir.us.core-$UsCoreVersion.tgz"

Invoke-Download "VSAC $VsacVersion" `
    "https://packages.fhir.org/us.nlm.vsac/$VsacVersion" `
    "us.nlm.vsac-$VsacVersion.tgz"

Invoke-Download "HL7 FHIR UV IPS $UvIpsVersion" `
    "https://packages.fhir.org/hl7.fhir.uv.ips/$UvIpsVersion" `
    "hl7.fhir.uv.ips-$UvIpsVersion.tgz"

Invoke-Download "CDC PHIN VADS $PhinvadsVersion" `
    "https://packages.fhir.org/us.cdc.phinvads/$PhinvadsVersion" `
    "us.cdc.phinvads-$PhinvadsVersion.tgz"

# ---------- 2. ICD-10-CM -------------------------------------------------------
# Landing page: https://www.cdc.gov/nchs/icd/icd-10-cm/files.html
# The "table and index" ZIP contains icd10cm_tabular_YYYY.xml.
Invoke-Download "ICD-10-CM FY$Icd10CmYear" `
    "https://ftp.cdc.gov/pub/Health_Statistics/NCHS/Publications/ICD10CM/$Icd10CmYear/icd10cm-table%20and%20index-$Icd10CmYear.zip" `
    "icd10cm-table-and-index-$Icd10CmYear.zip"

# ---------- 3. ICD-9-CM (retired 2015, final release) -------------------------
# Landing page: https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles
# ZIP contains CMS32_DESC_LONG_DX.txt and related pipe-delimited files.
Invoke-Download "ICD-9-CM $Icd9CmVersion" `
    "https://www.cms.gov/Medicare/Coding/ICD9ProviderDiagnosticCodes/Downloads/ICD-9-CM-$Icd9CmVersion-master-descriptions.zip" `
    "ICD-9-CM-$Icd9CmVersion-master-descriptions.zip"

# ---------- 4. UCUM ------------------------------------------------------------
# Already bundled inside the HL7 THO tarball (see step 1). Importing THO
# covers this — no separate download needed. Left in the script as a
# reference for customers who want to import UCUM without THO; uncomment
# and delete the THO UCUM entries to use it standalone.
Write-Host ""
Write-Host "=== UCUM ==="
Write-Host "  [bundled] included in hl7.terminology.*.tgz — no separate file"
$Results.Add('SKIP  UCUM -> bundled in THO') | Out-Null
# Landing page: https://github.com/ucum-org/ucum/releases
# Invoke-Download "UCUM $UcumVersion" `
#     "https://raw.githubusercontent.com/ucum-org/ucum/$UcumVersion/ucum-essence.xml" `
#     "ucum-essence-$UcumVersion.xml"

# ---------- 5. NCI Thesaurus (NCIt) -------------------------------------------
# Landing page: https://evs.nci.nih.gov/evs-download/thesaurus-downloads
Invoke-Download "NCIt $NcitVersion" `
    "https://evs.nci.nih.gov/ftp1/NCI_Thesaurus/Thesaurus_$NcitVersion.FLAT.zip" `
    "Thesaurus_$NcitVersion.FLAT.zip"

# ---------- 6. MeSH ------------------------------------------------------------
# Landing page: https://www.nlm.nih.gov/databases/download/mesh.html
# We grab the compressed descriptor file; the uncompressed desc$MeshYear.xml
# is ~300 MB which exceeds GitHub's 100 MB per-file push limit. The MeSH
# importer accepts the .zip directly. Qualifiers and supplementaries are
# available at the same URL stem if you need them:
#   https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/qual$MeshYear.zip
#   https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/supp$MeshYear.zip
Invoke-Download "MeSH $MeshYear" `
    "https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/desc$MeshYear.zip" `
    "desc$MeshYear.zip"

# ---------- 7. DICOM -----------------------------------------------------------
# Landing page: https://www.dicomstandard.org/current
# DICOM Part 16 is published as PDF/HTML/DOCX/XML only — there is no official
# CSV export of the code tables. The hts dicom importer expects a CSV that
# you generate yourself from Part 16 XML. Skipping the automated download;
# see the README "DICOM" section for guidance.
Write-Host ""
Write-Host "=== DICOM ==="
Write-Host "  [manual] no direct CSV download — see crates/hts/README.md#dicom"
$Results.Add("SKIP  DICOM -> manual CSV export required") | Out-Null

# ---------- 8. HL7 v2 tables ---------------------------------------------------
# Already bundled inside the HL7 THO tarball (see step 1). Importing THO
# covers this — no separate download needed.
Write-Host ""
Write-Host "=== HL7 v2 tables ==="
Write-Host "  [bundled] included in hl7.terminology.*.tgz — no separate file"
$Results.Add('SKIP  HL7 v2 tables -> bundled in THO') | Out-Null

# ---------- 9. NUCC Provider Taxonomy -----------------------------------------
# Landing page: https://www.nucc.org/index.php/code-sets-mainmenu-41/provider-taxonomy-mainmenu-40/csv-mainmenu-57
Invoke-Download "NUCC Provider Taxonomy $NuccVersion" `
    "https://www.nucc.org/images/stories/CSV/nucc_taxonomy_$NuccVersion.csv" `
    "nucc_taxonomy_$NuccVersion.csv"

# ---------- 10. NDC ------------------------------------------------------------
# Landing page: https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory
# FDA only publishes a rolling "current" ZIP — no version in the URL. The
# NdcTag suffix above records when this copy was fetched.
Invoke-Download "NDC (FDA, $NdcTag)" `
    'https://www.accessdata.fda.gov/cder/ndctext.zip' `
    "ndctext-$NdcTag.zip"

# ---------- summary ------------------------------------------------------------
Write-Host ""
Write-Host "============================================================"
Write-Host "Download summary ($OutputDir)"
Write-Host "============================================================"
foreach ($line in $Results) {
    Write-Host "  $line"
}

if ($Results | Where-Object { $_ -like 'FAIL*' }) {
    exit 1
}
