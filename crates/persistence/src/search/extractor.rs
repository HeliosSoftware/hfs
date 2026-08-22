//! SearchParameter Value Extractor.
//!
//! Uses FHIRPath expressions to extract searchable values from FHIR resources.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use helios_fhir::search::ABSTRACT_BASE_TYPES;
use helios_fhirpath::EvaluationContext;
use helios_fhirpath_support::EvaluationResult;
use parking_lot::RwLock;
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::SearchParamType;

use super::converters::{IndexValue, ValueConverter};
use super::errors::ExtractionError;
use super::registry::{SearchParameterDefinition, SearchParameterRegistry};

/// A value extracted from a resource for indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedValue {
    /// The parameter name (e.g., "name", "identifier").
    pub param_name: String,

    /// The parameter URL.
    pub param_url: String,

    /// The parameter type.
    pub param_type: SearchParamType,

    /// The extracted and converted value.
    pub value: IndexValue,

    /// Composite group ID (for composite parameters).
    /// Values with the same group ID are part of the same composite match.
    pub composite_group: Option<u32>,
}

impl ExtractedValue {
    /// Creates a new extracted value.
    pub fn new(
        param_name: impl Into<String>,
        param_url: impl Into<String>,
        param_type: SearchParamType,
        value: IndexValue,
    ) -> Self {
        Self {
            param_name: param_name.into(),
            param_url: param_url.into(),
            param_type,
            value,
            composite_group: None,
        }
    }

    /// Sets the composite group ID.
    pub fn with_composite_group(mut self, group: u32) -> Self {
        self.composite_group = Some(group);
        self
    }
}

/// Search values extracted from one `contained[]` entry of a container resource.
#[derive(Debug, Clone)]
pub struct ContainedExtraction {
    /// The contained resource's `resourceType`.
    pub contained_type: String,
    /// The contained resource's local `id` (used for `Container/cid#localid`).
    pub local_id: String,
    /// The contained resource's JSON (the `contained[]` entry itself). Backends
    /// that store content inline (Elasticsearch) index this directly.
    pub content: Value,
    /// The search values extracted from the contained resource.
    pub values: Vec<ExtractedValue>,
}

/// Extracts searchable values from FHIR resources using FHIRPath.
pub struct SearchParameterExtractor {
    registry: Arc<RwLock<SearchParameterRegistry>>,
}

impl SearchParameterExtractor {
    /// Creates a new extractor with the given registry.
    pub fn new(registry: Arc<RwLock<SearchParameterRegistry>>) -> Self {
        Self { registry }
    }

    /// Extracts all searchable values from a resource.
    ///
    /// Returns values for all active search parameters that apply to this resource type.
    pub fn extract(
        &self,
        resource: &Value,
        resource_type: &str,
    ) -> Result<Vec<ExtractedValue>, ExtractionError> {
        // Validate resource
        let obj = resource
            .as_object()
            .ok_or_else(|| ExtractionError::InvalidResource {
                message: "Resource must be a JSON object".to_string(),
            })?;

        // Verify resource type
        if let Some(rt) = obj.get("resourceType").and_then(|v| v.as_str()) {
            if rt != resource_type {
                return Err(ExtractionError::InvalidResource {
                    message: format!(
                        "Resource type mismatch: expected {}, got {}",
                        resource_type, rt
                    ),
                });
            }
        }

        let mut results = Vec::new();

        // Get active parameters for this resource type
        let params = {
            let registry = self.registry.read();
            registry.get_active_params(resource_type)
        };

        for param in &params {
            match self.extract_for_param(resource, param) {
                Ok(values) => results.extend(values),
                Err(e) => {
                    // Log the error but continue with other parameters
                    tracing::warn!(
                        "Failed to extract values for parameter '{}': {}",
                        param.code,
                        e
                    );
                }
            }
        }

        // Also extract the parameters registered against an abstract base. The
        // registry buckets parameters by each declared `base`, so a definition
        // with `base: ["DomainResource"]` (`_text`) lands in its own bucket and
        // is invisible to a lookup of "Resource" alone — both bases have to be
        // consulted, or half of `ABSTRACT_BASE_TYPES` is dead weight.
        let mut seen: HashSet<String> = params.iter().map(|p| p.code.clone()).collect();
        for base in ABSTRACT_BASE_TYPES {
            let common_params = {
                let registry = self.registry.read();
                registry.get_active_params(base)
            };

            for param in &common_params {
                if !seen.insert(param.code.clone()) {
                    continue;
                }
                match self.extract_for_param(resource, param) {
                    Ok(values) => results.extend(values),
                    Err(e) => {
                        tracing::warn!(
                            "Failed to extract values for common parameter '{}': {}",
                            param.code,
                            e
                        );
                    }
                }
            }
        }

        Ok(results)
    }

    /// Extracts searchable values from a container resource's `contained[]`
    /// entries, for `_contained` search.
    ///
    /// Each contained resource is treated as a standalone resource of its own
    /// `resourceType` and run through the normal [`Self::extract`] path. Contained
    /// resources without a `resourceType` or `id` are skipped — an `id` is
    /// required so the match can be addressed (`Container/cid#localid`) and the
    /// container can return the specific contained resource.
    pub fn extract_contained(&self, container: &Value) -> Vec<ContainedExtraction> {
        let Some(entries) = container.get("contained").and_then(|c| c.as_array()) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for entry in entries {
            let (Some(contained_type), Some(local_id)) = (
                entry.get("resourceType").and_then(|v| v.as_str()),
                entry.get("id").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            match self.extract(entry, contained_type) {
                Ok(values) if !values.is_empty() => out.push(ContainedExtraction {
                    contained_type: contained_type.to_string(),
                    local_id: local_id.to_string(),
                    content: entry.clone(),
                    values,
                }),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "Failed to extract contained {}/{}: {}",
                    contained_type,
                    local_id,
                    e
                ),
            }
        }
        out
    }

