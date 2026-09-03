//! Tests for `helios_sof::lint` (#753 ticket 03).

use super::*;
use serde_json::json;

/// The diagnostics for `doc` whose `code` is exactly `code`.
fn diagnostics_of(diagnostics: &[Diagnostic], code: DiagnosticCode) -> Vec<&Diagnostic> {
    diagnostics.iter().filter(|d| d.code == code).collect()
}

fn errors_only(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect()
}

/// A minimal, otherwise-valid ViewDefinition — every rule's negative test
/// starts from a clone of this and breaks exactly one thing.
fn valid_doc() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                { "name": "id", "path": "getResourceKey()" },
                { "name": "family", "path": "name.family.first()" }
            ]
        }]
    })
}

// ---------------------------------------------------------------------------
// NotAViewDefinition
// ---------------------------------------------------------------------------

#[test]
fn not_a_view_definition_positive() {
    for bad in [
        json!("just a string"),
        json!(null),
        json!(42),
        json!([1, 2, 3]),
        json!({}),
        json!({ "resourceType": "Patient" }),
        json!({ "resourceType": 5 }),
    ] {
        let diagnostics = lint_view_definition(&bad);
        assert_eq!(
            diagnostics.len(),
            1,
            "expected exactly one diagnostic for {bad}, got {diagnostics:?}"
        );
        assert_eq!(diagnostics[0].code, DiagnosticCode::NotAViewDefinition);
        assert_eq!(diagnostics[0].pointer, "");
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }
}

#[test]
fn not_a_view_definition_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::NotAViewDefinition
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// UnknownKey
// ---------------------------------------------------------------------------

#[test]
fn unknown_key_positive() {
    let mut doc = valid_doc();
    doc["notAField"] = json!("oops");
    doc["select"][0]["column"][0]["notAField"] = json!("oops");

    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey);
    assert!(unknown.iter().any(|d| d.pointer == "/notAField"));
    assert!(
        unknown
            .iter()
            .any(|d| d.pointer == "/select/0/column/0/notAField")
    );
}

#[test]
fn unknown_key_negative_every_modeled_root_key_is_accepted() {
    // Every root key this module's model declares, present at once, plus
    // both the `_`-prefixed primitive-extension sibling convention and the
    // three base-resource keys — none of it should ever be UnknownKey.
    // A raw string literal, not the json! macro: this many top-level keys
    // in one object pushes serde_json's json! macro past its default
    // recursion limit.
    let doc: Value = serde_json::from_str(
        r#"{
        "resourceType": "ViewDefinition",
        "id": "vd1",
        "meta": {},
        "implicitRules": "http://example.org",
        "_implicitRules": { "extension": [] },
        "language": "en",
        "text": {},
        "contained": [],
        "extension": [],
        "modifierExtension": [],
        "url": "http://example.org/vd",
        "identifier": [],
        "version": "1",
        "versionAlgorithmString": "semver",
        "name": "my_view",
        "title": "My View",
        "status": "active",
        "_status": { "extension": [] },
        "experimental": false,
        "date": "2026-01-01",
        "publisher": "Acme",
        "contact": [],
        "description": "a view",
        "useContext": [],
        "jurisdiction": [],
        "purpose": "testing",
        "copyright": "(c) Acme",
        "copyrightLabel": "Acme",
        "approvalDate": "2026-01-01",
        "lastReviewDate": "2026-01-01",
        "effectivePeriod": {},
        "topic": [],
        "author": [],
        "editor": [],
        "reviewer": [],
        "endorser": [],
        "relatedArtifact": [],
        "resource": "Patient",
        "profile": [],
        "fhirVersion": ["4.0.1"],
        "constant": [{ "name": "x", "valueString": "hi" }],
        "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }] }],
        "where": [{ "path": "active" }]
    }"#,
    )
    .expect("valid JSON literal");
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey);
    assert!(unknown.is_empty(), "unexpected UnknownKey: {unknown:?}");
}

// ---------------------------------------------------------------------------
// MissingRequired / EmptyRequired
// ---------------------------------------------------------------------------

