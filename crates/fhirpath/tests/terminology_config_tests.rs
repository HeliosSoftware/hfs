//! Terminology server configuration contract (issue #217).
//!
//! The FHIRPath evaluator has no built-in terminology server. Terminology
//! operations carry codes taken from the resource under evaluation — potentially
//! patient data — to the server, so the engine only ever contacts a host the caller
//! named, via `set_terminology_server()` or `FHIRPATH_TERMINOLOGY_SERVER`.
//!
//! These tests pin both halves of that contract: unconfigured fails with an
//! actionable error instead of silently calling a public server, and a configured
//! server is the one that actually gets called.

#![cfg(feature = "R4")]

use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};

const GENDER_VS: &str = "http://hl7.org/fhir/ValueSet/administrative-gender";

/// `FHIRPATH_TERMINOLOGY_SERVER` is process-global and read at evaluation time, so
/// a developer with one exported would otherwise see the "unconfigured" tests fail
/// spuriously. Setting it from inside the test is not an option: `set_var` is
/// `unsafe` in edition 2024 and racy across the threads the test harness runs.
fn env_server_is_set() -> bool {
    std::env::var("FHIRPATH_TERMINOLOGY_SERVER")
        .map(|url| !url.trim().is_empty())
        .unwrap_or(false)
}

/// Starts an in-process terminology server that answers `$expand` for the
/// administrative-gender ValueSet, and returns its base URL.
///
/// The server is leaked rather than dropped: `Drop for MockServer` signals shutdown,
/// and it has to keep serving for the whole test. The runtime that started it is
/// leaked alongside it, which costs a few idle threads in a process that is about to
/// exit.
fn start_terminology_stub() -> String {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let runtime = tokio::runtime::Runtime::new().expect("failed to build stub runtime");

    let uri = runtime.block_on(async {
        let server = MockServer::start().await;
        let gender_system = "http://hl7.org/fhir/administrative-gender";

        Mock::given(method("GET"))
            .and(path("/ValueSet/$expand"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "ValueSet",
                "url": GENDER_VS,
                "status": "active",
                "expansion": {
                    "identifier": "urn:uuid:00000000-0000-0000-0000-000000000001",
                    "timestamp": "2024-01-01T00:00:00Z",
                    "total": 4,
                    "contains": [
                        { "system": gender_system, "code": "male", "display": "Male" },
                        { "system": gender_system, "code": "female", "display": "Female" },
                        { "system": gender_system, "code": "other", "display": "Other" },
                        { "system": gender_system, "code": "unknown", "display": "Unknown" }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        std::mem::forget(server);
        uri
    });

    std::mem::forget(runtime);
    uri
}

#[test]
fn no_terminology_server_is_configured_by_default() {
    if env_server_is_set() {
        eprintln!("SKIP: FHIRPATH_TERMINOLOGY_SERVER is set in this environment");
        return;
    }

    let context = EvaluationContext::new_empty_with_default_version();

    assert_eq!(
        context.get_terminology_server_url(),
        None,
        "the evaluator must not fall back to any built-in terminology server"
    );
}

#[test]
fn terminology_operation_without_a_server_fails_with_an_actionable_error() {
    if env_server_is_set() {
        eprintln!("SKIP: FHIRPATH_TERMINOLOGY_SERVER is set in this environment");
        return;
    }

    let context = EvaluationContext::new_empty_with_default_version();

    let err = evaluate_expression(&format!("%terminologies.expand('{}')", GENDER_VS), &context)
        .expect_err("unconfigured terminology must error, not call a default server");

    assert!(
        err.contains("No terminology server is configured"),
        "error should explain the cause, got: {err}"
    );
    assert!(
        err.contains("FHIRPATH_TERMINOLOGY_SERVER"),
        "error should name the variable to set, got: {err}"
    );
}

#[test]
fn member_of_without_a_server_fails_instead_of_returning_false() {
    if env_server_is_set() {
        eprintln!("SKIP: FHIRPATH_TERMINOLOGY_SERVER is set in this environment");
        return;
    }

    let context = EvaluationContext::new_empty_with_default_version();

    // A silent `false` here would mislabel a valid code as not a member of the
    // ValueSet, which is worse than failing: it corrupts validation results.
    let err = evaluate_expression(&format!("'male'.memberOf('{}')", GENDER_VS), &context)
        .expect_err("unconfigured memberOf() must error rather than answer false");

    assert!(
        err.contains("No terminology server is configured"),
        "error should explain the cause, got: {err}"
    );
}

#[test]
fn referencing_terminologies_without_calling_it_needs_no_server() {
    if env_server_is_set() {
        eprintln!("SKIP: FHIRPATH_TERMINOLOGY_SERVER is set in this environment");
        return;
    }

    let context = EvaluationContext::new_empty_with_default_version();

    // Naming %terminologies resolves an object; only invoking an operation on it
    // requires a server. Failing here would break expressions that never call out.
    assert!(
        evaluate_expression("%terminologies", &context).is_ok(),
        "%terminologies should resolve without a configured server"
    );
}

#[test]
fn expand_calls_the_configured_server() {
    let stub_uri = start_terminology_stub();

    // An explicitly configured server takes precedence over the environment, so this
    // test is valid whether or not FHIRPATH_TERMINOLOGY_SERVER is set.
    let mut context = EvaluationContext::new_empty_with_default_version();
    context.set_terminology_server(stub_uri.clone());

    assert_eq!(context.get_terminology_server_url(), Some(stub_uri));

    let result = evaluate_expression(
        &format!(
            "%terminologies.expand('{}').expansion.contains.count()",
            GENDER_VS
        ),
        &context,
    )
    .expect("expansion against the configured server should succeed");

    match result {
        EvaluationResult::Integer(count, ..) => assert_eq!(count, 4),
        other => panic!("expected Integer(4) from the stub expansion, got: {other:?}"),
    }
}
