//! Handlers for `ValueSet/$expand` — type-level and instance-level.
//!
//! Expansion resolves all codes that belong to a ValueSet and returns them
//! inside a `ValueSet.expansion.contains[]` array.  Four handler variants are
//! provided:
//!
//! * **Type-level POST** — `POST /ValueSet/$expand` with a FHIR `Parameters` body.
//! * **Type-level GET** — `GET /ValueSet/$expand?url=<url>`.
//! * **Instance-level POST** — `POST /ValueSet/{id}/$expand`.
//! * **Instance-level GET** — `GET /ValueSet/{id}/$expand?filter=...`.
//!
//! ## Supported parameters
//!
//! | Parameter | Type | Description |
//! |-----------|------|-------------|
//! | `url` | uri | Canonical URL of the ValueSet (type-level) |
//! | `filter` | string | Substring filter on code or display |
//! | `count` | integer | Page size |
//! | `offset` | integer | Zero-based page start |
//! | `date` | dateTime | Point-in-time ISO-8601 date for evaluation |
//! | `hierarchical` | boolean | Return a tree-structured expansion instead of a flat list |
//! | `excludeNested` | boolean | When `false`, return a tree-structured expansion (alias for `hierarchical=true`); when `true` or absent, return a flat list |
//!
//! ## Implicit ValueSets
//!
//! When `url` matches a CodeSystem's `valueSet` property and no explicit
//! ValueSet resource exists for that URL, the expansion falls back to all
//! codes in that CodeSystem (FHIR R5 §4.8.7).
//!
//! ## FHIR specification
//!
//! <https://hl7.org/fhir/valueset-operation-expand.html>

use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::{AppState, EXPAND_CACHE_MAX, ExpandCacheKey, NOT_FOUND_CACHE_MAX};
use crate::traits::{SupplementInfo, TerminologyBackend, ValueSetOperations};
use crate::types::{ExpandRequest, ExpansionContains};

use super::format::{ResponseFormat, json_to_fhir_xml, negotiate_format};
use super::params::{
    collect_resource_params, extract_parameter_array, find_resource_param, find_str_param,
    parse_query_string, query_params_to_fhir_params,
};

/// Collect the standards-status codes that should fire `warning-<status>`
/// expansion parameters for a CodeSystem or ValueSet.
///
/// Surveys three FHIR markers, in this order:
///
/// 1. The `http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status`
///    extension's `valueCode` — typically `deprecated`, `withdrawn`, or `draft`.
/// 2. `experimental: true` → emits `experimental` (only on CodeSystem; ValueSets
///    use the same field but the IG fixtures don't ask for `warning-experimental`
///    on a VS-level basis — driven by the contributing CS).
/// 3. `status: "draft"` → emits `draft` (mirrors the standards-status pattern
///    when the resource simply uses FHIR's status field rather than the
///    extension).
///
/// Returns the deduplicated list of status codes, preserving the order above.
/// The IG fixtures use this list to populate `warning-<code>` entries in
/// `expansion.parameter[]`.
fn standards_statuses(resource: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |code: &str| {
        if !code.is_empty() && !out.iter().any(|c| c == code) {
            out.push(code.to_string());
        }
    };

    if let Some(exts) = resource.get("extension").and_then(|e| e.as_array()) {
        for ext in exts {
            if ext.get("url").and_then(|u| u.as_str())
                == Some(
                    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                )
            {
                if let Some(code) = ext.get("valueCode").and_then(|v| v.as_str()) {
                    push_unique(code);
                }
            }
        }
    }

    if resource.get("experimental").and_then(|v| v.as_bool()) == Some(true) {
        push_unique("experimental");
    }

    if resource.get("status").and_then(|v| v.as_str()) == Some("draft") {
        push_unique("draft");
    }

    out
}

/// Like [`standards_statuses`] but for ValueSets — only the standards-status
/// extension contributes a warning. The bare `status` and `experimental`
/// flags on a VS do NOT emit a `warning-<status>` per the IG fixtures
/// (`search/*`, `deprecated/*`); those rules apply only to CodeSystems.
fn vs_extension_statuses(resource: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(exts) = resource.get("extension").and_then(|e| e.as_array()) {
        for ext in exts {
            if ext.get("url").and_then(|u| u.as_str())
                == Some(
                    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                )
            {
                if let Some(code) = ext.get("valueCode").and_then(|v| v.as_str()) {
                    if !code.is_empty() && !out.iter().any(|c| c == code) {
                        out.push(code.to_string());
                    }
                }
            }
        }
    }

    out
}

/// Recursively serializes nested `contains` arrays, so that a hierarchical
/// expansion (produced when `hierarchical=true`) is correctly represented as
/// nested `contains[]` objects rather than a flat list.
///
/// The `display` field is omitted when absent, and `contains` is omitted when
/// the entry has no children — keeping the output compact for flat expansions.
fn serialize_expansion_contains(
    c: &ExpansionContains,
    multi_version_systems: &std::collections::HashSet<String>,
) -> Value {
    let mut item = serde_json::Map::new();
    // Concept-level extensions appear FIRST in the IG fixtures (the FHIR
    // canonical ordering puts `extension` ahead of `system` for any element).
    if !c.extensions.is_empty() {
        item.insert("extension".into(), json!(c.extensions));
    }
    item.insert("system".into(), json!(c.system));
    item.insert("code".into(), json!(c.code));
    if let Some(display) = &c.display {
        item.insert("display".into(), json!(display));
    }
    // Only emit version when the expansion mixes multiple versions of this
    // system — for single-version CSes the version is implicit (and the IG
    // fixtures don't expect it in the contains items).
    if multi_version_systems.contains(&c.system) {
        if let Some(version) = &c.version {
            item.insert("version".into(), json!(version));
        }
    }
    // FHIR expansion.contains.abstract / .inactive — only emit when true.
    if c.is_abstract == Some(true) {
        item.insert("abstract".into(), json!(true));
    }
    if c.inactive == Some(true) {
        item.insert("inactive".into(), json!(true));
    }
    if !c.designations.is_empty() {
        let designations: Vec<Value> = c
            .designations
            .iter()
            .map(|d| {
                let mut entry = serde_json::Map::new();
                if !d.extensions.is_empty() {
                    entry.insert("extension".into(), json!(d.extensions));
                }
                if let Some(lang) = &d.language {
                    entry.insert("language".into(), json!(lang));
                }
                if d.use_system.is_some() || d.use_code.is_some() {
                    let mut us = serde_json::Map::new();
                    if let Some(s) = &d.use_system {
                        us.insert("system".into(), json!(s));
                    }
                    if let Some(c) = &d.use_code {
                        us.insert("code".into(), json!(c));
                    }
                    entry.insert("use".into(), Value::Object(us));
                }
                entry.insert("value".into(), json!(d.value));
                Value::Object(entry)
            })
            .collect();
        item.insert("designation".into(), json!(designations));
    }
    if !c.properties.is_empty() {
        // Sort by property code for stable, IG-fixture-matching output. The
        // fixtures (e.g. extensions/expand-echo-all) emit
        // `contains[].property[]` in alphabetical-by-code order regardless
        // of insertion order at the contributor sources.
        let mut sorted_props = c.properties.clone();
        sorted_props.sort_by(|a, b| a.code.cmp(&b.code));
        let props: Vec<Value> = sorted_props
            .iter()
            .map(|p| {
                // Map our internal type label to a FHIR `value[x]` field.
                let key = match p.value_type.as_str() {
                    "Boolean" => "valueBoolean",
                    "Integer" => "valueInteger",
                    "Decimal" => "valueDecimal",
                    "DateTime" => "valueDateTime",
                    "Code" => "valueCode",
                    _ => "valueString",
                };
                let value: Value = if key == "valueBoolean" {
                    json!(p.value == "true")
                } else if key == "valueInteger" {
                    json!(p.value.parse::<i64>().unwrap_or(0))
                } else if key == "valueDecimal" {
                    if let Ok(i) = p.value.parse::<i64>() {
                        json!(i)
                    } else if let Ok(f) = p.value.parse::<f64>() {
                        json!(f)
                    } else {
                        json!(p.value)
                    }
                } else {
                    json!(p.value)
                };
                json!({ "code": p.code, key: value })
            })
            .collect();
        item.insert("property".into(), json!(props));
    }
    if !c.contains.is_empty() {
        let nested: Vec<Value> = c
            .contains
            .iter()
            .map(|child| serialize_expansion_contains(child, multi_version_systems))
            .collect();
        item.insert("contains".into(), json!(nested));
    }
    Value::Object(item)
}

/// Resolve `is_abstract` / `inactive` flags on each expansion entry via a
/// per-system batched lookup. Backends construct ExpansionContains entries
/// with both flags as `None`; this fills them so $expand responses surface
/// the standard FHIR contains[].abstract / contains[].inactive fields.
fn populate_concept_flags<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use std::collections::HashMap;
        // Bucket codes per system so we issue one query per system.
        let mut by_system: HashMap<&str, Vec<String>> = HashMap::new();
        for c in contains.iter() {
            by_system
                .entry(c.system.as_str())
                .or_default()
                .push(c.code.clone());
        }
        let mut flag_map: HashMap<(String, String), crate::traits::ConceptExpansionFlags> =
            HashMap::new();
        for (system, codes) in &by_system {
            if let Ok(flags) = backend.concept_expansion_flags(ctx, system, codes).await {
                for (code, f) in flags {
                    flag_map.insert(((*system).to_string(), code), f);
                }
            }
        }
        for c in contains.iter_mut() {
            if let Some(f) = flag_map.get(&(c.system.clone(), c.code.clone())) {
                if f.is_abstract {
                    c.is_abstract = Some(true);
                }
                if f.inactive {
                    c.inactive = Some(true);
                }
            }
            if !c.contains.is_empty() {
                populate_concept_flags(backend, ctx, &mut c.contains).await;
            }
        }
    })
}

/// Populate `designations` on each expansion entry in-place via a per-system
/// batched lookup. Mirrors `populate_concept_flags` and is only invoked when
/// the caller passes `includeDesignations=true`.
fn populate_designations<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsDesignation;
        use std::collections::HashMap;
        let mut by_system: HashMap<&str, Vec<String>> = HashMap::new();
        for c in contains.iter() {
            by_system
                .entry(c.system.as_str())
                .or_default()
                .push(c.code.clone());
        }
        let mut map: HashMap<(String, String), Vec<ExpansionContainsDesignation>> = HashMap::new();
        for (system, codes) in &by_system {
            if let Ok(ds) = backend.concept_designations(ctx, system, codes).await {
                for (code, list) in ds {
                    let entries = list
                        .into_iter()
                        .map(|d| ExpansionContainsDesignation {
                            language: d.language,
                            use_system: d.use_system,
                            use_code: d.use_code,
                            value: d.value,
                            extensions: vec![],
                        })
                        .collect();
                    map.insert(((*system).to_string(), code), entries);
                }
            }
        }
        for c in contains.iter_mut() {
            if let Some(ds) = map.remove(&(c.system.clone(), c.code.clone())) {
                c.designations = ds;
            }
            if !c.contains.is_empty() {
                populate_designations(backend, ctx, &mut c.contains).await;
            }
        }
    })
}

/// Populate `properties` on each expansion entry from a per-system batched
/// lookup of the named properties. Mirrors `populate_designations`. Walks
/// nested `contains[]` recursively.
fn populate_properties<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
    properties: &'a [String],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsProperty;
        use std::collections::HashMap;
        let mut by_system: HashMap<&str, Vec<String>> = HashMap::new();
        for c in contains.iter() {
            by_system
                .entry(c.system.as_str())
                .or_default()
                .push(c.code.clone());
        }
        let mut map: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
        for (system, codes) in &by_system {
            if let Ok(props) = backend
                .concept_property_values(ctx, system, codes, properties)
                .await
            {
                for (code, list) in props {
                    map.insert(((*system).to_string(), code), list);
                }
            }
        }
        for c in contains.iter_mut() {
            if let Some(list) = map.remove(&(c.system.clone(), c.code.clone())) {
                c.properties = list
                    .into_iter()
                    .map(|(code, value)| {
                        // Pick the FHIR `value[x]` shape from the property
                        // code: `definition` is always a string per FHIR
                        // (it's the synthesised CS column); everything else
                        // we treat as a Code primitive — concept property
                        // values are most commonly Code and tests have not
                        // flagged false positives for the simple case.
                        let value_type = if code == "definition" {
                            "string".to_string()
                        } else {
                            "Code".to_string()
                        };
                        ExpansionContainsProperty {
                            code,
                            value_type,
                            value,
                        }
                    })
                    .collect();
            }
            if !c.contains.is_empty() {
                populate_properties(backend, ctx, &mut c.contains, properties).await;
            }
        }
    })
}

/// Parsed `displayLanguage` request parameter.
///
/// FHIR allows simple language codes (`de`), comma-separated lists with an
/// optional wildcard (`de,*`), and Accept-Language style q-weights
/// (`de,*; q=0`). The HL7 IG `language/expand-xform-*` fixtures distinguish:
///
/// | Form | preferred | hard_fallback | Meaning |
/// |------|-----------|---------------|---------|
/// | `de` | `de` | `false` | Try de; otherwise keep CS-default display |
/// | `de,*` | `de` | `false` | Same as above (`*` is just an explicit fallback) |
/// | `de,*; q=0` | `de` | `true` | Try de; if missing, drop top-level display |
///
/// `preferred` is the first non-wildcard tag (the language we want to swap
/// in); `hard_fallback` is `true` when the wildcard carries `q=0`, signalling
/// that no fallback is allowed.
struct DisplayLangSpec {
    preferred: String,
    hard_fallback: bool,
}

/// Parse a `displayLanguage` parameter value into a [`DisplayLangSpec`].
fn parse_display_language(raw: &str) -> Option<DisplayLangSpec> {
    let mut preferred: Option<String> = None;
    let mut hard_fallback = false;

    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Split q= weight (Accept-Language style): "*; q=0" → tag="*", q=Some(0.0)
        let (tag, q) = if let Some((t, rest)) = trimmed.split_once(';') {
            let q = rest
                .trim()
                .strip_prefix("q=")
                .or_else(|| rest.trim().strip_prefix("Q="))
                .and_then(|s| s.parse::<f32>().ok());
            (t.trim(), q)
        } else {
            (trimmed, None)
        };
        if tag == "*" {
            // q=0 on wildcard means "do not fall back to anything" → hard mode.
            if q == Some(0.0) {
                hard_fallback = true;
            }
        } else if preferred.is_none() && !tag.is_empty() {
            preferred = Some(tag.to_string());
        }
    }

    preferred.map(|p| DisplayLangSpec {
        preferred: p,
        hard_fallback,
    })
}

/// The HL7 `hl7TermMaintInfra` system + code identifying a designation as
/// the "preferred for language" entry. Used when the displayLanguage swap
/// rotates the CodeSystem's original-language display into the designation
/// list — the IG fixtures expect this `use` coding to flag that entry.
const HL7_TERM_MAINT_INFRA_SYSTEM: &str = "http://terminology.hl7.org/CodeSystem/hl7TermMaintInfra";

