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
//! ## Not yet implemented
//!
//! The spec matches `context` against the subject's **transitive**
//! `relatedArtifact` graph, whose interior nodes may be SQLViews. We resolve
//! only the subject's direct dependencies, and only ViewDefinitions. See the
//! tracking issue for SQLView support.

use serde_json::Value;

use crate::error::RestError;

/// Extracts inline supporting artifacts from a `Parameters` body.
///
/// Each `context` parameter carries one inline resource. Matching is by
/// canonical `url` against `relatedArtifact.resource`; there is no name-based
/// fallback, so an artifact without a `url` can never match a dependency.
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
}
