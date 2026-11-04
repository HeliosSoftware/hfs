//! # FHIR document functions
//!
//! `section()`, `entry()`, `entryIn()`, `resources()`, `resourcesIn()`,
//! `sameSubject()` and `hasCoding()`: functions for navigating FHIR documents
//! (a `Bundle` of type `document` whose first entry is a `Composition`).
//!
//! These are **not** part of FHIRPath, nor of the FHIR specification's
//! additional FHIRPath functions. They are an extension of the HL7 Java
//! reference implementation (`org.hl7.fhir.core`, `DocumentFunctions.java`),
//! written for "clinical profiles" of IPS documents: Bundle profiles whose
//! invariants say what a document must contain, e.g.
//! `Bundle.section('http://loinc.org', '11450-4').exists()`. The conformance
//! tests are the `documentFunctions` group (`mode="custom"`) of the FHIRPath
//! suite in `fhir-test-cases`. This port follows the Java implementation, so
//! the two engines agree on what a document profile's invariants mean.
//!
//! - `section(system, code [, min, max])`: on documents or Compositions (a
//!   document's Composition is its first Composition entry), the Composition's
//!   sections, nested ones too, whose code has a coding that matches. `min` and
//!   `max` default to 1 and 1.
//! - `entry(system, code [, min, max])`: on sections, their entries, resolved
//!   in the document, whose code has a coding that matches. `min` and `max`
//!   default to 0 and `*`.
//! - `entryIn(valueSet [, min, max])`: as `entry()`, with a code in the value
//!   set. Empty when `valueSet` is empty.
//! - `resources(system, code [, type])`: on documents, or anything in one,
//!   every resource in the document whose code has a coding that matches, of
//!   that type unless `type` is empty or not given.
//! - `resourcesIn(valueSet [, type])`: as `resources()`, with a code in the
//!   value set. Empty when `valueSet` is empty.
//! - `sameSubject([other])`: whether the focus resource has the same subject
//!   (its `subject` or `patient`) as `other`, or without `other`, as the
//!   document's Composition. See [`same_subject`].
//! - `hasCoding(system, code [, version])`: on Codings and CodeableConcepts,
//!   whether any coding matches. See [`has_coding`].
//!
//! Shared rules:
//!
//! - A coding matches a system and code when each parameter that is a string
//!   equals the coding's value exactly. A parameter that is empty (`{}`) is
//!   ignored.
//! - A resource's code is its `code`, `vaccineCode` or `medication`, when that
//!   is a CodeableConcept or a CodeableReference's `concept`. A medication given
//!   by reference is not followed.
//! - `min` and `max` are both given or neither, and `max` may be `'*'`. An
//!   empty one takes the function's default. `min > max`, or a value that is not
//!   a non-negative integer, is an execution error. An input item whose count
//!   falls outside `min..max` adds nothing to the output; that is not an error.
//! - Every parameter except `sameSubject`'s `other` is a single value or empty.
//!   One with more than one value makes the result empty.
//! - A value set that cannot be found, or a membership the terminology server
//!   cannot decide, is an execution error.
//!
//! References in sections are resolved in the document the evaluation is on,
//! the way the validator resolves references in a Bundle:
//!
//! - an absolute reference by `fullUrl`, and by nothing else;
//! - a relative one (`Type/id`) against the base of the Composition's
//!   `fullUrl`, when that is a RESTful URL;
//! - `#id` as a resource contained in the Composition.
//!
//! A fragment on a reference is ignored. More than one match is no match, and a
//! reference that does not resolve is left out. Without a document (a
//! Composition evaluated on its own) only `#id` references resolve: unlike the
//! Java engine, this one has no host to hand other references to.
//!
//! Where the Java engine compares sections by object identity, this port
//! compares them by value. The results agree unless two Compositions in one
//! document carry identical sections.

use crate::evaluator::EvaluationContext;
use crate::terminology_functions::TerminologyFunctions;
use helios_fhir::FhirVersion;
use helios_fhirpath_support::{EvaluationError, EvaluationResult, IntoEvaluationResult};
use std::collections::HashMap;

/// The elements that hold a resource's code.
const CODE_ELEMENTS: [&str; 3] = ["code", "vaccineCode", "medication"];

