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
//!
//! [`apply_meta_operation`] implements `$meta-add` and `$meta-delete`, the
//! operations that add and remove labels explicitly.

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

/// Which `$meta` operation [`apply_meta_operation`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaOperation {
    /// `$meta-add`: add the profiles, tags and security labels not already
    /// present.
    Add,
    /// `$meta-delete`: remove the profiles, tags and security labels named.
    Delete,
}

/// Applies a `$meta-add` or `$meta-delete` `meta` to `content`, returning
/// whether anything changed.
///
/// FHIR's `Resource-meta-add` / `Resource-meta-delete` operations: profiles,
/// tags and security labels are sets. A tag or security label is identified by
/// `system` + `code`, with `version` and `display` ignored; a profile by its
/// full canonical URL. Adding one already present and deleting one that is
/// absent are not errors — they change nothing. A list that a delete empties is
/// removed, and so is a `meta` left with no members.
///
/// # Errors
///
/// A description of the problem when `meta` is not a `Meta`-shaped object:
/// `profile` must be a list of strings, `tag` and `security` lists of objects.
pub fn apply_meta_operation(
    content: &mut Value,
    meta: &Value,
    operation: MetaOperation,
) -> Result<bool, String> {
    let meta = meta
        .as_object()
        .ok_or_else(|| "'meta' must be a Meta object".to_string())?;
    let profiles = list(meta, "profile")?;
    if let Some(bad) = profiles.iter().find(|p| !p.is_string()) {
        return Err(format!(
            "meta.profile entries must be canonical URLs, got {bad}"
        ));
    }
    let tags = list(meta, "tag")?;
    let security = list(meta, "security")?;
    for (member, codings) in [("tag", tags), ("security", security)] {
        if let Some(bad) = codings.iter().find(|c| !c.is_object()) {
            return Err(format!("meta.{member} entries must be Codings, got {bad}"));
        }
    }

    let Some(resource) = content.as_object_mut() else {
        return Err("the stored resource is not a JSON object".to_string());
    };
    let mut changed = false;
    match operation {
        MetaOperation::Add => {
            let target = resource
                .entry("meta")
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(target) = target.as_object_mut() else {
                return Err("the stored resource's meta is not an object".to_string());
            };
            changed |= add_items(target, "profile", profiles, |a, b| a == b);
            changed |= add_items(target, "tag", tags, same_coding);
            changed |= add_items(target, "security", security, same_coding);
        }
        MetaOperation::Delete => {
            let Some(target) = resource.get_mut("meta").and_then(Value::as_object_mut) else {
                return Ok(false);
            };
            changed |= delete_items(target, "profile", profiles, |a, b| a == b);
            changed |= delete_items(target, "tag", tags, same_coding);
            changed |= delete_items(target, "security", security, same_coding);
            if target.is_empty() {
                resource.remove("meta");
            }
        }
    }
    Ok(changed)
}

/// `meta.<member>` as a slice; absent is empty, anything but a list an error.
fn list<'a>(meta: &'a Map<String, Value>, member: &str) -> Result<&'a [Value], String> {
    match meta.get(member) {
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(other) => Err(format!("meta.{member} must be a list, got {other}")),
    }
}

fn same_coding(a: &Value, b: &Value) -> bool {
    coding_key(a) == coding_key(b)
}

fn add_items(
    meta: &mut Map<String, Value>,
    member: &str,
    items: &[Value],
    same: impl Fn(&Value, &Value) -> bool,
) -> bool {
    if items.is_empty() {
        return false;
    }
    let existing = meta
        .entry(member)
        .or_insert_with(|| Value::Array(Vec::new()));
    if !existing.is_array() {
        // A malformed stored list is replaced by the items added.
        *existing = Value::Array(Vec::new());
    }
    let Some(existing) = existing.as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for item in items {
        if !existing.iter().any(|present| same(present, item)) {
            existing.push(item.clone());
            changed = true;
        }
    }
    changed
}

