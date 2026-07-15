//! PostgreSQL implementation of [`CodeSystemOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use std::sync::Arc;

use crate::error::HtsError;
use crate::traits::{
    CodeSystemOperations, ConceptDesignation, ConceptExpansionFlags, SupplementInfo,
};
use crate::types::{
    DesignationValue, LookupRequest, LookupResponse, PropertyValue, ResourceSearchQuery,
    SubsumesRequest, SubsumesResponse, SubsumptionOutcome, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::value_set::{
    cs_content_for_url, cs_is_case_insensitive, cs_property_local_codes, cs_version_for_msg,
    detect_cs_version_mismatch, is_concept_abstract, is_concept_inactive,
};
use super::{
    PG_LOOKUP_RESPONSE_CACHE_MAX, PG_SUBSUMES_RESPONSE_CACHE_MAX, PostgresTerminologyBackend,
    ResolvedMetaCache,
};

/// Cache wrapper around [`resolve_code_system`]. The free function is
/// preserved for tests and ad-hoc callers; impl blocks on
/// [`PostgresTerminologyBackend`] go through this method so resolved
/// `(system_id, name, version)` triples are memoised across requests.
///
/// Bypassed when `date.is_some()` — a point-in-time filter makes the
/// cacheable result conditional on metadata the cache key doesn't carry.
/// Mirrors backends/sqlite/code_system.rs:1683-1700.
async fn resolve_code_system_cached(
    cache: &ResolvedMetaCache,
    client: &tokio_postgres::Client,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    if date.is_none() {
        let key = (url.to_string(), version.map(str::to_string));
        if let Ok(read) = cache.read() {
            if let Some(v) = read.get(&key) {
                return Ok(v.clone());
            }
        }
        let resolved = resolve_code_system(client, url, version, date).await?;
        if let Ok(mut w) = cache.write() {
            w.insert(key, resolved.clone());
        }
        Ok(resolved)
    } else {
        resolve_code_system(client, url, version, date).await
    }
}

/// Build the `$lookup` response cache key. `None` signals "skip caching"
/// (currently when `useSupplement` is set, since merged responses depend on
/// per-request supplement resolution).
fn lookup_cache_key(req: &LookupRequest) -> Option<String> {
    if !req.use_supplements.is_empty() {
        return None;
    }
    let mut props = req.properties.clone();
    props.sort();
    Some(format!(
        "{}|{}|{}|{}|{}|{}",
        req.system,
        req.code,
        req.version.as_deref().unwrap_or(""),
        req.display_language.as_deref().unwrap_or(""),
        req.date.as_deref().unwrap_or(""),
        props.join(","),
    ))
}

#[async_trait]
impl CodeSystemOperations for PostgresTerminologyBackend {
    async fn lookup(
        &self,
        _ctx: &TenantContext,
        req: LookupRequest,
    ) -> Result<LookupResponse, HtsError> {
        if req.expression.is_some() {
            return Err(HtsError::NotSupported(
                "SNOMED post-coordination expressions are not supported in the PostgreSQL backend"
                    .into(),
            ));
        }

        // Cache check — LK01-04 hot path. The same `(system, code, version,
        // lang)` tuple is replayed across 50 VUs for a 30s run; the warm-hit
        // path skips the connection acquire and 4 DB roundtrips entirely.
        let cache_key = lookup_cache_key(&req);
        if let Some(ref k) = cache_key {
            if let Ok(read) = self.lookup_response_cache.read() {
                if let Some(arc) = read.get(k) {
                    return Ok((**arc).clone());
                }
            }
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let (system_id, cs_name, cs_version) = resolve_code_system_cached(
            &self.cs_resolved_meta_cache,
            &client,
            &req.system,
            req.version.as_deref(),
            req.date.as_deref(),
        )
        .await?;

        let (concept_id, display, definition) =
            find_concept(&client, &system_id, &req.code).await?;

        let stored_props = fetch_properties(&client, concept_id).await?;
        // Per FHIR spec, property="*" is the wildcard meaning "include
        // every property the concept has".
        let want_all = req.properties.is_empty() || req.properties.iter().any(|p| p == "*");
        let synth_props =
            fetch_synthesised_properties(&client, &system_id, &req.code, &stored_props).await?;
        let properties = if want_all {
            let mut out = stored_props;
            out.extend(synth_props);
            out
        } else {
            let mut out: Vec<PropertyValue> = stored_props
                .into_iter()
                .filter(|p| req.properties.contains(&p.code))
                .collect();
            out.extend(
                synth_props
                    .into_iter()
                    .filter(|p| req.properties.contains(&p.code)),
            );
            out
        };

        let mut all_designations = fetch_designations(&client, concept_id).await?;

        // Cross-version designation fallback: when the caller pinned no
        // version and the resolved (newest) version has no designation in the
        // requested language, pull matching designations from the newest
        // *other* stored version of the same canonical URL. This covers e.g.
        // a national SNOMED edition whose translations would otherwise be
        // shadowed by a newer international release ($expand and
        // $validate-code already match designations URL-wide via
        // `concept_designations`).
        if let Some(lang) = req.display_language.as_deref() {
            if req.version.is_none()
                && !all_designations.iter().any(|d| {
                    d.language
                        .as_deref()
                        .is_some_and(|l| crate::language::lang_matches(lang, l))
                })
            {
                all_designations.extend(
                    fetch_designations_cross_version(
                        &client,
                        &req.system,
                        &req.code,
                        &system_id,
                        lang,
                    )
                    .await?,
                );
            }
        }
        let all_designations = all_designations;

        // Matching is BCP-47-aware (RFC 4647 Lookup): an exact tag wins,
        // then a stored dialect of the requested tag (`de` → `de-CH`), then
        // truncations of the requested tag (`de-DE` → `de`).
        let display = if let Some(lang) = req.display_language.as_deref() {
            crate::language::best_lang_match_index(
                lang,
                all_designations.iter().map(|d| d.language.as_deref()),
            )
            .map(|idx| all_designations[idx].value.clone())
            .or(display)
        } else {
            display
        };

        let designations = if let Some(lang) = req.display_language.as_deref() {
            all_designations
                .into_iter()
                .filter(|d| {
                    d.language
                        .as_deref()
                        .is_some_and(|l| crate::language::lang_matches(lang, l))
                })
                .collect()
        } else {
            all_designations
        };

        let response = LookupResponse {
            name: cs_name,
            version: cs_version,
            display,
            definition,
            properties,
            designations,
        };
        // Populate the response cache so the warm path can return it directly.
        if let Some(k) = cache_key {
            if let Ok(mut w) = self.lookup_response_cache.write() {
                if w.len() < PG_LOOKUP_RESPONSE_CACHE_MAX || w.contains_key(&k) {
                    w.insert(k, Arc::new(response.clone()));
                }
            }
        }
        Ok(response)
    }

    async fn validate_code(
        &self,
        _ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError> {
        let system = req.system.clone().ok_or_else(|| {
            HtsError::InvalidRequest(
                "CodeSystem/$validate-code requires 'system' (or 'url') parameter".into(),
            )
        })?;

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Location strings depend on the FHIR input form. Mirrors
        // `postgres/value_set.rs:447-454` and is rewritten by the operations
        // layer for bare-code requests (`Coding.X` → `X`) and CodeableConcept
        // (`Coding.X` → `CodeableConcept.coding[0].X`).
        let (version_loc, system_loc, code_loc, display_loc) = match req.input_form.as_deref() {
            Some("code") => ("version", "system", "code", "display"),
            Some("codeableConcept") => (
                "CodeableConcept.coding[0].version",
                "CodeableConcept.coding[0].system",
                "CodeableConcept.coding[0].code",
                "CodeableConcept.coding[0].display",
            ),
            _ => (
                "Coding.version",
                "Coding.system",
                "Coding.code",
                "Coding.display",
            ),
        };

        // ─── Resolve the CS. NotFound has two flavours:
        //
        // 1. URL not stored at all → UNKNOWN_CODESYSTEM (single issue).
        // 2. URL exists at some version but not the requested one →
        //    delegate to `detect_cs_version_mismatch` for the
        //    UNKNOWN_CODESYSTEM_VERSION shape (+ caused-by canonical).
        //
        // Mirrors `sqlite/code_system.rs:396-419` for path 1; path 2 is the
        // PG-specific enhancement that re-uses the VS-port detector.
        let resolve_result = resolve_code_system_cached(
            &self.cs_resolved_meta_cache,
            &client,
            &system,
            req.version.as_deref(),
            req.date.as_deref(),
        )
        .await;

        // (system_id, version) — both None when the URL exists but the
        // requested version doesn't (the version-mismatch detector below
        // handles that case).
        let (resolved_system_id, resolved_cs_version) = match resolve_result {
            Ok((id, _, version)) => (Some(id), version),
            Err(HtsError::NotFound(_)) => {
                // Probe whether the URL exists at all (any version).
                let url_exists = client
                    .query_one(
                        "SELECT EXISTS(SELECT 1 FROM code_systems WHERE url = $1)",
                        &[&system],
                    )
                    .await
                    .map(|r| r.get::<_, bool>(0))
                    .unwrap_or(false);
                if !url_exists {
                    let text = format!(
                        "A definition for CodeSystem {system} could not be found, so the code cannot be validated"
                    );
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(text.clone()),
                        display: None,
                        system: None,
                        cs_version: None,
                        inactive: None,
                        issues: vec![crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "not-found".into(),
                            tx_code: "not-found".into(),
                            text,
                            expression: Some(system_loc.into()),
                            location: None,
                            message_id: Some("UNKNOWN_CODESYSTEM".into()),
                        }],
                        caused_by_unknown_system: None,
                        concept_status: None,
                        normalized_code: None,
                    });
                }
                // URL exists at some version; fall through to the version-mismatch
                // detector. The detector will produce the proper issues.
                (None, None)
            }
            Err(e) => return Err(e),
        };

        // ─── CS-version-mismatch detection: when the caller pinned a version
        //     that doesn't exist in the DB (or that the CS doesn't actually
        //     define at the requested version), produce the
        //     UNKNOWN_CODESYSTEM_VERSION shape from the version-detector. CS
        //     `$validate-code` has no VS compose context — `compose_json` and
        //     `vs_version` are both `None`.
        if let Some(req_ver) = req
            .version
            .as_deref()
            .filter(|v| !v.is_empty() && !v.contains(".x") && *v != "x")
        {
            if let Some((issues, caused_by, echo_version)) = detect_cs_version_mismatch(
                &client,
                &system,
                req_ver,
                None,
                None,
                version_loc,
                system_loc,
            )
            .await
            {
                // Echo the code's display from any stored version of the CS,
                // so consumers can see the concept exists (only the version is
                // wrong). Matches `postgres/value_set.rs:506-517`.
                let display = client
                    .query_opt(
                        "SELECT c.display FROM concepts c
                         JOIN code_systems s ON s.id = c.system_id
                         WHERE s.url = $1 AND c.code = $2
                         ORDER BY COALESCE(s.version, '') DESC LIMIT 1",
                        &[&system, &req.code],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|r| r.get::<_, Option<String>>(0));
                let mut texts: Vec<&str> = issues
                    .iter()
                    .filter(|i| i.severity == "error")
                    .map(|i| i.text.as_str())
                    .collect();
                texts.sort_unstable();
                let message = texts.join("; ");
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(message),
                    display,
                    system: Some(system.clone()),
                    cs_version: echo_version,
                    inactive: None,
                    issues,
                    caused_by_unknown_system: caused_by,
                    concept_status: None,
                    normalized_code: None,
                });
            }
        }

        // ─── Find the concept. ────────────────────────────────────────────────
        //
        // Try the literal code first. When the CS is case-insensitive and
        // there's no literal hit, fall back to a case-insensitive scan and
        // record the canonical (correct-case) `normalized_code` for the IG
        // `case/case-coding-insensitive-*` fixtures.
        //
        // Scope to the resolved CS row's `system_id` when available so a
        // request pinned to version 1.0.0 doesn't accidentally pick up a
        // concept that only exists in version 2.0.0 of the same URL.
        //
        // TODO: parity — wildcard versions ("1.x") whose pattern doesn't
        // match any stored version fall through here unhandled. The exact-
        // version detector above filters them out. SQLite has the same gap.
        let mut normalized_code: Option<String> = None;
        let concept_lookup = if let Some(sid) = resolved_system_id.as_deref() {
            match find_concept_by_system_id(&client, sid, &req.code).await {
                Some(c) => Some(c),
                None => {
                    if cs_is_case_insensitive(&client, &system).await {
                        if let Some(c) = find_concept_by_system_id_ci(&client, sid, &req.code).await
                        {
                            if c.code != req.code {
                                normalized_code = Some(c.code.clone());
                            }
                            Some(c)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        } else {
            // CS URL exists but the requested version doesn't — search across
            // all stored versions so the unknown-code branch can still produce
            // accurate "code does/doesn't exist in this CS" messaging.
            match find_concept_by_url(&client, &system, &req.code).await {
                Some(c) => Some(c),
                None => {
                    if cs_is_case_insensitive(&client, &system).await {
                        if let Some(c) = find_concept_by_url_ci(&client, &system, &req.code).await {
                            if c.code != req.code {
                                normalized_code = Some(c.code.clone());
                            }
                            Some(c)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        let concept = match concept_lookup {
            Some(c) => c,
            None => {
                // Match the IG `validation/cs-code-bad-code` text format exactly.
                let cs_version_str = cs_version_for_msg(&client, &system).await;
                let cs_content = cs_content_for_url(&client, &system).await;

                // Fragment CodeSystems: unknown code is a *warning*, not an error.
                // Mirrors `sqlite/code_system.rs:454-485`.
                if cs_content.as_deref() == Some("fragment") {
                    let text = match cs_version_str.as_deref() {
                        Some(v) => format!(
                            "Unknown Code '{}' in the CodeSystem '{}' version '{}' - note that the code system is labeled as a fragment, so the code may be valid in some other fragment",
                            req.code, system, v
                        ),
                        None => format!(
                            "Unknown Code '{}' in the CodeSystem '{}' - note that the code system is labeled as a fragment, so the code may be valid in some other fragment",
                            req.code, system
                        ),
                    };
                    return Ok(ValidateCodeResponse {
                        result: true,
                        message: None,
                        display: None,
                        system: Some(system.clone()),
                        cs_version: cs_version_str,
                        inactive: None,
                        issues: vec![crate::types::ValidationIssue {
                            severity: "warning".into(),
                            fhir_code: "code-invalid".into(),
                            tx_code: "invalid-code".into(),
                            text,
                            expression: Some(code_loc.into()),
                            location: Some(code_loc.into()),
                            message_id: Some("UNKNOWN_CODE_IN_FRAGMENT".into()),
                        }],
                        caused_by_unknown_system: None,
                        concept_status: None,
                        normalized_code: None,
                    });
                }

                if crate::ucum_validate::is_ucum_url(&system)
                    && crate::ucum_validate::is_valid_ucum_code(&req.code)
                {
                    return Ok(crate::ucum_validate::composed_code_success(
                        &req.code,
                        cs_version_str,
                    ));
                }

                let text = match cs_version_str.as_deref() {
                    Some(v) => format!(
                        "Unknown code '{}' in the CodeSystem '{}' version '{}'",
                        req.code, system, v
                    ),
                    None => format!("Unknown code '{}' in the CodeSystem '{}'", req.code, system),
                };
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(text.clone()),
                    display: None,
                    system: Some(system.clone()),
                    cs_version: cs_version_str,
                    inactive: None,
                    issues: vec![crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "invalid-code".into(),
                        text,
                        expression: Some(code_loc.into()),
                        location: None,
                        message_id: Some("Unknown_Code_in_Version".into()),
                    }],
                    caused_by_unknown_system: None,
                    concept_status: None,
                    normalized_code: None,
                });
            }
        };

        // ─── Concept found. Compute flag attributes. ─────────────────────────
        let canonical_code = concept.code.clone();
        let display = concept.display.clone();
        let is_inactive = is_concept_inactive(&client, &system, &canonical_code).await;
        let is_abstract = is_concept_abstract(&client, &system, &canonical_code).await;

        let mut issues: Vec<crate::types::ValidationIssue> = Vec::new();
        let qualified = format!("{system}#{canonical_code}");

        // Abstract concept with `abstract=false` request: reject with the IG
        // "Code 'X' is abstract, and not allowed in this context" message.
        // TODO: parity — SQLite CS validate_code doesn't currently emit this
        // (only the VS path does); included here for IG conformance.
        if is_abstract && req.include_abstract == Some(false) {
            let abstract_text =
                format!("Code '{qualified}' is abstract, and not allowed in this context");
            issues.push(crate::types::ValidationIssue {
                severity: "error".into(),
                fhir_code: "business-rule".into(),
                tx_code: "code-rule".into(),
                text: abstract_text.clone(),
                expression: Some(code_loc.into()),
                location: None,
                message_id: Some("ABSTRACT_CODE_NOT_ALLOWED".into()),
            });
        }

        // Case-insensitive normalisation note. The IG `case/case-coding-
        // insensitive-*` fixtures expect a CODE_CASE_DIFFERENCE informational
        // issue identifying the canonical code.
        // TODO: parity — SQLite CS validate_code doesn't currently emit this.
        if let Some(canonical) = normalized_code.as_deref() {
            let cs_qualifier = match cs_version_for_msg(&client, &system).await {
                Some(v) => format!("{system}|{v}"),
                None => system.clone(),
            };
            let text = format!(
                "The code '{}' differs from the correct code '{canonical}' by case. Although the code system '{cs_qualifier}' is case insensitive, implementers are strongly encouraged to use the correct case anyway",
                req.code
            );
            issues.push(crate::types::ValidationIssue {
                severity: "information".into(),
                fhir_code: "business-rule".into(),
                tx_code: "code-rule".into(),
                text,
                expression: Some(code_loc.into()),
                location: Some(code_loc.into()),
                message_id: Some("CODE_CASE_DIFFERENCE".into()),
            });
        }

        // Inactive concept: emit the canonical INACTIVE_CONCEPT_FOUND warning.
        // The operations layer also appends a specific-status companion (e.g.
        // "...status of retired...") via `lookup_concept_status`.
        //
        // No `location` field — the IG `validation/validate-contained-good`
        // fixture (inline-VS path) pins this warning WITHOUT a location.
        // The operations layer then clones the template for the
        // specific-status companion, so the location-less template flows
        // through both issues. URL-based VS-validate-code paths (e.g.
        // `inactive-2a-validate`) emit their own INACTIVE_CONCEPT_FOUND
        // WITH location via finish_validate_code_response, separately.
        // Mirrors SQLite's CS-side behaviour (which doesn't emit at all).
        if is_inactive {
            issues.push(crate::types::ValidationIssue {
                severity: "warning".into(),
                fhir_code: "business-rule".into(),
                tx_code: "code-comment".into(),
                text: format!(
                    "The concept '{}' has a status of inactive and its use should be reviewed",
                    canonical_code
                ),
                expression: Some("Coding".into()),
                location: None,
                message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
            });
        }

        // Display mismatch. The IG `validation/simple-*-bad-display` fixtures
        // expect the "Wrong Display Name 'X' for Y. Valid display is 'Z'..."
        // wording. With `lenient-display-validation=true`, the issue is a
        // warning and result stays true; otherwise it's an error.
        let mut display_message: Option<String> = None;
        if let Some(expected) = req.display.as_deref() {
            if let Some(actual) = display.as_deref() {
                if actual != expected {
                    let text = format!(
                        "Wrong Display Name '{expected}' for {qualified}. Valid display is '{actual}' (en) (for the language(s) '--')"
                    );
                    display_message = Some(text.clone());
                    let lenient = req.lenient_display_validation == Some(true);
                    issues.push(crate::types::ValidationIssue {
                        severity: if lenient { "warning" } else { "error" }.into(),
                        fhir_code: "invalid".into(),
                        tx_code: "invalid-display".into(),
                        text,
                        expression: Some(display_loc.into()),
                        location: None,
                        message_id: Some("Display_Name_for__should_be_one_of__instead_of".into()),
                    });
                }
            }
        }

        let has_error = issues.iter().any(|i| i.severity == "error");
        let message = if !issues.is_empty() {
            let mut sorted: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
            sorted.sort();
            Some(sorted.join("; "))
        } else {
            display_message
        };

        Ok(ValidateCodeResponse {
            result: !has_error,
            message,
            display,
            system: Some(system.clone()),
            cs_version: resolved_cs_version.or(cs_version_for_msg(&client, &system).await),
            inactive: if is_inactive { Some(true) } else { None },
            issues,
            caused_by_unknown_system: None,
            concept_status: None,
            normalized_code,
        })
    }

    async fn subsumes(
        &self,
        _ctx: &TenantContext,
        req: SubsumesRequest,
    ) -> Result<SubsumesResponse, HtsError> {
        // SS01 hot-path memo. Both ancestor-check directions are folded into
        // the cached outcome; the key includes version so different versions
        // of the same system don't collide.
        let cache_key = format!(
            "{}|{}|{}|{}",
            req.system,
            req.version.as_deref().unwrap_or(""),
            req.code_a,
            req.code_b,
        );
        if let Ok(read) = self.subsumes_response_cache.read() {
            if let Some(arc) = read.get(&cache_key) {
                return Ok((**arc).clone());
            }
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let (system_id, _, _) = resolve_code_system_cached(
            &self.cs_resolved_meta_cache,
            &client,
            &req.system,
            req.version.as_deref(),
            None,
        )
        .await?;

        find_concept(&client, &system_id, &req.code_a).await?;
        find_concept(&client, &system_id, &req.code_b).await?;

        let outcome = if req.code_a == req.code_b {
            SubsumptionOutcome::Equivalent
        } else if check_ancestor(&client, &system_id, &req.code_a, &req.code_b).await? {
            SubsumptionOutcome::Subsumes
        } else if check_ancestor(&client, &system_id, &req.code_b, &req.code_a).await? {
            SubsumptionOutcome::SubsumedBy
        } else {
            SubsumptionOutcome::NotSubsumed
        };

        let response = SubsumesResponse { outcome };
        if let Ok(mut w) = self.subsumes_response_cache.write() {
            if w.len() < PG_SUBSUMES_RESPONSE_CACHE_MAX || w.contains_key(&cache_key) {
                w.insert(cache_key, Arc::new(response.clone()));
            }
        }
        Ok(response)
    }

    async fn code_system_version_for_url(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<Option<String>, HtsError> {
        // Highest-stored version wins. Without ORDER BY, PostgreSQL returned
        // an arbitrary row (typically insertion order — for multi-version
        // CSes like overload, that's 1.0.0 instead of 2.0.0). The IG
        // `overload/validate-bad-unknown` fixture pins the latest version
        // in the response — sort DESC and LIMIT 1. Mirrors
        // sqlite/code_system.rs:651-655.
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_opt(
                "SELECT version FROM code_systems
                  WHERE url = $1
                  ORDER BY COALESCE(version, '') DESC
                  LIMIT 1",
                &[&url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
    }

    /// Existence-only check that skips reading the row's `resource_json`
    /// blob — the trait default falls back to `search(url=…, count=1)`
    /// which pulls multi-MB CodeSystem bodies just to drop them. Mirrors
    /// the SQLite override at `sqlite/code_system.rs:679`; the SQLite
    /// version also memoises across calls via `cs_exists_cache()` and
    /// the PG impl will gain the same cache once the PG backend grows a
    /// per-instance cache map (tracked under the Phase 2 work).
    async fn code_system_exists(&self, _ctx: &TenantContext, url: &str) -> Result<bool, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM code_systems WHERE url = $1)",
                &[&url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(row.get::<_, bool>(0))
    }

    async fn code_system_language(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<Option<String>, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_opt(
                "SELECT resource_json->>'language' FROM code_systems WHERE url = $1 LIMIT 1",
                &[&url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
    }

    async fn code_system_is_hierarchical(
        &self,
        _ctx: &TenantContext,
        url: &str,
        version: Option<&str>,
    ) -> Result<bool, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        // Mirror the SQLite implementation: hierarchyMeaning='is-a' AND at least
        // one materialised parent/child edge. `$2 IS NULL` lets an unpinned
        // request match any version of the URL.
        let row = client
            .query_opt(
                "SELECT 1 FROM code_systems s \
                 JOIN concept_hierarchy h ON h.system_id = s.id \
                 WHERE s.url = $1 \
                   AND ($2::text IS NULL OR s.version = $2) \
                   AND s.resource_json->>'hierarchyMeaning' = 'is-a' \
                 LIMIT 1",
                &[&url, &version],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(row.is_some())
    }

    async fn concept_designations(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError> {
        if codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let rows = client
            .query(
                "SELECT c.code, cd.language, cd.use_system, cd.use_code, cd.value
                 FROM concept_designations cd
                 JOIN concepts c ON c.id = cd.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)",
                &[&system_url, &codes],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let mut out: std::collections::HashMap<String, Vec<ConceptDesignation>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            out.entry(code).or_default().push(ConceptDesignation {
                language: row.get(1),
                use_system: row.get(2),
                use_code: row.get(3),
                value: row.get(4),
                source: None,
            });
        }
        Ok(out)
    }

    async fn concept_property_values(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
        properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError> {
        if codes.is_empty() || properties.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let rows = client
            .query(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)
                   AND cp.property = ANY($3)",
                &[&system_url, &codes, &properties],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let mut out: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let prop: String = row.get(1);
            let value: String = row.get(2);
            out.entry(code).or_default().push((prop, value));
        }

        // FHIR `definition` is stored as a column on `concepts` rather than
        // in concept_properties (it ships in a dedicated CodeSystem field).
        // When the caller asks for `property=definition`, surface the column
        // value so $expand emits it as a synthesised property — matches the
        // IG `parameters/parameters-expand-*-definitions*` fixtures. Mirrors
        // sqlite/code_system.rs:899-942.
        if properties.iter().any(|p| p == "definition") {
            let def_rows = client
                .query(
                    "SELECT c.code, c.definition
                       FROM concepts c
                       JOIN code_systems s ON s.id = c.system_id
                      WHERE s.url = $1
                        AND c.code = ANY($2)
                        AND c.definition IS NOT NULL",
                    &[&system_url, &codes],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            for row in def_rows {
                let code: String = row.get(0);
                let definition: String = row.get(1);
                let entry = out.entry(code).or_default();
                if !entry.iter().any(|(p, _)| p == "definition") {
                    entry.push(("definition".to_string(), definition));
                }
            }
        }

        Ok(out)
    }

    async fn concept_expansion_flags(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, ConceptExpansionFlags>, HtsError> {
        if codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Per FHIR concept-properties IG, the standard `notSelectable` and
        // `inactive` properties' local CodeSystem.property.code can be ANY
        // local name (e.g. `not-selectable` with a hyphen in the
        // tx-ecosystem `notSelectable/` fixtures). Resolve via property[].uri
        // → local-code mapping when available; always fall back to the
        // canonical name so a CS that never declares property[] still
        // reports correctly. Mirrors sqlite/code_system.rs:980-1012.
        let abstract_codes = cs_property_local_codes(&client, system_url, "notSelectable").await;
        let inactive_codes = cs_property_local_codes(&client, system_url, "inactive").await;

        let rows = client
            .query(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)
                   AND (
                       (cp.property = ANY($3) AND cp.value = 'true')
                    OR (cp.property = ANY($4) AND cp.value = 'true')
                    OR (cp.property = 'status'
                        AND cp.value IN ('retired', 'inactive'))
                   )",
                &[&system_url, &codes, &abstract_codes, &inactive_codes],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: std::collections::HashMap<String, ConceptExpansionFlags> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let property: String = row.get(1);
            // `deprecated` is intentionally excluded: per the FHIR
            // concept-properties IG, deprecated codes are discouraged but
            // still active (act-class expansion fixtures rely on this —
            // deprecated codes survive `activeOnly=true` filtering).
            let flags = out.entry(code).or_default();
            if property == "status" || inactive_codes.iter().any(|c| c == &property) {
                flags.inactive = true;
            } else if abstract_codes.iter().any(|c| c == &property) {
                flags.is_abstract = true;
            }
        }
        Ok(out)
    }

    async fn search(
        &self,
        _ctx: &TenantContext,
        query: ResourceSearchQuery,
    ) -> Result<Vec<serde_json::Value>, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let limit = i64::from(query.count.unwrap_or(20));
        let offset = i64::from(query.offset.unwrap_or(0));

        let rows = client
            .query(
                "SELECT id, url, version, name, title, status, resource_json
                 FROM code_systems
                 WHERE ($1::text IS NULL OR url = $1)
                   AND ($2::text IS NULL OR version = $2)
                   AND ($3::text IS NULL OR name = $3)
                   AND ($4::text IS NULL OR title = $4)
                   AND ($5::text IS NULL OR status = $5)
                 ORDER BY created_at
                 LIMIT $6 OFFSET $7",
                &[
                    &query.url,
                    &query.version,
                    &query.name,
                    &query.title,
                    &query.status,
                    &limit,
                    &offset,
                ],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let url: String = row.get(1);
            let version: Option<String> = row.get(2);
            let name: Option<String> = row.get(3);
            let title: Option<String> = row.get(4);
            let status: String = row.get(5);
            let resource_json: Option<serde_json::Value> = row.get(6);

            let mut resource = resource_json.unwrap_or_else(|| {
                build_synthetic_resource(
                    "CodeSystem",
                    &id,
                    &url,
                    version.as_deref(),
                    name.as_deref(),
                    title.as_deref(),
                    &status,
                )
            });
            // Ensure the resource id matches the table's authoritative id column
            // (may differ from resource_json after a URL-conflict upsert).
            if let Some(obj) = resource.as_object_mut() {
                obj.insert("id".to_string(), serde_json::Value::String(id));
            }
            results.push(resource);
        }
        Ok(results)
    }

    async fn concept_resource_entries(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, HtsError> {
        if codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Read the base CodeSystem's resource_json (highest version), then
        // walk concept[] picking entries whose code is in the requested set.
        // Mirrors sqlite/code_system.rs:1436-1475.
        let row = client
            .query_opt(
                "SELECT resource_json FROM code_systems
                 WHERE url = $1 AND content != 'supplement'
                 ORDER BY COALESCE(version, '') DESC LIMIT 1",
                &[&system_url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        if let Some(r) = row
            && let Some(v) = r.get::<_, Option<serde_json::Value>>(0)
        {
            walk_concepts(&v, codes, &mut out);
        }
        Ok(out)
    }

    async fn supplement_concept_entries(
        &self,
        _ctx: &TenantContext,
        supplement_urls: &[String],
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<serde_json::Value>>, HtsError> {
        if supplement_urls.is_empty() || codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Mirrors sqlite/code_system.rs:1477-1532.
        let rows = client
            .query(
                "SELECT resource_json FROM code_systems
                 WHERE url = ANY($1) AND content = 'supplement'",
                &[&supplement_urls],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for row in rows {
            let Some(v) = row.get::<_, Option<serde_json::Value>>(0) else {
                continue;
            };
            let mut local: std::collections::HashMap<String, serde_json::Value> =
                std::collections::HashMap::new();
            walk_concepts(&v, codes, &mut local);
            for (code, entry) in local {
                out.entry(code).or_default().push(entry);
            }
        }
        Ok(out)
    }

    async fn supplement_target(
        &self,
        _ctx: &TenantContext,
        supplement_url: &str,
    ) -> Result<Option<SupplementInfo>, HtsError> {
        // Supplements live in the same code_systems table as any other CS;
        // distinguishing field is `content='supplement'` and a `supplements`
        // pointer in resource_json. Mirrors sqlite/code_system.rs:1208-1259.
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let row = client
            .query_opt(
                "SELECT content, version, resource_json->>'supplements'
                 FROM code_systems
                 WHERE url = $1
                 LIMIT 1",
                &[&supplement_url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let Some(r) = row else { return Ok(None) };
        let content: String = r.get(0);
        if content != "supplement" {
            return Ok(None);
        }
        let version: Option<String> = r.get(1);
        let target: Option<String> = r.get(2);
        let Some(target_url) = target else {
            return Ok(None);
        };
        let supplement_canonical = match version {
            Some(v) => format!("{supplement_url}|{v}"),
            None => supplement_url.to_owned(),
        };
        Ok(Some(SupplementInfo {
            target_url,
            supplement_canonical,
        }))
    }

    async fn supplement_designations(
        &self,
        _ctx: &TenantContext,
        supplement_urls: &[String],
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError> {
        if supplement_urls.is_empty() || codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Mirrors sqlite/code_system.rs:1261-1345.
        let rows = client
            .query(
                "SELECT c.code, cd.language, cd.use_system, cd.use_code, cd.value,
                        s.url, s.version
                   FROM concept_designations cd
                   JOIN concepts c ON c.id = cd.concept_id
                   JOIN code_systems s ON s.id = c.system_id
                  WHERE s.url = ANY($1)
                    AND s.content = 'supplement'
                    AND c.code = ANY($2)",
                &[&supplement_urls, &codes],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: std::collections::HashMap<String, Vec<ConceptDesignation>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let language: Option<String> = row.get(1);
            let use_system: Option<String> = row.get(2);
            let use_code: Option<String> = row.get(3);
            let value: String = row.get(4);
            let supp_url: String = row.get(5);
            let supp_ver: Option<String> = row.get(6);
            let source = match supp_ver {
                Some(v) => format!("{supp_url}|{v}"),
                None => supp_url,
            };
            out.entry(code).or_default().push(ConceptDesignation {
                language,
                use_system,
                use_code,
                value,
                source: Some(source),
            });
        }
        Ok(out)
    }

    async fn supplement_property_values(
        &self,
        _ctx: &TenantContext,
        supplement_urls: &[String],
        codes: &[String],
        properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError> {
        if supplement_urls.is_empty() || codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Empty `properties` slice = "every property" (lookup wildcard mode).
        // Mirrors sqlite/code_system.rs:1347-1434.
        let want_all_props = properties.is_empty();
        let rows = if want_all_props {
            client
                .query(
                    "SELECT c.code, cp.property, cp.value
                       FROM concept_properties cp
                       JOIN concepts c ON c.id = cp.concept_id
                       JOIN code_systems s ON s.id = c.system_id
                      WHERE s.url = ANY($1)
                        AND s.content = 'supplement'
                        AND c.code = ANY($2)",
                    &[&supplement_urls, &codes],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?
        } else {
            client
                .query(
                    "SELECT c.code, cp.property, cp.value
                       FROM concept_properties cp
                       JOIN concepts c ON c.id = cp.concept_id
                       JOIN code_systems s ON s.id = c.system_id
                      WHERE s.url = ANY($1)
                        AND s.content = 'supplement'
                        AND c.code = ANY($2)
                        AND cp.property = ANY($3)",
                    &[&supplement_urls, &codes, &properties],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?
        };

        let mut out: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let prop: String = row.get(1);
            let value: String = row.get(2);
            out.entry(code).or_default().push((prop, value));
        }
        Ok(out)
    }
}

/// Recursively walk `concept[]` arrays in a CodeSystem JSON value, accumulating
/// each concept whose `code` matches one of `codes` into `out`. Used to pull
/// the original concept JSON (with extensions, designations, properties) from
/// `resource_json` when that data isn't broken out into the SQL schema.
///
/// Mirrors `sqlite/code_system.rs:walk_concepts`.
fn walk_concepts(
    resource: &serde_json::Value,
    codes: &[String],
    out: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    let Some(concepts) = resource.get("concept").and_then(|c| c.as_array()) else {
        return;
    };
    for c in concepts {
        if let Some(code) = c.get("code").and_then(|v| v.as_str())
            && codes.iter().any(|x| x == code)
            && !out.contains_key(code)
        {
            out.insert(code.to_string(), c.clone());
        }
        walk_concepts(c, codes, out);
    }
}

// ── Private DB helpers ─────────────────────────────────────────────────────────

/// Resolve a code system by URL, optional version, and optional date.
///
/// Returns `(id, name_or_url, version)`.
///
/// Mirrors the SQLite implementation: an unspecified version defaults to the
/// most recent (textual COALESCE-DESC), an explicit version with `.x` segments
/// (or a bare numeric prefix like `"1"`) matches the highest version that
/// shares the literal segments, and an exact version requires an exact match.
async fn resolve_code_system(
    client: &tokio_postgres::Client,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, COALESCE(name, url), version
             FROM code_systems
             WHERE url = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)
             ORDER BY (CASE COALESCE(content, 'complete')
                            WHEN 'complete'   THEN 0
                            WHEN 'supplement' THEN 0
                            WHEN 'fragment'   THEN 1
                            WHEN 'example'    THEN 1
                            WHEN 'not-present' THEN 2
                            ELSE 1 END),
                      (CASE WHEN EXISTS (
                          SELECT 1 FROM concepts c WHERE c.system_id = code_systems.id
                      ) THEN 0 ELSE 1 END),
                      COALESCE(version, '') DESC,
                      id",
            &[&url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if rows.is_empty() {
        return Err(HtsError::NotFound(format!("CodeSystem not found: {url}")));
    }
    let candidates: Vec<(String, String, Option<String>)> = rows
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();

    match version {
        Some(ver) if ver.contains(".x") || ver == "x" || is_short_version(ver) => {
            select_best_version_match(&candidates, ver).ok_or_else(|| {
                HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
            })
        }
        Some(ver) if crate::backends::code_system_version_is_current(ver) => {
            Ok(candidates.into_iter().next().expect("non-empty checked"))
        }
        Some(ver) => candidates
            .into_iter()
            .find(|(_, _, v)| v.as_deref() == Some(ver))
            .ok_or_else(|| {
                HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
            }),
        None => Ok(candidates.into_iter().next().expect("non-empty checked")),
    }
}

fn is_short_version(ver: &str) -> bool {
    !ver.contains('.') && ver.chars().all(|c| c.is_ascii_digit())
}

fn select_best_version_match(
    candidates: &[(String, String, Option<String>)],
    pattern: &str,
) -> Option<(String, String, Option<String>)> {
    let pattern_segments: Vec<&str> = pattern.split('.').collect();
    candidates
        .iter()
        .filter(|(_, _, v)| match v {
            Some(actual) => version_matches(actual, &pattern_segments),
            None => false,
        })
        .max_by(|a, b| a.2.cmp(&b.2))
        .cloned()
}

fn version_matches(actual: &str, pattern_segments: &[&str]) -> bool {
    let actual_segments: Vec<&str> = actual.split('.').collect();
    if pattern_segments.len() > actual_segments.len() {
        return false;
    }
    pattern_segments
        .iter()
        .zip(actual_segments.iter())
        .all(|(p, a)| *p == "x" || *p == *a)
}

/// A concept row resolved for `$validate-code` purposes — carries the literal
/// stored code so case-insensitive matches can echo the canonical form back to
/// the caller via `normalized_code`.
struct ValidateConcept {
    code: String,
    display: Option<String>,
}

/// Look up a concept scoped to a specific CS row (`system_id`) by literal
/// code. Use this when the caller has pinned a CS version and we need to
/// confirm the code exists in *that* row, not just somewhere under the URL.
async fn find_concept_by_system_id(
    client: &tokio_postgres::Client,
    system_id: &str,
    code: &str,
) -> Option<ValidateConcept> {
    let row = client
        .query_opt(
            "SELECT code, display FROM concepts
             WHERE system_id = $1 AND code = $2 LIMIT 1",
            &[&system_id, &code],
        )
        .await
        .ok()
        .flatten()?;
    Some(ValidateConcept {
        code: row.get(0),
        display: row.get(1),
    })
}

/// Case-insensitive variant of [`find_concept_by_system_id`]. Only called when
/// the CodeSystem has `caseSensitive: false`.
async fn find_concept_by_system_id_ci(
    client: &tokio_postgres::Client,
    system_id: &str,
    code: &str,
) -> Option<ValidateConcept> {
    let row = client
        .query_opt(
            "SELECT code, display FROM concepts
             WHERE system_id = $1 AND LOWER(code) = LOWER($2) LIMIT 1",
            &[&system_id, &code],
        )
        .await
        .ok()
        .flatten()?;
    Some(ValidateConcept {
        code: row.get(0),
        display: row.get(1),
    })
}

/// Look up a concept by CodeSystem URL + literal code. Walks all CS rows that
/// share `system_url` (handles URLs with multiple stored versions), preferring
/// the row whose `version` sorts highest.
async fn find_concept_by_url(
    client: &tokio_postgres::Client,
    system_url: &str,
    code: &str,
) -> Option<ValidateConcept> {
    let row = client
        .query_opt(
            "SELECT c.code, c.display FROM concepts c
             JOIN code_systems s ON s.id = c.system_id
             WHERE s.url = $1 AND c.code = $2
             ORDER BY COALESCE(s.version, '') DESC LIMIT 1",
            &[&system_url, &code],
        )
        .await
        .ok()
        .flatten()?;
    Some(ValidateConcept {
        code: row.get(0),
        display: row.get(1),
    })
}

/// Case-insensitive variant of [`find_concept_by_url`]. Only called when the
/// CodeSystem has `caseSensitive: false` — returns the canonical (stored)
/// code so the caller can populate `normalized_code` when it differs from
/// the request.
async fn find_concept_by_url_ci(
    client: &tokio_postgres::Client,
    system_url: &str,
    code: &str,
) -> Option<ValidateConcept> {
    let row = client
        .query_opt(
            "SELECT c.code, c.display FROM concepts c
             JOIN code_systems s ON s.id = c.system_id
             WHERE s.url = $1 AND LOWER(c.code) = LOWER($2)
             ORDER BY COALESCE(s.version, '') DESC LIMIT 1",
            &[&system_url, &code],
        )
        .await
        .ok()
        .flatten()?;
    Some(ValidateConcept {
        code: row.get(0),
        display: row.get(1),
    })
}

/// Look up a concept row by `(system_id, code)`.
///
/// Returns `(concept_id, display, definition)`.
async fn find_concept(
    client: &tokio_postgres::Client,
    system_id: &str,
    code: &str,
) -> Result<(i64, Option<String>, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, display, definition FROM concepts
             WHERE system_id = $1 AND code = $2",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| HtsError::NotFound(format!("Concept not found: {code}")))?;

    Ok((row.get(0), row.get(1), row.get(2)))
}

/// Fetch all properties for a concept.
async fn fetch_properties(
    client: &tokio_postgres::Client,
    concept_id: i64,
) -> Result<Vec<PropertyValue>, HtsError> {
    let rows = client
        .query(
            "SELECT property, value_type, value
             FROM concept_properties WHERE concept_id = $1 ORDER BY property",
            &[&concept_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| PropertyValue {
            code: row.get(0),
            value_type: row.get(1),
            value: row.get(2),
            description: None,
        })
        .collect())
}

/// Synthesise hierarchy- and status-derived properties for `$lookup`.
///
/// Mirrors the SQLite backend implementation — see
/// [`super::super::sqlite::code_system::fetch_synthesised_properties`] for
/// rationale.
async fn fetch_synthesised_properties(
    client: &tokio_postgres::Client,
    system_id: &str,
    code: &str,
    stored: &[PropertyValue],
) -> Result<Vec<PropertyValue>, HtsError> {
    let mut out = Vec::new();

    // Parents — synthesised from concept_hierarchy. Skip when the concept
    // already carries explicit `parent` properties (the bundle importer
    // mirrors `parent` properties into concept_hierarchy, so synthesising
    // here would duplicate every stored parent edge).
    let stored_parent_codes: std::collections::HashSet<&str> = stored
        .iter()
        .filter(|p| p.code == "parent")
        .map(|p| p.value.as_str())
        .collect();
    let parent_rows = client
        .query(
            "SELECT h.parent_code, c.display
             FROM concept_hierarchy h
             LEFT JOIN concepts c
                    ON c.system_id = h.system_id AND c.code = h.parent_code
             WHERE h.system_id = $1 AND h.child_code = $2
             ORDER BY h.parent_code",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    for row in parent_rows {
        let parent_code: String = row.get(0);
        if stored_parent_codes.contains(parent_code.as_str()) {
            continue;
        }
        out.push(PropertyValue {
            code: "parent".into(),
            value_type: "code".into(),
            value: parent_code,
            description: row.get(1),
        });
    }

    // Children.
    let child_rows = client
        .query(
            "SELECT h.child_code, c.display
             FROM concept_hierarchy h
             LEFT JOIN concepts c
                    ON c.system_id = h.system_id AND c.code = h.child_code
             WHERE h.system_id = $1 AND h.parent_code = $2
             ORDER BY h.child_code",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    for row in child_rows {
        out.push(PropertyValue {
            code: "child".into(),
            value_type: "code".into(),
            value: row.get(0),
            description: row.get(1),
        });
    }

    // Inactive flag (only when not already stored explicitly).
    if !stored.iter().any(|p| p.code == "inactive") {
        let row = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM concept_properties cp
                     JOIN concepts c ON c.id = cp.concept_id
                     WHERE c.system_id = $1
                       AND c.code = $2
                       AND cp.property = 'status'
                       AND cp.value IN ('retired', 'deprecated', 'withdrawn', 'inactive')
                 )",
                &[&system_id, &code],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let inactive: bool = row.get(0);
        out.push(PropertyValue {
            code: "inactive".into(),
            value_type: "boolean".into(),
            value: inactive.to_string(),
            description: None,
        });
    }

    Ok(out)
}

/// Fetch all designations for a concept.
async fn fetch_designations(
    client: &tokio_postgres::Client,
    concept_id: i64,
) -> Result<Vec<DesignationValue>, HtsError> {
    let rows = client
        .query(
            "SELECT language, use_system, use_code, value
             FROM concept_designations WHERE concept_id = $1",
            &[&concept_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| DesignationValue {
            language: row.get(0),
            use_system: row.get(1),
            use_code: row.get(2),
            value: row.get(3),
            source: None,
        })
        .collect())
}

/// Designations for `code` matching `lang` (BCP-47-aware, see
/// [`crate::language::lang_matches`]) from stored versions of the canonical
/// `url` *other than* `exclude_system_id`, taken from the newest such version
/// that has any (insertion order within that version is preserved so
/// preferred terms stay first).
async fn fetch_designations_cross_version(
    client: &tokio_postgres::Client,
    url: &str,
    code: &str,
    exclude_system_id: &str,
    lang: &str,
) -> Result<Vec<DesignationValue>, HtsError> {
    let rows = client
        .query(
            "SELECT cd.language, cd.use_system, cd.use_code, cd.value, COALESCE(s.version, '')
             FROM concept_designations cd
             JOIN concepts c ON c.id = cd.concept_id
             JOIN code_systems s ON s.id = c.system_id
             WHERE s.url = $1 AND c.code = $2 AND s.id <> $3
             ORDER BY COALESCE(s.version, '') DESC, cd.id",
            &[&url, &code, &exclude_system_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Language matching happens here rather than in SQL so the RFC 4647
    // fallback rules (`de-DE` → `de`, `de` → `de-CH`) apply.
    let lang_ok = |row: &tokio_postgres::Row| {
        row.get::<_, Option<String>>(0)
            .is_some_and(|l| crate::language::lang_matches(lang, &l))
    };
    let newest: String = match rows.iter().find(|r| lang_ok(r)) {
        Some(row) => row.get(4),
        None => return Ok(Vec::new()),
    };
    Ok(rows
        .into_iter()
        .filter(|row| row.get::<_, String>(4) == newest && lang_ok(row))
        .map(|row| DesignationValue {
            language: row.get(0),
            use_system: row.get(1),
            use_code: row.get(2),
            value: row.get(3),
            source: None,
        })
        .collect())
}

/// Return `true` if `ancestor_code` is a (possibly indirect) ancestor of
/// `descendant_code` within the given code system.
///
/// Uses a recursive CTE to walk up the parent chain.
pub(super) async fn check_ancestor(
    client: &tokio_postgres::Client,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    let rows = client
        .query(
            "WITH RECURSIVE ancestors(code) AS (
                 SELECT parent_code
                 FROM   concept_hierarchy
                 WHERE  system_id = $1 AND child_code = $3
                 UNION
                 SELECT h.parent_code
                 FROM   concept_hierarchy h
                 INNER JOIN ancestors a ON a.code = h.child_code
                 WHERE  h.system_id = $1
             )
             SELECT 1 FROM ancestors WHERE code = $2 LIMIT 1",
            &[&system_id, &ancestor_code, &descendant_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(!rows.is_empty())
}

/// Build a minimal synthetic FHIR resource JSON when `resource_json` is absent.
pub(super) fn build_synthetic_resource(
    resource_type: &str,
    id: &str,
    url: &str,
    version: Option<&str>,
    name: Option<&str>,
    title: Option<&str>,
    status: &str,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "resourceType": resource_type,
        "id": id,
        "url": url,
        "status": status,
    });
    if let Some(v) = version {
        obj["version"] = v.into();
    }
    if let Some(n) = name {
        obj["name"] = n.into();
    }
    if let Some(t) = title {
        obj["title"] = t.into();
    }
    obj
}
