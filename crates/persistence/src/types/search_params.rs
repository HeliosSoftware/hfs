//! FHIR search parameter types.
//!
//! This module defines types for representing FHIR search parameters,
//! including parameter types, modifiers, and prefixes. `SearchParamType`
//! itself was lifted to `helios_fhir::search::SearchParamType` so
//! `helios-sof` can use it without a circular dep; it is re-exported here
//! for backwards-compat with persistence callers.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub use helios_fhir::search::SearchParamType;

/// Search modifiers that can be applied to search parameters.
///
/// See: https://build.fhir.org/search.html#modifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchModifier {
    /// Exact string match (string parameters).
    Exact,
    /// Contains substring (string parameters).
    Contains,
    /// Text search (token parameters).
    Text,
    /// Negation - exclude matches.
    Not,
    /// Match if value is missing.
    Missing,
    /// Match codes above in hierarchy (token parameters).
    Above,
    /// Match codes below in hierarchy (token parameters).
    Below,
    /// Match codes in a value set (token parameters).
    In,
    /// Match codes not in a value set (token parameters).
    NotIn,
    /// Match on identifier (reference parameters).
    Identifier,
    /// Specify reference type (reference parameters).
    Type(String),
    /// Match on type (token parameters for polymorphic elements).
    OfType,
    /// Iterate through results (_include modifier).
    Iterate,
    /// Advanced text search with synonyms and linguistic matching (FHIR v6.0.0).
    ///
    /// This modifier enables sophisticated text matching that may include:
    /// - Synonym expansion
    /// - Linguistic stemming
    /// - Fuzzy matching
    ///
    /// Requires external terminology service integration.
    TextAdvanced,
    /// Match on code text/display value (token parameters, FHIR v6.0.0).
    ///
    /// Searches the text/display value of a CodeableConcept or Coding
    /// rather than the code itself.
    CodeText,
}

impl fmt::Display for SearchModifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchModifier::Exact => write!(f, "exact"),
            SearchModifier::Contains => write!(f, "contains"),
            SearchModifier::Text => write!(f, "text"),
            SearchModifier::Not => write!(f, "not"),
            SearchModifier::Missing => write!(f, "missing"),
            SearchModifier::Above => write!(f, "above"),
            SearchModifier::Below => write!(f, "below"),
            SearchModifier::In => write!(f, "in"),
            SearchModifier::NotIn => write!(f, "not-in"),
            SearchModifier::Identifier => write!(f, "identifier"),
            SearchModifier::Type(t) => write!(f, "{}", t),
            SearchModifier::OfType => write!(f, "ofType"),
            SearchModifier::Iterate => write!(f, "iterate"),
            SearchModifier::TextAdvanced => write!(f, "text-advanced"),
            SearchModifier::CodeText => write!(f, "code-text"),
        }
    }
}