    /// Extracts values for a specific parameter from a resource.
    pub fn extract_for_param(
        &self,
        resource: &Value,
        param: &SearchParameterDefinition,
    ) -> Result<Vec<ExtractedValue>, ExtractionError> {
        if NON_INDEXABLE_PARAM_CODES.contains(&param.code.as_str()) {
            return Ok(Vec::new());
        }

        // Composite parameters are indexed component-by-component, with all the
        // components of one composite instance sharing a `composite_group`.
        if matches!(param.param_type, SearchParamType::Composite) {
            return self.extract_composite(resource, param);
        }

        if param.expression.is_empty() {
            return Ok(Vec::new());
        }

        // Get the resource type from the resource
        let resource_type = resource
            .get("resourceType")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Rewrite choice-type casts (`value as Quantity` → `valueQuantity`) so they
        // resolve against schema-less JSON, then filter to this resource type.
        let rewritten = rewrite_choice_types(&param.expression);
        let filtered_expr = self.filter_expression_for_resource(&rewritten, resource_type);

        if filtered_expr.is_empty() {
            return Ok(Vec::new());
        }

        // Evaluate the filtered FHIRPath expression using the actual evaluator
        let values = self.evaluate_fhirpath(resource, &filtered_expr)?;

        let mut results = Vec::new();
        for value in values {
            let converted = ValueConverter::convert(&value, param.param_type, &param.code)?;
            for idx_value in converted {
                results.push(ExtractedValue::new(
                    &param.code,
                    &param.url,
                    param.param_type,
                    idx_value,
                ));
            }
        }

        Ok(results)
    }

    /// Extracts index rows for a composite search parameter.
    ///
    /// The composite's `expression` (e.g. `Observation` or
    /// `Observation.component`) selects the base instances. Each instance gets a
    /// `composite_group` id, and every component sub-expression is evaluated
    /// relative to that instance and stored as its own row under the composite
    /// parameter's code. Component value types are resolved from the registry by
    /// the component `definition` URL.
    fn extract_composite(
        &self,
        resource: &Value,
        param: &SearchParameterDefinition,
    ) -> Result<Vec<ExtractedValue>, ExtractionError> {
        let components = match &param.component {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(Vec::new()),
        };

        let resource_type = resource
            .get("resourceType")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let rewritten_base = rewrite_choice_types(&param.expression);
        let base_expr = self.filter_expression_for_resource(&rewritten_base, resource_type);
        if base_expr.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve each component's value type from the registry (by definition URL).
        let component_types: Vec<Option<SearchParamType>> = {
            let registry = self.registry.read();
            components
                .iter()
                .map(|c| registry.get_by_url(&c.definition).map(|d| d.param_type))
                .collect()
        };

        // Each base instance becomes a composite group.
        let base_nodes = self.evaluate_fhirpath(resource, &base_expr)?;

        let mut results = Vec::new();
        for (group_idx, node) in base_nodes.iter().enumerate() {
            let group = group_idx as u32;
            for (component, sub_type) in components.iter().zip(component_types.iter()) {
                let sub_type = match sub_type {
                    Some(t) => *t,
                    None => continue, // unknown component definition — skip
                };
                if component.expression.is_empty() {
                    continue;
                }
                let comp_expr = rewrite_choice_types(&component.expression);
                let values = self.evaluate_fhirpath(node, &comp_expr)?;
                for value in values {
                    let converted = ValueConverter::convert(&value, sub_type, &param.code)?;
                    for idx_value in converted {
                        results.push(
                            ExtractedValue::new(&param.code, &param.url, sub_type, idx_value)
                                .with_composite_group(group),
                        );
                    }
                }
            }
        }

        Ok(results)
    }

    /// Filters a FHIRPath expression to only include parts relevant to a specific resource type.
    ///
    /// Many FHIR SearchParameters have expressions that span multiple resource types, joined
    /// with `|` (union). For example, the `patient` parameter has:
    /// `AllergyIntolerance.patient | CarePlan.subject.where(resolve() is Patient) | ...`
    ///
    /// This method extracts only the parts that start with the given resource type and
    /// simplifies common patterns that use `resolve()`.
    ///
    /// Parts prefixed with an abstract base type (`Resource.`, `DomainResource.`)
    /// apply to every resource, so the prefix is stripped instead of being matched
    /// literally — see [`strip_abstract_base_prefix`].
    fn filter_expression_for_resource(&self, expression: &str, resource_type: &str) -> String {
        // Split into union members at top level only, then keep the ones that
        // apply to this resource type.
        let parts: Vec<String> = split_union_members(expression)
            .into_iter()
            .filter_map(|p| {
                // Abstract-base parts first: when `resource_type` is itself
                // "Resource", the literal prefix match below would keep the
                // unevaluable form.
                if let Some(rest) = strip_abstract_base_prefix(p) {
                    return Some(rest.into_owned());
                }
                // Check if this part starts with our resource type
                let matches_type = p.starts_with(resource_type)
                    && (p.len() == resource_type.len()
                        || p.chars().nth(resource_type.len()) == Some('.'));
                matches_type.then(|| p.to_string())
            })
            .map(|p| self.simplify_resolve_pattern(&p))
            .collect();

        if parts.is_empty() {
            // If no parts match, return the original expression
            // This handles expressions that don't use ResourceType prefix
            expression.to_string()
        } else {
            // Join the filtered parts back with |
            parts.join(" | ")
        }
    }

