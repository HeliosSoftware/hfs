//! VCL AST → `ValueSet` translation.
//!
//! The translator is the gate between the parser and the backends: it only
//! emits compose shapes that both the SQLite and the PostgreSQL backend
//! evaluate correctly, and rejects everything else (the backends turn an
//! unknown filter operator into an empty set without an error).
//!
//! ## Mapping
//!
//! | VCL | `compose` |
//! |-----|-----------|
//! | `;` | several `include` entries |
//! | `,` over filters of one system | one `include` with several `filter`s |
//! | `,` / `;` over plain codes | one `include` with `concept[]` |
//! | `A-B` | `exclude` |
//! | `^(vs)` | `include.valueSet` (an intersection with the rest of the include) |
//! | `(sys)*` | an `include` with only `system` |
//! | `(sys)(…)` | the system is distributed to the inner terms without one |
//! | `=`, `<<`, `~<<`, `<`, `<!`, `>>`, `/`, `^{…}`, `~^{…}` | `=`, `is-a`, `is-not-a`, `descendent-of`, `child-of`, `generalizes`, `regex`, `in`, `not-in` |
//!
//! `concept=X` and `concept^{X,Y}` become `concept[]` (the backends evaluate
//! `=` and `in` against concept properties, not against the code).
//!
//! An `include` never carries both `concept[]` and `filter[]`: SQLite ignores
//! the former and PostgreSQL intersects them. Groups that cannot be flattened
//! (an AND over unions, an exclusion nested inside another expression, a code
//! list combined with a filter) are hoisted into `contained` ValueSets named
//! `vcl-N` and referenced as `#vcl-N`. The backends stop resolving references
//! at depth 4, so more than [`MAX_REF_DEPTH`] levels of hoisting are rejected.
//!
//! Errors: malformed or incomplete expressions (no code system) are
//! [`HtsError::VsInvalid`] (400); constructs HTS cannot evaluate (`!!<`, `?`,
//! `of`, `p^URI`, nested filter lists, `concept~^{…}`, hierarchy operators on
//! properties other than `concept`) are [`HtsError::NotSupported`] (501).
//! Both carry the 1-based column of the construct.
//!
//! Deviation from the reference parsers: `,` between plain codes
//! (`(sys)(A,B)`) is a union, as in the published test cases; any other `,` is
//! an intersection.

use serde_json::{Map, Value, json};

use super::parser::{Expr, ExprKind, Filter, FilterOp, FilterValue, UriRef};
use crate::error::HtsError;

/// Deepest chain of `#vcl-N` references the translator will emit. The backends
/// silently skip references at depth 4 and beyond.
pub const MAX_REF_DEPTH: usize = 3;

/// Largest number of includes a flattened intersection may produce before the
/// translator falls back to `contained` groups.
const MAX_CROSS_PRODUCT: usize = 64;

fn invalid(pos: usize, message: impl std::fmt::Display) -> HtsError {
    HtsError::VsInvalid(format!(
        "Invalid VCL expression at position {pos}: {message}"
    ))
}

fn unsupported(pos: usize, what: impl std::fmt::Display) -> HtsError {
    HtsError::NotSupported(format!(
        "VCL construct not supported at position {pos}: {what}"
    ))
}

/// Translates `expr` into a synthetic `ValueSet` with `url` set to `url`.
pub fn to_value_set(expr: &Expr, url: &str) -> Result<Value, HtsError> {
    let mut t = Translator::default();
    let set = t.set(expr, &None)?;
    let compose = compose_json(&set)?;
    let mut vs = Map::new();
    vs.insert("resourceType".into(), json!("ValueSet"));
    vs.insert("status".into(), json!("active"));
    vs.insert("url".into(), json!(url));
    vs.insert("compose".into(), compose);
    if !t.contained.is_empty() {
        vs.insert("contained".into(), Value::Array(t.contained));
    }
    Ok(Value::Object(vs))
}

// ── Intermediate form ─────────────────────────────────────────────────────────

/// One `include` / `exclude` entry before serialisation.
#[derive(Debug, Clone)]
struct Clause {
    pos: usize,
    system: Option<UriRef>,
    concepts: Vec<String>,
    filters: Vec<Value>,
    value_sets: Vec<String>,
    /// Levels of `#vcl-N` references below this clause.
    ref_depth: usize,
}

