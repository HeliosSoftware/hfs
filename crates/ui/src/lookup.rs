//! Shared reference-picking helpers behind Bulk Export's Patient combobox and
//! SQL Export's "Narrow it down" card (#836): Since-preset resolution, FHIR
//! instant validation, `{ResourceType}/{id}` reference canonicalization and
//! list parsing, and the `/ui/lookup/*-options` combobox search endpoints
//! themselves.
//!
//! # Why this module exists
//!
//! Before #836, all of this lived in `bulk_export.rs`, hardcoded to
//! `Patient` and to a single `#bulk-export-patients-message` swap target.
//! SQL Export's own Patients/Groups pickers needed the same search-and-pick
//! behavior against two more resource types and two more targets, so the
//! resource-agnostic parts moved here and the endpoint itself grew a
//! `target` parameter instead of being copied. Bulk Export's own behavior is
//! unchanged — it is simply this module's first caller, reached through
//! `target=bulk-export-patients` (see `bulk-export.html`).
//!
//! # The `target` parameter
//!
//! Both `/ui/lookup/*-options` endpoints are called by
//! `partials/combobox.html`'s `hx-post`, which does not know or care which
//! page it renders on. What ties a search result back to its caller's own
//! `<fieldset id="{target}">` is `?target={target}`: the response fragment's
//! `hx-swap-oob` targets `#{target}-message`, an element that only exists on
//! the page that rendered that exact combobox. `target` is validated against
//! a closed list ([`PATIENT_TARGETS`], [`GROUP_TARGETS`]) before it ever
//! reaches the template, so an unrecognized value answers a bare `400`
//! rather than echoing an attacker-chosen id into the response.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use askama::Template;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Redirect, Response},
};
use axum_htmx::HxRequest;
use chrono::{Duration, SecondsFormat, Utc};
use futures_lite::future::zip;
use helios_fhir::FhirVersion;
use serde::Deserialize;
use serde_json::Value;

use crate::bulk_export::{forward_identity, internal_api_url, no_redirect_client};
use crate::i18n::{I18n, RequestLocale};
use crate::sql_libraries;
use crate::{RequestTenant, RequestVersion, WebState, render};

// ---------------------------------------------------------------------------
// Since presets and instant validation (moved from bulk_export.rs, #836)
// ---------------------------------------------------------------------------

/// Whether `value` is a lexically valid FHIR `instant`
/// (`YYYY-MM-DDThh:mm:ss(.sss+)(Z|(+|-)hh:mm)`), including calendar-range and
/// UTC-offset checks — not merely a string `chrono` happens to parse.
pub(crate) fn has_fhir_r4_instant_lexical_form(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }

    let number = |start: usize, end: usize| {
        bytes
            .get(start..end)?
            .iter()
            .try_fold(0_u32, |value, byte| {
                byte.is_ascii_digit()
                    .then_some(value * 10 + u32::from(*byte - b'0'))
            })
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    if year == 0
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return false;
    }

    let mut zone_start = 19;
    if bytes.get(zone_start) == Some(&b'.') {
        zone_start += 1;
        let fraction_start = zone_start;
        while bytes.get(zone_start).is_some_and(u8::is_ascii_digit) {
            zone_start += 1;
        }
        if zone_start == fraction_start {
            return false;
        }
    }

    match bytes.get(zone_start) {
        Some(b'Z') => zone_start + 1 == bytes.len(),
        Some(b'+') | Some(b'-') => {
            if bytes.len() != zone_start + 6 || bytes.get(zone_start + 3) != Some(&b':') {
                return false;
            }
            let (Some(offset_hour), Some(offset_minute)) = (
                number(zone_start + 1, zone_start + 3),
                number(zone_start + 4, zone_start + 6),
            ) else {
                return false;
            };
            offset_minute <= 59 && (offset_hour <= 13 || (offset_hour == 14 && offset_minute == 0))
        }
        _ => false,
    }
}

