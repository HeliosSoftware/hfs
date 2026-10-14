//! # Static Semantic Checks with Type Guards
//!
//! Checks an expression against the *declared* types of its focus, before any
//! data is seen. The FHIRPath conformance suite (`isSemanticChecks` group in
//! `fhir-test-cases`) exercises this through the `contextType` test attribute:
//! the focus is declared as, say, `Period|Timing`, and the expression is
//! semantically invalid if it can reach an operation none of those types
//! support, even though the example data (a `dateTime`) would evaluate fine.
//!
//! The interesting part is **type guards**. A type test (`X is T`, `X.is(T)`,
//! `(X is T).not()`, and `and`/`or` combinations of those) used by a logical
//! operator narrows the types the other operand is checked against:
//!
//! - `A implies B`, `A and B`, `iif(A, B, ..)`: `B` is checked assuming `A` is true
//! - `A or B`, `iif(A, .., B)`: `B` is checked assuming `A` is false
//!
//! When the narrowed set is empty the guarded operand is unreachable and is not
//! checked at all. `xor` evaluates both operands, so it never narrows.
//!
//! The checker is deliberately conservative: a type it cannot work out is
//! "unknown", and nothing is ever reported against an unknown type. Today the
//! only operation it checks is `toString()`, which the specification defines
//! for primitive values (and Quantity) only.

use crate::fhir_type_hierarchy::{is_fhir_primitive_type_for_version, is_fhir_subtype_for_version};
use crate::parser::{Expression, Invocation, Term, TypeSpecifier};
use helios_fhir::FhirVersion;

/// The static types an expression may have.
///
/// `None` means unknown (no checks apply). `Some(vec![])` means no type is
/// possible: the expression is unreachable under the current type guards.
type Types = Option<Vec<String>>;

