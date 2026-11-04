//! Terminology stubs for the `documentFunctions` group's `entryIn()` and
//! `resourcesIn()` cases.
//!
//! Those cases ask whether the codes of a document's resources are in
//! `http://hl7.org/fhir/ValueSet/data-absent-reason`, and expect an execution
//! error for a value set the server doesn't know. The answers below are what
//! live HTS (`https://hts.heliossoftware.com`, which the "Test FHIRPath" CI job
//! uses) returned for the same `$validate-code` requests on 2026-11-01:
//! `result = true` for the data-absent-reason code system's codes, `false` for
//! anything else, and `404` with an OperationOutcome for an unknown value set.
//!
//! Each mock matches the request body, not just the path, so a client that
//! stops sending the parameters HTS needs fails here as it would in production.
#![allow(dead_code)] // Each `tests/*.rs` is its own binary; R4 and R5 use different subsets.

use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const DATA_ABSENT_REASON_VS: &str = "http://hl7.org/fhir/ValueSet/data-absent-reason";
const DATA_ABSENT_REASON_CS: &str = "http://terminology.hl7.org/CodeSystem/data-absent-reason";
const UNKNOWN_VS: &str = "http://example.org/fhir/ValueSet/not-a-value-set";

/// The value of the named parameter in a `Parameters` request body: a
/// primitive value, or a Coding's `system`.
fn param(req: &Request, name: &str) -> Option<String> {
    let body: serde_json::Value = serde_json::from_slice(&req.body).ok()?;
    let p = body
        .get("parameter")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))?;
    ["valueUri", "valueCanonical", "valueString", "valueCode"]
        .iter()
        .find_map(|k| p.get(*k))
        .or_else(|| p.get("valueCoding").and_then(|c| c.get("system")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn validate_code_result(result: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "result", "valueBoolean": result }]
    }))
}

/// Mounts the `$validate-code` answers for the `documentFunctions` group.
///
/// They take priority over any catch-all `$validate-code` mock already on
/// `server`: wiremock tries lower priority numbers first.
pub async fn mount_document_function_mocks(server: &MockServer) {
    use wiremock::matchers::{method, path};

    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .and(|req: &Request| {
            param(req, "url").as_deref() == Some(DATA_ABSENT_REASON_VS)
                && param(req, "coding").as_deref() == Some(DATA_ABSENT_REASON_CS)
        })
        .respond_with(validate_code_result(true))
        .with_priority(1)
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .and(|req: &Request| param(req, "url").as_deref() == Some(DATA_ABSENT_REASON_VS))
        .respond_with(validate_code_result(false))
        .with_priority(2)
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .and(|req: &Request| param(req, "url").as_deref() == Some(UNKNOWN_VS))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "not-found",
                "diagnostics": format!("A definition for the value Set '{UNKNOWN_VS}' could not be found")
            }]
        })))
        .with_priority(1)
        .mount(server)
        .await;
}

/// Starts a stub terminology server with only the `documentFunctions` mocks,
/// for a suite that has no other terminology stub, and returns its base URL.
///
/// Leaked, with its runtime, for the same reason as the R5 suite's stub:
/// dropping a `MockServer` shuts it down, and it must serve the whole suite.
pub fn start_document_tx_stub() -> String {
    let runtime = tokio::runtime::Runtime::new().expect("failed to build stub runtime");
    let uri = runtime.block_on(async {
        let server = MockServer::start().await;
        mount_document_function_mocks(&server).await;
        let uri = server.uri();
        std::mem::forget(server);
        uri
    });
    std::mem::forget(runtime);
    uri
}

/// Whether a test needs the terminology stub: the `mode="tx"` cases, and the
/// `documentFunctions` cases that check value-set membership.
pub fn needs_terminology(group_name: &str, mode: &str) -> bool {
    mode == "tx" || group_name == "documentFunctions"
}
