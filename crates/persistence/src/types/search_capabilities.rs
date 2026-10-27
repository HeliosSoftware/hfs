//! Enhanced search capability types for FHIR search.
//!
//! This module provides comprehensive capability enums and structs for declaring
//! what FHIR search features a backend supports. These types enable:
//!
//! - Generating accurate CapabilityStatements
//! - Validating search queries before execution
//! - Capability-based query routing in polyglot deployments

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::SearchParamType;
use crate::search::date_value::{DateValuePrecision, FhirDateValue};

/// Special search parameters that apply across resource types.
///
/// These are the FHIR special parameters that have consistent behavior
/// regardless of resource type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecialSearchParam {
    /// `_id` - Resource id (without base URL)
    Id,
    /// `_lastUpdated` - When the resource was last changed
    LastUpdated,
    /// `_tag` - Tags applied to this resource
    Tag,
    /// `_profile` - Profiles the resource claims to conform to
    Profile,
    /// `_security` - Security labels applied to this resource
    Security,
    /// `_text` - Search on narrative content
    Text,
    /// `_content` - Search on entire resource content
    Content,
    /// `_list` - Search for resources in a List
    List,
    /// `_has` - Reverse chained search
    Has,
    /// `_type` - Resource type filter (for multi-type searches)
    Type,
    /// `_query` - Custom named query
    Query,
    /// `_filter` - Filter expression
    Filter,
    /// `_source` - Source of resource (meta.source)
    Source,
}

impl SpecialSearchParam {
    /// Returns the parameter name as used in FHIR URLs.
    pub fn name(&self) -> &'static str {
        match self {
            SpecialSearchParam::Id => "_id",
            SpecialSearchParam::LastUpdated => "_lastUpdated",
            SpecialSearchParam::Tag => "_tag",
            SpecialSearchParam::Profile => "_profile",
            SpecialSearchParam::Security => "_security",
            SpecialSearchParam::Text => "_text",
            SpecialSearchParam::Content => "_content",
            SpecialSearchParam::List => "_list",
            SpecialSearchParam::Has => "_has",
            SpecialSearchParam::Type => "_type",
            SpecialSearchParam::Query => "_query",
            SpecialSearchParam::Filter => "_filter",
            SpecialSearchParam::Source => "_source",
        }
    }

    /// Returns the parameter type for this special parameter.
    pub fn param_type(&self) -> SearchParamType {
        match self {
            SpecialSearchParam::Id => SearchParamType::Token,
            SpecialSearchParam::LastUpdated => SearchParamType::Date,
            SpecialSearchParam::Tag => SearchParamType::Token,
            SpecialSearchParam::Profile => SearchParamType::Uri,
            SpecialSearchParam::Security => SearchParamType::Token,
            SpecialSearchParam::Text => SearchParamType::Special,
            SpecialSearchParam::Content => SearchParamType::Special,
            SpecialSearchParam::List => SearchParamType::Reference,
            SpecialSearchParam::Has => SearchParamType::Special,
            SpecialSearchParam::Type => SearchParamType::Token,
            SpecialSearchParam::Query => SearchParamType::Special,
            SpecialSearchParam::Filter => SearchParamType::Special,
            SpecialSearchParam::Source => SearchParamType::Uri,
        }
    }

    /// All defined special parameters.
    pub fn all() -> &'static [SpecialSearchParam] {
        &[
            SpecialSearchParam::Id,
            SpecialSearchParam::LastUpdated,
            SpecialSearchParam::Tag,
            SpecialSearchParam::Profile,
            SpecialSearchParam::Security,
            SpecialSearchParam::Text,
            SpecialSearchParam::Content,
            SpecialSearchParam::List,
            SpecialSearchParam::Has,
            SpecialSearchParam::Type,
            SpecialSearchParam::Query,
            SpecialSearchParam::Filter,
            SpecialSearchParam::Source,
        ]
    }
}