impl SearchModifier {
    /// Parses a modifier string, returning None for unknown modifiers.
    ///
    /// Case-sensitive, as FHIR modifiers are: `exact` is a modifier, `EXACT`
    /// is not (#1339). A capitalised suffix is read as a `:[type]` qualifier;
    /// whether it names a resource type is for the caller to check.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(SearchModifier::Exact),
            "contains" => Some(SearchModifier::Contains),
            "text" => Some(SearchModifier::Text),
            "not" => Some(SearchModifier::Not),
            "missing" => Some(SearchModifier::Missing),
            "above" => Some(SearchModifier::Above),
            "below" => Some(SearchModifier::Below),
            "in" => Some(SearchModifier::In),
            "not-in" => Some(SearchModifier::NotIn),
            "identifier" => Some(SearchModifier::Identifier),
            // The FHIR spec (build.fhir.org) spells this `of-type`, and that is
            // the form advertised in our CapabilityStatement; accept the legacy
            // camelCase `ofType` too so older clients keep working (it is
            // also what `Display` writes).
            "of-type" | "ofType" => Some(SearchModifier::OfType),
            "iterate" => Some(SearchModifier::Iterate),
            "text-advanced" => Some(SearchModifier::TextAdvanced),
            "code-text" => Some(SearchModifier::CodeText),
            _ => {
                // Check if it's a resource type modifier
                if s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    Some(SearchModifier::Type(s.to_string()))
                } else {
                    None
                }
            }
        }
    }

    /// Returns true if this modifier is valid for the given parameter type.
    pub fn is_valid_for(&self, param_type: SearchParamType) -> bool {
        match self {
            SearchModifier::Exact => param_type == SearchParamType::String,
            // Per the FHIR spec, `:contains` is defined for string, reference,
            // and uri parameters (substring/containment match).
            SearchModifier::Contains => matches!(
                param_type,
                SearchParamType::String | SearchParamType::Reference | SearchParamType::Uri
            ),
            // Per the FHIR spec, `:text` is defined for string, token, and
            // reference params (reference matches the indexed `Reference.display`).
            SearchModifier::Text => matches!(
                param_type,
                SearchParamType::String | SearchParamType::Token | SearchParamType::Reference
            ),
            // Per the FHIR spec, `:not` is only defined for token parameters
            // (it negates a code match). Our backends only implement it for
            // token; advertising it for other types was incorrect.
            SearchModifier::Not => param_type == SearchParamType::Token,
            // `:missing` tests whether a single-element search parameter has an
            // indexed value. FHIR excludes composite/combination parameters;
            // special parameters do not have the single-element semantics this
            // modifier requires.
            SearchModifier::Missing => matches!(
                param_type,
                SearchParamType::String
                    | SearchParamType::Token
                    | SearchParamType::Date
                    | SearchParamType::Number
                    | SearchParamType::Quantity
                    | SearchParamType::Reference
                    | SearchParamType::Uri
            ),
            // `:above`/`:below` are defined for token, uri, and reference.
            // Reference uses URL/path-prefix hierarchy (canonical `|version`
            // comparison is not implemented).
            SearchModifier::Above | SearchModifier::Below => matches!(
                param_type,
                SearchParamType::Token | SearchParamType::Uri | SearchParamType::Reference
            ),
            // `:in`/`:not-in` are token-only per the FHIR spec. (`:not-in`
            // itself returns 501 at the REST layer — negated value-set
            // filtering is unimplemented.)
            SearchModifier::In | SearchModifier::NotIn => param_type == SearchParamType::Token,
            SearchModifier::Identifier | SearchModifier::Type(_) => {
                param_type == SearchParamType::Reference
            }
            SearchModifier::OfType => param_type == SearchParamType::Token,
            SearchModifier::Iterate => false, // Only for _include/_revinclude
            // Per the FHIR spec (build.fhir.org), `:text-advanced` is defined for
            // reference and token parameters (NOT string).
            SearchModifier::TextAdvanced => matches!(
                param_type,
                SearchParamType::Token | SearchParamType::Reference
            ),
            // `:code-text` is defined for token and reference params (matches a
            // code's display, or the indexed `Reference.display`).
            SearchModifier::CodeText => matches!(
                param_type,
                SearchParamType::Token | SearchParamType::Reference
            ),
        }
    }
}

/// Comparison prefixes for search parameters.
///
/// See: https://build.fhir.org/search.html#prefix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchPrefix {
    /// Equal (default).
    #[default]
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Less than.
    Lt,
    /// Greater than or equal.
    Ge,
    /// Less than or equal.
    Le,
    /// Starts after.
    Sa,
    /// Ends before.
    Eb,
    /// Approximately equal.
    Ap,
}

impl fmt::Display for SearchPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchPrefix::Eq => write!(f, "eq"),
            SearchPrefix::Ne => write!(f, "ne"),
            SearchPrefix::Gt => write!(f, "gt"),
            SearchPrefix::Lt => write!(f, "lt"),
            SearchPrefix::Ge => write!(f, "ge"),
            SearchPrefix::Le => write!(f, "le"),
            SearchPrefix::Sa => write!(f, "sa"),
            SearchPrefix::Eb => write!(f, "eb"),
            SearchPrefix::Ap => write!(f, "ap"),
        }
    }
}

impl FromStr for SearchPrefix {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "eq" => Ok(SearchPrefix::Eq),
            "ne" => Ok(SearchPrefix::Ne),
            "gt" => Ok(SearchPrefix::Gt),
            "lt" => Ok(SearchPrefix::Lt),
            "ge" => Ok(SearchPrefix::Ge),
            "le" => Ok(SearchPrefix::Le),
            "sa" => Ok(SearchPrefix::Sa),
            "eb" => Ok(SearchPrefix::Eb),
            "ap" => Ok(SearchPrefix::Ap),
            _ => Err(format!("unknown search prefix: {}", s)),
        }
    }
}

impl SearchPrefix {
    /// Extracts a prefix from the beginning of a value string.
    ///
    /// Returns the prefix and the remaining value.
    pub fn extract(value: &str) -> (Self, &str) {
        // `get(..2)` is char-boundary safe: it returns None when the first two
        // bytes don't form a valid prefix (e.g. a multibyte first character like
        // "Müller"), avoiding a panic from slicing mid-codepoint.
        if let Some(prefix) = value.get(..2) {
            if let Ok(p) = prefix.parse() {
                return (p, &value[2..]);
            }
        }
        (SearchPrefix::Eq, value)
    }