/// Evaluates one of the document functions (`section`, `entry`, `entryIn`,
/// `resources`, `resourcesIn`, `sameSubject`, `hasCoding`) on `base`, with its
/// arguments already evaluated.
pub fn call_document_function(
    name: &str,
    base: &EvaluationResult,
    args: &[EvaluationResult],
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    check_arity(name, args.len())?;

    // sameSubject's `other` is the one parameter that may be a collection: it is
    // judged by the function itself, not here.
    if name == "sameSubject" {
        return Ok(boolean_or_empty(same_subject(base, args.first(), context)));
    }

    // Every other parameter is a single value or empty. One with more than one
    // value makes the result empty. This is checked before the parameters are
    // validated, as the Java engine does.
    if args.iter().any(|a| a.count() > 1) {
        return Ok(EvaluationResult::Empty);
    }

    match name {
        "section" => {
            let bounds = Bounds::parse(name, args.get(2), args.get(3), Bounds::ONE)?;
            Ok(collection(section(
                base,
                string_param(&args[0]).as_deref(),
                string_param(&args[1]).as_deref(),
                bounds,
            )))
        }
        "entry" => {
            let bounds = Bounds::parse(name, args.get(2), args.get(3), Bounds::ANY)?;
            let system = string_param(&args[0]);
            let code = string_param(&args[1]);
            let doc = Document::new(context);
            let mut matches = |r: &EvaluationResult| {
                Ok(has_code(
                    r,
                    system.as_deref(),
                    code.as_deref(),
                    context.fhir_version,
                ))
            };
            Ok(collection(entry(base, &doc, bounds, &mut matches)?))
        }
        "entryIn" => {
            // Both parameters are checked even when the value set is empty.
            let url = value_set_param(name, &args[0])?;
            let bounds = Bounds::parse(name, args.get(1), args.get(2), Bounds::ANY)?;
            let Some(url) = url else {
                return Ok(EvaluationResult::Empty);
            };
            let doc = Document::new(context);
            let mut membership = Membership::new(name, url, context);
            let mut matches = |r: &EvaluationResult| membership.contains_code_of(r);
            Ok(collection(entry(base, &doc, bounds, &mut matches)?))
        }
        "resources" => {
            let resource_type = type_param(name, args.get(2))?;
            let system = string_param(&args[0]);
            let code = string_param(&args[1]);
            let doc = Document::new(context);
            let found = document_resources(base, &doc, resource_type.as_deref())
                .into_iter()
                .filter(|r| has_code(r, system.as_deref(), code.as_deref(), context.fhir_version))
                .collect();
            Ok(collection(found))
        }
        "resourcesIn" => {
            // Both parameters are checked even when the value set is empty.
            let url = value_set_param(name, &args[0])?;
            let resource_type = type_param(name, args.get(1))?;
            let Some(url) = url else {
                return Ok(EvaluationResult::Empty);
            };
            let doc = Document::new(context);
            let mut membership = Membership::new(name, url, context);
            let mut found = Vec::new();
            for r in document_resources(base, &doc, resource_type.as_deref()) {
                if membership.contains_code_of(&r)? {
                    found.push(r);
                }
            }
            Ok(collection(found))
        }
        "hasCoding" => Ok(boolean_or_empty(has_coding(
            base,
            string_param(&args[0]).as_deref(),
            string_param(&args[1]).as_deref(),
            args.get(2).and_then(string_param).as_deref(),
        ))),
        _ => Err(EvaluationError::InvalidOperation(format!(
            "'{name}' is not a document function"
        ))),
    }
}

/// The parameter counts each function accepts. `min` and `max` come as a pair,
/// so `section()` takes 2 or 4 parameters, never 3.
fn check_arity(name: &str, count: usize) -> Result<(), EvaluationError> {
    let allowed: &[usize] = match name {
        "section" | "entry" => &[2, 4],
        "entryIn" => &[1, 3],
        "resources" | "hasCoding" => &[2, 3],
        "resourcesIn" => &[1, 2],
        "sameSubject" => &[0, 1],
        _ => &[],
    };
    if allowed.contains(&count) {
        return Ok(());
    }
    let expected = allowed
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" or ");
    Err(EvaluationError::InvalidArity(format!(
        "Function '{name}' expects {expected} arguments, got {count}"
    )))
}

// --- parameters --------------------------------------------------------------

/// The number of items an input item may yield: `min..=max`, `max` `None` for `*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    min: usize,
    max: Option<usize>,
}

impl Bounds {
    const ONE: Bounds = Bounds {
        min: 1,
        max: Some(1),
    };
    const ANY: Bounds = Bounds { min: 0, max: None };

    fn within(self, count: usize) -> bool {
        count >= self.min && self.max.is_none_or(|max| count <= max)
    }

