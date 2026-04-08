# HTS Terminology Analysis

> **Purpose:** A mini-expert reference for the terminologies HTS targets. Every claim links to its source so you can verify and stay current. Use this to make decisions about what we can bundle, what requires customer licensing, and what to put in the README.

---

## Table of Contents

1. [The Landscape: What is a Terminology System?](#1-the-landscape-what-is-a-terminology-system)
2. [How Terminology Licensing Works in FHIR](#2-how-terminology-licensing-works-in-fhir)
3. [Our Target Terminology List — Detailed Analysis](#3-our-target-terminology-list--detailed-analysis)
   - [HL7 FHIR Core Terminology](#31-hl7-fhir-core-terminology)
   - [SNOMED CT](#32-snomed-ct)
   - [LOINC](#33-loinc)
   - [ICD-10-CM](#34-icd-10-cm)
   - [ICD-9-CM](#35-icd-9-cm)
   - [RxNorm](#36-rxnorm)
   - [CPT](#37-cpt-current-procedural-terminology)
   - [CVX](#38-cvx-vaccine-administered-codes)
   - [HCPCS](#39-hcpcs-healthcare-common-procedure-coding-system)
4. [Additional Terminologies on the World List (tx.fhir.org)](#4-additional-terminologies-on-the-world-list)
5. [Decision Matrix: What We Can Redistribute](#5-decision-matrix-what-we-can-redistribute)
6. [TODO: README Updates](#6-todo-readme-updates)

---

## 1. The Landscape: What is a Terminology System?

A **terminology system** (also called a *code system* or *vocabulary*) is a curated set of codes and their meanings used to represent clinical concepts in a computable way. Examples: `73211009` in SNOMED CT means "Diabetes mellitus"; `718-7` in LOINC means "Hemoglobin [Mass/volume] in Blood".

Terminology systems are the backbone of healthcare interoperability — without agreed codes, two systems cannot exchange meaning, only text.

The [HL7 FHIR Terminology Service specification](http://hl7.org/fhir/terminology-service.html) defines a standard HTTP API (`$lookup`, `$validate-code`, `$expand`, `$translate`, `$subsumes`, `$closure`) so any server can work with any terminology in a uniform way.

### The World Registry

The **FHIR Terminology Registry** at [tx.fhir.org/tx-reg](https://tx.fhir.org/tx-reg) lists publicly available FHIR terminology servers worldwide. As of early 2026, 15 servers are registered, including the HL7 reference server `tx.fhir.org`, national servers for Australia, Germany, Switzerland, Canada, Belgium, the Nordics, Estonia, and Chile. Each server exposes different terminologies depending on what their national affiliate has licensed.

Key insight: *different countries have access to different terminologies under different terms*. A US server can serve SNOMED CT for free; a non-member-country server might not legally be able to.

---

## 2. How Terminology Licensing Works in FHIR

The [FHIR license page](https://build.fhir.org/license.html) makes this explicit:

> "The FHIR specification itself is licensed under CC0 — Creative Commons No Rights Reserved. But code systems used within FHIR often require separate licenses. SNOMED CT requires separate licensing from IHTSDO. LOINC is available at no cost under its own license. DICOM, ICD, CPT: each requires consultation with respective governing organizations."

The [HL7 Terminology (THO) license](https://terminology.hl7.org/license.html) adds:

> "HL7 Terminology contains and references intellectual property owned by third parties ('Third Party IP'). Acceptance of these License Terms does not grant any rights with respect to Third Party IP. It is the sole responsibility of each organization deploying or testing this specification to ensure their implementations comply with licensing requirements of each Third Party IP."

**Bottom line for HTS:** HTS itself is open source. But the *data* you load into HTS may be under restrictive licenses. We must:
1. Not bundle restricted data in our distribution.
2. Tell users exactly which terminologies require a license and how to get one.
3. Only bundle data that is genuinely public domain or has an explicit redistribution-friendly license.

---

## 3. Our Target Terminology List — Detailed Analysis

---

### 3.1 HL7 FHIR Core Terminology

| Field | Value |
|-------|-------|
| **Authority** | HL7 International |
| **FHIR URI prefix** | `http://hl7.org/fhir/...` and `http://terminology.hl7.org/...` |
| **License** | [HL7 FHIR License](https://build.fhir.org/license.html) — the FHIR *specification* is CC0; the THO NPM *packages* are published under the HL7 FHIR License which permits free use and redistribution **with attribution** |
| **Cost** | Free |
| **Registration required** | No |
| **Can we redistribute** | **YES — with attribution** |
| **HTS import support** | YES (`hl7-npm` format) |
| **Download** | [terminology.hl7.org/en/downloads.html](https://terminology.hl7.org/en/downloads.html) |

**What's included:** All HL7-defined CodeSystems and ValueSets used in FHIR, including:
- Administrative codes (gender, marital status, encounter status, etc.)
- HL7 v2 and v3 vocabulary (e.g., `http://terminology.hl7.org/CodeSystem/v2-0001`)
- FHIR defined code systems (e.g., `http://hl7.org/fhir/observation-status`)
- HL7 NamingSystems and identifier types
- UCUM (Unified Code for Units of Measure) — included in THO packages
- CVX (Vaccine Administered Codes) — included in THO packages

**What's NOT included:** Third-party terminologies like SNOMED CT, LOINC, CPT — even when referenced by HL7 value sets. Those retain their own licenses.

**License note:** The FHIR specification document itself is CC0 (public domain, no attribution required). However, the THO NPM packages distributed at terminology.hl7.org are published under the HL7 FHIR License, which does specify attribution. When redistributing the packages (not just the spec), include attribution to HL7. The practical difference is minor since both allow full redistribution at no cost.

**Required attribution for THO packages:**
> "This product includes content from HL7 Terminology (THO). Copyright © Health Level Seven International. Licensed under the HL7 FHIR License."

**Steps to redistribute:**
1. Download the NPM package from [terminology.hl7.org/en/downloads.html](https://terminology.hl7.org/en/downloads.html)
2. Include the attribution statement above in your product documentation or `NOTICES` file
3. Import into HTS: `hts import ./hl7.terminology.r4-6.0.0.tgz`
4. No registration, no fee, no further approval required

**Source:** [FHIR License](https://build.fhir.org/license.html) · [THO License](https://terminology.hl7.org/license.html)

---

### 3.2 SNOMED CT

| Field | Value |
|-------|-------|
| **Full name** | Systematized Nomenclature of Medicine — Clinical Terms |
| **Authority** | [SNOMED International](https://www.snomed.org) (formerly IHTSDO) |
| **FHIR URI** | `http://snomed.info/sct` |
| **License** | [Affiliate License — see SNOMED International](https://www.snomed.org/licensing) |
| **Cost** | FREE (no monetary fee) in member countries and LDCs — but registration as an Affiliate or sub-licensee is still required regardless; paid in non-member countries |
| **Registration required** | YES — must register with your country's NRC or directly with SNOMED International |
| **Can we redistribute** | **NO** |
| **HTS import support** | YES (`snomed-rf2` format) |
| **Download** | [MLDS — Member Licensing and Distribution Service](https://mlds.ihtsdotools.org/) |

#### Who needs a license?

Anyone who *uses* SNOMED CT in a product or service must either:
- Be a SNOMED International Affiliate themselves, OR
- Be a sub-licensee of an Affiliate.

Software vendors that distribute SNOMED-enabled products must be Affiliates and must issue sub-licenses to their customers.

#### Cost model

| Scenario | Cost |
|----------|------|
| Use in a **member country** (US, UK, Australia, Canada, Germany, and ~46 others — **51 members total** as of 2025) | **Free** |
| Use in a **least-developed country** (World Bank LDC list) | **Free** |
| Qualifying research projects (any country) | **Free** |
| Use in a **non-member country** | **Paid** — fee based on use type and World Bank income classification |

**United States specifically:** Free via the NLM UMLS program. US users can download SNOMED CT through UMLS without charge. See [NLM SNOMED CT page](https://www.nlm.nih.gov/healthit/snomedct/index.html).

#### Why we can't redistribute

SNOMED CT's license model requires every user to have their own Affiliate License or sub-license. We cannot bundle the data in a public distribution because we cannot know whether each downloader of HTS is in a member country or has a valid license.

#### RF2 release types — choose Snapshot for most deployments

SNOMED CT is distributed in three RF2 release types. For HTS, the **Snapshot** release is almost always the right choice:

| Release type | Description | Typical compressed size |
|---|---|---|
| **Snapshot** | Current state of every component only — no history | ~1–2 GB compressed |
| **Full** | Complete history of all component states across all release dates | ~2–3 GB compressed, **5–8 GB uncompressed** |
| **Delta** | Only changes since the prior release | <100 MB |

Import the Snapshot release from your NRC unless you specifically need historical data. Use `--batch-size 200 --verbose` to monitor progress.

**Sources:** [SNOMED licensing](https://www.snomed.org/licensing) · [NLM SNOMED](https://www.nlm.nih.gov/healthit/snomedct/snomed_licensing.html) · [SNOMED vendor licensing guide](https://confluence.ihtsdotools.org/display/docvendor/7+licensing) · [NRC affiliate role](https://confluence.ihtsdotools.org/display/DOCNRCG/5.+The+role+of+NRCs+related+to+SNOMED+CT+licensing)

---

### 3.3 LOINC

| Field | Value |
|-------|-------|
| **Full name** | Logical Observation Identifiers Names and Codes |
| **Authority** | [Regenstrief Institute, Inc.](https://www.regenstrief.org) |
| **FHIR URI** | `http://loinc.org` |
| **License** | [LOINC License](https://loinc.org/kb/license/) — free with conditions |
| **Cost** | Free |
| **Registration required** | YES — free account at [loinc.org](https://loinc.org) required **to download from Regenstrief** |
| **Can we redistribute** | **CONDITIONAL — the LOINC License explicitly permits redistribution with attribution** (see below) |
| **HTS import support** | YES (`loinc` format) |
| **Download** | [loinc.org/downloads](https://loinc.org/downloads/) |

#### What the LOINC License allows

- Free commercial and non-commercial use
- Incorporation into software products and online services
- **Redistribution in products with attribution** — the LOINC License explicitly grants this right:
  > "This material contains content from LOINC (http://loinc.org). LOINC is copyright © Regenstrief Institute, Inc. and the Regenstrief LOINC Committee. Terms of Use, see https://loinc.org/license/."

#### What the LOINC License prohibits

- Modifying field names or content in LOINC core files (Group 1 artifacts)
- Creating derivative works that change LOINC codes or definitions without Regenstrief's written permission
- Using LOINC to develop or promulgate a *competing* standard for clinical observations

#### Why we currently do not bundle LOINC

Redistribution with attribution **is** legally permitted. We have chosen not to bundle LOINC for the following operational reasons — not because the license prohibits it:

1. **Currency:** LOINC is updated quarterly (March, June, September, December). A bundled copy would quickly become stale. We cannot commit to releasing HTS on the same cadence as Regenstrief. Sending users to download directly guarantees they get the latest version.
2. **Regenstrief tracking:** Regenstrief uses registrations to track the global deployment of LOINC. Redirecting users to register respects this intent.
3. **Attribution friction reduced:** If we bundle, we must ensure the attribution string is visible to end users in the right place. Redirecting users to loinc.org is simpler.

**Important clarification for users who receive LOINC through a properly attributed redistribution:** The registration requirement at loinc.org applies to *downloading from Regenstrief's site*. A user who receives LOINC via a licensed redistributor (with proper attribution in place) is not additionally required to register at loinc.org to use it. Registration at loinc.org is only needed if they want to download future updates directly from Regenstrief.

**Steps if a user needs LOINC:**
1. Create a free account at [loinc.org](https://loinc.org) and accept the LOINC License
2. Download the CSV ZIP from [loinc.org/downloads](https://loinc.org/downloads/)
3. Import: `hts import ./Loinc_2.78.zip`
4. Include the attribution string in any product documentation

**Sources:** [LOINC License](https://loinc.org/kb/license/) · [Regenstrief Institute](https://www.regenstrief.org/real-world-solutions/loinc/)

---

### 3.4 ICD-10-CM

| Field | Value |
|-------|-------|
| **Full name** | International Classification of Diseases, 10th Revision, Clinical Modification |
| **Authority** | U.S. CDC / NCHS (authorized by WHO to create the US clinical modification) |
| **FHIR URI** | `http://hl7.org/fhir/sid/icd-10-cm` |
| **License** | **US federal government work — Public Domain** |
| **Cost** | Free |
| **Registration required** | No |
| **Can we redistribute** | **YES — fully** |
| **HTS import support** | YES (`icd10-cm` format) |
| **Download** | [CDC ICD-10-CM files](https://www.cdc.gov/nchs/icd/icd-10-cm/files.html) · [CMS ICD-10](https://www.cms.gov/medicare/coding-billing/icd-10-codes) |

#### What is ICD-10-CM vs ICD-10?

The **WHO publishes ICD-10** (the international version) — that version has its own separate copyright ([WHO FAQ on ICD-10 licensing](https://cdn.who.int/media/docs/default-source/publishing-policies/copyright/who-faq-licensing-icd-10.pdf)). The WHO allows national adaptations.

**ICD-10-CM** is the US clinical modification, authored by the US National Center for Health Statistics (NCHS), a federal agency. Works of the US federal government are not subject to copyright under 17 U.S.C. § 105 and are therefore in the **public domain**. This covers ICD-10-CM only, not the underlying WHO ICD-10.

ICD-10-CM is updated annually (effective October 1). The FY2026 release (effective Oct 1, 2025) added 614 new codes, 28 deletions, and 38 revisions. CMS also releases minor quarterly updates between annual releases; verify current quarterly changes at the [CMS ICD-10 quarterly update page](https://www.cms.gov/medicare/coding-billing/icd-10-codes) before assuming a specific quarterly update's content.

**Steps to redistribute:**
1. Download `icd10cm_tabular_YYYY.xml` from [CDC ICD-10-CM files](https://www.cdc.gov/nchs/icd/icd-10-cm/files.html)
2. No registration, no attribution, no fee required
3. Import: `hts import ./icd10cm_tabular_2025.xml`
4. The file can be bundled directly in a distribution or shipped alongside HTS — no license text required

#### What about ICD-10-PCS?

ICD-10-PCS (Procedure Coding System) is also a US government work maintained by CMS and is similarly public domain. It uses 7-character alphanumeric codes for inpatient procedures. Currently not imported by HTS but could be added.

**Sources:** [CDC ICD-10-CM](https://www.cdc.gov/nchs/icd/icd-10-cm/index.html) · [CMS ICD-10](https://www.cms.gov/medicare/coding-billing/icd-10-codes) · [icd10data.com FY2026 notes](https://www.icd10data.com)

---

### 3.5 ICD-9-CM

| Field | Value |
|-------|-------|
| **Full name** | International Classification of Diseases, 9th Revision, Clinical Modification |
| **Authority** | US NCHS / CMS (retired) |
| **FHIR URI** | `http://hl7.org/fhir/sid/icd-9-cm` |
| **License** | Public Domain (US government work) |
| **Cost** | Free |
| **Status** | **RETIRED** — replaced by ICD-10-CM on October 1, 2015 |
| **Can we redistribute** | YES — public domain |
| **HTS import support** | **NOT YET** |
| **Download** | [CMS ICD-9-CM archive](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles) |

#### Should we support it?

ICD-9-CM is obsolete for current clinical workflows. Its value today is **historical data** — organizations migrating legacy EHR data or doing longitudinal research across the 2015 transition boundary need it. It is not a priority for new implementations.

**Decision: Defer.** Support is technically simple (same format as ICD-10-CM plus legacy codes) but demand is low. Add a `--format icd9-cm` flag when a customer requests it.

**Sources:** [CMS ICD-9-CM archive](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles)

---

### 3.6 RxNorm

| Field | Value |
|-------|-------|
| **Full name** | RxNorm (normalized drug names and identifiers) |
| **Authority** | U.S. National Library of Medicine (NLM) |
| **FHIR URI** | `http://www.nlm.nih.gov/research/umls/rxnorm` |
| **License** | [NLM/UMLS Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html) |
| **Cost** | Free (UMLS account required) |
| **Registration required** | YES — free UMLS account at [uts.nlm.nih.gov](https://uts.nlm.nih.gov) |
| **Can we redistribute** | **PARTIALLY — see below** |
| **HTS import support** | YES (`rxnorm` format — RRF files) |
| **Download** | [NLM RxNorm files](https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html) |

#### What RxNorm is

RxNorm provides normalized names and unique identifiers (RXCUIs) for generic and branded drugs available in the US. It solves the "same drug, 20 names" problem across pharmacy systems. Created by NLM, released monthly.

**Two tiers:**
1. **Current Prescribable Content** — a subset containing only currently prescribable drugs. No license required for use. Available via the free [RxNorm API](https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html).
2. **Full monthly release** — includes historical, retired, and branded content. Requires a free UMLS account and acceptance of the NLM Terms of Service.

#### Public domain status and Source Restriction Levels (SRLs)

RxNorm is not a uniform dataset — it aggregates content from multiple sources at different restriction levels:

| SRL | Meaning | Redistribution |
|-----|---------|---------------|
| SRL 0 | NLM-created content (RXCUIs, normalized names) | **Public domain** — US government work |
| SRL 1 | Openly available sources | Redistributable per source terms |
| SRL 2 | Academic/legacy restricted sources (e.g., BI98, CPM2003) | Restricted per Section 12.2 of UMLS License Agreement |
| SRL 3 | Restricted commercial sources (e.g., Micromedex) | **NOT redistributable** |
| SRL 4 | Most restricted commercial sources (e.g., First DataBank) | **NOT redistributable** |

The full RxNorm download is an **interleaved mix** of all SRL levels. You cannot naively redistribute the full RxNorm dataset because SRL 2/3/4 content from academic and commercial sources is embedded in the same files. Separating SRL 0 content requires filtering by the `SAB` and `SUPPRESS` columns in the RRF files.

**Practical consequence:** We cannot bundle or redistribute the full RxNorm monthly release. Users must download it themselves under their own UMLS account.

#### Redistribution rules (for SRL 0/1 content only — SRL 2/3/4 are not redistributable)

From the [NLM Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html):
- Redistribution is allowed with attribution to NLM
- Redistributors must either maintain the most current version OR clearly disclose that their copy may not be current
- Required attribution: *"This product uses publicly available data courtesy of the U.S. National Library of Medicine (NLM), National Institutes of Health, Department of Health and Human Services; NLM is not responsible for the product and does not endorse or recommend this or any other product."*

#### Why we can't bundle the full release

The full RxNorm download requires UMLS ToS acceptance and contains non-redistributable SRL 3/4 content. We redirect users to download directly.

**Sources:** [RxNorm overview](https://www.nlm.nih.gov/research/umls/rxnorm/overview.html) · [RxNorm terms of service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html) · [UMLS licensing](https://www.nlm.nih.gov/databases/umls.html)

---

### 3.7 CPT (Current Procedural Terminology)

| Field | Value |
|-------|-------|
| **Full name** | Current Procedural Terminology |
| **Authority** | [American Medical Association (AMA)](https://www.ama-assn.org) |
| **FHIR URI** | `http://www.ama-assn.org/go/cpt` |
| **License** | **Proprietary — AMA copyright, paid distribution license required** |
| **Cost** | Annual royalty fees (substantial; varies by product type and distribution volume) |
| **Registration required** | YES — must contract with AMA |
| **Can we redistribute** | **NO** |
| **HTS import support** | **NOT YET** |
| **Download** | Via AMA licensing agreement only |

#### What CPT is

CPT is the dominant procedure code set in the US, used for billing physician and outpatient services to Medicare, Medicaid, and commercial payers. It contains approximately 10,000 codes covering evaluation & management, surgery, radiology, pathology, medicine, and Category II/III codes.

CPT is NOT a government work — it is exclusively owned and copyrighted by the AMA. This makes it fundamentally different from ICD-10-CM or CVX.

#### License restrictions (strict)

From the [AMA CPT licensing FAQ](https://www.ama-assn.org/practice-management/cpt/cpt-licensing-frequently-asked-questions-faqs):
- Any use in a product, application, or service requires a license
- Distribution to third parties requires a *distribution license* with annual royalty payment
- Prohibited without license: copying for resale, transferring to unbound third parties, creating modified/derivative works, any commercial use
- AMA also publishes a [Standard CPT Distribution Pricing Schedule](https://compliance.ama-assn.org/hc/en-us/articles/15253095972247)
- There is a separate [CPT for AI licensing FAQ](https://www.ama-assn.org/practice-management/cpt/licensing-cpt-ai-faqs) for AI product use cases

#### What about HCPCS Level I?

HCPCS Level I **is** CPT. When CMS mandates HCPCS usage, providers use Level I (CPT) for physician services. The AMA license covers it.

#### Decision

CPT is the most expensive and restrictive terminology on our list. Supporting import would require users to have their own AMA distribution license. **Support is deferred** — when we implement it, the importer itself is not a licensing problem (we parse files the user provides), but we must include strong license warnings.

**Sources:** [AMA CPT licensing FAQs](https://www.ama-assn.org/practice-management/cpt/cpt-licensing-frequently-asked-questions-faqs) · [AMA CPT royalties & licensing news](https://www.ama-assn.org/topics/cpt-royalties-licenses) · [CMS AMA license agreement](https://www.cms.gov/license/ama)

---

### 3.8 CVX (Vaccine Administered Codes)

| Field | Value |
|-------|-------|
| **Full name** | HL7 Standard Code Set CVX — Vaccines Administered |
| **Authority** | U.S. CDC / NCIRD (Immunization Information Systems Support Branch) |
| **FHIR URI** | `http://hl7.org/fhir/sid/cvx` |
| **License** | **US government work — Public Domain** |
| **Cost** | Free |
| **Registration required** | No |
| **Can we redistribute** | **YES — fully** |
| **HTS import support** | Via HL7 NPM packages (CVX is published in THO); direct CSV import not yet implemented |
| **Download** | [CDC CVX table](https://www2a.cdc.gov/vaccines/iis/iisstandards/vaccines.asp?rpt=cvx) · Also in [HL7 THO](https://terminology.hl7.org/CodeSystem-CVX.html) |

#### What CVX is

CVX is a numeric code set for vaccine products, used in immunization records and HL7 messaging (v2.x and FHIR). Maintained by CDC's NCIRD and updated regularly. Includes both active and historical/inactive vaccine codes to support historical immunization records.

A companion code set, **MVX**, identifies vaccine manufacturers using alphabetic codes.

#### Public domain

CVX is produced by CDC, a US federal agency, and is published without copyright restrictions on a public CDC website with no licensing terms. This is consistent with the US federal government works doctrine. HL7 THO includes CVX in its published packages (also freely redistributable with attribution). Importing the HL7 THO NPM package automatically brings in CVX — no separate import step is needed.

**Sources:** [CDC CVX](https://www2a.cdc.gov/vaccines/iis/iisstandards/vaccines.asp?rpt=cvx) · [CDC IIS code sets](https://www.cdc.gov/iis/code-sets/index.html) · [NLM CVX source info](https://www.nlm.nih.gov/research/umls/rxnorm/sourcereleasedocs/cvx.html) · [HL7 THO CVX](https://terminology.hl7.org/CodeSystem-CVX.html)

---

### 3.9 HCPCS (Healthcare Common Procedure Coding System)

HCPCS has two levels with completely different licensing situations:

#### HCPCS Level I = CPT (see §3.7)

Level I is simply the CPT code set. It inherits all AMA licensing restrictions. Every reference to "HCPCS" in a billing context that involves physician services uses CPT codes.

#### HCPCS Level II — CMS Government Codes

| Field | Value |
|-------|-------|
| **Full name** | HCPCS Level II — alphanumeric codes for non-physician services |
| **Authority** | U.S. CMS (Centers for Medicare & Medicaid Services) |
| **FHIR URI** | `http://www.cms.gov/Medicare/Coding/HCPCSReleaseCodeSets` |
| **License** | **US government work — Public Domain** (with a caveat for D-codes — see below) |
| **Cost** | Free |
| **Registration required** | No |
| **Can we redistribute** | **YES for A–V codes; D-codes are legally ambiguous — see caveat** |
| **HTS import support** | **NOT YET** |
| **Download** | [CMS HCPCS quarterly update](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system/quarterly-update) |

#### What Level II covers

HCPCS Level II codes begin with a letter (A–V) followed by 4 digits. They cover:
- Durable medical equipment (DME)
- Prosthetics and orthotics
- Ambulance services
- Certain drugs and biologicals (J-codes)
- Temporary national codes
- Dental codes (D-codes, maintained by ADA — see caveat below)

#### D-codes caveat

The dental D-codes within HCPCS Level II are derived from the ADA's Current Dental Terminology (CDT), which the ADA claims copyright over. CMS publishes D-codes as part of its government document (a federal publication), but the ADA's position is that its CDT copyright is not waived by CMS's publication. This creates an unresolved legal ambiguity:

- **CMS's position (implicit):** Publishing D-codes as part of a federal government document brings them into the public domain, or at minimum, CMS's government publication constitutes a lawful redistribution.
- **ADA's position:** CDT is ADA-copyrighted property regardless of CMS's publication; use of D-codes in a software product requires ADA licensing.

**Our decision:** For A–V codes (the majority of HCPCS Level II), redistribution is clearly public domain. For a complete HCPCS Level II file that includes D-codes, the conservative approach is to note the ambiguity and not assert the D-codes are unrestricted. When we implement HCPCS Level II import, include a warning that D-codes may require ADA CDT licensing depending on the user's legal interpretation.

#### Quarterly update cadence

CMS releases HCPCS Level II updates quarterly (January, April, July, October).

**Sources:** [CMS HCPCS general information](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system) · [HCPCS Level II coding process](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system/level-ii-coding-process) · [CMS vs AMA HCPCS overview](https://www.cms.gov/cms-guide-medical-technology-companies-and-other-interested-parties/coding/overview-coding-classification-systems)

---

## 4. Additional Terminologies on the World List

The [FHIR Terminology Registry (tx.fhir.org/tx-reg)](https://tx.fhir.org/tx-reg) and HL7's [External Terminologies page](https://confluence.hl7.org/spaces/TA/pages/16646186/External+Terminologies+-+Information) list dozens of systems used globally. Below are the most relevant beyond our initial target list:

| Terminology | Authority | License | Redistribution | Notes |
|-------------|-----------|---------|----------------|-------|
| **UCUM** (Unified Code for Units of Measure) | Regenstrief / HL7 | Free, permissive | YES | Used for physical quantities in every FHIR `Quantity` field — arguably the single most pervasive FHIR terminology after the HL7 core. Included in THO packages. |
| **NCI Thesaurus (NCIt)** | NCI / NIH | Free, public domain | YES | ~170k biomedical concepts spanning anatomy, genes, proteins, drugs, diseases, and more — not limited to oncology. NCI is a US federal agency. |
| **MeSH** (Medical Subject Headings) | NLM / NIH | Free, public domain | YES | NLM vocabulary. Used for PubMed indexing. |
| **NDC** (National Drug Code) | FDA | Free for codes; conditional for full dataset | Conditional | FDA publishes the NDC directory as a government work (public domain for the codes themselves). However, the associated drug product data includes proprietary manufacturer submissions with potential trademark concerns. The codes (11-digit NDC numbers) are public domain; bundling the full NDC product database requires care. Used in FHIR `Medication.code`. |
| **DICOM** | NEMA | Free; DICOM standard is publicly available | YES | Codes from the DICOM standard (URI: `http://dicom.nema.org/resources/ontology/DCM`) are used in FHIR imaging resources (`ImagingStudy`, `ImagingSelection`). NEMA makes the DICOM standard freely available; the code tables can be redistributed. |
| **MedDRA** | MSSO (ICH) | **Paid** | NO | Required for drug adverse event reporting (FDA, EMA). Annual license fee — contact MSSO for current pricing as fees vary by organization size and commercial/non-profit status; do not rely on any unverified estimate. |
| **ICD-10** (WHO international) | WHO | **Paid/restricted** | Conditional | WHO charges for translated versions; English adaptation requires contact. Distinct from ICD-10-CM. |
| **ICD-11** | WHO | CC BY-ND 3.0 IGO | **YES — with attribution, no modifications** | WHO published ICD-11 under a Creative Commons Attribution-NoDerivatives 3.0 IGO license. Redistribution with attribution is permitted; the ND clause prohibits modifications/derivatives. This is more open than previously thought — "❌ NO" was overly conservative. |
| **HL7 v2 tables** | HL7 | Freely redistributable (included in THO) | YES | Included in HL7 NPM packages. |
| **OMOP** vocabulary | OHDSI | Mixed (varies by source) | Mixed | OHDSI vocabularies include both open and licensed sources. |
| **NDFRT** | NLM | Public domain | YES | Drug terminology (being retired in favor of RxNorm + NCI). |
| **NUCC** (Provider taxonomy) | NUCC | Free | YES | Used in US provider directories. |

**Key takeaway:** The terminologies that are both *globally important* and *freely redistributable* are mostly US federal government works (ICD-10-CM, CVX, NDC codes, DICOM), HL7/FHIR-native (THO, which includes UCUM and CVX), or under permissive WHO licensing (ICD-11). The globally dominant clinical terminologies that are **not** freely redistributable are SNOMED CT (requires Affiliate license), LOINC (redistribution allowed with attribution but not bundled for operational reasons), RxNorm full release (UMLS account + SRL restrictions), CPT (paid AMA license), and MedDRA (paid MSSO license).

---

## 5. Decision Matrix: What We Can Redistribute

| Terminology | Redistribute in HTS? | Customer Action Required | Import Support | Priority |
|-------------|---------------------|--------------------------|----------------|----------|
| **HL7 FHIR Core (THO)** | ✅ YES — bundle with attribution | None | ✅ `hl7-npm` | High — do it |
| **ICD-10-CM** | ✅ YES — bundle it | None (public domain) | ✅ `icd10-cm` | High — do it |
| **CVX** | ✅ YES — included in THO | None | ✅ Via THO | Done |
| **HCPCS Level II (A–V codes)** | ✅ YES for A–V codes; ⚠️ D-codes ambiguous (ADA CDT dispute) | None for A–V codes; see D-codes caveat in §3.9 | ❌ Not yet | Medium |
| **SNOMED CT** | ❌ NO | Get Affiliate License via NRC (free in US + member countries) | ✅ `snomed-rf2` | High — docs needed |
| **LOINC** | ⚠️ CONDITIONAL — license permits redistribution with attribution; we currently don't bundle for operational reasons (currency, user tracking) | Register free at loinc.org to download directly; no separate registration required when receiving a properly attributed redistributed copy | ✅ `loinc` | High — docs needed |
| **RxNorm** | ❌ NO (full release contains non-redistributable SRL 3/4 content) | Create free UMLS account at uts.nlm.nih.gov | ✅ `rxnorm` | High — docs needed |
| **NDC** | ⚠️ CONDITIONAL — codes are public domain; full product database has caveats | Download from FDA NDC directory (free) | ❌ Not yet | Medium |
| **CPT** | ❌ NO | Purchase AMA distribution license (expensive) | ❌ Not yet | Low — deferred |
| **ICD-11** | ✅ YES — CC BY-ND 3.0 IGO; with attribution, no modifications | Attribution required | ❌ Not yet | Low — WHO adoption still early |
| **ICD-9-CM** | ✅ YES (public domain) | None | ❌ Not yet | Low — historical use |
| **MedDRA** | ❌ NO | Purchase MSSO license (contact MSSO for pricing) | ❌ Not yet | Low — deferred |
| **HCPCS Level I (CPT)** | ❌ NO | Same as CPT above | ❌ Not yet | Low — deferred |

### Terminologies we are NOT going to support (for now)

These are explicitly out of scope for the current phase:
- **CPT** — AMA license cost and complexity; deferred until enterprise customers request it
- **MedDRA** — MSSO paid license; niche use (drug safety reporting, not general clinical workflows)
- **ICD-9-CM** — Retired in 2015; historical-only use case, low demand
- **HCPCS Level I** — Same as CPT
- **WHO ICD-10** (international) — US implementers use ICD-10-CM; WHO ICD-10 use is rare in our target market

---

*Last updated: 2026-04-08. Sources are linked inline. Re-verify license terms annually as they can change — particularly SNOMED CT member country list and LOINC license conditions.*
