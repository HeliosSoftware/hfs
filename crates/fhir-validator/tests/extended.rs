//! Runner for the Helios extended fixture suite
//! (`tests/fixtures/extended/*.json`).
//!
//! These fixtures pin behavior the upstream conformance suite leaves
//! unspecified: `excluded`, numeric array cardinality messages, fixed/pattern
//! messages, slicing rules (closed/openAtEnd/ordered/@default, prohibited
//! `max: 0` slices), primitive-extension sidecars (`_field`),
//! `elementReference` recursion, and required-satisfied-by-choice-branch.
//! Same fixture format and exact-match contract as the upstream suite.

mod common;

fn run_extended(name: &str) {
    common::run_fixture_file("extended", name)
}

#[test]
fn extended_excluded() {
    run_extended("excluded.json");
}

#[test]
fn extended_cardinality() {
    run_extended("cardinality.json");
}

#[test]
fn extended_fixed_pattern() {
    run_extended("fixed_pattern.json");
}

#[test]
fn extended_choice_required() {
    run_extended("choice_required.json");
}

#[test]
fn extended_slicing_rules() {
    run_extended("slicing_rules.json");
}

#[test]
fn extended_sidecars() {
    run_extended("sidecars.json");
}

#[test]
fn extended_element_reference() {
    run_extended("element_reference.json");
}

#[test]
fn extended_primitives() {
    run_extended("primitives.json");
}