    /// Reads `min` and `max`. One that is not given, or is empty, takes the
    /// default's value.
    fn parse(
        function: &str,
        min: Option<&EvaluationResult>,
        max: Option<&EvaluationResult>,
        defaults: Bounds,
    ) -> Result<Bounds, EvaluationError> {
        let min_text = min
            .and_then(string_param)
            .unwrap_or_else(|| defaults.min.to_string());
        let max_text = max.and_then(string_param).unwrap_or_else(|| {
            defaults
                .max
                .map_or_else(|| "*".to_string(), |m| m.to_string())
        });
        let min = non_negative(&min_text);
        let max = if max_text == "*" {
            Some(None)
        } else {
            non_negative(&max_text).map(Some)
        };
        match (min, max) {
            (Some(min), Some(max)) if max.is_none_or(|max| max >= min) => Ok(Bounds { min, max }),
            _ => Err(EvaluationError::InvalidArgument(format!(
                "{function}(): min and max must be non-negative integers with min <= max \
                 (max may be '*'), got min '{min_text}' and max '{max_text}'"
            ))),
        }
    }
}

/// A non-negative integer of at most 9 digits, as the Java engine accepts.
fn non_negative(s: &str) -> Option<usize> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A parameter's value as a string: `None` when it is empty (and so ignored).
/// A number is its decimal text, so `min` and `max` can be integers or strings.
fn string_param(param: &EvaluationResult) -> Option<String> {
    let item = match param {
        EvaluationResult::Collection { items, .. } if items.len() == 1 => &items[0],
        other => other,
    };
    match item {
        EvaluationResult::String(s, _, _)
        | EvaluationResult::Date(s, _, _)
        | EvaluationResult::DateTime(s, _, _)
        | EvaluationResult::Time(s, _, _) => Some(s.clone()),
        EvaluationResult::Integer(i, _, _) | EvaluationResult::Integer64(i, _, _) => {
            Some(i.to_string())
        }
        EvaluationResult::Decimal(d, _, _) => Some(d.to_string()),
        EvaluationResult::Boolean(b, _, _) => Some(b.to_string()),
        _ => None,
    }
}

/// The value set URL; `None` when the parameter is empty (and the function's
/// result is then empty). Anything other than a canonical URL is an error.
fn value_set_param(
    function: &str,
    param: &EvaluationResult,
) -> Result<Option<String>, EvaluationError> {
    if param.count() == 0 {
        return Ok(None);
    }
    match string_param(param) {
        Some(url) if is_canonical(&url) => Ok(Some(url)),
        other => Err(EvaluationError::InvalidArgument(format!(
            "{function}(): '{}' is not a value set canonical URL",
            other.unwrap_or_default()
        ))),
    }
}

/// The resource type to keep; `None` (no constraint) when the parameter is not
/// given or is empty. A value that is not a type name is an error.
fn type_param(
    function: &str,
    param: Option<&EvaluationResult>,
) -> Result<Option<String>, EvaluationError> {
    let Some(param) = param.filter(|p| p.count() > 0) else {
        return Ok(None);
    };
    match string_param(param) {
        Some(t) if is_type_name(&t) => Ok(Some(t)),
        other => Err(EvaluationError::InvalidArgument(format!(
            "{function}(): '{}' is not a resource type name",
            other.unwrap_or_default()
        ))),
    }
}

/// A canonical URL: `scheme:rest`, with `|version` or not, and no whitespace.
fn is_canonical(url: &str) -> bool {
    if url.len() > 320 || url.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(colon) = url.find(':') else {
        return false;
    };
    if colon < 1 || !url.starts_with(|c: char| c.is_alphabetic()) {
        return false;
    }
    if !url[..colon]
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '+' | '.' | '-'))
    {
        return false;
    }
    let end = match url.find('|') {
        Some(bar) => {
            if bar == url.len() - 1 || url[bar + 1..].contains('|') {
                return false;
            }
            bar
        }
        None => url.len(),
    };
    // There must be something between the scheme and the end (or the version).
    colon != end - 1
}

/// A resource type name: an uppercase letter, then letters only, 2 to 64 long.
fn is_type_name(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_uppercase())
        && s.bytes().all(|b| b.is_ascii_alphabetic())
}

// --- the document -------------------------------------------------------------

/// The resource the evaluation is on (`%rootResource`, which in this engine is
/// also `%resource`), converted once per function call.
struct Document {
    root: Option<EvaluationResult>,
}

impl Document {
    fn new(context: &EvaluationContext) -> Self {
        Document {
            root: context
                .resources
                .first()
                .map(IntoEvaluationResult::to_evaluation_result),
        }
    }

    /// The document the evaluation is on: the root resource when it is a Bundle.
    fn bundle(&self) -> Option<&EvaluationResult> {
        self.root
            .as_ref()
            .filter(|r| fhir_type(r).as_deref() == Some("Bundle"))
    }

    /// The root resource when it is a Composition evaluated on its own.
    fn composition(&self) -> Option<&EvaluationResult> {
        self.root
            .as_ref()
            .filter(|r| fhir_type(r).as_deref() == Some("Composition"))
    }
}

