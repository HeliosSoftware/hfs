//! Normalization + bucketing for the differential harness (issue #427).
//!
//! This is the **comparability layer** the issue flags as "where this task can
//! quietly become expensive". It is deliberately its own file with its own
//! fast, Java-free unit tests, so the mapping is proven before any JVM is
//! provisioned. `differential.rs` `use`s it via `#[path]`.
//!
//! # Why a coarse structural comparison
//!
//! Our engine ([`Validator::validate_sync`]) is **structural only**: cardinality,
//! unknown elements, JSON type classes, required/excluded, fixed/pattern,
//! primitive regex, slicing. It does **not** evaluate FHIRPath invariants or
//! terminology bindings in this sweep (see `spec_examples.rs`). The HL7
//! reference validator (`validator_cli.jar`) runs the **full** stack.
//!
//! A naive issue-for-issue diff would therefore be swamped by `invariant` and
//! terminology (`code-invalid`, `not-found`) findings that we never compute —
//! and if those landed in the "only they flag it" bucket they would masquerade
//! as hundreds of engine false negatives that are really just out of scope.
//!
//! So we compare at **(index-free path, structural class)** granularity, and we
//! **count-but-exclude** the non-structural classes into a separate tally that
//! is always reported, never silently dropped (Persona 4's requirement).
//!
//! # The three buckets (issue #427)
//!
//! | Bucket        | Meaning                          |
//! |---------------|----------------------------------|
//! | `Both`        | genuinely-invalid published example |
//! | `OnlyOurs`    | our false positive (the #424/#425 class) |
//! | `OnlyTheirs`  | our **false negative** — the highest-value discovery |
//!
//! `OnlyTheirs` is restricted to *structural* reference findings; terminology
//! and invariant findings are surfaced under `out_of_scope_theirs` instead.

use std::collections::BTreeSet;

/// A coarse structural class shared by both validators' issue vocabularies.
///
/// Reference-validator terminology/invariant issues map to
/// [`StructuralClass::OutOfScope`] and are excluded from the false-negative
/// bucket (but counted — see [`Diff::out_of_scope_theirs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StructuralClass {
    /// Cardinality: min/max, not-array/not-singular, slice cardinality.
    Cardinality,
    /// A required element is absent, or an excluded one present.
    Presence,
    /// Unknown element, wrong JSON container type, unresolvable schema.
    Structure,
    /// fixed/pattern/primitive/choice value constraints.
    Value,
    /// Slice matching/order (closed/openAtEnd/ordered).
    Slicing,
    /// FHIRPath invariant or terminology binding — **not** computed by our
    /// structural sweep, so never a false negative against it.
    OutOfScope,
}

impl StructuralClass {
    /// Structural classes participate in bucketing; [`Self::OutOfScope`] does not.
    pub fn is_structural(self) -> bool {
        !matches!(self, StructuralClass::OutOfScope)
    }
}

/// Map one of *our* engine `ErrorKind` strings (kebab-case, as serialized in the
/// baseline and by [`errors::ErrorKind`]) to a coarse class.
///
/// Unknown strings map to [`StructuralClass::Structure`] rather than panicking:
/// a new engine kind must not make the harness crash mid-sweep, and "structure"
/// is the safe conservative bucket (it keeps the finding *in* the comparison).
pub fn class_of_ours(kind: &str) -> StructuralClass {
    match kind {
        "min" | "max" | "not-array" | "not-singular" | "slice-cardinality" => {
            StructuralClass::Cardinality
        }
        "required" | "excluded" => StructuralClass::Presence,
        "unknown-element" | "type" | "unknown-schema" => StructuralClass::Structure,
        "fixed-value" | "pattern-value" | "primitive-value" | "choice" | "choice-excluded" => {
            StructuralClass::Value
        }
        "slice-unmatched" | "slice-order" => StructuralClass::Slicing,
        // Deferred effects — never produced by `validate_sync`, but classed
        // out-of-scope defensively so a future wiring change cannot silently
        // start counting them as structural.
        "fhirpath-constraint" | "terminology-binding" | "unknown-profile" => {
            StructuralClass::OutOfScope
        }
        _ => StructuralClass::Structure,
    }
}

