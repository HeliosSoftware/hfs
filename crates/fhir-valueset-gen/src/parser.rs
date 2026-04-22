use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::models::{Bundle, CodeSystem, ValueSet};

fn resource_type(v: &Value) -> Option<&str> {
    v.get("resourceType")?.as_str()
}

fn entry_label(full_url: &Option<String>, res: &Value) -> String {
    let rt = resource_type(res).unwrap_or("<missing resourceType>");
    match full_url {
        Some(u) => format!("{rt} ({u})"),
        None => format!("{rt} (<no fullUrl>)"),
    }
}

pub fn parse_valuesets_bundle(json: &str) -> Result<(Vec<CodeSystem>, Vec<ValueSet>)> {
    let bundle: Bundle = serde_json::from_str(json).context("failed parsing Bundle")?;

    if bundle.resource_type.as_deref() != Some("Bundle") {
        return Err(anyhow!(
            "expected resourceType=Bundle but got {:?}",
            bundle.resource_type
        ));
    }

    let mut code_systems = Vec::new();
    let mut value_sets = Vec::new();

    for e in bundle.entry.unwrap_or_default() {
        let Some(res) = e.resource else {
            // Some bundles have entries without a resource; safe to skip.
            continue;
        };

        let label = entry_label(&e.full_url, &res);

        match resource_type(&res) {
            Some("CodeSystem") => {
                let cs: CodeSystem = serde_json::from_value(res)
                    .with_context(|| format!("failed parsing CodeSystem entry: {label}"))?;
                code_systems.push(cs);
            }
            Some("ValueSet") => {
                let vs: ValueSet = serde_json::from_value(res)
                    .with_context(|| format!("failed parsing ValueSet entry: {label}"))?;
                value_sets.push(vs);
            }
            Some(_) => {
                // ignore other resource types
            }
            None => {
                return Err(anyhow!("bundle entry missing resourceType: {label}"));
            }
        }
    }

    Ok((code_systems, value_sets))
}