/// The Composition for an input item: the item itself, or a document's first
/// Composition entry.
fn composition_of(item: &EvaluationResult) -> Option<&EvaluationResult> {
    match fhir_type(item).as_deref() {
        Some("Composition") => Some(item),
        Some("Bundle") => bundle_resources(item).find(|r| is_type(r, "Composition")),
        _ => None,
    }
}

/// A Bundle's entries.
fn bundle_entries(bundle: &EvaluationResult) -> impl Iterator<Item = &EvaluationResult> {
    children(bundle, "entry")
}

/// A Bundle's entry resources, in order.
fn bundle_resources(bundle: &EvaluationResult) -> impl Iterator<Item = &EvaluationResult> {
    bundle_entries(bundle).filter_map(|e| first_child(e, "resource"))
}

/// All a Composition's sections, nested ones after their parent.
fn all_sections<'a>(parent: &'a EvaluationResult, out: &mut Vec<&'a EvaluationResult>) {
    for s in children(parent, "section") {
        out.push(s);
        all_sections(s, out);
    }
}

fn section(
    base: &EvaluationResult,
    system: Option<&str>,
    code: Option<&str>,
    bounds: Bounds,
) -> Vec<EvaluationResult> {
    let mut result = Vec::new();
    for item in items(base) {
        let Some(composition) = composition_of(item) else {
            continue;
        };
        let mut sections = Vec::new();
        all_sections(composition, &mut sections);
        let found: Vec<_> = sections
            .into_iter()
            .filter(|s| concept_has_coding(first_child(s, "code"), system, code))
            .collect();
        if bounds.within(found.len()) {
            result.extend(found.into_iter().cloned());
        }
    }
    result
}

fn entry(
    base: &EvaluationResult,
    doc: &Document,
    bounds: Bounds,
    matches: &mut dyn FnMut(&EvaluationResult) -> Result<bool, EvaluationError>,
) -> Result<Vec<EvaluationResult>, EvaluationError> {
    let mut resolver = Resolver::new(doc);
    let mut result = Vec::new();
    for item in items(base) {
        let Some(entries) = resolver.section_entries(item) else {
            continue;
        };
        let mut found = Vec::new();
        for r in entries {
            if matches(&r)? {
                found.push(r);
            }
        }
        if bounds.within(found.len()) {
            result.extend(found);
        }
    }
    Ok(result)
}

/// The resources of the documents the input items are, or are in, each
/// document once, keeping only those of `resource_type` when it is given.
fn document_resources(
    base: &EvaluationResult,
    doc: &Document,
    resource_type: Option<&str>,
) -> Vec<EvaluationResult> {
    let mut bundles: Vec<&EvaluationResult> = Vec::new();
    for item in items(base) {
        let bundle = if is_type(item, "Bundle") {
            Some(item)
        } else {
            doc.bundle()
        };
        if let Some(bundle) = bundle {
            if !bundles.contains(&bundle) {
                bundles.push(bundle);
            }
        }
    }
    bundles
        .into_iter()
        .flat_map(bundle_resources)
        .filter(|r| resource_type.is_none_or(|t| is_type(r, t)))
        .cloned()
        .collect()
}

// --- reference resolution ------------------------------------------------------

/// Resolves section entries in the document, building the indexes it needs
/// once per function call rather than once per section.
struct Resolver<'d> {
    doc: &'d Document,
    /// Each Composition entry of the document, with its fullUrl and sections.
    compositions: Option<Vec<CompositionEntry<'d>>>,
    /// The document's entries by fullUrl (and fullUrl/_history/versionId).
    by_full_url: Option<HashMap<String, Vec<&'d EvaluationResult>>>,
}

struct CompositionEntry<'d> {
    resource: &'d EvaluationResult,
    full_url: Option<String>,
    sections: Vec<&'d EvaluationResult>,
}

impl<'d> Resolver<'d> {
    fn new(doc: &'d Document) -> Self {
        Resolver {
            doc,
            compositions: None,
            by_full_url: None,
        }
    }

    /// A section's entries, resolved; `None` when the item cannot be a section
    /// (a resource or a primitive). References that do not resolve are left out.
    fn section_entries(&mut self, section: &EvaluationResult) -> Option<Vec<EvaluationResult>> {
        let EvaluationResult::Object { map, .. } = section else {
            return None;
        };
        if map.contains_key("resourceType") {
            return None;
        }

        let (composition, full_url) = self.owner(section);
        let mut result = Vec::new();
        for reference in children(section, "entry") {
            let Some(reference) = primitive(reference, "reference").filter(|r| !r.is_empty())
            else {
                continue;
            };
            let resolved = if let Some(id) = reference.strip_prefix('#') {
                composition.and_then(|c| {
                    children(c, "contained").find(|r| primitive(r, "id").as_deref() == Some(id))
                })
            } else {
                self.in_bundle(&reference, composition, full_url.as_deref())
            };
            if let Some(r) = resolved {
                result.push(r.clone());
            }
        }
        Some(result)
    }

