//! QuestionnaireResponse validation against a Questionnaire definition.
//!
//! Structural profile validation still applies via the main engine; this
//! module adds Questionnaire-driven checks: known `linkId`s, required items
//! (when enabled), answer type vs `item.type`, `answerOption` membership, and
//! `answerValueSet` membership via an optional [`TerminologyProvider`].

use crate::effects::{CodedValue, TerminologyProvider};
use crate::engine::{ErrorKind, Severity, ValidationError};
use serde_json::Value;
use std::collections::HashMap;

/// Validate `qr` against `questionnaire`.
///
/// `terminology` is used only for `answerValueSet` checks; when absent those
/// checks are skipped (not failed).
pub async fn validate_questionnaire_response(
    qr: &Value,
    questionnaire: &Value,
    terminology: Option<&dyn TerminologyProvider>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let defs = index_questionnaire_items(questionnaire);
    let answers = collect_qr_items(qr);

    // Required items that are enabled must appear.
    for (link_id, def) in &defs {
        if !def.required {
            continue;
        }
        if !item_enabled(def, &answers, &defs) {
            continue;
        }
        if !answers.contains_key(link_id.as_str()) {
            errors.push(ValidationError::new(
                ErrorKind::Questionnaire,
                format!("QuestionnaireResponse.item[{link_id}]"),
                format!("required Questionnaire item '{link_id}' is missing"),
            ));
        }
    }

    for (link_id, answered) in &answers {
        let Some(def) = defs.get(link_id.as_str()) else {
            errors.push(ValidationError::new(
                ErrorKind::Questionnaire,
                format!("QuestionnaireResponse.item[{link_id}]"),
                format!("linkId '{link_id}' is not defined by the Questionnaire"),
            ));
            continue;
        };
        if !item_enabled(def, &answers, &defs) && !answered.answers.is_empty() {
            errors.push(
                ValidationError::new(
                    ErrorKind::Questionnaire,
                    format!("QuestionnaireResponse.item[{link_id}]"),
                    format!("item '{link_id}' has answers but enableWhen is not satisfied"),
                )
                .with_severity(Severity::Warning),
            );
        }
        for answer in &answered.answers {
            if let Some(err) = check_answer_type(link_id, def, answer) {
                errors.push(err);
            }
            if let Some(err) = check_answer_option(link_id, def, answer) {
                errors.push(err);
            }
            if let Some(vs) = &def.answer_value_set
                && let Some(provider) = terminology
                && let Some(coded) = answer_as_coded(answer)
                && let Ok(false) = provider.validate_code(vs, &coded).await
            {
                errors.push(ValidationError::new(
                    ErrorKind::Questionnaire,
                    format!("QuestionnaireResponse.item[{link_id}].answer"),
                    format!("answer for '{link_id}' is not in answerValueSet '{vs}'"),
                ));
            }
        }
    }

    errors
}

#[derive(Debug, Clone)]
struct ItemDef {
    type_: String,
    required: bool,
    answer_options: Vec<Value>,
    answer_value_set: Option<String>,
    enable_when: Vec<EnableWhen>,
    enable_behavior: String,
}

#[derive(Debug, Clone)]
struct EnableWhen {
    question: String,
    operator: String,
    answer: Option<Value>,
}

#[derive(Debug, Default)]
struct AnsweredItem {
    answers: Vec<Value>,
}

fn index_questionnaire_items(questionnaire: &Value) -> HashMap<String, ItemDef> {
    let mut out = HashMap::new();
    if let Some(items) = questionnaire.get("item").and_then(Value::as_array) {
        walk_q_items(items, &mut out);
    }
    out
}

