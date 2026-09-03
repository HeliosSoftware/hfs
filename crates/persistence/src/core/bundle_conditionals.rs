//! Shared handling of URL-borne conditional entries in transaction Bundles.
//!
//! A transaction Bundle may address a resource by search criteria rather than
//! by id: `PUT [type]?[criteria]` and `DELETE [type]?[criteria]`. Every
//! transaction executor (SQLite, PostgreSQL, MongoDB) has to do the same three
//! things with such an entry, and this module is the single place they are
//! written down so the backends cannot drift apart (#859):
//!
//! 1. **Resolve before any write.** R4 §3.1.0.11.2 lists the resolution of
//!    conditional identities among the steps that happen *before* the entries
//!    are processed, and the overlap rule below is only meaningful if every
//!    target is known up front. So an executor resolves every conditional
//!    entry against the transaction's starting view, pins the outcome in a
//!    [`ConditionalTarget`], and executes against that pin rather than
//!    re-resolving mid-bundle. (`ifNoneExist` is resolved in entry order
//!    instead, and sees earlier writes of the same bundle; the spec ties it to
//!    the POST it decorates rather than to this pre-pass.)
//! 2. **Fail the bundle on an overlap.** "If any resource identities
//!    (including resolved identities from conditional update/delete) overlap
//!    in steps 1-3, then the transaction SHALL fail" — [`check_identity_overlap`].
//! 3. **Answer with the resource endpoints' status mapping.** Several matches
//!    are a `412 multiple-matches` for the whole bundle
//!    ([`TransactionError::MultipleMatches`]); a delete names what it deleted
//!    through `location`, because a `204` has no body and the URL no id.
//!
//! Criteria arrive typed ([`SearchParameter`]) on [`BundleEntry::criteria`];
//! the caller parses them with the search parser, so the backends run the
//! same query builder the search endpoint runs.

use std::collections::HashMap;

use super::transaction::{BundleEntry, BundleEntryResult, BundleMethod, ConditionalTransaction};
use crate::error::TransactionError;
use crate::types::{SearchParameter, SearchQuery, StoredResource};

/// Upper bound on the matches fetched for one conditional interaction.
///
/// Two would prove non-uniqueness, but the `412` names the count it found, as
/// the resource endpoints' conditional interactions do, so the same bound they
/// use applies here.
pub const CONDITIONAL_MATCH_LIMIT: u32 = 1000;

/// The search a conditional interaction's criteria describe.
pub fn conditional_query(resource_type: &str, criteria: &[SearchParameter]) -> SearchQuery {
    SearchQuery {
        resource_type: resource_type.to_string(),
        parameters: criteria.to_vec(),
        count: Some(CONDITIONAL_MATCH_LIMIT),
        ..Default::default()
    }
}

/// The resource type a conditional entry addresses: the last path segment
/// before the `?`, with any scheme, host and server prefix stripped.
///
/// `None` when the entry carries no criteria — such an entry addresses an
/// instance and goes through the executor's instance URL parser.
pub fn conditional_resource_type(entry: &BundleEntry) -> Option<&str> {
    entry.criteria.as_ref()?;
    let path = entry
        .url
        .split_once('?')
        .map_or(entry.url.as_str(), |(p, _)| p);
    let path = strip_origin(path);
    path.rsplit('/').find(|segment| !segment.is_empty())
}

/// A conditional entry's resolution, produced inside the transaction before
/// any entry is written.
#[derive(Debug)]
pub struct ConditionalTarget {
    /// Index of the entry in the ordered list the executor received.
    pub entry_index: usize,
    /// The type the entry's URL names.
    pub resource_type: String,
    /// The single match, or `None` when the criteria matched nothing (a `PUT`
    /// then creates; a `DELETE` is a no-op `204`).
    pub resolved: Option<StoredResource>,
}

impl ConditionalTarget {
    /// The resolved identity as `Type/id`, when there is one.
    pub fn identity(&self) -> Option<String> {
        self.resolved
            .as_ref()
            .map(|r| format!("{}/{}", r.resource_type(), r.id()))
    }
}

/// Turns a conditional entry's match set into its pinned target, or into the
/// whole-bundle `412` several matches call for.
pub fn conditional_target(
    entry_index: usize,
    entry: &BundleEntry,
    resource_type: &str,
    matches: Vec<StoredResource>,
) -> Result<ConditionalTarget, TransactionError> {
    match matches.len() {
        0 | 1 => Ok(ConditionalTarget {
            entry_index,
            resource_type: resource_type.to_string(),
            resolved: matches.into_iter().next(),
        }),
        count => Err(TransactionError::MultipleMatches {
            operation: conditional_operation(entry.method).to_string(),
            count,
        }),
    }
}

