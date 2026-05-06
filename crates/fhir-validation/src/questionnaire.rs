//! Validate a [`QuestionnaireResponse`](helios_fhir::r5::QuestionnaireResponse) against a
//! resolved [`Questionnaire`](helios_fhir::r5::Questionnaire).
//!
//! Callers are responsible for fetching the questionnaire (via REST, bundle `contained`,
//! or an application-specific resolver) and invoking this module after structural validation. This does **not** replace profile/snapshot validation.

#[cfg(feature = "R5")]
use crate::ValidationIssue;
#[cfg(feature = "R5")]
use crate::issue_code;
#[cfg(feature = "R5")]
use helios_fhir::r5::{
    Canonical, Questionnaire, QuestionnaireItem, QuestionnaireResponse, QuestionnaireResponseItem,
    QuestionnaireResponseItemAnswer, QuestionnaireResponseItemAnswerValue, Uri,
};

#[cfg(feature = "R5")]
fn link_id_string_from_instance(x: &impl serde::Serialize) -> Option<String> {
    let v = serde_json::to_value(x).ok()?;
    match v {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(mut m) => match m.remove("value") {
            Some(serde_json::Value::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(feature = "R5")]
fn canonical_str(c: &Canonical) -> Option<&str> {
    c.value.as_deref()
}

#[cfg(feature = "R5")]
fn uri_str(u: &Uri) -> Option<&str> {
    u.value.as_deref()
}

#[cfg(feature = "R5")]
fn bool_true(b: &helios_fhir::r5::Boolean) -> bool {
    b.value == Some(true)
}

#[cfg(feature = "R5")]
fn collect_items<'a>(
    items: &'a [QuestionnaireItem],
    out: &mut std::collections::HashMap<String, &'a QuestionnaireItem>,
) {
    for it in items {
        if let Some(key) = link_id_string_from_instance(&it.link_id) {
            out.insert(key, it);
        }
        if let Some(children) = &it.item {
            collect_items(children, out);
        }
    }
}

#[cfg(feature = "R5")]
fn type_code(item: &QuestionnaireItem) -> &str {
    item.r#type.value.as_deref().unwrap_or("")
}

#[cfg(feature = "R5")]
fn answer_matches_declared_type(
    answer: &QuestionnaireResponseItemAnswer,
    declared: &str,
    instance_path: &str,
) -> Option<ValidationIssue> {
    let Some(v) = &answer.value else {
        return Some(
            ValidationIssue::error(
                issue_code::STRUCTURE,
                instance_path,
                format!("Answer at '{instance_path}' is missing a value choice"),
            )
            .with_instance_path(instance_path),
        );
    };
    let ok = match declared {
        "boolean" => matches!(v, QuestionnaireResponseItemAnswerValue::Boolean(_)),
        "decimal" => matches!(v, QuestionnaireResponseItemAnswerValue::Decimal(_)),
        "integer" => matches!(v, QuestionnaireResponseItemAnswerValue::Integer(_)),
        "date" => matches!(v, QuestionnaireResponseItemAnswerValue::Date(_)),
        "dateTime" => matches!(v, QuestionnaireResponseItemAnswerValue::DateTime(_)),
        "time" => matches!(v, QuestionnaireResponseItemAnswerValue::Time(_)),
        "string" | "text" => matches!(v, QuestionnaireResponseItemAnswerValue::String(_)),
        "url" => matches!(v, QuestionnaireResponseItemAnswerValue::Uri(_)),
        "choice" | "open-choice" => matches!(
            v,
            QuestionnaireResponseItemAnswerValue::Coding(_)
                | QuestionnaireResponseItemAnswerValue::String(_)
        ),
        "attachment" => matches!(v, QuestionnaireResponseItemAnswerValue::Attachment(_)),
        "reference" => matches!(v, QuestionnaireResponseItemAnswerValue::Reference(_)),
        "quantity" => matches!(v, QuestionnaireResponseItemAnswerValue::Quantity(_)),
        _ => true,
    };
    if ok {
        None
    } else {
        Some(
            ValidationIssue::error(
                issue_code::VALUE,
                instance_path,
                format!(
                    "Answer value shape does not match Questionnaire item type '{declared}' at '{instance_path}'"
                ),
            )
            .with_instance_path(instance_path),
        )
    }
}

#[cfg(feature = "R5")]
fn walk_items(
    items: &[QuestionnaireResponseItem],
    defs: &std::collections::HashMap<String, &QuestionnaireItem>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    for (i, ri) in items.iter().enumerate() {
        let item_path = format!("{path}.item[{i}]");
        let link = link_id_string_from_instance(&ri.link_id).unwrap_or_default();
        let Some(def) = defs.get(&link) else {
            issues.push(
                ValidationIssue::error(
                    issue_code::STRUCTURE,
                    &item_path,
                    format!("linkId '{link}' does not match any Questionnaire.item.linkId"),
                )
                .with_instance_path(&item_path),
            );
            continue;
        };

        let t = type_code(def);
        match t {
            "group" => {
                if let Some(a) = &ri.answer
                    && !a.is_empty()
                {
                    issues.push(
                        ValidationIssue::error(
                            issue_code::STRUCTURE,
                            &item_path,
                            "Group item must not include answers; use nested item instead",
                        )
                        .with_instance_path(&item_path),
                    );
                }
                if let Some(children) = &ri.item {
                    walk_items(children, defs, &item_path, issues);
                }
            }
            "display" => {
                if let Some(a) = &ri.answer
                    && !a.is_empty()
                {
                    issues.push(
                        ValidationIssue::warning(
                            issue_code::STRUCTURE,
                            &item_path,
                            "Display item typically has no answers",
                        )
                        .with_instance_path(&item_path),
                    );
                }
            }
            _ => {
                let repeats = def.repeats.as_ref().map(bool_true).unwrap_or(false);
                let required = def.required.as_ref().map(bool_true).unwrap_or(false);
                let n_answers = ri.answer.as_ref().map(|a| a.len()).unwrap_or(0);
                if required && n_answers == 0 {
                    issues.push(
                        ValidationIssue::error(
                            issue_code::STRUCTURE,
                            &item_path,
                            format!(
                                "Required question '{}' (linkId {link}) has no answer",
                                item_path
                            ),
                        )
                        .with_instance_path(&item_path),
                    );
                }
                if !repeats && n_answers > 1 {
                    issues.push(
                        ValidationIssue::error(
                            issue_code::STRUCTURE,
                            &item_path,
                            "Question does not allow repeats=true but multiple answers were provided",
                        )
                        .with_instance_path(&item_path),
                    );
                }
                if let Some(answers) = &ri.answer {
                    for (ai, ans) in answers.iter().enumerate() {
                        let ap = format!("{item_path}.answer[{ai}]");
                        if let Some(issue) = answer_matches_declared_type(ans, t, &ap) {
                            issues.push(issue);
                        }
                        if let Some(nested) = &ans.item {
                            walk_items(nested, defs, &ap, issues);
                        }
                    }
                }
                if let Some(children) = &ri.item {
                    walk_items(children, defs, &item_path, issues);
                }
            }
        }
    }
}

/// Compare a populated [`QuestionnaireResponse`] to the design-time [`Questionnaire`].
///
/// Checks include: known `linkId`s, group vs answer shape, `replies` vs `repeats`,
/// required questions, and coarse answer datatype vs `Questionnaire.item.type`.
#[cfg(feature = "R5")]
pub fn validate_questionnaire_response_against_questionnaire(
    response: &QuestionnaireResponse,
    questionnaire: &Questionnaire,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if let (Some(q_url), Some(r_canonical)) = (
        questionnaire.url.as_ref().and_then(uri_str),
        canonical_str(&response.questionnaire),
    ) {
        if r_canonical != q_url && !r_canonical.starts_with(q_url) {
            issues.push(
                ValidationIssue::warning(
                    "value",
                    "QuestionnaireResponse.questionnaire",
                    format!(
                        "QuestionnaireResponse.questionnaire '{r_canonical}' does not match Questionnaire.url '{q_url}'"
                    ),
                )
                .with_instance_path("QuestionnaireResponse.questionnaire"),
            );
        }
    }

    let mut defs = std::collections::HashMap::new();
    if let Some(items) = &questionnaire.item {
        collect_items(items, &mut defs);
    }

    if let Some(items) = &response.item {
        walk_items(items, &defs, "QuestionnaireResponse", &mut issues);
    }

    issues
}

#[cfg(all(test, feature = "R5"))]
mod tests {
    use super::*;
    use helios_fhir::r5::{
        Boolean, Code, Integer, QuestionnaireItem, QuestionnaireResponseItem,
        QuestionnaireResponseItemAnswer, QuestionnaireResponseItemAnswerValue,
    };

    #[test]
    fn rejects_unknown_link_id() {
        let q = Questionnaire {
            item: Some(vec![QuestionnaireItem {
                link_id: "q1".to_string().into(),
                r#type: Code {
                    value: Some("string".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }]),
            ..Default::default()
        };
        let qr = QuestionnaireResponse {
            item: Some(vec![QuestionnaireResponseItem {
                link_id: "nope".to_string().into(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let issues = validate_questionnaire_response_against_questionnaire(&qr, &q);
        assert!(issues.iter().any(|i| i.diagnostics.contains("linkId")));
    }

    #[test]
    fn rejects_answer_type_mismatch() {
        let q = Questionnaire {
            item: Some(vec![QuestionnaireItem {
                link_id: "q1".to_string().into(),
                r#type: Code {
                    value: Some("integer".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }]),
            ..Default::default()
        };
        let qr = QuestionnaireResponse {
            item: Some(vec![QuestionnaireResponseItem {
                link_id: "q1".to_string().into(),
                answer: Some(vec![QuestionnaireResponseItemAnswer {
                    value: Some(QuestionnaireResponseItemAnswerValue::String(
                        "x".to_string().into(),
                    )),
                    ..Default::default()
                }]),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let issues = validate_questionnaire_response_against_questionnaire(&qr, &q);
        assert!(issues.iter().any(|i| i.code == issue_code::VALUE));
    }

    #[test]
    fn repeats_enforced() {
        let q = Questionnaire {
            item: Some(vec![QuestionnaireItem {
                link_id: "q1".to_string().into(),
                r#type: Code {
                    value: Some("integer".to_string()),
                    ..Default::default()
                },
                repeats: Some(Boolean {
                    value: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let qr = QuestionnaireResponse {
            item: Some(vec![QuestionnaireResponseItem {
                link_id: "q1".to_string().into(),
                answer: Some(vec![
                    QuestionnaireResponseItemAnswer {
                        value: Some(QuestionnaireResponseItemAnswerValue::Integer(Integer {
                            value: Some(1),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                    QuestionnaireResponseItemAnswer {
                        value: Some(QuestionnaireResponseItemAnswerValue::Integer(Integer {
                            value: Some(2),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let issues = validate_questionnaire_response_against_questionnaire(&qr, &q);
        assert!(issues.iter().any(|i| i.diagnostics.contains("repeats")));
    }
}