#[test]
fn missing_required_positive() {
    let doc = json!({ "resourceType": "ViewDefinition" });
    let diagnostics = lint_view_definition(&doc);
    let missing = diagnostics_of(&diagnostics, DiagnosticCode::MissingRequired);
    let missing_keys: Vec<&str> = missing.iter().map(|d| d.message.as_str()).collect();
    assert!(
        missing
            .iter()
            .any(|d| d.pointer.is_empty() && d.message.contains("resource")),
        "{missing_keys:?}"
    );
    assert!(
        missing
            .iter()
            .any(|d| d.pointer.is_empty() && d.message.contains("select")),
        "{missing_keys:?}"
    );
    // `status` is deliberately NOT required - see the doc comment on its
    // Field entry in lint.rs for the evidence (33/133 valid official
    // SQL-on-FHIR fixtures omit it).
    assert!(
        !missing.iter().any(|d| d.message.contains("status")),
        "{missing_keys:?}"
    );
}

#[test]
fn missing_required_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::MissingRequired
        )
        .is_empty()
    );
}

#[test]
fn empty_required_positive() {
    let mut doc = valid_doc();
    doc["resource"] = json!("   ");
    doc["select"] = json!([]);
    let diagnostics = lint_view_definition(&doc);
    let empty = diagnostics_of(&diagnostics, DiagnosticCode::EmptyRequired);
    assert!(empty.iter().any(|d| d.pointer == "/resource"));
    assert!(empty.iter().any(|d| d.pointer == "/select"));
}

#[test]
fn empty_required_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::EmptyRequired
        )
        .is_empty()
    );
}

#[test]
fn constant_missing_value_x_is_missing_required() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "x" }]);
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::MissingRequired)
            .iter()
            .any(|d| d.pointer == "/constant/0")
    );
}

// ---------------------------------------------------------------------------
// WrongType
// ---------------------------------------------------------------------------

#[test]
fn wrong_type_positive() {
    let mut doc = valid_doc();
    doc["status"] = json!(42);
    doc["select"][0]["column"] = json!("not an array");
    let diagnostics = lint_view_definition(&doc);
    let wrong = diagnostics_of(&diagnostics, DiagnosticCode::WrongType);
    assert!(wrong.iter().any(|d| d.pointer == "/status"));
    assert!(wrong.iter().any(|d| d.pointer == "/select/0/column"));
}

#[test]
fn wrong_type_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::WrongType
        )
        .is_empty()
    );
}

#[test]
fn wrong_type_stops_descent_into_the_bad_value() {
    // A `column` that is a string, not an array, must not also produce
    // UnknownKey/MissingRequired noise for a "column" that never existed as
    // an object.
    let mut doc = valid_doc();
    doc["select"][0]["column"] = json!("nope");
    let diagnostics = lint_view_definition(&doc);
    assert_eq!(
        diagnostics_of(&diagnostics, DiagnosticCode::WrongType).len(),
        1
    );
    assert!(diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey).is_empty());
}

#[test]
fn constant_value_x_present_twice_is_wrong_type() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "x", "valueString": "a", "valueBoolean": true }]);
    let diagnostics = lint_view_definition(&doc);
    let wrong = diagnostics_of(&diagnostics, DiagnosticCode::WrongType);
    assert!(wrong.iter().any(|d| d.pointer == "/constant/0"));
}

// ---------------------------------------------------------------------------
// SelectWithoutOutput
// ---------------------------------------------------------------------------

#[test]
fn select_without_output_positive() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{}]
    });
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::SelectWithoutOutput)
            .iter()
            .any(|d| d.pointer == "/select/0")
    );
}

#[test]
fn select_without_output_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::SelectWithoutOutput
        )
        .is_empty()
    );
}

#[test]
fn select_with_only_nested_select_has_output() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }] }]
        }]
    });
    assert!(
        diagnostics_of(
            &lint_view_definition(&doc),
            DiagnosticCode::SelectWithoutOutput
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// MultipleIterationDirectives
// ---------------------------------------------------------------------------

#[test]
fn multiple_iteration_directives_positive() {
    let mut doc = valid_doc();
    doc["select"][0]["forEach"] = json!("name");
    doc["select"][0]["forEachOrNull"] = json!("telecom");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::MultipleIterationDirectives)
            .iter()
            .any(|d| d.pointer == "/select/0")
    );
}

#[test]
fn multiple_iteration_directives_negative() {
    let mut doc = valid_doc();
    doc["select"][0]["forEach"] = json!("name");
    assert!(
        diagnostics_of(
            &lint_view_definition(&doc),
            DiagnosticCode::MultipleIterationDirectives
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// DuplicateColumnName
// ---------------------------------------------------------------------------

#[test]
fn duplicate_column_name_positive() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                { "name": "id", "path": "getResourceKey()" },
                { "name": "id", "path": "id" }
            ]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 1, "{dups:?}");
    assert_eq!(dups[0].pointer, "/select/0/column/1/name");
}