/// Map a reference-validator `OperationOutcome.issue.code` (FHIR `issue-type`)
/// to the same coarse class.
///
/// The reference validator's `code` vocabulary is broader and blunter than our
/// `ErrorKind`; several of its codes (`value`, `structure`) span more than one
/// of our classes. We therefore compare primarily on **path**, using the class
/// only to route terminology/invariant issues out of the false-negative bucket.
pub fn class_of_reference(code: &str) -> StructuralClass {
    match code {
        // Terminology + invariants: the reason for the whole out-of-scope tally.
        "code-invalid" | "not-found" | "invalid-code" | "invariant" | "business-rule" => {
            StructuralClass::OutOfScope
        }
        "required" => StructuralClass::Presence,
        "structure" | "invalid" | "unknown" => StructuralClass::Structure,
        "value" => StructuralClass::Value,
        "too-long" | "too-costly" => StructuralClass::Cardinality,
        // Anything the reference validator classes as informational/processing
        // that still carries a structural path defaults to Structure so it stays
        // visible in the diff rather than vanishing.
        _ => StructuralClass::Structure,
    }
}

/// Reference-validator issue severities we treat as *findings*. Warnings and
/// information are recorded but not bucketed as errors, matching the issue's
/// "(path, rough-severity)" granularity.
pub fn reference_severity_is_error(severity: &str) -> bool {
    matches!(severity, "fatal" | "error")
}