    /// The Composition a section belongs to, and that Composition's fullUrl.
    fn owner(
        &mut self,
        section: &EvaluationResult,
    ) -> (Option<&'d EvaluationResult>, Option<String>) {
        let doc = self.doc;
        let compositions = self.compositions.get_or_insert_with(|| {
            doc.bundle()
                .into_iter()
                .flat_map(bundle_entries)
                .filter_map(|e| {
                    let resource =
                        first_child(e, "resource").filter(|r| is_type(r, "Composition"))?;
                    let mut sections = Vec::new();
                    all_sections(resource, &mut sections);
                    Some(CompositionEntry {
                        resource,
                        full_url: primitive(e, "fullUrl"),
                        sections,
                    })
                })
                .collect()
        });
        if let Some(c) = compositions.iter().find(|c| c.sections.contains(&section)) {
            return (Some(c.resource), c.full_url.clone());
        }
        // Without a document: a Composition evaluated on its own.
        if let Some(c) = doc.composition() {
            let mut sections = Vec::new();
            all_sections(c, &mut sections);
            if sections.contains(&section) {
                return (Some(c), None);
            }
        }
        (None, None)
    }

    /// A reference resolved in the document, as the validator resolves one in
    /// a Bundle (`BaseValidator.resolveInBundle`), less its error reporting.
    fn in_bundle(
        &mut self,
        reference: &str,
        composition: Option<&EvaluationResult>,
        composition_full_url: Option<&str>,
    ) -> Option<&'d EvaluationResult> {
        let bundle = self.doc.bundle()?;
        if self.by_full_url.is_none() {
            let mut by_full_url: HashMap<String, Vec<&'d EvaluationResult>> = HashMap::new();
            for e in bundle_entries(bundle) {
                let full_url = primitive(e, "fullUrl");
                if let Some(fu) = &full_url {
                    by_full_url.entry(fu.clone()).or_default().push(e);
                }
                let version_id = first_child(e, "resource")
                    .and_then(|r| first_child(r, "meta"))
                    .and_then(|m| primitive(m, "versionId"));
                if let (Some(fu), Some(v)) = (&full_url, version_id) {
                    by_full_url
                        .entry(format!("{fu}/_history/{v}"))
                        .or_default()
                        .push(e);
                }
            }
            self.by_full_url = Some(by_full_url);
        }
        let by_full_url = self.by_full_url.as_ref()?;

        // A fragment on the end is set aside: the resource is what is resolved.
        let reference = reference.split('#').next().unwrap_or(reference);
        if is_absolute_url(reference) {
            return single_resource(by_full_url.get(reference));
        }
        let base = composition.and_then(|c| {
            restful_base(composition_full_url?, &fhir_type(c)?, &primitive(c, "id")?)
        });
        // From R4 on (every version this crate supports), a relative reference
        // does not resolve by type and id alone: it needs a RESTful base.
        single_resource(by_full_url.get(&format!("{}{reference}", base?)))
    }
}

/// The resource of the one entry found; `None` if there is none, or more than one.
fn single_resource<'d>(
    entries: Option<&Vec<&'d EvaluationResult>>,
) -> Option<&'d EvaluationResult> {
    match entries {
        Some(entries) if entries.len() == 1 => first_child(entries[0], "resource"),
        _ => None,
    }
}

/// Whether a reference is an absolute URL, as `Utilities.isAbsoluteUrl` in the
/// Java engine judges one: a lowercase scheme, and something after it with no
/// spaces.
fn is_absolute_url(reference: &str) -> bool {
    let Some((scheme, rest)) = reference.split_once(':') else {
        return false;
    };
    let token = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    token && !rest.is_empty() && !rest.contains(' ')
}

/// The base a relative reference is resolved against: the referring resource's
/// fullUrl less its own `Type/id`, when the fullUrl is a RESTful one
/// (`[http(s)://...]Type/id`). `None` if it is not.
fn restful_base(full_url: &str, resource_type: &str, id: &str) -> Option<String> {
    if !is_restful_id(id) {
        return None;
    }
    let base = full_url.strip_suffix(&format!("{resource_type}/{id}"))?;
    if base.is_empty() {
        return Some(String::new());
    }
    let rest = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))?;
    if !rest.is_empty() && !rest.ends_with('/') {
        return None;
    }
    rest.chars()
        .all(|c| c.is_ascii_alphanumeric() || "\\.:%$-_/".contains(c))
        .then(|| base.to_string())
}

fn is_restful_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

