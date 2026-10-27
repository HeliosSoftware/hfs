//! A curated, hand-maintained catalog of the FHIRPath functions this crate's
//! evaluator implements.
//!
//! [`evaluator`](crate::evaluator) dispatches function calls through a single
//! large `match` on the function name — there is no registry a caller could
//! introspect at runtime. Tooling that needs to answer "what functions exist"
//! (a completion endpoint, a linter suggesting a fix) has nowhere else to
//! look, so this module is that list, kept in sync with the evaluator by
//! hand and guarded by a test (`builtin_functions_are_all_known_to_the_evaluator`)
//! that calls every cataloged name and asserts the evaluator recognizes it.
//!
//! Only functions callable with FHIRPath's `name(args)` invocation syntax are
//! listed here — infix type operators (`is`, `as`) and other grammar-level
//! operators are a different part of the language and are out of scope.

/// A closed set of groupings for [`FunctionInfo::category`], mirroring the
/// section structure of the [FHIRPath functions specification](https://hl7.org/fhirpath/2025Jan/#functions)
/// plus the FHIR- and SQL-on-FHIR-specific extensions this crate adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionCategory {
    /// Existence checks and cardinality (`empty`, `exists`, `count`, ...).
    Existence,
    /// Filtering and projection (`where`, `select`, `ofType`, ...).
    Filtering,
    /// Subsetting a collection (`first`, `skip`, `single`, ...).
    Subsetting,
    /// Combining two collections (`union`, `combine`).
    Combining,
    /// Type conversion (`toString`, `convertsToInteger`, `iif`, ...).
    Conversion,
    /// String manipulation (`substring`, `replace`, `join`, ...).
    String,
    /// Arithmetic and numeric aggregation (`abs`, `round`, `sum`, ...).
    Math,
    /// Tree navigation (`children`, `descendants`, `extension`).
    Tree,
    /// General-purpose utilities (`trace`, `now`, `defineVariable`, ...).
    Utility,
    /// Boolean logic functions (`not`).
    Boolean,
    /// Date/time interval arithmetic (`duration`, `difference`).
    Datetime,
    /// Type reflection (`type`).
    Types,
    /// FHIRPath extensions specific to SQL-on-FHIR ViewDefinitions
    /// (`getResourceKey`, `getReferenceKey`) — not part of the base
    /// FHIRPath specification.
    SqlOnFhir,
    /// Terminology operations (`memberOf`).
    Terminology,
    /// Doesn't fit the other categories cleanly (`resolve`, `comparable`).
    Other,
}

impl FunctionCategory {
    /// The wire/display form of this category, as used by API consumers
    /// (e.g. a JSON `category` field). Stable — treat as part of the public
    /// contract.
    pub const fn as_str(self) -> &'static str {
        match self {
            FunctionCategory::Existence => "existence",
            FunctionCategory::Filtering => "filtering",
            FunctionCategory::Subsetting => "subsetting",
            FunctionCategory::Combining => "combining",
            FunctionCategory::Conversion => "conversion",
            FunctionCategory::String => "string",
            FunctionCategory::Math => "math",
            FunctionCategory::Tree => "tree",
            FunctionCategory::Utility => "utility",
            FunctionCategory::Boolean => "boolean",
            FunctionCategory::Datetime => "datetime",
            FunctionCategory::Types => "types",
            FunctionCategory::SqlOnFhir => "sql-on-fhir",
            FunctionCategory::Terminology => "terminology",
            FunctionCategory::Other => "other",
        }
    }
}

impl std::fmt::Display for FunctionCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry in the [`builtin_functions`] catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionInfo {
    /// The function's bare name, as written before the argument list (e.g.
    /// `"where"` for `where(criteria)`).
    pub name: &'static str,
    /// A human-readable call signature, in `name(args)` form. Optional
    /// arguments are bracketed (e.g. `"round([precision])"`); a trailing
    /// `...` marks a variadic argument (e.g. `"coalesce(value, ...)"`).
    /// Not machine-parsed — for display only.
    pub signature: &'static str,
    /// The functional grouping this function belongs to.
    pub category: FunctionCategory,
}