    /// Returns true if this prefix is valid for the given parameter type.
    ///
    /// Per the FHIR search specification a comparator prefix exists only on
    /// the ordered types — number, date and quantity. That includes `eq` and
    /// `ne`: on a string, token, uri, reference, composite or special
    /// parameter the two leading characters are part of the value
    /// (`family=nelson`, `code=eq77`), never a prefix (#1307).
    pub fn is_valid_for(&self, param_type: SearchParamType) -> bool {
        match self {
            SearchPrefix::Eq
            | SearchPrefix::Ne
            | SearchPrefix::Gt
            | SearchPrefix::Lt
            | SearchPrefix::Ge
            | SearchPrefix::Le => {
                matches!(
                    param_type,
                    SearchParamType::Number | SearchParamType::Date | SearchParamType::Quantity
                )
            }
            // Per the FHIR spec, `sa`/`eb` (starts-after / ends-before) apply to
            // date and quantity ordered types (not number).
            SearchPrefix::Sa | SearchPrefix::Eb => matches!(
                param_type,
                SearchParamType::Date | SearchParamType::Quantity
            ),
            SearchPrefix::Ap => {
                matches!(
                    param_type,
                    SearchParamType::Number | SearchParamType::Date | SearchParamType::Quantity
                )
            }
        }
    }
}

/// A parsed search parameter with its value.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchParameter {
    /// The parameter name (e.g., "name", "identifier").
    #[serde(default)]
    pub name: String,

    /// The parameter type.
    #[serde(default)]
    pub param_type: SearchParamType,

    /// Modifier, if any.
    ///
    /// For a chained parameter (non-empty [`chain`](Self::chain)) this is the
    /// modifier of the chain's *terminal* parameter — the `:exact` of
    /// `subject:Patient.name:exact` — since that is the only place a chain can
    /// carry one; the `:Type` qualifiers of its reference hops live in
    /// [`ChainedParameter::target_type`].
    #[serde(default)]
    pub modifier: Option<SearchModifier>,

    /// The search value(s). Multiple values are ORed.
    #[serde(default)]
    pub values: Vec<SearchValue>,

    /// Chained parameters (e.g., patient.name=Smith).
    #[serde(default)]
    pub chain: Vec<ChainedParameter>,

    /// Components for composite parameters.
    /// Each component defines the type and expression for extracting component values.
    #[serde(default)]
    pub components: Vec<CompositeSearchComponent>,
}

/// Component definition for a composite search parameter.
///
/// Used when building composite search queries to define how each
/// component of the composite value should be matched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeSearchComponent {
    /// The parameter type of this component (token, quantity, date, etc.).
    pub param_type: SearchParamType,
    /// The parameter name/code for this component.
    pub param_name: String,
}

/// A single search value with optional prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchValue {
    /// The comparison prefix.
    pub prefix: SearchPrefix,

    /// The value to search for.
    pub value: String,
}

impl SearchValue {
    /// Creates a new search value with the given prefix and value.
    pub fn new(prefix: SearchPrefix, value: impl Into<String>) -> Self {
        Self {
            prefix,
            value: value.into(),
        }
    }

    /// Creates a search value with the default (eq) prefix.
    pub fn eq(value: impl Into<String>) -> Self {
        Self::new(SearchPrefix::Eq, value)
    }

    /// Parses a value string, extracting any prefix.
    pub fn parse(s: &str) -> Self {
        let (prefix, value) = SearchPrefix::extract(s);
        Self::new(prefix, value)
    }

    /// Parses a raw value for a parameter of a known type, extracting a
    /// comparator prefix only where that type has one.
    ///
    /// [`SearchValue::parse`] splits any value that starts with two prefix
    /// letters, which is wrong for every type but number, date and quantity:
    /// `family=nelson` is not `ne` + `lson`, and `code=eq77` is not `eq` +
    /// `77`. This keeps such a value whole, and for the ordered types strips
    /// any of the nine prefixes — an explicit `eq` included, so `eq5.4` is the
    /// number `5.4` (#1307).
    ///
    /// This is the one implementation of that rule (#1340):
    /// [`parse_typed_values`](crate::search::parse_typed_values), which types
    /// the values of REST, chain-resolver and conditional-criteria parameters,
    /// calls it, as the SQLite and PostgreSQL `resolve_chain` do directly. The
    /// two used to differ on `sa` / `eb` before a number — stripped there, left
    /// in the value here, where the backend then read `sa10` as not a number —
    /// so a chained `sa10` meant something else than a direct one. Every
    /// backend compares a number under `sa` / `eb` as under `gt` / `lt`, so
    /// they are stripped.
    pub fn parse_for_type(s: &str, param_type: SearchParamType) -> Self {
        if matches!(
            param_type,
            SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
        ) {
            Self::parse(s)
        } else {
            Self::eq(s)
        }
    }

    /// Creates a token search value with optional system and code.
    ///
    /// Format: `[system]|[code]` or just `[code]`
    pub fn token(system: Option<&str>, code: impl Into<String>) -> Self {
        let code = code.into();
        match system {
            Some(sys) => Self::eq(format!("{}|{}", sys, code)),
            None => Self::eq(code),
        }
    }

    /// Creates a token search value with system only (no code).
    ///
    /// Format: `[system]|`
    pub fn token_system_only(system: impl Into<String>) -> Self {
        Self::eq(format!("{}|", system.into()))
    }