/// Resolves a Since preset (`"day"` | `"week"` | `"month"` | `"custom"` | `""`)
/// plus its custom text into a `_since`-ready RFC 3339 instant. `"custom"`
/// with non-empty text must be a lexically valid FHIR instant that also
/// parses; `"custom"` left empty and any unrecognized preset both resolve to
/// no filter (`""`). Shared by Bulk Export's own Since field and SQL
/// Export's "Narrow it down" card (#836).
pub(crate) fn since_instant(preset: &str, custom: &str) -> Result<String, ()> {
    let ago = |d: Duration| (Utc::now() - d).to_rfc3339_opts(SecondsFormat::Secs, true);
    match preset {
        "day" => Ok(ago(Duration::days(1))),
        "week" => Ok(ago(Duration::days(7))),
        "month" => Ok(ago(Duration::weeks(4))),
        "custom" => {
            let custom = custom.trim();
            if custom.is_empty() {
                Ok(String::new())
            } else {
                has_fhir_r4_instant_lexical_form(custom)
                    .then_some(())
                    .ok_or(())
                    .and_then(|_| chrono::DateTime::parse_from_rfc3339(custom).map_err(|_| ()))
                    .map(|_| custom.to_string())
            }
        }
        _ => Ok(String::new()),
    }
}

// ---------------------------------------------------------------------------
// Reference canonicalization & parsing (moved from bulk_export.rs, #836)
// ---------------------------------------------------------------------------

/// Canonicalizes a single `{resource_type}/{id}` reference from either a bare
/// logical id or an already-prefixed reference: trims whitespace, strips a
/// leading `"{resource_type}/"` if present, and validates the remaining id
/// against HFS's logical-id grammar (1-64 characters of `[A-Za-z0-9.-]`).
/// `None` for anything else — an empty id, one over 64 characters, or one
/// carrying a character the grammar disallows.
pub(crate) fn canonical_reference(resource_type: &str, value: &str) -> Option<String> {
    let value = value.trim();
    let prefix = format!("{resource_type}/");
    let id = value.strip_prefix(prefix.as_str()).unwrap_or(value);
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(format!("{resource_type}/{id}"))
}

/// Parses `values` (each entry possibly a comma/newline-separated list, the
/// fallback textarea's own shape) into deduplicated, canonicalized
/// `{resource_type}/{id}` references, in first-seen order. `Err` the moment
/// any candidate fails [`canonical_reference`] — the caller re-renders with a
/// single validation message rather than reporting which token was invalid.
pub(crate) fn parse_reference_list(
    resource_type: &str,
    values: &[String],
) -> Result<Vec<String>, ()> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for raw in values {
        for candidate in raw.split([',', '\n', '\r']) {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                continue;
            }
            let reference = canonical_reference(resource_type, candidate).ok_or(())?;
            if seen.insert(reference.clone()) {
                refs.push(reference);
            }
        }
    }
    Ok(refs)
}

/// `application/fhir+json` `Accept` value for `version`.
pub(crate) fn fhir_json(version: FhirVersion) -> String {
    format!(
        "application/fhir+json; fhirVersion={}",
        version.as_mime_param()
    )
}

// ---------------------------------------------------------------------------
// The shared combobox result fragment
// ---------------------------------------------------------------------------

struct LookupOption {
    value: String,
    label: String,
    /// The referenced artifact's own `name` (#842's own `table_options`
    /// only — [`patient_option`]/[`group_option`] never set this; see
    /// `partials/lookup_options.html`'s own `data-name` for why it needs
    /// one of its own rather than reusing `label`, which already carries a
    /// " — ViewDefinition"/" — SQL View" suffix a JS-side alias
    /// autocomplete has no use for).
    name: Option<String>,
}

/// The closed list of `target` values [`patient_options`] accepts — one per
/// combobox that searches Patients. `bulk-export-patients` is the Bulk
/// Export builder's own field; `sql-export-patients` is SQL Export's
/// "Narrow it down" Patients field (#836).
const PATIENT_TARGETS: [&str; 2] = ["bulk-export-patients", "sql-export-patients"];

/// The closed list of `target` values [`group_options`] accepts.
const GROUP_TARGETS: [&str; 1] = ["sql-export-groups"];