impl std::fmt::Display for SpecialSearchParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Include/revinclude capability variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IncludeCapability {
    /// `_include` - Forward include of referenced resources
    Include,
    /// `_revinclude` - Reverse include of resources that reference results
    Revinclude,
    /// `_include:iterate` - Recursive includes
    IncludeIterate,
    /// `_revinclude:iterate` - Recursive reverse includes
    RevincludeIterate,
    /// `_include=*` - Wildcard includes (all references)
    IncludeWildcard,
    /// `_revinclude=*` - Wildcard reverse includes
    RevincludeWildcard,
}

impl IncludeCapability {
    /// Returns the modifier suffix for this capability.
    pub fn modifier(&self) -> Option<&'static str> {
        match self {
            IncludeCapability::Include => None,
            IncludeCapability::Revinclude => None,
            IncludeCapability::IncludeIterate => Some("iterate"),
            IncludeCapability::RevincludeIterate => Some("iterate"),
            IncludeCapability::IncludeWildcard => None,
            IncludeCapability::RevincludeWildcard => None,
        }
    }

    /// Returns whether this is an include (true) or revinclude (false).
    pub fn is_include(&self) -> bool {
        matches!(
            self,
            IncludeCapability::Include
                | IncludeCapability::IncludeIterate
                | IncludeCapability::IncludeWildcard
        )
    }
}

/// Chaining capability variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChainingCapability {
    /// Forward chaining (e.g., `Observation?patient.name=Smith`)
    ForwardChain,
    /// Reverse chaining via `_has` (e.g., `Patient?_has:Observation:patient:code=xyz`)
    ReverseChain,
    /// Maximum chain depth supported
    MaxDepth(u8),
}

impl ChainingCapability {
    /// Returns the maximum depth for MaxDepth variant.
    pub fn max_depth(&self) -> Option<u8> {
        match self {
            ChainingCapability::MaxDepth(d) => Some(*d),
            _ => None,
        }
    }
}

/// Pagination capability variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaginationCapability {
    /// `_count` parameter support
    Count,
    /// Offset-based pagination (`_offset`)
    Offset,
    /// Cursor-based pagination (opaque page tokens)
    Cursor,
    /// Maximum page size supported
    MaxPageSize(u32),
    /// Default page size when not specified
    DefaultPageSize(u32),
}

impl PaginationCapability {
    /// Returns the page size value for MaxPageSize or DefaultPageSize variants.
    pub fn page_size(&self) -> Option<u32> {
        match self {
            PaginationCapability::MaxPageSize(s) | PaginationCapability::DefaultPageSize(s) => {
                Some(*s)
            }
            _ => None,
        }
    }
}

/// Result mode capability variants.
///
/// These control what data is returned in search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultModeCapability {
    /// `_summary` parameter support (any mode)
    Summary,
    /// `_summary=true` - Summary elements only
    SummaryTrue,
    /// `_summary=text` - Text narrative only
    SummaryText,
    /// `_summary=data` - Data elements only (no text)
    SummaryData,
    /// `_summary=count` - Count only, no resources
    SummaryCount,
    /// `_summary=false` - Full resource (default)
    SummaryFalse,
    /// `_elements` parameter support
    Elements,
    /// `_total` parameter support (any mode)
    Total,
    /// `_total=none` - No total count
    TotalNone,
    /// `_total=estimate` - Estimated total count
    TotalEstimate,
    /// `_total=accurate` - Accurate total count
    TotalAccurate,
    /// `_contained` parameter support
    Contained,
    /// `_containedType` parameter support
    ContainedType,
}

/// Component of a composite search parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompositeComponent {
    /// Definition URL of the component parameter.
    pub definition: String,
    /// FHIRPath expression for extracting this component.
    pub expression: String,
}

impl CompositeComponent {
    /// Creates a new composite component.
    pub fn new(definition: impl Into<String>, expression: impl Into<String>) -> Self {
        Self {
            definition: definition.into(),
            expression: expression.into(),
        }
    }
}