/// R4 §3.1.0.11.2: a conditional entry whose resolved identity is also
/// addressed by another entry — an instance-addressed `PUT`/`DELETE`/`PATCH`,
/// or another conditional entry that resolved to the same resource — fails
/// the bundle.
///
/// Two instance-addressed entries naming the same id are not reported here:
/// that has never been detected on this path, and the entries execute in
/// order as they always did. The rule is enforced where a *resolved* identity
/// is involved, which is what the conditional pre-pass exists to see.
pub fn check_identity_overlap(
    entries: &[BundleEntry],
    targets: &[ConditionalTarget],
) -> Result<(), TransactionError> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.criteria.is_some()
            || !matches!(
                entry.method,
                BundleMethod::Put | BundleMethod::Delete | BundleMethod::Patch
            )
        {
            continue;
        }
        if let Some(identity) = instance_identity(&entry.url) {
            seen.entry(identity).or_insert(index);
        }
    }

    for target in targets {
        let Some(identity) = target.identity() else {
            continue;
        };
        if let Some(&other) = seen.get(&identity) {
            let entry = &entries[target.entry_index];
            let other_entry = &entries[other];
            return Err(TransactionError::BundleError {
                index: target.entry_index,
                message: format!(
                    "{} {} resolves to {identity}, which entry {other} ({} {}) also \
                     addresses; a transaction whose resolved identities overlap \
                     fails as a whole (R4 §3.1.0.11.2)",
                    entry.method, entry.url, other_entry.method, other_entry.url
                ),
            });
        }
        seen.insert(identity, target.entry_index);
    }
    Ok(())
}

/// Resolves every conditional entry of a transaction against `tx`'s view,
/// before any entry is written, and enforces [`check_identity_overlap`].
///
/// `unsupported` names why this backend cannot evaluate criteria inside its
/// transaction (search offloaded to a secondary, so the local index is empty);
/// when set, the first conditional entry fails the bundle with that reason
/// rather than resolving to "no match" and duplicating.
///
/// Returns the targets keyed by entry index; an entry without criteria has no
/// target and executes through the instance path as before.
pub async fn resolve_conditional_targets<T>(
    tx: &mut T,
    entries: &[BundleEntry],
    unsupported: Option<&str>,
) -> Result<HashMap<usize, ConditionalTarget>, TransactionError>
where
    T: ConditionalTransaction + ?Sized,
{
    let mut targets = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(criteria) = entry.criteria.as_deref() else {
            continue;
        };
        if let Some(reason) = unsupported {
            return Err(unsupported_conditional_entry(index, reason));
        }
        let resource_type = conditional_resource_type(entry)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| TransactionError::BundleError {
                index,
                message: format!("Entry request.url '{}' names no resource type", entry.url),
            })?
            .to_string();
        let matches = tx
            .find_matching(&resource_type, criteria)
            .await
            .map_err(|e| TransactionError::BundleError {
                index,
                message: format!("Entry processing failed: {e}"),
            })?;
        targets.push(conditional_target(index, entry, &resource_type, matches)?);
    }
    check_identity_overlap(entries, &targets)?;
    Ok(targets
        .into_iter()
        .map(|target| (target.entry_index, target))
        .collect())
}

/// The `200` a conditional update answers when its criteria matched.
///
/// Carries `location`, as the batch arm's and `ifNoneExist`'s `200`s do, so a
/// `urn:uuid` reference to the entry resolves to the match.
pub fn conditional_update_entry(updated: StoredResource) -> BundleEntryResult {
    let location = updated.versioned_url();
    let mut result = BundleEntryResult::ok(updated);
    result.location = Some(location);
    result
}

/// The `204` a conditional delete answers, naming the resource it deleted
/// through `location` (the version that was current when it was deleted).
///
/// A `204` has no body and a criteria URL no id, so without this the audit
/// trail and a composite's secondary sync would have nothing to name.
pub fn conditional_delete_entry(deleted: &StoredResource) -> BundleEntryResult {
    let mut result = BundleEntryResult::deleted();
    result.location = Some(deleted.versioned_url());
    result
}

/// The whole-bundle error for a conditional entry a backend cannot evaluate
/// inside its transaction, carrying the `501` the entry would have answered.
pub fn unsupported_conditional_entry(index: usize, diagnostics: &str) -> TransactionError {
    TransactionError::BundleError {
        index,
        message: format!("Entry failed with status 501: {diagnostics}"),
    }
}

fn conditional_operation(method: BundleMethod) -> &'static str {
    match method {
        BundleMethod::Put | BundleMethod::Patch => "update",
        BundleMethod::Delete => "delete",
        BundleMethod::Post => "create",
        BundleMethod::Get => "read",
    }
}

fn strip_origin(url: &str) -> &str {
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    match without_scheme {
        Some(rest) => rest.find('/').map(|i| &rest[i..]).unwrap_or(""),
        None => url,
    }
}

