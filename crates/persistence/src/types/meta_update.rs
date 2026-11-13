//! Carrying `meta.tag` and `meta.security` across an update (#2562).
//!
//! FHIR R4 §2.26.3.9 / R5 §2.1.27.5.3.10 ("Updates to Tags, Profiles, and
//! Security Labels"): when a resource is updated, servers generally merge
//! existing and new tags, replace existing profiles with the new profiles, and
//! merge existing and new security labels. The Resource Metadata table adds,
//! for both `tag` and `security` on RESTful writes, "the server SHOULD preserve
//! the labels unless applicable business rules dictate otherwise".
//!
//! Every backend's `update` (and its in-transaction `update`) calls
//! [`merge_update_meta`] on the incoming body before storing it, so a tag or
//! security label the current version holds survives an update whose body
//! leaves it out. `meta.profile` is not touched: the update's profiles replace
//! the current ones.

use serde_json::{Map, Value};

/// The `meta` members that an update merges rather than replaces.
const MERGED_MEMBERS: [&str; 2] = ["tag", "security"];

/// Merges `current`'s `meta.tag` and `meta.security` into `incoming`.
///
/// For each of the two lists the result holds the current version's codings in
/// their stored order — each replaced by the incoming coding with the same
/// identity, when there is one, so an update can change a label's `display` —
/// followed by the incoming codings the current version does not hold, in
/// their submitted order. A coding's identity is its `system` and `code`; a
/// coding with neither is identified by its whole value.
///
/// Nothing changes when the current version holds no tags or security labels.
/// `meta.profile` and every other `meta` member are left as submitted.
pub fn merge_update_meta(current: &Value, incoming: &mut Value) {
    let Some(current_meta) = current.get("meta").and_then(Value::as_object) else {
        return;
    };
    for member in MERGED_MEMBERS {
        let Some(existing) = current_meta
            .get(member)
            .and_then(Value::as_array)
            .filter(|existing| !existing.is_empty())
        else {
            continue;
        };
        let Some(incoming_obj) = incoming.as_object_mut() else {
            return;
        };
        let meta = incoming_obj
            .entry("meta")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(meta) = meta.as_object_mut() else {
            // A malformed `meta`; validation reports it, so leave it as sent.
            return;
        };
        let submitted = match meta.remove(member) {
            Some(Value::Array(submitted)) => submitted,
            Some(other) => {
                // Not a list: leave it for validation to report.
                meta.insert(member.to_string(), other);
                continue;
            }
            None => Vec::new(),
        };
        meta.insert(
            member.to_string(),
            Value::Array(merge_codings(existing, submitted)),
        );
    }
}

/// The current codings (each superseded by an incoming one with the same
/// identity), then the incoming codings not already present.
fn merge_codings(existing: &[Value], submitted: Vec<Value>) -> Vec<Value> {
    let mut submitted: Vec<Option<Value>> = submitted.into_iter().map(Some).collect();
    let mut merged = Vec::with_capacity(existing.len() + submitted.len());
    for coding in existing {
        let key = coding_key(coding);
        let replacement = submitted
            .iter_mut()
            .find(|candidate| candidate.as_ref().is_some_and(|c| coding_key(c) == key))
            .and_then(Option::take);
        if merged.iter().any(|kept| coding_key(kept) == key) {
            // A duplicate in the stored list; keep it as stored.
            merged.push(coding.clone());
            continue;
        }
        merged.push(replacement.unwrap_or_else(|| coding.clone()));
    }
    for coding in submitted.into_iter().flatten() {
        let key = coding_key(&coding);
        if !merged.iter().any(|kept| coding_key(kept) == key) {
            merged.push(coding);
        }
    }
    merged
}

/// Identity of a tag or security coding: `system` + `code`, or the whole value
/// when it carries neither.
#[derive(PartialEq)]
enum CodingKey<'a> {
    SystemCode(Option<&'a str>, Option<&'a str>),
    Whole(&'a Value),
}

fn coding_key(coding: &Value) -> CodingKey<'_> {
    let system = coding.get("system").and_then(Value::as_str);
    let code = coding.get("code").and_then(Value::as_str);
    if system.is_none() && code.is_none() {
        CodingKey::Whole(coding)
    } else {
        CodingKey::SystemCode(system, code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tag(code: &str) -> Value {
        json!({"system": "http://example.org/tags", "code": code})
    }

    #[test]
    fn tags_and_security_left_out_of_the_update_are_kept() {
        let current = json!({"resourceType": "Task", "meta": {
            "tag": [tag("a")],
            "security": [{"system": "http://terminology.hl7.org/CodeSystem/v3-Confidentiality", "code": "R"}],
            "profile": ["http://example.org/old"]
        }});
        let mut incoming = json!({"resourceType": "Task", "meta": {
            "tag": [tag("b")],
            "profile": ["http://example.org/new"]
        }});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(incoming["meta"]["tag"], json!([tag("a"), tag("b")]));
        assert_eq!(
            incoming["meta"]["security"],
            json!([{"system": "http://terminology.hl7.org/CodeSystem/v3-Confidentiality", "code": "R"}])
        );
        assert_eq!(
            incoming["meta"]["profile"],
            json!(["http://example.org/new"]),
            "profiles are replaced, not merged"
        );
    }

    #[test]
    fn an_update_without_meta_keeps_every_label() {
        let current = json!({"meta": {"versionId": "3", "tag": [tag("a")]}});
        let mut incoming = json!({"resourceType": "Task", "status": "requested"});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(incoming["meta"], json!({"tag": [tag("a")]}));
    }

    #[test]
    fn a_resent_label_is_not_duplicated_and_its_display_is_updated() {
        let current = json!({"meta": {"tag": [tag("a"), tag("b")]}});
        let mut incoming = json!({"meta": {"tag": [
            tag("c"),
            {"system": "http://example.org/tags", "code": "a", "display": "Alpha"}
        ]}});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(
            incoming["meta"]["tag"],
            json!([
                {"system": "http://example.org/tags", "code": "a", "display": "Alpha"},
                tag("b"),
                tag("c")
            ])
        );
    }

    #[test]
    fn same_code_in_another_system_is_a_different_label() {
        let current = json!({"meta": {"tag": [{"system": "urn:one", "code": "x"}]}});
        let mut incoming = json!({"meta": {"tag": [{"system": "urn:two", "code": "x"}]}});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(
            incoming["meta"]["tag"],
            json!([{"system": "urn:one", "code": "x"}, {"system": "urn:two", "code": "x"}])
        );
    }

    #[test]
    fn nothing_changes_when_the_current_version_has_no_labels() {
        let current = json!({"meta": {"versionId": "1", "profile": ["http://example.org/p"]}});
        let original = json!({"resourceType": "Task"});
        let mut incoming = original.clone();
        merge_update_meta(&current, &mut incoming);
        assert_eq!(incoming, original, "no empty meta is added");
    }

    #[test]
    fn malformed_incoming_meta_is_left_for_validation() {
        let current = json!({"meta": {"tag": [tag("a")]}});
        let mut incoming = json!({"meta": "nonsense"});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(incoming, json!({"meta": "nonsense"}));

        let mut incoming = json!({"meta": {"tag": "nonsense"}});
        merge_update_meta(&current, &mut incoming);
        assert_eq!(incoming, json!({"meta": {"tag": "nonsense"}}));
    }
}