    /// Simplifies common `.where(resolve() is ResourceType)` patterns.
    ///
    /// In FHIR SearchParameters, patterns like `subject.where(resolve() is Patient)`
    /// are used to filter references by target type. Since we're extracting references
    /// for indexing (not actually resolving them), we can safely strip this pattern
    /// and just extract the reference value.
    fn simplify_resolve_pattern(&self, expr: &str) -> String {
        // Pattern: .where(resolve() is SomeType)
        // We want to remove this suffix since we just need the reference value
        if let Some(where_pos) = expr.find(".where(resolve()") {
            // Find the matching closing paren
            let after_where = &expr[where_pos..];
            if after_where.rfind(')').is_some() {
                // Return everything before .where(...)
                return expr[..where_pos].to_string();
            }
        }
        expr.to_string()
    }

    /// Evaluates a FHIRPath expression against a resource using the helios-fhirpath evaluator.
    fn evaluate_fhirpath(
        &self,
        resource: &Value,
        expression: &str,
    ) -> Result<Vec<Value>, ExtractionError> {
        // Convert JSON to EvaluationResult and set up context
        let eval_result = json_to_evaluation_result(resource)?;

        // Create evaluation context with the resource as 'this'
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(eval_result);

        // Evaluate the FHIRPath expression
        let result = helios_fhirpath::evaluate_expression(expression, &context).map_err(|e| {
            ExtractionError::FhirPathError {
                expression: expression.to_string(),
                message: e,
            }
        })?;

        // Convert EvaluationResult back to JSON values
        evaluation_result_to_json_values(&result)
    }
}

/// Search parameters whose spec `expression` names an element that is *not*
/// the data they filter on. They must never produce index rows, whatever the
/// expression says.
///
/// `_in` (R5/R6) is the whole set today. It carries `expression:
/// "Resource.id"`, but the parameter means "this resource is a member of the
/// referenced List or Group" — the id is a placeholder, not a filter target.
/// Indexing it writes one reference row per resource pointing at the resource
/// itself, so `?_in=42` would match `Patient/42` through the ordinary bare-id
/// reference branch and answer a membership question with an identity test,
/// while every reindex adds a junk row per resource. Membership resolution is
/// not implemented (#638); `helios-rest` rejects `_in` outright rather than
/// answer it wrongly.
///
/// This list is deliberately about *expressions that lie*, not about
/// `_`-prefixed parameters in general: `_id`, `_lastUpdated` and the `meta`
/// set carry truthful expressions and are indexed normally, and the
/// `SearchParamType::Special` parameters (`_filter`, `_has`, `_text`, …) ship
/// with an empty expression and are already skipped below.
const NON_INDEXABLE_PARAM_CODES: [&str; 1] = ["_in"];