impl Clause {
    fn new(pos: usize, system: Option<UriRef>) -> Self {
        Self {
            pos,
            system,
            concepts: Vec::new(),
            filters: Vec::new(),
            value_sets: Vec::new(),
            ref_depth: 0,
        }
    }

    fn is_pure_concepts(&self) -> bool {
        !self.concepts.is_empty() && self.filters.is_empty() && self.value_sets.is_empty()
    }

    fn is_unrestricted(&self) -> bool {
        self.concepts.is_empty() && self.filters.is_empty() && self.value_sets.is_empty()
    }
}

/// A compose: the union of `includes` minus the union of `excludes`.
#[derive(Debug, Clone, Default)]
struct Set {
    includes: Vec<Clause>,
    excludes: Vec<Clause>,
}

impl Set {
    fn single(clause: Clause) -> Self {
        Self {
            includes: vec![clause],
            excludes: Vec::new(),
        }
    }

    fn max_ref_depth(&self) -> usize {
        self.includes
            .iter()
            .chain(&self.excludes)
            .map(|c| c.ref_depth)
            .max()
            .unwrap_or(0)
    }

    fn pos(&self) -> usize {
        self.includes.first().map_or(1, |c| c.pos)
    }
}

fn same_system(a: &UriRef, b: &UriRef) -> bool {
    a.uri == b.uri && a.version == b.version
}

fn same_optional_system(a: &Option<UriRef>, b: &Option<UriRef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => same_system(x, y),
        _ => false,
    }
}

/// The intersection of two clauses as one clause, or `None` when that cannot
/// be expressed in a single `include` (different systems, a code list next to
/// a filter, an empty code intersection).
fn and_clause(a: &Clause, b: &Clause) -> Option<Clause> {
    let system = match (&a.system, &b.system) {
        (None, None) => None,
        (Some(s), None) | (None, Some(s)) => Some(s.clone()),
        (Some(x), Some(y)) if same_system(x, y) => Some(x.clone()),
        _ => return None,
    };
    let mut out = Clause::new(a.pos, system);
    match (a.concepts.is_empty(), b.concepts.is_empty()) {
        (false, false) => {
            if !a.is_pure_concepts() || !b.is_pure_concepts() {
                return None;
            }
            out.concepts = a
                .concepts
                .iter()
                .filter(|c| b.concepts.contains(c))
                .cloned()
                .collect();
            if out.concepts.is_empty() {
                return None;
            }
        }
        (false, true) | (true, false) => {
            let (codes, other) = if a.concepts.is_empty() {
                (b, a)
            } else {
                (a, b)
            };
            if !other.is_unrestricted() {
                return None;
            }
            out.concepts = codes.concepts.clone();
        }
        (true, true) => {}
    }
    out.filters = a.filters.iter().chain(&b.filters).cloned().collect();
    for vs in a.value_sets.iter().chain(&b.value_sets) {
        if !out.value_sets.contains(vs) {
            out.value_sets.push(vs.clone());
        }
    }
    out.ref_depth = a.ref_depth.max(b.ref_depth);
    Some(out)
}