/// The closed list of `target` values [`table_options`] accepts (#842) — one
/// per SQL Query/SQL View *Add table* combobox; both routes share the same
/// `id`/`target` (`#lib-tables-add-table`) since only one can ever be on
/// screen at a time.
const TABLE_TARGETS: [&str; 1] = ["lib-tables"];

#[derive(Deserialize)]
pub(crate) struct TargetQuery {
    target: String,
}

/// [`table_options`]'s own query (#842): `target`, plus the artifact
/// currently open — omitted (`?lib=new`) rather than an empty string, so
/// [`table_option`]'s `exclude` comparison never accidentally matches a
/// resource with no `id` at all.
#[derive(Deserialize)]
pub(crate) struct TableOptionsQuery {
    target: String,
    exclude: Option<String>,
}

/// Where a non-htmx submission to either lookup endpoint redirects back to —
/// a `bulk-export-*` target belongs to the Bulk Export builder, a
/// `sql-export-*` target to the SQL Export builder, and `lib-tables` (#842)
/// to SQL Queries — an arbitrary but reasonable default, since that target
/// alone gives no clue whether it was SQL Queries or SQL Views that posted
/// it, and this redirect only exists as a defensive fallback: the combobox
/// that posts here only ever fires through htmx, which always sets the
/// header this branch checks for.
fn fallback_page(target: &str) -> &'static str {
    if target.starts_with("bulk-export") {
        "/ui/bulk-export/new"
    } else if target == "lib-tables" {
        "/ui/sql/queries"
    } else {
        "/ui/sql/export/new"
    }
}

#[derive(Template)]
#[template(path = "partials/lookup_options.html")]
struct LookupOptionsFragment {
    target: String,
    options: Vec<LookupOption>,
    message: String,
    error: bool,
    /// Only ever `true` for a [`patient_options`] response — Patient name
    /// search can be runtime-downgraded ([`WebState::patient_name_search`]);
    /// [`group_options`] never sets this, since Group name search is a
    /// static, version-only fact ([`supports_group_name_search`]).
    id_only: bool,
}

fn options_response(fragment: LookupOptionsFragment) -> Response {
    let mut response = render(fragment);
    response.headers_mut().insert(
        CACHE_CONTROL,
        "private, no-store"
            .parse()
            .expect("static cache-control value"),
    );
    response
}

fn lookup_error(i18n: &I18n, target: &str, id_only: bool) -> Response {
    options_response(LookupOptionsFragment {
        target: target.to_string(),
        options: Vec::new(),
        message: i18n.t("ui-combobox-error"),
        error: true,
        id_only,
    })
}

/// Shapes a `Bundle` search-result into [`LookupOption`]s for whichever
/// resource type the caller searched (`resource_type`, parsed by `option` —
/// [`patient_option`] or [`group_option`]). `None` for a bundle that is not
/// a `searchset`, or one whose `entry` is present but not an array.
///
/// A `Bundle.entry` the server included for reasons other than a match
/// (`search.mode = "outcome"` — HFS's own warning `OperationOutcome` for an
/// ignored search parameter, for instance) is a resource of some *other*
/// type and is silently skipped, never a parse failure (#836): the two
/// existing HFS searches this backs (`Patient?identifier=`/`name=`,
/// `Group?identifier=`/`name=`) can legitimately answer a search with a mix
/// of matches and such outcomes. An entry that *does* claim to be
/// `resource_type` but still fails to parse (an invalid id, most commonly)
/// is a genuine failure — `option` returning `None` for it propagates via
/// `?` and aborts the whole parse, exactly as before.
fn search_options(
    bundle: &Value,
    resource_type: &str,
    option: impl Fn(&Value) -> Option<LookupOption>,
) -> Option<Vec<LookupOption>> {
    let bundle_obj = bundle.as_object()?;
    if bundle_obj.get("resourceType").and_then(Value::as_str) != Some("Bundle")
        || bundle_obj.get("type").and_then(Value::as_str) != Some("searchset")
    {
        return None;
    }
    let entries = match bundle_obj.get("entry") {
        None => return Some(Vec::new()),
        Some(Value::Array(entries)) => entries,
        Some(_) => return None,
    };
    let mut options = Vec::new();
    for entry in entries {
        let resource = entry.as_object()?.get("resource")?;
        if resource.get("resourceType").and_then(Value::as_str) != Some(resource_type) {
            continue;
        }
        options.push(option(resource)?);
    }
    Some(options)
}