// --- codes ---------------------------------------------------------------------

/// A resource's codes: its `code`, `vaccineCode` and `medication`, when each is
/// a CodeableConcept or a CodeableReference's `concept`.
fn codes(resource: &EvaluationResult, version: FhirVersion) -> Vec<EvaluationResult> {
    let EvaluationResult::Object { map, .. } = resource else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for name in CODE_ELEMENTS {
        // `medication` is a choice element before R5 (medicationCodeableConcept).
        let Some(value) = map
            .get(name)
            .cloned()
            .or_else(|| crate::polymorphic_access::access_polymorphic_element(map, name, version))
        else {
            continue;
        };
        for v in items(&value) {
            if is_codeable_concept(v) {
                result.push(v.clone());
            } else if is_codeable_reference(v) {
                if let Some(concept) = first_child(v, "concept") {
                    result.push(concept.clone());
                }
            }
        }
    }
    result
}

fn has_code(
    resource: &EvaluationResult,
    system: Option<&str>,
    code: Option<&str>,
    version: FhirVersion,
) -> bool {
    codes(resource, version)
        .iter()
        .any(|cc| concept_has_coding(Some(cc), system, code))
}

/// Whether the concept has a coding that matches `system` and `code`, each
/// `None` to match anything.
fn concept_has_coding(
    concept: Option<&EvaluationResult>,
    system: Option<&str>,
    code: Option<&str>,
) -> bool {
    concept.is_some_and(|c| children(c, "coding").any(|c| coding_matches(c, system, code, None)))
}

fn coding_matches(
    coding: &EvaluationResult,
    system: Option<&str>,
    code: Option<&str>,
    version: Option<&str>,
) -> bool {
    let field_is = |name: &str, want: Option<&str>| {
        want.is_none_or(|w| primitive(coding, name).as_deref() == Some(w))
    };
    field_is("system", system) && field_is("code", code) && field_is("version", version)
}

/// `hasCoding(system, code [, version])`: whether any Coding in the input (the
/// Codings, and the codings of CodeableConcepts; anything else is passed over)
/// matches. A parameter that is a string must equal the coding's system, code
/// or version exactly, so a coding without a version does not match a version
/// that is given. A parameter that is empty, or the version when it is not
/// given, is ignored. Empty when the input is empty; otherwise true or false.
pub fn has_coding(
    base: &EvaluationResult,
    system: Option<&str>,
    code: Option<&str>,
    version: Option<&str>,
) -> Option<bool> {
    if base.count() == 0 {
        return None;
    }
    Some(items(base).into_iter().any(|item| {
        if is_coding(item) {
            coding_matches(item, system, code, version)
        } else if is_codeable_concept(item) {
            children(item, "coding").any(|c| coding_matches(c, system, code, version))
        } else {
            false
        }
    }))
}

/// Value-set membership of resources' codes, through the terminology server.
struct Membership<'c> {
    function: &'static str,
    url: String,
    context: &'c EvaluationContext,
    terminology: Option<TerminologyFunctions>,
}

impl<'c> Membership<'c> {
    fn new(function: &str, url: String, context: &'c EvaluationContext) -> Self {
        Membership {
            function: if function == "entryIn" {
                "entryIn"
            } else {
                "resourcesIn"
            },
            url,
            context,
            terminology: None,
        }
    }