/// Splits a FHIRPath expression into its top-level union (`|`) members.
///
/// A plain `split('|')` also cuts inside string literals and parentheses, and
/// the fragments it leaves are not parseable FHIRPath. That used to be
/// harmless — an unbalanced fragment matched no resource-type prefix and was
/// dropped — but the abstract-base strip accepts a fragment on its prefix
/// alone, so `Resource.x.where(v = 'a|b')` would hand the evaluator
/// `x.where(v = 'a`, whose parse error aborts extraction for *every* member of
/// that parameter, concrete ones included.
///
/// A `|` inside `(...)`, `[...]`, `'...'` (with `\'` escapes) or a backtick
/// delimited identifier therefore stays part of its member. Members are
/// returned trimmed.
fn split_union_members(expression: &str) -> Vec<&str> {
    let bytes = expression.as_bytes();
    let mut members = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                // Skip an escaped character whole, so `\'` does not close the
                // literal and `\\` does not swallow the quote that follows.
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'\'' | b'`' => quote = Some(c),
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                // `depth <= 0` rather than `== 0`: an expression with more
                // closing than opening parens must still split into members
                // rather than collapsing into one.
                b'|' if depth <= 0 => {
                    members.push(expression[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    members.push(expression[start..].trim());
    members
}

/// Strips a leading abstract base type from one union member of a FHIRPath
/// expression, returning the resource-relative remainder.
///
/// The `Resource`-level SearchParameters carry expressions like
/// `Resource.meta.source` (`_source`) and `Resource.id` (`_id`). Evaluated
/// against a concrete resource these resolve to nothing — FHIRPath matches the
/// leading identifier against the resource's own type, so `Patient.meta.source`
/// and `meta.source` both work but `Resource.meta.source` yields an empty
/// collection, and the parameter is never indexed (#523). Since these
/// expressions apply to every resource, the prefix is dropped and the rest is
/// evaluated relative to the resource.
///
/// Leading parentheses are carried across, so a parenthesized member
/// (`(Resource.meta.source)`) strips to `(meta.source)` rather than falling
/// through and silently extracting nothing. The closing parens sit in the
/// untouched remainder, so the member stays balanced.
///
/// Returns `None` for parts that are not abstract-base-prefixed, so concrete
/// prefixes (`Patient.name`) keep their normal filtering behaviour. The common
/// case borrows from `part`; only the forms needing reassembly allocate.
fn strip_abstract_base_prefix(part: &str) -> Option<Cow<'_, str>> {
    let open = part.len() - part.trim_start_matches('(').len();
    let (parens, body) = part.split_at(open);
    let body = body.trim_start();

    for base in ABSTRACT_BASE_TYPES {
        let Some(rest) = body.strip_prefix(base) else {
            continue;
        };
        let stripped: Cow<'_, str> = if let Some(path) = rest.strip_prefix('.') {
            Cow::Borrowed(path)
        } else if rest.is_empty() {
            // The bare type name selects the resource itself (composite base
            // expressions use this form, e.g. `Observation`).
            Cow::Borrowed("$this")
        } else if rest.starts_with(')') {
            Cow::Owned(format!("$this{rest}"))
        } else {
            // A concrete type that merely starts with the same letters
            // (`ResourceThing.name`) is not an abstract base.
            continue;
        };
        return Some(if parens.is_empty() {
            stripped
        } else {
            Cow::Owned(format!("{parens}{stripped}"))
        });
    }
    None
}

/// Rewrites FHIRPath choice-type casts to concrete element names.
///
/// The extractor evaluates expressions against schema-less JSON, where a cast
/// like `value as Quantity` cannot resolve `value` to the stored `valueQuantity`
/// field. FHIR choice elements are serialized as `<element><Type>` (e.g.
/// `valueQuantity`, `medicationCodeableConcept`, `occurrenceDateTime`), so we
/// rewrite the three cast forms used in SearchParameter expressions to that
/// concrete name:
///
/// - `(Observation.value as Quantity)` → `Observation.valueQuantity`
/// - `value.as(Quantity)`              → `valueQuantity`
/// - `Observation.value.ofType(Quantity)` → `Observation.valueQuantity`
/// - `Observation.value as Quantity`   → `Observation.valueQuantity`
///
/// (The loader normalizes the `X as Type` form to `X.ofType(Type)`, so that
/// form is what usually reaches the extractor.)
///
/// Stripping the parentheses in the `(... as Type)` form is intentional: it also
/// lets `filter_expression_for_resource` recognize the `ResourceType.` prefix,
/// which it otherwise drops for parenthesized union members.
fn rewrite_choice_types(expression: &str) -> String {
    static AS_FN: OnceLock<Regex> = OnceLock::new();
    static OF_TYPE: OnceLock<Regex> = OnceLock::new();
    static PAREN_AS: OnceLock<Regex> = OnceLock::new();
    static BARE_AS: OnceLock<Regex> = OnceLock::new();

    let path = r"[A-Za-z_][A-Za-z0-9_.]*";
    let ty = r"[A-Za-z][A-Za-z0-9]*";
    let as_fn =
        AS_FN.get_or_init(|| Regex::new(&format!(r"({path})\.as\(\s*({ty})\s*\)")).unwrap());
    let of_type =
        OF_TYPE.get_or_init(|| Regex::new(&format!(r"({path})\.ofType\(\s*({ty})\s*\)")).unwrap());
    let paren_as =
        PAREN_AS.get_or_init(|| Regex::new(&format!(r"\(\s*({path})\s+as\s+({ty})\s*\)")).unwrap());
    let bare_as = BARE_AS.get_or_init(|| Regex::new(&format!(r"({path})\s+as\s+({ty})")).unwrap());

    let concrete = |caps: &regex::Captures| -> String {
        let base = &caps[1];
        let type_name = &caps[2];
        let mut chars = type_name.chars();
        let capitalized = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
        format!("{}{}", base, capitalized)
    };

    // `.as(Type)` / `.ofType(Type)` and `(path as Type)` first (the latter also
    // drops parens), then any remaining bare `path as Type`.
    let step1 = as_fn.replace_all(expression, &concrete);
    let step2 = of_type.replace_all(&step1, &concrete);
    let step3 = paren_as.replace_all(&step2, &concrete);
    bare_as.replace_all(&step3, &concrete).into_owned()
}

/// Converts a serde_json::Value to an EvaluationResult.
fn json_to_evaluation_result(value: &Value) -> Result<EvaluationResult, ExtractionError> {
    match value {
        Value::Null => Ok(EvaluationResult::Empty),
        Value::Bool(b) => Ok(EvaluationResult::boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvaluationResult::integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvaluationResult::decimal(Decimal::try_from(f).map_err(
                    |e| ExtractionError::ConversionError {
                        message: format!("Invalid decimal: {}", e),
                    },
                )?))
            } else {
                Err(ExtractionError::ConversionError {
                    message: "Invalid number".to_string(),
                })
            }
        }
        Value::String(s) => Ok(EvaluationResult::string(s.clone())),
        Value::Array(arr) => {
            let results: Result<Vec<_>, _> = arr.iter().map(json_to_evaluation_result).collect();
            Ok(EvaluationResult::collection(results?))
        }
        Value::Object(obj) => {
            let mut map = HashMap::new();
            for (key, val) in obj {
                let eval_val = json_to_evaluation_result(val)?;
                map.insert(key.clone(), eval_val);
            }
            Ok(EvaluationResult::Object {
                map,
                type_info: None,
            })
        }
    }
}

