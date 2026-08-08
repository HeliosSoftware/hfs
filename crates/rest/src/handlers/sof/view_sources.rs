use serde_json::Value;

use crate::error::RestError;

/// Extracts inline ViewDefinition table sources from a `Parameters` body.
///
/// Each `view` parameter must carry a `viewResource` part containing an
/// inline ViewDefinition. The sole binding rule (SoF spec #384) is
/// `viewResource.url` matched against `relatedArtifact.resource`; no
/// name-based fallback is applied.
pub(super) fn extract_table_source_views(body: &Value) -> Result<Vec<Value>, RestError> {
    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<Value> = Vec::new();
    for p in &entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("view") {
            continue;
        }
        let inline = p
            .get("part")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|part| part.get("name").and_then(|v| v.as_str()) == Some("viewResource"))
            })
            .and_then(|part| part.get("resource"))
            .cloned();

        match inline {
            Some(vd) => out.push(vd),
            None => {
                return Err(RestError::BadRequest {
                    message: "each `view` parameter must supply a `viewResource` part".to_string(),
                });
            }
        }
    }
    Ok(out)
}