/// Parses a `contextType` declaration such as `Period|Timing` into a type set.
///
/// A `FHIR.` prefix is accepted and dropped; blank entries are ignored.
pub fn parse_context_type(context_type: &str) -> Vec<String> {
    context_type
        .split('|')
        .map(|t| t.trim())
        .map(|t| t.strip_prefix("FHIR.").unwrap_or(t))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Checks `expression` assuming its focus (`$this`) is one of `focus_types`.
///
/// Returns a human-readable description of the first semantic error found.
pub fn check_semantics(
    expression: &Expression,
    focus_types: &[String],
    fhir_version: FhirVersion,
) -> Result<(), String> {
    let checker = Checker { fhir_version };
    checker.check(expression, &Some(focus_types.to_vec()))?;
    Ok(())
}

struct Checker {
    fhir_version: FhirVersion,
}

impl Checker {
    /// Checks `expr` with `$this` typed as `this`, returning the result's types.
    fn check(&self, expr: &Expression, this: &Types) -> Result<Types, String> {
        match expr {
            Expression::Term(term) => self.check_term(term, this),
            Expression::Invocation(base, invocation) => {
                let focus = self.check(base, this)?;
                match invocation {
                    Invocation::Function(name, args) => {
                        self.check_function(&focus, name, args, &None, false)
                    }
                    Invocation::This => Ok(focus),
                    _ => Ok(None),
                }
            }
            Expression::Indexer(base, index) => {
                let focus = self.check(base, this)?;
                self.check(index, this)?;
                Ok(focus)
            }
            Expression::Polarity(_, inner) => {
                self.check(inner, this)?;
                Ok(None)
            }
            Expression::Type(inner, op, type_spec) => {
                self.check(inner, this)?;
                if op == "as" {
                    Ok(Some(vec![type_spec_name(type_spec)]))
                } else {
                    Ok(None)
                }
            }
            Expression::And(left, right) => {
                self.check(left, this)?;
                self.check_guarded(right, &self.narrow(left, true, this))?;
                Ok(None)
            }
            Expression::Or(left, op, right) => {
                self.check(left, this)?;
                if op == "or" {
                    self.check_guarded(right, &self.narrow(left, false, this))?;
                } else {
                    // xor always evaluates both operands, so nothing is guarded.
                    self.check(right, this)?;
                }
                Ok(None)
            }
            Expression::Implies(left, right) => {
                self.check(left, this)?;
                self.check_guarded(right, &self.narrow(left, true, this))?;
                Ok(None)
            }
            Expression::Multiplicative(left, _, right)
            | Expression::Additive(left, _, right)
            | Expression::Union(left, right)
            | Expression::Inequality(left, _, right)
            | Expression::Equality(left, _, right)
            | Expression::Membership(left, _, right) => {
                self.check(left, this)?;
                self.check(right, this)?;
                Ok(None)
            }
            Expression::Lambda(_, body) => {
                self.check(body, &None)?;
                Ok(None)
            }
            Expression::InstanceSelector(_, fields) => {
                for (_, value) in fields {
                    self.check(value, this)?;
                }
                Ok(None)
            }
        }
    }

    /// Checks an operand that only runs when its guard allows it.
    fn check_guarded(&self, expr: &Expression, this: &Types) -> Result<(), String> {
        if matches!(this, Some(types) if types.is_empty()) {
            // No type survives the guard, so this operand can never run.
            return Ok(());
        }
        self.check(expr, this)?;
        Ok(())
    }

    fn check_term(&self, term: &Term, this: &Types) -> Result<Types, String> {
        match term {
            Term::Invocation(Invocation::This) => Ok(this.clone()),
            // A function with no explicit focus is invoked on `$this`.
            Term::Invocation(Invocation::Function(name, args)) => {
                self.check_function(this, name, args, this, true)
            }
            Term::Parenthesized(inner) => self.check(inner, this),
            _ => Ok(None),
        }
    }

    /// Checks a function call on `focus`.
    ///
    /// `this` and `on_this` describe the scope the call sits in: guards in
    /// `iif()` arguments only narrow `$this` when `iif()` is invoked on it
    /// implicitly, because otherwise the arguments see a different focus.
    fn check_function(
        &self,
        focus: &Types,
        name: &str,
        args: &[Expression],
        this: &Types,
        on_this: bool,
    ) -> Result<Types, String> {
        match name {
            "iif" if on_this && !args.is_empty() => {
                self.check(&args[0], this)?;
                if let Some(then_branch) = args.get(1) {
                    self.check_guarded(then_branch, &self.narrow(&args[0], true, this))?;
                }
                if let Some(else_branch) = args.get(2) {
                    self.check_guarded(else_branch, &self.narrow(&args[0], false, this))?;
                }
                return Ok(None);
            }
            // Lambda-style functions evaluate their argument once per input
            // item, so `$this` inside it ranges over the focus types.
            "where" | "select" | "all" | "exists" | "repeat" | "repeatAll" => {
                for arg in args {
                    self.check(arg, focus)?;
                }
            }
            _ => {
                for arg in args {
                    self.check(arg, &None)?;
                }
            }
        }

        match name {
            "toString" => {
                self.check_to_string(focus)?;
                Ok(None)
            }
            "ofType" | "as" => Ok(args.first().and_then(type_name_from_arg).map(|t| vec![t])),
            // Filtering and subsetting functions return items of the input.
            "where" | "first" | "last" | "single" | "tail" | "skip" | "take" | "distinct"
            | "intersect" | "exclude" | "trace" => Ok(focus.clone()),
            _ => Ok(None),
        }
    }

    fn check_to_string(&self, focus: &Types) -> Result<(), String> {
        let Some(types) = focus else {
            return Ok(());
        };
        if types.is_empty() || types.iter().any(|t| self.supports_to_string(t)) {
            return Ok(());
        }
        Err(format!(
            "toString() is not defined for {} (it applies to primitive values only)",
            describe(types)
        ))
    }

    /// `toString()` converts primitives and Quantity. Complex types and
    /// resources have no string representation. A name the type hierarchy
    /// does not know is given the benefit of the doubt.
    fn supports_to_string(&self, type_name: &str) -> bool {
        if let Some(system) = type_name.strip_prefix("System.") {
            return !system.eq_ignore_ascii_case("Any");
        }
        if is_fhir_primitive_type_for_version(type_name, &self.fhir_version)
            || self.is_subtype(type_name, "Quantity")
        {
            return true;
        }
        let complex =
            self.is_subtype(type_name, "Element") || self.is_subtype(type_name, "Resource");
        !complex
    }

    /// Narrows `types` (the types of `$this`) given that `cond` evaluated to
    /// `assume`. A condition that is not a type test on `$this` narrows nothing.
    fn narrow(&self, cond: &Expression, assume: bool, types: &Types) -> Types {
        let Some(current) = types else {
            return None;
        };
        match cond {
            Expression::Term(Term::Parenthesized(inner)) => self.narrow(inner, assume, types),
            Expression::Type(subject, op, type_spec) if op == "is" && is_this(subject) => {
                Some(self.filter_is(current, &type_spec_name(type_spec), assume))
            }
            // `$this.is(T)` and the implicit-focus form `is(T)`.
            Expression::Invocation(subject, Invocation::Function(name, args))
                if name == "is" && is_this(subject) =>
            {
                match args.first().and_then(type_name_from_arg) {
                    Some(t) => Some(self.filter_is(current, &t, assume)),
                    None => types.clone(),
                }
            }
            Expression::Term(Term::Invocation(Invocation::Function(name, args)))
                if name == "is" =>
            {
                match args.first().and_then(type_name_from_arg) {
                    Some(t) => Some(self.filter_is(current, &t, assume)),
                    None => types.clone(),
                }
            }
            Expression::Invocation(inner, Invocation::Function(name, args))
                if name == "not" && args.is_empty() =>
            {
                self.narrow(inner, !assume, types)
            }
            Expression::And(left, right) => {
                if assume {
                    // Both are true.
                    self.narrow(right, true, &self.narrow(left, true, types))
                } else {
                    // At least one is false.
                    union(
                        self.narrow(left, false, types),
                        self.narrow(right, false, types),
                    )
                }
            }
            Expression::Or(left, op, right) if op == "or" => {
                if assume {
                    // At least one is true.
                    union(
                        self.narrow(left, true, types),
                        self.narrow(right, true, types),
                    )
                } else {
                    // Both are false.
                    self.narrow(right, false, &self.narrow(left, false, types))
                }
            }
            _ => types.clone(),
        }
    }

    /// The types that remain once `$this is target` is known to be `assume`.
    fn filter_is(&self, types: &[String], target: &str, assume: bool) -> Vec<String> {
        if !assume {
            return types
                .iter()
                .filter(|t| !self.is_subtype(t, target))
                .cloned()
                .collect();
        }
        let mut narrowed: Vec<String> = types
            .iter()
            .filter(|t| self.is_subtype(t, target))
            .cloned()
            .collect();
        // Testing for a specialisation of a declared type narrows to it,
        // e.g. a Quantity focus tested with `is Age` is an Age.
        if !narrowed.iter().any(|t| t == target) && types.iter().any(|t| self.is_subtype(target, t))
        {
            narrowed.push(target.to_string());
        }
        narrowed
    }

    fn is_subtype(&self, child: &str, parent: &str) -> bool {
        is_fhir_subtype_for_version(child, parent, &self.fhir_version)
    }
}

/// True when `expr` is `$this`, possibly parenthesized.
fn is_this(expr: &Expression) -> bool {
    match expr {
        Expression::Term(Term::Invocation(Invocation::This)) => true,
        Expression::Term(Term::Parenthesized(inner)) => is_this(inner),
        _ => false,
    }
}

/// The type name from a type specifier, without a `FHIR.` namespace.
fn type_spec_name(spec: &TypeSpecifier) -> String {
    let TypeSpecifier::QualifiedIdentifier(first, second) = spec;
    match second {
        Some(name) if first == "FHIR" => name.clone(),
        Some(name) => format!("{first}.{name}"),
        None => first.clone(),
    }
}

/// The type name from a function argument such as `ofType(dateTime)` or
/// `is(FHIR.Period)`, which the parser reads as member access.
fn type_name_from_arg(arg: &Expression) -> Option<String> {
    match arg {
        Expression::Term(Term::Invocation(Invocation::Member(name))) => Some(name.clone()),
        Expression::Invocation(base, Invocation::Member(name)) => match base.as_ref() {
            Expression::Term(Term::Invocation(Invocation::Member(ns))) if ns == "FHIR" => {
                Some(name.clone())
            }
            Expression::Term(Term::Invocation(Invocation::Member(ns))) => {
                Some(format!("{ns}.{name}"))
            }
            _ => None,
        },
        _ => None,
    }
}

fn union(a: Types, b: Types) -> Types {
    let (Some(mut a), Some(b)) = (a, b) else {
        return None;
    };
    for t in b {
        if !a.contains(&t) {
            a.push(t);
        }
    }
    Some(a)
}

fn describe(types: &[String]) -> String {
    match types {
        [single] => format!("type {single}"),
        _ => format!("types {}", types.join(" | ")),
    }
}

#[cfg(all(test, feature = "R4"))]
mod tests {
    use super::*;
    use crate::parse_expression;

    fn check(expr: &str, context_type: &str) -> Result<(), String> {
        let parsed = parse_expression(expr).expect("expression parses");
        check_semantics(&parsed, &parse_context_type(context_type), FhirVersion::R4)
    }

    #[test]
    fn to_string_on_complex_type_is_rejected() {
        assert!(check("$this.toString()", "Period").is_err());
        assert!(check("toString()", "Period|Timing").is_err());
        assert!(check("$this.toString()", "dateTime").is_ok());
        assert!(check("$this.toString()", "Period|dateTime").is_ok());
        assert!(check("$this.toString()", "Quantity").is_ok());
        assert!(check("$this.toString()", "Age").is_ok());
    }

    #[test]
    fn implies_and_and_check_the_right_operand_assuming_the_left_is_true() {
        assert!(
            check(
                "$this is dateTime implies $this.toString().length() >= 10",
                "Period"
            )
            .is_ok()
        );
        assert!(check("$this.is(dateTime) implies toString().exists()", "Period").is_ok());
        assert!(
            check(
                "$this is Period implies $this.toString().exists()",
                "Period|Timing"
            )
            .is_err()
        );
        assert!(check("$this is dateTime and $this.toString().exists()", "Period").is_ok());
        assert!(
            check(
                "$this is Period and $this.toString().exists()",
                "Period|Timing"
            )
            .is_err()
        );
    }

    #[test]
    fn or_checks_the_right_operand_assuming_the_left_is_false() {
        assert!(check("$this is Period or $this.toString().exists()", "Period").is_ok());
        assert!(
            check(
                "$this is Period or $this.toString().exists()",
                "Period|dateTime"
            )
            .is_ok()
        );
        assert!(
            check(
                "$this is Period or $this.toString().exists()",
                "Period|Timing"
            )
            .is_err()
        );
        assert!(
            check(
                "$this is Period or $this is Timing or $this.toString().exists()",
                "Period|Timing"
            )
            .is_ok()
        );
    }

    #[test]
    fn xor_never_narrows() {
        assert!(check("$this is dateTime xor $this.toString().exists()", "Period").is_err());
        assert!(
            check(
                "$this is dateTime xor $this.toString().exists()",
                "Period|dateTime"
            )
            .is_ok()
        );
    }

    #[test]
    fn combined_and_negated_guards() {
        let ok = |e: &str, t: &str| assert!(check(e, t).is_ok(), "{e} with {t} should pass");
        let bad = |e: &str, t: &str| assert!(check(e, t).is_err(), "{e} with {t} should fail");
        ok(
            "($this is dateTime or $this is instant) implies $this.toString().exists()",
            "Period",
        );
        bad(
            "($this is dateTime or $this is Period) implies $this.toString().exists()",
            "Period",
        );
        bad(
            "($this.is(dateTime) or $this.is(Period)) implies $this.toString().exists()",
            "Period",
        );
        ok(
            "($this is dateTime and %resource.status = 'final') implies $this.toString().exists()",
            "Period",
        );
        ok(
            "(%resource.status = 'final' and $this is dateTime) implies $this.toString().exists()",
            "Period",
        );
        bad(
            "($this is dateTime or %resource.status = 'final') implies $this.toString().exists()",
            "Period",
        );
        bad(
            "$this is dateTime or %resource.status = 'final' implies $this.toString().exists()",
            "Period",
        );
        ok(
            "$this is Period or (%resource.status = 'final' implies $this.toString().exists())",
            "Period",
        );
        bad(
            "$this is Period or (%resource.status = 'final' implies $this.toString().exists())",
            "Period|Timing",
        );
        ok(
            "($this is Period).not() implies $this.toString().exists()",
            "Period",
        );
        bad(
            "($this is Period).not() implies $this.toString().exists()",
            "Period|Timing",
        );
        ok(
            "$this is Period implies ($this is dateTime implies $this.toString().exists())",
            "Period|Timing",
        );
    }

    #[test]
    fn iif_narrows_its_branches() {
        assert!(
            check(
                "iif($this is dateTime, $this.toString().exists(), true)",
                "Period"
            )
            .is_ok()
        );
        assert!(
            check(
                "iif($this is Period, true, $this.toString().exists())",
                "Period"
            )
            .is_ok()
        );
        assert!(
            check(
                "iif($this is Period, $this.toString().exists(), true)",
                "Period"
            )
            .is_err()
        );
    }

    #[test]
    fn only_type_tests_on_this_are_guards() {
        // Element is a supertype of Period, so the guard keeps Period.
        assert!(
            check(
                "$this is Element implies $this.toString().exists()",
                "Period"
            )
            .is_err()
        );
        // A type test on something other than $this says nothing about $this.
        assert!(
            check(
                "$this.start is dateTime implies $this.toString().exists()",
                "Period"
            )
            .is_err()
        );
    }

    #[test]
    fn of_type_and_as_produce_the_named_type() {
        assert!(check("$this.ofType(dateTime).toString().exists()", "Period").is_ok());
        assert!(check("($this as dateTime).toString().exists()", "Period").is_ok());
        assert!(check("$this.ofType(Period).toString().exists()", "dateTime").is_err());
    }

    #[test]
    fn unknown_types_are_never_reported() {
        assert!(check("name.toString()", "Period").is_ok());
        assert!(check("$this.toString()", "NotARealType").is_ok());
        assert!(check("$this.where(true).toString()", "Period").is_err());
    }
}