    /// Whether any coding of the resource's code is in the value set. A value
    /// set the server cannot find, or a membership it cannot decide, is an error.
    fn contains_code_of(&mut self, resource: &EvaluationResult) -> Result<bool, EvaluationError> {
        for concept in codes(resource, self.context.fhir_version) {
            for coding in children(&concept, "coding") {
                if primitive(coding, "code").is_none_or(|c| c.is_empty()) {
                    continue;
                }
                if self.validate(coding)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn validate(&mut self, coding: &EvaluationResult) -> Result<bool, EvaluationError> {
        if self.terminology.is_none() {
            self.terminology = Some(TerminologyFunctions::new(self.context)?);
        }
        let terminology = self.terminology.as_ref().expect("set above");
        let response = terminology
            .validate_vs(&EvaluationResult::string(self.url.clone()), coding, None)
            .map_err(|e| {
                EvaluationError::InvalidOperation(format!(
                    "{}(): cannot check membership of value set '{}': {e}",
                    self.function, self.url
                ))
            })?;
        validation_result(&response).ok_or_else(|| {
            EvaluationError::InvalidOperation(format!(
                "{}(): the terminology server did not say whether the code is in value set '{}'",
                self.function, self.url
            ))
        })
    }
}

/// The `result` of a `$validate-code` Parameters response.
fn validation_result(response: &EvaluationResult) -> Option<bool> {
    children(response, "parameter").find_map(|p| {
        if primitive(p, "name").as_deref() != Some("result") {
            return None;
        }
        match first_child(p, "valueBoolean") {
            Some(EvaluationResult::Boolean(b, _, _)) => Some(*b),
            _ => None,
        }
    })
}

// --- sameSubject -----------------------------------------------------------------

/// `sameSubject([other])`: whether the focus resource is about the same subject
/// as `other`, or, without `other`, as the Composition of the document the
/// evaluation is on.
///
/// A resource's subject is its `subject` or its `patient`. `None` (empty) when
/// the focus or `other` is not a single resource, when either has no subject
/// (or more than one), or, without `other`, when the evaluation is not on a
/// Bundle of type `document`. `other` given as empty is not the same as no
/// `other`: the result is empty. Otherwise true when both subjects have a
/// reference and the strings are identical; else, when both have an
/// identifier, whether the identifiers' system and value are identical;
/// otherwise false. References are compared as strings, never resolved, and a
/// reference is never compared with an identifier.
pub fn same_subject(
    base: &EvaluationResult,
    other: Option<&EvaluationResult>,
    context: &EvaluationContext,
) -> Option<bool> {
    let focus = single_item(base)?;
    let doc;
    let other = match other {
        Some(other) => single_item(other)?,
        None => {
            doc = Document::new(context);
            let bundle = doc
                .bundle()
                .filter(|b| primitive(b, "type").as_deref() == Some("document"))?;
            composition_of(bundle)?
        }
    };
    let a = subject_of(focus)?;
    let b = subject_of(other)?;

    let ref_a = primitive(a, "reference").filter(|r| !r.is_empty());
    let ref_b = primitive(b, "reference").filter(|r| !r.is_empty());
    if let (Some(ref_a), Some(ref_b)) = (&ref_a, &ref_b) {
        return Some(ref_a == ref_b);
    }
    if let (Some(id_a), Some(id_b)) = (first_child(a, "identifier"), first_child(b, "identifier")) {
        let value_a = primitive(id_a, "value").filter(|v| !v.is_empty());
        return Some(
            value_a.is_some()
                && value_a == primitive(id_b, "value")
                && primitive(id_a, "system") == primitive(id_b, "system"),
        );
    }
    Some(false)
}

/// A resource's subject or patient; `None` if it has neither, or more than one
/// (Composition.subject repeats from R5 on).
fn subject_of(resource: &EvaluationResult) -> Option<&EvaluationResult> {
    let EvaluationResult::Object { .. } = resource else {
        return None;
    };
    let mut list: Vec<_> = children(resource, "subject").collect();
    if list.is_empty() {
        list = children(resource, "patient").collect();
    }
    if list.len() == 1 { Some(list[0]) } else { None }
}

// --- access ------------------------------------------------------------------------

/// The items of a collection; a single value is a collection of one.
fn items(value: &EvaluationResult) -> Vec<&EvaluationResult> {
    match value {
        EvaluationResult::Empty => Vec::new(),
        EvaluationResult::Collection { items, .. } => items.iter().collect(),
        other => vec![other],
    }
}

fn single_item(value: &EvaluationResult) -> Option<&EvaluationResult> {
    match items(value).as_slice() {
        [one] => Some(one),
        _ => None,
    }
}

/// The values of an object's element (none when it is absent).
fn children<'a>(
    value: &'a EvaluationResult,
    name: &str,
) -> Box<dyn Iterator<Item = &'a EvaluationResult> + 'a> {
    match value {
        EvaluationResult::Object { map, .. } => match map.get(name) {
            Some(EvaluationResult::Collection { items, .. }) => Box::new(items.iter()),
            Some(EvaluationResult::Empty) | None => Box::new(std::iter::empty()),
            Some(one) => Box::new(std::iter::once(one)),
        },
        _ => Box::new(std::iter::empty()),
    }
}

fn first_child<'a>(value: &'a EvaluationResult, name: &str) -> Option<&'a EvaluationResult> {
    children(value, name).next()
}

/// An element's primitive value as a string.
fn primitive(value: &EvaluationResult, name: &str) -> Option<String> {
    first_child(value, name).and_then(string_param)
}

/// The FHIR type of an object: its type information, or its `resourceType`.
fn fhir_type(value: &EvaluationResult) -> Option<String> {
    let EvaluationResult::Object { map, type_info } = value else {
        return None;
    };
    if let Some(EvaluationResult::String(t, _, _)) = map.get("resourceType") {
        return Some(t.clone());
    }
    type_info.as_ref().map(|t| t.name.clone())
}

fn is_type(value: &EvaluationResult, type_name: &str) -> bool {
    fhir_type(value).as_deref() == Some(type_name)
}