/// Returns the catalog of FHIRPath functions the evaluator implements,
/// sorted alphabetically by [`FunctionInfo::name`] (case-insensitively —
/// `today` sorts before `toDecimal`, matching how the names read as words
/// rather than raw byte order, which would otherwise interleave on case).
///
/// Includes every function from the [FHIRPath 3.0.0 specification](https://hl7.org/fhirpath/2025Jan/#functions)
/// this crate's evaluator implements, plus the FHIR- and SQL-on-FHIR-specific
/// extensions it also supports (`extension`, `getResourceKey`,
/// `getReferenceKey`, `memberOf`, `hasValue`, `comparable`, `resolve`).
///
/// Deliberately excludes:
/// - Infix operators (`is`, `as`, `in`, `contains`, `and`, `or`, `xor`,
///   `implies`, `div`, `mod`) — a different part of the grammar, not
///   `name(args)` invocations.
/// - Terminology functions only reachable as `%terminologies.<name>(...)`
///   (`expand`, `lookup`, `validateVS`, `validateCS`, `subsumes`,
///   `translate`) — calling `{}.<name>()` for these does not resolve to the
///   same function and would misrepresent what a bare call does.
///
/// A test (`builtin_functions_are_all_known_to_the_evaluator`) keeps this
/// list from drifting out of sync with the evaluator: every name here must
/// be callable without producing the evaluator's "unknown function" error.
pub fn builtin_functions() -> &'static [FunctionInfo] {
    use FunctionCategory::*;

    const CATALOG: &[FunctionInfo] = &[
        FunctionInfo {
            name: "abs",
            signature: "abs()",
            category: Math,
        },
        FunctionInfo {
            name: "aggregate",
            signature: "aggregate(aggregator, [init])",
            category: Utility,
        },
        FunctionInfo {
            name: "all",
            signature: "all([criteria])",
            category: Existence,
        },
        FunctionInfo {
            name: "allFalse",
            signature: "allFalse()",
            category: Existence,
        },
        FunctionInfo {
            name: "allTrue",
            signature: "allTrue()",
            category: Existence,
        },
        FunctionInfo {
            name: "anyFalse",
            signature: "anyFalse()",
            category: Existence,
        },
        FunctionInfo {
            name: "anyTrue",
            signature: "anyTrue()",
            category: Existence,
        },
        FunctionInfo {
            name: "avg",
            signature: "avg()",
            category: Math,
        },
        FunctionInfo {
            name: "ceiling",
            signature: "ceiling()",
            category: Math,
        },
        FunctionInfo {
            name: "children",
            signature: "children()",
            category: Tree,
        },
        FunctionInfo {
            name: "coalesce",
            signature: "coalesce(value, ...)",
            category: Filtering,
        },
        FunctionInfo {
            name: "combine",
            signature: "combine(other)",
            category: Combining,
        },
        FunctionInfo {
            name: "comparable",
            signature: "comparable(other)",
            category: Other,
        },
        FunctionInfo {
            name: "contains",
            signature: "contains(substring)",
            category: String,
        },
        FunctionInfo {
            name: "convertsToBoolean",
            signature: "convertsToBoolean()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToDate",
            signature: "convertsToDate()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToDateTime",
            signature: "convertsToDateTime()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToDecimal",
            signature: "convertsToDecimal()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToInteger",
            signature: "convertsToInteger()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToLong",
            signature: "convertsToLong()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToQuantity",
            signature: "convertsToQuantity([unit])",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToString",
            signature: "convertsToString()",
            category: Conversion,
        },
        FunctionInfo {
            name: "convertsToTime",
            signature: "convertsToTime()",
            category: Conversion,
        },
        FunctionInfo {
            name: "count",
            signature: "count()",
            category: Existence,
        },
        FunctionInfo {
            name: "decode",
            signature: "decode(format)",
            category: String,
        },
        FunctionInfo {
            name: "defineVariable",
            signature: "defineVariable(name, [expr])",
            category: Utility,
        },
        FunctionInfo {
            name: "descendants",
            signature: "descendants()",
            category: Tree,
        },
        FunctionInfo {
            name: "difference",
            signature: "difference(value, precision)",
            category: Datetime,
        },
        FunctionInfo {
            name: "distinct",
            signature: "distinct()",
            category: Existence,
        },
        FunctionInfo {
            name: "duration",
            signature: "duration(value, precision)",
            category: Datetime,
        },
        FunctionInfo {
            name: "empty",
            signature: "empty()",
            category: Existence,
        },
        FunctionInfo {
            name: "encode",
            signature: "encode(format)",
            category: String,
        },
        FunctionInfo {
            name: "endsWith",
            signature: "endsWith(suffix)",
            category: String,
        },
        FunctionInfo {
            name: "escape",
            signature: "escape(target)",
            category: String,
        },
        FunctionInfo {
            name: "exclude",
            signature: "exclude(other)",
            category: Subsetting,
        },
        FunctionInfo {
            name: "exists",
            signature: "exists([criteria])",
            category: Existence,
        },
        FunctionInfo {
            name: "exp",
            signature: "exp()",
            category: Math,
        },
        FunctionInfo {
            name: "extension",
            signature: "extension(url)",
            category: Tree,
        },
        FunctionInfo {
            name: "first",
            signature: "first()",
            category: Subsetting,
        },
        FunctionInfo {
            name: "floor",
            signature: "floor()",
            category: Math,
        },
        FunctionInfo {
            name: "getReferenceKey",
            signature: "getReferenceKey([type])",
            category: SqlOnFhir,
        },
        FunctionInfo {
            name: "getResourceKey",
            signature: "getResourceKey()",
            category: SqlOnFhir,
        },
        FunctionInfo {
            name: "hasValue",
            signature: "hasValue()",
            category: Existence,
        },
        FunctionInfo {
            name: "highBoundary",
            signature: "highBoundary([precision])",
            category: Utility,
        },
        FunctionInfo {
            name: "iif",
            signature: "iif(criterion, trueResult, [otherwiseResult])",
            category: Conversion,
        },
        FunctionInfo {
            name: "indexOf",
            signature: "indexOf(substring)",
            category: String,
        },
        FunctionInfo {
            name: "intersect",
            signature: "intersect(other)",
            category: Subsetting,
        },
        FunctionInfo {
            name: "isDistinct",
            signature: "isDistinct()",
            category: Existence,
        },
        FunctionInfo {
            name: "join",
            signature: "join([separator])",
            category: String,
        },
        FunctionInfo {
            name: "last",
            signature: "last()",
            category: Subsetting,
        },
        FunctionInfo {
            name: "lastIndexOf",
            signature: "lastIndexOf(substring)",
            category: String,
        },
        FunctionInfo {
            name: "length",
            signature: "length()",
            category: String,
        },
        FunctionInfo {
            name: "ln",
            signature: "ln()",
            category: Math,
        },
        FunctionInfo {
            name: "log",
            signature: "log(base)",
            category: Math,
        },
        FunctionInfo {
            name: "lowBoundary",
            signature: "lowBoundary([precision])",
            category: Utility,
        },
        FunctionInfo {
            name: "lower",
            signature: "lower()",
            category: String,
        },
        FunctionInfo {
            name: "matches",
            signature: "matches(regex, [flags])",
            category: String,
        },
        FunctionInfo {
            name: "matchesFull",
            signature: "matchesFull(regex, [flags])",
            category: String,
        },
        FunctionInfo {
            name: "max",
            signature: "max()",
            category: Math,
        },
        FunctionInfo {
            name: "memberOf",
            signature: "memberOf(valueSet)",
            category: Terminology,
        },
        FunctionInfo {
            name: "min",
            signature: "min()",
            category: Math,
        },
        FunctionInfo {
            name: "not",
            signature: "not()",
            category: Boolean,
        },
        FunctionInfo {
            name: "now",
            signature: "now()",
            category: Utility,
        },
        FunctionInfo {
            name: "ofType",
            signature: "ofType(type)",
            category: Filtering,
        },
        FunctionInfo {
            name: "power",
            signature: "power(exponent)",
            category: Math,
        },
        FunctionInfo {
            name: "precision",
            signature: "precision()",
            category: Utility,
        },
        FunctionInfo {
            name: "repeat",
            signature: "repeat(projection)",
            category: Filtering,
        },
        FunctionInfo {
            name: "repeatAll",
            signature: "repeatAll(projection)",
            category: Filtering,
        },
        FunctionInfo {
            name: "replace",
            signature: "replace(pattern, substitution)",
            category: String,
        },
        FunctionInfo {
            name: "replaceMatches",
            signature: "replaceMatches(regex, substitution, [flags])",
            category: String,
        },
        FunctionInfo {
            name: "resolve",
            signature: "resolve()",
            category: Other,
        },
        FunctionInfo {
            name: "round",
            signature: "round([precision])",
            category: Math,
        },
        FunctionInfo {
            name: "select",
            signature: "select(projection)",
            category: Filtering,
        },
        FunctionInfo {
            name: "single",
            signature: "single()",
            category: Subsetting,
        },
        FunctionInfo {
            name: "skip",
            signature: "skip(num)",
            category: Subsetting,
        },
        FunctionInfo {
            name: "sort",
            signature: "sort([criteria])",
            category: Filtering,
        },
        FunctionInfo {
            name: "split",
            signature: "split(separator)",
            category: String,
        },
        FunctionInfo {
            name: "sqrt",
            signature: "sqrt()",
            category: Math,
        },
        FunctionInfo {
            name: "startsWith",
            signature: "startsWith(prefix)",
            category: String,
        },
        FunctionInfo {
            name: "subsetOf",
            signature: "subsetOf(other)",
            category: Existence,
        },
        FunctionInfo {
            name: "substring",
            signature: "substring(start, [length])",
            category: String,
        },
        FunctionInfo {
            name: "sum",
            signature: "sum()",
            category: Math,
        },
        FunctionInfo {
            name: "supersetOf",
            signature: "supersetOf(other)",
            category: Existence,
        },
        FunctionInfo {
            name: "tail",
            signature: "tail()",
            category: Subsetting,
        },
        FunctionInfo {
            name: "take",
            signature: "take(num)",
            category: Subsetting,
        },
        FunctionInfo {
            name: "timeOfDay",
            signature: "timeOfDay()",
            category: Utility,
        },
        FunctionInfo {
            name: "toBoolean",
            signature: "toBoolean()",
            category: Conversion,
        },
        FunctionInfo {
            name: "toChars",
            signature: "toChars()",
            category: String,
        },
        FunctionInfo {
            name: "toDate",
            signature: "toDate([format])",
            category: Conversion,
        },
        FunctionInfo {
            name: "toDateTime",
            signature: "toDateTime([format])",
            category: Conversion,
        },
        FunctionInfo {
            name: "today",
            signature: "today()",
            category: Utility,
        },
        FunctionInfo {
            name: "toDecimal",
            signature: "toDecimal()",
            category: Conversion,
        },
        FunctionInfo {
            name: "toInteger",
            signature: "toInteger()",
            category: Conversion,
        },
        FunctionInfo {
            name: "toLong",
            signature: "toLong()",
            category: Conversion,
        },
        FunctionInfo {
            name: "toQuantity",
            signature: "toQuantity([unit])",
            category: Conversion,
        },
        FunctionInfo {
            name: "toString",
            signature: "toString([format])",
            category: Conversion,
        },
        FunctionInfo {
            name: "toTime",
            signature: "toTime()",
            category: Conversion,
        },
        FunctionInfo {
            name: "trace",
            signature: "trace(name, [projection])",
            category: Utility,
        },
        FunctionInfo {
            name: "trim",
            signature: "trim()",
            category: String,
        },
        FunctionInfo {
            name: "truncate",
            signature: "truncate()",
            category: Math,
        },
        FunctionInfo {
            name: "type",
            signature: "type()",
            category: Types,
        },
        FunctionInfo {
            name: "unescape",
            signature: "unescape(target)",
            category: String,
        },
        FunctionInfo {
            name: "union",
            signature: "union(other)",
            category: Combining,
        },
        FunctionInfo {
            name: "upper",
            signature: "upper()",
            category: String,
        },
        FunctionInfo {
            name: "where",
            signature: "where(criteria)",
            category: Filtering,
        },
    ];
    CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_sorted_case_insensitively_by_name() {
        let names: Vec<String> = builtin_functions()
            .iter()
            .map(|f| f.name.to_lowercase())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "builtin_functions() must be sorted alphabetically (case-insensitively) by name"
        );
    }

    #[test]
    fn catalog_has_no_duplicate_names() {
        let mut names: Vec<&str> = builtin_functions().iter().map(|f| f.name).collect();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), original_len, "duplicate function name found");
    }

    #[test]
    fn every_category_string_is_from_the_closed_set() {
        const ALLOWED: &[&str] = &[
            "existence",
            "filtering",
            "subsetting",
            "combining",
            "conversion",
            "string",
            "math",
            "tree",
            "utility",
            "boolean",
            "datetime",
            "types",
            "sql-on-fhir",
            "terminology",
            "other",
        ];
        for f in builtin_functions() {
            assert!(
                ALLOWED.contains(&f.category.as_str()),
                "function {:?} has category {:?} outside the closed set",
                f.name,
                f.category.as_str()
            );
        }
    }
}
