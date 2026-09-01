//! SearchParameter Value Extractor.
//!
//! Uses FHIRPath expressions to extract searchable values from FHIR resources.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use helios_fhir::search::ABSTRACT_BASE_TYPES;
use helios_fhirpath::EvaluationContext;
use helios_fhirpath::parser::Expression as FhirPathExpression;
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

    /// Which slot of its column family this composite component occupies.
    ///
    /// A composite's components are stored in the value columns of their own
    /// type — `code-value-quantity` puts `code` in the token columns and
    /// `value-quantity` in the quantity columns. 24 of the 46 R4 composites
    /// have two components of the *same* type (almost all `token`+`token`,
    /// e.g. `Observation.code-value-concept`), which would otherwise collide
    /// in one row. The slot disambiguates them: 1 for the first component of
    /// a given type in the parameter's component list, 2 for the second.
    ///
    /// `None` for non-composite values. Backends that keep one row per
    /// component ignore it; the Postgres writer uses it to fold a group's
    /// components into a single row (issue #279).
    pub composite_slot: Option<u8>,

    /// How many components the composite parameter's definition declares that
    /// this extractor can actually index — i.e. how many distinct axes a
    /// *complete* instance of this composite contributes.
    ///
    /// A composite search matches only when EVERY component matches, so a group
    /// that is missing a component can never satisfy one. Postgres stores a
    /// composite instance as one denormalized row, and without this the writer
    /// cannot tell a complete row from a partial one — it emitted both. On the
    /// benchmark corpus that was ~5M rows (of 39.5M) that no query can reach:
    /// every Observation with a `code` but no `valueDateTime` still got a
    /// `code-value-date` row.
    ///
    /// `None` for non-composite values.
    pub composite_arity: Option<u8>,
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
            composite_slot: None,
            composite_arity: None,
        }
    }

    /// Sets the composite group ID.
    pub fn with_composite_group(mut self, group: u32) -> Self {
        self.composite_group = Some(group);
        self
    }

    /// Sets the composite component's slot within its column family.
    pub fn with_composite_slot(mut self, slot: u8) -> Self {
        self.composite_slot = Some(slot);
        self
    }

    /// Sets how many components a complete instance of this composite has.
    pub fn with_composite_arity(mut self, arity: u8) -> Self {
        self.composite_arity = Some(arity);
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

/// A search-parameter expression with all of its per-`(expression,
/// resource_type)` preparation already done.
///
/// Everything the extractor did to an expression before evaluating it is a pure
/// function of the expression text and the resource type:
///
/// | step                                        | cost per call            |
/// |---------------------------------------------|--------------------------|
/// | `rewrite_choice_types`                      | 4 regex `replace_all`    |
/// | `filter_expression_for_resource`            | split unions, `String` per member, join |
/// | `helios_fhirpath::parse_expression`         | **build the chumsky parser**, then parse |
/// | `resolve_target_types`                      | split the unions a second time |
///
/// None of it reads the resource and none of it reads the registry —
/// `retained_parts`, `simplify_resolve_pattern`, `strip_abstract_base_prefix`
/// and `resolve_target_type` are free functions of those two strings — yet all
/// of it ran again for every resource written, once per active parameter:
/// 32 parameters for a Patient, 47 for an Observation.
///
/// On the FHIR benchmark's import suite (1.63M resources across 1,000 Synthea
/// transaction bundles) that is on the order of 50M chumsky parser
/// constructions to obtain a few thousand distinct ASTs. Memoizing on exactly
/// the two inputs the computation depends on makes a hit indistinguishable from
/// recomputing it.
///
/// Parse *failures* are cached as well, so a malformed custom SearchParameter
/// costs one parse for the process instead of one per resource; the
/// per-resource warning it produces is unchanged.
///
/// Bounded by the number of distinct `(expression, resource_type)` pairs the
/// registry can produce. A parameter is only ever prepared against a type in
/// its own `base`, so for the 1,375-parameter R4 spec file the live set is a
/// few thousand entries — the same order as the registry itself.
struct PreparedExpression {
    /// `filter_expression_for_resource(rewrite_choice_types(expression), rt)`.
    /// Kept as text because it appears verbatim in `ExtractionError` messages.
    filtered: String,
    /// `filtered`, parsed — or the parse error, verbatim as
    /// `helios_fhirpath::parse_expression` produced it. `None` when `filtered`
    /// is empty and there is nothing to evaluate.
    ast: Option<Result<Arc<FhirPathExpression>, String>>,
    /// `resolve_target_types(rewritten, rt)` — the reference-target restriction.
    /// Computed for every parameter and applied, as before, only to `reference`
    /// ones; computing it eagerly costs one extra union split per cache miss.
    target_types: Option<Vec<String>>,
}

/// A composite component's sub-expression, rewritten and parsed.
///
/// Component expressions evaluate relative to a base instance rather than the
/// resource root, so they are never resource-type filtered.
struct PreparedComponent {
    rewritten: String,
    ast: Result<Arc<FhirPathExpression>, String>,
}

/// Keyed resource type first, then expression, so a lookup borrows both keys
/// as `&str` and a cache hit allocates nothing.
type PreparedMap = HashMap<String, HashMap<String, Arc<PreparedExpression>>>;

fn prepared_cache() -> &'static RwLock<PreparedMap> {
    static CACHE: OnceLock<RwLock<PreparedMap>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn component_cache() -> &'static RwLock<HashMap<String, Arc<PreparedComponent>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Arc<PreparedComponent>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Parses `expr` into a shareable AST, keeping the exact error text
/// `helios_fhirpath::parse_expression` produces so a cached failure reads
/// identically to a fresh one.
fn parse_prepared(expr: &str) -> Result<Arc<FhirPathExpression>, String> {
    helios_fhirpath::parse_expression(expr).map(Arc::new)
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

        // One conversion of this resource into the evaluator's own tree, shared
        // by every parameter below. It used to live inside `evaluate_fhirpath`,
        // i.e. it was redone from scratch for each of the ~32 (Patient) to ~47
        // (Observation) active parameters — making the deep copy of the
        // resource, not the FHIRPath evaluation, the dominant cost of indexing
        // it. See the `PreparedExpression` note for the arithmetic.
        //
        // A conversion failure used to surface once per parameter, get logged
        // and skipped, and leave `extract` returning an empty vector. It still
        // returns an empty vector, with one warning instead of N. (In practice
        // it cannot fail: the only fallible arm is a `serde_json` number that is
        // neither `i64` nor `f64`, which the parser cannot produce.)
        let context = match Self::evaluation_context(resource) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::warn!(
                    "Failed to convert {} into an evaluation tree for indexing: {}",
                    resource_type,
                    e
                );
                return Ok(Vec::new());
            }
        };

        // Base instances for composite parameters, memoized per resource by
        // base expression. See `extract_composite`.
        let mut composite_bases: HashMap<String, Vec<EvaluationContext>> = HashMap::new();

        let mut results = Vec::new();

        // Get active parameters for this resource type
        let params = {
            let registry = self.registry.read();
            registry.get_active_params(resource_type)
        };

        for param in &params {
            match self.extract_for_param_in(resource, &context, &mut composite_bases, param) {
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
                match self.extract_for_param_in(resource, &context, &mut composite_bases, param) {
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
    ///
    /// Builds a one-off evaluation context for `resource`. Callers indexing a
    /// whole resource should go through [`Self::extract`], which builds one
    /// context and reuses it across every parameter.
    pub fn extract_for_param(
        &self,
        resource: &Value,
        param: &SearchParameterDefinition,
    ) -> Result<Vec<ExtractedValue>, ExtractionError> {
        let context = Self::evaluation_context(resource)?;
        let mut composite_bases = HashMap::new();
        self.extract_for_param_in(resource, &context, &mut composite_bases, param)
    }

    /// Extracts values for one parameter against an already-built context.
    ///
    /// `context` must have `resource` as its `this`; `resource` itself is still
    /// read for the `resourceType` the expression is filtered against.
    fn extract_for_param_in(
        &self,
        resource: &Value,
        context: &EvaluationContext,
        composite_bases: &mut HashMap<String, Vec<EvaluationContext>>,
        param: &SearchParameterDefinition,
    ) -> Result<Vec<ExtractedValue>, ExtractionError> {
        if NON_INDEXABLE_PARAM_CODES.contains(&param.code.as_str()) {
            return Ok(Vec::new());
        }

        // Composite parameters are indexed component-by-component, with all the
        // components of one composite instance sharing a `composite_group`.
        if matches!(param.param_type, SearchParamType::Composite) {
            return self.extract_composite(resource, context, composite_bases, param);
        }

        if param.expression.is_empty() {
            return Ok(Vec::new());
        }

        // Get the resource type from the resource
        let resource_type = resource
            .get("resourceType")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Choice-type rewriting, union filtering, parsing, and the reference
        // target restriction are all pure functions of
        // (`param.expression`, `resource_type`) and are done once per pair.
        let prepared = self.prepared(&param.expression, resource_type);
        let Some(ast) = prepared.ast.as_ref() else {
            return Ok(Vec::new());
        };
        let ast = ast.as_ref().map_err(|e| ExtractionError::FhirPathError {
            expression: prepared.filtered.clone(),
            message: e.clone(),
        })?;

        let values = Self::evaluate_prepared(context, ast, &prepared.filtered)?;

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

        // Re-apply the target-type restriction that `simplify_resolve_pattern`
        // had to strip from the expression. See [`Self::resolve_target_types`].
        if matches!(param.param_type, SearchParamType::Reference)
            && let Some(allowed) = prepared.target_types.as_deref()
        {
            Self::restrict_reference_targets(&mut results, allowed);
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
        context: &EvaluationContext,
        composite_bases: &mut HashMap<String, Vec<EvaluationContext>>,
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

        let prepared_base = self.prepared(&param.expression, resource_type);
        let Some(base_ast) = prepared_base.ast.as_ref() else {
            return Ok(Vec::new());
        };
        let base_ast = base_ast
            .as_ref()
            .map_err(|e| ExtractionError::FhirPathError {
                expression: prepared_base.filtered.clone(),
                message: e.clone(),
            })?;

        // Resolve each component's value type from the registry (by definition URL).
        let component_types: Vec<Option<SearchParamType>> = {
            let registry = self.registry.read();
            components
                .iter()
                .map(|c| registry.get_by_url(&c.definition).map(|d| d.param_type))
                .collect()
        };

        // Slot each component within its own column family: 1 for the first
        // component of a given type, 2 for the second. 24 of the 46 R4
        // composites pair two components of the same type, so without this a
        // denormalized row could not tell `code` from `value-concept`. The
        // Postgres query builder derives the same slots from the same registry
        // component order, so the two sides agree without storing a mapping.
        let component_slots: Vec<u8> = {
            let mut seen: std::collections::HashMap<SearchParamType, u8> =
                std::collections::HashMap::new();
            component_types
                .iter()
                .map(|t| match t {
                    Some(t) => {
                        let slot = seen.entry(*t).or_insert(0);
                        *slot += 1;
                        *slot
                    }
                    None => 1,
                })
                .collect()
        };

        // A composite search matches only when every component matches, so the
        // number of components this extractor can index is the arity a complete
        // instance must reach. Components with an unresolvable definition or an
        // empty expression are skipped below and can never contribute an axis,
        // so they do not count toward it.
        let arity = components
            .iter()
            .zip(component_types.iter())
            .filter(|(c, t)| t.is_some() && !c.expression.is_empty())
            .count()
            .min(u8::MAX as usize) as u8;

        // Each base instance becomes a composite group, and each gets its own
        // evaluation context because the component sub-expressions are relative
        // to it.
        //
        // Both halves of that are memoized per resource by base expression,
        // because several composites routinely share one. Observation — 42% of
        // the resources in the benchmark's Synthea corpus — carries eight
        // composite parameters over just three distinct bases:
        //
        //   Observation                            code-value-{concept,date,quantity,string}
        //   Observation | Observation.component    combo-code-value-{concept,quantity}
        //   Observation.component                  component-code-value-{concept,quantity}
        //
        // Six of those eight have a base that selects the resource itself, so
        // each one used to evaluate the base (which clones the whole evaluation
        // tree), rebuild the whole resource as JSON via
        // `evaluation_result_to_json_values`, and then convert that JSON back
        // into a tree — three full deep copies of the resource, eight times
        // over, to obtain three distinct answers.
        //
        // The contexts are built eagerly for every base instance rather than on
        // first use by a component. Every composite in the R4 spec has at least
        // one resolvable component, so nothing is built that is not used; the
        // only behavioural edge is that a conversion failure on a node whose
        // components are all unindexable would now surface. That conversion is
        // infallible in practice (see `extract`).
        if !composite_bases.contains_key(&prepared_base.filtered) {
            let nodes = Self::evaluate_prepared(context, base_ast, &prepared_base.filtered)?;
            let mut contexts = Vec::with_capacity(nodes.len());
            for node in &nodes {
                contexts.push(Self::evaluation_context(node)?);
            }
            composite_bases.insert(prepared_base.filtered.clone(), contexts);
        }
        let base_contexts = &composite_bases[&prepared_base.filtered];

        let mut results = Vec::new();
        for (group_idx, node_context) in base_contexts.iter().enumerate() {
            let group = group_idx as u32;
            for ((component, sub_type), slot) in components
                .iter()
                .zip(component_types.iter())
                .zip(component_slots.iter())
            {
                let sub_type = match sub_type {
                    Some(t) => *t,
                    None => continue, // unknown component definition — skip
                };
                if component.expression.is_empty() {
                    continue;
                }
                let comp = self.prepared_component(&component.expression);
                let comp_ast = comp
                    .ast
                    .as_ref()
                    .map_err(|e| ExtractionError::FhirPathError {
                        expression: comp.rewritten.clone(),
                        message: e.clone(),
                    })?;
                let values = Self::evaluate_prepared(node_context, comp_ast, &comp.rewritten)?;
                for value in values {
                    let converted = ValueConverter::convert(&value, sub_type, &param.code)?;
                    for idx_value in converted {
                        results.push(
                            ExtractedValue::new(&param.code, &param.url, sub_type, idx_value)
                                .with_composite_group(group)
                                .with_composite_slot(*slot)
                                .with_composite_arity(arity),
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
        match self.retained_parts(expression, resource_type) {
            // If no parts match, return the original expression.
            // This handles expressions that don't use ResourceType prefix.
            None => expression.to_string(),
            Some(parts) => parts
                .iter()
                .map(|p| self.simplify_resolve_pattern(p))
                .collect::<Vec<_>>()
                .join(" | "),
        }
    }

    /// The union members of `expression` that apply to `resource_type`, before
    /// `.where(resolve() is X)` is stripped.
    ///
    /// `None` means no member named this resource type, in which case callers
    /// fall back to the whole expression and nothing can be concluded about the
    /// reference targets.
    fn retained_parts(&self, expression: &str, resource_type: &str) -> Option<Vec<String>> {
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
            .collect();
        (!parts.is_empty()).then_some(parts)
    }

    /// The resource types a reference parameter is restricted to, if the
    /// definition restricts it at all.
    ///
    /// `simplify_resolve_pattern` has to strip `.where(resolve() is Patient)`
    /// before evaluation — `resolve()` cannot follow a reference out of the one
    /// resource being indexed. Stripping it silently widened the parameter:
    /// `Provenance.patient` is defined as `Provenance.target.where(resolve() is
    /// Patient)`, and indexing every `target` made `Provenance?patient=X` match
    /// a Provenance whose target is an *Encounter* with id X. On the benchmark
    /// corpus it also made `Provenance | patient` the second-largest parameter
    /// in the table at 1,626,336 rows — one per target, 1,626 per resource —
    /// nearly all of which no `patient` search should ever return.
    ///
    /// The type a reference points at is already in the reference string, so the
    /// restriction can be applied at extraction time without resolving anything.
    /// Returns `None` — meaning "do not filter" — unless EVERY retained union
    /// member carries a `resolve() is` clause; a member without one contributes
    /// unrestricted references, and dropping those would lose real index rows.
    fn resolve_target_types(&self, expression: &str, resource_type: &str) -> Option<Vec<String>> {
        let parts = self.retained_parts(expression, resource_type)?;
        let mut types = Vec::new();
        for part in &parts {
            let ty = resolve_target_type(part)?;
            if !types.contains(&ty) {
                types.push(ty);
            }
        }
        (!types.is_empty()).then_some(types)
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

    /// Keeps only the reference values whose target type the parameter allows.
    ///
    /// Conservative by construction: a value is dropped only when its type can
    /// be read off the reference AND is not in `allowed`. Contained (`#x`),
    /// `urn:uuid:` and bare-id references carry no type, so they are kept — a
    /// false positive costs a row, a false negative costs a search result.
    fn restrict_reference_targets(values: &mut Vec<ExtractedValue>, allowed: &[String]) {
        values.retain(|v| match &v.value {
            IndexValue::Reference {
                reference,
                resource_type,
                ..
            } => {
                // The converter already parses the plain forms; fall back for
                // the ones it does not (versioned references).
                let ty = resource_type
                    .as_deref()
                    .or_else(|| reference_target_type(reference));
                match ty {
                    Some(ty) => allowed.iter().any(|a| a == ty),
                    None => true,
                }
            }
            _ => true,
        });
    }

    /// The evaluation context for one resource: the resource converted into the
    /// evaluator's `EvaluationResult` tree, installed as `this`.
    ///
    /// This conversion is a full recursive copy of the resource — every string
    /// cloned, every JSON object rebuilt as a `HashMap` — so it is the single
    /// most expensive thing indexing does per resource, and the whole point of
    /// hoisting it is that one is enough for all of a resource's parameters.
    fn evaluation_context(resource: &Value) -> Result<EvaluationContext, ExtractionError> {
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(json_to_evaluation_result(resource)?);
        Ok(context)
    }

    /// Evaluates an already-parsed expression against an already-built context.
    ///
    /// The error text is assembled exactly as
    /// `helios_fhirpath::evaluate_expression` assembled it, so
    /// `ExtractionError::FhirPathError` messages are unchanged by the split into
    /// parse-once / evaluate-many.
    fn evaluate_prepared(
        context: &EvaluationContext,
        ast: &FhirPathExpression,
        expression: &str,
    ) -> Result<Vec<Value>, ExtractionError> {
        let result = helios_fhirpath::evaluator::evaluate(ast, context, None).map_err(|e| {
            ExtractionError::FhirPathError {
                expression: expression.to_string(),
                message: format!(
                    "Failed to evaluate FHIRPath expression '{}': {}",
                    expression, e
                ),
            }
        })?;

        evaluation_result_to_json_values(&result)
    }

    /// Evaluates a FHIRPath expression against a resource using the
    /// helios-fhirpath evaluator, building a context and parsing the expression
    /// from scratch.
    ///
    /// Test-only: the indexing path goes through [`Self::prepared`] and
    /// [`Self::evaluate_prepared`] so that neither the parse nor the tree
    /// conversion is repeated per resource. The tests below still want to
    /// evaluate a one-off expression against a one-off resource.
    #[cfg(test)]
    fn evaluate_fhirpath(
        &self,
        resource: &Value,
        expression: &str,
    ) -> Result<Vec<Value>, ExtractionError> {
        let context = Self::evaluation_context(resource)?;
        let ast = parse_prepared(expression).map_err(|e| ExtractionError::FhirPathError {
            expression: expression.to_string(),
            message: e,
        })?;
        Self::evaluate_prepared(&context, &ast, expression)
    }

    /// The memoized preparation of `expression` for `resource_type`.
    ///
    /// See the [`PreparedExpression`] note for why this is sound and why it
    /// matters.
    fn prepared(&self, expression: &str, resource_type: &str) -> Arc<PreparedExpression> {
        if let Some(hit) = prepared_cache()
            .read()
            .get(resource_type)
            .and_then(|by_expr| by_expr.get(expression))
        {
            return Arc::clone(hit);
        }

        // Rewrite choice-type casts (`value as Quantity` -> `valueQuantity`) so
        // they resolve against schema-less JSON, then filter to this resource
        // type, then parse.
        let rewritten = rewrite_choice_types(expression);
        let filtered = self.filter_expression_for_resource(&rewritten, resource_type);
        let ast = (!filtered.is_empty()).then(|| parse_prepared(&filtered));
        let target_types = self.resolve_target_types(&rewritten, resource_type);

        let entry = Arc::new(PreparedExpression {
            filtered,
            ast,
            target_types,
        });
        prepared_cache()
            .write()
            .entry(resource_type.to_string())
            .or_default()
            .insert(expression.to_string(), Arc::clone(&entry));
        entry
    }

    /// The memoized preparation of a composite component's sub-expression.
    ///
    /// Component expressions are evaluated relative to a base instance, not to
    /// the resource root, so they are rewritten but never resource-type
    /// filtered — hence a cache keyed by the expression alone.
    fn prepared_component(&self, expression: &str) -> Arc<PreparedComponent> {
        if let Some(hit) = component_cache().read().get(expression) {
            return Arc::clone(hit);
        }

        let rewritten = rewrite_choice_types(expression);
        let ast = parse_prepared(&rewritten);
        let entry = Arc::new(PreparedComponent { rewritten, ast });
        component_cache()
            .write()
            .insert(expression.to_string(), Arc::clone(&entry));
        entry
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

/// The resource type in a `.where(resolve() is X)` clause, if the part has one.
fn resolve_target_type(part: &str) -> Option<String> {
    let start = part.find(".where(resolve() is ")? + ".where(resolve() is ".len();
    let rest = &part[start..];
    let end = rest.find(')')?;
    let ty = rest[..end].trim();
    // Only a plain resource type name is usable; anything else (a union, a
    // profile URL) leaves the parameter unrestricted.
    let plain = !ty.is_empty()
        && ty.starts_with(|c: char| c.is_ascii_uppercase())
        && ty.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| ty.to_string())
}

/// The resource type a reference string points at, when it is stated.
///
/// Handles the relative (`Patient/123`), versioned (`Patient/123/_history/2`)
/// and absolute (`http://host/fhir/Patient/123`) forms. Returns `None` for
/// contained (`#id`), `urn:` and bare-id references, whose type is not stated.
fn reference_target_type(reference: &str) -> Option<&str> {
    if reference.starts_with('#') || reference.starts_with("urn:") {
        return None;
    }
    let base = reference.split("/_history/").next().unwrap_or(reference);
    let mut segments = base.rsplitn(3, '/');
    let _id = segments.next()?;
    let ty = segments.next()?;
    let plain = !ty.is_empty()
        && ty.starts_with(|c: char| c.is_ascii_uppercase())
        && ty.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then_some(ty)
}

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
    fn resolve_target_types_recovers_what_stripping_discards() {
        let extractor = create_test_extractor();

        // Provenance.patient — the parameter this cost the most on. Stripping
        // the clause is required for evaluation, but the restriction it
        // expressed is recoverable and must be re-applied to the values.
        let prov = "Provenance.target.where(resolve() is Patient)";
        assert_eq!(
            extractor.filter_expression_for_resource(prov, "Provenance"),
            "Provenance.target"
        );
        assert_eq!(
            extractor.resolve_target_types(prov, "Provenance"),
            Some(vec!["Patient".to_string()])
        );

        // A member without a resolve clause leaves the parameter unrestricted:
        // filtering on the other member's type would drop real index rows.
        let mixed = "Observation.subject.where(resolve() is Patient) | Observation.performer";
        assert_eq!(extractor.resolve_target_types(mixed, "Observation"), None);

        // No member names this resource type, so the caller falls back to the
        // whole expression and nothing can be concluded.
        assert_eq!(extractor.resolve_target_types(prov, "Patient"), None);
    }

    #[test]
    fn reference_target_type_reads_only_the_stated_forms() {
        assert_eq!(reference_target_type("Patient/123"), Some("Patient"));
        assert_eq!(
            reference_target_type("Patient/123/_history/2"),
            Some("Patient")
        );
        assert_eq!(
            reference_target_type("http://ex.org/fhir/Encounter/9"),
            Some("Encounter")
        );
        // Untyped forms must stay unfiltered — a false positive costs one index
        // row, a false negative costs a search result.
        assert_eq!(reference_target_type("#contained"), None);
        assert_eq!(reference_target_type("urn:uuid:0f2b"), None);
        assert_eq!(reference_target_type("123"), None);
        assert_eq!(reference_target_type("some/lowercase/123"), None);
    }

    #[test]
    fn restrict_reference_targets_keeps_the_undeterminable() {
        let make = |r: &str| {
            ExtractedValue::new(
                "patient",
                "http://example.org/sp",
                SearchParamType::Reference,
                IndexValue::reference(r),
            )
        };
        let mut vals = vec![
            make("Patient/1"),
            make("Encounter/2"),
            make("urn:uuid:3"),
            make("#c"),
        ];
        SearchParameterExtractor::restrict_reference_targets(&mut vals, &["Patient".to_string()]);
        let refs: Vec<String> = vals
            .iter()
            .map(|v| match &v.value {
                IndexValue::Reference { reference, .. } => reference.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(refs, vec!["Patient/1", "urn:uuid:3", "#c"]);
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

/// Local-only profiler for the import path. Not compiled into a release build
/// and a no-op unless `HFS_PROFILE_CORPUS` names a directory of Synthea
/// transaction bundles.
#[cfg(test)]
mod import_profile {
    use super::*;
    use helios_fhir::FhirVersion;
    use helios_fhir::search::SearchParameterLoader;
    use std::path::PathBuf;
    use std::time::Instant;

    fn extractor() -> SearchParameterExtractor {
        let loader = SearchParameterLoader::new(FhirVersion::R4);
        let mut registry = SearchParameterRegistry::new();
        if let Ok(params) = loader.load_embedded() {
            for p in params {
                let _ = registry.register(p);
            }
        }
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        if let Ok(params) = loader.load_from_spec_file(&data_dir) {
            for p in params {
                let _ = registry.register(p);
            }
        }
        SearchParameterExtractor::new(Arc::new(RwLock::new(registry)))
    }

    #[test]
    fn profile_extract_over_corpus() {
        let Ok(dir) = std::env::var("HFS_PROFILE_CORPUS") else {
            return;
        };
        let limit: usize = std::env::var("HFS_PROFILE_BUNDLES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4);

        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .collect();
        files.sort();
        files.truncate(limit);

        let ex = extractor();

        // Collect the resources first so file IO is out of the timings.
        let mut resources: Vec<(String, Value)> = Vec::new();
        for f in &files {
            let b: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            for e in b.get("entry").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                if let Some(r) = e.get("resource") {
                    if let Some(rt) = r.get("resourceType").and_then(|v| v.as_str()) {
                        resources.push((rt.to_string(), r.clone()));
                    }
                }
            }
        }

        // Warm caches (prepared expressions are memoised per (expr, type)).
        for (rt, r) in resources.iter().take(200) {
            let _ = ex.extract(r, rt);
        }

        let t0 = Instant::now();
        let mut rows = 0usize;
        for (rt, r) in &resources {
            rows += ex.extract(r, rt).map(|v| v.len()).unwrap_or(0);
        }
        let total = t0.elapsed();

        // Split: context construction alone.
        let t1 = Instant::now();
        for (_, r) in &resources {
            let _ = SearchParameterExtractor::evaluation_context(r).unwrap();
        }
        let ctx = t1.elapsed();

        let n = resources.len() as f64;
        println!(
            "PROFILE resources={} extracted_values={} total={:?} per_resource={:.1}us \
             ctx_only={:.1}us ({:.0}%) params_loop={:.1}us",
            resources.len(),
            rows,
            total,
            total.as_secs_f64() * 1e6 / n,
            ctx.as_secs_f64() * 1e6 / n,
            100.0 * ctx.as_secs_f64() / total.as_secs_f64(),
            (total.as_secs_f64() - ctx.as_secs_f64()) * 1e6 / n,
        );

        // Per-resource-type breakdown.
        let mut by_type: HashMap<String, (usize, f64)> = HashMap::new();
        for (rt, r) in &resources {
            let t = Instant::now();
            let _ = ex.extract(r, rt);
            let e = by_type.entry(rt.clone()).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += t.elapsed().as_secs_f64();
        }
        let mut v: Vec<_> = by_type.into_iter().collect();
        v.sort_by(|a, b| b.1.1.partial_cmp(&a.1.1).unwrap());
        println!(
            "PROFILE  {:<28} {:>7} {:>10} {:>10}",
            "resourceType", "n", "total_ms", "us/res"
        );
        for (rt, (n, s)) in v.iter().take(15) {
            println!(
                "PROFILE  {:<28} {:>7} {:>10.1} {:>10.1}",
                rt,
                n,
                s * 1e3,
                s * 1e6 / *n as f64
            );
        }
    }
}