/// Converts an EvaluationResult back to JSON values for the converter.
fn evaluation_result_to_json_values(
    result: &EvaluationResult,
) -> Result<Vec<Value>, ExtractionError> {
    match result {
        EvaluationResult::Empty => Ok(Vec::new()),
        EvaluationResult::Boolean(b, _, _) => Ok(vec![Value::Bool(*b)]),
        EvaluationResult::String(s, _, _) => Ok(vec![Value::String(s.clone())]),
        EvaluationResult::Integer(i, _, _) => Ok(vec![Value::Number((*i).into())]),
        EvaluationResult::Integer64(i, _, _) => Ok(vec![Value::Number((*i).into())]),
        EvaluationResult::Decimal(d, _, _) => {
            // Convert decimal to JSON number
            let f: f64 = (*d).try_into().unwrap_or(0.0);
            Ok(vec![Value::Number(
                serde_json::Number::from_f64(f).unwrap_or_else(|| serde_json::Number::from(0)),
            )])
        }
        EvaluationResult::Date(s, _, _) => Ok(vec![Value::String(s.clone())]),
        EvaluationResult::DateTime(s, _, _) => Ok(vec![Value::String(s.clone())]),
        EvaluationResult::Time(s, _, _) => Ok(vec![Value::String(s.clone())]),
        EvaluationResult::Quantity(value, unit, _, _) => {
            // Convert Quantity to JSON object
            let f: f64 = (*value).try_into().unwrap_or(0.0);
            Ok(vec![serde_json::json!({
                "value": f,
                "unit": unit
            })])
        }
        EvaluationResult::Collection { items, .. } => {
            let mut values = Vec::new();
            for item in items {
                values.extend(evaluation_result_to_json_values(item)?);
            }
            Ok(values)
        }
        EvaluationResult::Object { map, .. } => {
            // Convert object back to JSON
            let mut obj = serde_json::Map::new();
            for (key, val) in map {
                let json_vals = evaluation_result_to_json_values(val)?;
                // Check if the original value was a Collection - if so, preserve it as an array
                // even if it has only one element, since FHIR arrays should stay as arrays
                let is_collection = matches!(val, EvaluationResult::Collection { .. });
                if is_collection {
                    // Always preserve arrays as arrays
                    obj.insert(key.clone(), Value::Array(json_vals));
                } else if json_vals.len() == 1 {
                    obj.insert(key.clone(), json_vals.into_iter().next().unwrap());
                } else if !json_vals.is_empty() {
                    obj.insert(key.clone(), Value::Array(json_vals));
                }
            }
            Ok(vec![Value::Object(obj)])
        }
    }
}