fn is_coding(value: &EvaluationResult) -> bool {
    match fhir_type(value) {
        Some(t) => t == "Coding",
        // Untyped data: a Coding has a system or code, and no codings.
        None => matches!(value, EvaluationResult::Object { map, .. }
            if !map.contains_key("coding") && (map.contains_key("code") || map.contains_key("system"))),
    }
}

fn is_codeable_concept(value: &EvaluationResult) -> bool {
    match fhir_type(value) {
        Some(t) => t == "CodeableConcept",
        None => matches!(value, EvaluationResult::Object { map, .. }
            if map.contains_key("coding") || (map.contains_key("text") && map.len() == 1)),
    }
}

fn is_codeable_reference(value: &EvaluationResult) -> bool {
    match fhir_type(value) {
        Some(t) => t == "CodeableReference",
        None => {
            matches!(value, EvaluationResult::Object { map, .. } if map.contains_key("concept"))
        }
    }
}

fn collection(items: Vec<EvaluationResult>) -> EvaluationResult {
    match items.len() {
        0 => EvaluationResult::Empty,
        1 => items.into_iter().next().expect("one item"),
        _ => EvaluationResult::Collection {
            items,
            has_undefined_order: false,
            type_info: None,
        },
    }
}

fn boolean_or_empty(value: Option<bool>) -> EvaluationResult {
    value.map_or(EvaluationResult::Empty, EvaluationResult::boolean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_defaults_and_errors() {
        let int = |i: i64| EvaluationResult::integer(i);
        let s = |v: &str| EvaluationResult::string(v.to_string());
        assert_eq!(
            Bounds::parse("f", None, None, Bounds::ONE).unwrap(),
            Bounds::ONE
        );
        assert_eq!(
            Bounds::parse(
                "f",
                Some(&EvaluationResult::Empty),
                Some(&EvaluationResult::Empty),
                Bounds::ANY
            )
            .unwrap(),
            Bounds::ANY
        );
        assert_eq!(
            Bounds::parse("f", Some(&int(2)), Some(&s("*")), Bounds::ONE).unwrap(),
            Bounds { min: 2, max: None }
        );
        assert_eq!(
            Bounds::parse("f", Some(&s("0")), Some(&int(0)), Bounds::ONE).unwrap(),
            Bounds {
                min: 0,
                max: Some(0)
            }
        );
        assert!(Bounds::parse("f", Some(&int(2)), Some(&int(1)), Bounds::ONE).is_err());
        assert!(Bounds::parse("f", Some(&int(1)), Some(&s("many")), Bounds::ONE).is_err());
        assert!(Bounds::parse("f", Some(&int(-1)), Some(&s("*")), Bounds::ONE).is_err());
        // An empty max takes the default's max, which must still be >= min.
        assert!(
            Bounds::parse(
                "f",
                Some(&int(2)),
                Some(&EvaluationResult::Empty),
                Bounds::ONE
            )
            .is_err()
        );
    }

    #[test]
    fn canonical_urls() {
        for ok in [
            "http://hl7.org/fhir/ValueSet/data-absent-reason",
            "http://hl7.org/fhir/ValueSet/x|4.0.1",
            "urn:oid:1.2.3",
        ] {
            assert!(is_canonical(ok), "{ok}");
        }
        for bad in [
            "not a value set",
            "http:",
            ":x",
            "1http://x",
            "http://x|",
            "http://x|1|2",
            "",
        ] {
            assert!(!is_canonical(bad), "{bad}");
        }
    }

    #[test]
    fn type_names() {
        assert!(is_type_name("Condition"));
        assert!(!is_type_name("condition"));
        assert!(!is_type_name("C"));
        assert!(!is_type_name("Condition1"));
    }

    #[test]
    fn restful_bases() {
        assert_eq!(
            restful_base(
                "http://example.org/fhir/Composition/c1",
                "Composition",
                "c1"
            )
            .as_deref(),
            Some("http://example.org/fhir/")
        );
        assert_eq!(
            restful_base("Composition/c1", "Composition", "c1").as_deref(),
            Some("")
        );
        assert_eq!(restful_base("urn:uuid:1234", "Composition", "c1"), None);
        assert_eq!(
            restful_base("ftp://example.org/Composition/c1", "Composition", "c1"),
            None
        );
        assert_eq!(
            restful_base("http://example.orgComposition/c1", "Composition", "c1"),
            None
        );
    }

    #[test]
    fn absolute_urls() {
        assert!(is_absolute_url("urn:uuid:5b6a5a3e"));
        assert!(is_absolute_url("http://example.org/fhir/Condition/c2"));
        assert!(!is_absolute_url("Condition/c2"));
        assert!(!is_absolute_url("HTTP://x"));
        assert!(!is_absolute_url("urn:"));
    }
}