#[test]
fn duplicate_column_name_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::DuplicateColumnName
        )
        .is_empty()
    );
}

#[test]
fn duplicate_column_name_crosses_nested_select_but_not_union_all_branches() {
    // A name repeated across a parent select and its nested select (same
    // row) is a duplicate; the same name repeated in two different
    // unionAll branches (different rows) is not.
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey()" }],
            "select": [{
                "column": [{ "name": "id", "path": "id" }]
            }],
            "unionAll": [
                { "column": [{ "name": "kind", "path": "'a'" }] },
                { "column": [{ "name": "kind", "path": "'b'" }] }
            ]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 1, "{dups:?}");
    assert_eq!(dups[0].pointer, "/select/0/select/0/column/0/name");
}

#[test]
fn duplicate_column_name_union_all_branch_still_sees_external_columns() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey()" }],
            "unionAll": [
                { "column": [{ "name": "id", "path": "id" }] }
            ]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 1, "{dups:?}");
    assert_eq!(dups[0].pointer, "/select/0/unionAll/0/column/0/name");
}

#[test]
fn duplicate_column_name_is_case_sensitive() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                { "name": "Id", "path": "getResourceKey()" },
                { "name": "id", "path": "id" }
            ]
        }]
    });
    assert!(
        diagnostics_of(
            &lint_view_definition(&doc),
            DiagnosticCode::DuplicateColumnName
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// FhirPathSyntax
// ---------------------------------------------------------------------------

#[test]
fn fhirpath_syntax_positive() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("getResourceKey(");
    let diagnostics = lint_view_definition(&doc);
    let syntax = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax);
    assert_eq!(syntax.len(), 1, "{syntax:?}");
    assert_eq!(syntax[0].pointer, "/select/0/column/0/path");
    assert!(syntax[0].span.is_some());
}

#[test]
fn fhirpath_syntax_negative() {
    assert!(
        diagnostics_of(
            &lint_view_definition(&valid_doc()),
            DiagnosticCode::FhirPathSyntax
        )
        .is_empty()
    );
}

#[test]
fn fhirpath_syntax_covers_foreach_foreach_or_null_repeat_and_where() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "forEach": "name.",
            "column": [{ "name": "id", "path": "getResourceKey()" }]
        }],
        "where": [{ "path": "active =" }]
    });
    let diagnostics = lint_view_definition(&doc);
    let syntax = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax);
    let pointers: Vec<&str> = syntax.iter().map(|d| d.pointer.as_str()).collect();
    assert!(pointers.contains(&"/select/0/forEach"), "{pointers:?}");
    assert!(pointers.contains(&"/where/0/path"), "{pointers:?}");
}

#[test]
fn fhirpath_syntax_repeat_elements_are_each_checked() {
    let mut doc = valid_doc();
    doc["select"][0] = json!({
        "repeat": ["contact", "link.other."],
        "column": [{ "name": "id", "path": "getResourceKey()" }]
    });
    let diagnostics = lint_view_definition(&doc);
    let syntax = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax);
    assert!(
        syntax.iter().any(|d| d.pointer == "/select/0/repeat/1"),
        "{syntax:?}"
    );
    assert!(
        !syntax.iter().any(|d| d.pointer == "/select/0/repeat/0"),
        "{syntax:?}"
    );
}

#[test]
fn fhirpath_syntax_empty_expression_gets_zero_width_span() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("   ");
    let diagnostics = lint_view_definition(&doc);
    let syntax = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax);
    assert_eq!(syntax.len(), 1);
    assert_eq!(syntax[0].span, Some(Span { start: 0, end: 0 }));
    assert_eq!(syntax[0].message, "empty expression");
}

// ---------------------------------------------------------------------------
// Pointer escaping (RFC 6901) and ordering (RF1)
// ---------------------------------------------------------------------------

#[test]
fn pointer_escapes_tilde_and_slash_in_keys() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey()" }]
        }],
        "a~b/c": "unexpected"
    });
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey);
    assert!(
        unknown.iter().any(|d| d.pointer == "/a~0b~1c"),
        "{unknown:?}"
    );
}