/// Normalize a path for cross-validator matching by stripping array indices.
///
/// Our engine emits `Patient.name.0.family`; the reference validator emits
/// FHIRPath `Patient.name[0].family`. Both collapse to `Patient.name.family`.
/// Index representation is a known, documented coarsening: two issues on
/// different array elements of the same element path match. That is the correct
/// trade for a first-pass adjudication signal — precise index alignment is a
/// phase-2 refinement once the numbers justify it.
pub fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // `[0]` FHIRPath index — skip to the closing bracket.
            '[' => {
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                }
            }
            // `.0.` dotted numeric index — skip a run of digits that forms a
            // whole segment (between dots), collapsing the surrounding dots.
            '.' => {
                // Peek: is the next segment all digits?
                let mut digits = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        digits.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if digits.is_empty() {
                    out.push('.');
                } else {
                    // Numeric segment: drop it. If a non-dot follows without a
                    // separating dot (shouldn't happen in these paths), keep a
                    // dot to avoid gluing identifiers together.
                    match chars.peek() {
                        Some('.') | None => { /* the next '.' (or end) is the separator */ }
                        Some(_) => out.push('.'),
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A normalized error finding: (structural class, index-free path). Severity is
/// pre-filtered to errors before a finding is constructed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub class: StructuralClass,
    pub path: String,
}

/// The comparison outcome for a single resource file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Flagged (structurally) by both — a genuinely-invalid example.
    pub both: BTreeSet<String>,
    /// Flagged by us only — false-positive candidate (the #424/#425 class).
    pub only_ours: BTreeSet<String>,
    /// Flagged structurally by the reference validator only — **false-negative
    /// candidate**, the highest-value discovery.
    pub only_theirs: BTreeSet<String>,
    /// Reference findings routed out of the false-negative bucket because they
    /// are terminology/invariant — counted, never silently dropped.
    pub out_of_scope_theirs: BTreeSet<String>,
}

/// Render a finding as a stable `class@path` key for the set-based diff.
fn key(f: &Finding) -> String {
    format!("{:?}@{}", f.class, f.path)
}

/// Diff one file's findings into the three buckets plus the out-of-scope tally.
///
/// `ours` and `theirs` are the already-normalized, error-severity findings from
/// each validator for the same resource.
pub fn diff_file(ours: &[Finding], theirs: &[Finding]) -> Diff {
    let ours_struct: BTreeSet<String> = ours
        .iter()
        .filter(|f| f.class.is_structural())
        .map(key)
        .collect();

    let mut theirs_struct: BTreeSet<String> = BTreeSet::new();
    let mut out_of_scope: BTreeSet<String> = BTreeSet::new();
    for f in theirs {
        if f.class.is_structural() {
            theirs_struct.insert(key(f));
        } else {
            out_of_scope.insert(key(f));
        }
    }

    Diff {
        both: ours_struct.intersection(&theirs_struct).cloned().collect(),
        only_ours: ours_struct.difference(&theirs_struct).cloned().collect(),
        only_theirs: theirs_struct.difference(&ours_struct).cloned().collect(),
        out_of_scope_theirs: out_of_scope,
    }
}

// ---------------------------------------------------------------------------
// Fast, Java-free unit tests — prove the comparability layer before any JVM.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_kinds_map_to_expected_classes() {
        assert_eq!(class_of_ours("required"), StructuralClass::Presence);
        assert_eq!(class_of_ours("min"), StructuralClass::Cardinality);
        assert_eq!(class_of_ours("unknown-element"), StructuralClass::Structure);
        assert_eq!(class_of_ours("primitive-value"), StructuralClass::Value);
        assert_eq!(class_of_ours("slice-order"), StructuralClass::Slicing);
        // Deferred effects are out of scope even though validate_sync never emits them.
        assert_eq!(
            class_of_ours("terminology-binding"),
            StructuralClass::OutOfScope
        );
        assert_eq!(
            class_of_ours("fhirpath-constraint"),
            StructuralClass::OutOfScope
        );
        // Unknown kind stays IN the comparison (Structure), never panics.
        assert_eq!(
            class_of_ours("some-future-kind"),
            StructuralClass::Structure
        );
    }

    #[test]
    fn reference_codes_route_terminology_and_invariants_out_of_scope() {
        assert_eq!(
            class_of_reference("code-invalid"),
            StructuralClass::OutOfScope
        );
        assert_eq!(class_of_reference("not-found"), StructuralClass::OutOfScope);
        assert_eq!(class_of_reference("invariant"), StructuralClass::OutOfScope);
        assert!(!StructuralClass::OutOfScope.is_structural());
        assert_eq!(class_of_reference("required"), StructuralClass::Presence);
        assert_eq!(class_of_reference("structure"), StructuralClass::Structure);
        assert!(StructuralClass::Structure.is_structural());
    }

    #[test]
    fn only_error_severities_are_findings() {
        assert!(reference_severity_is_error("error"));
        assert!(reference_severity_is_error("fatal"));
        assert!(!reference_severity_is_error("warning"));
        assert!(!reference_severity_is_error("information"));
    }

    #[test]
    fn path_normalization_collapses_both_index_styles() {
        assert_eq!(
            normalize_path("Patient.name.0.family"),
            "Patient.name.family"
        );
        assert_eq!(
            normalize_path("Patient.name[0].family"),
            "Patient.name.family"
        );
        assert_eq!(
            normalize_path("Bundle.entry.12.resource.name.3.given"),
            "Bundle.entry.resource.name.given"
        );
        // No indices: unchanged.
        assert_eq!(normalize_path("Patient.gender"), "Patient.gender");
        // Trailing index.
        assert_eq!(normalize_path("Patient.name[2]"), "Patient.name");
    }

    fn f(class: StructuralClass, path: &str) -> Finding {
        Finding {
            class,
            path: path.to_string(),
        }
    }

    #[test]
    fn diff_buckets_a_false_negative() {
        // We are silent; they flag a structural required-element issue.
        let ours: Vec<Finding> = vec![];
        let theirs = vec![f(StructuralClass::Presence, "Patient.name")];
        let d = diff_file(&ours, &theirs);
        assert!(d.both.is_empty());
        assert!(d.only_ours.is_empty());
        assert_eq!(
            d.only_theirs.len(),
            1,
            "structural-only-theirs is a false negative"
        );
        assert!(d.out_of_scope_theirs.is_empty());
    }

    #[test]
    fn diff_excludes_terminology_from_false_negatives() {
        // They flag a terminology issue and an invariant; neither is a false
        // negative against a structural sweep, but both are counted.
        let ours: Vec<Finding> = vec![];
        let theirs = vec![
            f(StructuralClass::OutOfScope, "Observation.code"),
            f(StructuralClass::OutOfScope, "Observation"),
        ];
        let d = diff_file(&ours, &theirs);
        assert!(
            d.only_theirs.is_empty(),
            "terminology/invariant must not be false negatives"
        );
        assert_eq!(
            d.out_of_scope_theirs.len(),
            2,
            "but they are counted, not dropped"
        );
    }

    #[test]
    fn diff_buckets_both_and_false_positive() {
        // Same structural issue on Patient.name (both) + a value issue only we
        // raise (false positive) + a matching value issue on a different index.
        let ours = vec![
            f(StructuralClass::Presence, "Patient.name"),
            f(StructuralClass::Value, "Patient.gender"),
        ];
        let theirs = vec![f(StructuralClass::Presence, "Patient.name")];
        let d = diff_file(&ours, &theirs);
        assert_eq!(d.both.len(), 1);
        assert!(d.both.iter().next().unwrap().contains("Patient.name"));
        assert_eq!(d.only_ours.len(), 1);
        assert!(
            d.only_ours
                .iter()
                .next()
                .unwrap()
                .contains("Patient.gender")
        );
        assert!(d.only_theirs.is_empty());
    }

    #[test]
    fn same_element_different_index_matches_after_normalization() {
        // Our finding on name[0], theirs on name[1]: index-free paths match, so
        // this is `both`, not a spurious pair of only-ours + only-theirs.
        let ours = vec![Finding {
            class: StructuralClass::Presence,
            path: normalize_path("Patient.name.0.family"),
        }];
        let theirs = vec![Finding {
            class: StructuralClass::Presence,
            path: normalize_path("Patient.name[1].family"),
        }];
        let d = diff_file(&ours, &theirs);
        assert_eq!(d.both.len(), 1);
        assert!(d.only_ours.is_empty());
        assert!(d.only_theirs.is_empty());
    }
}