/// Full capability declaration for a search parameter.
///
/// This provides complete information about what features are supported
/// for a specific search parameter on a specific resource type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParamFullCapability {
    /// The parameter name (e.g., "name", "identifier").
    pub name: String,

    /// The parameter type.
    pub param_type: SearchParamType,

    /// Canonical URL of the SearchParameter definition.
    pub definition: Option<String>,

    /// Supported modifiers for this parameter.
    pub modifiers: HashSet<String>,

    /// Supported comparison prefixes for this parameter.
    pub prefixes: HashSet<String>,

    /// Chaining capability for reference parameters.
    pub chaining: Option<ChainingCapability>,

    /// Target resource types (for reference parameters).
    pub target_types: Vec<String>,

    /// Components (for composite parameters).
    pub components: Vec<CompositeComponent>,

    /// Whether this parameter is required in capability statements.
    pub shall_support: bool,
}

impl SearchParamFullCapability {
    /// Creates a new capability for a search parameter.
    pub fn new(name: impl Into<String>, param_type: SearchParamType) -> Self {
        Self {
            name: name.into(),
            param_type,
            definition: None,
            modifiers: HashSet::new(),
            prefixes: Self::default_prefixes(param_type),
            chaining: None,
            target_types: Vec::new(),
            components: Vec::new(),
            shall_support: false,
        }
    }

    /// Returns default prefixes for a parameter type.
    fn default_prefixes(param_type: SearchParamType) -> HashSet<String> {
        let mut prefixes = HashSet::new();
        prefixes.insert("eq".to_string());

        match param_type {
            SearchParamType::Number | SearchParamType::Date | SearchParamType::Quantity => {
                prefixes.insert("ne".to_string());
                prefixes.insert("gt".to_string());
                prefixes.insert("lt".to_string());
                prefixes.insert("ge".to_string());
                prefixes.insert("le".to_string());
            }
            _ => {}
        }

        if param_type == SearchParamType::Date {
            prefixes.insert("sa".to_string());
            prefixes.insert("eb".to_string());
            prefixes.insert("ap".to_string());
        }

        prefixes
    }

    /// Sets the definition URL.
    pub fn with_definition(mut self, url: impl Into<String>) -> Self {
        self.definition = Some(url.into());
        self
    }

    /// Adds supported modifiers.
    pub fn with_modifiers<I, S>(mut self, modifiers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.modifiers = modifiers.into_iter().map(Into::into).collect();
        self
    }

    /// Sets chaining capability.
    pub fn with_chaining(mut self, chaining: ChainingCapability) -> Self {
        self.chaining = Some(chaining);
        self
    }

    /// Sets target types for reference parameters.
    pub fn with_targets<I, S>(mut self, targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.target_types = targets.into_iter().map(Into::into).collect();
        self
    }

    /// Sets components for composite parameters.
    pub fn with_components(mut self, components: Vec<CompositeComponent>) -> Self {
        self.components = components;
        self
    }

    /// Marks this parameter as SHALL support.
    pub fn shall(mut self) -> Self {
        self.shall_support = true;
        self
    }

    /// Returns whether a specific modifier is supported.
    pub fn supports_modifier(&self, modifier: &str) -> bool {
        self.modifiers.contains(modifier)
    }

    /// Returns whether a specific prefix is supported.
    pub fn supports_prefix(&self, prefix: &str) -> bool {
        self.prefixes.contains(prefix)
    }

    /// Returns whether this is a composite parameter.
    pub fn is_composite(&self) -> bool {
        self.param_type == SearchParamType::Composite && !self.components.is_empty()
    }

    /// Returns whether this is a reference parameter with chaining support.
    pub fn supports_chaining(&self) -> bool {
        self.chaining.is_some()
    }
}

/// Date precision for search parameter values.
///
/// Used to track the precision of date values for proper range matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatePrecision {
    /// Year only (e.g., "2024")
    Year,
    /// Year and month (e.g., "2024-01")
    Month,
    /// Full date (e.g., "2024-01-15")
    Day,
    /// Date and time to hours (e.g., "2024-01-15T10")
    Hour,
    /// Date and time to minutes (e.g., "2024-01-15T10:30")
    Minute,
    /// Date and time to seconds (e.g., "2024-01-15T10:30:00")
    Second,
    /// Full precision with milliseconds
    Millisecond,
}

