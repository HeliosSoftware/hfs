//! Natural-language search translation (`POST /$nl-search`, issue #255).
//!
//! Takes free text and returns a **generated FHIR search query — not
//! results**. The client reviews the query and executes it through the
//! normal search path, so auth, tenancy, audit, and search semantics stay on
//! the one code path we already trust, and the query is editable before it
//! runs.
//!
//! Three abuse-prevention layers, none optional:
//! - The checked-in system prompt (`nl_search_prompt.md`) scopes the model to
//!   translating search intent, treats user input as data, refuses off-topic
//!   requests, and translates only the FIRST request when several are packed
//!   into one input.
//! - The model's output shape is constrained by a forced, strict tool call —
//!   a jailbroken model still cannot emit a useful free-text payload.
//! - The server never trusts the output anyway: the query is parsed and every
//!   parameter validated against the SearchParameter registry before it is
//!   returned; unknown resource types or parameters fail closed.
//!
//! A per-key sliding-window rate limiter plus a daily ceiling and an input
//! length cap protect the operator's LLM credentials; limits are
//! `HFS_NL_SEARCH_*`-configurable.

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ResourceStorage, SearchProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::{PeerIp, TenantExtractor, UserKey};
use crate::fhir_types::get_resource_type_names_for_version;
use crate::state::AppState;

/// The reviewable system prompt. The live prompt appends the server's real
/// searchable vocabulary (see [`build_system_prompt`]) so it never advertises
/// a parameter this server can't execute.
pub const SYSTEM_PROMPT: &str = include_str!("nl_search_prompt.md");

/// Result-control parameters accepted in generated queries in addition to
/// registry-backed search parameters.
const CONTROL_PARAMS: &[&str] = &[
    "_count",
    "_sort",
    "_total",
    "_summary",
    "_elements",
    "_include",
    "_revinclude",
    "_contained",
];

/// Request body: the natural-language text to translate.
#[derive(Deserialize)]
pub struct NlSearchRequest {
    /// The user's natural-language search request.
    pub text: String,
}

/// The response: a query for the client to review and run, never results.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct NlSearchResponse {
    /// Whether the input was a supported FHIR search request.
    pub supported: bool,
    /// Target resource type of the generated query.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub target: String,
    /// The generated FHIR search query string (no leading ?).
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub query: String,
    /// Plain-language description of what the query does.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub explanation: String,
    /// Approximations or assumptions the translation made.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub caveats: Vec<String>,
    /// Why the request was unsupported (when supported is false).
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub reason: String,
}

/// What the model must emit, via a forced strict tool call.
#[derive(Deserialize, Debug)]
struct ModelOutput {
    supported: bool,
    #[serde(default)]
    resource_type: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    caveats: Vec<String>,
    #[serde(default)]
    reason: String,
}