#[test]
fn diagnostics_are_ordered_by_pointer_with_numeric_array_indices() {
    // 12 columns so a lexicographic sort would put "/select/0/column/10/..."
    // before "/select/0/column/2/...".
    let mut columns = Vec::new();
    for i in 0..12 {
        columns.push(json!({ "name": format!("dup{i}"), "path": "" }));
    }
    // Force every one of them empty (FhirPathSyntax) so there is one
    // diagnostic per column to check the order of.
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{ "column": columns }]
    });
    let diagnostics = lint_view_definition(&doc);
    let pointers: Vec<&str> = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax)
        .iter()
        .map(|d| d.pointer.as_str())
        .collect();
    let expected: Vec<String> = (0..12)
        .map(|i| format!("/select/0/column/{i}/path"))
        .collect();
    assert_eq!(pointers, expected);
}

#[test]
fn diagnostic_order_is_stable_across_runs() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("bad(");
    doc["notAKey"] = json!(1);
    let first = lint_view_definition(&doc);
    let second = lint_view_definition(&doc);
    assert_eq!(first, second);
}

#[test]
fn diagnostics_sort_by_true_document_position_not_by_pointer_text() {
    // "where" is declared BEFORE "select" in this document's actual key
    // order (unusual, but valid JSON — and exactly the shape #753's C3
    // validation caught). Sorting by the *text* of the pointer would put
    // every /select/* diagnostic first ("select" < "where"
    // lexicographically) regardless of where each one actually sits in the
    // source; sorting by true document position must put the /where/0/path
    // diagnostic first, since "where" is textually first here.
    let doc: Value = serde_json::from_str(
        r#"{
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "where": [{ "path": "active =" }],
            "select": [
                { "column": [{ "name": "id", "path": "getResourceKey()" }] },
                { "column": [{ "name": "bad", "path": "1 +" }] }
            ]
        }"#,
    )
    .expect("valid JSON literal");

    let diagnostics = lint_view_definition(&doc);
    let syntax_pointers: Vec<&str> = diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax)
        .iter()
        .map(|d| d.pointer.as_str())
        .collect();
    assert_eq!(
        syntax_pointers,
        vec!["/where/0/path", "/select/1/column/0/path"],
        "where[] precedes select[] in this document, so its diagnostic must sort first: {syntax_pointers:?}"
    );
}

#[test]
fn diagnostics_sort_a_container_diagnostic_before_its_own_children() {
    // MissingRequired's pointer is the *container* (here, constant[0]
    // itself, missing its value[x]) — it must sort before anything
    // reported inside a LATER sibling select, and before anything inside
    // ITS OWN later-declared siblings at the root, matching where the
    // container's own opening "{" sits in the text.
    let doc: Value = serde_json::from_str(
        r#"{
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "constant": [{ "name": "x" }],
            "select": [{ "column": [{ "name": "bad", "path": "1 +" }] }]
        }"#,
    )
    .expect("valid JSON literal");

    let diagnostics = lint_view_definition(&doc);
    let pointers: Vec<&str> = diagnostics.iter().map(|d| d.pointer.as_str()).collect();
    let constant_index = pointers
        .iter()
        .position(|p| *p == "/constant/0")
        .expect("MissingRequired on /constant/0");
    let select_index = pointers
        .iter()
        .position(|p| *p == "/select/0/column/0/path")
        .expect("FhirPathSyntax on /select/0/column/0/path");
    assert!(
        constant_index < select_index,
        "/constant/0 (declared first) must sort before /select/0/column/0/path: {pointers:?}"
    );
}

// ---------------------------------------------------------------------------
// Never panics (RF1)
// ---------------------------------------------------------------------------

#[test]
fn never_panics_on_arbitrary_shapes() {
    let inputs = [
        json!(null),
        json!(true),
        json!(false),
        json!(0),
        json!(-1.5),
        json!(""),
        json!("just a string"),
        json!([]),
        json!([1, "two", null, {}, []]),
        json!({}),
        json!({ "resourceType": "ViewDefinition" }),
        json!({ "resourceType": "ViewDefinition", "select": "not an array" }),
        json!({ "resourceType": "ViewDefinition", "select": [null, 1, "x", [], {}] }),
        json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [{ "column": [null, 1, "x", [], {}, { "name": 5, "path": true }] }]
        }),
        json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [{
                "select": [{
                    "select": [{
                        "select": [{ "column": [{ "name": "n", "path": "x" }] }]
                    }]
                }]
            }]
        }),
        json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "constant": [{}, { "name": 1 }, "not an object"],
            "select": []
        }),
    ];
    for input in inputs {
        // The only assertion is that this returns instead of panicking.
        let _ = lint_view_definition(&input);
    }
}

// ---------------------------------------------------------------------------
// RF2: the key model matches the generated helios_fhir::r4 structs
// ---------------------------------------------------------------------------

