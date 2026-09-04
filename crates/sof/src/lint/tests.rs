//! Tests for `helios_sof::lint` (#753, #820, #821).

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
// UndeclaredConstant (#821)
// ---------------------------------------------------------------------------

#[test]
fn undeclared_constant_in_column_path() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("%notDeclared");
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
    assert_eq!(undeclared[0].pointer, "/select/0/column/0/path");
    assert_eq!(undeclared[0].message, "undeclared constant `%notDeclared`");
    assert_eq!(undeclared[0].severity, Severity::Error);
    assert_eq!(
        undeclared[0].span,
        Some(Span {
            start: 0,
            end: "%notDeclared".chars().count()
        })
    );
}

#[test]
fn undeclared_constant_in_foreach() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "forEach": "name.where(use = %notDeclared)",
            "column": [{ "name": "id", "path": "getResourceKey()" }]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
    assert_eq!(undeclared[0].pointer, "/select/0/forEach");
}

#[test]
fn undeclared_constant_in_root_where_path() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{ "name": "id", "path": "getResourceKey()" }]
        }],
        "where": [{ "path": "active = %notDeclared" }]
    });
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
    assert_eq!(undeclared[0].pointer, "/where/0/path");
}

#[test]
fn undeclared_constant_in_repeat_element() {
    let mut doc = valid_doc();
    doc["select"][0] = json!({
        "repeat": ["contact", "link.where(active = %notDeclared)"],
        "column": [{ "name": "id", "path": "getResourceKey()" }]
    });
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
    assert_eq!(undeclared[0].pointer, "/select/0/repeat/1");
}

#[test]
fn undeclared_constant_in_nested_select_and_union_all() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "select": [{
                "column": [{ "name": "n", "path": "%nestedUndeclared" }]
            }],
            "unionAll": [{
                "column": [{ "name": "u", "path": "%unionUndeclared" }]
            }]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let pointers: Vec<&str> = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant)
        .iter()
        .map(|d| d.pointer.as_str())
        .collect();
    assert!(
        pointers.contains(&"/select/0/select/0/column/0/path"),
        "{pointers:?}"
    );
    assert!(
        pointers.contains(&"/select/0/unionAll/0/column/0/path"),
        "{pointers:?}"
    );
}

#[test]
fn constant_declared_in_constant_array_is_not_undeclared() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "myConst", "valueString": "hello" }]);
    doc["select"][0]["column"][0]["path"] = json!("%myConst");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant).is_empty(),
        "{diagnostics:?}"
    );
}

#[test]
fn declared_constant_check_is_case_sensitive() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "myConst", "valueString": "hello" }]);
    doc["select"][0]["column"][0]["path"] = json!("%MyConst");
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
}

#[test]
fn constant_declared_without_value_x_is_still_not_undeclared() {
    // A `constant` entry missing its value[x] is its own MissingRequired
    // diagnostic (see `constant_missing_value_x_is_missing_required`) — it
    // must not *also* make every reference to its name UndeclaredConstant.
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "noValue" }]);
    doc["select"][0]["column"][0]["path"] = json!("%noValue");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant).is_empty(),
        "{diagnostics:?}"
    );
    assert!(!diagnostics_of(&diagnostics, DiagnosticCode::MissingRequired).is_empty());
}

#[test]
fn environment_variables_are_not_undeclared() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] =
        json!("%context.combine(%resource).combine(%rootResource) & %ucum & %sct & %loinc");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant).is_empty(),
        "{diagnostics:?}"
    );
}

#[test]
fn sql_on_fhir_row_index_is_not_undeclared() {
    // `%rowIndex` is bound by `helios_sof` itself (see
    // `extract_view_definition_constants`), not by the FHIRPath evaluator —
    // this is the case that regressed `official_sql_on_fhir_fixtures_that_
    // are_not_error_cases_lint_clean` (row_index.json) during development.
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("%rowIndex");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant).is_empty(),
        "{diagnostics:?}"
    );
}

#[test]
fn two_occurrences_of_the_same_undeclared_constant_produce_two_diagnostics() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("%dup = 1 or %dup = 2");
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 2, "{undeclared:?}");
    assert_ne!(undeclared[0].span, undeclared[1].span);
}