    /// Creates a boolean search value.
    pub fn boolean(value: bool) -> Self {
        Self::eq(value.to_string())
    }

    /// Creates a string search value (alias for eq).
    pub fn string(value: impl Into<String>) -> Self {
        Self::eq(value)
    }

    /// Creates a search value for :of-type modifier with three-part format.
    ///
    /// Format: `[type-system]|[type-code]|[value]`
    ///
    /// This is used with the :of-type modifier to search typed identifiers.
    /// The format specifies the identifier type (system and code) followed
    /// by the identifier value to match.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Search for SSN identifier with value "123-45-6789"
    /// SearchValue::of_type(
    ///     "http://terminology.hl7.org/CodeSystem/v2-0203",
    ///     "SS",
    ///     "123-45-6789"
    /// )
    /// ```
    pub fn of_type(
        type_system: impl Into<String>,
        type_code: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self::eq(format!(
            "{}|{}|{}",
            type_system.into(),
            type_code.into(),
            value.into()
        ))
    }
}

/// A chained search parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainedParameter {
    /// The reference parameter being chained through.
    pub reference_param: String,

    /// Optional type modifier on the reference.
    pub target_type: Option<String>,

    /// The target parameter on the referenced resource.
    pub target_param: String,
}

/// A reverse chained parameter (_has).
///
/// Supports nested `_has` queries like:
/// `Patient?_has:Observation:subject:_has:Provenance:target:agent=X`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseChainedParameter {
    /// The resource type that references this resource.
    pub source_type: String,

    /// The reference parameter on the source type.
    pub reference_param: String,

    /// The search parameter on the source type, optionally followed by a
    /// search modifier in FHIR syntax (`code`, `code:not`, `date:missing`) —
    /// see [`terminal_param`](Self::terminal_param).
    /// For nested `_has`, this may be empty or "_has" indicating nesting.
    pub search_param: String,

    /// The search value (None if this is a nested `_has` with further chaining).
    pub value: Option<SearchValue>,

    /// Nested reverse chain for multi-level `_has` queries.
    pub nested: Option<Box<ReverseChainedParameter>>,
}

impl ReverseChainedParameter {
    /// Creates a new terminal (non-nested) reverse chain parameter.
    pub fn terminal(
        source_type: impl Into<String>,
        reference_param: impl Into<String>,
        search_param: impl Into<String>,
        value: SearchValue,
    ) -> Self {
        Self {
            source_type: source_type.into(),
            reference_param: reference_param.into(),
            search_param: search_param.into(),
            value: Some(value),
            nested: None,
        }
    }

    /// Creates a nested reverse chain parameter.
    pub fn nested(
        source_type: impl Into<String>,
        reference_param: impl Into<String>,
        inner: ReverseChainedParameter,
    ) -> Self {
        Self {
            source_type: source_type.into(),
            reference_param: reference_param.into(),
            search_param: String::new(),
            value: None,
            nested: Some(Box::new(inner)),
        }
    }

    /// Splits [`search_param`](Self::search_param) into the parameter name and
    /// its modifier suffix, still unparsed: `code:not` is `("code", Some("not"))`.
    ///
    /// `_has:Observation:subject:code:not=1234-5` puts a modifier on the
    /// terminal parameter exactly as `Observation?code:not=1234-5` does. It is
    /// carried inside `search_param`, as written, rather than in a field of its
    /// own; parse it with [`SearchModifier::parse`].
    pub fn terminal_param(&self) -> (&str, Option<&str>) {
        match self.search_param.split_once(':') {
            Some((name, modifier)) => (name, Some(modifier)),
            None => (&self.search_param, None),
        }
    }

    /// Returns the depth of nesting (1 for non-nested, 2+ for nested).
    pub fn depth(&self) -> usize {
        match &self.nested {
            Some(inner) => 1 + inner.depth(),
            None => 1,
        }
    }

    /// Returns true if this is a terminal (non-nested) reverse chain.
    pub fn is_terminal(&self) -> bool {
        self.nested.is_none()
    }
}

/// Configuration for chain query limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Maximum depth for forward chained parameters.
    /// Default: 4, Maximum: 8
    pub max_forward_depth: usize,

    /// Maximum depth for reverse chained parameters (_has).
    /// Default: 4, Maximum: 8
    pub max_reverse_depth: usize,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            max_forward_depth: 4,
            max_reverse_depth: 4,
        }
    }
}

impl ChainConfig {
    /// Creates a new chain configuration with specified depths.
    pub fn new(max_forward_depth: usize, max_reverse_depth: usize) -> Self {
        Self {
            max_forward_depth: max_forward_depth.min(8),
            max_reverse_depth: max_reverse_depth.min(8),
        }
    }

    /// Validates that forward chain depth is within limits.
    pub fn validate_forward_depth(&self, depth: usize) -> bool {
        depth <= self.max_forward_depth
    }