/// Appends up to 8 total options from `source` into `options`, skipping a
/// value already `seen` — the union-with-a-cap rule both combobox endpoints
/// apply across their identifier/name search results.
fn append_options(
    source: Vec<LookupOption>,
    options: &mut Vec<LookupOption>,
    seen: &mut HashSet<String>,
) {
    for option in source {
        if options.len() >= 8 {
            break;
        }
        if seen.insert(option.value.clone()) {
            options.push(option);
        }
    }
}

// ---------------------------------------------------------------------------
// Patients (moved from bulk_export.rs, #836)
// ---------------------------------------------------------------------------

fn human_name_label(name: &Value) -> Option<String> {
    if let Some(text) = name.get("text").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    let given = name
        .get("given")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let family = name
        .get("family")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let parts: Vec<&str> = given.chain(family).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn patient_option(resource: &Value) -> Option<LookupOption> {
    if resource.get("resourceType").and_then(Value::as_str) != Some("Patient") {
        return None;
    }
    let id = resource.get("id")?.as_str()?;
    let value = canonical_reference("Patient", id)?;
    let name = resource
        .get("name")
        .and_then(Value::as_array)
        .and_then(|names| names.iter().find_map(human_name_label));
    let label = match name {
        Some(name) if !name.is_empty() => format!("{name} — {value}"),
        _ => value.clone(),
    };
    Some(LookupOption {
        value,
        label,
        name: None,
    })
}

fn patient_search_options(bundle: &Value) -> Option<Vec<LookupOption>> {
    search_options(bundle, "Patient", patient_option)
}

/// `POST /ui/lookup/patient-options` — a small HTML result fragment for the
/// progressively-enhanced Patient combobox, shared by Bulk Export
/// (`target=bulk-export-patients`) and SQL Export's "Narrow it down"
/// (`target=sql-export-patients`, #836). Behavior is unchanged from Bulk
/// Export's own pre-#836 endpoint — only the route and the `target`
/// parameter are new.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn patient_options(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    HxRequest(is_htmx): HxRequest,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TargetQuery>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    if !PATIENT_TARGETS.contains(&query.target.as_str()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = query.target;
    if !is_htmx {
        return Redirect::to(fallback_page(&target)).into_response();
    }
    let i18n = I18n::new(locale);
    let q = form_urlencoded::parse(&body)
        .find(|(key, _)| key == "q")
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default();
    let id_only = !state.patient_name_search.load(Ordering::Relaxed);
    if q.is_empty() {
        return options_response(LookupOptionsFragment {
            target,
            options: Vec::new(),
            message: String::new(),
            error: false,
            id_only,
        });
    }
    if q.chars().count() > 64 {
        return lookup_error(&i18n, &target, id_only);
    }

    let Ok(client) = no_redirect_client() else {
        return lookup_error(&i18n, &target, id_only);
    };
    let media = fhir_json(rv.0);
    let exact_ref = canonical_reference("Patient", &q);
    let mut options = Vec::new();
    let mut seen = HashSet::new();

    if let Some(reference) = &exact_ref {
        let id = reference.trim_start_matches("Patient/");
        let Ok(url) = internal_api_url(&state, &rt.id, ["Patient", id]) else {
            return lookup_error(&i18n, &target, id_only);
        };
        let request = forward_identity(
            client
                .get(url)
                .header("Accept", &media)
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        match request.send().await {
            Ok(response)
                if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) => {}
            Ok(response) if response.status().is_success() => {
                let Ok(resource) = response.json::<Value>().await else {
                    return lookup_error(&i18n, &target, id_only);
                };
                let Some(option) = patient_option(&resource) else {
                    return lookup_error(&i18n, &target, id_only);
                };
                if option.value != *reference {
                    return lookup_error(&i18n, &target, id_only);
                }
                seen.insert(option.value.clone());
                options.push(option);
            }
            Ok(_) | Err(_) => return lookup_error(&i18n, &target, id_only),
        }
    }

    let search_patients = q.chars().count() >= 2 && !q.starts_with("Patient/") && !id_only;
    let mut downgraded = id_only;
    if search_patients {
        let Ok(url) = internal_api_url(&state, &rt.id, ["Patient", "_search"]) else {
            return lookup_error(&i18n, &target, false);
        };
        let identifier_request = forward_identity(
            client
                .post(url.clone())
                .header("Accept", &media)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("identifier", q.as_str()),
                    ("_count", "9"),
                    ("_elements", "id,name"),
                ])
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        let name_request = forward_identity(
            client
                .post(url)
                .header("Accept", &media)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("name", q.as_str()),
                    ("_count", "9"),
                    ("_elements", "id,name"),
                ])
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        let (identifier_result, name_result) =
            zip(identifier_request.send(), name_request.send()).await;

        let not_implemented = matches!(
            &identifier_result,
            Ok(response) if response.status() == StatusCode::NOT_IMPLEMENTED
        ) || matches!(
            &name_result,
            Ok(response) if response.status() == StatusCode::NOT_IMPLEMENTED
        );
        if not_implemented {
            state.patient_name_search.store(false, Ordering::Relaxed);
            downgraded = true;
        } else {
            let (Ok(identifier_response), Ok(name_response)) = (identifier_result, name_result)
            else {
                return lookup_error(&i18n, &target, false);
            };
            if !identifier_response.status().is_success() || !name_response.status().is_success() {
                return lookup_error(&i18n, &target, false);
            }
            let Ok(identifier_bundle) = identifier_response.json::<Value>().await else {
                return lookup_error(&i18n, &target, false);
            };
            let Ok(name_bundle) = name_response.json::<Value>().await else {
                return lookup_error(&i18n, &target, false);
            };
            let Some(identifier_options) = patient_search_options(&identifier_bundle) else {
                return lookup_error(&i18n, &target, false);
            };
            let Some(name_options) = patient_search_options(&name_bundle) else {
                return lookup_error(&i18n, &target, false);
            };
            append_options(identifier_options, &mut options, &mut seen);
            append_options(name_options, &mut options, &mut seen);
        }
    }

    let message = if options.is_empty() {
        i18n.t("bulk-export-patient-options-empty")
    } else {
        String::new()
    };
    options_response(LookupOptionsFragment {
        target,
        options,
        message,
        error: false,
        id_only: downgraded,
    })
}