#[test]
fn undeclared_constant_span_covers_exactly_the_percent_name_token() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("true and %notDeclared");
    let diagnostics = lint_view_definition(&doc);
    let undeclared = diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.len(), 1, "{undeclared:?}");
    let span = undeclared[0].span.expect("span set");
    let expression = "true and %notDeclared";
    let chars: Vec<char> = expression.chars().collect();
    let underlined: String = chars[span.start..span.end].iter().collect();
    assert_eq!(underlined, "%notDeclared");
}

#[test]
fn non_parsing_expression_only_reports_fhirpath_syntax() {
    let mut doc = valid_doc();
    // A trailing dot before the constant reference means the expression
    // never parses: only FhirPathSyntax should fire, never
    // UndeclaredConstant for the `%notDeclared` reference inside it.
    doc["select"][0]["column"][0]["path"] = json!("name.%notDeclared");
    let diagnostics = lint_view_definition(&doc);
    assert!(
        diagnostics_of(&diagnostics, DiagnosticCode::UndeclaredConstant).is_empty(),
        "{diagnostics:?}"
    );
    assert!(!diagnostics_of(&diagnostics, DiagnosticCode::FhirPathSyntax).is_empty());
}

#[test]
fn undeclared_constant_sorts_by_true_document_position() {
    let doc: Value = serde_json::from_str(
        r#"{
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                { "column": [{ "name": "a", "path": "%firstUndeclared" }] },
                { "unknownField": true, "column": [{ "name": "b", "path": "getResourceKey()" }] }
            ]
        }"#,
    )
    .expect("valid JSON literal");

    let diagnostics = lint_view_definition(&doc);
    let pointers: Vec<&str> = diagnostics.iter().map(|d| d.pointer.as_str()).collect();
    let undeclared_index = pointers
        .iter()
        .position(|p| *p == "/select/0/column/0/path")
        .expect("UndeclaredConstant on /select/0/column/0/path");
    let unknown_key_index = pointers
        .iter()
        .position(|p| *p == "/select/1/unknownField")
        .expect("UnknownKey on /select/1/unknownField");
    assert!(
        undeclared_index < unknown_key_index,
        "undeclared-constant in /select/0 must sort before unknown-key in /select/1: {pointers:?}"
    );
}

#[test]
fn undeclared_constant_serializes_with_the_documented_code_and_a_span_object() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("%notDeclared");
    let diagnostics = lint_view_definition(&doc);
    let value = serde_json::to_value(&diagnostics).unwrap();
    let undeclared = value
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "undeclared-constant")
        .expect("an undeclared-constant diagnostic was serialized");
    assert!(undeclared["span"].is_object());
    assert!(undeclared["span"]["start"].is_u64());
    assert!(undeclared["span"]["end"].is_u64());
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
    for key in [
        "pointer", "message", "severity", "code", "span", "args", "fixes",
    ] {
        assert!(
            diagnostic.get(key).is_some(),
            "missing `{key}` in {diagnostic}"
        );
    }
}

#[test]
fn args_and_fixes_serialize_as_empty_object_and_array_when_absent() {
    // not-a-view-definition carries `args.found` but never a fix; a code with
    // neither (select-without-output) proves the "nothing to report" shape.
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{}]
    });
    let diagnostics = lint_view_definition(&doc);
    let value = serde_json::to_value(&diagnostics).unwrap();
    let without_output = value
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "select-without-output")
        .expect("a select-without-output diagnostic");
    assert_eq!(without_output["args"], json!({}));
    assert_eq!(without_output["fixes"], json!([]));
}

// ---------------------------------------------------------------------------
// pointer_to_fhirpath (#821)
// ---------------------------------------------------------------------------

#[test]
fn pointer_to_fhirpath_root_is_bare_view_definition() {
    assert_eq!(pointer_to_fhirpath(""), "ViewDefinition");
}

#[test]
fn pointer_to_fhirpath_renders_array_indices_and_keys() {
    assert_eq!(
        pointer_to_fhirpath("/select/0/column/1/path"),
        "ViewDefinition.select[0].column[1].path"
    );
    assert_eq!(pointer_to_fhirpath("/resource"), "ViewDefinition.resource");
    assert_eq!(
        pointer_to_fhirpath("/select/12"),
        "ViewDefinition.select[12]"
    );
}