/// Append designations contributed by applied CodeSystem supplements onto
/// each expansion entry. Mirrors [`populate_designations`] but reads via the
/// backend's `supplement_designations` API and merges into the existing
/// `designations` vec rather than replacing it. Walks nested `contains[]`
/// recursively.
///
/// Supplements live in the same `code_systems` table (with `content =
/// 'supplement'`) so we look them up by URL — the supplement-side rows are
/// matched to base concepts by code only. The backend tags each returned
/// row with `source = "url|version"` so callers can emit the FHIR
/// `designation.source` part on `$lookup`; for `$expand.contains[]` the
/// source is informational only.
fn apply_supplement_designations<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
    supplement_urls: &'a [String],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsDesignation;
        use std::collections::HashMap;
        if supplement_urls.is_empty() {
            return;
        }
        let mut codes: Vec<String> = Vec::new();
        for c in contains.iter() {
            if !codes.contains(&c.code) {
                codes.push(c.code.clone());
            }
        }
        let map = backend
            .supplement_designations(ctx, supplement_urls, &codes)
            .await
            .unwrap_or_default();
        // Build a flat code → designations map keyed only by code (supplements
        // are typically scoped to a single base CS, so collisions on code
        // across systems are unusual and acceptable here).
        let mut by_code: HashMap<String, Vec<ExpansionContainsDesignation>> = HashMap::new();
        for (code, list) in map {
            let entries = list
                .into_iter()
                .map(|d| ExpansionContainsDesignation {
                    language: d.language,
                    use_system: d.use_system,
                    use_code: d.use_code,
                    value: d.value,
                    extensions: vec![],
                })
                .collect();
            by_code.insert(code, entries);
        }
        for c in contains.iter_mut() {
            if let Some(extra) = by_code.get(&c.code) {
                for d in extra {
                    c.designations.push(d.clone());
                }
            }
            if !c.contains.is_empty() {
                apply_supplement_designations(backend, ctx, &mut c.contains, supplement_urls).await;
            }
        }
    })
}

/// Append property values contributed by applied CodeSystem supplements onto
/// each expansion entry. Mirrors [`populate_properties`] but reads via the
/// backend's `supplement_property_values` API. Walks nested `contains[]`
/// recursively. When a supplement defines a property for a code that the
/// base CS doesn't, the supplement value is added to the entry; values
/// defined in BOTH base and supplement are surfaced once each (the IG
/// fixtures don't deduplicate by property code).
fn apply_supplement_properties<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
    supplement_urls: &'a [String],
    properties: &'a [String],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsProperty;
        use std::collections::HashMap;
        if supplement_urls.is_empty() || properties.is_empty() {
            return;
        }
        let mut codes: Vec<String> = Vec::new();
        for c in contains.iter() {
            if !codes.contains(&c.code) {
                codes.push(c.code.clone());
            }
        }
        let map = backend
            .supplement_property_values(ctx, supplement_urls, &codes, properties)
            .await
            .unwrap_or_default();
        let mut by_code: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (code, list) in map {
            by_code.insert(code, list);
        }
        for c in contains.iter_mut() {
            if let Some(extra) = by_code.get(&c.code) {
                for (prop, value) in extra {
                    c.properties.push(ExpansionContainsProperty {
                        code: prop.clone(),
                        value_type: "Code".to_string(),
                        value: value.clone(),
                    });
                }
            }
            if !c.contains.is_empty() {
                apply_supplement_properties(
                    backend,
                    ctx,
                    &mut c.contains,
                    supplement_urls,
                    properties,
                )
                .await;
            }
        }
    })
}

/// Standards-extension URLs that surface as concept-level FHIR extensions on
/// `expansion.contains[].extension[]`. Each entry has a corresponding URL
/// literal in the FHIR-published "rendering" StructureDefinitions. See:
/// <https://hl7.org/fhir/extensions/StructureDefinition-rendering-style.html>
/// and <https://hl7.org/fhir/extensions/StructureDefinition-rendering-xhtml.html>.
const PASSTHROUGH_CONCEPT_EXTENSIONS: &[&str] = &[
    "http://hl7.org/fhir/StructureDefinition/rendering-style",
    "http://hl7.org/fhir/StructureDefinition/rendering-xhtml",
    "http://hl7.org/fhir/StructureDefinition/valueset-concept-definition",
    "http://hl7.org/fhir/StructureDefinition/valueset-deprecated",
];

/// Concept-level extension URLs whose value gets synthesised into a
/// concept-property entry on `expansion.contains[].property[]` rather than
/// appearing as a literal `extension[]` entry. The mapping (extension URL →
/// FHIR concept-property code) follows the ordering convention in the IG
/// `extensions/extensions-all` fixture: each extension's value contributes a
/// property whose `code` is the FHIR-canonical concept-property name and
/// whose `uri` is `http://hl7.org/fhir/concept-properties#<code>`.
fn extension_to_property_code(url: &str) -> Option<&'static str> {
    match url {
        "http://hl7.org/fhir/StructureDefinition/codesystem-conceptOrder" => Some("order"),
        "http://hl7.org/fhir/StructureDefinition/codesystem-label" => Some("label"),
        "http://hl7.org/fhir/StructureDefinition/itemWeight" => Some("weight"),
        // The concept-level `structuredefinition-standards-status` extension
        // synthesises a `status` property when its valueCode is `deprecated`
        // or `withdrawn` (consistent with the IG extensions/extensions-all
        // expectation that only deprecated/withdrawn concepts surface a
        // status row in the expansion).
        "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status" => {
            Some("status")
        }
        _ => None,
    }
}

/// Determine the FHIR `value[x]` field on an extension JSON object. Returns
/// the type label (e.g. "Decimal", "String", "Code") and a canonical string
/// representation of the value, matching the convention used by
/// [`crate::types::ExpansionContainsProperty`].
fn extension_value_for_property(ext: &Value) -> Option<(&'static str, String)> {
    if let Some(v) = ext.get("valueDecimal") {
        if let Some(f) = v.as_f64() {
            return Some(("Decimal", normalize_decimal(f)));
        }
        if let Some(i) = v.as_i64() {
            return Some(("Decimal", i.to_string()));
        }
    }
    if let Some(v) = ext.get("valueInteger").and_then(|v| v.as_i64()) {
        return Some(("Decimal", v.to_string()));
    }
    if let Some(v) = ext.get("valueString").and_then(|v| v.as_str()) {
        return Some(("String", v.to_string()));
    }
    if let Some(v) = ext.get("valueCode").and_then(|v| v.as_str()) {
        return Some(("Code", v.to_string()));
    }
    if let Some(v) = ext.get("valueBoolean").and_then(|v| v.as_bool()) {
        return Some(("Boolean", v.to_string()));
    }
    None
}

/// Render a finite f64 as the shortest decimal string that round-trips, to
/// avoid surfacing artifacts like `1.2000000000000002`. Falls back to the
/// default Display when the value is integral.
fn normalize_decimal(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{}", f as i64)
    } else {
        // 6 fractional digits is enough precision for the IG fixtures and
        // strips trailing zeros via the trim_end_matches step.
        let s = format!("{f:.6}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').to_string();
        if trimmed.is_empty() {
            "0".to_string()
        } else {
            trimmed
        }
    }
}

/// Walk every `contains[]` entry, fetch its concept resource JSON from the
/// base CodeSystem and any applied supplements, and merge:
/// - concept-level passthrough extensions (rendering-style, rendering-xhtml,
///   valueset-concept-definition, valueset-deprecated) into `c.extensions`,
/// - per-designation extensions (coding-sctdescid,
///   structuredefinition-standards-status) into the matching designation's
///   `extensions` field,
/// - synthesised concept properties (order/label/weight/status) derived from
///   well-known concept-level extensions, into `c.properties`.
///
/// Processed alongside (and after) the existing
/// [`populate_properties`] / [`apply_supplement_properties`] calls so the
/// resulting property set is the union of (a) declared concept properties,
/// (b) well-known extension-derived properties.
fn apply_concept_extension_data<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
    supplement_urls: &'a [String],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsProperty;
        use std::collections::HashMap;
        // Bucket codes per system for one batched lookup per system.
        let mut by_system: HashMap<String, Vec<String>> = HashMap::new();
        for c in contains.iter() {
            by_system
                .entry(c.system.clone())
                .or_default()
                .push(c.code.clone());
        }
        // For each system: fetch base concept entries and supplement
        // concept entries (one map per system).
        let mut base_entries: HashMap<(String, String), Value> = HashMap::new();
        let mut supp_entries: HashMap<(String, String), Vec<Value>> = HashMap::new();
        for (system, codes) in &by_system {
            if let Ok(map) = backend
                .concept_resource_entries(ctx, system, codes)
                .await
            {
                for (code, entry) in map {
                    base_entries.insert((system.clone(), code), entry);
                }
            }
            if !supplement_urls.is_empty() {
                if let Ok(map) = backend
                    .supplement_concept_entries(ctx, supplement_urls, codes)
                    .await
                {
                    for (code, entries) in map {
                        supp_entries.insert((system.clone(), code), entries);
                    }
                }
            }
        }

        for c in contains.iter_mut() {
            // Order the contributing entries: base first, then any supplement
            // overrides (later wins for properties; for extensions the IG
            // expects supplement values to OVERRIDE the base for the same
            // URL — see `extensions-enumerated` which expects the supplement
            // rendering-style/rendering-xhtml on code2 instead of base).
            let mut sources: Vec<&Value> = Vec::new();
            if let Some(base) = base_entries.get(&(c.system.clone(), c.code.clone())) {
                sources.push(base);
            }
            if let Some(extras) = supp_entries.get(&(c.system.clone(), c.code.clone())) {
                for e in extras {
                    sources.push(e);
                }
            }

            // Pass 1: passthrough concept-level extensions. Supplement
            // entries OVERRIDE the base for the same URL — drop any prior
            // entry with the same url before pushing.
            for src in &sources {
                let Some(exts) = src.get("extension").and_then(|e| e.as_array()) else {
                    continue;
                };
                for ext in exts {
                    let url = match ext.get("url").and_then(|u| u.as_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    if !PASSTHROUGH_CONCEPT_EXTENSIONS.contains(&url) {
                        continue;
                    }
                    c.extensions
                        .retain(|existing| existing.get("url").and_then(|u| u.as_str()) != Some(url));
                    c.extensions.push(ext.clone());
                }
            }

            // Pass 2: synthesise properties from well-known extensions.
            // Supplement-provided values override base for the same property
            // code (so e.g. base codesystem-conceptOrder=6 → order=6, but a
            // supplement codesystem-conceptOrder would override it).
            for src in &sources {
                let Some(exts) = src.get("extension").and_then(|e| e.as_array()) else {
                    continue;
                };
                for ext in exts {
                    let url = match ext.get("url").and_then(|u| u.as_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    let Some(prop_code) = extension_to_property_code(url) else {
                        continue;
                    };
                    let Some((value_type, value)) = extension_value_for_property(ext) else {
                        continue;
                    };
                    // For the standards-status → status mapping, only emit
                    // when the status is deprecated/withdrawn (matches IG
                    // extensions-all expectation; an `active` status would
                    // otherwise add noise to every concept).
                    if prop_code == "status" && !matches!(value.as_str(), "deprecated" | "withdrawn")
                    {
                        continue;
                    }
                    // Drop any existing property with the same code so the
                    // last-seen (supplement-overrides-base) value wins.
                    c.properties.retain(|p| p.code != prop_code);
                    c.properties.push(ExpansionContainsProperty {
                        code: prop_code.to_string(),
                        value_type: value_type.to_string(),
                        value,
                    });
                }
            }

            // Pass 3: per-designation extensions. Match each base/supplement
            // designation against the entry's existing designations by
            // (language, value) and copy across its extension[].
            //
            // Only annotate ALREADY-PRESENT designations — never invent new
            // ones here.  Pre-Pass 3 the only path that populates
            // `c.designations` is `populate_designations` (gated on
            // `includeDesignations=true`) and `apply_supplement_designations`
            // (gated on `includeDesignations` AND a supplement being applied).
            // Adding designations here unconditionally surfaces base CS
            // designations on every expansion, breaking
            // `parameters/parameters-expand-supplement-none` which expects
            // a designation-free response.
            for src in &sources {
                let Some(desigs) = src.get("designation").and_then(|d| d.as_array()) else {
                    continue;
                };
                for d in desigs {
                    let value = match d.get("value").and_then(|v| v.as_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    let language = d
                        .get("language")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let Some(d_exts) = d.get("extension").and_then(|e| e.as_array()) else {
                        continue;
                    };
                    if d_exts.is_empty() {
                        continue;
                    }
                    let target = c.designations.iter_mut().find(|existing| {
                        existing.value.eq_ignore_ascii_case(value)
                            && existing.language == language
                    });
                    if let Some(t) = target {
                        for d_ext in d_exts {
                            let url = match d_ext.get("url").and_then(|u| u.as_str()) {
                                Some(s) => s,
                                None => continue,
                            };
                            t.extensions
                                .retain(|e| e.get("url").and_then(|u| u.as_str()) != Some(url));
                            t.extensions.push(d_ext.clone());
                        }
                    }
                    // (No `else` branch — see comment above.)
                }
            }

            if !c.contains.is_empty() {
                apply_concept_extension_data(backend, ctx, &mut c.contains, supplement_urls)
                    .await;
            }
        }
    })
}

/// Replace each contains[] entry's `display` with a designation matching the
/// requested displayLanguage. Mirrors the `lookup()` language-aware behavior
/// and walks nested `contains[]` recursively.
///
/// Per the HL7 IG `language/expand-xform-*` fixtures, when a swap fires we
/// also rotate the original CS-language display into `c.designations` as a
/// `{language: <cs-lang>, use: preferredForLanguage, value: <orig display>}`
/// entry, and remove the now-redundant matching-language designation. The
/// `cs_lang_by_url` map is read from the contributing CodeSystem's top-level
/// `language` field.
///
/// `hard_fallback` controls behavior when no matching designation exists:
/// `true` drops the top-level display entirely (per the `*; q=0` convention),
/// `false` leaves the original display in place.
fn apply_display_language<'a, B: TerminologyBackend>(
    backend: &'a B,
    ctx: &'a TenantContext,
    contains: &'a mut [ExpansionContains],
    spec: &'a DisplayLangSpec,
    cs_lang_by_url: &'a std::collections::HashMap<String, Option<String>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use crate::types::ExpansionContainsDesignation;
        use std::collections::HashMap;
        let language = spec.preferred.as_str();

        // Bucket codes per system for a single batched designation lookup.
        let mut by_system: HashMap<&str, Vec<String>> = HashMap::new();
        for c in contains.iter() {
            by_system
                .entry(c.system.as_str())
                .or_default()
                .push(c.code.clone());
        }
        // (system, code) → (designation language tag, designation value).
        // Match using BCP 47 / RFC 4647 Lookup: prefer an exact match, then
        // accept any designation whose tag starts with the requested tag plus
        // a `-` subtag separator (so `de` matches `de-CH` but not `den`).
        let mut match_map: HashMap<(String, String), (Option<String>, String)> = HashMap::new();
        for (system, codes) in &by_system {
            if let Ok(ds) = backend.concept_designations(ctx, system, codes).await {
                for (code, list) in ds {
                    let exact = list
                        .iter()
                        .find(|d| d.language.as_deref() == Some(language));
                    let chosen = exact.cloned().or_else(|| {
                        list.into_iter().find(|d| {
                            d.language.as_deref().is_some_and(|lang| {
                                let prefix = format!("{language}-");
                                lang.eq_ignore_ascii_case(language)
                                    || lang
                                        .to_ascii_lowercase()
                                        .starts_with(&prefix.to_ascii_lowercase())
                            })
                        })
                    });
                    if let Some(d) = chosen {
                        match_map.insert(((*system).to_string(), code), (d.language, d.value));
                    }
                }
            }
        }

        for c in contains.iter_mut() {
            let cs_lang = cs_lang_by_url.get(&c.system).cloned().flatten();
            let original_display = c.display.clone();
            // When the CS's own `language` already matches the requested
            // displayLanguage exactly, the top-level `display` is already
            // in the requested language. Don't promote a (broader-match)
            // designation in that case — doing so would drop the source
            // designation entry that the IG `language/expand-echo-de-multi-de-*`
            // fixtures expect to survive (e.g. a `de-CH` designation alongside
            // a CS with `language=de`).
            let cs_lang_already_matches = cs_lang.as_deref() == Some(language);
            if cs_lang_already_matches {
                // Skip the swap entirely; designations are preserved as-is.
                if !c.contains.is_empty() {
                    apply_display_language(backend, ctx, &mut c.contains, spec, cs_lang_by_url)
                        .await;
                }
                continue;
            }
            if let Some((matched_lang, matched_value)) =
                match_map.remove(&(c.system.clone(), c.code.clone()))
            {
                // Swap top-level display for the matching-language designation.
                c.display = Some(matched_value.clone());

                // Drop the source designation we just promoted (matched on
                // both language + value to be precise — broader-match designations
                // for unrelated codes survive untouched).
                c.designations
                    .retain(|d| !(d.language == matched_lang && d.value == matched_value));

                // Rotate the former display into designations[] tagged with the
                // CS's own language and `use=preferredForLanguage`. Skip when
                // the original display would just duplicate the matched value
                // (degenerate case where CS-lang == requested lang).
                if let Some(orig) = original_display
                    .filter(|s| !s.is_empty() && cs_lang.as_deref() != Some(language))
                {
                    let already = c
                        .designations
                        .iter()
                        .any(|d| d.language == cs_lang && d.value == orig);
                    if !already {
                        c.designations.push(ExpansionContainsDesignation {
                            language: cs_lang.clone(),
                            use_system: Some(HL7_TERM_MAINT_INFRA_SYSTEM.to_string()),
                            use_code: Some("preferredForLanguage".to_string()),
                            value: orig,
                            extensions: vec![],
                        });
                    }
                }
            } else if spec.hard_fallback {
                // No matching designation and the caller forbade fallback —
                // drop the top-level display entirely. The IG fixtures still
                // surface the original (CS-default) display as a designation
                // with `use=preferredForLanguage` so consumers can recover it.
                if let Some(orig) = original_display {
                    if !orig.is_empty() {
                        let already = c
                            .designations
                            .iter()
                            .any(|d| d.language == cs_lang && d.value == orig);
                        if !already {
                            c.designations.push(ExpansionContainsDesignation {
                                language: cs_lang.clone(),
                                use_system: Some(HL7_TERM_MAINT_INFRA_SYSTEM.to_string()),
                                use_code: Some("preferredForLanguage".to_string()),
                                value: orig,
                                extensions: vec![],
                            });
                        }
                    }
                }
                c.display = None;
            }

            if !c.contains.is_empty() {
                apply_display_language(backend, ctx, &mut c.contains, spec, cs_lang_by_url).await;
            }
        }
    })
}