/// Handler for `POST /$nl-search`.
pub async fn nl_search_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    user: UserKey,
    PeerIp(peer): PeerIp,
    Json(request): Json<NlSearchRequest>,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let config = state.config();

    // Master switch off → indistinguishable from the route not existing.
    if !config.nl_search_enabled {
        return Err(RestError::NotFound {
            resource_type: "OperationDefinition".to_string(),
            id: "nl-search".to_string(),
        });
    }
    // Advertised but unconfigured: the UI shows setup instructions instead of
    // calling this; a direct call gets a clear answer rather than a mystery.
    let Some(api_key) = config.nl_search_api_key.clone() else {
        return Err(RestError::NotImplemented {
            feature: "natural-language search (set HFS_NL_SEARCH_API_KEY)".to_string(),
        });
    };

    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Err(RestError::BadRequest {
            message: "text is required".to_string(),
        });
    }
    if text.chars().count() > config.nl_search_max_chars {
        return Err(RestError::PayloadTooLarge {
            message: format!(
                "Input is longer than {} characters",
                config.nl_search_max_chars
            ),
        });
    }

    check_rate_limit(
        &rate_limit_key(tenant.tenant_id(), &user, peer),
        config.nl_search_rate_limit,
        Duration::from_secs(config.nl_search_rate_window_secs),
        config.nl_search_daily_limit,
    )?;

    // Ground the prompt in the registry's actual vocabulary.
    let fhir_version = config.default_fhir_version;
    let system = {
        let registry = state.storage().search_param_registry(tenant.context());
        let registry = registry.read();
        build_system_prompt(fhir_version, &registry)
    };

    // Never trust the model's output as a query: parse + validate against the
    // registry before returning it.
    //
    // A model can also just have a bad sample — we have watched one leak its
    // own tool-call markup into a field. That is rare, transient, and caught by
    // the validation below, so it is worth one more roll of the dice before the
    // user sees an error. Two calls, hard maximum.
    let mut attempt = 0;
    let response = loop {
        let raw = call_llm(
            &config.nl_search_base_url,
            &api_key,
            &config.nl_search_model,
            &system,
            &text,
        )
        .await?;

        let verdict = {
            let registry = state.storage().search_param_registry(tenant.context());
            let registry = registry.read();
            validate_output(raw, fhir_version, &registry)
        };

        match verdict {
            Ok(response) => break response,
            Err(error) if attempt == 0 => {
                warn!(%error, "nl-search: unusable translation, retrying once");
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    };

    Ok(Json(response).into_response())
}

// ---------------------------------------------------------------------------
// Prompt grounding
// ---------------------------------------------------------------------------

/// Appends today's date, the FHIR version, and the registry's per-type
/// parameter vocabulary to the checked-in prompt.
///
/// The date is not decoration. Half of what a clinician asks for is relative —
/// "over 65", "in the last year", "still open" — and a model with no clock
/// guesses one, silently, and hands back a cutoff that is off by however far
/// its training data is from today. It has to be told.
///
/// Sorted and otherwise stable, so the provider's prompt cache still gets a
/// byte-identical prefix for the whole day.
pub fn build_system_prompt(
    version: helios_fhir::FhirVersion,
    registry: &helios_fhir::search::SearchParameterRegistry,
) -> String {
    build_system_prompt_on(version, registry, chrono::Utc::now().date_naive())
}

/// [`build_system_prompt`] with the date injected, so it can be tested.
pub fn build_system_prompt_on(
    version: helios_fhir::FhirVersion,
    registry: &helios_fhir::search::SearchParameterRegistry,
    today: chrono::NaiveDate,
) -> String {
    let mut prompt = String::with_capacity(64 * 1024);
    prompt.push_str(SYSTEM_PROMPT);
    prompt.push_str("\n# This server\n\nToday's date is ");
    prompt.push_str(&today.format("%Y-%m-%d").to_string());
    prompt.push_str(
        " (UTC). Compute every relative date — ages, \"in the last N days\", \"still open\" — from it. Never guess what today is.\n\nFHIR version: ",
    );
    prompt.push_str(version.as_str());
    prompt.push_str(
        "\n\nSearchable parameters by resource type (name(type); `Resource` entries apply to every type):\n\n",
    );

    let mut types = registry.resource_types();
    types.sort();
    for resource_type in types {
        let mut params: Vec<String> = registry
            .get_active_params(&resource_type)
            .iter()
            .map(|p| format!("{}({})", p.code, p.param_type))
            .collect();
        if params.is_empty() {
            continue;
        }
        params.sort();
        params.dedup();
        prompt.push_str(&resource_type);
        prompt.push_str(": ");
        prompt.push_str(&params.join(", "));
        prompt.push('\n');
    }
    prompt
}

// ---------------------------------------------------------------------------
// LLM call (Anthropic Messages API via reqwest; forced strict tool use)
// ---------------------------------------------------------------------------

fn tool_definition() -> Value {
    json!({
        "name": "emit_fhir_search",
        "description": "Emit the translated FHIR search query, or mark the request unsupported.",
        "input_schema": {
            "type": "object",
            "properties": {
                "supported": {
                    "type": "boolean",
                    "description": "false when the input is not a FHIR search request for this server"
                },
                "resource_type": {
                    "type": "string",
                    "description": "Target resource type, e.g. Patient; empty when unsupported"
                },
                "query": {
                    "type": "string",
                    "description": "FHIR search query string without the leading ?; empty when unsupported"
                },
                "explanation": {
                    "type": "string",
                    "description": "Plain-language description of what the query finds; empty when unsupported"
                },
                "caveats": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Approximations, assumptions, and ignored trailing requests"
                },
                "reason": {
                    "type": "string",
                    "description": "One-sentence refusal reason when unsupported; empty otherwise"
                }
            },
            "required": ["supported", "resource_type", "query", "explanation", "caveats", "reason"],
            "additionalProperties": false
        }
    })
}