// ---------------------------------------------------------------------------
// Groups (new, #836)
// ---------------------------------------------------------------------------

fn group_option(resource: &Value) -> Option<LookupOption> {
    if resource.get("resourceType").and_then(Value::as_str) != Some("Group") {
        return None;
    }
    let id = resource.get("id")?.as_str()?;
    let value = canonical_reference("Group", id)?;
    // Unlike a Patient option's "name — reference" label, a Group option's
    // label is the bare name (or the id) — verified against the design
    // (`design/new-sql-export.png`'s "Group/diabetes-cohort" chip is the
    // *value* a selection renders, never the option list's own label).
    let label = resource
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.to_string());
    Some(LookupOption {
        value,
        label,
        name: None,
    })
}

fn group_search_options(bundle: &Value) -> Option<Vec<LookupOption>> {
    search_options(bundle, "Group", group_option)
}

/// Whether `version` defines `Group.name` as a search parameter — `false` for
/// R4/R4B, `true` for R5+ (verified against
/// `data/search-parameters-{r4,r5}.json`, #836). Unlike Patient name search,
/// this never downgrades at runtime: it is a static fact of the FHIR version
/// itself, not a capability HFS might or might not have compiled in.
///
/// `pub(crate)`: `crate::sql_export`'s "Narrow it down" card picks the
/// Groups field's hint text from the same fact, so the two agree on exactly
/// which versions can search by name.
pub(crate) fn supports_group_name_search(version: FhirVersion) -> bool {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => false,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => false,
        #[cfg(feature = "R5")]
        FhirVersion::R5 => true,
        #[cfg(feature = "R6")]
        FhirVersion::R6 => true,
    }
}