impl DatePrecision {
    /// The precision an ISO date string was written at.
    ///
    /// Read from the parsed value, through the grammar every date search
    /// value goes through ([`FhirDateValue`]), so the zone — `Z`, an offset of
    /// either sign, or none — and the number of fraction digits cannot change
    /// the answer. It used to be inferred from the string's length after
    /// cutting at `+` or `Z`, which left a `-hh:mm` offset in place and read
    /// `2016-01-23T17:07:42-04:00` as millisecond precision (#1316).
    ///
    /// This runs on the index path, so it never fails: a stored value the
    /// grammar rejects (an hour without minutes, `2024-02-30`, a lower-case
    /// `t`) is classified by its shape instead.
    pub fn from_date_string(s: &str) -> Self {
        match FhirDateValue::parse(s) {
            Ok(parsed) => parsed.precision.into(),
            Err(_) => Self::from_shape(s.trim()),
        }
    }

    /// Best-effort precision of a value that is not a valid FHIR date, by how
    /// many of its components are present.
    fn from_shape(s: &str) -> Self {
        let (date, time) = s.split_once(['T', 't']).unwrap_or((s, ""));
        if time.is_empty() {
            return match date.len() {
                0..=4 => DatePrecision::Year,
                5..=7 => DatePrecision::Month,
                _ => DatePrecision::Day,
            };
        }
        // Whatever follows the zone sign belongs to the zone, and no sign can
        // appear inside the time itself. A space there is a form-decoded `+`.
        let time = time.split(['Z', 'z', '+', '-', ' ']).next().unwrap_or(time);
        if time.contains(['.', ',']) {
            return DatePrecision::Millisecond;
        }
        match time.matches(':').count() {
            0 => DatePrecision::Hour,
            1 => DatePrecision::Minute,
            _ => DatePrecision::Second,
        }
    }

    /// Returns the SQL date format for this precision.
    pub fn sql_format(&self) -> &'static str {
        match self {
            DatePrecision::Year => "%Y",
            DatePrecision::Month => "%Y-%m",
            DatePrecision::Day => "%Y-%m-%d",
            DatePrecision::Hour => "%Y-%m-%dT%H",
            DatePrecision::Minute => "%Y-%m-%dT%H:%M",
            DatePrecision::Second => "%Y-%m-%dT%H:%M:%S",
            DatePrecision::Millisecond => "%Y-%m-%dT%H:%M:%S%.f",
        }
    }
}

impl std::fmt::Display for DatePrecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatePrecision::Year => write!(f, "year"),
            DatePrecision::Month => write!(f, "month"),
            DatePrecision::Day => write!(f, "day"),
            DatePrecision::Hour => write!(f, "hour"),
            DatePrecision::Minute => write!(f, "minute"),
            DatePrecision::Second => write!(f, "second"),
            DatePrecision::Millisecond => write!(f, "millisecond"),
        }
    }
}

impl From<DateValuePrecision> for DatePrecision {
    /// A fraction of any length is [`DatePrecision::Millisecond`], the finest
    /// precision tracked here. The shared grammar has no hour-only form, so
    /// [`DatePrecision::Hour`] never comes from a parsed value.
    fn from(precision: DateValuePrecision) -> Self {
        match precision {
            DateValuePrecision::Year => DatePrecision::Year,
            DateValuePrecision::Month => DatePrecision::Month,
            DateValuePrecision::Day => DatePrecision::Day,
            DateValuePrecision::Minute => DatePrecision::Minute,
            DateValuePrecision::Second => DatePrecision::Second,
            DateValuePrecision::Fraction(_) => DatePrecision::Millisecond,
        }
    }
}

/// Search strategy - determines HOW searches are executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SearchStrategy {
    /// Pre-computed indexes.
    /// - Values extracted at write time, stored in search_index table.
    /// - Fast queries, slower writes.
    /// - Requires reindexing for new SearchParameters.
    #[default]
    PrecomputedIndex,

    /// Query-time JSONB/JSON evaluation.
    /// - FHIRPath evaluated at query time against stored JSON.
    /// - Immediate SearchParameter activation, no reindexing needed.
    /// - Slower queries for complex expressions.
    QueryTimeEvaluation,

    /// Hybrid: use indexes where available, fallback to JSONB.
    /// - Best of both worlds.
    /// - Pre-computed for common params, JSONB for custom/new params.
    Hybrid {
        /// Parameter codes that have pre-computed indexes.
        indexed_params: Vec<String>,
    },
}

