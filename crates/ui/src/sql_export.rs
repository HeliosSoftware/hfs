//! View models for the SQL Export and Files pages (#649): shaping the
//! `$sql-export` completion manifest (a `Parameters` resource) for display.

use serde_json::Value;

/// One `output` entry of the manifest: a subject's name and its download
/// URL(s), one per shard.
pub(crate) struct ManifestOutput {
    pub name: String,
    pub locations: Vec<String>,
}

/// Every `output` parameter of the manifest, in manifest order.
pub(crate) fn manifest_outputs(manifest: &Value) -> Vec<ManifestOutput> {
    manifest
        .get("parameter")
        .and_then(Value::as_array)
        .map(|params| {
            params
                .iter()
                .filter(|p| p.get("name").and_then(Value::as_str) == Some("output"))
                .map(|p| {
                    let parts = p.get("part").and_then(Value::as_array);
                    let name = parts
                        .and_then(|parts| {
                            parts.iter().find_map(|part| {
                                (part.get("name").and_then(Value::as_str) == Some("name"))
                                    .then(|| part.get("valueString").and_then(Value::as_str))
                                    .flatten()
                            })
                        })
                        .unwrap_or_default()
                        .to_string();
                    let locations = parts
                        .map(|parts| {
                            parts
                                .iter()
                                .filter(|part| {
                                    part.get("name").and_then(Value::as_str) == Some("location")
                                })
                                .filter_map(|part| {
                                    part.get("valueUri")
                                        .and_then(Value::as_str)
                                        .map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    ManifestOutput { name, locations }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A top-level manifest parameter's primitive value, whichever `value[x]` it
/// carries.
pub(crate) fn manifest_value(manifest: &Value, name: &str) -> Option<String> {
    manifest
        .get("parameter")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|p| {
            ["valueString", "valueCode", "valueUri", "valueInstant"]
                .iter()
                .find_map(|k| p.get(*k).and_then(Value::as_str))
                .map(String::from)
                .or_else(|| p.get("valueInteger").map(|v| v.to_string()))
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn outputs_and_values_read_the_manifest_shape() {
        let manifest = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "exportId", "valueString": "job-1"},
                {"name": "_format", "valueCode": "csv"},
                {"name": "exportDuration", "valueInteger": 4},
                {"name": "output", "part": [
                    {"name": "name", "valueString": "patients"},
                    {"name": "location", "valueUri": "http://s/export/job-1/patients-0.csv"},
                    {"name": "location", "valueUri": "http://s/export/job-1/patients-1.csv"},
                ]},
                {"name": "output", "part": [
                    {"name": "name", "valueString": "obs"},
                    {"name": "location", "valueUri": "http://s/export/job-1/obs-0.csv"},
                ]},
            ]
        });
        let outputs = manifest_outputs(&manifest);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "patients");
        assert_eq!(outputs[0].locations.len(), 2);
        assert_eq!(outputs[1].locations, ["http://s/export/job-1/obs-0.csv"]);
        assert_eq!(manifest_value(&manifest, "_format").as_deref(), Some("csv"));
        assert_eq!(
            manifest_value(&manifest, "exportDuration").as_deref(),
            Some("4")
        );
        assert_eq!(manifest_value(&manifest, "missing"), None);
    }
}