#[test]
fn pointer_to_fhirpath_unescapes_rfc_6901_keys() {
    // `~1` -> `/`, `~0` -> `~`, decoded in that order (the encoding escapes
    // `~` before `/`, so decoding must undo `/` first or a literal `~1` in a
    // key would be corrupted into `~/`).
    assert_eq!(pointer_to_fhirpath("/a~1b"), "ViewDefinition.a/b");
    assert_eq!(pointer_to_fhirpath("/a~0b"), "ViewDefinition.a~b");
    // `~01` decodes to `~1` verbatim (not `/`): confirms `~1` -> `/` runs
    // before `~0` -> `~`, so the `~0` this segment starts with can't create
    // a spurious `~1` for the first pass to consume.
    assert_eq!(pointer_to_fhirpath("/a~01"), "ViewDefinition.a~1");
}

// ---------------------------------------------------------------------------
// Diagnostic args (#821)
// ---------------------------------------------------------------------------

/// The single diagnostic with `code` on `doc`, panicking with everything
/// found if there isn't exactly one — most of the `args`/`fixes` tests below
/// only care about one diagnostic and want a clean failure message if the
/// document accidentally produces more (or fewer) than expected.
fn only_diagnostic_of(doc: &Value, code: DiagnosticCode) -> Diagnostic {
    let diagnostics = lint_view_definition(doc);
    let mut matching = diagnostics_of(&diagnostics, code).into_iter();
    let found = matching
        .next()
        .unwrap_or_else(|| panic!("expected one {code:?} diagnostic, found none"))
        .clone();
    assert!(
        matching.next().is_none(),
        "expected exactly one {code:?} diagnostic on {doc}"
    );
    found
}

#[test]
fn args_not_a_view_definition_names_what_was_found() {
    assert_eq!(
        only_diagnostic_of(&json!([1, 2, 3]), DiagnosticCode::NotAViewDefinition).args["found"],
        "a non-object document"
    );
    assert_eq!(
        only_diagnostic_of(
            &json!({ "resourceType": "Patient" }),
            DiagnosticCode::NotAViewDefinition
        )
        .args["found"],
        r#"resourceType "Patient""#
    );
    assert_eq!(
        only_diagnostic_of(
            &json!({ "resourceType": 5 }),
            DiagnosticCode::NotAViewDefinition
        )
        .args["found"],
        "a non-string resourceType (a number)"
    );
}

#[test]
fn args_missing_required_and_empty_required_carry_the_key() {
    // `{"resourceType": "ViewDefinition"}` is missing both `resource` and
    // `select` - both diagnostics land on the same (root) pointer, so only
    // the missing `select` one is picked out by its own `args.key`.
    let doc = json!({ "resourceType": "ViewDefinition" });
    let diagnostics = lint_view_definition(&doc);
    let missing = diagnostics_of(&diagnostics, DiagnosticCode::MissingRequired)
        .into_iter()
        .find(|d| d.args.get("key").map(String::as_str) == Some("select"))
        .expect("a missing-required diagnostic for select");
    assert_eq!(missing.args["key"], "select");

    let mut doc = valid_doc();
    doc["resource"] = json!("   ");
    let diagnostics = lint_view_definition(&doc);
    let empty = diagnostics_of(&diagnostics, DiagnosticCode::EmptyRequired)
        .into_iter()
        .find(|d| d.pointer == "/resource")
        .expect("an empty-required diagnostic on /resource");
    assert_eq!(empty.args["key"], "resource");
}

#[test]
fn args_wrong_type_carries_expected_and_found() {
    let mut doc = valid_doc();
    doc["status"] = json!(42);
    let wrong = only_diagnostic_of(&doc, DiagnosticCode::WrongType);
    assert_eq!(wrong.args["expected"], "a string");
    assert_eq!(wrong.args["found"], "a number");
}

#[test]
fn args_duplicate_column_name_carries_the_name() {
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
    let dup = only_diagnostic_of(&doc, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dup.args["name"], "id");
}

#[test]
fn args_multiple_iteration_directives_lists_present_keys_in_struct_order() {
    let mut doc = valid_doc();
    doc["select"][0]["repeat"] = json!(["contact"]);
    doc["select"][0]["forEach"] = json!("name");
    doc["select"][0]["forEachOrNull"] = json!("telecom");
    let multi = only_diagnostic_of(&doc, DiagnosticCode::MultipleIterationDirectives);
    assert_eq!(multi.args["keys"], "forEach, forEachOrNull, repeat");
}

