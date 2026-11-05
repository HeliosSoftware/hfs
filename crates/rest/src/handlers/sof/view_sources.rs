//! Supporting artifacts supplied inline via the `context` parameter.
//!
//! A SQLQuery or SQLView names the tables it selects from through its
//! `relatedArtifact` entries: each `depends-on` entry carries the dependency's
//! canonical URL in `resource` and the SQL identifier the query selects from in
//! `label`. A server may be unable to resolve every dependency — a client may
//! hold a view that exists only locally — so the repeating `context` parameter
//! carries such artifacts inline, matched to dependencies **by canonical URL**.
//!
//! `context` applies to the job as a whole rather than to one subject, so an
//! artifact several subjects depend on is supplied once.
//!
//! It deliberately has no `contextCanonical` or `contextReference` sibling,
//! even though the subject parameters come in exactly that trio: dependencies
//! are matched to the supplied entries *by* canonical URL, and the parameter
//! exists precisely for dependencies the server could not resolve, so naming
//! one by URL would hand the server back the URL it has already failed on.
//!
//! ## Matching against the transitive dependency graph
//!
//! [`super::graph::build_plan`] walks the subject's **transitive**
//! `relatedArtifact` graph — including interior SQLView nodes — one dependency
//! URL at a time, so a `context` entry fills a gap at *any* depth, not only
//! among the subject's direct dependencies. Resolution order is
//! server-first (design #568): for each dependency URL, storage is checked
//! before `context`, so a `context` entry whose URL the server can also
//! resolve is silently ignored — `context` exists to fill gaps, not to
//! override what the server already has. There is deliberately no warning
//! for this case: the operation's response is the streamed result data
//! itself, with no channel to carry an advisory OperationOutcome alongside
//! it. An artifact reached through `context` passes through exactly the same
//! post-fetch validation as one from storage (type classification, the
//! SQLView profile's `parameter 0..0` constraint, SELECT-only SQL,
//! resource structure) — the resolver does not branch on origin.
//!
//! ## Degenerate `context` entries
//!
//! Two entries sharing the same canonical `url`, or an entry whose resource
//! has no `url` at all, can never be matched unambiguously and are rejected
//! with `400 Bad Request` here, before the graph is even walked. So is a
//! request with more than 256 `context` entries
//! ([`MAX_CONTEXT_ENTRIES`](input_limits::MAX_CONTEXT_ENTRIES)), counted before
//! any of them is collected.

use std::collections::HashSet;

use serde_json::Value;

use super::input_limits;
use super::references::split_canonical_version;
use crate::error::RestError;