impl std::fmt::Debug for SearchParameterExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchParameterExtractor").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::loader::SearchParameterLoader;
    use helios_fhir::FhirVersion;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn rewrite_choice_types_handles_all_forms() {
        // `.as(Type)` form.
        assert_eq!(rewrite_choice_types("value.as(Quantity)"), "valueQuantity");
        // `.ofType(Type)` form (what the loader normalizes `as` to).
        assert_eq!(
            rewrite_choice_types("(Observation.value.ofType(Quantity))"),
            "(Observation.valueQuantity)"
        );
        // Parenthesized `as` form drops the parens.
        assert_eq!(
            rewrite_choice_types("(Observation.value as Quantity)"),
            "Observation.valueQuantity"
        );
        // Lower-case primitive type names are capitalized.
        assert_eq!(
            rewrite_choice_types("(RiskAssessment.occurrence as dateTime)"),
            "RiskAssessment.occurrenceDateTime"
        );
        // Unions are rewritten member-by-member; non-cast parts are untouched.
        assert_eq!(
            rewrite_choice_types("value.as(Quantity) | value.as(Range)"),
            "valueQuantity | valueRange"
        );
        assert_eq!(rewrite_choice_types("Observation.code"), "Observation.code");
    }

    fn create_test_extractor() -> SearchParameterExtractor {
        create_test_extractor_for(FhirVersion::R4)
    }

    fn create_test_extractor_for(version: FhirVersion) -> SearchParameterExtractor {
        let loader = SearchParameterLoader::new(version);
        let mut registry = SearchParameterRegistry::new();

        // Load minimal fallback
        if let Ok(params) = loader.load_embedded() {
            for param in params {
                let _ = registry.register(param);
            }
        }

        // Load spec file for full parameter support
        // CARGO_MANIFEST_DIR for this crate is crates/persistence
        // We need to go up two levels to reach the workspace root
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        if let Ok(params) = loader.load_from_spec_file(&data_dir) {
            for param in params {
                let _ = registry.register(param);
            }
        }

        SearchParameterExtractor::new(Arc::new(RwLock::new(registry)))
    }

    #[test]
    fn test_extract_patient_name() {
        let extractor = create_test_extractor();

        let patient = json!({
            "resourceType": "Patient",
            "id": "123",
            "name": [
                {
                    "family": "Smith",
                    "given": ["John", "James"]
                }
            ]
        });

        let values = extractor.extract(&patient, "Patient").unwrap();

        // Should have extracted name values
        let name_values: Vec<_> = values.iter().filter(|v| v.param_name == "name").collect();
        assert!(!name_values.is_empty(), "Should extract 'name' values");

        // Should have extracted family
        let family_values: Vec<_> = values.iter().filter(|v| v.param_name == "family").collect();
        assert!(!family_values.is_empty(), "Should extract 'family' values");
    }

    #[test]
    fn test_extract_patient_identifier() {
        let extractor = create_test_extractor();

        let patient = json!({
            "resourceType": "Patient",
            "id": "123",
            "identifier": [
                {
                    "system": "http://hospital.org/mrn",
                    "value": "12345"
                }
            ]
        });

        let values = extractor.extract(&patient, "Patient").unwrap();

        let id_values: Vec<_> = values
            .iter()
            .filter(|v| v.param_name == "identifier")
            .collect();
        assert!(!id_values.is_empty(), "Should extract 'identifier' values");

        if let IndexValue::Token { system, code, .. } = &id_values[0].value {
            assert_eq!(system.as_ref().unwrap(), "http://hospital.org/mrn");
            assert_eq!(code, "12345");
        }
    }

    #[test]
    fn test_extract_observation_values() {
        let extractor = create_test_extractor();

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs1",
            "code": {
                "coding": [
                    {
                        "system": "http://loinc.org",
                        "code": "8867-4"
                    }
                ]
            },
            "subject": {
                "reference": "Patient/123"
            },
            "valueQuantity": {
                "value": 120.5,
                "unit": "mmHg"
            }
        });

        let values = extractor.extract(&observation, "Observation").unwrap();

        // Should have code
        let code_values: Vec<_> = values.iter().filter(|v| v.param_name == "code").collect();
        assert!(!code_values.is_empty(), "Should extract 'code' values");

        // Should have subject
        let subject_values: Vec<_> = values
            .iter()
            .filter(|v| v.param_name == "subject")
            .collect();
        assert!(
            !subject_values.is_empty(),
            "Should extract 'subject' values"
        );
    }

    #[test]
    fn test_invalid_resource() {
        let extractor = create_test_extractor();

        let not_object = json!("string");
        let result = extractor.extract(&not_object, "Patient");
        assert!(result.is_err());
    }

    #[test]
    fn test_resource_type_mismatch() {
        let extractor = create_test_extractor();

        let patient = json!({
            "resourceType": "Patient",
            "id": "123"
        });

        let result = extractor.extract(&patient, "Observation");
        assert!(result.is_err());
    }

    #[test]
    fn test_fhirpath_with_where_clause() {
        let extractor = create_test_extractor();

        // Test a patient with multiple names - FHIRPath should be able to filter
        let patient = json!({
            "resourceType": "Patient",
            "id": "123",
            "name": [
                {
                    "use": "official",
                    "family": "Smith",
                    "given": ["John"]
                },
                {
                    "use": "nickname",
                    "given": ["Johnny"]
                }
            ]
        });

        let values = extractor.extract(&patient, "Patient").unwrap();

        // Should extract all names (both official and nickname)
        let name_values: Vec<_> = values.iter().filter(|v| v.param_name == "name").collect();
        assert!(
            name_values.len() >= 2,
            "Should extract multiple name values"
        );
    }

    #[test]
    fn test_extract_observation_code_with_display() {
        let extractor = create_test_extractor();

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs1",
            "status": "final",
            "code": {
                "coding": [
                    {
                        "system": "http://loinc.org",
                        "code": "8867-4",
                        "display": "Heart rate"
                    }
                ]
            }
        });

        // Extract values
        let values = extractor.extract(&observation, "Observation").unwrap();

        // Should have extracted code values
        let code_values: Vec<_> = values.iter().filter(|v| v.param_name == "code").collect();
        assert!(!code_values.is_empty(), "Should extract 'code' values");

        // Check that display is populated
        if let Some(first_code) = code_values.first() {
            if let IndexValue::Token { display, .. } = &first_code.value {
                assert_eq!(
                    display.as_deref(),
                    Some("Heart rate"),
                    "Display should be populated"
                );
            }
        }
    }

    #[test]
    fn test_extract_resource_id() {
        let extractor = create_test_extractor();

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1"
        });

        let values = extractor.extract(&patient, "Patient").unwrap();

        // Should have extracted _id
        let id_values: Vec<_> = values.iter().filter(|v| v.param_name == "_id").collect();
        assert!(!id_values.is_empty(), "Should extract '_id' parameter");

        // Check the value
        if let Some(first_id) = id_values.first() {
            if let IndexValue::Token { code, .. } = &first_id.value {
                assert_eq!(code, "p1", "_id should be 'p1'");
            }
        }
    }

    #[test]
    fn test_json_to_evaluation_result() {
        // Test basic types
        assert!(matches!(
            json_to_evaluation_result(&json!(null)).unwrap(),
            EvaluationResult::Empty
        ));

        assert!(matches!(
            json_to_evaluation_result(&json!(true)).unwrap(),
            EvaluationResult::Boolean(true, _, _)
        ));

        assert!(matches!(
            json_to_evaluation_result(&json!("test")).unwrap(),
            EvaluationResult::String(s, _, _) if s == "test"
        ));

        assert!(matches!(
            json_to_evaluation_result(&json!(42)).unwrap(),
            EvaluationResult::Integer(42, _, _)
        ));

        // Test array
        if let EvaluationResult::Collection { items, .. } =
            json_to_evaluation_result(&json!([1, 2, 3])).unwrap()
        {
            assert_eq!(items.len(), 3);
        } else {
            panic!("Expected collection");
        }

        // Test object
        if let EvaluationResult::Object { map, .. } =
            json_to_evaluation_result(&json!({"key": "value"})).unwrap()
        {
            assert!(map.contains_key("key"));
        } else {
            panic!("Expected object");
        }
    }

    #[test]
    fn test_filter_expression_for_resource() {
        let extractor = create_test_extractor();

        // Test multi-resource expression (like patient search param)
        let complex_expr =
            "AllergyIntolerance.patient | Immunization.patient | Observation.subject";
        let filtered = extractor.filter_expression_for_resource(complex_expr, "Immunization");
        assert_eq!(filtered, "Immunization.patient");

        // Test with no matching parts - should return original
        let no_match = extractor.filter_expression_for_resource(complex_expr, "Patient");
        assert_eq!(no_match, complex_expr);

        // Test simple expression (single resource type)
        let simple_expr = "Patient.name";
        let simple_filtered = extractor.filter_expression_for_resource(simple_expr, "Patient");
        assert_eq!(simple_filtered, "Patient.name");

        // Test that partial matches don't count (Observation shouldn't match Obs)
        let partial = extractor.filter_expression_for_resource("Observation.code", "Obs");
        assert_eq!(partial, "Observation.code");

        // Test stripping .where(resolve() is X) pattern
        let with_resolve = "Observation.subject.where(resolve() is Patient) | Patient.link.other";
        let stripped = extractor.filter_expression_for_resource(with_resolve, "Observation");
        assert_eq!(stripped, "Observation.subject");

        // Test real-world patient search param pattern
        let patient_expr = "CarePlan.subject.where(resolve() is Patient) | Observation.subject.where(resolve() is Patient)";
        let careplan_filtered = extractor.filter_expression_for_resource(patient_expr, "CarePlan");
        assert_eq!(careplan_filtered, "CarePlan.subject");
        let obs_filtered = extractor.filter_expression_for_resource(patient_expr, "Observation");
        assert_eq!(obs_filtered, "Observation.subject");
    }

    #[test]
    fn test_filter_expression_strips_abstract_base_prefix() {
        let extractor = create_test_extractor();

        // `Resource.`-prefixed expressions apply to every resource; the prefix
        // has to go, or FHIRPath resolves nothing against a concrete resource.
        assert_eq!(
            extractor.filter_expression_for_resource("Resource.meta.source", "Patient"),
            "meta.source"
        );
        assert_eq!(
            extractor.filter_expression_for_resource("Resource.id", "Observation"),
            "id"
        );
        assert_eq!(
            extractor.filter_expression_for_resource("DomainResource.meta.tag", "Patient"),
            "meta.tag"
        );

        // Also when the resource type is itself the abstract base.
        assert_eq!(
            extractor.filter_expression_for_resource("Resource.meta.source", "Resource"),
            "meta.source"
        );

        // A bare abstract base selects the resource itself. No shipped parameter
        // uses that form, but a composite base expression could; assert the
        // substitution and that the evaluator actually resolves it.
        assert_eq!(
            extractor.filter_expression_for_resource("Resource", "Patient"),
            "$this"
        );
        let patient = json!({"resourceType": "Patient", "id": "p1"});
        let this = extractor.evaluate_fhirpath(&patient, "$this").unwrap();
        assert_eq!(this.len(), 1);
        assert_eq!(this[0]["id"], "p1");

        // A concrete type that merely starts with the same letters is untouched.
        assert_eq!(
            extractor.filter_expression_for_resource("ResourceThing.name", "ResourceThing"),
            "ResourceThing.name"
        );

        // Union members are handled independently.
        assert_eq!(
            extractor.filter_expression_for_resource(
                "Patient.name | Resource.meta.source | Observation.code",
                "Patient"
            ),
            "Patient.name | meta.source"
        );

        // A parenthesized member keeps its parens and stays balanced. Before
        // this, `(Resource.meta.source)` matched neither branch, the whole
        // expression came back unchanged, and it evaluated to nothing — the
        // #523 symptom, silently.
        assert_eq!(
            extractor.filter_expression_for_resource("(Resource.meta.source)", "Patient"),
            "(meta.source)"
        );
        assert_eq!(
            extractor.filter_expression_for_resource("((Resource.meta.source))", "Patient"),
            "((meta.source))"
        );
        assert_eq!(
            extractor.filter_expression_for_resource("(Resource)", "Patient"),
            "($this)"
        );
        let source_carrier = json!({
            "resourceType": "Patient",
            "id": "p1",
            "meta": {"source": "http://example.org/src"}
        });
        let evaluated = extractor
            .evaluate_fhirpath(&source_carrier, "(meta.source)")
            .expect("the stripped form must parse and evaluate");
        assert_eq!(evaluated.len(), 1);
    }

    /// `|` inside a string literal or parentheses is data, not a union
    /// separator.
    #[test]
    fn union_split_respects_literals_and_parens() {
        assert_eq!(
            split_union_members("Patient.name | Observation.code"),
            vec!["Patient.name", "Observation.code"]
        );
        assert_eq!(
            split_union_members("Patient.telecom.where(system = 'a|b')"),
            vec!["Patient.telecom.where(system = 'a|b')"]
        );
        assert_eq!(
            split_union_members("Patient.a.where(v = 'x|y') | Observation.b"),
            vec!["Patient.a.where(v = 'x|y')", "Observation.b"]
        );
        // An escaped quote does not end the literal.
        assert_eq!(
            split_union_members(r"Patient.a.where(v = 'it\'s|fine')"),
            vec![r"Patient.a.where(v = 'it\'s|fine')"]
        );
        // Backtick-delimited identifiers behave like literals.
        assert_eq!(
            split_union_members("Patient.`odd|name`"),
            vec!["Patient.`odd|name`"]
        );
    }

    /// A `|` inside a `Resource.`-prefixed member used to be split anyway,
    /// handing the evaluator an unbalanced fragment. The parse error aborts
    /// `extract_for_param`, so **every** member of that parameter — the
    /// concrete ones included — stopped being indexed.
    #[test]
    fn literal_pipe_in_abstract_member_does_not_break_extraction() {
        let extractor = create_test_extractor();

        let expr = "Patient.name | Resource.meta.tag.where(code = 'a|b')";
        let filtered = extractor.filter_expression_for_resource(expr, "Patient");
        assert_eq!(filtered, "Patient.name | meta.tag.where(code = 'a|b')");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith"}],
            "meta": {"tag": [{"code": "a|b"}]}
        });
        // The whole filtered expression must parse; the pre-fix fragment
        // `meta.tag.where(code = 'a` does not.
        let values = extractor
            .evaluate_fhirpath(&patient, &filtered)
            .expect("the filtered expression must parse");
        assert!(!values.is_empty());
    }

    /// The `DomainResource` half of `ABSTRACT_BASE_TYPES` has to be reachable:
    /// the registry buckets a definition under each declared `base`, so
    /// `base: ["DomainResource"]` lands in its own bucket that a lookup of
    /// `"Resource"` alone never sees.
    #[test]
    fn domain_resource_based_parameters_are_extracted() {
        let extractor = create_test_extractor();
        {
            let mut registry = extractor.registry.write();
            registry
                .register(
                    SearchParameterDefinition::new(
                        "http://example.org/SearchParameter/DomainResource-narrative-status",
                        "narrative-status",
                        SearchParamType::Token,
                        "DomainResource.text.status",
                    )
                    .with_base(vec!["DomainResource"]),
                )
                .expect("register DomainResource-based parameter");
        }

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "text": {"status": "generated", "div": "<div>x</div>"}
        });

        let values = extractor.extract(&patient, "Patient").unwrap();
        let found: Vec<_> = values
            .iter()
            .filter(|v| v.param_name == "narrative-status")
            .collect();
        assert_eq!(
            found.len(),
            1,
            "a DomainResource-based parameter should be indexed, got {values:?}"
        );
    }

    /// `_in` (R5/R6) carries the spec's placeholder `Resource.id` expression.
    /// Indexing it writes one self-referential row per resource, and
    /// `?_in=42` then matches `Patient/42` through the ordinary bare-id
    /// reference branch — a membership question answered with an identity
    /// test. `_language`, whose expression is truthful, must still be indexed.
    #[cfg(feature = "R5")]
    #[test]
    fn r5_membership_parameter_is_not_indexed_but_language_is() {
        let extractor = create_test_extractor_for(FhirVersion::R5);

        // Both are registered on R5; only one of them may reach the index.
        {
            let registry = extractor.registry.read();
            for code in ["_in", "_language"] {
                assert!(
                    registry.get_param("Resource", code).is_some(),
                    "{code} should be a registered R5 Resource-level parameter"
                );
            }
        }

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "language": "en-US"
        });

        let values = extractor.extract(&patient, "Patient").unwrap();

        assert!(
            !values.iter().any(|v| v.param_name == "_in"),
            "_in must never be indexed, got {:?}",
            values
                .iter()
                .filter(|v| v.param_name == "_in")
                .collect::<Vec<_>>()
        );

        let language: Vec<_> = values
            .iter()
            .filter(|v| v.param_name == "_language")
            .collect();
        assert_eq!(
            language.len(),
            1,
            "_language should be indexed exactly once"
        );
        match &language[0].value {
            IndexValue::Token { code, .. } => assert_eq!(code, "en-US"),
            other => panic!("_language should index as a token, got {other:?}"),
        }
    }

    /// Every `Resource`-level meta parameter must produce a positive index row —
    /// the failure in #523 was silent, because an unindexed parameter simply
    /// matches nothing (or, on backends that drop it, everything).
    #[test]
    fn test_extract_meta_level_parameters() {
        let extractor = create_test_extractor();

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "meta": {
                "source": "http://example.org/src",
                "profile": ["http://example.org/StructureDefinition/my-patient"],
                "lastUpdated": "2024-01-01T00:00:00Z",
                "tag": [{"system": "http://example.org/tags", "code": "t1"}],
                "security": [{"system": "http://example.org/labels", "code": "R"}]
            }
        });

        let values = extractor.extract(&patient, "Patient").unwrap();
        let find = |name: &str| -> Vec<&ExtractedValue> {
            values.iter().filter(|v| v.param_name == name).collect()
        };

        let source = find("_source");
        assert_eq!(source.len(), 1, "_source should be indexed exactly once");
        match &source[0].value {
            IndexValue::Uri(value) => assert_eq!(value, "http://example.org/src"),
            other => panic!("_source should index as a URI, got {:?}", other),
        }

        match &find("_profile")[..] {
            [v] => match &v.value {
                IndexValue::Uri(value) => {
                    assert_eq!(value, "http://example.org/StructureDefinition/my-patient")
                }
                other => panic!("_profile should index as a URI, got {:?}", other),
            },
            other => panic!("_profile should be indexed exactly once, got {:?}", other),
        }

        match &find("_tag")[..] {
            [v] => match &v.value {
                IndexValue::Token { system, code, .. } => {
                    assert_eq!(system.as_deref(), Some("http://example.org/tags"));
                    assert_eq!(code, "t1");
                }
                other => panic!("_tag should index as a token, got {:?}", other),
            },
            other => panic!("_tag should be indexed exactly once, got {:?}", other),
        }

        match &find("_security")[..] {
            [v] => match &v.value {
                IndexValue::Token { system, code, .. } => {
                    assert_eq!(system.as_deref(), Some("http://example.org/labels"));
                    assert_eq!(code, "R");
                }
                other => panic!("_security should index as a token, got {:?}", other),
            },
            other => panic!("_security should be indexed exactly once, got {:?}", other),
        }

        assert!(
            !find("_lastUpdated").is_empty(),
            "_lastUpdated should be indexed"
        );
        assert!(!find("_id").is_empty(), "_id should be indexed");
    }

    #[test]
    fn test_extract_immunization_patient() {
        let extractor = create_test_extractor();

        let immunization = json!({
            "resourceType": "Immunization",
            "id": "test-imm",
            "status": "completed",
            "vaccineCode": {
                "coding": [{
                    "system": "http://hl7.org/fhir/sid/cvx",
                    "code": "140"
                }]
            },
            "patient": {
                "reference": "Patient/test-patient"
            },
            "occurrenceDateTime": "2021-01-01"
        });

        let values = extractor.extract(&immunization, "Immunization").unwrap();

        // Should have extracted patient reference
        let patient_values: Vec<_> = values
            .iter()
            .filter(|v| v.param_name == "patient")
            .collect();
        assert!(
            !patient_values.is_empty(),
            "Should extract 'patient' values from Immunization"
        );

        // Check the reference value
        if let IndexValue::Reference { reference, .. } = &patient_values[0].value {
            assert!(
                reference.contains("Patient/test-patient") || reference.contains("test-patient"),
                "Should contain patient reference, got: {}",
                reference
            );
        }
    }
}