/// Folds pure code clauses of the same system into one clause.
fn merge_pure(clauses: Vec<Clause>) -> Vec<Clause> {
    let mut out: Vec<Clause> = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_pure_concepts()
            && let Some(prev) = out
                .iter_mut()
                .find(|p| p.is_pure_concepts() && same_optional_system(&p.system, &c.system))
        {
            for code in c.concepts {
                if !prev.concepts.contains(&code) {
                    prev.concepts.push(code);
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn is_concept_property(property: &str) -> bool {
    property == "concept" || property == "code"
}

/// A code, optionally scoped to a system: the operands of a `,` that the
/// published test cases read as a union.
fn is_plain_code(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Code(_) => true,
        ExprKind::Scoped { expr, .. } => is_plain_code(expr),
        _ => false,
    }
}

fn uri_text(r: &UriRef) -> String {
    match &r.version {
        Some(v) => format!("{}|{v}", r.uri),
        None => r.uri.clone(),
    }
}

// ── Translator ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Translator {
    contained: Vec<Value>,
}

impl Translator {
    fn set(&mut self, expr: &Expr, sys: &Option<UriRef>) -> Result<Set, HtsError> {
        match &expr.kind {
            ExprKind::All => Ok(Set::single(Clause::new(expr.pos, sys.clone()))),
            ExprKind::Code(code) => {
                let mut c = Clause::new(expr.pos, sys.clone());
                c.concepts.push(code.clone());
                Ok(Set::single(c))
            }
            ExprKind::Filter(f) => Ok(Set::single(filter_clause(f, sys)?)),
            ExprKind::IncludeValueSet(r) => {
                let mut c = Clause::new(expr.pos, sys.clone());
                c.value_sets.push(uri_text(r));
                Ok(Set::single(c))
            }
            ExprKind::Scoped {
                system,
                expr: inner,
            } => self.set(inner, &Some(system.clone())),
            ExprKind::Disjunction(items) => {
                let sets = self.operands(items, sys)?;
                self.union(sets, expr.pos)
            }
            ExprKind::Conjunction(items) => {
                let sets = self.operands(items, sys)?;
                if items.iter().all(is_plain_code) {
                    self.union(sets, expr.pos)
                } else {
                    self.intersect(sets, expr.pos)
                }
            }
            ExprKind::Exclusion { left, right } => {
                let l = self.set(left, sys)?;
                let r = self.set(right, sys)?;
                let includes = if l.excludes.is_empty() {
                    l.includes
                } else {
                    vec![self.hoist(l, expr.pos)?]
                };
                let excludes = if r.excludes.is_empty() {
                    r.includes
                } else {
                    vec![self.hoist(r, expr.pos)?]
                };
                Ok(Set { includes, excludes })
            }
        }
    }

    fn operands(&mut self, items: &[Expr], sys: &Option<UriRef>) -> Result<Vec<Set>, HtsError> {
        items.iter().map(|e| self.set(e, sys)).collect()
    }

    /// `A;B`: concatenated includes. An operand with its own excludes cannot
    /// be flattened (`(A-B);C` is not `A;C` minus `B`) and is hoisted.
    fn union(&mut self, sets: Vec<Set>, pos: usize) -> Result<Set, HtsError> {
        let mut includes = Vec::new();
        for s in sets {
            if s.excludes.is_empty() {
                includes.extend(s.includes);
            } else {
                includes.push(self.hoist(s, pos)?);
            }
        }
        Ok(Set {
            includes: merge_pure(includes),
            excludes: Vec::new(),
        })
    }

    /// `A,B`: the intersection, distributed over unions when every pair of
    /// clauses combines into one `include`, otherwise as the intersection of
    /// hoisted groups.
    fn intersect(&mut self, sets: Vec<Set>, pos: usize) -> Result<Set, HtsError> {
        let mut ops = Vec::with_capacity(sets.len());
        for s in sets {
            if s.excludes.is_empty() {
                ops.push(s);
            } else {
                ops.push(Set::single(self.hoist(s, pos)?));
            }
        }
        if let Some(flat) = cross_product(&ops) {
            return Ok(Set {
                includes: merge_pure(flat),
                excludes: Vec::new(),
            });
        }
        let mut out = Clause::new(pos, None);
        for s in ops {
            let clause = match s.includes.as_slice() {
                [c] if s.excludes.is_empty()
                    && c.system.is_none()
                    && c.is_unrestricted_but_refs() =>
                {
                    c.clone()
                }
                _ => self.hoist(s, pos)?,
            };
            for r in clause.value_sets {
                if !out.value_sets.contains(&r) {
                    out.value_sets.push(r);
                }
            }
            out.ref_depth = out.ref_depth.max(clause.ref_depth);
        }
        Ok(Set::single(out))
    }

    /// Moves a compose into `contained` and returns a clause that references
    /// it.
    fn hoist(&mut self, set: Set, pos: usize) -> Result<Clause, HtsError> {
        let depth = set.max_ref_depth();
        if depth + 1 > MAX_REF_DEPTH {
            return Err(invalid(
                set.pos(),
                format!(
                    "expression is too deeply grouped to evaluate (more than {MAX_REF_DEPTH} levels of nested exclusions or intersections)"
                ),
            ));
        }
        let compose = compose_json(&set)?;
        let id = format!("vcl-{}", self.contained.len() + 1);
        self.contained.push(json!({
            "resourceType": "ValueSet",
            "id": id,
            "status": "active",
            "compose": compose,
        }));
        let mut c = Clause::new(pos, None);
        c.value_sets.push(format!("#{id}"));
        c.ref_depth = depth + 1;
        Ok(c)
    }
}

impl Clause {
    /// Only `valueSet` references, nothing else.
    fn is_unrestricted_but_refs(&self) -> bool {
        self.concepts.is_empty() && self.filters.is_empty() && !self.value_sets.is_empty()
    }
}

fn cross_product(ops: &[Set]) -> Option<Vec<Clause>> {
    let (first, rest) = ops.split_first()?;
    let mut acc = first.includes.clone();
    for next in rest {
        let mut out = Vec::with_capacity(acc.len() * next.includes.len());
        for a in &acc {
            for b in &next.includes {
                out.push(and_clause(a, b)?);
            }
        }
        if out.len() > MAX_CROSS_PRODUCT {
            return None;
        }
        acc = out;
    }
    Some(acc)
}

// ── Filters ───────────────────────────────────────────────────────────────────

fn filter_json(property: &str, op: &str, value: &str) -> Value {
    json!({ "property": property, "op": op, "value": value })
}

fn filter_clause(f: &Filter, sys: &Option<UriRef>) -> Result<Clause, HtsError> {
    let mut c = Clause::new(f.pos, sys.clone());
    let concept_property = is_concept_property(&f.property);
    let hierarchy_op = match f.op {
        FilterOp::IsA => Some("is-a"),
        FilterOp::IsNotA => Some("is-not-a"),
        FilterOp::DescendantOf => Some("descendent-of"),
        FilterOp::ChildOf => Some("child-of"),
        FilterOp::Generalizes => Some("generalizes"),
        _ => None,
    };
    if let Some(op) = hierarchy_op {
        if !concept_property {
            return Err(unsupported(
                f.pos,
                format!(
                    "the '{op}' operator on property '{}' (only 'concept' is supported)",
                    f.property
                ),
            ));
        }
        let FilterValue::Code(code) = &f.value else {
            return Err(unsupported(
                f.pos,
                "a hierarchy operator without a single code",
            ));
        };
        c.filters.push(filter_json(&f.property, op, code));
        return Ok(c);
    }
    match (&f.op, &f.value) {
        (FilterOp::Eq, FilterValue::Code(code)) => {
            if concept_property {
                c.concepts.push(code.clone());
            } else {
                c.filters.push(filter_json(&f.property, "=", code));
            }
        }
        (FilterOp::Regex, FilterValue::Regex(pattern)) => {
            c.filters.push(filter_json(&f.property, "regex", pattern));
        }
        (FilterOp::In, FilterValue::CodeList(codes)) => {
            check_list(f, codes)?;
            if concept_property {
                c.concepts.extend(codes.iter().cloned());
            } else {
                c.filters
                    .push(filter_json(&f.property, "in", &codes.join(",")));
            }
        }
        (FilterOp::NotIn, FilterValue::CodeList(codes)) => {
            check_list(f, codes)?;
            if concept_property {
                return Err(unsupported(f.pos, "'~^' on the 'concept' property"));
            }
            c.filters
                .push(filter_json(&f.property, "not-in", &codes.join(",")));
        }
        (FilterOp::In | FilterOp::NotIn, FilterValue::Uri(_)) => {
            return Err(unsupported(
                f.pos,
                "a value set URI after '^' / '~^' in a property filter",
            ));
        }
        (FilterOp::In | FilterOp::NotIn, FilterValue::FilterList(_)) => {
            return Err(unsupported(f.pos, "a nested filter list after '^' / '~^'"));
        }
        (FilterOp::DescendantLeaf, _) => {
            return Err(unsupported(f.pos, "the '!!<' (descendent-leaf) operator"));
        }
        (FilterOp::Exists, _) => {
            return Err(unsupported(f.pos, "the '?' (exists) operator"));
        }
        (FilterOp::Of, _) => {
            return Err(unsupported(f.pos, "the 'of' operator ('{…}.property')"));
        }
        _ => return Err(unsupported(f.pos, "this filter form")),
    }
    Ok(c)
}

/// Lists are serialised comma-separated, so a code containing a comma would be
/// split by the backends.
fn check_list(f: &Filter, codes: &[String]) -> Result<(), HtsError> {
    if codes.iter().any(|c| c.contains(',')) {
        return Err(unsupported(
            f.pos,
            "a code containing ',' inside a '{…}' list",
        ));
    }
    Ok(())
}

// ── Serialisation ─────────────────────────────────────────────────────────────

fn clause_json(c: &Clause) -> Result<Value, HtsError> {
    if c.system.is_none() && c.value_sets.is_empty() {
        return Err(invalid(
            c.pos,
            "a code system is required, e.g. '(http://snomed.info/sct)…'",
        ));
    }
    let mut m = Map::new();
    if let Some(s) = &c.system {
        m.insert("system".into(), json!(s.uri));
        if let Some(v) = s.version.as_ref().filter(|v| !v.is_empty()) {
            m.insert("version".into(), json!(v));
        }
    }
    if !c.concepts.is_empty() {
        let codes: Vec<Value> = c
            .concepts
            .iter()
            .map(|code| json!({ "code": code }))
            .collect();
        m.insert("concept".into(), Value::Array(codes));
    }
    if !c.filters.is_empty() {
        m.insert("filter".into(), Value::Array(c.filters.clone()));
    }
    if !c.value_sets.is_empty() {
        m.insert("valueSet".into(), json!(c.value_sets));
    }
    Ok(Value::Object(m))
}

fn compose_json(set: &Set) -> Result<Value, HtsError> {
    let mut m = Map::new();
    let includes: Result<Vec<Value>, HtsError> = set.includes.iter().map(clause_json).collect();
    m.insert("include".into(), Value::Array(includes?));
    if !set.excludes.is_empty() {
        let excludes: Result<Vec<Value>, HtsError> = set.excludes.iter().map(clause_json).collect();
        m.insert("exclude".into(), Value::Array(excludes?));
    }
    Ok(Value::Object(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcl::parser::parse;

    const URL: &str = "http://fhir.org/VCL?v1=x";

    fn vs(expr: &str) -> Value {
        to_value_set(&parse(expr).unwrap(), URL).unwrap()
    }

    fn compose(expr: &str) -> Value {
        vs(expr)["compose"].clone()
    }

    fn err(expr: &str) -> HtsError {
        to_value_set(&parse(expr).unwrap(), URL).unwrap_err()
    }

    fn include_one(system: &str, member: (&str, Value)) -> Value {
        json!({ "include": [{ "system": system, member.0: member.1 }] })
    }

    #[test]
    fn value_set_envelope() {
        let v = vs("(http://s)A");
        assert_eq!(v["resourceType"], "ValueSet");
        assert_eq!(v["status"], "active");
        assert_eq!(v["url"], URL);
        assert!(v.get("contained").is_none());
    }

    #[test]
    fn code_becomes_concept() {
        assert_eq!(
            compose("(http://s)A"),
            include_one("http://s", ("concept", json!([{ "code": "A" }])))
        );
    }

    #[test]
    fn system_version_is_split() {
        assert_eq!(
            compose("(http://s|20210131)123"),
            json!({ "include": [{
                "system": "http://s",
                "version": "20210131",
                "concept": [{ "code": "123" }],
            }] })
        );
    }

    #[test]
    fn code_list_and_comma_over_codes_are_one_include() {
        let expected = include_one(
            "http://s",
            ("concept", json!([{ "code": "A" }, { "code": "B" }])),
        );
        assert_eq!(compose("(http://s)(A;B)"), expected);
        assert_eq!(compose("(http://s)(A,B)"), expected);
    }

    #[test]
    fn disjunction_makes_separate_includes() {
        assert_eq!(
            compose("(http://s)A;(http://t)B"),
            json!({ "include": [
                { "system": "http://s", "concept": [{ "code": "A" }] },
                { "system": "http://t", "concept": [{ "code": "B" }] },
            ] })
        );
    }

    #[test]
    fn conjunction_of_filters_shares_one_include() {
        assert_eq!(
            compose("(http://s)(a=1,b=2)"),
            include_one(
                "http://s",
                (
                    "filter",
                    json!([
                        { "property": "a", "op": "=", "value": "1" },
                        { "property": "b", "op": "=", "value": "2" },
                    ])
                )
            )
        );
    }

    #[test]
    fn conjunction_distributes_over_disjunction() {
        let c = compose("(http://s)(a=1;b=2),(http://s)(c=3;d=4)");
        assert_eq!(c["include"].as_array().unwrap().len(), 4);
        assert_eq!(c["include"][1]["filter"][0]["property"], "a");
        assert_eq!(c["include"][1]["filter"][1]["property"], "d");
    }

    #[test]
    fn exclusion_becomes_exclude() {
        assert_eq!(
            compose("(http://snomed.info/sct)<<404684003 - (http://snomed.info/sct)<<64572001"),
            json!({
                "include": [{
                    "system": "http://snomed.info/sct",
                    "filter": [{ "property": "concept", "op": "is-a", "value": "404684003" }],
                }],
                "exclude": [{
                    "system": "http://snomed.info/sct",
                    "filter": [{ "property": "concept", "op": "is-a", "value": "64572001" }],
                }],
            })
        );
    }

    #[test]
    fn system_is_distributed_into_exclusion() {
        let c = compose("(http://s)((A;B)-(C))");
        assert_eq!(c["include"][0]["system"], "http://s");
        assert_eq!(c["exclude"][0]["system"], "http://s");
        assert_eq!(c["exclude"][0]["concept"], json!([{ "code": "C" }]));
    }

    #[test]
    fn star_is_a_whole_system_include() {
        assert_eq!(
            compose("(http://snomed.info/sct)*"),
            json!({ "include": [{ "system": "http://snomed.info/sct" }] })
        );
    }

    #[test]
    fn value_set_inclusion() {
        assert_eq!(
            compose("^(http://hl7.org/fhir/ValueSet/example)"),
            json!({ "include": [{ "valueSet": ["http://hl7.org/fhir/ValueSet/example"] }] })
        );
        assert_eq!(
            compose("^(http://example.org/vs|1.0)")["include"][0]["valueSet"],
            json!(["http://example.org/vs|1.0"])
        );
    }

    #[test]
    fn value_set_inclusion_intersects_with_system_filter() {
        assert_eq!(
            compose("(http://s)(p=1,^(http://vs))"),
            json!({ "include": [{
                "system": "http://s",
                "filter": [{ "property": "p", "op": "=", "value": "1" }],
                "valueSet": ["http://vs"],
            }] })
        );
    }

    fn only_filter(expr: &str) -> Value {
        compose(expr)["include"][0]["filter"][0].clone()
    }

    #[test]
    fn property_equals() {
        assert_eq!(
            only_filter("(http://s)COMPONENT=LP15653-6"),
            json!({ "property": "COMPONENT", "op": "=", "value": "LP15653-6" })
        );
    }

    #[test]
    fn concept_equals_is_a_code() {
        assert_eq!(
            compose("(http://s)concept=X")["include"][0]["concept"],
            json!([{ "code": "X" }])
        );
    }

    #[test]
    fn is_a() {
        assert_eq!(
            only_filter("(http://s)concept<<5"),
            json!({ "property": "concept", "op": "is-a", "value": "5" })
        );
    }

    #[test]
    fn missing_property_defaults_to_concept() {
        assert_eq!(
            only_filter("(http://s)<<5"),
            json!({ "property": "concept", "op": "is-a", "value": "5" })
        );
    }

    #[test]
    fn is_not_a() {
        assert_eq!(
            only_filter("(http://s)concept~<<5"),
            json!({ "property": "concept", "op": "is-not-a", "value": "5" })
        );
    }

    #[test]
    fn descendant_of() {
        assert_eq!(
            only_filter("(http://s)concept<5"),
            json!({ "property": "concept", "op": "descendent-of", "value": "5" })
        );
    }

    #[test]
    fn child_of() {
        assert_eq!(
            only_filter("(http://s)concept<!5"),
            json!({ "property": "concept", "op": "child-of", "value": "5" })
        );
    }

    #[test]
    fn generalizes() {
        assert_eq!(
            only_filter("(http://s)concept>>5"),
            json!({ "property": "concept", "op": "generalizes", "value": "5" })
        );
    }

    #[test]
    fn regex() {
        assert_eq!(
            only_filter(r#"(http://s)COMPONENT/".*test.*""#),
            json!({ "property": "COMPONENT", "op": "regex", "value": ".*test.*" })
        );
    }

    #[test]
    fn in_list() {
        assert_eq!(
            only_filter("(http://s)parent^{a,b}"),
            json!({ "property": "parent", "op": "in", "value": "a,b" })
        );
        assert_eq!(
            only_filter("(http://s)parent^{a}"),
            json!({ "property": "parent", "op": "in", "value": "a" })
        );
    }

    #[test]
    fn concept_in_list_is_a_code_list() {
        assert_eq!(
            compose("(http://s)concept^{a,b}")["include"][0]["concept"],
            json!([{ "code": "a" }, { "code": "b" }])
        );
    }

    #[test]
    fn not_in_list() {
        assert_eq!(
            only_filter("(http://s)parent~^{a,b}"),
            json!({ "property": "parent", "op": "not-in", "value": "a,b" })
        );
    }

    #[test]
    fn code_filter_and_filter_group_hoists_into_contained() {
        let v = vs("(http://s)A,(http://s)p=1");
        assert_eq!(
            v["compose"],
            json!({ "include": [{ "valueSet": ["#vcl-1", "#vcl-2"] }] })
        );
        let contained = v["contained"].as_array().unwrap();
        assert_eq!(contained.len(), 2);
        assert_eq!(contained[0]["id"], "vcl-1");
        assert_eq!(contained[0]["resourceType"], "ValueSet");
        assert_eq!(
            contained[0]["compose"],
            include_one("http://s", ("concept", json!([{ "code": "A" }])))
        );
        assert_eq!(contained[1]["id"], "vcl-2");
    }

    #[test]
    fn nested_exclusion_is_hoisted() {
        let v = vs("(((http://s)(A))-((http://s)(B)))-((http://s)(C))");
        assert_eq!(
            v["compose"],
            json!({
                "include": [{ "valueSet": ["#vcl-1"] }],
                "exclude": [{ "system": "http://s", "concept": [{ "code": "C" }] }],
            })
        );
        let inner = &v["contained"][0]["compose"];
        assert_eq!(inner["include"][0]["concept"], json!([{ "code": "A" }]));
        assert_eq!(inner["exclude"][0]["concept"], json!([{ "code": "B" }]));
    }

    #[test]
    fn exclusion_inside_union_is_hoisted() {
        let v = vs("(((http://s)(A))-((http://s)(B)));(http://s)(C)");
        let includes = v["compose"]["include"].as_array().unwrap();
        assert_eq!(includes.len(), 2);
        assert_eq!(includes[0]["valueSet"], json!(["#vcl-1"]));
        assert!(v["compose"].get("exclude").is_none());
    }

    fn nested_exclusions(n: usize) -> String {
        let mut e = "(http://s)(A)".to_string();
        for x in ["B", "C", "D", "E", "F", "G"].iter().take(n) {
            e = format!("({e})-((http://s)({x}))");
        }
        e
    }

    #[test]
    fn three_levels_of_nesting_are_accepted() {
        let v = vs(&nested_exclusions(4));
        assert_eq!(v["contained"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn more_than_three_levels_are_rejected() {
        match err(&nested_exclusions(5)) {
            HtsError::VsInvalid(m) => assert!(m.contains("too deeply"), "{m}"),
            e => panic!("unexpected {e:?}"),
        }
    }

    #[test]
    fn missing_system_is_a_400_with_position() {
        match err("A") {
            HtsError::VsInvalid(m) => {
                assert!(
                    m.starts_with("Invalid VCL expression at position 1:"),
                    "{m}"
                );
                assert!(m.contains("code system is required"), "{m}");
            }
            e => panic!("unexpected {e:?}"),
        }
        assert!(matches!(err("p=1"), HtsError::VsInvalid(_)));
        assert!(matches!(err("(http://s)A;B"), HtsError::VsInvalid(_)));
        assert!(matches!(err("*"), HtsError::VsInvalid(_)));
    }

    #[test]
    fn unsupported_constructs_are_501_with_position() {
        for expr in [
            "(http://s)p!!<5",
            "(http://s)p?true",
            "(http://s){a,b}.p",
            "(http://s)p^http://vs/x",
            "(http://s)p^{q=1}",
            "(http://s)concept~^{a,b}",
            "(http://s)parent<<5",
        ] {
            match err(expr) {
                HtsError::NotSupported(m) => {
                    assert!(m.contains("not supported at position"), "{expr}: {m}")
                }
                e => panic!("{expr}: unexpected {e:?}"),
            }
        }
    }

    #[test]
    fn unsupported_position_points_at_the_filter() {
        match err("(http://s)(A;p?true)") {
            HtsError::NotSupported(m) => assert!(m.contains("position 14"), "{m}"),
            e => panic!("unexpected {e:?}"),
        }
    }

    #[test]
    fn output_is_deterministic() {
        let e = "((http://s)(a=1;b=2),(http://s)(A))-((http://s)(B))";
        assert_eq!(vs(e).to_string(), vs(e).to_string());
    }
}