#[test]
fn key_model_matches_generated_structs() {
    use helios_fhir::r4::ViewDefinition;

    // Every key this module's model declares, across every node, deserializes
    // into `helios_fhir::r4::ViewDefinition` without error. This is a
    // deliberately loose check (the generated deserializer does not use
    // `deny_unknown_fields`, so it cannot by itself prove every key has a
    // matching field) — the precise cross-check is the required-field
    // non-Option assertions below, verified by hand against
    // `crates/fhir/src/r4.rs`.
    // A raw string literal, not the json! macro: this many top-level keys
    // in one object pushes serde_json's json! macro past its default
    // recursion limit (see unknown_key_negative_every_modeled_root_key_is_accepted).
    let doc: Value = serde_json::from_str(
        r#"{
        "resourceType": "ViewDefinition",
        "id": "vd1",
        "meta": {},
        "implicitRules": "http://example.org",
        "language": "en",
        "text": { "status": "generated", "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"/>" },
        "contained": [],
        "extension": [],
        "modifierExtension": [],
        "url": "http://example.org/vd",
        "identifier": [],
        "version": "1",
        "versionAlgorithmString": "semver",
        "name": "my_view",
        "title": "My View",
        "status": "active",
        "experimental": false,
        "date": "2026-01-01",
        "publisher": "Acme",
        "contact": [],
        "description": "a view",
        "useContext": [],
        "jurisdiction": [],
        "purpose": "testing",
        "copyright": "(c) Acme",
        "copyrightLabel": "Acme",
        "approvalDate": "2026-01-01",
        "lastReviewDate": "2026-01-01",
        "effectivePeriod": {},
        "topic": [],
        "author": [],
        "editor": [],
        "reviewer": [],
        "endorser": [],
        "relatedArtifact": [],
        "resource": "Patient",
        "profile": [],
        "fhirVersion": ["4.0.1"],
        "constant": [{ "name": "x", "valueString": "hi" }],
        "select": [{
            "column": [{
                "name": "id",
                "path": "getResourceKey()",
                "description": "the id",
                "collection": false,
                "type": "id",
                "tag": [{ "name": "ansi/type", "value": "varchar" }]
            }],
            "select": [],
            "forEach": "name",
            "repeat": ["contact"],
            "unionAll": []
        }],
        "where": [{ "path": "active", "description": "only active" }]
    }"#,
    )
    .expect("valid JSON literal");
    let parsed: Result<ViewDefinition, _> = serde_json::from_value(doc);
    assert!(parsed.is_ok(), "{parsed:?}");

    // The required-field / non-Option cross-check RF2 asks for.
    //
    // An earlier version of this test proved a scalar field was required by
    // removing its key and checking that deserialization failed. That
    // premise turned out to be wrong for this codebase: helios-fhir's
    // generated Deserialize deliberately defaults a missing required scalar
    // (via `Default::default()`) rather than rejecting it - which is *why*
    // `status` is modeled as optional above despite the FHIR
    // StructureDefinition marking it 1..1 (33 of 133 valid, non-error views
    // in the official SQL-on-FHIR conformance fixtures this crate vendors
    // omit it; see the Field entry's own doc comment in lint.rs).
    //
    // The proof this test relies on instead is a compile-time one: every
    // struct below derives `Default`, so constructing it with `..Default::
    // default()` plus a *bare* (non-`Some`-wrapped) value for a field this
    // module's model calls required only compiles if that field's own type
    // is genuinely non-Option. If any of `resource`, `path`, `name`, or
    // `value` below were ever regenerated as `Option<T>`, this test stops
    // compiling - a stronger guarantee than a runtime assertion gives.
    let _: ViewDefinition = ViewDefinition {
        resource: helios_fhir::r4::Code::default(),
        ..Default::default()
    };

    use helios_fhir::r4::{
        ViewDefinitionSelectColumn, ViewDefinitionSelectColumnTag, ViewDefinitionWhere,
    };
    let _: ViewDefinitionSelectColumn = ViewDefinitionSelectColumn {
        path: String::default().into(),
        name: String::default().into(),
        ..Default::default()
    };
    let _: ViewDefinitionWhere = ViewDefinitionWhere {
        path: String::default().into(),
        ..Default::default()
    };
    let _: ViewDefinitionSelectColumnTag = ViewDefinitionSelectColumnTag {
        name: String::default().into(),
        value: String::default().into(),
    };
    let _: helios_fhir::r4::ViewDefinitionConstant = helios_fhir::r4::ViewDefinitionConstant {
        name: String::default().into(),
        ..Default::default()
    };
}