fn delete_items(
    meta: &mut Map<String, Value>,
    member: &str,
    items: &[Value],
    same: impl Fn(&Value, &Value) -> bool,
) -> bool {
    if items.is_empty() {
        return false;
    }
    let Some(existing) = meta.get_mut(member).and_then(Value::as_array_mut) else {
        return false;
    };
    let before = existing.len();
    existing.retain(|present| !items.iter().any(|item| same(present, item)));
    let changed = existing.len() != before;
    if existing.is_empty() {
        meta.remove(member);
    }
    changed
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

    #[test]
    fn meta_add_adds_only_what_is_missing() {
        let mut content = json!({"resourceType": "Task", "meta": {
            "versionId": "2",
            "tag": [tag("a")],
            "profile": ["http://example.org/p1"]
        }});
        let add = json!({
            "tag": [
                {"system": "http://example.org/tags", "code": "a", "display": "ignored"},
                tag("b")
            ],
            "security": [{"system": "urn:sec", "code": "R"}],
            "profile": ["http://example.org/p1", "http://example.org/p2"]
        });
        assert_eq!(
            apply_meta_operation(&mut content, &add, MetaOperation::Add),
            Ok(true)
        );
        assert_eq!(content["meta"]["tag"], json!([tag("a"), tag("b")]));
        assert_eq!(
            content["meta"]["security"],
            json!([{"system": "urn:sec", "code": "R"}])
        );
        assert_eq!(
            content["meta"]["profile"],
            json!(["http://example.org/p1", "http://example.org/p2"])
        );
        assert_eq!(
            apply_meta_operation(&mut content, &add, MetaOperation::Add),
            Ok(false),
            "adding again changes nothing"
        );
    }

    #[test]
    fn meta_add_creates_meta_when_absent() {
        let mut content = json!({"resourceType": "Task"});
        let add = json!({"tag": [tag("a")]});
        assert_eq!(
            apply_meta_operation(&mut content, &add, MetaOperation::Add),
            Ok(true)
        );
        assert_eq!(content["meta"], json!({"tag": [tag("a")]}));
    }

    #[test]
    fn meta_delete_matches_system_and_code_and_drops_emptied_lists() {
        let mut content = json!({"resourceType": "Task", "meta": {
            "tag": [
                {"system": "http://example.org/tags", "code": "a", "display": "Alpha", "version": "1"},
                tag("b")
            ],
            "security": [{"system": "urn:sec", "code": "R"}],
            "profile": ["http://example.org/p1"]
        }});
        let delete = json!({
            "tag": [tag("a"), tag("missing")],
            "security": [{"system": "urn:sec", "code": "R", "display": "ignored"}],
            "profile": ["http://example.org/p1"]
        });
        assert_eq!(
            apply_meta_operation(&mut content, &delete, MetaOperation::Delete),
            Ok(true)
        );
        assert_eq!(content["meta"], json!({"tag": [tag("b")]}));

        let delete = json!({"tag": [tag("b")]});
        assert_eq!(
            apply_meta_operation(&mut content, &delete, MetaOperation::Delete),
            Ok(true)
        );
        assert!(content.get("meta").is_none(), "an emptied meta is removed");
        assert_eq!(
            apply_meta_operation(&mut content, &delete, MetaOperation::Delete),
            Ok(false),
            "deleting what is absent is not an error"
        );
    }

    #[test]
    fn meta_operation_rejects_a_malformed_meta() {
        let mut content = json!({"resourceType": "Task"});
        for bad in [
            json!("x"),
            json!({"tag": "x"}),
            json!({"tag": ["x"]}),
            json!({"profile": [1]}),
        ] {
            assert!(
                apply_meta_operation(&mut content, &bad, MetaOperation::Add).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            content,
            json!({"resourceType": "Task"}),
            "nothing was applied"
        );
    }
}