/// Extracts inline supporting artifacts from a `Parameters` body.
///
/// Each `context` parameter carries one inline resource, and a request may
/// carry at most [`MAX_CONTEXT_ENTRIES`](input_limits::MAX_CONTEXT_ENTRIES).
/// Matching is by canonical `url` against `relatedArtifact.resource` (see
/// [`canonical_matches`](super::references::canonical_matches)); there is no
/// name-based fallback, so an artifact without a `url` can never match a
/// dependency and is rejected outright, as is a second entry that collides
/// with an already-collected one on `url`.
pub(super) fn extract_table_source_views(body: &Value) -> Result<Vec<Value>, RestError> {
    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let count = entries
        .iter()
        .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some("context"))
        .count();
    input_limits::check_context_entries(count)?;

    let mut out: Vec<Value> = Vec::new();
    for p in &entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("context") {
            continue;
        }
        match p.get("resource").cloned() {
            Some(artifact) => out.push(artifact),
            None => {
                return Err(RestError::BadRequest {
                    message: "each `context` parameter must carry an inline resource; \
                              there is no contextCanonical or contextReference, because a \
                              dependency is matched to a context entry by canonical URL"
                        .to_string(),
                });
            }
        }
    }

    // Linear form of the pairwise `canonical_matches(prior, url)` rule: an
    // earlier entry collides when its own `url` equals this entry's canonical
    // part and, if this entry's `url` pins a version, its `version` equals that
    // pin. An unpinned entry therefore collides with any earlier `url`; a
    // pinned one only with an earlier `(url, version)` pair.
    let mut urls: HashSet<&str> = HashSet::new();
    let mut versioned: HashSet<(&str, &str)> = HashSet::new();
    for artifact in &out {
        let Some(url) = artifact.get("url").and_then(|v| v.as_str()) else {
            return Err(RestError::BadRequest {
                message: "each `context` entry's resource must declare a canonical `url`; \
                          without one it can never be matched to a dependency"
                    .to_string(),
            });
        };
        let (canonical, pin) = split_canonical_version(url);
        let duplicate = match &pin {
            None => urls.contains(canonical.as_str()),
            Some(v) => versioned.contains(&(canonical.as_str(), v.as_str())),
        };
        if duplicate {
            return Err(RestError::BadRequest {
                message: format!(
                    "duplicate `context` entry for canonical URL '{url}'; each URL may be \
                     supplied at most once, since a dependency matched against it would be \
                     ambiguous"
                ),
            });
        }
        urls.insert(url);
        if let Some(version) = artifact.get("version").and_then(|v| v.as_str()) {
            versioned.insert((url, version));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(parameter: Vec<Value>) -> Value {
        json!({"resourceType": "Parameters", "parameter": parameter})
    }

    #[test]
    fn collects_inline_context_artifacts() {
        let body = params(vec![
            json!({"name": "context", "resource": {
                "resourceType": "ViewDefinition",
                "url": "http://example.org/vd/a"
            }}),
            json!({"name": "context", "resource": {
                "resourceType": "ViewDefinition",
                "url": "http://example.org/vd/b"
            }}),
        ]);
        let views = extract_table_source_views(&body).unwrap();
        assert_eq!(views.len(), 2);
        assert_eq!(views[0]["url"], "http://example.org/vd/a");
    }

    #[test]
    fn a_context_entry_without_a_resource_is_rejected() {
        let body = params(vec![json!({
            "name": "context",
            "valueCanonical": "http://example.org/vd/a"
        })]);
        let err = extract_table_source_views(&body).unwrap_err();
        let RestError::BadRequest { message } = err else {
            panic!("expected 400");
        };
        assert!(message.contains("contextCanonical"), "{message}");
    }

    #[test]
    fn other_parameters_are_ignored() {
        let body = params(vec![
            json!({"name": "_format", "valueCode": "csv"}),
            json!({"name": "subject", "part": []}),
        ]);
        assert!(extract_table_source_views(&body).unwrap().is_empty());
    }

    #[test]
    fn a_context_entry_whose_resource_has_no_url_is_rejected() {
        let body = params(vec![json!({
            "name": "context",
            "resource": {"resourceType": "ViewDefinition"}
        })]);
        let err = extract_table_source_views(&body).unwrap_err();
        let RestError::BadRequest { message } = err else {
            panic!("expected 400");
        };
        assert!(message.contains("url"), "{message}");
    }

    #[test]
    fn two_context_entries_for_the_same_canonical_url_are_rejected() {
        let body = params(vec![
            json!({"name": "context", "resource": {
                "resourceType": "ViewDefinition",
                "url": "http://example.org/vd/dup"
            }}),
            json!({"name": "context", "resource": {
                "resourceType": "Library",
                "url": "http://example.org/vd/dup"
            }}),
        ]);
        let err = extract_table_source_views(&body).unwrap_err();
        let RestError::BadRequest { message } = err else {
            panic!("expected 400");
        };
        assert!(message.contains("duplicate"), "{message}");
        assert!(message.contains("http://example.org/vd/dup"), "{message}");
    }

    fn context_entry(url: &str) -> Value {
        json!({"name": "context", "resource": {
            "resourceType": "ViewDefinition",
            "url": url
        }})
    }

    #[test]
    fn at_most_256_context_entries_are_accepted() {
        let at_limit = params(
            (0..256)
                .map(|i| context_entry(&format!("http://example.org/vd/{i}")))
                .collect(),
        );
        assert_eq!(extract_table_source_views(&at_limit).unwrap().len(), 256);

        let over = params(
            (0..257)
                .map(|i| context_entry(&format!("http://example.org/vd/{i}")))
                .collect(),
        );
        let RestError::BadRequest { message } = extract_table_source_views(&over).unwrap_err()
        else {
            panic!("expected 400");
        };
        assert!(message.contains("256"), "{message}");
        assert!(message.contains("context"), "{message}");
    }

    #[test]
    fn a_version_pinned_url_collides_only_with_a_matching_earlier_version() {
        let earlier = json!({"name": "context", "resource": {
            "resourceType": "ViewDefinition",
            "url": "http://example.org/vd/a",
            "version": "1"
        }});

        let pinned_to_same = params(vec![
            earlier.clone(),
            context_entry("http://example.org/vd/a|1"),
        ]);
        let RestError::BadRequest { message } =
            extract_table_source_views(&pinned_to_same).unwrap_err()
        else {
            panic!("expected 400");
        };
        assert!(message.contains("duplicate"), "{message}");

        let pinned_to_other = params(vec![earlier, context_entry("http://example.org/vd/a|2")]);
        assert_eq!(
            extract_table_source_views(&pinned_to_other).unwrap().len(),
            2
        );
    }
    /// The linear duplicate check must reject exactly the pairs the pairwise
    /// `canonical_matches(earlier, later_url)` rule would.
    #[test]
    fn linear_duplicate_check_matches_the_pairwise_canonical_rule() {
        let candidates = [
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a", "version": "1"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a", "version": "2"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a|1"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a@1"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/a@1", "version": "1"}),
            json!({"resourceType": "ViewDefinition", "url": "http://e.org/vd/b"}),
        ];

        for (i, earlier) in candidates.iter().enumerate() {
            for (j, later) in candidates.iter().enumerate() {
                let body = params(vec![
                    json!({"name": "context", "resource": earlier}),
                    json!({"name": "context", "resource": later}),
                ]);
                let later_url = later["url"].as_str().unwrap();
                let expected = super::super::references::canonical_matches(earlier, later_url);
                assert_eq!(
                    extract_table_source_views(&body).is_err(),
                    expected,
                    "pair ({i}, {j}): {earlier} then {later}"
                );
            }
        }
    }
}