    /// Validates that reverse chain depth is within limits.
    pub fn validate_reverse_depth(&self, depth: usize) -> bool {
        depth <= self.max_reverse_depth
    }
}

/// Include directive for _include and _revinclude.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncludeDirective {
    /// The type of include.
    pub include_type: IncludeType,

    /// The source resource type.
    pub source_type: String,

    /// The search parameter (reference) to follow.
    pub search_param: String,

    /// Optional target resource type filter.
    pub target_type: Option<String>,

    /// Whether to iterate (follow includes of included resources).
    pub iterate: bool,
}

/// Type of include operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncludeType {
    /// Forward include (_include).
    Include,
    /// Reverse include (_revinclude).
    Revinclude,
}

/// Sort direction for _sort parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortDirection {
    /// Ascending order.
    #[default]
    Ascending,
    /// Descending order.
    Descending,
}

/// A sort directive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortDirective {
    /// The parameter to sort by.
    pub parameter: String,
    /// The sort direction.
    pub direction: SortDirection,
    /// The resolved search-parameter type, when the sort is on an indexed
    /// search parameter (rather than `_id` / `_lastUpdated`). Lets backends pick
    /// the correct `search_index` value column. `None` for `_id`/`_lastUpdated`
    /// or unresolved parameters.
    #[serde(default)]
    pub param_type: Option<SearchParamType>,
}

impl SortDirective {
    /// Parses a sort parameter value (e.g., "-date" for descending).
    pub fn parse(s: &str) -> Self {
        if let Some(stripped) = s.strip_prefix('-') {
            Self {
                parameter: stripped.to_string(),
                direction: SortDirection::Descending,
                param_type: None,
            }
        } else {
            Self {
                parameter: s.to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            }
        }
    }

    /// Sets the resolved search-parameter type for this sort directive.
    pub fn with_param_type(mut self, param_type: Option<SearchParamType>) -> Self {
        self.param_type = param_type;
        self
    }
}

/// A complete search query with all parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The resource type being searched.
    pub resource_type: String,

    /// Standard search parameters.
    pub parameters: Vec<SearchParameter>,

    /// Reverse chain parameters (_has).
    pub reverse_chains: Vec<ReverseChainedParameter>,

    /// `_list` filters: logical ids of `List` resources whose `entry.item`
    /// references restrict the result set. Multiple values are AND-ed (a result
    /// must be a member of every listed `List`). Resolved application-side into
    /// an `_id` filter by [`crate::search::list_resolver`], so any backend can
    /// execute the rewritten query.
    pub list: Vec<String>,

    /// `_contained` mode: whether the search matches against resources nested in
    /// other resources' `contained[]` arrays.
    pub contained: ContainedMode,

    /// `_containedType`: when a contained resource matches, whether to return the
    /// container resource (default) or the contained resource itself.
    pub contained_return: ContainedReturn,

    /// Include directives.
    pub includes: Vec<IncludeDirective>,

    /// Sort directives.
    pub sort: Vec<SortDirective>,

    /// Result count limit (_count).
    pub count: Option<u32>,

    /// Offset for pagination.
    pub offset: Option<u32>,

    /// Cursor for keyset pagination.
    pub cursor: Option<String>,

    /// Whether to include total count (_total).
    pub total: Option<TotalMode>,

    /// Summary mode (_summary).
    pub summary: Option<SummaryMode>,

    /// Elements to include (_elements).
    pub elements: Vec<String>,

    /// Compartment membership filter, when this is a compartment search
    /// (`GET /{compartmentType}/{id}/{targetType}`). Restricts results to
    /// resources that reference the compartment via *any* of the membership
    /// params (OR), per the FHIR CompartmentDefinition.
    pub compartment: Option<CompartmentMembership>,

    /// Raw query parameters for debugging.
    pub raw_params: HashMap<String, Vec<String>>,
}

/// Compartment membership constraint for a compartment search.
///
/// A resource is in the compartment if it references `reference` through *any*
/// of the `params` reference search parameters (logical OR). See
/// https://hl7.org/fhir/compartmentdefinition.html.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CompartmentMembership {
    /// The membership reference search parameters (e.g. `patient`, `recorder`,
    /// `asserter`). Matching ANY of these satisfies membership.
    pub params: Vec<String>,
    /// The compartment reference value, e.g. `Patient/123`.
    pub reference: String,
}

/// Mode for the `_contained` parameter — whether contained resources (nested in
/// another resource's `contained[]`) participate in matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContainedMode {
    /// `_contained=false` (default): match only top-level resources.
    #[default]
    Off,
    /// `_contained=true`: match contained resources only.
    On,
    /// `_contained=both`: match both top-level and contained resources.
    Both,
}

/// Mode for the `_containedType` parameter — what to return when a contained
/// resource matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContainedReturn {
    /// `_containedType=container` (default): return the container resource.
    #[default]
    Container,
    /// `_containedType=contained`: return the contained resource itself.
    Contained,
}