#[test]
fn args_select_without_output_has_none() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{}]
    });
    let diag = only_diagnostic_of(&doc, DiagnosticCode::SelectWithoutOutput);
    assert!(diag.args.is_empty(), "{:?}", diag.args);
}

#[test]
fn args_fhirpath_syntax_carries_the_parser_detail_verbatim() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("getResourceKey(");
    let syntax = only_diagnostic_of(&doc, DiagnosticCode::FhirPathSyntax);
    assert_eq!(syntax.args["detail"], syntax.message);

    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("   ");
    let empty = only_diagnostic_of(&doc, DiagnosticCode::FhirPathSyntax);
    assert_eq!(empty.args["detail"], "empty expression");
}

#[test]
fn args_undeclared_constant_carries_the_bare_name() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0]["path"] = json!("%notDeclared");
    let undeclared = only_diagnostic_of(&doc, DiagnosticCode::UndeclaredConstant);
    assert_eq!(undeclared.args["name"], "notDeclared");
}

#[test]
fn args_constant_value_x_missing_carries_variant_and_the_constants_name() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "name": "myConst" }]);
    let diagnostics = lint_view_definition(&doc);
    let missing = diagnostics_of(&diagnostics, DiagnosticCode::MissingRequired)
        .into_iter()
        .find(|d| d.args.get("variant").map(String::as_str) == Some("missing"))
        .expect("a missing-required diagnostic with variant \"missing\"");
    assert_eq!(missing.args["name"], "myConst");
}

#[test]
fn args_constant_value_x_multiple_carries_variant_multiple_and_omits_name_when_absent() {
    let mut doc = valid_doc();
    doc["constant"] = json!([{ "valueString": "a", "valueBoolean": true }]);
    let diagnostics = lint_view_definition(&doc);
    let multiple = diagnostics_of(&diagnostics, DiagnosticCode::WrongType)
        .into_iter()
        .find(|d| d.args.get("variant").map(String::as_str) == Some("multiple"))
        .expect("a wrong-type diagnostic with variant \"multiple\"");
    assert!(!multiple.args.contains_key("name"), "{:?}", multiple.args);
}

// ---------------------------------------------------------------------------
// unknown-key typo suggestions and fixes (#821)
// ---------------------------------------------------------------------------

#[test]
fn unknown_key_suggests_the_singular_for_a_pluralized_typo() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{ "columns": [{ "name": "id", "path": "getResourceKey()" }] }]
    });
    let unknown = only_diagnostic_of(&doc, DiagnosticCode::UnknownKey);
    assert_eq!(unknown.args["key"], "columns");
    assert_eq!(unknown.args["suggestion"], "column");
    assert_eq!(
        unknown.fixes,
        vec![
            Fix::RenameKey {
                pointer: "/select/0/columns".to_string(),
                to: "column".to_string(),
            },
            Fix::RemoveKey {
                pointer: "/select/0/columns".to_string(),
            },
        ]
    );
}

#[test]
fn unknown_key_suggestion_ignores_case() {
    let mut doc = valid_doc();
    doc["select"][0]["column"][0] = json!({ "Path": "getResourceKey()", "name": "id" });
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey)
        .into_iter()
        .find(|d| d.pointer == "/select/0/column/0/Path")
        .expect("an unknown-key diagnostic for Path");
    assert_eq!(unknown.args["suggestion"], "path");
}

#[test]
fn unknown_key_suggests_within_edit_distance_two() {
    let mut doc = valid_doc();
    doc["select"][0]["forEachh"] = json!("name");
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey)
        .into_iter()
        .find(|d| d.pointer == "/select/0/forEachh")
        .expect("an unknown-key diagnostic for forEachh");
    assert_eq!(unknown.args["suggestion"], "forEach");
}

#[test]
fn unknown_key_offers_no_suggestion_when_nothing_is_close() {
    let mut doc = valid_doc();
    doc["zzz"] = json!("oops");
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey)
        .into_iter()
        .find(|d| d.pointer == "/zzz")
        .expect("an unknown-key diagnostic for zzz");
    assert!(
        !unknown.args.contains_key("suggestion"),
        "{:?}",
        unknown.args
    );
    assert_eq!(
        unknown.fixes,
        vec![Fix::RemoveKey {
            pointer: "/zzz".to_string()
        }]
    );
}

