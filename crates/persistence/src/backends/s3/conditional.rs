//! Conditional-create criteria an S3 primary answers without a search index
//! (#1435).
//!
//! S3 keeps no index, so `If-None-Exist` cannot be a search. What it can be
//! is the shape bulk loads use for idempotency — `identifier`, and `_id` —
//! evaluated against the stored resources themselves: `_id` by reading the
//! object it names, `identifier` by a scan of the type's prefix that stops at
//! the second match. Any other criterion is refused before anything is read,
//! so a write never goes ahead on a precondition that was not evaluated.

use serde_json::Value;

use crate::error::{SearchError, StorageError, StorageResult};
use crate::search::conditional::{
    RESULT_PARAMS, parse_conditional_criteria, reject_empty_criterion_values,
};
use crate::search::value_parser::split_unescaped_commas;

/// What an `identifier` alternative requires of an identifier's `system`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SystemMatch {
    /// `code`: any system, or none.
    Any,
    /// `|code`: the identifier carries no system.
    Absent,
    /// `system|code` or `system|`: exactly this system.
    Exact(String),
}

/// One alternative of an `identifier` criterion, read as FHIR's token search
/// reads `[system]|[code]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdentifierToken {
    system: SystemMatch,
    value: Option<String>,
}

impl IdentifierToken {
    fn parse(raw: &str) -> Self {
        match raw.split_once('|') {
            None => Self {
                system: SystemMatch::Any,
                value: Some(raw.to_string()),
            },
            Some((system, value)) => Self {
                system: if system.is_empty() {
                    SystemMatch::Absent
                } else {
                    SystemMatch::Exact(system.to_string())
                },
                value: (!value.is_empty()).then(|| value.to_string()),
            },
        }
    }

    fn matches(&self, identifier: &Value) -> bool {
        let system = identifier.get("system").and_then(Value::as_str);
        let value = identifier.get("value").and_then(Value::as_str);
        let system_matches = match &self.system {
            SystemMatch::Any => true,
            SystemMatch::Absent => system.is_none(),
            SystemMatch::Exact(expected) => system == Some(expected.as_str()),
        };
        let value_matches = match &self.value {
            None => true,
            Some(expected) => value == Some(expected.as_str()),
        };
        system_matches && value_matches
    }
}

/// The criteria of one conditional create, reduced to what a scan can decide.
/// Every criterion must hold (FHIR ANDs repeated parameters); within one, any
/// alternative may (an OR-list).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanCriteria {
    ids: Vec<Vec<String>>,
    identifiers: Vec<Vec<IdentifierToken>>,
}

impl ScanCriteria {
    /// Parses form-urlencoded criteria, refusing an empty value and any
    /// parameter other than `_id` and `identifier` (result-shaping ones such
    /// as `_format` are ignored, as direct search ignores them).
    pub(crate) fn parse(criteria: &str) -> StorageResult<Self> {
        let pairs = parse_conditional_criteria(criteria);
        reject_empty_criterion_values(&pairs)?;
        let mut parsed = Self::default();
        for (name, value) in &pairs {
            if RESULT_PARAMS.contains(&name.as_str()) {
                continue;
            }
            match name.as_str() {
                "_id" => parsed.ids.push(split_unescaped_commas(value)),
                "identifier" => parsed.identifiers.push(
                    split_unescaped_commas(value)
                        .iter()
                        .map(|alternative| IdentifierToken::parse(alternative))
                        .collect(),
                ),
                other => {
                    return Err(StorageError::Search(SearchError::QueryParseError {
                        message: format!(
                            "conditional create on the S3 storage backend evaluates only \
                             `identifier` and `_id` criteria; `{other}` cannot be evaluated \
                             without a search index"
                        ),
                    }));
                }
            }
        }
        Ok(parsed)
    }

