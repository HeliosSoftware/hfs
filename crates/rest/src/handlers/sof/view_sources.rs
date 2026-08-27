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
//! with `400 Bad Request` here, before the graph is even walked.

use serde_json::Value;

use super::references::canonical_matches;
use crate::error::RestError;

/// Extracts inline supporting artifacts from a `Parameters` body.
///
/// Each `context` parameter carries one inline resource. Matching is by
/// canonical `url` against `relatedArtifact.resource` (see
/// [`canonical_matches`]); there is no name-based fallback, so an artifact
/// without a `url` can never match a dependency and is rejected outright, as
/// is a second entry that collides with an already-collected one on `url`.
pub(super) fn extract_table_source_views(body: &Value) -> Result<Vec<Value>, RestError> {
    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

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

    for (i, artifact) in out.iter().enumerate() {
        let Some(url) = artifact.get("url").and_then(|v| v.as_str()) else {
            return Err(RestError::BadRequest {
                message: "each `context` entry's resource must declare a canonical `url`; \
                          without one it can never be matched to a dependency"
                    .to_string(),
            });
        };
        if out[..i].iter().any(|prior| canonical_matches(prior, url)) {
            return Err(RestError::BadRequest {
                message: format!(
                    "duplicate `context` entry for canonical URL '{url}'; each URL may be \
                     supplied at most once, since a dependency matched against it would be \
                     ambiguous"
                ),
            });
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
}