/// Indexing mode - determines WHEN indexes are updated (for PrecomputedIndex strategy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum IndexingMode {
    /// Synchronous indexing during create/update (default).
    /// - Resources are searchable immediately.
    /// - Slightly slower write operations.
    #[default]
    Inline,

    /// Asynchronous indexing via event stream (future).
    /// - Resources eventually searchable.
    /// - Faster write operations.
    /// - Requires Kafka infrastructure.
    Async,

    /// Hybrid: inline for critical params, async for others.
    HybridAsync {
        /// Parameter codes to index inline.
        inline_params: Vec<String>,
    },
}

/// JSONB query capabilities for a backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JsonbCapabilities {
    /// JSON path extraction (PostgreSQL ->/->>).
    pub path_extraction: bool,
    /// JSON array iteration (SQLite json_each, PostgreSQL jsonb_array_elements).
    pub array_iteration: bool,
    /// JSON containment operator (PostgreSQL @>).
    pub containment_operator: bool,
    /// GIN indexing support.
    pub gin_index: bool,
}

impl JsonbCapabilities {
    /// SQLite capabilities (JSON1 extension).
    pub fn sqlite() -> Self {
        Self {
            path_extraction: true,
            array_iteration: true,
            containment_operator: false,
            gin_index: false,
        }
    }