/// Sends `request`, decodes a `searchset` Bundle, and shapes it into
/// [`LookupOption`]s — the one path both the identifier and (on R5+) the
/// name search follow, so [`group_options`] only has to branch on which
/// requests it sends, not on how each answer is handled.
async fn group_search_result(request: reqwest::RequestBuilder) -> Result<Vec<LookupOption>, ()> {
    let response = request.send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let bundle = response.json::<Value>().await.map_err(|_| ())?;
    group_search_options(&bundle).ok_or(())
}

/// `POST /ui/lookup/group-options` — the Group combobox's fragment, today
/// only SQL Export's "Narrow it down" (`target=sql-export-groups`, #836).
/// Shaped like [`patient_options`] (exact id/reference read, then an
/// identifier search), but with no runtime id-only downgrade: whether the
/// name search is even attempted is decided once from the request's FHIR
/// version ([`supports_group_name_search`]).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn group_options(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    HxRequest(is_htmx): HxRequest,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TargetQuery>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    if !GROUP_TARGETS.contains(&query.target.as_str()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = query.target;
    if !is_htmx {
        return Redirect::to(fallback_page(&target)).into_response();
    }
    let i18n = I18n::new(locale);
    let q = form_urlencoded::parse(&body)
        .find(|(key, _)| key == "q")
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default();
    if q.is_empty() {
        return options_response(LookupOptionsFragment {
            target,
            options: Vec::new(),
            message: String::new(),
            error: false,
            id_only: false,
        });
    }
    if q.chars().count() > 64 {
        return lookup_error(&i18n, &target, false);
    }

    let Ok(client) = no_redirect_client() else {
        return lookup_error(&i18n, &target, false);
    };
    let media = fhir_json(rv.0);
    let exact_ref = canonical_reference("Group", &q);
    let mut options = Vec::new();
    let mut seen = HashSet::new();

    if let Some(reference) = &exact_ref {
        let id = reference.trim_start_matches("Group/");
        let Ok(url) = internal_api_url(&state, &rt.id, ["Group", id]) else {
            return lookup_error(&i18n, &target, false);
        };
        let request = forward_identity(
            client
                .get(url)
                .header("Accept", &media)
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        match request.send().await {
            Ok(response)
                if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) => {}
            Ok(response) if response.status().is_success() => {
                let Ok(resource) = response.json::<Value>().await else {
                    return lookup_error(&i18n, &target, false);
                };
                let Some(option) = group_option(&resource) else {
                    return lookup_error(&i18n, &target, false);
                };
                if option.value != *reference {
                    return lookup_error(&i18n, &target, false);
                }
                seen.insert(option.value.clone());
                options.push(option);
            }
            Ok(_) | Err(_) => return lookup_error(&i18n, &target, false),
        }
    }

    let search_groups = q.chars().count() >= 2 && !q.starts_with("Group/");
    if search_groups {
        let Ok(url) = internal_api_url(&state, &rt.id, ["Group", "_search"]) else {
            return lookup_error(&i18n, &target, false);
        };
        let identifier_request = forward_identity(
            client
                .post(url.clone())
                .header("Accept", &media)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("identifier", q.as_str()),
                    ("_count", "9"),
                    ("_elements", "id,name,identifier"),
                ])
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        let identifier_future = group_search_result(identifier_request);
        // R4/R4B define no `name` search parameter for Group at all
        // (`supports_group_name_search`), so a name request is never even
        // built on those versions — not merely skipped after the fact.
        let searched = if supports_group_name_search(rv.0) {
            let name_request = forward_identity(
                client
                    .post(url)
                    .header("Accept", &media)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("name", q.as_str()),
                        ("_count", "9"),
                        ("_elements", "id,name,identifier"),
                    ])
                    .timeout(std::time::Duration::from_secs(10)),
                &headers,
                &rt.id,
            );
            let (identifier_result, name_result) =
                zip(identifier_future, group_search_result(name_request)).await;
            identifier_result.and_then(|ids| name_result.map(|names| (ids, names)))
        } else {
            identifier_future.await.map(|ids| (ids, Vec::new()))
        };
        match searched {
            Ok((identifier_options, name_options)) => {
                append_options(identifier_options, &mut options, &mut seen);
                append_options(name_options, &mut options, &mut seen);
            }
            Err(()) => return lookup_error(&i18n, &target, false),
        }
    }

    let message = if options.is_empty() {
        i18n.t("sql-export-group-options-empty")
    } else {
        String::new()
    };
    options_response(LookupOptionsFragment {
        target,
        options,
        message,
        error: false,
        id_only: false,
    })
}