    /// No criterion at all: nothing can match, the create goes ahead.
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.identifiers.is_empty()
    }

    /// When `_id` criteria alone name the candidates, the ids to read instead
    /// of scanning the type: the intersection of every `_id` OR-list. `None`
    /// when an `identifier` criterion needs the scan anyway.
    pub(crate) fn candidate_ids(&self) -> Option<Vec<String>> {
        if !self.identifiers.is_empty() {
            return None;
        }
        let mut groups = self.ids.iter();
        let mut candidates: Vec<String> = groups.next()?.clone();
        for group in groups {
            candidates.retain(|id| group.contains(id));
        }
        candidates.sort();
        candidates.dedup();
        Some(candidates)
    }

    /// Whether `resource` (its FHIR JSON) satisfies every criterion.
    pub(crate) fn matches(&self, resource: &Value) -> bool {
        let id = resource.get("id").and_then(Value::as_str);
        if !self
            .ids
            .iter()
            .all(|group| id.is_some_and(|id| group.iter().any(|wanted| wanted == id)))
        {
            return false;
        }
        let identifiers = resource
            .get("identifier")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        self.identifiers.iter().all(|alternatives| {
            identifiers.iter().any(|identifier| {
                alternatives
                    .iter()
                    .any(|alternative| alternative.matches(identifier))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patient(identifiers: Value) -> Value {
        json!({"resourceType": "Patient", "id": "p1", "identifier": identifiers})
    }

    #[test]
    fn a_bare_code_matches_any_system_and_a_qualified_one_only_its_own() {
        let with_system = patient(json!([{"system": "http://example.org/mrn", "value": "123"}]));
        let without_system = patient(json!([{"value": "123"}]));
        let system_no_value = patient(json!([{"system": "http://example.org/mrn"}]));

        let any = ScanCriteria::parse("identifier=123").unwrap();
        assert!(any.matches(&with_system));
        assert!(any.matches(&without_system));

        let qualified =
            ScanCriteria::parse("identifier=http%3A%2F%2Fexample.org%2Fmrn%7C123").unwrap();
        assert!(qualified.matches(&with_system));
        assert!(!qualified.matches(&without_system));

        let other_system = ScanCriteria::parse("identifier=http://other|123").unwrap();
        assert!(!other_system.matches(&with_system));

        let no_system = ScanCriteria::parse("identifier=|123").unwrap();
        assert!(!no_system.matches(&with_system));
        assert!(no_system.matches(&without_system));

        let system_only = ScanCriteria::parse("identifier=http://example.org/mrn|").unwrap();
        assert!(system_only.matches(&with_system));
        assert!(system_only.matches(&system_no_value));
        assert!(!system_only.matches(&without_system));
    }

    #[test]
    fn alternatives_or_and_repeated_criteria_and() {
        let resource = patient(json!([
            {"system": "http://a", "value": "1"},
            {"system": "http://b", "value": "2"}
        ]));
        assert!(
            ScanCriteria::parse("identifier=9,1")
                .unwrap()
                .matches(&resource)
        );
        assert!(
            !ScanCriteria::parse("identifier=9,8")
                .unwrap()
                .matches(&resource)
        );
        assert!(
            ScanCriteria::parse("identifier=http://a|1&identifier=http://b|2")
                .unwrap()
                .matches(&resource)
        );
        assert!(
            !ScanCriteria::parse("identifier=http://a|1&identifier=http://b|9")
                .unwrap()
                .matches(&resource)
        );
        // A value carrying an escaped comma is one alternative, not two.
        let commas = patient(json!([{"value": "a,b"}]));
        assert!(
            ScanCriteria::parse("identifier=a%5C%2Cb")
                .unwrap()
                .matches(&commas)
        );
        assert!(
            !ScanCriteria::parse("identifier=a,b")
                .unwrap()
                .matches(&commas)
        );
    }

    #[test]
    fn id_criteria_name_the_candidates_unless_an_identifier_needs_the_scan() {
        let by_id = ScanCriteria::parse("_id=p1,p2&_id=p2,p3").unwrap();
        assert_eq!(by_id.candidate_ids(), Some(vec!["p2".to_string()]));
        assert!(by_id.matches(&json!({"resourceType": "Patient", "id": "p2"})));
        assert!(!by_id.matches(&json!({"resourceType": "Patient", "id": "p1"})));

        let mixed = ScanCriteria::parse("_id=p1&identifier=1").unwrap();
        assert_eq!(mixed.candidate_ids(), None);
        assert!(mixed.matches(&patient(json!([{"value": "1"}]))));
        assert!(!mixed.matches(&json!({"resourceType": "Patient", "id": "p1"})));

        assert!(ScanCriteria::parse("_format=json").unwrap().is_empty());
        assert_eq!(ScanCriteria::parse("").unwrap().candidate_ids(), None);
    }

    #[test]
    fn anything_a_scan_cannot_evaluate_is_refused_before_reading() {
        for criteria in [
            "active=true",
            "identifier:exact=1",
            "name=Smith&identifier=1",
            "_has:Observation:subject:code=x",
        ] {
            match ScanCriteria::parse(criteria) {
                Err(StorageError::Search(SearchError::QueryParseError { message })) => {
                    assert!(message.contains("identifier"), "{criteria}: {message}");
                }
                other => panic!("{criteria}: expected a query parse error, got {other:?}"),
            }
        }
        // An empty value would widen the precondition (#1360); refused too.
        assert!(ScanCriteria::parse("identifier=").is_err());
        assert!(ScanCriteria::parse("identifier=1,").is_err());
    }
}