#[test]
fn unknown_key_never_suggests_a_key_already_present() {
    let mut doc = valid_doc();
    // The select already has a valid `column`; `Column` is the typo.
    // Renaming it to `column` would collide with the one already there, so
    // the suggestion rule excludes it even though the edit distance
    // qualifies.
    doc["select"][0]["Column"] = doc["select"][0]["column"].clone();
    let diagnostics = lint_view_definition(&doc);
    let unknown = diagnostics_of(&diagnostics, DiagnosticCode::UnknownKey)
        .into_iter()
        .find(|d| d.pointer == "/select/0/Column")
        .expect("an unknown-key diagnostic for Column");
    assert!(
        !unknown.args.contains_key("suggestion"),
        "{:?}",
        unknown.args
    );
    assert_eq!(
        unknown.fixes,
        vec![Fix::RemoveKey {
            pointer: "/select/0/Column".to_string()
        }]
    );
}

// ---------------------------------------------------------------------------
// duplicate-column-name and multiple-iteration-directives fixes (#821)
// ---------------------------------------------------------------------------

#[test]
fn duplicate_column_name_fix_increments_the_suffix_past_a_real_collision() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                { "name": "id", "path": "getResourceKey()" },
                { "name": "id", "path": "id" },
                { "name": "id_2", "path": "id" },
                { "name": "id", "path": "id" }
            ]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 2, "{dups:?}");
    // The `id_2` real column claims that suffix document-wide, so even the
    // *first* duplicate (declared before it) must skip past it — not just
    // duplicates declared after it in document order.
    assert_eq!(
        dups[0].fixes,
        vec![Fix::SetString {
            pointer: "/select/0/column/1/name".to_string(),
            value: "id_3".to_string(),
        }]
    );
    // The first duplicate's own fix just claimed `id_3`, so the second
    // duplicate's fix skips to `id_4`.
    assert_eq!(
        dups[1].fixes,
        vec![Fix::SetString {
            pointer: "/select/0/column/3/name".to_string(),
            value: "id_4".to_string(),
        }]
    );
}

/// Regression: the collision the suggested name must avoid is not limited to
/// names the row-scoped duplicate walk has already passed — a column named
/// `g_2` declared *later*, in a wholly different `select` entry, is just as
/// real a collision as one declared earlier. Reported against the fix
/// generation before it pre-collected every column name in the document.
#[test]
fn duplicate_column_name_fix_avoids_a_name_declared_later_in_a_different_select() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "draft",
        "select": [
            { "column": [{ "name": "id", "path": "id" }] },
            {
                "forEach": "name",
                "column": [
                    { "name": "g", "path": "given" },
                    { "name": "g", "path": "family" },
                    { "name": "g_2", "path": "id" }
                ]
            }
        ]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 1, "{dups:?}");
    assert_eq!(dups[0].pointer, "/select/1/column/1/name");
    assert_eq!(
        dups[0].fixes,
        vec![Fix::SetString {
            pointer: "/select/1/column/1/name".to_string(),
            value: "g_3".to_string(),
        }]
    );
}

/// Three columns sharing one name in a row produce two duplicates, each
/// picking a suffix the other's fix hasn't already claimed this same pass —
/// `g_2` then `g_3`, never `g_2` twice.
#[test]
fn duplicate_column_name_fix_increments_across_two_consecutive_duplicates() {
    let doc = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                { "name": "g", "path": "a" },
                { "name": "g", "path": "b" },
                { "name": "g", "path": "c" }
            ]
        }]
    });
    let diagnostics = lint_view_definition(&doc);
    let dups = diagnostics_of(&diagnostics, DiagnosticCode::DuplicateColumnName);
    assert_eq!(dups.len(), 2, "{dups:?}");
    assert_eq!(
        dups[0].fixes,
        vec![Fix::SetString {
            pointer: "/select/0/column/1/name".to_string(),
            value: "g_2".to_string(),
        }]
    );
    assert_eq!(
        dups[1].fixes,
        vec![Fix::SetString {
            pointer: "/select/0/column/2/name".to_string(),
            value: "g_3".to_string(),
        }]
    );
}

