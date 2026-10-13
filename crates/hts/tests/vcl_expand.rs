//! Integration tests for Value Set Composition Language (VCL) support in
//! `ValueSet/$expand`, `ValueSet/$validate-code` and
//! `ValueSet/$batch-validate-code` (SQLite backend).
//!
//! A VCL expression arrives as the implicit ValueSet URL
//! `http://fhir.org/VCL?v1=<expression>`. The anatomy CodeSystem is described
//! on [`bundles::vcl_bundle`].

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};
use serde_json::{Value, json};

const SYS: &str = bundles::VCL_CS_URL;

fn vcl_url(expr: &str) -> String {
    format!("http://fhir.org/VCL?v1={expr}")
}

/// Percent-encodes everything but unreserved characters, as a GET client must.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn collect(items: Option<&Vec<Value>>, out: &mut Vec<String>) {
    for e in items.into_iter().flatten() {
        if let Some(code) = e["code"].as_str() {
            out.push(code.to_string());
        }
        collect(e["contains"].as_array(), out);
    }
}

fn codes_of(body: &Value) -> Vec<String> {
    let mut codes = Vec::new();
    collect(body["expansion"]["contains"].as_array(), &mut codes);
    codes.sort();
    codes
}

async fn expand_with(
    app: &TestApp,
    expr: &str,
    extra: &[(&str, &str, &str)],
) -> (StatusCode, Value) {
    let url = vcl_url(expr);
    let mut entries = vec![("url", "valueUri", url.as_str())];
    entries.extend_from_slice(extra);
    app.post_fhir("/ValueSet/$expand", TestApp::params(&entries))
        .await
}

async fn expand_codes(app: &TestApp, expr: &str) -> Vec<String> {
    let (status, body) = expand_with(app, expr, &[]).await;
    assert_eq!(status, StatusCode::OK, "expand failed for {expr}: {body}");
    codes_of(&body)
}

async fn app() -> TestApp {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::vcl_bundle()).await;
    app
}

fn issue_text(body: &Value) -> String {
    body["issue"][0]["details"]["text"]
        .as_str()
        .or_else(|| body["issue"][0]["diagnostics"].as_str())
        .unwrap_or_default()
        .to_string()
}