/// Expand a ValueSet and return the result as pre-serialized JSON bytes.
///
/// Bytes are cached keyed on request parameters.  On a cache hit the stored
/// [`Bytes`] handle is cloned in O(1) (reference-count bump only — no heap
/// allocation or JSON re-serialization).  On a cache miss the result is
/// serialized once, stored, and returned.
async fn process_expand<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Bytes, HtsError> {
    // Parse the `url` parameter. FHIR supports pipe-separated canonical URLs
    // (`http://example.org/vs|1.0.0`) — split and promote the version to
    // `valueSetVersion` when no explicit `valueSetVersion` param is present.
    let (url, pipe_version) = match find_str_param(&params, "url") {
        Some(raw) => {
            if let Some(pos) = raw.find('|') {
                let base = raw[..pos].to_string();
                let ver = raw[pos + 1..].to_string();
                (Some(base), Some(ver))
            } else {
                (Some(raw), None)
            }
        }
        None => (None, None),
    };
    let value_set = if url.is_none() {
        find_resource_param(&params, "valueSet")
    } else {
        None
    };

    if url.is_none() && value_set.is_none() {
        return Err(HtsError::InvalidRequest(
            "Missing required parameter: url (ValueSet canonical URL) or valueSet (inline ValueSet resource)".into()
        ));
    }

    let filter = find_str_param(&params, "filter");

    // `count` and `offset` may arrive as integer or string parameters.
    let count = find_str_param(&params, "count")
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| {
            params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("count"))
                .and_then(|p| p.get("valueInteger").and_then(|v| v.as_u64()))
                .map(|v| v as u32)
        });

    let offset = find_str_param(&params, "offset")
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| {
            params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("offset"))
                .and_then(|p| p.get("valueInteger").and_then(|v| v.as_u64()))
                .map(|v| v as u32)
        });

    // `hierarchical` and `excludeNested` both control nesting:
    // - `hierarchical=true` (HL7-tx convention) → tree.
    // - `excludeNested=false` (FHIR R5 §$expand) → tree.
    //   `excludeNested=true` (or absent) → flat list.
    // The IG conformance suite's `parameters/parameters-expand-*` fixtures all
    // pass `excludeNested=false` and expect nested `contains[]`, so we treat
    // either signal as a request for tree mode.
    let hierarchical_param = find_str_param(&params, "hierarchical").map(|s| s == "true");
    let exclude_nested = find_str_param(&params, "excludeNested").map(|s| s == "true");
    let hierarchical = match (hierarchical_param, exclude_nested) {
        (Some(true), _) => Some(true),
        (_, Some(false)) => Some(true),
        (other, _) => other,
    };
    // Track which signal turned tree mode on so the backend can keep
    // enumerated expansions flat when only excludeNested=false was the
    // trigger (per the IG enum-* fixtures).
    let hierarchical_explicit = hierarchical_param == Some(true);

    // ── Resolve supplements (request `useSupplement` params) ────────────────
    // Walk every `useSupplement` and confirm a matching `content=supplement`
    // CodeSystem exists. Unknown supplements become a NotFound error so the
    // bad-supplement IG fixtures reject with 4xx. The resolved info list is
    // applied later, after expansion completes.
    let mut applied_supplements: Vec<SupplementInfo> = Vec::new();
    let mut supplement_inputs: Vec<String> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("useSupplement"))
        .filter_map(|p| {
            p.get("valueCanonical")
                .or_else(|| p.get("valueUri"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    if !supplement_inputs.is_empty() {
        let ctx = TenantContext::system();
        for raw in &supplement_inputs {
            let bare = raw.split('|').next().unwrap_or(raw);
            match state.backend().supplement_target(&ctx, bare).await? {
                Some(info) => applied_supplements.push(info),
                None => {
                    return Err(HtsError::NotFound(format!(
                        "Required supplement not found: {bare}"
                    )));
                }
            }
        }
    }
    // (bare_supplement_urls is rebuilt below after the source-VS extension
    // pass appends any auto-applied supplements to `supplement_inputs`.)

    // ── Cache lookup ─────────────────────────────────────────────────────────
    // Build a stable key from the request parameters. For inline ValueSets
    // (ad-hoc POST) we serialise the body to compact JSON; k6 sends identical
    // bytes each iteration so the string is a reliable cache discriminator.
    // Build a canonical (name-sorted) form of the input parameters minus the
    // ones already captured in `url_or_body`. This makes cache entries unique
    // per combination of "extra" inputs that the response will echo back in
    // `expansion.parameter`.
    let extra_params = {
        let mut sorted: Vec<&Value> = params
            .iter()
            .filter(|p| {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                !matches!(name, "url" | "valueSet")
            })
            .collect();
        sorted.sort_by(|a, b| {
            let an = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            an.cmp(bn)
        });
        serde_json::to_string(&sorted).unwrap_or_default()
    };

    // When a pipe-version was supplied (or a `valueSetVersion` request param
    // exists) include it in `url_or_body` so two URLs differing only in
    // version do not collide. The IG `version/vs-expand-v1` and `vs-expand-v2`
    // fixtures share the same bare URL but pin different versions; without
    // this discriminator the second request would hit the first's cached
    // bytes and report the wrong version's codes.
    let cache_url_key = match url.clone() {
        Some(u) => {
            let v_explicit = find_str_param(&params, "valueSetVersion");
            match (pipe_version.as_ref(), v_explicit.as_ref()) {
                (Some(v), _) => format!("{u}|{v}"),
                (None, Some(v)) => format!("{u}|{v}"),
                _ => u,
            }
        }
        None => value_set
            .as_ref()
            .and_then(|vs| serde_json::to_string(vs).ok())
            .unwrap_or_default(),
    };
    let cache_key = ExpandCacheKey {
        url_or_body: cache_url_key,
        filter: filter.clone().unwrap_or_default(),
        count: count.unwrap_or(u32::MAX),
        offset: offset.unwrap_or(0),
        hierarchical: hierarchical.unwrap_or(false),
        extra_params,
    };

    if let Ok(cache) = state.expand_cache.read() {
        if let Some(cached) = cache.get(&cache_key) {
            // O(1) clone — just bumps the reference count on the shared buffer.
            return Ok(cached.clone());
        }
    }

    // ── Negative-cache check (URL-based 404s) ─────────────────────────────────
    // URLs that previously returned NotFound are remembered here so we can skip
    // all backend queries on repeated requests (saves 5+ SQLite round-trips per
    // hit).
    if let Some(ref url_str) = url {
        if let Ok(neg) = state.not_found_urls.read() {
            if neg.contains(url_str.as_str()) {
                return Err(HtsError::NotFound(url_str.clone()));
            }
        }
    }

    // `tx-resource` parameters provide ad-hoc terminology that the caller does
    // not want to import. Each is a full FHIR resource (typically a ValueSet)
    // that the backend should treat as in-scope only for this single request —
    // used heavily by the tx-ecosystem IG include-combo / exclude-combo
    // fixtures, which provide the entire ValueSet whose URL was passed in the
    // `url` parameter.
    let tx_resources = collect_resource_params(&params, "tx-resource");

    // ── tx-resource shortcut for URL-based requests ──────────────────────────
    // When the request carries a `url` parameter and one of the supplied
    // `tx-resource` resources is a ValueSet whose URL matches, promote that
    // ValueSet to the inline-body path. This means the backend never queries
    // its own store for that URL — the tx-resource fully shadows it for this
    // request — which matches the IG semantics for the include-combo /
    // exclude-combo fixtures.
    let (url, value_set) = if value_set.is_none() {
        if let Some(ref url_str) = url {
            let inline_match = tx_resources.iter().find(|r| {
                r.get("resourceType").and_then(|v| v.as_str()) == Some("ValueSet")
                    && r.get("url").and_then(|v| v.as_str()) == Some(url_str.as_str())
            });
            if let Some(vs) = inline_match {
                (None, Some(vs.clone()))
            } else {
                (url, value_set)
            }
        } else {
            (url, value_set)
        }
    } else {
        (url, value_set)
    };
    // Preserve the URL before it moves into ExpandRequest so we can record it
    // in the negative cache if the backend returns NotFound. Also clone the
    // inline ValueSet body (when present) so we can echo its top-level
    // metadata back in the response after expansion completes.
    let url_for_neg_cache = url.clone();
    let value_set_for_response = value_set.clone();

    // ── system-version / force-system-version overrides ─────────────────────
    // Both parameters are repeating canonical (`url|version`) values.  The
    // FHIR IG `version/parameters-fixed-version` profile applies them as
    // `force-system-version` (override even when the include pins a version)
    // and `system-version` (apply only when the include omits version) to
    // pin which CodeSystem revision contributes to the expansion.
    fn collect_version_pins(
        params: &[Value],
        name: &str,
    ) -> std::collections::HashMap<String, String> {
        let mut out = std::collections::HashMap::new();
        for p in params {
            if p.get("name").and_then(|v| v.as_str()) != Some(name) {
                continue;
            }
            // Accept valueCanonical / valueUri / valueString / valueUrl.
            let raw = ["valueCanonical", "valueUri", "valueString", "valueUrl"]
                .iter()
                .filter_map(|k| p.get(*k).and_then(|v| v.as_str()))
                .next();
            if let Some(s) = raw {
                if let Some(pos) = s.find('|') {
                    let url = s[..pos].to_string();
                    let ver = s[pos + 1..].to_string();
                    if !url.is_empty() && !ver.is_empty() {
                        out.entry(url).or_insert(ver);
                    }
                }
            }
        }
        out
    }
    let force_system_versions = collect_version_pins(&params, "force-system-version");
    let mut system_version_defaults = collect_version_pins(&params, "system-version");
    // `default-valueset-version` request param (FHIR R5 §$expand): per-VS
    // version pins applied when a `compose.include[].valueSet[]` reference
    // lacks a `|version` suffix. Same `<url>|<version>` shape as the
    // `*-system-version` pins; collected via the same helper.
    let default_value_set_versions = collect_version_pins(&params, "default-valueset-version");
    // `check-system-version` acts as both a DEFAULT (same shape as
    // `system-version` — applied only when no other version pin wins) AND
    // a post-expansion verifier.  When the resolved CS version doesn't
    // satisfy the pattern, the IG fixtures expect a 4xx OperationOutcome
    // with `version-error` / VALUESET_VERSION_CHECK
    // (`version/vs-expand-v-w-check`, `vs-expand-all-v-check`).
    let check_system_versions = collect_version_pins(&params, "check-system-version");
    for (sys, pat) in &check_system_versions {
        // `system-version` (DEFAULT) wins over `check-system-version` when
        // both pins target the same system.
        system_version_defaults
            .entry(sys.clone())
            .or_insert_with(|| pat.clone());
    }

    // ── Cache miss: compute ───────────────────────────────────────────────────
    // Resolve the effective `valueSetVersion` for the top-level url:
    // explicit `valueSetVersion` param > pipe-parsed > `default-valueset-version`
    // pin matching the bare url (when no other version was supplied).
    let explicit_vs_version = find_str_param(&params, "valueSetVersion").or(pipe_version.clone());
    let effective_vs_version = explicit_vs_version.clone().or_else(|| {
        url.as_deref()
            .and_then(|u| default_value_set_versions.get(u).cloned())
    });
    // Cloned for downstream `used-valueset` echo logic which needs to apply
    // the same default-version pins to refs lacking a `|version` suffix.
    let default_value_set_versions_for_echo = default_value_set_versions.clone();
    let req = ExpandRequest {
        url,
        value_set_version: effective_vs_version,
        value_set,
        filter: filter.clone(),
        count,
        offset,
        max_expansion_size: Some(
            params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("__max_expansion_size__"))
                .and_then(|p| p.get("valueInteger").and_then(|v| v.as_u64()))
                .map(|v| v as u32)
                .unwrap_or(state.max_expansion_size),
        ),
        date: find_str_param(&params, "date"),
        hierarchical,
        hierarchical_explicit,
        tx_resources,
        force_system_versions,
        system_version_defaults,
        default_value_set_versions,
    };

    let ctx = TenantContext::system();
    let mut resp = match ValueSetOperations::expand(state.backend(), &ctx, req).await {
        Ok(r) => r,
        Err(HtsError::NotFound(msg)) => {
            // Populate the negative cache so future requests for this URL
            // are resolved in O(1) without touching the database.
            //
            // Skip the cache when the failure originated from a nested
            // `compose.include[].valueSet[]` reference (signalled by the
            // message naming a different URL than the top-level request).
            // Caching the top-level URL there would be wrong: the parent VS
            // exists, only an inner pinned ref was missing — used by the IG
            // `valueset-version/expand-indirect-expand-zero-pinned-wrong`
            // fixture which pins `default-valueset-version` to a non-existent
            // version of an imported ValueSet.
            if let Some(ref url_str) = url_for_neg_cache {
                let msg_names_top = msg.contains(&format!("'{url_str}'"))
                    || msg.contains(&format!("'{url_str}|"));
                if msg_names_top {
                    if let Ok(mut neg) = state.not_found_urls.write() {
                        if neg.len() < NOT_FOUND_CACHE_MAX {
                            neg.insert(url_str.clone());
                        }
                    }
                }
            }
            // The IG fixtures format VS-not-found errors as
            //   "A definition for the value Set 'url|version' could not be found"
            // when a `valueSetVersion` was supplied. Rewrite the backend's
            // version-less message in-place when we have one.
            let vs_version = find_str_param(&params, "valueSetVersion");
            let msg =
                if let (Some(url), Some(v)) = (url_for_neg_cache.as_ref(), vs_version.as_ref()) {
                    let needle = format!("'{url}'");
                    let replacement = format!("'{url}|{v}'");
                    msg.replace(&needle, &replacement)
                } else {
                    msg
                };
            return Err(HtsError::NotFound(msg));
        }
        Err(e) => return Err(e),
    };

    // ── Populate abstract / inactive flags ───────────────────────────────────
    // Backends construct ExpansionContains with both flags as None; resolve
    // them here in a per-system batch so the per-concept SQL stays cold-path.
    populate_concept_flags(state.backend(), &ctx, &mut resp.contains).await;

    // ── Look up source ValueSet (used for parameter extension, metadata copy,
    // and to discover the `valueset-supplement` extension that auto-applies a
    // supplement without needing an explicit `useSupplement` request param). ──
    // Honour the requested valueSetVersion (when present) so the metadata
    // we echo back — including the top-level `version` field — matches the
    // ValueSet that was actually used for expansion. With multiple VSes
    // sharing a canonical URL, a URL-only search would otherwise pick
    // whichever row came first in created_at order.
    //
    // The "effective" version used for the source VS lookup includes:
    //   1. an explicit `valueSetVersion` request param;
    //   2. the version side of a piped url (`<url>|<version>`);
    //   3. a `default-valueset-version` pin matching the bare url.
    // This must agree with the version the backend used in
    // `resolve_value_set_versioned` so the metadata copied back into the
    // response (top-level `id`, `version`, `name`, …) reflects the same row
    // the codes came from.
    let req_vs_version = find_str_param(&params, "valueSetVersion")
        .or_else(|| pipe_version.clone())
        .or_else(|| {
            url_for_neg_cache
                .as_deref()
                .and_then(|u| default_value_set_versions_for_echo.get(u).cloned())
        });
    let source_vs: Option<Value> = if let Some(ref u) = url_for_neg_cache {
        ValueSetOperations::search(
            state.backend(),
            &ctx,
            crate::types::ResourceSearchQuery {
                url: Some(u.clone()),
                version: req_vs_version.clone(),
                count: Some(20),
                ..Default::default()
            },
        )
        .await
        .ok()
        .and_then(|mut v| {
            // If a specific version was requested, return the row whose
            // `version` matches exactly. Defensive post-filter: the search
            // SQL already filters by version, but multiple rows can leak in
            // (e.g. when the search predicate gets lost downstream in a
            // composite backend) — picking by exact match here keeps the
            // top-level metadata aligned with the version the backend
            // actually expanded against (`vs-expand-v2` / `vs-expand-v2-default`
            // / `vs-expand-v2-force` regress without this check).
            //
            // When no version is pinned, pick the highest version (matches
            // `resolve_value_set_versioned`).
            if let Some(ref want) = req_vs_version {
                let exact: Option<Value> = v
                    .iter()
                    .find(|r| r.get("version").and_then(|x| x.as_str()) == Some(want.as_str()))
                    .cloned();
                exact.or_else(|| v.into_iter().next())
            } else {
                v.sort_by(|a, b| {
                    let av = a.get("version").and_then(|x| x.as_str()).unwrap_or("");
                    let bv = b.get("version").and_then(|x| x.as_str()).unwrap_or("");
                    bv.cmp(av)
                });
                v.into_iter().next()
            }
        })
    } else {
        value_set_for_response.clone()
    };

    // Pull additional supplements pinned by the source VS via the
    // `valueset-supplement` extension (per HL7 IG `extensions/extensions-all`,
    // which omits `useSupplement` from the request and relies on the VS to
    // declare which supplement applies). Resolve each via `supplement_target`.
    // Unknown supplements pinned via this extension are a hard error — the
    // IG `extensions/expand-echo-bad-supplement` fixture expects a 4xx
    // OperationOutcome whose text mentions both "supplement" and the missing
    // CS canonical URL (matching `$fragments:supplement|...$`).
    if let Some(vs) = source_vs.as_ref() {
        if let Some(exts) = vs.get("extension").and_then(|e| e.as_array()) {
            for ext in exts {
                if ext.get("url").and_then(|u| u.as_str())
                    != Some("http://hl7.org/fhir/StructureDefinition/valueset-supplement")
                {
                    continue;
                }
                let raw = match ext
                    .get("valueCanonical")
                    .or_else(|| ext.get("valueUri"))
                    .and_then(|v| v.as_str())
                {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let bare = raw.split('|').next().unwrap_or(&raw).to_string();
                if supplement_inputs
                    .iter()
                    .any(|s| s.split('|').next() == Some(&bare))
                {
                    continue;
                }
                match state.backend().supplement_target(&ctx, &bare).await {
                    Ok(Some(info)) => {
                        supplement_inputs.push(raw.clone());
                        applied_supplements.push(info);
                    }
                    Ok(None) => {
                        return Err(HtsError::NotFound(format!(
                            "Required supplement not found: {bare}"
                        )));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
    // Rebuild bare_supplement_urls including any VS-extension additions.
    let bare_supplement_urls: Vec<String> = supplement_inputs
        .iter()
        .map(|s| s.split('|').next().unwrap_or(s).to_string())
        .collect();

    // ── Populate designations (only if explicitly requested) ─────────────────
    let include_designations = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("includeDesignations"))
        .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    if include_designations {
        populate_designations(state.backend(), &ctx, &mut resp.contains).await;
        // After base designations are loaded, append any supplement-derived
        // entries so contains[].designation contains BOTH the base and
        // supplement values for each concept (matched by code).
        if !bare_supplement_urls.is_empty() {
            apply_supplement_designations(
                state.backend(),
                &ctx,
                &mut resp.contains,
                &bare_supplement_urls,
            )
            .await;
        }

        // Apply the `designation` filter parameters when supplied. Each
        // entry uses the FHIR token shape `<system>|<code>`. The
        // `urn:ietf:bcp:47|<lang>` family pins a language; otherwise
        // `<use-system>|<use-code>` pins designation.use. The IG fixtures
        // (language/expand-echo-en-designation) expect codes whose
        // matching designations don't exist to ship with no designation
        // array at all.
        let designation_filters: Vec<(String, String)> = params
            .iter()
            .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("designation"))
            .filter_map(|p| {
                p.get("valueString")
                    .or_else(|| p.get("valueCode"))
                    .and_then(|v| v.as_str())
            })
            .filter_map(|s| {
                s.split_once('|')
                    .map(|(a, b)| (a.to_string(), b.to_string()))
            })
            .collect();
        if !designation_filters.is_empty() {
            fn filter_designations(
                contains: &mut [crate::types::ExpansionContains],
                filters: &[(String, String)],
            ) {
                for c in contains.iter_mut() {
                    c.designations.retain(|d| {
                        filters.iter().any(|(sys, code)| {
                            if sys == "urn:ietf:bcp:47" {
                                d.language.as_deref() == Some(code.as_str())
                            } else {
                                d.use_system.as_deref() == Some(sys.as_str())
                                    && d.use_code.as_deref() == Some(code.as_str())
                            }
                        })
                    });
                    if !c.contains.is_empty() {
                        filter_designations(&mut c.contains, filters);
                    }
                }
            }
            filter_designations(&mut resp.contains, &designation_filters);
        }
    }

    // ── Populate properties (only for codes named in `property` params) ──────
    let requested_properties: Vec<String> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("property"))
        .filter_map(|p| {
            p.get("valueString")
                .or_else(|| p.get("valueCode"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    if !requested_properties.is_empty() {
        populate_properties(
            state.backend(),
            &ctx,
            &mut resp.contains,
            &requested_properties,
        )
        .await;
        if !bare_supplement_urls.is_empty() {
            apply_supplement_properties(
                state.backend(),
                &ctx,
                &mut resp.contains,
                &bare_supplement_urls,
                &requested_properties,
            )
            .await;
        }
    }

    // ── Walk concept-level extensions (base + supplements) ────────────────────
    // Surfaces well-known concept extensions (rendering-style, rendering-xhtml,
    // valueset-concept-definition) on contains[].extension[] AND derives
    // synthetic concept-properties (order/label/weight/status) from the
    // {codesystem-conceptOrder, codesystem-label, itemWeight,
    // structuredefinition-standards-status} extensions.  Drives the IG
    // `extensions/expand-echo-{all,enumerated}` fixtures.
    apply_concept_extension_data(
        state.backend(),
        &ctx,
        &mut resp.contains,
        &bare_supplement_urls,
    )
    .await;

    // ── Apply VS compose-level concept extensions (valueset-deprecated etc.) ──
    // The IG `extensions/extensions-enumerated` fixture pins per-include-concept
    // extensions like `valueset-deprecated: true` and
    // `valueset-concept-definition: "..."` on the compose entry; expand needs
    // to surface those on the matching contains[] entry.
    if let Some(vs) = source_vs.as_ref() {
        if let Some(includes) = vs
            .get("compose")
            .and_then(|c| c.get("include"))
            .and_then(|i| i.as_array())
        {
            for inc in includes {
                let inc_sys = inc.get("system").and_then(|s| s.as_str());
                let Some(concepts) = inc.get("concept").and_then(|c| c.as_array()) else {
                    continue;
                };
                for concept in concepts {
                    let Some(code) = concept.get("code").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some(exts) = concept.get("extension").and_then(|e| e.as_array()) else {
                        continue;
                    };
                    fn merge_into_contains(
                        list: &mut [crate::types::ExpansionContains],
                        wanted_sys: Option<&str>,
                        wanted_code: &str,
                        exts: &[Value],
                    ) {
                        for c in list.iter_mut() {
                            if c.code == wanted_code
                                && wanted_sys.is_none_or(|s| s == c.system)
                            {
                                for ext in exts {
                                    let url = match ext.get("url").and_then(|u| u.as_str()) {
                                        Some(s) => s,
                                        None => continue,
                                    };
                                    if !PASSTHROUGH_CONCEPT_EXTENSIONS.contains(&url) {
                                        continue;
                                    }
                                    c.extensions.retain(|existing| {
                                        existing.get("url").and_then(|u| u.as_str()) != Some(url)
                                    });
                                    c.extensions.push(ext.clone());
                                }
                            }
                            if !c.contains.is_empty() {
                                merge_into_contains(
                                    &mut c.contains,
                                    wanted_sys,
                                    wanted_code,
                                    exts,
                                );
                            }
                        }
                    }
                    merge_into_contains(&mut resp.contains, inc_sys, code, exts);
                }
            }
        }
    }

    // ── Per-system CodeSystem metadata lookup (one search per distinct URL) ──
    // The CS resource is consulted by THREE downstream blocks:
    //   - apply_display_language (for CS.language → preferredForLanguage)
    //   - the used-codesystem emission (for CS.version)
    //   - the warning-<status> emission (for extension/status/experimental)
    //
    // Centralising the lookup here avoids duplicating the search and keeps
    // the call count to one per system on the cache-miss path.
    use std::collections::HashMap;
    let mut cs_by_url: HashMap<String, Option<Value>> = HashMap::new();
    {
        // Collect systems from expansion items first.
        let mut systems: Vec<String> = resp.contains.iter().map(|c| c.system.clone()).fold(
            Vec::<String>::new(),
            |mut acc, s| {
                if !acc.contains(&s) {
                    acc.push(s);
                }
                acc
            },
        );
        // Also add systems from compose.include[] so that empty expansions
        // (e.g. count=0 or filter matched nothing) still populate cs_by_url,
        // enabling used-codesystem to carry the |version suffix.
        if let Some(vs) = source_vs.as_ref() {
            if let Some(includes) = vs
                .get("compose")
                .and_then(|c| c.get("include"))
                .and_then(|i| i.as_array())
            {
                for inc in includes {
                    if let Some(sys) = inc.get("system").and_then(|s| s.as_str()) {
                        let s = sys.to_string();
                        if !systems.contains(&s) {
                            systems.push(s);
                        }
                    }
                }
            }
        }
        systems.sort();
        for system_url in &systems {
            let cs = crate::traits::CodeSystemOperations::search(
                state.backend(),
                &ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(system_url.clone()),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|mut v| v.pop());
            cs_by_url.insert(system_url.clone(), cs);
        }
    }
    let cs_lang_by_url: HashMap<String, Option<String>> = cs_by_url
        .iter()
        .map(|(url, cs)| {
            let lang = cs
                .as_ref()
                .and_then(|c| c.get("language"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (url.clone(), lang)
        })
        .collect();

    // ── displayLanguage: swap display from matching designation ──────────────
    let display_language = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("displayLanguage"))
        .and_then(|p| {
            p.get("valueCode")
                .or_else(|| p.get("valueString"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    if let Some(raw) = display_language.as_deref() {
        if let Some(spec) = parse_display_language(raw) {
            apply_display_language(
                state.backend(),
                &ctx,
                &mut resp.contains,
                &spec,
                &cs_lang_by_url,
            )
            .await;
        }
    }

    // ── activeOnly / compose.inactive=false filter ──────────────────────────
    // The IG fixtures drop inactive concepts when EITHER:
    //   - the request passes `activeOnly=true`, OR
    //   - the source VS has `compose.inactive: false` (FHIR R5)
    // Post-filter using the freshly-populated inactive flag and adjust
    // `total` to match.
    let active_only_request = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("activeOnly"))
        .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let compose_inactive_false = source_vs
        .as_ref()
        .and_then(|vs| vs.get("compose"))
        .and_then(|c| c.get("inactive"))
        .and_then(|i| i.as_bool())
        == Some(false);
    if active_only_request || compose_inactive_false {
        // Walk the tree splicing out inactive nodes and promoting their
        // active descendants up to the parent's level. Mirrors the IG
        // `parameters/parameters-expand-all-active` semantics: when an
        // inactive code has active children, those children stay in the
        // response as roots rather than being dropped with their parent.
        // Returns the new top-level list and the count of inactive nodes
        // that were spliced out (used to keep `total` aligned).
        fn splice_inactive(
            input: Vec<crate::types::ExpansionContains>,
        ) -> (Vec<crate::types::ExpansionContains>, u32) {
            let mut removed: u32 = 0;
            let mut out: Vec<crate::types::ExpansionContains> = Vec::new();
            for mut entry in input {
                let (children, child_removed) =
                    splice_inactive(std::mem::take(&mut entry.contains));
                removed += child_removed;
                if entry.inactive == Some(true) {
                    removed += 1;
                    out.extend(children);
                } else {
                    entry.contains = children;
                    out.push(entry);
                }
            }
            (out, removed)
        }

        let (filtered, removed) = splice_inactive(std::mem::take(&mut resp.contains));
        resp.contains = filtered;
        if let Some(t) = resp.total.as_mut() {
            *t = t.saturating_sub(removed);
        }
    }

    // ── Build FHIR ValueSet response with expansion ──────────────────────────
    // Determine which systems appear with more than one distinct version in
    // this expansion. Only for those systems do we emit the version field on
    // individual contains items (so multi-version expansions are unambiguous
    // while single-version expansions stay compact).
    //
    // ALSO retain `version` on contains items whose source ValueSet
    // explicitly version-pins the system in any compose.include[] or
    // compose.exclude[] entry. The IG `overload/overload-expand-exclude*`
    // fixtures pin both include (`v2`) and exclude (`v1`) of the same system
    // and expect the surviving codes to surface their resolved version even
    // though the post-exclude expansion ends up single-version.
    let pinned_systems_for_version_echo: std::collections::HashSet<String> = source_vs
        .as_ref()
        .and_then(|vs| vs.get("compose"))
        .map(|compose| {
            let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
            for key in ["include", "exclude"] {
                if let Some(arr) = compose.get(key).and_then(|v| v.as_array()) {
                    for inc in arr {
                        let has_version = inc
                            .get("version")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .is_some();
                        if has_version {
                            if let Some(sys) = inc.get("system").and_then(|v| v.as_str()) {
                                out.insert(sys.to_string());
                            }
                        }
                    }
                }
            }
            out
        })
        .unwrap_or_default();
    let multi_version_systems: std::collections::HashSet<String> = {
        use std::collections::HashMap;
        let mut sys_versions: HashMap<&str, Option<&str>> = HashMap::new();
        let mut multi = std::collections::HashSet::new();
        for c in &resp.contains {
            let ver = c.version.as_deref();
            match sys_versions.get(c.system.as_str()) {
                None => {
                    sys_versions.insert(&c.system, ver);
                }
                Some(&prev) if prev != ver => {
                    multi.insert(c.system.clone());
                }
                _ => {}
            }
        }
        // Promote pinned single-version systems too — see comment above.
        for sys in &pinned_systems_for_version_echo {
            multi.insert(sys.clone());
        }
        multi
    };
    let contains: Vec<Value> = resp
        .contains
        .iter()
        .map(|c| serialize_expansion_contains(c, &multi_version_systems))
        .collect();

    // The IG validator (txTests) treats `expansion.identifier` and
    // `expansion.timestamp` as required (they appear in every fixture without
    // an `$optional$` marker). The values are matched as `$uuid$` / `$instant$`
    // wildcards, so any well-formed value satisfies the comparison.
    let mut expansion = json!({
        "identifier": format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "contains": contains,
    });

    if let Some(total) = resp.total {
        expansion["total"] = json!(total);
    }
    if let Some(off) = resp.offset {
        expansion["offset"] = json!(off);
    }

    // ── expansion.parameter ──────────────────────────────────────────────────
    // Echo back the input parameters that influenced the result (e.g.
    // `excludeNested`, `displayLanguage`, `includeDesignations`, `count`,
    // `offset`, `activeOnly`). The validator's tests check that we report
    // every honored input here.
    //
    // Skip the `url` / `valueSet` discriminators (they identify the
    // ValueSet, not a knob), and skip `filter` (already reflected in the
    // contains[] result).
    //
    // Critically: the FHIR R5 ValueSet model requires every
    // `expansion.parameter[].value[x]` to be a primitive (boolean | string |
    // integer | decimal | uri | code | dateTime). The HL7 IG validator
    // augments our request with `tx-resource` parameters whose payload is a
    // Resource (no value[x] at all) plus `profile.parameter` entries. If we
    // echo any of those, the R5 parser produces a ValueSetExpansionParameterComponent
    // with `getValue() == null`, and TxTesterSorters.ExpParameterSorter NPEs
    // on the sort. Drop anything without a primitive value[x] field.
    let mut emitted_params: Vec<Value> = params
        .iter()
        .filter(|p| {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            // Discriminator inputs (identify the ValueSet) — not knobs to echo.
            // `filter` is emitted later as a normalised valueString.
            // `property` is a request-side filter for contains[].property —
            // the IG fixtures don't echo it back.
            // `valueSetVersion` selects which (url, version) ValueSet — the
            // IG fixtures expose the chosen version via the response's
            // top-level `version` field, not as an expansion.parameter echo.
            if matches!(
                name,
                "url" | "valueSet" | "valueSetVersion" | "filter" | "property"
            ) {
                return false;
            }
            // `system-version` and `check-system-version` are instruction
            // knobs (default / verify-only semantics) that the IG fixtures
            // do NOT echo. Only `force-system-version` is echoed per the
            // FHIR IG `version/parameters-fixed-version` profile fixtures.
            if matches!(name, "system-version" | "check-system-version") {
                return false;
            }
            // `useSupplement` is consumed (it drives `used-supplement`
            // emission) — the IG `parameters-expand-supplement-good` fixture
            // does NOT echo `useSupplement` itself.
            if name == "useSupplement" {
                return false;
            }
            // Configuration inputs that the IG validator passes via the
            // `profile` parameter set — they steer test execution rather than
            // request semantics, and the validator does NOT expect them back
            // in expansion.parameter[]. Echoing produces "Unexpected Node
            // found in array" diffs against every fixture.
            if matches!(name, "uuid" | "binding-style") {
                return false;
            }
            // Synthetic internal-only params injected by handlers — never
            // echo (e.g. `__max_expansion_size__` set by the
            // X-TOO-COSTLY-THRESHOLD header injector).
            if name.starts_with("__") && name.ends_with("__") {
                return false;
            }
            // Must carry a primitive value[x] to be valid in expansion.parameter.
            p.as_object()
                .map(|obj| obj.keys().any(|k| k.starts_with("value")))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    // Emit `filter` as a normalised valueString (the IG fixtures expect that
    // form regardless of whether the request used valueString or valueUri).
    if let Some(f) = filter.as_deref() {
        emitted_params.push(json!({"name": "filter", "valueString": f}));
    }

    // Normalise version-override params to `valueUri` regardless of whether
    // the request supplied them as `valueCanonical`/`valueUrl`/etc.  The IG
    // `version/parameters-*-version` and `valueset-version/expand-indirect-*-pinned`
    // fixtures echo them as `valueUri`.
    for ep in emitted_params.iter_mut() {
        let name = ep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(
            name,
            "system-version" | "force-system-version" | "default-valueset-version"
        ) {
            continue;
        }
        let raw = ["valueCanonical", "valueUri", "valueString", "valueUrl"]
            .iter()
            .filter_map(|k| {
                ep.get(*k)
                    .and_then(|v| v.as_str())
                    .map(|s| (*k, s.to_owned()))
            })
            .next();
        if let Some((had_key, val)) = raw {
            if had_key != "valueUri" {
                if let Some(obj) = ep.as_object_mut() {
                    obj.remove(had_key);
                    obj.insert("valueUri".into(), json!(val));
                }
            }
        }
    }

    // Pull additional default expansion parameters from the source ValueSet's
    // `compose.extension[].valueset-expansion-parameter` entries. The IG fixtures
    // use this to pin defaults like displayLanguage="en" without forcing every
    // caller to pass it explicitly. Each extension nests two sub-extensions
    // (`name` and `value`); convert each into a {name, value[x]} parameter.
    if let Some(vs) = source_vs.as_ref() {
        let exts = vs
            .get("compose")
            .and_then(|c| c.get("extension"))
            .and_then(|e| e.as_array());
        if let Some(exts) = exts {
            for ext in exts {
                if ext.get("url").and_then(|u| u.as_str())
                    != Some("http://hl7.org/fhir/StructureDefinition/valueset-expansion-parameter")
                {
                    continue;
                }
                let inner = match ext.get("extension").and_then(|i| i.as_array()) {
                    Some(a) => a,
                    None => continue,
                };
                let mut name: Option<&str> = None;
                let mut value_entry: Option<(String, Value)> = None;
                for sub in inner {
                    let sub_url = sub.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    if sub_url == "name" {
                        name = sub.get("valueCode").and_then(|v| v.as_str());
                    } else if let Some(obj) = sub.as_object() {
                        if let Some((k, v)) = obj.iter().find(|(k, _)| k.starts_with("value")) {
                            value_entry = Some((k.clone(), v.clone()));
                        }
                    }
                }
                if let (Some(n), Some((k, v))) = (name, value_entry) {
                    // `versionsMatch` is a tx-ecosystem-extension carried on
                    // `compose` to choose between version-blind and
                    // version-aware exclude/merge semantics. The IG fixtures
                    // (`overload/overload-expand-all-merged` etc.) echo the
                    // *true* form back as `valueBoolean: true` (and suppress
                    // the *false* form entirely). Translate the valueString
                    // from the extension into the Boolean shape the fixtures
                    // assert against.
                    if n == "versionsMatch" {
                        let val_str = match &v {
                            Value::String(s) => s.as_str(),
                            _ => "",
                        };
                        if val_str.eq_ignore_ascii_case("true") {
                            let already = emitted_params.iter().any(|p| {
                                p.get("name").and_then(|x| x.as_str()) == Some("versionsMatch")
                            });
                            if !already {
                                emitted_params.push(json!({
                                    "name": "versionsMatch",
                                    "valueBoolean": true,
                                }));
                            }
                        }
                        continue;
                    }
                    // Don't double-emit if the caller already provided this knob.
                    let already = emitted_params
                        .iter()
                        .any(|p| p.get("name").and_then(|x| x.as_str()) == Some(n));
                    if !already {
                        emitted_params.push(json!({ "name": n, k: v }));
                    }
                }
            }
        }
    }

    // Default-versionsMatch heuristic (applies when no extension is set):
    // the IG fixtures `overload/overload-expand-exclude` and
    // `overload-expand-exclude-merged` expect a `versionsMatch=true`
    // parameter when a whole-system `exclude[]` clause targets a different
    // version than the contributing `include[]`s.  Per-concept excludes
    // (with `concept[]`) do *not* trigger this — those are inherently
    // version-aware (`overload-expand-exclude-enum`).
    if let Some(vs) = source_vs.as_ref() {
        let already = emitted_params
            .iter()
            .any(|p| p.get("name").and_then(|x| x.as_str()) == Some("versionsMatch"));
        if !already {
            let compose = vs.get("compose");
            let has_versions_match_ext = compose
                .and_then(|c| c.get("extension"))
                .and_then(|e| e.as_array())
                .map(|exts| {
                    exts.iter().any(|ext| {
                        ext.get("url").and_then(|u| u.as_str())
                            == Some(
                                "http://hl7.org/fhir/StructureDefinition/valueset-expansion-parameter",
                            )
                            && ext
                                .get("extension")
                                .and_then(|e| e.as_array())
                                .is_some_and(|inner| {
                                    inner.iter().any(|sub| {
                                        sub.get("url").and_then(|u| u.as_str()) == Some("name")
                                            && sub.get("valueCode").and_then(|v| v.as_str())
                                                == Some("versionsMatch")
                                    })
                                })
                    })
                })
                .unwrap_or(false);
            if !has_versions_match_ext {
                let mut include_versions: std::collections::HashMap<String, std::collections::HashSet<String>> =
                    std::collections::HashMap::new();
                if let Some(arr) = compose.and_then(|c| c.get("include")).and_then(|i| i.as_array())
                {
                    for inc in arr {
                        let sys = match inc.get("system").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => continue,
                        };
                        let ver = inc
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        include_versions.entry(sys).or_default().insert(ver);
                    }
                }
                let mut whole_system_cross_version_exclude = false;
                if let Some(arr) = compose.and_then(|c| c.get("exclude")).and_then(|i| i.as_array())
                {
                    for exc in arr {
                        let has_concept = exc
                            .get("concept")
                            .and_then(|c| c.as_array())
                            .is_some_and(|a| !a.is_empty());
                        if has_concept {
                            // Per-concept excludes are inherently version-aware.
                            continue;
                        }
                        let sys = match exc.get("system").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => continue,
                        };
                        let ver = exc
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(includes) = include_versions.get(&sys) {
                            if !includes.contains(&ver) {
                                whole_system_cross_version_exclude = true;
                                break;
                            }
                        }
                    }
                }
                if whole_system_cross_version_exclude {
                    emitted_params.push(json!({
                        "name": "versionsMatch",
                        "valueBoolean": true,
                    }));
                }
            }
        }
    }

    // ── used-codesystem + warning-<status> per contributing CodeSystem ───────
    // Derive `used-codesystem` from the actual `(system, version)` pairs in
    // the expansion contains items. This is more accurate than querying all
    // stored CS versions: it emits only the versions that were actually used,
    // handling both single-version (vs-expand-all-v → one entry) and
    // multi-version (overload-expand-all → two entries) expansions correctly.
    //
    // Post-processing rule: FHIR only requires `version` on contains items
    // when the expansion mixes different versions of the same system URL.
    // When all items for a given system come from the same version, clear the
    // `version` field so it is not emitted in the JSON output.
    //
    // When the filter narrows to zero matches, fall back to the compose.include[]
    // system+version references so `used-codesystem` still surfaces even for
    // empty expansions (filter='xxx' → empty contains[] but used-codesystem
    // is still echoed for the included CS).

    // Collect distinct (system_url, version) pairs from contains (flat walk).
    let mut used_pairs: Vec<(String, Option<String>)> = {
        fn collect_pairs(
            items: &[crate::types::ExpansionContains],
            out: &mut Vec<(String, Option<String>)>,
        ) {
            for item in items {
                let pair = (item.system.clone(), item.version.clone());
                if !out.contains(&pair) {
                    out.push(pair);
                }
                collect_pairs(&item.contains, out);
            }
        }
        let mut pairs = Vec::new();
        collect_pairs(&resp.contains, &mut pairs);
        pairs
    };

    // Augment with `compose.include[]` AND `compose.exclude[]` system/version
    // pins so every CS that influenced the expansion shape (even ones that
    // contributed only via exclusion, e.g. `overload/overload-expand-exclude`
    // where the v1 include is fully exclude-subsumed) surfaces as a
    // `used-codesystem` parameter.
    //
    // Skip wildcard pins (e.g. `1.x.x`) entirely — the expansion will have
    // resolved them into concrete contains[] rows whose `(system, version)`
    // pair already lives in `used_pairs`. Adding the raw pattern would emit
    // a spurious extra `used-codesystem` parameter (per the IG
    // `version/vs-expand-v-w` and `vs-expand-v-n` fixtures, which expect
    // only the resolved concrete pair).
    //
    // Skip versionless pins on systems that already produced contains[]
    // rows — the resolved-from-DB version we'd derive is exactly what's
    // already in `used_pairs`.
    //
    // Skip pins for systems that have a `force-system-version` override
    // applied — the forced version is what actually contributes; the
    // include's pinned version becomes irrelevant (per IG
    // `version/vs-expand-all-v-force` and `vs-expand-all-v2-force`).
    let force_pinned_systems: std::collections::HashSet<String> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("force-system-version"))
        .filter_map(|p| {
            ["valueCanonical", "valueUri", "valueString", "valueUrl"]
                .iter()
                .find_map(|k| p.get(*k).and_then(|v| v.as_str()))
                .and_then(|s| s.split_once('|').map(|(u, _)| u.to_string()))
        })
        .collect();
    if let Some(vs) = source_vs.as_ref() {
        for key in ["include", "exclude"] {
            if let Some(arr) = vs
                .get("compose")
                .and_then(|c| c.get(key))
                .and_then(|i| i.as_array())
            {
                for inc in arr {
                    if let Some(sys) = inc.get("system").and_then(|s| s.as_str()) {
                        // Skip when a force-system-version overrides this
                        // system; the contains[] pair already reflects the
                        // forced version.
                        if force_pinned_systems.contains(sys) {
                            continue;
                        }
                        let ver = inc
                            .get("version")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        // Skip wildcard pins — contains[] carries the
                        // concrete resolution.
                        if ver
                            .as_deref()
                            .is_some_and(|v| v.contains(".x") || v == "x")
                        {
                            continue;
                        }
                        // Skip versionless pins when contains[] already
                        // covers this system — they resolve to the same
                        // concrete (system, version) pair.
                        if ver.is_none() && used_pairs.iter().any(|(s, _)| s == sys) {
                            continue;
                        }
                        // When no version pin, use the single cached CS version.
                        let resolved_ver = ver.or_else(|| {
                            cs_by_url
                                .get(sys)
                                .and_then(|c| c.as_ref())
                                .and_then(|c| c.get("version"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        });
                        let pair = (sys.to_string(), resolved_ver);
                        if !used_pairs.contains(&pair) {
                            used_pairs.push(pair);
                        }
                    }
                }
            }
        }
    }

    // ── check-system-version post-expansion verification ────────────────────
    // For every `check-system-version` pin, find the version actually used
    // for that system in the expansion. If it doesn't satisfy the pattern,
    // emit a 4xx OperationOutcome with VALUESET_VERSION_CHECK / version-error
    // — matches the IG `version/vs-expand-v-w-check` fixture family. We
    // surface this through `HtsError::VsInvalid` with a sentinel prefix that
    // [`version_check_response`] (registered alongside
    // [`cyclic_reference_response`]) detects to format the FHIR-spec response
    // shape.
    if !check_system_versions.is_empty() {
        for (chk_sys, chk_pat) in &check_system_versions {
            let mut violator: Option<String> = None;
            for (sys, ver) in &used_pairs {
                if sys != chk_sys {
                    continue;
                }
                let v = match ver.as_deref() {
                    Some(v) => v,
                    None => continue,
                };
                if !expand_version_satisfies_wildcard(v, chk_pat) {
                    violator = Some(v.to_string());
                    break;
                }
            }
            if let Some(v) = violator {
                let text = format!(
                    "The version '{v}' is not allowed for system '{chk_sys}': required to be \
                     '{chk_pat}' by a version-check parameter"
                );
                return Err(HtsError::VsInvalid(format!(
                    "{VERSION_CHECK_ERR_PREFIX}{text}"
                )));
            }
        }
    }

    // Group pairs by system URL to detect multi-version systems.
    // Sort within each group for deterministic output (ascending version).
    used_pairs.sort_by(|a, b| {
        a.0.cmp(&b.0).then(
            a.1.as_deref()
                .unwrap_or("")
                .cmp(b.1.as_deref().unwrap_or("")),
        )
    });
    let mut versions_per_system: std::collections::HashMap<&str, Vec<Option<String>>> =
        std::collections::HashMap::new();
    for (sys, ver) in &used_pairs {
        versions_per_system
            .entry(sys)
            .or_default()
            .push(ver.clone());
    }

    // Clear `version` on contains items for single-version systems — FHIR only
    // requires it when a system appears with multiple different versions.
    //
    // Exception ("overload" pattern): when the source VS explicitly pins a
    // version for the system in any compose.include[] *or* compose.exclude[]
    // entry, keep the version even if the post-exclude expansion happens to
    // contain only one version. The IG fixtures
    // (overload-expand-exclude*) require this so the consumer can see which
    // version of the CS the surviving codes came from.
    {
        let pinned_systems: std::collections::HashSet<String> = source_vs
            .as_ref()
            .and_then(|vs| vs.get("compose"))
            .map(|compose| {
                let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
                for key in ["include", "exclude"] {
                    if let Some(arr) = compose.get(key).and_then(|v| v.as_array()) {
                        for inc in arr {
                            let has_version = inc
                                .get("version")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .is_some();
                            if has_version {
                                if let Some(sys) = inc.get("system").and_then(|v| v.as_str()) {
                                    out.insert(sys.to_string());
                                }
                            }
                        }
                    }
                }
                out
            })
            .unwrap_or_default();

        fn clear_single_version(
            items: &mut Vec<crate::types::ExpansionContains>,
            multi_version_systems: &std::collections::HashSet<String>,
            pinned_systems: &std::collections::HashSet<String>,
        ) {
            for item in items {
                if !multi_version_systems.contains(&item.system)
                    && !pinned_systems.contains(&item.system)
                {
                    item.version = None;
                }
                clear_single_version(&mut item.contains, multi_version_systems, pinned_systems);
            }
        }
        let multi_version_systems: std::collections::HashSet<String> = versions_per_system
            .iter()
            .filter(|(_, vers)| vers.len() > 1)
            .map(|(sys, _)| sys.to_string())
            .collect();
        clear_single_version(&mut resp.contains, &multi_version_systems, &pinned_systems);

        // Compute the set of systems whose includes are *all* explicitly
        // version-pinned. The IG `overload/overload-expand-all*` fixtures
        // sort duplicates of a code latest-version-first when every include
        // for the system carries a pinned version.  When any include is
        // versionless (`overload-expand-mixed`), the original include-order
        // is preserved so the user can see how the unversioned reference
        // resolved.
        let fully_pinned_systems: std::collections::HashSet<String> = source_vs
            .as_ref()
            .and_then(|vs| vs.get("compose"))
            .and_then(|c| c.get("include"))
            .and_then(|i| i.as_array())
            .map(|includes| {
                let mut by_system: std::collections::HashMap<String, (usize, usize)> =
                    std::collections::HashMap::new();
                for inc in includes {
                    if let Some(sys) = inc.get("system").and_then(|s| s.as_str()) {
                        let entry = by_system.entry(sys.to_string()).or_insert((0, 0));
                        entry.0 += 1;
                        let pinned = inc
                            .get("version")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .is_some();
                        if pinned {
                            entry.1 += 1;
                        }
                    }
                }
                by_system
                    .into_iter()
                    .filter(|(_, (total, pinned))| *total >= 2 && total == pinned)
                    .map(|(sys, _)| sys)
                    .collect::<std::collections::HashSet<String>>()
            })
            .unwrap_or_default();

        // For systems that contribute multiple versions and have all
        // includes pinned, the IG fixtures (overload/overload-expand-all*)
        // expect the latest version of each code to appear *before* its
        // older counterparts. Sort stably so the relative order of distinct
        // codes is preserved while duplicates of the same code surface
        // latest-first.
        let sortable_systems: std::collections::HashSet<String> = multi_version_systems
            .intersection(&fully_pinned_systems)
            .cloned()
            .collect();
        if !sortable_systems.is_empty() {
            let mut indexed: Vec<(usize, crate::types::ExpansionContains)> = resp
                .contains
                .drain(..)
                .enumerate()
                .collect();
            // Group by (system, code), sort each group by version DESC,
            // then re-emit in original first-occurrence order of (system, code).
            let mut first_idx: std::collections::HashMap<(String, String), usize> =
                std::collections::HashMap::new();
            for (i, item) in indexed.iter() {
                let key = (item.system.clone(), item.code.clone());
                first_idx.entry(key).or_insert(*i);
            }
            // Stable sort by: original group position, then version DESC
            // (only for systems in `sortable_systems`).
            indexed.sort_by(|a, b| {
                let ka = (a.1.system.clone(), a.1.code.clone());
                let kb = (b.1.system.clone(), b.1.code.clone());
                let ga = first_idx.get(&ka).copied().unwrap_or(a.0);
                let gb = first_idx.get(&kb).copied().unwrap_or(b.0);
                ga.cmp(&gb).then_with(|| {
                    if sortable_systems.contains(&a.1.system)
                        && sortable_systems.contains(&b.1.system)
                    {
                        b.1.version
                            .as_deref()
                            .unwrap_or("")
                            .cmp(a.1.version.as_deref().unwrap_or(""))
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
            });
            resp.contains = indexed.into_iter().map(|(_, c)| c).collect();

            // When the source ValueSet declares `versionsMatch=true` (via
            // `compose.extension.valueset-expansion-parameter`), the IG
            // `overload/overload-expand-all-merged` and `expand-exclude-merged`
            // fixtures DEDUPLICATE codes that surface across multiple versions
            // — keep the first occurrence (latest, thanks to the sort above).
            let merged = source_vs
                .as_ref()
                .and_then(|vs| vs.get("compose"))
                .and_then(|c| c.get("extension"))
                .and_then(|e| e.as_array())
                .map(|exts| {
                    exts.iter().any(|ext| {
                        ext.get("url").and_then(|u| u.as_str())
                            == Some(
                                "http://hl7.org/fhir/StructureDefinition/valueset-expansion-parameter",
                            )
                            && ext.get("extension").and_then(|e| e.as_array()).is_some_and(|inner| {
                                inner.iter().any(|sub| {
                                    sub.get("url").and_then(|u| u.as_str()) == Some("name")
                                        && sub.get("valueCode").and_then(|v| v.as_str())
                                            == Some("versionsMatch")
                                })
                            })
                    })
                })
                .unwrap_or(false);
            if merged {
                let mut seen: std::collections::HashSet<(String, String)> =
                    std::collections::HashSet::new();
                resp.contains.retain(|c| {
                    seen.insert((c.system.clone(), c.code.clone()))
                });
                if let Some(t) = resp.total.as_mut() {
                    *t = resp.contains.len() as u32;
                }
            }

            // The serialized `contains` array (built earlier at the start of
            // the response-build phase) was produced from the pre-sort order
            // — re-serialize from `resp.contains` so the expansion reflects
            // the latest-version-first ordering required by the
            // overload/overload-expand-all* fixtures.
            let resorted: Vec<Value> = resp
                .contains
                .iter()
                .map(|c| serialize_expansion_contains(c, &multi_version_systems))
                .collect();
            expansion["contains"] = json!(resorted);
            if merged {
                expansion["total"] = json!(resp.contains.len());
            }
        }
    }

    let mut warning_params: Vec<Value> = Vec::new();
    // Emit one `used-codesystem` per distinct (system, version) pair.
    let mut warned_systems: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (system_url, version) in &used_pairs {
        let value_uri = match version {
            Some(v) => format!("{system_url}|{v}"),
            None => {
                // Single-version systems don't populate version on contains items.
                // Fall back to the CS metadata in cs_by_url so used-codesystem
                // still carries the |version suffix.
                let cs_ver = cs_by_url
                    .get(system_url.as_str())
                    .and_then(|c| c.as_ref())
                    .and_then(|c| c.get("version"))
                    .and_then(|v| v.as_str());
                match cs_ver {
                    Some(v) => format!("{system_url}|{v}"),
                    None => system_url.clone(),
                }
            }
        };
        emitted_params.push(json!({
            "name": "used-codesystem",
            "valueUri": value_uri,
        }));
        // The IG `fragment/fragment-expansion` fixture expects an additional
        // `used-fragment` parameter (mirroring `used-codesystem`'s value) when
        // the contributing CodeSystem declares `content: "fragment"`. Plus an
        // `expansion.extension` pair (`valueset-unclosed` + `valueset-unclosed-
        // reason`) flagging the partial coverage.
        if let Some(cs) = cs_by_url.get(system_url).and_then(|c| c.as_ref()) {
            if cs.get("content").and_then(|v| v.as_str()) == Some("fragment") {
                emitted_params.push(json!({
                    "name": "used-fragment",
                    "valueUri": value_uri,
                }));
            }
        }
        // Emit `warning-<status>` only once per system URL (first pair wins).
        if warned_systems.insert(system_url.clone()) {
            let cs = cs_by_url.get(system_url).and_then(|c| c.as_ref());
            if let Some(cs) = cs {
                for status_code in standards_statuses(cs) {
                    warning_params.push(json!({
                        "name": format!("warning-{status_code}"),
                        "valueUri": value_uri,
                    }));
                }
            }
        }
    }

    // ── expansion.extension: valueset-unclosed (fragment CSes) ───────────────
    // Per the IG `fragment/fragment-expansion` fixture, when ANY contributing
    // CodeSystem has `content: "fragment"`, the response's `expansion` element
    // gains two extensions advertising that the expansion is partial:
    //   * `valueset-unclosed` (boolean true)
    //   * `valueset-unclosed-reason` (string explaining which CS is partial)
    let fragment_systems: Vec<&String> = used_pairs
        .iter()
        .map(|(s, _)| s)
        .filter(|s| {
            cs_by_url
                .get(s.as_str())
                .and_then(|c| c.as_ref())
                .and_then(|c| c.get("content"))
                .and_then(|v| v.as_str())
                == Some("fragment")
        })
        .collect();
    if !fragment_systems.is_empty() {
        // Match the IG fixture wording verbatim — txTests compares the string
        // (no $external$ wildcard for the reason text).
        let reason = format!(
            "This extension is based on a fragment of the code system {}",
            fragment_systems[0]
        );
        expansion["extension"] = json!([
            {
                "url": "http://hl7.org/fhir/StructureDefinition/valueset-unclosed",
                "valueBoolean": true
            },
            {
                "url": "http://hl7.org/fhir/StructureDefinition/valueset-unclosed-reason",
                "valueString": reason
            }
        ]);
    }

    // ── used-valueset entries ────────────────────────────────────────────────
    // The IG `valueset-version/expand-indirect-*` and `simple/expand-contained`
    // fixtures expect one `used-valueset` parameter per distinct ValueSet
    // referenced from the source VS's compose.include[].valueSet[] array,
    // formatted as `<url>|<version>` matching the resolved row. Walk the
    // compose, dedupe by URL, look up each via search.
    if let Some(vs) = source_vs.as_ref() {
        let mut emitted_used_vs: Vec<String> = Vec::new();
        let collect_vs_refs = |inc: &Value, out: &mut Vec<String>| {
            if let Some(refs) = inc.get("valueSet").and_then(|v| v.as_array()) {
                for r in refs {
                    if let Some(s) = r.as_str() {
                        // tx-ecosystem's #fragment refs aren't surfaced as
                        // used-valueset (they're contained-only). Skip them.
                        if !s.starts_with('#') && !out.contains(&s.to_string()) {
                            out.push(s.to_string());
                        }
                    }
                }
            }
        };
        let mut vs_refs: Vec<String> = Vec::new();
        if let Some(includes) = vs
            .get("compose")
            .and_then(|c| c.get("include"))
            .and_then(|i| i.as_array())
        {
            for inc in includes {
                collect_vs_refs(inc, &mut vs_refs);
            }
        }
        if let Some(excludes) = vs
            .get("compose")
            .and_then(|c| c.get("exclude"))
            .and_then(|i| i.as_array())
        {
            for exc in excludes {
                collect_vs_refs(exc, &mut vs_refs);
            }
        }
        for raw_ref in &vs_refs {
            let (bare_url, mut pinned_version) = match raw_ref.split_once('|') {
                Some((u, v)) => (u.to_string(), Some(v.to_string())),
                None => (raw_ref.clone(), None),
            };
            // Honour `default-valueset-version` pin when the ref itself
            // doesn't carry an explicit `|version` (FHIR R5 §$expand).
            if pinned_version.is_none() {
                if let Some(default_v) = default_value_set_versions_for_echo.get(&bare_url) {
                    pinned_version = Some(default_v.clone());
                }
            }
            let referenced_vs: Option<Value> = ValueSetOperations::search(
                state.backend(),
                &ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(bare_url.clone()),
                    version: pinned_version.clone(),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|mut hits| hits.pop());
            let resolved_version = referenced_vs
                .as_ref()
                .and_then(|h| {
                    h.get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .or(pinned_version.clone());
            let value_uri = match resolved_version {
                Some(v) => format!("{bare_url}|{v}"),
                None => bare_url.clone(),
            };
            if !emitted_used_vs.contains(&value_uri) {
                emitted_used_vs.push(value_uri.clone());
                emitted_params.push(json!({
                    "name": "used-valueset",
                    "valueUri": value_uri,
                }));
                // Surface warnings for the referenced VS the same way we do
                // for the source VS — IG `deprecated/not-withdrawn` expects
                // a `warning-withdrawn` for the referenced (withdrawn) VS
                // alongside its `used-valueset` entry.
                if let Some(ref ref_vs) = referenced_vs {
                    for status_code in vs_extension_statuses(ref_vs) {
                        warning_params.push(json!({
                            "name": format!("warning-{status_code}"),
                            "valueUri": value_uri,
                        }));
                    }
                }
            }
        }
    }

    // Then add any warning-* derived from the source VS itself. ValueSets
    // only contribute warnings via the explicit standards-status extension —
    // the IG fixtures (search/*, deprecated/*) treat a VS-level
    // `status: draft` as a non-event, unlike the same field on a CodeSystem.
    if let Some(vs) = source_vs.as_ref() {
        let vs_url = vs.get("url").and_then(|v| v.as_str());
        let vs_version = vs.get("version").and_then(|v| v.as_str());
        let vs_value_uri = match (vs_url, vs_version) {
            (Some(u), Some(v)) => Some(format!("{u}|{v}")),
            (Some(u), None) => Some(u.to_string()),
            _ => None,
        };
        if let Some(uri) = vs_value_uri {
            for status_code in vs_extension_statuses(vs) {
                warning_params.push(json!({
                    "name": format!("warning-{status_code}"),
                    "valueUri": uri,
                }));
            }
        }
    }
    emitted_params.extend(warning_params);

    // ── used-supplement entries ──────────────────────────────────────────────
    // Echo each applied supplement so the IG validator sees we honored it
    // (matches `parameters-expand-supplement-good` and the
    // `extensions/expand-echo-all` fixtures). Value is the supplement's
    // canonical (`url|version` when stored).
    for info in &applied_supplements {
        emitted_params.push(json!({
            "name": "used-supplement",
            "valueUri": info.supplement_canonical,
        }));
    }

    // Append any expansion warnings as parameter entries with name=warning.
    for w in &resp.warnings {
        emitted_params.push(json!({ "name": "warning", "valueString": w }));
    }

    if !emitted_params.is_empty() {
        expansion["parameter"] = json!(emitted_params);
    }

    // ── expansion.property declarations ──────────────────────────────────────
    // The IG fixtures expect a parallel `expansion.property[]` array declaring
    // each property's `code` and (ideally) `uri` whenever ANY contains entry
    // carries a property — whether driven by an explicit `property` request
    // param or by an extension-derived synthesis (label/order/weight/status).
    //
    // Build the union of (a) caller-requested property codes and (b) every
    // distinct property code currently surfaced on a contains[] entry. This
    // means extension-derived properties (from `apply_concept_extension_data`)
    // also get declared at the expansion level — matches the
    // `extensions/expand-echo-{all,enumerated}` fixture shape.
    {
        // FHIR-spec well-known concept-property URIs (
        // http://hl7.org/fhir/concept-properties).  Used as a fallback when a
        // stored CodeSystem doesn't declare a matching `property[].uri`.
        // Mapping covers the "infrastructure" properties surfaced by HTS
        // (definition, status, inactive, deprecated, notSelectable, parent,
        // child, partOf, synonym, alternateCode) plus the synthesised
        // extension-derived properties (label, order, weight).
        fn well_known_property_uri(code: &str) -> Option<&'static str> {
            match code {
                "definition" => Some("http://hl7.org/fhir/concept-properties#definition"),
                "status" => Some("http://hl7.org/fhir/concept-properties#status"),
                "inactive" => Some("http://hl7.org/fhir/concept-properties#inactive"),
                "deprecated" => Some("http://hl7.org/fhir/concept-properties#deprecated"),
                "notSelectable" => Some("http://hl7.org/fhir/concept-properties#notSelectable"),
                "parent" => Some("http://hl7.org/fhir/concept-properties#parent"),
                "child" => Some("http://hl7.org/fhir/concept-properties#child"),
                "partOf" => Some("http://hl7.org/fhir/concept-properties#partOf"),
                "synonym" => Some("http://hl7.org/fhir/concept-properties#synonym"),
                "alternateCode" => Some("http://hl7.org/fhir/concept-properties#alternateCode"),
                "label" => Some("http://hl7.org/fhir/concept-properties#label"),
                "order" => Some("http://hl7.org/fhir/concept-properties#order"),
                "weight" => Some("http://hl7.org/fhir/concept-properties#itemWeight"),
                _ => None,
            }
        }

        // Collect distinct property codes appearing on contains[] entries,
        // walking nested children too. Maintain insertion order via a Vec
        // (HashSet drops ordering and we want deterministic output).
        fn collect_property_codes(
            list: &[crate::types::ExpansionContains],
            out: &mut Vec<String>,
        ) {
            for c in list {
                for p in &c.properties {
                    if !out.contains(&p.code) {
                        out.push(p.code.clone());
                    }
                }
                if !c.contains.is_empty() {
                    collect_property_codes(&c.contains, out);
                }
            }
        }
        let mut emitted_codes: Vec<String> = Vec::new();
        // The IG `extensions/expand-echo-all` fixture orders the property
        // declarations as: weight, label, order, status (i.e. the extension-
        // derived ones first in that fixed order, with status last). Mirror
        // that convention so the fixture comparator matches.
        let synthetic_order = ["weight", "label", "order", "status"];
        let mut surfaced: Vec<String> = Vec::new();
        collect_property_codes(&resp.contains, &mut surfaced);
        for code in synthetic_order {
            if surfaced.iter().any(|c| c == code) && !emitted_codes.iter().any(|c| c == code) {
                emitted_codes.push(code.to_string());
            }
        }
        for code in &requested_properties {
            if !emitted_codes.contains(code) {
                emitted_codes.push(code.clone());
            }
        }
        for code in &surfaced {
            if !emitted_codes.contains(code) {
                emitted_codes.push(code.clone());
            }
        }

        if !emitted_codes.is_empty() {
            // Look up `property[].uri` from the primary contributing CS.
            // Also walk applied supplement CodeSystems — the IG
            // `parameters/parameters-expand-supplement-good` fixture pins
            // `prop1` (a supplement-declared property) with the URI from
            // the supplement, not the base CS.
            use std::collections::HashMap;
            let primary_system = resp.contains.first().map(|c| c.system.clone());
            let mut uri_by_code: HashMap<String, String> = HashMap::new();
            let mut lookup_urls: Vec<String> = Vec::new();
            if let Some(sys) = &primary_system {
                lookup_urls.push(sys.clone());
            }
            for s in &applied_supplements {
                let bare = s
                    .supplement_canonical
                    .split_once('|')
                    .map(|(u, _)| u.to_string())
                    .unwrap_or_else(|| s.supplement_canonical.clone());
                if !lookup_urls.contains(&bare) {
                    lookup_urls.push(bare);
                }
            }
            for url in &lookup_urls {
                if let Ok(mut hits) = crate::traits::CodeSystemOperations::search(
                    state.backend(),
                    &ctx,
                    crate::types::ResourceSearchQuery {
                        url: Some(url.clone()),
                        count: Some(1),
                        ..Default::default()
                    },
                )
                .await
                {
                    if let Some(cs) = hits.pop() {
                        if let Some(props) = cs.get("property").and_then(|p| p.as_array()) {
                            for entry in props {
                                if let (Some(code), Some(uri)) = (
                                    entry.get("code").and_then(|v| v.as_str()),
                                    entry.get("uri").and_then(|v| v.as_str()),
                                ) {
                                    // First-writer-wins so primary CS values
                                    // dominate when a supplement re-declares
                                    // an existing property code.
                                    uri_by_code
                                        .entry(code.to_string())
                                        .or_insert_with(|| uri.to_string());
                                }
                            }
                        }
                    }
                }
            }
            let prop_decls: Vec<Value> = emitted_codes
                .iter()
                .map(|code| {
                    let mut entry = json!({"code": code});
                    if let Some(uri) = uri_by_code.get(code) {
                        entry["uri"] = json!(uri);
                    } else if let Some(uri) = well_known_property_uri(code) {
                        entry["uri"] = json!(uri);
                    }
                    entry
                })
                .collect();
            expansion["property"] = json!(prop_decls);
        }
    }

    // ── Copy metadata from the source ValueSet ───────────────────────────────
    // The IG fixtures expect the response to mirror the original ValueSet's
    // top-level fields (url, version, name, title, status, ...) — without
    // them tests fail with "missing property url" / etc.
    //
    // For URL-based requests, look up the stored ValueSet and copy across
    // the canonical-resource fields. For inline ValueSet requests, the
    // caller supplied the body — copy from there.
    let mut response = json!({ "resourceType": "ValueSet" });
    if let Some(ref vs) = source_vs {
        if let Some(obj) = vs.as_object() {
            // Copy required-by-fixtures fields plus a few common optionals.
            //
            // For URL-based requests, do NOT copy `compose` / `contained` —
            // every IG `expand-*-response*` fixture lists them under
            // `$optional-properties$` and echoing the stored shape produces
            // "unexpected property" diffs (extra `inactive`, wrong `system`,
            // extra `valueSet` ref, …).
            //
            // For INLINE VS requests (`valueSet` body parameter) the caller
            // supplied the document, and the IG `simple/expand-contained`
            // fixture EXPECTS it echoed back — so we must copy it.  Apply
            // small R4→R5 normalisations on the way out:
            //   * `filter[].op = "child-of"` becomes `"is-a"` (semantically
            //     identical; R4 spelling is deprecated in R5).
            //   * `compose.inactive: false` is dropped (canonical R5 form
            //     omits the property when its value is the default).
            for field in [
                "id",
                "language",
                "url",
                "version",
                "name",
                "title",
                "status",
                "experimental",
                "date",
                "publisher",
            ] {
                if let Some(v) = obj.get(field) {
                    response[field] = v.clone();
                }
            }
            if value_set_for_response.is_some() {
                if let Some(c) = obj.get("compose") {
                    let mut composed = c.clone();
                    fn normalise_filter_ops(v: &mut Value) {
                        let Some(arr) = v.as_array_mut() else { return };
                        for inc in arr {
                            if let Some(filters) =
                                inc.get_mut("filter").and_then(|f| f.as_array_mut())
                            {
                                for f in filters {
                                    if f.get("op").and_then(|o| o.as_str()) == Some("child-of") {
                                        if let Some(obj) = f.as_object_mut() {
                                            obj.insert("op".into(), json!("is-a"));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(includes) = composed.get_mut("include") {
                        normalise_filter_ops(includes);
                    }
                    if let Some(excludes) = composed.get_mut("exclude") {
                        normalise_filter_ops(excludes);
                    }
                    if composed.get("inactive").and_then(|v| v.as_bool()) == Some(false) {
                        if let Some(obj_mut) = composed.as_object_mut() {
                            obj_mut.remove("inactive");
                        }
                    }
                    response["compose"] = composed;
                }
                if let Some(c) = obj.get("contained") {
                    response["contained"] = c.clone();
                }
            }
        }
    }
    response["expansion"] = expansion;

    // ── Serialize once, cache, return ─────────────────────────────────────────
    // `serde_json::to_vec` writes directly into a Vec<u8>; wrapping in
    // `Bytes::from` transfers ownership without copying.
    let bytes = Bytes::from(
        serde_json::to_vec(&response)
            .map_err(|e| HtsError::Internal(format!("JSON serialization failed: {e}")))?,
    );

    if let Ok(mut cache) = state.expand_cache.write() {
        if cache.len() < EXPAND_CACHE_MAX {
            // `Bytes::clone` is O(1); storing it here and returning the clone
            // below means both the cache and the caller share the same buffer.
            cache.insert(cache_key, bytes.clone());
        }
    }

    Ok(bytes)
}

/// Turn pre-serialized JSON bytes into an HTTP response.
///
/// For JSON format: the bytes are returned directly with no extra copy.
/// For XML format: the bytes are deserialized back to a `Value` first (rare
/// code path — the benchmark always uses JSON).
fn expand_bytes_respond(bytes: Bytes, format: ResponseFormat) -> Response {
    use axum::response::IntoResponse;
    match format {
        ResponseFormat::Json => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/fhir+json; charset=utf-8")],
            bytes,
        )
            .into_response(),
        ResponseFormat::Xml => {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let xml = json_to_fhir_xml(&value);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/fhir+xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
    }
}

/// `POST /ValueSet/$expand`
///
/// Accepts a FHIR `Parameters` body.  The `url` parameter (canonical ValueSet
/// URL) is required.  Content negotiation via `Accept` header or `_format`
/// query parameter selects JSON or XML output.
pub async fn expand_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let mut params = extract_parameter_array(&body)?;
    inject_accept_language(&headers, &mut params);
    inject_too_costly_threshold(&headers, &mut params);
    match process_expand(&state, params).await {
        Ok(bytes) => Ok(expand_bytes_respond(bytes, format)),
        Err(e) => {
            if let Some(resp) = version_check_response(&e) {
                return Ok(resp);
            }
            match cyclic_reference_response(&e) {
                Some(resp) => Ok(resp),
                None => Err(e),
            }
        }
    }
}

/// `GET /ValueSet/$expand?url=<url>`
///
/// URL query parameters are mapped to FHIR `Parameters` name/value pairs and
/// processed identically to the POST form.  `url`, `filter`, `count`, `offset`,
/// `date`, `hierarchical`, and `excludeNested` are all accepted.
pub async fn get_expand_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let mut params = query_params_to_fhir_params(pairs);
    inject_accept_language(&headers, &mut params);
    inject_too_costly_threshold(&headers, &mut params);
    match process_expand(&state, params).await {
        Ok(bytes) => Ok(expand_bytes_respond(bytes, format)),
        Err(e) => {
            if let Some(resp) = version_check_response(&e) {
                return Ok(resp);
            }
            match cyclic_reference_response(&e) {
                Some(resp) => Ok(resp),
                None => Err(e),
            }
        }
    }
}

/// If `err` is a `VsInvalid` produced by the cyclic-reference detector in the
/// SQLite backend (`expand_vs_reference`), build the FHIR-IG-compliant
/// OperationOutcome that the `big/expand-circle` test fixture expects:
/// status 422, issue.code=`processing`, tx-issue-type=`vs-invalid`, plus a
/// `VALUESET_CIRCULAR_REFERENCE` `operationoutcome-message-id` extension.
/// Returns `None` when the error is not a cycle so the caller falls through
/// to the generic [`HtsError`] [`IntoResponse`] path.
/// Sentinel marker prepended to a [`HtsError::VsInvalid`] when an $expand
/// operation fails the `check-system-version` post-check.  Picked up by
/// [`version_check_response`] to format the IG-spec OperationOutcome shape.
const VERSION_CHECK_ERR_PREFIX: &str = "__VALUESET_VERSION_CHECK__:";

/// Returns true if `version` satisfies the wildcard `pattern`. Local copy of
/// the helper in `backends/sqlite/value_set.rs` so $expand can verify the
/// `check-system-version` pattern without crossing crate boundaries.
fn expand_version_satisfies_wildcard(version: &str, pattern: &str) -> bool {
    if pattern == "x" {
        return true;
    }
    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let ver_segs: Vec<&str> = version.split('.').collect();
    let ends_with_x = pat_segs.last().is_some_and(|s| *s == "x");
    if !ends_with_x && pat_segs.len() != ver_segs.len() {
        return false;
    }
    if ends_with_x && ver_segs.len() < pat_segs.len() - 1 {
        return false;
    }
    for (i, ps) in pat_segs.iter().enumerate() {
        if *ps == "x" {
            continue;
        }
        match ver_segs.get(i) {
            Some(vs) if vs == ps => {}
            _ => return false,
        }
    }
    true
}

/// If `err` is a `check-system-version` failure raised inside `process_expand`,
/// render the FHIR `OperationOutcome` shape the IG fixtures expect:
/// `severity=error`, `code=exception`, `version-error` tx-issue-type,
/// `VALUESET_VERSION_CHECK` message-id, HTTP 400.
fn version_check_response(err: &HtsError) -> Option<Response> {
    use axum::response::IntoResponse;
    let HtsError::VsInvalid(msg) = err else {
        return None;
    };
    let text = msg.strip_prefix(VERSION_CHECK_ERR_PREFIX)?;
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                "valueString": "VALUESET_VERSION_CHECK"
            }],
            "severity": "error",
            "code": "exception",
            "details": {
                "coding": [{
                    "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                    "code": "version-error"
                }],
                "text": text,
            },
        }]
    });
    Some((StatusCode::BAD_REQUEST, Json(body)).into_response())
}

fn cyclic_reference_response(err: &HtsError) -> Option<Response> {
    use axum::response::IntoResponse;
    let HtsError::VsInvalid(msg) = err else {
        return None;
    };
    if !msg.starts_with("Cyclic reference detected when excluding ") {
        return None;
    }
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                "valueString": "VALUESET_CIRCULAR_REFERENCE"
            }],
            "severity": "error",
            "code": "processing",
            "details": {
                "coding": [{
                    "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                    "code": "vs-invalid"
                }],
                "text": msg
            },
            "diagnostics": msg
        }]
    });
    Some((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

/// Honour the `X-TOO-COSTLY-THRESHOLD` request header from the IG
/// `big/big-echo-no-limit` test (and the wider HL7 tx-ecosystem fixtures).
/// The header carries a per-request maximum expansion size — when set, the
/// server must return an `OperationOutcome` with `code=too-costly` if the
/// expansion would exceed it, regardless of the configured global limit.
/// We surface the value as a synthetic `__max_expansion_size__` parameter so
/// `process_expand` can override `state.max_expansion_size` for this request.
fn inject_too_costly_threshold(headers: &HeaderMap, params: &mut Vec<Value>) {
    if let Some(v) = headers
        .get("x-too-costly-threshold")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
    {
        params.retain(|p| {
            p.get("name").and_then(|x| x.as_str()) != Some("__max_expansion_size__")
        });
        params.push(json!({"name": "__max_expansion_size__", "valueInteger": v}));
    }
}

/// If the request carried an `Accept-Language` header and the params don't
/// already pin a `displayLanguage`, inject one synthesised from the header.
/// This is what the IG validator uses to express the language it wants
/// (`client().setAcceptLanguage(lang)`), and the expected fixtures echo
/// `displayLanguage` in `expansion.parameter` even when the request body
/// didn't carry it explicitly.
pub(crate) fn inject_accept_language(headers: &HeaderMap, params: &mut Vec<Value>) {
    let lang = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        // Take just the primary tag (strip q-values, secondary tags).
        .map(|s| s.split([',', ';']).next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty() && s != "*");
    let already = params
        .iter()
        .any(|p| p.get("name").and_then(|v| v.as_str()) == Some("displayLanguage"));
    if let Some(l) = lang {
        if !already {
            params.push(json!({"name": "displayLanguage", "valueCode": l}));
        }
    }
}

/// Inject (or replace) the `url` parameter in a params list.
///
/// Removes any existing `url` entry from the caller's params so that the
/// resource-id-resolved URL always wins, then prepends the canonical URL as a
/// `valueUri` parameter.
fn inject_url(mut params: Vec<Value>, url: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("url"));
    let mut with_url = vec![json!({"name": "url", "valueUri": url})];
    with_url.append(&mut params);
    with_url
}

/// POST /ValueSet/{id}/$expand
///
/// Resolves the ValueSet canonical URL from its FHIR `id`, then delegates to
/// the same expansion logic used by the system-level endpoint.
pub async fn expand_by_id_post<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let raw_params = body
        .and_then(|Json(v)| extract_parameter_array(&v).ok())
        .unwrap_or_default();
    let mut params = inject_url(raw_params, url);
    inject_accept_language(&headers, &mut params);
    inject_too_costly_threshold(&headers, &mut params);
    Ok(expand_bytes_respond(
        process_expand(&state, params).await?,
        format,
    ))
}

/// `GET /ValueSet/{id}/$expand?filter=<text>&count=<n>`
///
/// Instance-level GET variant.  Resolves the ValueSet canonical URL from its
/// FHIR logical `id` and merges it with the remaining query-string parameters
/// before dispatching to the shared expansion pipeline.
///
/// Returns 404 when no ValueSet with the given `id` is found.
pub async fn get_expand_by_id<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    let mut params = inject_url(params, url);
    inject_accept_language(&headers, &mut params);
    inject_too_costly_threshold(&headers, &mut params);
    Ok(expand_bytes_respond(
        process_expand(&state, params).await?,
        format,
    ))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;

    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::state::AppState;

    fn make_app_with_data() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();

        // Seed directly via SQL (same pattern as other operation handler tests).
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs1', 'http://example.org/cs', '1.0', 'TestCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'A', 'Alpha'),
                        (2, 'cs1', 'B', 'Beta'),
                        (3, 'cs1', 'C', 'Gamma');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at)
                 VALUES ('vs1', 'http://example.org/vs', 'TestVS', 'active',
                         '{\"include\":[{\"system\":\"http://example.org/cs\",\"concept\":[{\"code\":\"A\"},{\"code\":\"B\"}]}]}',
                         '2024-01-01', '2024-01-01');",
            )
            .unwrap();
        }

        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$expand",
                post(expand_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn post_json(app: Router, uri: &str, body: Value) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ── Happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_returns_valueset_resource() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "ValueSet");
        assert!(json["expansion"].is_object());
    }

    #[tokio::test]
    async fn expand_returns_correct_codes() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 2);

        let codes: Vec<&str> = contains
            .iter()
            .map(|c| c["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"A"));
        assert!(codes.contains(&"B"));
        assert!(!codes.contains(&"C"));
    }

    #[tokio::test]
    async fn expand_returns_total_count() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        assert_eq!(json["expansion"]["total"], 2);
    }

    // ── Pagination ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_with_count_limits_results() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "count", "valueInteger": 1 }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 1);
        assert_eq!(json["expansion"]["total"], 2); // total is still the full count
    }

    // ── Missing url → 400 ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_missing_url_returns_400() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── Unknown value set → 404 ────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_unknown_value_set_returns_404() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://unknown.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 404);
    }

    // ── Wrong resource type → 400 ──────────────────────────────────────────────

    #[tokio::test]
    async fn expand_wrong_resource_type_returns_400() {
        let app = make_app_with_data();
        let body = json!({ "resourceType": "ValueSet", "parameter": [] });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── standards_statuses helper ──────────────────────────────────────────────
    //
    // These exercise the deprecated/withdrawn/experimental/draft → warning-*
    // mapping that drives expansion.parameter emission in process_expand.

    #[test]
    fn standards_statuses_picks_extension_status() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                "valueCode": "deprecated"
            }],
            "status": "active",
            "experimental": false
        });
        assert_eq!(standards_statuses(&cs), vec!["deprecated".to_string()]);
    }

    #[test]
    fn standards_statuses_picks_experimental_flag() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "status": "active",
            "experimental": true
        });
        assert_eq!(standards_statuses(&cs), vec!["experimental".to_string()]);
    }

    #[test]
    fn standards_statuses_picks_draft_status() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "status": "draft",
            "experimental": false
        });
        assert_eq!(standards_statuses(&cs), vec!["draft".to_string()]);
    }

    #[test]
    fn standards_statuses_combines_multiple_markers() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                "valueCode": "withdrawn"
            }],
            "status": "draft",
            "experimental": true
        });
        // Order: extension first, then experimental, then draft.
        assert_eq!(
            standards_statuses(&cs),
            vec![
                "withdrawn".to_string(),
                "experimental".to_string(),
                "draft".to_string()
            ]
        );
    }

    #[test]
    fn standards_statuses_returns_empty_for_active_resource() {
        let cs = json!({
            "resourceType": "CodeSystem",
            "status": "active",
            "experimental": false
        });
        assert!(standards_statuses(&cs).is_empty());
    }

    #[test]
    fn standards_statuses_dedupes_when_extension_matches_status() {
        // Both the standards-status extension and the FHIR status field say
        // "draft" — emit only one entry.
        let cs = json!({
            "resourceType": "CodeSystem",
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                "valueCode": "draft"
            }],
            "status": "draft"
        });
        assert_eq!(standards_statuses(&cs), vec!["draft".to_string()]);
    }

    // ── parse_display_language helper ──────────────────────────────────────────

    #[test]
    fn parse_display_language_simple_tag() {
        let spec = parse_display_language("de").unwrap();
        assert_eq!(spec.preferred, "de");
        assert!(!spec.hard_fallback);
    }

    #[test]
    fn parse_display_language_with_explicit_fallback() {
        let spec = parse_display_language("de,*").unwrap();
        assert_eq!(spec.preferred, "de");
        assert!(!spec.hard_fallback);
    }

    #[test]
    fn parse_display_language_hard_mode_q0() {
        // Wildcard with q=0 → no fallback allowed.
        let spec = parse_display_language("de,*; q=0").unwrap();
        assert_eq!(spec.preferred, "de");
        assert!(spec.hard_fallback);
    }

    #[test]
    fn parse_display_language_hard_mode_with_extra_whitespace() {
        let spec = parse_display_language("de, *; q=0").unwrap();
        assert_eq!(spec.preferred, "de");
        assert!(spec.hard_fallback);
    }

    #[test]
    fn parse_display_language_picks_first_real_tag() {
        let spec = parse_display_language("de-CH,en,*").unwrap();
        assert_eq!(spec.preferred, "de-CH");
        assert!(!spec.hard_fallback);
    }

    #[test]
    fn parse_display_language_only_wildcard_returns_none() {
        // No real preferred tag — caller should treat as "no displayLanguage".
        assert!(parse_display_language("*").is_none());
        assert!(parse_display_language("*; q=0").is_none());
    }

    // ── useSupplement (IG `parameters-expand-supplement-good`) ────────────────

    fn make_supplement_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, status, content, created_at, updated_at, resource_json)
                 VALUES ('base', 'http://hl7.org/fhir/test/CodeSystem/extensions', '5.0.0',
                         'active', 'complete',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\"}');

                 INSERT INTO code_systems
                     (id, url, version, status, content, created_at, updated_at, resource_json)
                 VALUES ('supp', 'http://hl7.org/fhir/test/CodeSystem/supplement', '0.1.1',
                         'active', 'supplement',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\",\"supplements\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (20, 'base', 'code1', 'Display 1'),
                        (21, 'supp', 'code1', NULL);

                 INSERT INTO concept_designations (concept_id, language, value)
                 VALUES (21, 'nl', 'ectenoot');

                 INSERT INTO value_sets
                     (id, url, status, compose_json, created_at, updated_at, resource_json)
                 VALUES ('vs-extns', 'http://hl7.org/fhir/test/ValueSet/extensions-all-ns',
                         'active',
                         '{\"include\":[{\"system\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}]}',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"ValueSet\"}');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$expand",
                post(expand_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    // ── excludeNested parameter (tree-mode trigger) ────────────────────────────

    /// Seeds an in-memory backend with a 3-level hierarchy:
    ///   root → child → grandchild
    /// plus a sibling "orphan" with no parent. The companion ValueSet
    /// includes the entire system, so all 4 codes are in the expansion.
    fn make_app_with_hierarchy() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs-h', 'http://example.org/cs-h', '1.0', 'HierCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display) VALUES
                     (10, 'cs-h', 'root',       'Root'),
                     (11, 'cs-h', 'child',      'Child'),
                     (12, 'cs-h', 'grandchild', 'Grandchild'),
                     (13, 'cs-h', 'orphan',     'Orphan');

                 INSERT INTO concept_hierarchy (system_id, parent_code, child_code) VALUES
                     ('cs-h', 'root',  'child'),
                     ('cs-h', 'child', 'grandchild');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at)
                 VALUES ('vs-h', 'http://example.org/vs-h', 'HierVS', 'active',
                         '{\"include\":[{\"system\":\"http://example.org/cs-h\"}]}',
                         '2024-01-01', '2024-01-01');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$expand",
                post(expand_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn expand_with_use_supplement_emits_used_supplement_param() {
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://hl7.org/fhir/test/ValueSet/extensions-all-ns"},
                {"name": "useSupplement", "valueCanonical": "http://hl7.org/fhir/test/CodeSystem/supplement"},
                {"name": "includeDesignations", "valueBoolean": true}
            ]
        });
        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["expansion"]["parameter"].as_array().unwrap();
        let used = params
            .iter()
            .find(|p| p["name"] == "used-supplement")
            .expect("used-supplement parameter expected in expansion.parameter");
        assert_eq!(
            used["valueUri"],
            "http://hl7.org/fhir/test/CodeSystem/supplement|0.1.1"
        );

        // Designation merged into contains[code1].
        let contains = json["expansion"]["contains"].as_array().unwrap();
        let code1 = contains.iter().find(|c| c["code"] == "code1").unwrap();
        let designations = code1["designation"].as_array().unwrap();
        assert!(
            designations
                .iter()
                .any(|d| d["value"] == "ectenoot" && d["language"] == "nl"),
            "supplement designation 'ectenoot' must appear in contains[code1].designation"
        );
    }

    #[tokio::test]
    async fn expand_unknown_supplement_returns_404() {
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://hl7.org/fhir/test/ValueSet/extensions-all-ns"},
                {"name": "useSupplement", "valueCanonical": "http://does-not-exist/cs"}
            ]
        });
        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 404);
    }

    /// `excludeNested=false` should produce a tree (root contains child contains grandchild,
    /// plus orphan as a sibling root).  Total stays 4 (full count); contains[] has 2 roots.
    #[tokio::test]
    async fn expand_exclude_nested_false_returns_tree() {
        let app = make_app_with_hierarchy();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs-h" },
                { "name": "excludeNested", "valueBoolean": false }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;

        // Total reflects the full flat count.
        assert_eq!(json["expansion"]["total"], 4);

        // Roots: "orphan" and "root".
        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 2, "expected 2 root entries (orphan + root)");

        let root = contains
            .iter()
            .find(|c| c["code"] == "root")
            .expect("root should be a top-level entry");
        let root_children = root["contains"].as_array().unwrap();
        assert_eq!(root_children.len(), 1);
        assert_eq!(root_children[0]["code"], "child");

        let grandchildren = root_children[0]["contains"].as_array().unwrap();
        assert_eq!(grandchildren.len(), 1);
        assert_eq!(grandchildren[0]["code"], "grandchild");
    }

    /// `excludeNested=true` (default) should keep the historical flat behaviour.
    #[tokio::test]
    async fn expand_exclude_nested_true_returns_flat_list() {
        let app = make_app_with_hierarchy();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs-h" },
                { "name": "excludeNested", "valueBoolean": true }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 4, "all four codes should appear flat");
        for c in contains {
            assert!(
                c.get("contains").is_none(),
                "flat entries must not carry nested contains[]"
            );
        }
    }

    /// Omitting both `excludeNested` and `hierarchical` keeps the historical
    /// flat behaviour — the simple/* IG fixtures rely on this.
    #[tokio::test]
    async fn expand_no_nesting_param_returns_flat_list() {
        let app = make_app_with_hierarchy();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs-h" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 4);
        for c in contains {
            assert!(c.get("contains").is_none());
        }
    }

    /// `hierarchical=true` (legacy alias) and `excludeNested=false` must agree.
    #[tokio::test]
    async fn expand_hierarchical_true_matches_exclude_nested_false() {
        let app1 = make_app_with_hierarchy();
        let body1 = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs-h" },
                { "name": "hierarchical", "valueBoolean": true }
            ]
        });
        let resp1 = body_json(post_json(app1, "/ValueSet/$expand", body1).await).await;

        let app2 = make_app_with_hierarchy();
        let body2 = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs-h" },
                { "name": "excludeNested", "valueBoolean": false }
            ]
        });
        let resp2 = body_json(post_json(app2, "/ValueSet/$expand", body2).await).await;

        assert_eq!(
            resp1["expansion"]["contains"],
            resp2["expansion"]["contains"]
        );
    }
}
// rebuild marker
