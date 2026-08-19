//! Canonical URLs published by the SQL on FHIR implementation guide.
//!
//! Edition 3 (`3.0.0-ballot`) moved the guide to HL7 stewardship, and a change
//! of stewardship reissues every canonical URL it publishes. The base changed
//! from `https://sql-on-fhir.org/ig` to [`IG_CANONICAL_BASE`], and the package
//! identifier from `org.sql-on-fhir.ig` to `hl7.fhir.uv.sql-on-fhir`.
//!
//! ViewDefinition is the one exception. It became an *additional resource*
//! rather than a logical model, so it carries a canonical in the **core FHIR**
//! namespace ([`VIEW_DEFINITION_STRUCTURE_DEFINITION`]) following the
//! convention for a resource incubated outside core — not one under this
//! guide's base.
//!
//! The 2.0.0-era URLs are kept alongside as `LEGACY_*` so resources authored
//! against the previous release still parse. Version 2.0.0 remains published at
//! its original canonical, and the two releases install side by side, so a
//! reader accepting both is correct against either.

/// Canonical base for every artifact this guide publishes, except
/// ViewDefinition itself.
pub const IG_CANONICAL_BASE: &str = "http://hl7.org/fhir/uv/sql-on-fhir";

/// Canonical base used by version 2.0.0 and the pre-ballot continuous build.
pub const LEGACY_IG_CANONICAL_BASE: &str = "https://sql-on-fhir.org/ig";

/// FHIR package identifier for the guide.
pub const IG_PACKAGE_ID: &str = "hl7.fhir.uv.sql-on-fhir";

/// ViewDefinition's StructureDefinition, in the core FHIR namespace because it
/// is an additional resource rather than an artifact of this guide.
pub const VIEW_DEFINITION_STRUCTURE_DEFINITION: &str =
    "http://hl7.org/fhir/StructureDefinition/ViewDefinition";

/// ViewDefinition's 2.0.0 canonical, when it was a logical model. Instances
/// authored against 2.0.0 carried this string in `resourceType`; 3.0.0
/// instances carry the plain `ViewDefinition` token instead.
pub const LEGACY_VIEW_DEFINITION_STRUCTURE_DEFINITION: &str =
    "https://sql-on-fhir.org/ig/StructureDefinition/ViewDefinition";

/// `Library.type.coding.system` fixed by the SQLQuery and SQLView profiles.
pub const LIBRARY_TYPES_CODE_SYSTEM: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";

/// Pre-ballot `LibraryTypesCodes` system. Accepted on read for back-compat.
pub const LEGACY_LIBRARY_TYPES_CODE_SYSTEM: &str =
    "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

/// The `sql-text` extension carrying plain-text SQL alongside the base64
/// `Attachment.data`.
pub const SQL_TEXT_EXTENSION: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition/sql-text";

/// Output formats bound by `$sql-run` (`csv`, `ndjson`, `parquet`, `json`,
/// `fhir`).
pub const OUTPUT_FORMAT_VALUE_SET: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/ValueSet/OutputFormatCodes";

/// Output formats bound by `$sql-export`. Narrower than
/// [`OUTPUT_FORMAT_VALUE_SET`]: it omits `fhir`, which only a run can produce.
pub const EXPORT_OUTPUT_FORMAT_VALUE_SET: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/ValueSet/ExportOutputFormatCodes";

/// States an asynchronous export passes through.
pub const EXPORT_STATUS_VALUE_SET: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/ValueSet/ExportStatusCodes";

/// `$sql-run` — synchronous evaluation of a single subject.
pub const SQL_RUN_OPERATION_DEFINITION: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/OperationDefinition/SQLRun";

/// `$sql-export` — asynchronous export of one or more subjects as a single job.
pub const SQL_EXPORT_OPERATION_DEFINITION: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/OperationDefinition/SQLExport";

/// The SQLQuery profile on `Library`.
pub const SQL_QUERY_PROFILE: &str =
    "http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition/SQLQuery";

/// The SQLView profile on `Library`.
pub const SQL_VIEW_PROFILE: &str = "http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition/SQLView";