// ---------------------------------------------------------------------------
// Tables (new, #842) — the SQL Query/SQL View *Add table* combobox
// ---------------------------------------------------------------------------

/// A `Value` — either a ViewDefinition or a `sql-view` Library — into the
/// `value`/`label`/`name` shape [`table_options`] emits: `value` the
/// `{resourceType}/{id}` reference the combobox submits and #842's own
/// `document` endpoint resolves right back with [`sql_libraries::
/// dependency_lookup`]; `label` "{name} — {kind}" for the listbox row;
/// `name` (#842's own `data-name`) the bare artifact name alone, for the
/// alias field's autocomplete (`sql-library-panels.js`), which has no use
/// for `label`'s own " — ViewDefinition"/" — SQL View" suffix. `None` for a
/// resource missing `id`, or one whose reference equals `exclude`.
fn table_option(
    resource: &Value,
    resource_type: &str,
    kind_label: &str,
    exclude: Option<&str>,
) -> Option<LookupOption> {
    let id = resource.get("id").and_then(Value::as_str)?;
    let value = format!("{resource_type}/{id}");
    if exclude == Some(value.as_str()) {
        return None;
    }
    let name = resource
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(id)
        .to_string();
    let label = format!("{name} — {kind_label}");
    Some(LookupOption {
        value,
        label,
        name: Some(name),
    })
}