async fn call_llm(
    base_url: &str,
    api_key: &str,
    model: &str,
    system: &str,
    text: &str,
) -> RestResult<Value> {
    let body = json!({
        "model": model,
        "max_tokens": 1024,
        // Byte-stable system prompt + cache_control: the vocabulary block is
        // large and identical across requests, so provider-side prompt
        // caching applies. The volatile user text stays in messages.
        "system": [{
            "type": "text",
            "text": system,
            "cache_control": {"type": "ephemeral"}
        }],
        "messages": [{"role": "user", "content": text}],
        "tools": [tool_definition()],
        "tool_choice": {"type": "tool", "name": "emit_fhir_search"}
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| RestError::InternalError {
            message: format!("nl-search HTTP client: {e}"),
        })?;

    let response = client
        .post(format!("{}/v1/messages", base_url.trim_end_matches('/')))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("LLM request failed: {e}"),
        })?;

    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("LLM response was not JSON: {e}"),
        })?;

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(RestError::TooManyRequests {
            message: "The translation service is rate-limited; try again shortly".to_string(),
        });
    }
    if !status.is_success() {
        warn!(status = %status, "nl-search LLM error");
        return Err(RestError::InternalError {
            message: format!(
                "LLM returned {status}: {}",
                payload["error"]["message"].as_str().unwrap_or("unknown")
            ),
        });
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Output validation — the fail-closed layer
// ---------------------------------------------------------------------------

/// Extracts the forced tool call from a Messages API response and validates
/// the generated query against the registry. Anything malformed — free text
/// instead of a tool call, an unknown resource type, an unknown parameter —
/// fails closed with a friendly error instead of passing through.
pub fn validate_output(
    payload: Value,
    version: helios_fhir::FhirVersion,
    registry: &helios_fhir::search::SearchParameterRegistry,
) -> RestResult<NlSearchResponse> {
    // The whole raw answer, once, at debug: when a translation misbehaves this
    // is the only place the truth lives, and it never leaves the server.
    debug!(payload = %payload, "nl-search raw model response");

    let output: ModelOutput = payload["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "tool_use" && b["name"] == "emit_fhir_search")
        })
        .map(|block| block["input"].clone())
        .and_then(|input| serde_json::from_value(input).ok())
        .ok_or_else(|| {
            warn!(
                stop_reason = %payload["stop_reason"],
                "nl-search: model did not answer with the emit_fhir_search tool call"
            );
            RestError::InternalError {
                message: "The translation service returned an unusable response".to_string(),
            }
        })?;

    if !output.supported {
        return Ok(NlSearchResponse {
            supported: false,
            target: String::new(),
            query: String::new(),
            explanation: String::new(),
            caveats: Vec::new(),
            reason: if output.reason.is_empty() {
                "This doesn't look like a search over this server's FHIR data".to_string()
            } else {
                output.reason
            },
        });
    }

    // Resource type must exist in this FHIR version.
    //
    // Nothing the model produced is echoed back: a malformed answer can carry
    // anything at all (we have seen tool-call markup leak into a field), and
    // this text lands in a browser. The caller gets a fixed sentence; the raw
    // value goes to the log, where diagnosing it belongs.
    let resource_type = output.resource_type.trim().to_string();
    if !get_resource_type_names_for_version(version).contains(&resource_type.as_str()) {
        warn!(
            resource_type = %resource_type,
            "nl-search: model named a resource type this server does not have"
        );
        return Err(RestError::UnprocessableEntity {
            message: "The translation named a resource type this server does not have. Try rephrasing, or write the query by hand.".to_string(),
        });
    }

    // Every parameter must resolve in the registry (or be a result control).
    let query = output.query.trim().trim_start_matches('?').to_string();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let raw_key = pair.split('=').next().unwrap_or(pair);
        let base_key = raw_key.split(':').next().unwrap_or(raw_key);
        if CONTROL_PARAMS.contains(&base_key) || base_key.starts_with("_has") {
            continue;
        }
        // Chained params (subject.name) validate on their head.
        let head = base_key.split('.').next().unwrap_or(base_key);
        if registry.get_param(&resource_type, head).is_none() {
            warn!(param = %head, resource_type = %resource_type, "nl-search rejected unknown param");
            return Err(RestError::UnprocessableEntity {
                message: format!(
                    "The translation used a search parameter {resource_type} does not have. Try rephrasing, or write the query by hand."
                ),
            });
        }
    }

    Ok(NlSearchResponse {
        supported: true,
        target: resource_type,
        query,
        explanation: output.explanation,
        caveats: output.caveats,
        reason: String::new(),
    })
}