/// Mode for _total parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TotalMode {
    /// No total.
    None,
    /// Estimated total.
    Estimate,
    /// Accurate total.
    Accurate,
}

/// Mode for _summary parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryMode {
    /// Return summary elements only.
    True,
    /// Return full resource.
    False,
    /// Return text narrative only.
    Text,
    /// Return data elements only (no text).
    Data,
    /// Return count only.
    Count,
}

/// Strips a `/_history/<vid>` version suffix from a FHIR reference, returning
/// the version-agnostic base. References without a version are returned
/// unchanged. Used so reference search matches regardless of version, per the
/// FHIR spec.
pub fn strip_reference_version(reference: &str) -> &str {
    match reference.find("/_history/") {
        Some(i) => &reference[..i],
        None => reference,
    }
}

impl SearchQuery {
    /// Creates a new search query for the given resource type.
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            ..Default::default()
        }
    }

    /// Returns true if the client requested a total count via
    /// `_total=accurate` or `_total=estimate`.
    ///
    /// `_total=none` or an unspecified `_total` returns `false`, so backends
    /// skip the extra count query (FHIR allows omitting `Bundle.total`).
    pub fn wants_total(&self) -> bool {
        matches!(self.total, Some(TotalMode::Estimate | TotalMode::Accurate))
    }

    /// Adds a search parameter.
    pub fn with_parameter(mut self, param: SearchParameter) -> Self {
        self.parameters.push(param);
        self
    }

    /// Adds an include directive.
    pub fn with_include(mut self, include: IncludeDirective) -> Self {
        self.includes.push(include);
        self
    }

    /// Adds a sort directive.
    pub fn with_sort(mut self, sort: SortDirective) -> Self {
        self.sort.push(sort);
        self
    }

    /// Sets the count limit.
    pub fn with_count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    /// Sets the cursor for keyset pagination.
    pub fn with_cursor(mut self, cursor: String) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Returns true if this query uses any features that require special backend support.
    pub fn requires_advanced_features(&self) -> bool {
        // Chained parameters
        if self.parameters.iter().any(|p| !p.chain.is_empty()) {
            return true;
        }

        // Reverse chains
        if !self.reverse_chains.is_empty() {
            return true;
        }

        // Includes
        if !self.includes.is_empty() {
            return true;
        }

        // Terminology modifiers
        if self.parameters.iter().any(|p| {
            matches!(
                p.modifier,
                Some(SearchModifier::Above)
                    | Some(SearchModifier::Below)
                    | Some(SearchModifier::In)
                    | Some(SearchModifier::NotIn)
            )
        }) {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_param_type_display() {
        assert_eq!(SearchParamType::String.to_string(), "string");
        assert_eq!(SearchParamType::Token.to_string(), "token");
        assert_eq!(SearchParamType::Reference.to_string(), "reference");
    }

    #[test]
    fn test_search_param_type_parse() {
        assert_eq!(
            "string".parse::<SearchParamType>().unwrap(),
            SearchParamType::String
        );
        assert_eq!(
            "TOKEN".parse::<SearchParamType>().unwrap(),
            SearchParamType::Token
        );
    }

    #[test]
    fn test_search_modifier_parse() {
        assert_eq!(SearchModifier::parse("exact"), Some(SearchModifier::Exact));
        assert_eq!(
            SearchModifier::parse("contains"),
            Some(SearchModifier::Contains)
        );
        assert_eq!(
            SearchModifier::parse("Patient"),
            Some(SearchModifier::Type("Patient".to_string()))
        );
        // Both the spec/CapabilityStatement spelling (`of-type`) and the legacy
        // camelCase (`ofType`) must parse to the same modifier.
        assert_eq!(
            SearchModifier::parse("of-type"),
            Some(SearchModifier::OfType)
        );
        assert_eq!(
            SearchModifier::parse("ofType"),
            Some(SearchModifier::OfType)
        );
        assert_eq!(SearchModifier::parse("unknown"), None);
    }

    /// #1339: FHIR modifiers are case-sensitive; a differently-cased spelling
    /// is not the modifier.
    #[test]
    fn test_search_modifier_parse_is_case_sensitive() {
        // A capitalised suffix reads as a `:[type]` qualifier, which is the
        // caller's to check against the resource types; the rest are unknown.
        for s in ["EXACT", "Exact", "Missing", "NOT-IN", "Of-Type", "OfType"] {
            assert_eq!(
                SearchModifier::parse(s),
                Some(SearchModifier::Type(s.to_string())),
                "{s}"
            );
        }
        for s in ["eXact", "oftype", "not-In", "patient"] {
            assert_eq!(SearchModifier::parse(s), None, "{s}");
        }
        // Every modifier parses back from the spelling it displays as.
        for m in [
            SearchModifier::Exact,
            SearchModifier::Contains,
            SearchModifier::Text,
            SearchModifier::Not,
            SearchModifier::Missing,
            SearchModifier::Above,
            SearchModifier::Below,
            SearchModifier::In,
            SearchModifier::NotIn,
            SearchModifier::Identifier,
            SearchModifier::OfType,
            SearchModifier::Iterate,
            SearchModifier::TextAdvanced,
            SearchModifier::CodeText,
        ] {
            assert_eq!(SearchModifier::parse(&m.to_string()), Some(m));
        }
    }

    #[test]
    fn test_search_modifier_validity() {
        assert!(SearchModifier::Exact.is_valid_for(SearchParamType::String));
        assert!(!SearchModifier::Exact.is_valid_for(SearchParamType::Token));
        // `:contains` is valid for string, reference, and uri (FHIR spec).
        assert!(SearchModifier::Contains.is_valid_for(SearchParamType::String));
        assert!(SearchModifier::Contains.is_valid_for(SearchParamType::Reference));
        assert!(SearchModifier::Contains.is_valid_for(SearchParamType::Uri));
        assert!(!SearchModifier::Contains.is_valid_for(SearchParamType::Token));
        assert!(SearchModifier::Text.is_valid_for(SearchParamType::Token));
        // `:text` is valid for string and token (FHIR spec).
        assert!(SearchModifier::Text.is_valid_for(SearchParamType::String));
        assert!(!SearchModifier::Text.is_valid_for(SearchParamType::Uri));
        // `:not` is token-only per the FHIR spec.
        assert!(SearchModifier::Not.is_valid_for(SearchParamType::Token));
        assert!(!SearchModifier::Not.is_valid_for(SearchParamType::String));
        // `:missing` is valid for ordinary single-element parameter types, but
        // not for composite/combination or special parameters.
        for param_type in [
            SearchParamType::String,
            SearchParamType::Token,
            SearchParamType::Date,
            SearchParamType::Number,
            SearchParamType::Quantity,
            SearchParamType::Reference,
            SearchParamType::Uri,
        ] {
            assert!(SearchModifier::Missing.is_valid_for(param_type));
        }
        assert!(!SearchModifier::Missing.is_valid_for(SearchParamType::Composite));
        assert!(!SearchModifier::Missing.is_valid_for(SearchParamType::Special));
        // `:in`/`:not-in` are token-only per the FHIR spec (not uri).
        assert!(SearchModifier::In.is_valid_for(SearchParamType::Token));
        assert!(!SearchModifier::In.is_valid_for(SearchParamType::Uri));
        assert!(SearchModifier::NotIn.is_valid_for(SearchParamType::Token));
        assert!(!SearchModifier::NotIn.is_valid_for(SearchParamType::Uri));
        // `:above`/`:below` are valid for token, uri, and reference.
        assert!(SearchModifier::Above.is_valid_for(SearchParamType::Uri));
        assert!(SearchModifier::Below.is_valid_for(SearchParamType::Token));
        assert!(SearchModifier::Above.is_valid_for(SearchParamType::Reference));
        assert!(SearchModifier::Below.is_valid_for(SearchParamType::Reference));
        assert!(!SearchModifier::Above.is_valid_for(SearchParamType::String));
    }

    #[test]
    fn test_search_prefix_extract() {
        assert_eq!(
            SearchPrefix::extract("gt2020-01-01"),
            (SearchPrefix::Gt, "2020-01-01")
        );
        assert_eq!(
            SearchPrefix::extract("2020-01-01"),
            (SearchPrefix::Eq, "2020-01-01")
        );
        assert_eq!(SearchPrefix::extract("le100"), (SearchPrefix::Le, "100"));
    }

    #[test]
    fn test_search_prefix_validity() {
        assert!(SearchPrefix::Gt.is_valid_for(SearchParamType::Number));
        assert!(SearchPrefix::Gt.is_valid_for(SearchParamType::Date));
        assert!(!SearchPrefix::Gt.is_valid_for(SearchParamType::String));
        assert!(SearchPrefix::Sa.is_valid_for(SearchParamType::Date));
        // `sa`/`eb` apply to date and quantity, but not number, per the spec.
        assert!(SearchPrefix::Sa.is_valid_for(SearchParamType::Quantity));
        assert!(SearchPrefix::Eb.is_valid_for(SearchParamType::Quantity));
        assert!(!SearchPrefix::Sa.is_valid_for(SearchParamType::Number));
    }

    /// Every prefix against every parameter type (#1307). `eq` and `ne` used
    /// to be reported valid for all nine types.
    #[test]
    fn test_search_prefix_validity_exhaustive() {
        use SearchParamType as T;
        use SearchPrefix as P;

        const TYPES: [T; 9] = [
            T::String,
            T::Uri,
            T::Number,
            T::Date,
            T::Quantity,
            T::Token,
            T::Reference,
            T::Composite,
            T::Special,
        ];
        // Compile-time guard: a new `SearchParamType` variant must be added
        // to `TYPES` above and to the table below.
        let _exhaustive = |t: T| match t {
            T::String
            | T::Uri
            | T::Number
            | T::Date
            | T::Quantity
            | T::Token
            | T::Reference
            | T::Composite
            | T::Special => (),
        };

        // (prefix, the types it is valid for)
        let table: [(P, &[T]); 9] = [
            (P::Eq, &[T::Number, T::Date, T::Quantity]),
            (P::Ne, &[T::Number, T::Date, T::Quantity]),
            (P::Gt, &[T::Number, T::Date, T::Quantity]),
            (P::Lt, &[T::Number, T::Date, T::Quantity]),
            (P::Ge, &[T::Number, T::Date, T::Quantity]),
            (P::Le, &[T::Number, T::Date, T::Quantity]),
            (P::Sa, &[T::Date, T::Quantity]),
            (P::Eb, &[T::Date, T::Quantity]),
            (P::Ap, &[T::Number, T::Date, T::Quantity]),
        ];
        for (prefix, valid) in table {
            for param_type in TYPES {
                assert_eq!(
                    prefix.is_valid_for(param_type),
                    valid.contains(&param_type),
                    "{prefix} on {param_type}"
                );
            }
        }
    }

    #[test]
    fn test_search_value_parse_for_type() {
        use SearchParamType as T;

        let parsed = |s: &str, t: T| {
            let v = SearchValue::parse_for_type(s, t);
            (v.prefix, v.value)
        };
        let whole = |s: &str| (SearchPrefix::Eq, s.to_string());

        // Non-ordered types never lose their first two characters, whatever
        // their case.
        for t in [
            T::String,
            T::Token,
            T::Uri,
            T::Reference,
            T::Composite,
            T::Special,
        ] {
            for s in [
                "nelson", "Nelson", "NE123", "eq77", "Equus", "Levine", "gtin", "ltd", "geo",
                "sam", "ebony", "apple", "Smith", "", "n", "Müller",
            ] {
                assert_eq!(parsed(s, t), whole(s), "{s} as {t}");
            }
        }

        // Ordered types strip every prefix they admit, explicit `eq` included.
        for t in [T::Number, T::Date, T::Quantity] {
            assert_eq!(parsed("5.4", t), whole("5.4"), "{t}");
            assert_eq!(parsed("eq5.4", t), whole("5.4"), "{t}");
            assert_eq!(parsed("EQ5.4", t), whole("5.4"), "{t}");
            for (s, prefix) in [
                ("ne5.4", SearchPrefix::Ne),
                ("gt5.4", SearchPrefix::Gt),
                ("lt5.4", SearchPrefix::Lt),
                ("ge5.4", SearchPrefix::Ge),
                ("le5.4", SearchPrefix::Le),
                ("ap5.4", SearchPrefix::Ap),
            ] {
                assert_eq!(parsed(s, t), (prefix, "5.4".to_string()), "{s} as {t}");
            }
        }
        assert_eq!(
            parsed("sa2020-01-01", T::Date),
            (SearchPrefix::Sa, "2020-01-01".to_string())
        );
        assert_eq!(
            parsed("eb5.4", T::Quantity),
            (SearchPrefix::Eb, "5.4".to_string())
        );
        // As a direct search always has, `sa` / `eb` before a number too:
        // every backend compares them as `gt` / `lt`.
        assert_eq!(
            parsed("sa10", T::Number),
            (SearchPrefix::Sa, "10".to_string())
        );
        assert_eq!(
            parsed("eb10", T::Number),
            (SearchPrefix::Eb, "10".to_string())
        );
    }

    #[test]
    fn test_search_value_parse() {
        let value = SearchValue::parse("gt100");
        assert_eq!(value.prefix, SearchPrefix::Gt);
        assert_eq!(value.value, "100");

        let value2 = SearchValue::parse("Smith");
        assert_eq!(value2.prefix, SearchPrefix::Eq);
        assert_eq!(value2.value, "Smith");
    }

    #[test]
    fn test_sort_directive_parse() {
        let asc = SortDirective::parse("date");
        assert_eq!(asc.parameter, "date");
        assert_eq!(asc.direction, SortDirection::Ascending);

        let desc = SortDirective::parse("-date");
        assert_eq!(desc.parameter, "date");
        assert_eq!(desc.direction, SortDirection::Descending);
    }

    #[test]
    fn test_search_query_builder() {
        let query = SearchQuery::new("Patient")
            .with_count(10)
            .with_sort(SortDirective::parse("-_lastUpdated"));

        assert_eq!(query.resource_type, "Patient");
        assert_eq!(query.count, Some(10));
        assert_eq!(query.sort.len(), 1);
    }

    #[test]
    fn test_requires_advanced_features() {
        let simple = SearchQuery::new("Patient");
        assert!(!simple.requires_advanced_features());

        let with_include = SearchQuery::new("Patient").with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: None,
            iterate: false,
        });
        assert!(with_include.requires_advanced_features());
    }
}