#[test]
fn multiple_iteration_directives_fix_removes_every_directive_after_the_first() {
    let mut doc = valid_doc();
    doc["select"][0]["forEach"] = json!("name");
    doc["select"][0]["forEachOrNull"] = json!("telecom");
    doc["select"][0]["repeat"] = json!(["contact"]);
    let multi = only_diagnostic_of(&doc, DiagnosticCode::MultipleIterationDirectives);
    assert_eq!(
        multi.fixes,
        vec![
            Fix::RemoveKey {
                pointer: "/select/0/forEachOrNull".to_string(),
            },
            Fix::RemoveKey {
                pointer: "/select/0/repeat".to_string(),
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// Fix serialization (#821)
// ---------------------------------------------------------------------------

#[test]
fn fix_serializes_with_a_kebab_case_kind_tag() {
    assert_eq!(
        serde_json::to_value(Fix::RenameKey {
            pointer: "/select/0/columns".to_string(),
            to: "column".to_string(),
        })
        .unwrap(),
        json!({ "kind": "rename-key", "pointer": "/select/0/columns", "to": "column" })
    );
    assert_eq!(
        serde_json::to_value(Fix::RemoveKey {
            pointer: "/select/0/columns".to_string(),
        })
        .unwrap(),
        json!({ "kind": "remove-key", "pointer": "/select/0/columns" })
    );
    assert_eq!(
        serde_json::to_value(Fix::SetString {
            pointer: "/select/0/column/1/name".to_string(),
            value: "id_2".to_string(),
        })
        .unwrap(),
        json!({ "kind": "set-string", "pointer": "/select/0/column/1/name", "value": "id_2" })
    );
}

// ---------------------------------------------------------------------------
// node_keys (#821)
// ---------------------------------------------------------------------------

#[test]
fn node_keys_root_lists_resource_type_and_select() {
    let keys = node_keys("").expect("the root resolves to a node");
    let names: Vec<&str> = keys.iter().map(|k| k.key).collect();
    assert!(names.contains(&"resourceType"), "{names:?}");
    assert!(names.contains(&"resource"), "{names:?}");
    assert!(names.contains(&"select"), "{names:?}");
    let resource_type = keys.iter().find(|k| k.key == "resourceType").unwrap();
    assert!(resource_type.required);
    assert_eq!(resource_type.kind, KeyKind::String);
    let select = keys.iter().find(|k| k.key == "select").unwrap();
    assert!(select.required);
    assert_eq!(select.kind, KeyKind::ObjectArray);
}

#[test]
fn node_keys_select_index_and_union_all_resolve_to_the_select_node() {
    let expected: Vec<&str> = node_keys("/select/0")
        .expect("/select/0 resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert!(expected.contains(&"forEach"), "{expected:?}");
    assert!(expected.contains(&"unionAll"), "{expected:?}");

    // A `unionAll` branch is itself a `select` node — same keys.
    let union_all: Vec<&str> = node_keys("/select/0/unionAll/1")
        .expect("/select/0/unionAll/1 resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert_eq!(expected, union_all);
}

#[test]
fn node_keys_column_index_and_the_column_array_resolve_to_the_same_node() {
    let indexed: Vec<&str> = node_keys("/select/0/column/0")
        .expect("/select/0/column/0 resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    let array: Vec<&str> = node_keys("/select/0/column")
        .expect("/select/0/column resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert_eq!(indexed, array);
    assert!(indexed.contains(&"path"), "{indexed:?}");
    assert!(indexed.contains(&"name"), "{indexed:?}");
    assert!(indexed.contains(&"tag"), "{indexed:?}");
}

#[test]
fn node_keys_resolves_tag_where_and_constant_nodes() {
    let tag: Vec<&str> = node_keys("/select/0/column/0/tag/0")
        .expect("tag resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert_eq!(tag, vec!["name", "value"]);

    let where_keys: Vec<&str> = node_keys("/where/2")
        .expect("where resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert_eq!(where_keys, vec!["path", "description"]);

    let constant: Vec<&str> = node_keys("/constant/0")
        .expect("constant resolves")
        .iter()
        .map(|k| k.key)
        .collect();
    assert!(constant.contains(&"name"), "{constant:?}");
    assert!(constant.contains(&"valueString"), "{constant:?}");
}

#[test]
fn node_keys_returns_none_for_a_scalar_or_unmodeled_pointer() {
    assert_eq!(node_keys("/resource"), None);
    assert_eq!(node_keys("/notAField"), None);
    assert_eq!(node_keys("/select/0/column/0/notAField"), None);
}