    /// PostgreSQL capabilities (JSONB).
    pub fn postgresql() -> Self {
        Self {
            path_extraction: true,
            array_iteration: true,
            containment_operator: true,
            gin_index: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_search_param() {
        assert_eq!(SpecialSearchParam::Id.name(), "_id");
        assert_eq!(
            SpecialSearchParam::LastUpdated.param_type(),
            SearchParamType::Date
        );
        assert_eq!(SpecialSearchParam::all().len(), 13);
    }

    #[test]
    fn test_include_capability() {
        assert!(IncludeCapability::Include.is_include());
        assert!(!IncludeCapability::Revinclude.is_include());
        assert_eq!(
            IncludeCapability::IncludeIterate.modifier(),
            Some("iterate")
        );
    }

    #[test]
    fn test_chaining_capability() {
        assert_eq!(ChainingCapability::MaxDepth(3).max_depth(), Some(3));
        assert_eq!(ChainingCapability::ForwardChain.max_depth(), None);
    }

    #[test]
    fn test_pagination_capability() {
        assert_eq!(
            PaginationCapability::MaxPageSize(100).page_size(),
            Some(100)
        );
        assert_eq!(PaginationCapability::Cursor.page_size(), None);
    }

    #[test]
    fn test_search_param_full_capability() {
        let cap = SearchParamFullCapability::new("name", SearchParamType::String)
            .with_modifiers(vec!["exact", "contains"])
            .with_definition("http://hl7.org/fhir/SearchParameter/Patient-name");

        assert_eq!(cap.name, "name");
        assert!(cap.supports_modifier("exact"));
        assert!(!cap.supports_modifier("above"));
        assert!(cap.supports_prefix("eq"));
    }

    #[test]
    fn test_date_precision() {
        assert_eq!(DatePrecision::from_date_string("2024"), DatePrecision::Year);
        assert_eq!(
            DatePrecision::from_date_string("2024-01"),
            DatePrecision::Month
        );
        assert_eq!(
            DatePrecision::from_date_string("2024-01-15"),
            DatePrecision::Day
        );
        assert_eq!(
            DatePrecision::from_date_string("2024-01-15T10:30:00"),
            DatePrecision::Second
        );
    }

    /// #1316: precision comes from the parsed value, so every FHIR form
    /// classifies the same whatever its zone or fraction length. The old
    /// length-based reading stripped `+hh:mm` and `Z` but not `-hh:mm`, so a
    /// negative offset added six characters and landed on millisecond.
    #[test]
    fn test_date_precision_every_form_zone_and_fraction() {
        const ZONES: [&str; 4] = ["", "Z", "+05:30", "-04:00"];
        let mut cases: Vec<(String, DatePrecision)> = vec![
            ("2016".into(), DatePrecision::Year),
            ("2016-01".into(), DatePrecision::Month),
            ("2016-01-23".into(), DatePrecision::Day),
        ];
        for zone in ZONES {
            cases.push((format!("2016-01-23T17:07{zone}"), DatePrecision::Minute));
            for digits in [0usize, 1, 3, 6, 9] {
                let (fraction, expected) = if digits == 0 {
                    (String::new(), DatePrecision::Second)
                } else {
                    (
                        format!(".{}", &"123456789"[..digits]),
                        DatePrecision::Millisecond,
                    )
                };
                cases.push((format!("2016-01-23T17:07:42{fraction}{zone}"), expected));
            }
        }

        let wrong: Vec<String> = cases
            .iter()
            .filter_map(|(input, expected)| {
                let got = DatePrecision::from_date_string(input);
                (got != *expected).then(|| format!("{input}: got {got}, expected {expected}"))
            })
            .collect();
        assert!(wrong.is_empty(), "misclassified:\n{}", wrong.join("\n"));
    }

    /// A stored value the strict grammar rejects still gets a precision, from
    /// its shape: indexing must never fail, and a negative offset must not
    /// count towards the precision here either.
    #[test]
    fn test_date_precision_lenient_fallback() {
        let cases = [
            ("", DatePrecision::Year),
            ("0000", DatePrecision::Year),
            ("2016-13", DatePrecision::Month),
            ("2016-02-30", DatePrecision::Day),
            ("2016-01-23T", DatePrecision::Day),
            ("garbage", DatePrecision::Month),
            ("2016-01-23T17", DatePrecision::Hour),
            ("2016-01-23T17Z", DatePrecision::Hour),
            ("2016-01-23T17-04:00", DatePrecision::Hour),
            ("2016-01-23t17:07-04:00", DatePrecision::Minute),
            ("2016-01-23T24:00", DatePrecision::Minute),
            ("2016-02-30T17:07:42-04:00", DatePrecision::Second),
            ("2016-01-23T17:07:42-15:00", DatePrecision::Second),
            ("2016-01-23T17:07:42z", DatePrecision::Second),
            ("2016-02-30T17:07:42.123-04:00", DatePrecision::Millisecond),
            ("2016-01-23T17:07:42,5+01:00", DatePrecision::Millisecond),
        ];
        for (input, expected) in cases {
            assert!(FhirDateValue::parse(input).is_err(), "input {input:?}");
            assert_eq!(
                DatePrecision::from_date_string(input),
                expected,
                "input {input:?}"
            );
        }
    }

    /// Surrounding whitespace and a form-decoded `+` are the grammar's to
    /// handle, and do not change the precision.
    #[test]
    fn test_date_precision_ignores_whitespace_and_decoded_plus() {
        assert_eq!(
            DatePrecision::from_date_string(" 2016-01-23T17:07:42 05:30 "),
            DatePrecision::Second
        );
    }

    #[test]
    fn test_composite_component() {
        let comp = CompositeComponent::new(
            "http://hl7.org/fhir/SearchParameter/Observation-code",
            "Observation.code",
        );
        assert!(!comp.definition.is_empty());
        assert!(!comp.expression.is_empty());
    }

    #[test]
    fn test_search_strategy_default() {
        assert_eq!(SearchStrategy::default(), SearchStrategy::PrecomputedIndex);
    }

    #[test]
    fn test_indexing_mode_default() {
        assert_eq!(IndexingMode::default(), IndexingMode::Inline);
    }

    #[test]
    fn test_jsonb_capabilities() {
        let sqlite = JsonbCapabilities::sqlite();
        assert!(sqlite.path_extraction);
        assert!(!sqlite.containment_operator);

        let pg = JsonbCapabilities::postgresql();
        assert!(pg.containment_operator);
        assert!(pg.gin_index);
    }
}