/// `POST /ui/lookup/table-options` — the *Add table* combobox's own result
/// fragment (#842): up to 8 ViewDefinitions and `sql-view` Libraries whose
/// `name` contains `q` — all of them, name-sorted, when `q` is empty (#842's
/// own "browse" default; unlike the Patient/Group pickers above, which stay
/// closed until the visitor types, an artifact list this short is worth
/// showing up front) — excluding `exclude` (the artifact currently open, so
/// *Add table* never offers a self-dependency) and, for Libraries, every
/// `sql-query` one: only a ViewDefinition or a `sql-view` Library can ever
/// be a table. ViewDefinitions are listed first, mirroring both the
/// server's own dependency-resolution order (`crates/rest/.../graph.rs`,
/// ViewDefinition tried before Library) and this file's own
/// [`append_options`] idiom for the Patient/Group pickers above, rather
/// than a name-interleaved merge of the two resource types.
pub(crate) async fn table_options(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    HxRequest(is_htmx): HxRequest,
    axum::extract::Query(query): axum::extract::Query<TableOptionsQuery>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    if !TABLE_TARGETS.contains(&query.target.as_str()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let target = query.target;
    if !is_htmx {
        return Redirect::to(fallback_page(&target)).into_response();
    }
    let i18n = I18n::new(locale);
    let q = form_urlencoded::parse(&body)
        .find(|(key, _)| key == "q")
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default();
    if q.chars().count() > 64 {
        return lookup_error(&i18n, &target, false);
    }
    let exclude = query.exclude.as_deref();

    // A generous buffer over the 8 the response ever shows: `exclude` (and,
    // for Libraries, a `sql-query` skip) can remove a candidate the server
    // already returned, and re-fetching to top the count back up is not
    // worth another request for a combobox's own result list.
    const FETCH_BUFFER: usize = 16;

    let mut vd_params = vec![("_sort".to_string(), "name".to_string())];
    if !q.is_empty() {
        vd_params.push(("name:contains".to_string(), q.clone()));
    }
    let view_definition_kind = i18n.t("lib-tables-kind-view-definition");
    let vd_options: Vec<LookupOption> = match state
        .conformance
        .search_page("ViewDefinition", &vd_params, FETCH_BUFFER, 0, rv.0, &rt.id)
        .await
    {
        Ok(page) => page
            .resources
            .iter()
            .filter_map(|vd| table_option(vd, "ViewDefinition", &view_definition_kind, exclude))
            .collect(),
        // A search failure degrades to "no ViewDefinition matches" for this
        // one result list — there is no banner slot in a result fragment to
        // explain it in, and the sql-view half below can still answer.
        Err(_) => Vec::new(),
    };

    let mut sql_view_candidates: Vec<Value> = state
        .conformance
        .fetch("Library", rv.0, &rt.id)
        .await
        .unwrap_or_default();
    sql_view_candidates.retain(|lib| sql_libraries::has_library_code(lib, "sql-view"));
    if !q.is_empty() {
        let needle = q.to_lowercase();
        sql_view_candidates.retain(|lib| {
            lib.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.to_lowercase().contains(&needle))
        });
    }
    sql_view_candidates.sort_by(|a, b| {
        let name_of = |v: &Value| {
            v.get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        name_of(a).cmp(&name_of(b))
    });
    let sql_view_kind = i18n.t("sql-views-chip");
    let sql_view_options: Vec<LookupOption> = sql_view_candidates
        .iter()
        .filter_map(|lib| table_option(lib, "Library", &sql_view_kind, exclude))
        .take(FETCH_BUFFER)
        .collect();

    let mut options = Vec::new();
    let mut seen = HashSet::new();
    append_options(vd_options, &mut options, &mut seen);
    append_options(sql_view_options, &mut options, &mut seen);

    let message = if options.is_empty() {
        i18n.t("lib-tables-options-empty")
    } else {
        String::new()
    };
    options_response(LookupOptionsFragment {
        target,
        options,
        message,
        error: false,
        id_only: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_lists_accept_bare_and_canonical_ids_and_deduplicate() {
        let values = vec![" p-1,Patient/p-2\np-1 ".to_string()];
        assert_eq!(
            parse_reference_list("Patient", &values),
            Ok(vec!["Patient/p-1".to_string(), "Patient/p-2".to_string()])
        );
        assert!(parse_reference_list("Patient", &["Patient/not/valid".to_string()]).is_err());

        let values = vec!["Group/g-1, g-2".to_string()];
        assert_eq!(
            parse_reference_list("Group", &values),
            Ok(vec!["Group/g-1".to_string(), "Group/g-2".to_string()])
        );
    }

    #[test]
    fn canonical_reference_rejects_ids_that_are_empty_too_long_or_out_of_grammar() {
        assert_eq!(
            canonical_reference("Group", "diabetes-cohort"),
            Some("Group/diabetes-cohort".to_string())
        );
        assert_eq!(
            canonical_reference("Group", "Group/diabetes-cohort"),
            Some("Group/diabetes-cohort".to_string())
        );
        assert_eq!(canonical_reference("Group", ""), None);
        assert_eq!(canonical_reference("Group", &"g".repeat(65)), None);
        assert_eq!(canonical_reference("Group", "g1/g2"), None);
    }

    #[test]
    fn group_option_prefers_the_name_and_falls_back_to_the_bare_id() {
        let named =
            serde_json::json!({"resourceType": "Group", "id": "g1", "name": "Diabetes cohort"});
        let option = group_option(&named).expect("a Group resource yields an option");
        assert_eq!(option.value, "Group/g1");
        assert_eq!(option.label, "Diabetes cohort");

        let unnamed = serde_json::json!({"resourceType": "Group", "id": "g2"});
        let option = group_option(&unnamed).expect("a Group resource yields an option");
        assert_eq!(option.label, "g2");

        assert!(
            group_option(&serde_json::json!({"resourceType": "Patient", "id": "p1"})).is_none()
        );
    }

    #[test]
    fn supports_group_name_search_is_false_before_r5() {
        #[cfg(feature = "R4")]
        assert!(!supports_group_name_search(FhirVersion::R4));
        #[cfg(feature = "R5")]
        assert!(supports_group_name_search(FhirVersion::R5));
    }
}