// ---------------------------------------------------------------------------
// Every ViewDefinition the existing helios-sof suite already considers
// valid must lint clean of *errors* (warnings tolerated).
// ---------------------------------------------------------------------------

#[test]
fn official_sql_on_fhir_fixtures_that_are_not_error_cases_lint_clean() {
    #[derive(serde::Deserialize)]
    struct FixtureFile {
        tests: Vec<FixtureTest>,
    }
    #[derive(serde::Deserialize)]
    struct FixtureTest {
        title: String,
        view: Value,
        #[serde(rename = "expectError", default)]
        expect_error: Option<bool>,
    }

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sql-on-fhir/tests");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("fixture directory exists") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let file: FixtureFile = match serde_json::from_str(&contents) {
            Ok(file) => file,
            // A handful of upstream fixtures use a slightly different shape
            // (e.g. a top-level `view` instead of per-test `view`s); this
            // suite only needs the ones that match the common shape to get
            // solid coverage across real, spec-conformant ViewDefinitions.
            Err(_) => continue,
        };
        for test in file.tests {
            if test.expect_error == Some(true) {
                continue;
            }
            let mut doc = test.view;
            doc["resourceType"] = json!("ViewDefinition");
            let diagnostics = lint_view_definition(&doc);
            let errors = errors_only(&diagnostics);
            assert!(
                errors.is_empty(),
                "{}::{} should lint clean of errors, got {errors:?}",
                path.display(),
                test.title
            );
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "expected substantial fixture coverage, only checked {checked}"
    );
}

// ---------------------------------------------------------------------------
// Wire serialization (RF1): kebab-case severity/code, span as object or null
// ---------------------------------------------------------------------------

#[test]
fn serializes_every_diagnostic_code_as_the_documented_kebab_case_string() {
    // A literal table, not a generated one: this is exactly the wire
    // contract RF1 promises consumers (the browser, and this crate's own
    // README-level docs) - a derive's automatic kebab-casing would silently
    // rename this list on a future variant with an internal capital run
    // (see FhirPathSyntax's own doc comment for the case that bit this).
    let expected: &[(DiagnosticCode, &str)] = &[
        (DiagnosticCode::NotAViewDefinition, "not-a-view-definition"),
        (DiagnosticCode::UnknownKey, "unknown-key"),
        (DiagnosticCode::MissingRequired, "missing-required"),
        (DiagnosticCode::WrongType, "wrong-type"),
        (DiagnosticCode::EmptyRequired, "empty-required"),
        (DiagnosticCode::DuplicateColumnName, "duplicate-column-name"),
        (
            DiagnosticCode::MultipleIterationDirectives,
            "multiple-iteration-directives",
        ),
        (DiagnosticCode::SelectWithoutOutput, "select-without-output"),
        (DiagnosticCode::FhirPathSyntax, "fhirpath-syntax"),
        (DiagnosticCode::UndeclaredConstant, "undeclared-constant"),
    ];
    for (code, wire) in expected {
        let value = serde_json::to_value(code).unwrap();
        assert_eq!(value, json!(wire), "{code:?}");
    }
}

#[test]
fn serializes_severity_as_kebab_case() {
    assert_eq!(
        serde_json::to_value(Severity::Error).unwrap(),
        json!("error")
    );
    assert_eq!(
        serde_json::to_value(Severity::Warning).unwrap(),
        json!("warning")
    );
}

#[test]
fn serializes_span_as_an_object_and_absent_span_as_null() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("getResourceKey(");
    doc["notAField"] = json!(1);
    let diagnostics = lint_view_definition(&doc);
    let value = serde_json::to_value(&diagnostics).unwrap();

    let with_span = value
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "fhirpath-syntax")
        .unwrap();
    assert!(with_span["span"].is_object());
    assert!(with_span["span"]["start"].is_u64());
    assert!(with_span["span"]["end"].is_u64());

    let without_span = value
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "unknown-key")
        .unwrap();
    assert_eq!(without_span["span"], serde_json::Value::Null);
}

#[test]
fn diagnostic_fields_are_camel_case() {
    let doc = json!({ "resourceType": "Patient" });
    let value = serde_json::to_value(lint_view_definition(&doc)).unwrap();
    let diagnostic = &value.as_array().unwrap()[0];
    for key in ["pointer", "message", "severity", "code", "span"] {
        assert!(
            diagnostic.get(key).is_some(),
            "missing `{key}` in {diagnostic}"
        );
    }
}