/// `Type/id` of an instance-addressed entry URL, or `None` for a type-level
/// one. Tolerates a server prefix and a `/_history/{v}` suffix.
fn instance_identity(url: &str) -> Option<String> {
    let path = url.split_once('?').map_or(url, |(p, _)| p);
    let segments: Vec<&str> = strip_origin(path)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let type_index = segments
        .iter()
        .rposition(|s| s.chars().next().is_some_and(|c| c.is_ascii_uppercase()))?;
    let id = segments.get(type_index + 1)?;
    Some(format!("{}/{}", segments[type_index], id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::TenantId;
    use crate::types::SearchParamType;

    fn entry(method: BundleMethod, url: &str, conditional: bool) -> BundleEntry {
        BundleEntry {
            method,
            url: url.to_string(),
            criteria: conditional.then(|| {
                vec![SearchParameter {
                    name: "identifier".to_string(),
                    param_type: SearchParamType::Token,
                    ..Default::default()
                }]
            }),
            ..Default::default()
        }
    }

    fn stored(resource_type: &str, id: &str) -> StoredResource {
        StoredResource::new(
            resource_type,
            id,
            TenantId::new("t"),
            serde_json::json!({"resourceType": resource_type, "id": id}),
            helios_fhir::FhirVersion::R4,
        )
    }

    fn target(index: usize, resolved: Option<StoredResource>) -> ConditionalTarget {
        ConditionalTarget {
            entry_index: index,
            resource_type: "Patient".to_string(),
            resolved,
        }
    }

    #[test]
    fn resource_type_is_the_segment_before_the_query() {
        for url in [
            "Patient?identifier=x",
            "/Patient?identifier=x",
            "http://example.org/fhir/Patient?identifier=x",
        ] {
            let e = entry(BundleMethod::Put, url, true);
            assert_eq!(conditional_resource_type(&e), Some("Patient"), "{url}");
        }
        let plain = entry(BundleMethod::Put, "Patient/1", false);
        assert_eq!(conditional_resource_type(&plain), None);
    }

    #[test]
    fn instance_identity_tolerates_prefix_and_history() {
        assert_eq!(instance_identity("Patient/1"), Some("Patient/1".into()));
        assert_eq!(
            instance_identity("/fhir/Patient/1/_history/3"),
            Some("Patient/1".into())
        );
        assert_eq!(
            instance_identity("https://h/fhir/Patient/1?_format=json"),
            Some("Patient/1".into())
        );
        assert_eq!(instance_identity("Patient"), None);
        assert_eq!(instance_identity("Patient?identifier=x"), None);
    }

    #[test]
    fn several_matches_are_a_412_naming_the_operation() {
        let e = entry(BundleMethod::Delete, "Patient?identifier=x", true);
        let err = conditional_target(
            0,
            &e,
            "Patient",
            vec![stored("Patient", "a"), stored("Patient", "b")],
        )
        .expect_err("two matches");
        match err {
            TransactionError::MultipleMatches { operation, count } => {
                assert_eq!(operation, "delete");
                assert_eq!(count, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
        let none = conditional_target(0, &e, "Patient", vec![]).expect("no match is fine");
        assert!(none.resolved.is_none());
    }

    #[test]
    fn resolved_identity_colliding_with_an_instance_entry_fails() {
        let entries = vec![
            entry(BundleMethod::Put, "Patient/p1", false),
            entry(BundleMethod::Put, "Patient?identifier=x", true),
        ];
        let targets = vec![target(1, Some(stored("Patient", "p1")))];
        let err = check_identity_overlap(&entries, &targets).expect_err("overlap");
        match err {
            TransactionError::BundleError { index, message } => {
                assert_eq!(index, 1);
                assert!(message.contains("Patient/p1"), "{message}");
                assert!(message.contains("entry 0"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn two_conditional_entries_resolving_to_one_resource_fail() {
        let entries = vec![
            entry(BundleMethod::Delete, "Patient?identifier=x", true),
            entry(BundleMethod::Put, "Patient?identifier=x", true),
        ];
        let targets = vec![
            target(0, Some(stored("Patient", "p1"))),
            target(1, Some(stored("Patient", "p1"))),
        ];
        let err = check_identity_overlap(&entries, &targets).expect_err("overlap");
        assert!(
            matches!(err, TransactionError::BundleError { index: 1, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn distinct_ids_gets_and_unresolved_targets_do_not_overlap() {
        let entries = vec![
            entry(BundleMethod::Get, "Patient/p1", false),
            entry(BundleMethod::Put, "Patient/p2", false),
            entry(BundleMethod::Put, "Patient?identifier=x", true),
            entry(BundleMethod::Delete, "Patient?identifier=y", true),
        ];
        let targets = vec![target(2, Some(stored("Patient", "p1"))), target(3, None)];
        check_identity_overlap(&entries, &targets).expect("no overlap");
    }

    #[test]
    fn update_entry_names_the_updated_version() {
        let result = conditional_update_entry(stored("Patient", "p1"));
        assert_eq!(result.status, 200);
        assert_eq!(result.location.as_deref(), Some("Patient/p1/_history/1"));
        assert_eq!(
            result.resource.as_ref().and_then(|r| r["id"].as_str()),
            Some("p1")
        );
    }

    #[test]
    fn delete_entry_names_the_deleted_version() {
        let result = conditional_delete_entry(&stored("Patient", "p1"));
        assert_eq!(result.status, 204);
        assert_eq!(result.location.as_deref(), Some("Patient/p1/_history/1"));
        assert!(result.resource.is_none());
    }
}