fn walk_q_items(items: &[Value], out: &mut HashMap<String, ItemDef>) {
    for item in items {
        if let Some(link_id) = item.get("linkId").and_then(Value::as_str) {
            let enable_when = item
                .get("enableWhen")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|ew| {
                            Some(EnableWhen {
                                question: ew.get("question")?.as_str()?.to_string(),
                                operator: ew
                                    .get("operator")
                                    .and_then(Value::as_str)
                                    .unwrap_or("exists")
                                    .to_string(),
                                answer: ew.as_object().and_then(|o| {
                                    o.iter()
                                        .find(|(k, _)| k.starts_with("answer"))
                                        .map(|(_, v)| v.clone())
                                }),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let answer_options = item
                .get("answerOption")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|opt| {
                    opt.as_object().and_then(|o| {
                        o.iter()
                            .find(|(k, _)| k.starts_with("value"))
                            .map(|(_, v)| v.clone())
                    })
                })
                .collect();
            out.insert(
                link_id.to_string(),
                ItemDef {
                    type_: item
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("string")
                        .to_string(),
                    required: item
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    answer_options,
                    answer_value_set: item
                        .get("answerValueSet")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    enable_when,
                    enable_behavior: item
                        .get("enableBehavior")
                        .and_then(Value::as_str)
                        .unwrap_or("all")
                        .to_string(),
                },
            );
        }
        if let Some(nested) = item.get("item").and_then(Value::as_array) {
            walk_q_items(nested, out);
        }
    }
}

fn collect_qr_items(qr: &Value) -> HashMap<String, AnsweredItem> {
    let mut out = HashMap::new();
    if let Some(items) = qr.get("item").and_then(Value::as_array) {
        walk_qr_items(items, &mut out);
    }
    out
}

fn walk_qr_items(items: &[Value], out: &mut HashMap<String, AnsweredItem>) {
    for item in items {
        if let Some(link_id) = item.get("linkId").and_then(Value::as_str) {
            let answers = item
                .get("answer")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            out.entry(link_id.to_string())
                .or_default()
                .answers
                .extend(answers);
        }
        if let Some(nested) = item.get("item").and_then(Value::as_array) {
            walk_qr_items(nested, out);
        }
    }
}

fn item_enabled(
    def: &ItemDef,
    answers: &HashMap<String, AnsweredItem>,
    _defs: &HashMap<String, ItemDef>,
) -> bool {
    if def.enable_when.is_empty() {
        return true;
    }
    let results: Vec<bool> = def
        .enable_when
        .iter()
        .map(|ew| eval_enable_when(ew, answers))
        .collect();
    if def.enable_behavior == "any" {
        results.iter().any(|r| *r)
    } else {
        results.iter().all(|r| *r)
    }
}

fn eval_enable_when(ew: &EnableWhen, answers: &HashMap<String, AnsweredItem>) -> bool {
    let answered = answers.get(&ew.question);
    match ew.operator.as_str() {
        "exists" => {
            let exists = answered.is_some_and(|a| !a.answers.is_empty());
            match &ew.answer {
                Some(Value::Bool(expected)) => exists == *expected,
                None => exists,
                _ => exists,
            }
        }
        "=" | "equal" => answered.is_some_and(|a| {
            a.answers.iter().any(|ans| {
                ew.answer
                    .as_ref()
                    .is_some_and(|expected| answer_equals(ans, expected))
            })
        }),
        "!=" => answered.is_some_and(|a| {
            a.answers.iter().any(|ans| {
                ew.answer
                    .as_ref()
                    .is_some_and(|expected| !answer_equals(ans, expected))
            })
        }),
        _ => true, // unsupported operators: do not block
    }
}

fn answer_equals(answer: &Value, expected: &Value) -> bool {
    // Compare the value[x] payload of an answer object to enableWhen.answer[x].
    let actual = answer_value(answer).unwrap_or(answer);
    actual == expected
}

fn answer_value(answer: &Value) -> Option<&Value> {
    answer.as_object().and_then(|o| {
        o.iter()
            .find(|(k, _)| k.starts_with("value"))
            .map(|(_, v)| v)
    })
}

fn check_answer_type(link_id: &str, def: &ItemDef, answer: &Value) -> Option<ValidationError> {
    let obj = answer.as_object()?;
    let key = obj.keys().find(|k| k.starts_with("value"))?;
    let expected = match def.type_.as_str() {
        "boolean" => "valueBoolean",
        "decimal" => "valueDecimal",
        "integer" => "valueInteger",
        "date" => "valueDate",
        "dateTime" => "valueDateTime",
        "time" => "valueTime",
        "string" | "text" => "valueString",
        "url" => "valueUri",
        "coding" => "valueCoding",
        "quantity" => "valueQuantity",
        "reference" => "valueReference",
        "attachment" => "valueAttachment",
        // display / group / question have no answers
        "display" | "group" | "question" => {
            return Some(ValidationError::new(
                ErrorKind::Questionnaire,
                format!("QuestionnaireResponse.item[{link_id}].answer"),
                format!(
                    "item '{link_id}' has type '{}' and should not have answers",
                    def.type_
                ),
            ));
        }
        _ => return None,
    };
    if key != expected {
        return Some(ValidationError::new(
            ErrorKind::Questionnaire,
            format!("QuestionnaireResponse.item[{link_id}].answer"),
            format!(
                "answer for '{link_id}' uses '{key}' but Questionnaire type '{}' expects '{expected}'",
                def.type_
            ),
        ));
    }
    None
}

fn check_answer_option(link_id: &str, def: &ItemDef, answer: &Value) -> Option<ValidationError> {
    if def.answer_options.is_empty() {
        return None;
    }
    let value = answer_value(answer)?;
    let ok = def.answer_options.iter().any(|opt| opt == value);
    if ok {
        None
    } else {
        Some(ValidationError::new(
            ErrorKind::Questionnaire,
            format!("QuestionnaireResponse.item[{link_id}].answer"),
            format!("answer for '{link_id}' is not one of the Questionnaire answerOption values"),
        ))
    }
}

fn answer_as_coded(answer: &Value) -> Option<CodedValue> {
    let value = answer_value(answer)?;
    match value {
        Value::String(s) => Some(CodedValue::Code(s.clone())),
        Value::Object(o) if o.contains_key("coding") => {
            Some(CodedValue::CodeableConcept(value.clone()))
        }
        Value::Object(_) => Some(CodedValue::Coding(value.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::TerminologyError;
    use serde_json::json;

    /// Accepts exactly one code, whatever the ValueSet.
    struct StubTerminology {
        allowed: &'static str,
    }

    #[async_trait::async_trait]
    impl TerminologyProvider for StubTerminology {
        async fn validate_code(
            &self,
            _value_set: &str,
            coded: &CodedValue,
        ) -> Result<bool, TerminologyError> {
            let code = match coded {
                CodedValue::Code(c) => Some(c.as_str()),
                CodedValue::Coding(v) => v.get("code").and_then(Value::as_str),
                CodedValue::CodeableConcept(v) => v
                    .get("coding")
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("code"))
                    .and_then(Value::as_str),
            };
            Ok(code == Some(self.allowed))
        }
    }

    #[tokio::test]
    async fn missing_required_item() {
        let q = json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [{
                "linkId": "name",
                "type": "string",
                "required": true
            }]
        });
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "questionnaire": "http://example.org/q",
            "item": []
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(
            errors.iter().any(|e| e.message.contains("required")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn answer_type_mismatch() {
        let q = json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [{ "linkId": "age", "type": "integer" }]
        });
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{
                "linkId": "age",
                "answer": [{ "valueString": "12" }]
            }]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(
            errors.iter().any(|e| e.message.contains("valueString")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn unknown_link_id() {
        let q = json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [{ "linkId": "a", "type": "string" }]
        });
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{ "linkId": "nope", "answer": [{ "valueString": "x" }] }]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(
            errors.iter().any(|e| e.message.contains("not defined")),
            "{errors:?}"
        );
    }

    fn smoker_questionnaire(enable_behavior: Option<&str>) -> Value {
        let mut packs = json!({
            "linkId": "packs",
            "type": "integer",
            "required": true,
            "enableWhen": [
                { "question": "smoker", "operator": "=", "answerBoolean": true }
            ]
        });
        if let Some(behavior) = enable_behavior {
            packs["enableBehavior"] = json!(behavior);
            packs["enableWhen"] = json!([
                { "question": "smoker", "operator": "=", "answerBoolean": true },
                { "question": "vaper", "operator": "=", "answerBoolean": true }
            ]);
        }
        json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [
                { "linkId": "smoker", "type": "boolean" },
                { "linkId": "vaper", "type": "boolean" },
                packs
            ]
        })
    }

    #[tokio::test]
    async fn enable_when_gates_required_item() {
        let q = smoker_questionnaire(None);

        // Condition unmet: the disabled required item may be absent.
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{ "linkId": "smoker", "answer": [{ "valueBoolean": false }] }]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(errors.is_empty(), "{errors:?}");

        // Condition met: the item becomes required.
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{ "linkId": "smoker", "answer": [{ "valueBoolean": true }] }]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("required Questionnaire item 'packs'")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn answers_on_disabled_item_warn() {
        let q = smoker_questionnaire(None);
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [
                { "linkId": "smoker", "answer": [{ "valueBoolean": false }] },
                { "linkId": "packs", "answer": [{ "valueInteger": 2 }] }
            ]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        let warning = errors
            .iter()
            .find(|e| e.message.contains("enableWhen is not satisfied"))
            .unwrap_or_else(|| panic!("expected disabled-item warning, got {errors:?}"));
        assert_eq!(warning.severity, Severity::Warning);
    }

    #[tokio::test]
    async fn enable_behavior_any_enables_on_one_condition() {
        let q = smoker_questionnaire(Some("any"));
        // Only the second condition (vaper) holds; behavior `any` still
        // enables the item, so its absence is a required-item error.
        let qr = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [
                { "linkId": "smoker", "answer": [{ "valueBoolean": false }] },
                { "linkId": "vaper", "answer": [{ "valueBoolean": true }] }
            ]
        });
        let errors = validate_questionnaire_response(&qr, &q, None).await;
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("required Questionnaire item 'packs'")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn answer_option_membership() {
        let q = json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [{
                "linkId": "color",
                "type": "string",
                "answerOption": [
                    { "valueString": "red" },
                    { "valueString": "blue" }
                ]
            }]
        });
        let qr_ok = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{ "linkId": "color", "answer": [{ "valueString": "red" }] }]
        });
        let errors = validate_questionnaire_response(&qr_ok, &q, None).await;
        assert!(errors.is_empty(), "{errors:?}");

        let qr_bad = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{ "linkId": "color", "answer": [{ "valueString": "green" }] }]
        });
        let errors = validate_questionnaire_response(&qr_bad, &q, None).await;
        assert!(
            errors.iter().any(|e| e.message.contains("answerOption")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn answer_value_set_membership() {
        let q = json!({
            "resourceType": "Questionnaire",
            "status": "active",
            "item": [{
                "linkId": "dx",
                "type": "coding",
                "answerValueSet": "http://example.org/ValueSet/dx-codes"
            }]
        });
        let provider = StubTerminology { allowed: "ok" };

        let qr_ok = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{
                "linkId": "dx",
                "answer": [{ "valueCoding": { "system": "http://example.org/cs", "code": "ok" } }]
            }]
        });
        let errors = validate_questionnaire_response(&qr_ok, &q, Some(&provider)).await;
        assert!(errors.is_empty(), "{errors:?}");

        let qr_bad = json!({
            "resourceType": "QuestionnaireResponse",
            "status": "completed",
            "item": [{
                "linkId": "dx",
                "answer": [{ "valueCoding": { "system": "http://example.org/cs", "code": "bad" } }]
            }]
        });
        let errors = validate_questionnaire_response(&qr_bad, &q, Some(&provider)).await;
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("not in answerValueSet")),
            "{errors:?}"
        );

        // Without a provider the check is skipped, not failed.
        let errors = validate_questionnaire_response(&qr_bad, &q, None).await;
        assert!(errors.is_empty(), "{errors:?}");
    }
}