// ── $expand: operators ─────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_is_a() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
    assert_eq!(codes, ["arm", "leg", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_is_a_without_property_extension() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})<<limb")).await;
    assert_eq!(codes, ["arm", "leg", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_exclusion_issue_example() {
    let app = app().await;
    let expr = format!("({SYS})<<body - ({SYS})<<limb");
    assert_eq!(expand_codes(&app, &expr).await, ["body", "head"]);

    // The same with paging: the exclusion must survive the paged fast paths.
    let (status, body) = expand_with(&app, &expr, &[("count", "valueInteger", "10")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(codes_of(&body), ["body", "head"]);

    // And with a text filter.
    let (status, body) = expand_with(&app, &expr, &[("filter", "valueString", "e")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(codes_of(&body), ["head"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_is_not_a() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept~<<limb")).await;
    assert_eq!(codes, ["body", "head"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_descendent_of() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept<limb")).await;
    assert_eq!(codes, ["arm", "leg"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_child_of() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept<!body")).await;
    assert_eq!(codes, ["head", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_generalizes() {
    // `>>` maps to `generalizes`: the code plus its ancestors.
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept>>arm")).await;
    assert_eq!(codes, ["arm", "body", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_property_equals_and_conjunction() {
    let app = app().await;
    assert_eq!(
        expand_codes(&app, &format!("({SYS})region=upper")).await,
        ["arm", "head"]
    );
    assert_eq!(
        expand_codes(&app, &format!("({SYS})(region=upper,paired=true)")).await,
        ["arm"]
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_regex() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})code/\"a.*\"")).await;
    assert_eq!(codes, ["arm"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_in_list() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})concept^{{arm,leg}}")).await;
    assert_eq!(codes, ["arm", "leg"]);
    // `~^` on `concept` is rejected by the translator (use `~<<` or `-`).
    let (status, body) = expand_with(&app, &format!("({SYS})concept~^{{arm,leg}}"), &[]).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_code_list_and_disjunction() {
    let app = app().await;
    assert_eq!(
        expand_codes(&app, &format!("({SYS})(arm;head)")).await,
        ["arm", "head"]
    );
    assert_eq!(
        expand_codes(&app, &format!("({SYS})<<limb;({SYS})head")).await,
        ["arm", "head", "leg", "limb"]
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_whole_system() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS})*")).await;
    assert_eq!(codes, ["arm", "body", "head", "leg", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_system_with_version() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("({SYS}|1.0)<<limb")).await;
    assert_eq!(codes, ["arm", "leg", "limb"]);
}

// ── $expand: value set references ──────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_nested_stored_value_set() {
    let app = app().await;
    let codes = expand_codes(&app, &format!("^({})", bundles::VCL_LIMBS_VS_URL)).await;
    assert_eq!(codes, ["arm", "leg", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_self_importing_value_set_does_not_loop() {
    let app = app().await;
    let (status, body) = expand_with(
        &app,
        &format!("^({})", bundles::VCL_SELF_IMPORT_VS_URL),
        &[],
    )
    .await;
    // SQLite reports a cyclic include as a warning plus an empty set; when
    // that is the only include the expansion has nothing to resolve (404).
    assert!(
        status == StatusCode::NOT_FOUND || codes_of(&body).is_empty(),
        "unexpected {status}: {body}"
    );
}

// ── $expand: transport, errors, caching ────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_get_with_percent_encoded_url() {
    let app = app().await;
    let url = vcl_url(&encode(&format!("({SYS})concept<<limb")));
    let (status, body) = app
        .get_fhir(&format!("/ValueSet/$expand?url={}", encode(&url)))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(codes_of(&body), ["arm", "leg", "limb"]);
    assert_eq!(body["url"], url);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_percent_encoded_expression_in_post() {
    let app = app().await;
    let encoded = encode(&format!("({SYS})concept<<limb"));
    assert_eq!(expand_codes(&app, &encoded).await, ["arm", "leg", "limb"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_distinct_expressions_do_not_share_cache() {
    let app = app().await;
    let a = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
    let b = expand_codes(&app, &format!("({SYS})head")).await;
    let a_again = expand_codes(&app, &format!("({SYS})concept<<limb")).await;
    assert_eq!(a, ["arm", "leg", "limb"]);
    assert_eq!(b, ["head"]);
    assert_eq!(a, a_again);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_malformed_returns_400_with_position() {
    let app = app().await;
    let (status, body) = expand_with(&app, &format!("({SYS})<<"), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["resourceType"], "OperationOutcome");
    let text = issue_text(&body);
    assert!(
        text.contains("Invalid VCL expression at position"),
        "{text}"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_mixed_operators_without_parentheses_is_400() {
    let app = app().await;
    let (status, body) = expand_with(&app, &format!("({SYS})(a,b;c)"), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_missing_system_is_400() {
    let app = app().await;
    let (status, body) = expand_with(&app, "arm", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_expand_unsupported_construct_is_501() {
    let app = app().await;
    let (status, body) = expand_with(&app, &format!("({SYS})concept!!<body"), &[]).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
}

// ── $validate-code ─────────────────────────────────────────────────────────────

async fn validate(app: &TestApp, expr: &str, code: &str) -> (StatusCode, Value) {
    let url = vcl_url(expr);
    app.post_fhir(
        "/ValueSet/$validate-code",
        TestApp::params(&[
            ("url", "valueUri", url.as_str()),
            ("system", "valueUri", SYS),
            ("code", "valueCode", code),
        ]),
    )
    .await
}

fn result_of(body: &Value) -> Option<bool> {
    body["parameter"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == "result")?["valueBoolean"]
        .as_bool()
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_validate_code_member_and_non_member() {
    let app = app().await;
    let expr = format!("({SYS})concept<<limb");
    let (status, body) = validate(&app, &expr, "arm").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_of(&body), Some(true), "{body}");

    let (status, body) = validate(&app, &expr, "head").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_of(&body), Some(false), "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_validate_code_honours_exclusion() {
    let app = app().await;
    let expr = format!("({SYS})<<body - ({SYS})<<limb");
    let (_, body) = validate(&app, &expr, "head").await;
    assert_eq!(result_of(&body), Some(true), "{body}");
    let (_, body) = validate(&app, &expr, "arm").await;
    assert_eq!(result_of(&body), Some(false), "{body}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_validate_code_unresolvable_nested_value_set_is_false() {
    let app = app().await;
    let (status, body) = validate(&app, "^(http://hts.test/vs/does-not-exist)", "arm").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_of(&body), Some(false), "{body}");
    assert!(
        body.to_string().contains("Unable_to_resolve_value_Set_"),
        "{body}"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_validate_code_malformed_is_400() {
    let app = app().await;
    let (status, body) = validate(&app, &format!("({SYS})<<"), "arm").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

// ── $batch-validate-code ───────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vcl_batch_validate_code_uses_batch_level_url() {
    let app = app().await;
    let url = vcl_url(&format!("({SYS})concept<<limb"));
    let validation = |code: &str| {
        json!({
            "name": "validation",
            "resource": {
                "resourceType": "Parameters",
                "parameter": [
                    { "name": "system", "valueUri": SYS },
                    { "name": "code", "valueCode": code }
                ]
            }
        })
    };
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "url", "valueUri": url },
            validation("arm"),
            validation("head")
        ]
    });
    let (status, out) = app
        .post_fhir("/ValueSet/$batch-validate-code", body.to_string())
        .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let results: Vec<Option<bool>> = out["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| result_of(&v["resource"]))
        .collect();
    assert_eq!(results, [Some(true), Some(false)], "{out}");
}