// ---------------------------------------------------------------------------
// Rate limiting (per key sliding window + daily ceiling)
// ---------------------------------------------------------------------------

/// Who a request is billed to. An authenticated principal is the strongest
/// discriminator; without auth the peer address is all we have, and a
/// deployment with neither shares one bucket per tenant — deliberately the
/// most restrictive reading, since the operator's LLM credentials are what is
/// being spent.
fn rate_limit_key(tenant: &str, user: &UserKey, peer: Option<IpAddr>) -> String {
    if !user.is_local() {
        return format!("{tenant}|{}", user.as_str());
    }
    match peer {
        Some(ip) => format!("{tenant}|ip:{ip}"),
        None => format!("{tenant}|{}", user.as_str()),
    }
}

/// Keys are unbounded in principle (one per IP), so the map is pruned of
/// inactive buckets once it grows past this.
const RATE_MAP_PRUNE_AT: usize = 10_000;

struct RateState {
    window: VecDeque<Instant>,
    day: u64,
    day_count: u32,
}

fn rate_map() -> &'static Mutex<HashMap<String, RateState>> {
    static MAP: OnceLock<Mutex<HashMap<String, RateState>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or_default()
}

fn check_rate_limit(key: &str, limit: u32, window: Duration, daily_limit: u32) -> RestResult<()> {
    let mut map = rate_map().lock().expect("rate limiter lock");
    let now = Instant::now();
    let today = current_day();

    if map.len() > RATE_MAP_PRUNE_AT {
        map.retain(|_, state| {
            state.day == today
                && (state.day_count > 0
                    || state
                        .window
                        .back()
                        .is_some_and(|t| now.duration_since(*t) <= window))
        });
    }

    let state = map.entry(key.to_string()).or_insert_with(|| RateState {
        window: VecDeque::new(),
        day: today,
        day_count: 0,
    });

    if state.day != today {
        state.day = today;
        state.day_count = 0;
    }
    while state
        .window
        .front()
        .is_some_and(|t| now.duration_since(*t) > window)
    {
        state.window.pop_front();
    }

    if state.day_count >= daily_limit {
        return Err(RestError::TooManyRequests {
            message: "Daily natural-language search limit reached".to_string(),
        });
    }
    if state.window.len() >= limit as usize {
        return Err(RestError::TooManyRequests {
            message: "Too many natural-language searches; slow down and retry".to_string(),
        });
    }

    state.window.push_back(now);
    state.day_count += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhir::FhirVersion;
    use helios_fhir::search::{
        SearchParamType, SearchParameterDefinition, SearchParameterRegistry,
    };

    fn registry() -> SearchParameterRegistry {
        let mut registry = SearchParameterRegistry::new();
        for (url, code, ptype, base) in [
            (
                "http://hl7.org/fhir/SearchParameter/Patient-name",
                "name",
                SearchParamType::String,
                "Patient",
            ),
            (
                "http://hl7.org/fhir/SearchParameter/Patient-birthdate",
                "birthdate",
                SearchParamType::Date,
                "Patient",
            ),
            (
                "http://hl7.org/fhir/SearchParameter/Patient-gender",
                "gender",
                SearchParamType::Token,
                "Patient",
            ),
        ] {
            registry
                .register(
                    SearchParameterDefinition::new(url, code, ptype, "x").with_base(vec![base]),
                )
                .expect("registers");
        }
        registry
    }

    fn llm_payload(input: Value) -> Value {
        json!({"content": [{"type": "tool_use", "name": "emit_fhir_search", "input": input}]})
    }

    #[test]
    fn valid_query_passes_validation() {
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": "Patient",
            "query": "gender=female&birthdate=le1961-07-13&_count=20",
            "explanation": "Female patients born on or before 1961-07-13.",
            "caveats": [],
            "reason": ""
        }));
        let out = validate_output(payload, FhirVersion::default(), &registry()).unwrap();
        assert!(out.supported);
        assert_eq!(out.target, "Patient");
        assert_eq!(out.query, "gender=female&birthdate=le1961-07-13&_count=20");
    }

    #[test]
    fn unknown_parameter_fails_closed() {
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": "Patient",
            "query": "favorite-color=blue",
            "explanation": "",
            "caveats": [],
            "reason": ""
        }));
        let err = validate_output(payload, FhirVersion::default(), &registry()).unwrap_err();
        assert!(matches!(err, RestError::UnprocessableEntity { .. }));
    }

    /// A model can answer through the tool call and still put nonsense in a
    /// field — we have seen its own tool-call markup leak into `resource_type`.
    /// It fails closed like any other unknown type, and, because the caller is
    /// a browser, none of that text is handed back.
    #[test]
    fn malformed_model_output_is_never_echoed_back_to_the_caller() {
        // Built, not written out: the literal sequence would be tool-call
        // markup here too.
        let junk = format!("{}parameter> <parameter name=\"supported\">true", "</");
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": junk,
            "query": "name=x",
            "explanation": "",
            "caveats": [],
            "reason": ""
        }));
        let err = validate_output(payload, FhirVersion::default(), &registry()).unwrap_err();
        assert!(matches!(err, RestError::UnprocessableEntity { .. }));

        let shown = err.to_string();
        assert!(
            !shown.contains("parameter"),
            "the model's leaked markup must not reach the browser, got: {shown}"
        );
    }

    #[test]
    fn unknown_resource_type_fails_closed() {
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": "Wizard",
            "query": "name=gandalf",
            "explanation": "",
            "caveats": [],
            "reason": ""
        }));
        let err = validate_output(payload, FhirVersion::default(), &registry()).unwrap_err();
        assert!(matches!(err, RestError::UnprocessableEntity { .. }));
    }

    /// A jailbroken model that answers with free text instead of the forced
    /// tool call never reaches the client.
    #[test]
    fn free_text_instead_of_tool_call_fails_closed() {
        let payload =
            json!({"content": [{"type": "text", "text": "Sure! Here's a poem about FHIR..."}]});
        let err = validate_output(payload, FhirVersion::default(), &registry()).unwrap_err();
        assert!(matches!(err, RestError::InternalError { .. }));
    }

    /// SQL-ish or path-traversal payloads smuggled into the query still only
    /// pass if every parameter resolves in the registry.
    #[test]
    fn injected_garbage_query_fails_closed() {
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": "Patient",
            "query": "name=x&;DROP TABLE=1",
            "explanation": "",
            "caveats": [],
            "reason": ""
        }));
        let err = validate_output(payload, FhirVersion::default(), &registry()).unwrap_err();
        assert!(matches!(err, RestError::UnprocessableEntity { .. }));
    }

    #[test]
    fn unsupported_requests_return_supported_false_not_an_error() {
        let payload = llm_payload(json!({
            "supported": false,
            "resource_type": "",
            "query": "",
            "explanation": "",
            "caveats": [],
            "reason": "That is a general question, not a search over this server's data."
        }));
        let out = validate_output(payload, FhirVersion::default(), &registry()).unwrap();
        assert!(!out.supported);
        assert!(!out.reason.is_empty());
    }

    #[test]
    fn modifiers_chains_and_controls_validate_on_the_base_param() {
        let payload = llm_payload(json!({
            "supported": true,
            "resource_type": "Patient",
            "query": "name:contains=smith&_sort=-birthdate&_include=Patient:organization",
            "explanation": "",
            "caveats": [],
            "reason": ""
        }));
        let out = validate_output(payload, FhirVersion::default(), &registry()).unwrap();
        assert!(out.supported);
    }

    #[test]
    fn rate_limit_key_prefers_the_authenticated_principal_then_the_peer_ip() {
        let addr: IpAddr = "203.0.113.7".parse().unwrap();
        let authenticated = UserKey("https://idp.example.com|user-123".to_string());
        let anonymous = UserKey("local|default".to_string());

        assert_eq!(
            rate_limit_key("acme", &authenticated, Some(addr)),
            "acme|https://idp.example.com|user-123",
            "an authenticated principal outranks the peer address"
        );
        assert_eq!(
            rate_limit_key("acme", &anonymous, Some(addr)),
            "acme|ip:203.0.113.7",
            "without auth, callers are separated by peer address"
        );
        // Two tenants never share a bucket.
        assert_ne!(
            rate_limit_key("acme", &anonymous, Some(addr)),
            rate_limit_key("mercy", &anonymous, Some(addr))
        );
    }

    #[test]
    fn rate_limiter_enforces_window_and_daily_ceiling() {
        let key = "test-tenant-rate";
        for _ in 0..3 {
            check_rate_limit(key, 3, Duration::from_secs(60), 100).unwrap();
        }
        let err = check_rate_limit(key, 3, Duration::from_secs(60), 100).unwrap_err();
        assert!(matches!(err, RestError::TooManyRequests { .. }));

        let key2 = "test-tenant-daily";
        for _ in 0..2 {
            check_rate_limit(key2, 100, Duration::from_secs(60), 2).unwrap();
        }
        let err = check_rate_limit(key2, 100, Duration::from_secs(60), 2).unwrap_err();
        assert!(matches!(err, RestError::TooManyRequests { .. }));
    }

    /// The checked-in prompt carries the abuse-prevention instructions and
    /// the grounding hook the builder appends the vocabulary to.
    #[test]
    fn prompt_hardening_is_present() {
        assert!(SYSTEM_PROMPT.contains("supported: false"));
        assert!(SYSTEM_PROMPT.contains("ignore previous instructions"));
        assert!(SYSTEM_PROMPT.contains("ONLY the first search request"));
        assert!(SYSTEM_PROMPT.contains("DATA to translate"));
    }

    /// Without today's date the model invents one: a real run produced a
    /// birthdate cutoff for "over 65" computed against a date 17 months in the
    /// past, silently. The date is part of the grounding, not a nicety.
    #[test]
    fn system_prompt_carries_todays_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 14).unwrap();
        let prompt = build_system_prompt_on(FhirVersion::default(), &registry(), today);

        assert!(prompt.contains("Today's date is 2026-07-14"));
        assert!(prompt.contains("Never guess what today is"));
    }

    #[test]
    fn system_prompt_is_grounded_in_the_registry() {
        let prompt = build_system_prompt(FhirVersion::default(), &registry());
        assert!(prompt.contains("Patient: birthdate(date), gender(token), name(string)"));
        assert!(prompt.contains(FhirVersion::default().as_str()));
    }
}
