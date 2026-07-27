//! Runner for the vendored upstream FHIR Schema conformance suite
//! (`tests/fixtures/upstream/*.json` — see UPSTREAM.md there for provenance).

mod common;

fn run_upstream(name: &str) {
    common::run_fixture_file("upstream", name)
}

#[test]
fn upstream_1_elements() {
    run_upstream("1_elements.json");
}

#[test]
fn upstream_2_base() {
    run_upstream("2_base.json");
}

#[test]
fn upstream_3_choices() {
    run_upstream("3_choices.json");
}

#[test]
fn upstream_4_required() {
    run_upstream("4_required.json");
}

#[test]
fn upstream_5_slices() {
    run_upstream("5_slices.json");
}

#[test]
fn upstream_6_extensions() {
    run_upstream("6_extensions.json");
}

#[test]
fn upstream_7_bundles() {
    run_upstream("7_bundles.json");
}
