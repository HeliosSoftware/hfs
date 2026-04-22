//! Optional HTTP comparison with a FHIR server’s **instance** `$validate` operation.
//!
//! Default target: **HAPI** `https://hapi.fhir.org/baseR5` (or set `FHIR_ONLINE_VALIDATOR_BASE_URL`).
//! The server is third-party; use [`post_instance_validate`] only from `#[ignore]` tests.

use serde_json::Value;
use std::time::Duration;

pub fn online_validator_base_url() -> Option<String> {
    std::env::var("FHIR_ONLINE_VALIDATOR_BASE_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// POST raw JSON to `{base}/{resourceType}/$validate` (content negotiation `application/fhir+json`).
pub async fn post_instance_validate(
    resource_type: &str,
    resource_json: &str,
) -> Result<Value, String> {
    let base = online_validator_base_url().ok_or_else(|| {
        "set FHIR_ONLINE_VALIDATOR_BASE_URL (e.g. https://hapi.fhir.org/baseR5)".to_owned()
    })?;
    let url = format!("{}/{}/$validate", base.trim_end_matches('/'), resource_type);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let status = client
        .post(&url)
        .header(reqwest::header::ACCEPT, "application/fhir+json")
        .header(reqwest::header::CONTENT_TYPE, "application/fhir+json")
        .body(resource_json.to_owned())
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let code = status.status();
    let body = status.text().await.map_err(|e| e.to_string())?;
    if !code.is_success() {
        return Err(format!("HTTP {code}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("parse OperationOutcome: {e}; body: {body}"))
}

pub fn count_operation_outcome_severities(oo: &Value) -> (usize, usize, usize) {
    let mut err = 0usize;
    let mut warn = 0usize;
    let mut info = 0usize;
    let Some(issues) = oo.get("issue").and_then(|i| i.as_array()) else {
        return (0, 0, 0);
    };
    for i in issues {
        match i.get("severity").and_then(|s| s.as_str()) {
            Some("fatal" | "error") => err += 1,
            Some("warning") => warn += 1,
            Some("information") => info += 1,
            _ => {}
        }
    }
    (err, warn, info)
}
