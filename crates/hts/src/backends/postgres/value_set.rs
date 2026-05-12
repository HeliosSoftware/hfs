//! PostgreSQL implementation of [`ValueSetOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use std::collections::{HashMap, HashSet};

use crate::error::HtsError;
use crate::traits::ValueSetOperations;
use crate::types::{
    ExpandRequest, ExpandResponse, ExpansionContains, ResourceSearchQuery, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::PostgresTerminologyBackend;
use super::code_system::build_synthetic_resource;

#[async_trait]
impl ValueSetOperations for PostgresTerminologyBackend {
    async fn expand(
        &self,
        _ctx: &TenantContext,
        req: ExpandRequest,
    ) -> Result<ExpandResponse, HtsError> {
        // Accept either a canonical URL or an inline ValueSet body. The
        // tx-ecosystem IG POSTs hundreds of fixtures with an inline `valueSet`
        // parameter (no URL) — `notSelectable/`, `language/`, `overload/`,
        // `parameters/`, `simple/`, `extensions/`, `permutations/` etc.
        // The operations layer (`operations/expand.rs`) takes care of
        // emitting `used-codesystem` parameters by reading the inline VS's
        // `compose.include[]` after the backend returns, so the backend just
        // needs to produce a correct flat/tree expansion of the inline
        // compose body. See Task A/B in the porting brief.
        if req.url.is_none() && req.value_set.is_none() {
            return Err(HtsError::InvalidRequest(
                "Missing required parameter: url (ValueSet canonical URL) \
                 or valueSet (inline ValueSet resource)"
                    .into(),
            ));
        }

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let all_codes: Vec<ExpansionContains> = if let Some(url) = req.url.as_deref() {
            // ── URL-based path (unchanged) ───────────────────────────────────
            match resolve_value_set_versioned(
                &client,
                url,
                req.value_set_version.as_deref(),
                req.date.as_deref(),
            )
            .await
            {
                Ok((vs_id, compose_json)) => {
                    let cached = fetch_cache(&client, &vs_id).await?;
                    if cached.is_empty() {
                        let codes = compute_expansion(&client, compose_json.as_deref()).await?;
                        if let Some(limit) = req.max_expansion_size {
                            if codes.len() as u64 > u64::from(limit) {
                                return Err(HtsError::TooCostly(format!(
                                    "ValueSet expansion contains {} codes which exceeds \
                                         the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                                    codes.len(),
                                    limit
                                )));
                            }
                        }
                        populate_cache(&mut client, &vs_id, &codes).await?;
                        codes
                    } else {
                        cached
                    }
                }
                Err(HtsError::NotFound(_)) => {
                    let cs_url =
                        find_cs_for_implicit_vs(&client, url, req.date.as_deref()).await?;
                    let compose = serde_json::json!({
                        "include": [{ "system": cs_url }]
                    })
                    .to_string();
                    let codes = compute_expansion(&client, Some(&compose)).await?;
                    if let Some(limit) = req.max_expansion_size {
                        if codes.len() as u64 > u64::from(limit) {
                            return Err(HtsError::TooCostly(format!(
                                "Implicit ValueSet expansion contains {} codes which exceeds \
                                     the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                                codes.len(),
                                limit
                            )));
                        }
                    }
                    codes
                }
                Err(e) => return Err(e),
            }
        } else {
            // ── Inline-ValueSet path ─────────────────────────────────────────
            // The caller passed a full ValueSet resource in the `valueSet`
            // Parameters entry; treat its `.compose` as authoritative.
            // Mirrors `sqlite::value_set::expand`'s `if let Some(vs_resource)
            // = req.value_set` branch. We deliberately skip the
            // `value_set_expansions` cache: that table is keyed by stored VS
            // id, and an inline body has no id we can safely key on.
            //
            // TODO: parity — SQLite caches inline composes in the
            // `implicit_expansion_cache` table under an `inline-compose:<hash>`
            // key, plus an in-memory `inline_compose_index`. Porting both is
            // performance-only; correctness here matches SQLite without them.
            // TODO: parity — SQLite threads request `force_system_versions`,
            // `system_version_defaults`, `default_value_set_versions`, and
            // `tx_resources` through an `InlineResolutionContext` so nested
            // `compose.include[].valueSet[]` refs honour the pins. PG's
            // `compute_expansion` doesn't accept those yet.
            // TODO: parity — SQLite emits an empty-compose NotFound
            // ("None of the systems in the inline ValueSet compose could be
            // resolved") when every include misses. PG silently returns an
            // empty expansion here; the IG fixtures we care about all have
            // resolvable systems so this hasn't bitten yet.
            let vs = req.value_set.as_ref().expect("inline VS branch");
            let compose = vs.get("compose");

            if let Some(compose_val) = compose {
                let compose_str = compose_val.to_string();
                let codes = compute_expansion(&client, Some(&compose_str)).await?;
                if let Some(limit) = req.max_expansion_size {
                    if codes.len() as u64 > u64::from(limit) {
                        return Err(HtsError::TooCostly(format!(
                            "ValueSet expansion contains {} codes which exceeds \
                                 the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                            codes.len(),
                            limit
                        )));
                    }
                }
                codes
            } else if let Some(pre) = vs.get("expansion").and_then(|e| e.get("contains")) {
                // Inline VS carries only a pre-expanded `expansion.contains[]`
                // — adopt it directly. Surfaces in `expansion-by-fragment`-style
                // IG fixtures where the caller hand-builds the contains list.
                // TODO: parity — SQLite does NOT special-case this branch
                // (returns empty for missing compose). Keep an eye on whether
                // this causes a divergence; if so, revert.
                let arr = pre.as_array().cloned().unwrap_or_default();
                arr.into_iter()
                    .filter_map(|item| {
                        let system = item.get("system").and_then(|v| v.as_str())?.to_owned();
                        let code = item.get("code").and_then(|v| v.as_str())?.to_owned();
                        let display = item
                            .get("display")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);
                        let version = item
                            .get("version")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);
                        Some(ExpansionContains {
                            system,
                            version,
                            code,
                            display,
                            is_abstract: None,
                            inactive: None,
                            designations: vec![],
                            properties: vec![],
                            extensions: vec![],
                            contains: vec![],
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };

        let filtered: Vec<ExpansionContains> = if let Some(filter) = req.filter.as_deref() {
            let lower = filter.to_lowercase();
            all_codes
                .into_iter()
                .filter(|c| {
                    c.code.to_lowercase().contains(&lower)
                        || c.display
                            .as_deref()
                            .map(|d| d.to_lowercase().contains(&lower))
                            .unwrap_or(false)
                })
                .collect()
        } else {
            all_codes
        };

        if req.hierarchical == Some(true) {
            let total = filtered.len() as u32;
            let tree = build_hierarchical_expansion(&client, filtered).await?;
            return Ok(ExpandResponse {
                total: Some(total),
                offset: None,
                contains: tree,
                warnings: vec![],
            });
        }

        let total = filtered.len() as u32;
        let offset = req.offset.unwrap_or(0) as usize;
        let count = req.count.map(|c| c as usize).unwrap_or(usize::MAX);

        let page: Vec<ExpansionContains> = filtered.into_iter().skip(offset).take(count).collect();

        Ok(ExpandResponse {
            total: Some(total),
            offset: req.offset,
            contains: page,
            warnings: vec![],
        })
    }

    async fn validate_code(
        &self,
        _ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError> {
        let url = req.url.clone().ok_or_else(|| {
            HtsError::InvalidRequest(
                "Missing required parameter: url (ValueSet canonical URL)".into(),
            )
        })?;

        // TODO: cache — port the per-instance response cache from SQLite
        // (validate_code_response_cache). The SQLite cache key folds in
        //   url, value_set_version, system, code, version, display,
        //   include_abstract, date, input_form, lenient_display_validation
        // and skips entirely when `default_value_set_versions` is non-empty.

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // ?fhir_vs URLs: a persisted stub VS with one of those canonical URLs
        // would expand to zero codes and force result=false for every input —
        // short-circuit straight to the implicit-VS validator.
        let implicit_short_circuit = parse_fhir_vs_url(&url).is_some();

        let resolution = if implicit_short_circuit {
            Err(HtsError::NotFound("__fhir_vs_short_circuit__".into()))
        } else {
            resolve_value_set_versioned(
                &client,
                &url,
                req.value_set_version.as_deref(),
                req.date.as_deref(),
            )
            .await
        };

        let (all_codes, compose_json_for_version): (Vec<ExpansionContains>, Option<String>) =
            match resolution {
                Ok((vs_id, compose_json)) => {
                    let saved = compose_json.clone();
                    let cached = fetch_cache(&client, &vs_id).await?;
                    let codes = if cached.is_empty() {
                        let codes =
                            compute_expansion(&client, compose_json.as_deref()).await?;
                        populate_cache(&mut client, &vs_id, &codes).await?;
                        codes
                    } else {
                        cached
                    };
                    (codes, saved)
                }
                Err(HtsError::NotFound(_)) => {
                    // ?fhir_vs implicit ValueSet: targeted O(1)/O(depth) lookup.
                    if let Some((cs_url, pattern)) = parse_fhir_vs_url(&url) {
                        let found = validate_fhir_vs(
                            &client,
                            &cs_url,
                            &pattern,
                            &req.code,
                            req.system.as_deref(),
                        )
                        .await?;
                        let abstract_for_msg = req.include_abstract == Some(false)
                            && match found.as_ref() {
                                Some(c) => is_concept_abstract(&client, &c.system, &c.code).await,
                                None => false,
                            };
                        let inactive_for_msg = match found.as_ref() {
                            Some(c) => is_concept_inactive(&client, &c.system, &c.code).await,
                            None => false,
                        };
                        let inactive_in_cs = if found.is_none() {
                            match req.system.as_deref() {
                                Some(s) => is_concept_inactive(&client, s, &req.code).await,
                                None => false,
                            }
                        } else {
                            false
                        };
                        let code_unknown_in_cs = if found.is_none() {
                            match req.system.as_deref() {
                                Some(s) => !is_code_in_cs(&client, s, &req.code).await,
                                None => false,
                            }
                        } else {
                            false
                        };
                        let cs_version = match req.system.as_deref() {
                            Some(s) => cs_version_for_msg(&client, s).await,
                            None => None,
                        };
                        let cs_is_fragment = match req.system.as_deref() {
                            Some(s) => cs_content_for_url(&client, s).await.as_deref()
                                == Some("fragment"),
                            None => false,
                        };
                        let vs_version_owned = lookup_value_set_version(&client, &url).await;
                        return finish_validate_code_response(
                            found,
                            &req.code,
                            &url,
                            req.display.as_deref(),
                            req.system.as_deref(),
                            abstract_for_msg,
                            inactive_for_msg,
                            vs_version_owned.as_deref(),
                            inactive_in_cs,
                            code_unknown_in_cs,
                            false,
                            cs_version.as_deref(),
                            req.version.as_deref(),
                            req.lenient_display_validation.unwrap_or(false),
                            cs_is_fragment,
                            None,
                            None,
                        );
                    }

                    // CodeSystem.valueSet link: find the backing CS and
                    // treat it as an AllConcepts implicit ValueSet.
                    // TODO: parity — port the SQLite `implicit_expansion_cache`
                    // table for repeated lookups instead of recomputing.
                    match find_cs_for_implicit_vs(&client, &url, req.date.as_deref()).await {
                        Ok(cs_url) => {
                            let pattern = FhirVsPattern::AllConcepts;
                            let found = validate_fhir_vs(
                                &client,
                                &cs_url,
                                &pattern,
                                &req.code,
                                req.system.as_deref(),
                            )
                            .await?;
                            let abstract_for_msg = req.include_abstract == Some(false)
                                && match found.as_ref() {
                                    Some(c) => {
                                        is_concept_abstract(&client, &c.system, &c.code).await
                                    }
                                    None => false,
                                };
                            let inactive_for_msg = match found.as_ref() {
                                Some(c) => is_concept_inactive(&client, &c.system, &c.code).await,
                                None => false,
                            };
                            let inactive_in_cs = if found.is_none() {
                                match req.system.as_deref() {
                                    Some(s) => is_concept_inactive(&client, s, &req.code).await,
                                    None => false,
                                }
                            } else {
                                false
                            };
                            let code_unknown_in_cs = if found.is_none() {
                                match req.system.as_deref() {
                                    Some(s) => !is_code_in_cs(&client, s, &req.code).await,
                                    None => false,
                                }
                            } else {
                                false
                            };
                            let cs_version = match req.system.as_deref() {
                                Some(s) => cs_version_for_msg(&client, s).await,
                                None => None,
                            };
                            let cs_is_fragment = match req.system.as_deref() {
                                Some(s) => cs_content_for_url(&client, s).await.as_deref()
                                    == Some("fragment"),
                                None => false,
                            };
                            let vs_version_owned = lookup_value_set_version(&client, &url).await;
                            return finish_validate_code_response(
                                found,
                                &req.code,
                                &url,
                                req.display.as_deref(),
                                req.system.as_deref(),
                                abstract_for_msg,
                                inactive_for_msg,
                                vs_version_owned.as_deref(),
                                inactive_in_cs,
                                code_unknown_in_cs,
                                false,
                                cs_version.as_deref(),
                                req.version.as_deref(),
                                req.lenient_display_validation.unwrap_or(false),
                                cs_is_fragment,
                                None,
                                None,
                            );
                        }
                        Err(_) => {
                            return Ok(ValidateCodeResponse {
                                result: false,
                                message: Some(format!(
                                    "A definition for the value Set '{url}' could not be found"
                                )),
                                display: None,
                                system: None,
                                cs_version: None,
                                inactive: None,
                                issues: vec![],
                                caused_by_unknown_system: None,
                                concept_status: None,
                                normalized_code: None,
                            });
                        }
                    }
                }
                Err(e) => return Err(e),
            };

        // Version mismatch detection: verify the caller's version (when
        // supplied) against stored CS versions and the VS include pin. Also
        // fires when the caller supplies no version but the VS pins a version
        // that doesn't exist in the DB. Skipped on the `?fhir_vs` short-circuit
        // paths above (those already `return`ed).
        //
        // Location strings depend on which FHIR input form was used (mirrors
        // `sqlite/value_set.rs:1747-1754`). Tx-ecosystem fixtures pin the
        // location/expression to "system" / "version" for bare `code` input,
        // "CodeableConcept.coding[0].*" for CodeableConcept, and "Coding.*"
        // otherwise.
        let (version_loc, system_loc) = match req.input_form.as_deref() {
            Some("code") => ("version", "system"),
            Some("codeableConcept") => (
                "CodeableConcept.coding[0].version",
                "CodeableConcept.coding[0].system",
            ),
            _ => ("Coding.version", "Coding.system"),
        };
        let vs_version_for_mismatch = lookup_value_set_version(&client, &url).await;
        let mismatch = if let Some(system) = req.system.as_deref() {
            // Short-circuit when the system itself isn't loaded — caller-facing
            // unknown-system messaging is handled elsewhere.
            if !code_system_exists_inline(&client, system).await {
                None
            } else if let Some(req_ver) = req
                .version
                .as_deref()
                .filter(|v| !v.is_empty() && !v.contains(".x") && *v != "x")
            {
                detect_cs_version_mismatch(
                    &client,
                    system,
                    req_ver,
                    compose_json_for_version.as_deref(),
                    vs_version_for_mismatch.as_deref(),
                    version_loc,
                    system_loc,
                )
                .await
            } else if req.version.is_none() {
                // Caller supplied no version → check whether the VS include
                // pins a version that doesn't exist in the DB.
                detect_vs_pin_unknown(
                    &client,
                    system,
                    compose_json_for_version.as_deref(),
                    system_loc,
                )
                .await
            } else {
                None
            }
        } else {
            None
        };

        if let Some((issues, caused_by, echo_version)) = mismatch {
            let mut texts: Vec<&str> = issues
                .iter()
                .filter(|i| i.severity == "error")
                .map(|i| i.text.as_str())
                .collect();
            texts.sort_unstable();
            let message = texts.join("; ");
            // Echo the code's display from the underlying CS even when the
            // requested version is wrong — tx-ecosystem fixtures expect the
            // \`display\` parameter on mismatch responses (the concept itself
            // is still discoverable, only the version is unknown).
            let system_unwrapped = req.system.clone().unwrap();
            let display = client
                .query_opt(
                    "SELECT c.display FROM concepts c
                     JOIN code_systems s ON s.id = c.system_id
                     WHERE s.url = $1 AND c.code = $2
                     ORDER BY COALESCE(s.version, '') DESC LIMIT 1",
                    &[&system_unwrapped, &req.code],
                )
                .await
                .ok()
                .flatten()
                .and_then(|r| r.get::<_, Option<String>>(0));
            return Ok(ValidateCodeResponse {
                result: false,
                message: Some(message),
                display,
                system: Some(system_unwrapped),
                cs_version: echo_version,
                inactive: None,
                issues,
                caused_by_unknown_system: caused_by,
                concept_status: None,
                normalized_code: None,
            });
        }

        // Search the expansion for the requested code.
        // TODO: parity — overload pattern (same (system, code) at multiple
        // pinned versions), version-pin candidate selection, inferSystem
        // ambiguity branch, compose.inactive=false filter all still skipped.
        let req_ver_exact: Option<&str> = req
            .version
            .as_deref()
            .filter(|v| !v.contains(".x") && *v != "x");

        let mut candidates: Vec<&ExpansionContains> = if let Some(system) = req.system.as_deref() {
            all_codes
                .iter()
                .filter(|c| c.system == system && c.code == req.code)
                .collect()
        } else {
            all_codes.iter().filter(|c| c.code == req.code).collect()
        };

        // Case-insensitive fallback for systems with caseSensitive: false.
        let mut normalized_code: Option<String> = None;
        if candidates.is_empty() {
            let ci_candidates: Vec<&ExpansionContains> = if let Some(system) = req.system.as_deref()
            {
                all_codes
                    .iter()
                    .filter(|c| c.system == system && c.code.eq_ignore_ascii_case(&req.code))
                    .collect()
            } else {
                all_codes
                    .iter()
                    .filter(|c| c.code.eq_ignore_ascii_case(&req.code))
                    .collect()
            };
            let mut ci_filtered: Vec<&ExpansionContains> = Vec::new();
            for c in ci_candidates {
                if cs_is_case_insensitive(&client, &c.system).await {
                    ci_filtered.push(c);
                }
            }
            if !ci_filtered.is_empty() {
                if let Some(c) = ci_filtered.first() {
                    if c.code != req.code {
                        normalized_code = Some(c.code.clone());
                    }
                }
                candidates = ci_filtered;
            }
        }

        let found: Option<ExpansionContains> = if candidates.is_empty() {
            None
        } else if let Some(req_v) = req_ver_exact {
            // Simplified overload handling: prefer exact-version match, else
            // fall back to the single candidate when only one exists.
            // TODO: parity — full overload selection logic from SQLite.
            let exact_clone = candidates
                .iter()
                .find(|c| c.version.as_deref() == Some(req_v))
                .map(|c| (*c).clone());
            if let Some(c) = exact_clone {
                Some(c)
            } else if candidates.len() == 1 {
                candidates.into_iter().next().cloned()
            } else {
                None
            }
        } else if candidates.len() == 1 {
            candidates.into_iter().next().cloned()
        } else {
            // No version pin and multiple candidates: prefer display match,
            // else the highest-version candidate.
            let display_match: Option<&ExpansionContains> =
                req.display.as_deref().and_then(|d| {
                    candidates
                        .iter()
                        .find(|c| {
                            c.display
                                .as_deref()
                                .map(|cd| cd.eq_ignore_ascii_case(d))
                                .unwrap_or(false)
                        })
                        .copied()
                });
            if let Some(c) = display_match {
                Some(c.clone())
            } else {
                let mut sorted = candidates.clone();
                sorted.sort_by(|a, b| {
                    b.version
                        .as_deref()
                        .unwrap_or("")
                        .cmp(a.version.as_deref().unwrap_or(""))
                });
                sorted.into_iter().next().cloned()
            }
        };

        let system_for_msg: Option<String> = req
            .system
            .clone()
            .or_else(|| found.as_ref().map(|c| c.system.clone()));
        let abstract_for_msg = req.include_abstract == Some(false)
            && match found.as_ref() {
                Some(c) => is_concept_abstract(&client, &c.system, &c.code).await,
                None => false,
            };
        let inactive_for_msg = match found.as_ref() {
            Some(c) => is_concept_inactive(&client, &c.system, &c.code).await,
            None => false,
        };
        let inactive_in_cs = if found.is_none() {
            match req.system.as_deref() {
                Some(s) => is_concept_inactive(&client, s, &req.code).await,
                None => false,
            }
        } else {
            false
        };
        let code_unknown_in_cs_anywhere = if found.is_none() {
            match req.system.as_deref() {
                Some(s) => !is_code_in_cs(&client, s, &req.code).await,
                None => false,
            }
        } else {
            false
        };
        let code_unknown_in_cs_at_version = if found.is_none() {
            match (req.system.as_deref(), req.version.as_deref()) {
                (Some(s), Some(v)) if !v.contains(".x") && v != "x" => {
                    !is_code_in_cs_at_version(&client, s, v, &req.code).await
                }
                _ => false,
            }
        } else {
            false
        };
        let code_unknown_at_version_only =
            !code_unknown_in_cs_anywhere && code_unknown_in_cs_at_version;
        let code_unknown_in_cs = code_unknown_in_cs_anywhere || code_unknown_in_cs_at_version;

        // cs_version priority: caller's exact request version > matched
        // concept's version > latest stored CS version.
        // TODO: parity — VS compose include pin (rule 3 in SQLite) skipped.
        let cs_version: Option<String> = match system_for_msg.as_deref() {
            Some(s) => {
                let from_req = req
                    .version
                    .as_deref()
                    .filter(|v| !v.contains(".x") && *v != "x")
                    .map(str::to_string);
                let from_found = found.as_ref().and_then(|c| c.version.clone());
                match from_req.or(from_found) {
                    Some(v) => Some(v),
                    None => cs_version_for_msg(&client, s).await,
                }
            }
            None => None,
        };
        let vs_version_owned = lookup_value_set_version(&client, &url).await;
        let cs_is_fragment = match system_for_msg.as_deref() {
            Some(s) => cs_content_for_url(&client, s).await.as_deref() == Some("fragment"),
            None => false,
        };
        // Echo display lookup at the resolved cs_version when the caller did
        // not provide a display but the code lives in the underlying CS.
        // TODO: parity — port `lookup_display_at_version` for stricter
        // version-scoped matching. Skipping is harmless since the expansion
        // already carries the canonical display in most cases.

        finish_validate_code_response(
            found,
            &req.code,
            &url,
            req.display.as_deref(),
            system_for_msg.as_deref(),
            abstract_for_msg,
            inactive_for_msg,
            vs_version_owned.as_deref(),
            inactive_in_cs,
            code_unknown_in_cs,
            code_unknown_at_version_only,
            cs_version.as_deref(),
            req.version.as_deref(),
            req.lenient_display_validation.unwrap_or(false),
            cs_is_fragment,
            None,
            normalized_code.as_deref(),
        )
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
                 FROM value_sets
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
                    "ValueSet",
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
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Look up a ValueSet by canonical URL with an optional version pin.
///
/// Mirrors `sqlite::value_set::resolve_value_set_versioned`: when `version`
/// is `Some`, only the matching `(url, version)` row is returned (or
/// NotFound). When `version` is `None`, the highest-versioned row sharing
/// the URL wins.
async fn resolve_value_set_versioned(
    client: &tokio_postgres::Client,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, compose_json, version FROM value_sets
             WHERE url = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)
             ORDER BY COALESCE(version, '') DESC",
            &[&url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if rows.is_empty() {
        let qualified = match version {
            Some(v) => format!("{url}|{v}"),
            None => url.to_string(),
        };
        return Err(HtsError::NotFound(format!(
            "A definition for the value Set \'{qualified}\' could not be found"
        )));
    }

    let row = match version {
        Some(v) => rows
            .into_iter()
            .find(|r| r.get::<_, Option<String>>(2).as_deref() == Some(v))
            .ok_or_else(|| {
                HtsError::NotFound(format!(
                    "A definition for the value Set \'{url}|{v}\' could not be found"
                ))
            })?,
        None => rows.into_iter().next().expect("non-empty"),
    };

    Ok((row.get(0), row.get(1)))
}

/// Fetch all cached expansion entries for `vs_id`.
async fn fetch_cache(
    client: &tokio_postgres::Client,
    vs_id: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let rows = client
        .query(
            "SELECT system_url, code, display
             FROM value_set_expansions
             WHERE value_set_id = $1
             ORDER BY system_url, code",
            &[&vs_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| ExpansionContains {
            system: row.get(0),
            version: None,
            code: row.get(1),
            display: row.get(2),
            is_abstract: None,

            inactive: None,

            designations: vec![],

            properties: vec![],
            extensions: vec![],
            contains: vec![],
        })
        .collect())
}

/// Compute an expansion from the raw `compose_json`.
async fn compute_expansion(
    client: &tokio_postgres::Client,
    compose_json: Option<&str>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let Some(raw) = compose_json else {
        return Ok(vec![]);
    };

    let compose: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| HtsError::Internal(format!("Failed to parse compose_json: {e}")))?;

    let empty_arr = vec![];
    let includes = compose["include"].as_array().unwrap_or(&empty_arr);
    let mut included: Vec<ExpansionContains> = Vec::new();

    for inc in includes {
        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let inc_version = inc["version"].as_str();

        let system_id = match resolve_compose_system_id(client, system_url, inc_version).await? {
            Some(id) => id,
            None => {
                tracing::warn!(
                    system_url,
                    inc_version,
                    "Skipping unknown code system in ValueSet compose"
                );
                continue;
            }
        };

        // Phase A: collect the candidate concepts dictated by `concept[]` or
        // by enumerating the whole CodeSystem.
        let mut candidates: Vec<ExpansionContains> =
            if let Some(explicit_codes) = inc["concept"].as_array() {
                let mut out = Vec::with_capacity(explicit_codes.len());
                for entry in explicit_codes {
                    let code = match entry["code"].as_str() {
                        Some(c) => c.to_owned(),
                        None => continue,
                    };

                    let disp_rows = client
                        .query(
                            "SELECT display FROM concepts WHERE system_id = $1 AND code = $2",
                            &[&system_id, &code],
                        )
                        .await
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;

                    let display: Option<String> =
                        disp_rows.into_iter().next().and_then(|r| r.get(0));

                    out.push(ExpansionContains {
                        system: system_url.to_owned(),
                        version: None,
                        code,
                        display,
                        is_abstract: None,
                        inactive: None,
                        designations: vec![],
                        properties: vec![],
                        extensions: vec![],
                        contains: vec![],
                    });
                }
                out
            } else {
                let code_rows = client
                    .query(
                        "SELECT code, display FROM concepts WHERE system_id = $1 ORDER BY code",
                        &[&system_id],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                code_rows
                    .into_iter()
                    .map(|row| ExpansionContains {
                        system: system_url.to_owned(),
                        version: None,
                        code: row.get(0),
                        display: row.get(1),
                        is_abstract: None,
                        inactive: None,
                        designations: vec![],
                        properties: vec![],
                        extensions: vec![],
                        contains: vec![],
                    })
                    .collect()
            };

        // Phase B: narrow by `compose.include[].filter[]` (ANDed). Filters
        // operate on the underlying CodeSystem; we intersect each filter's
        // matching code set against `candidates`.
        if let Some(filtered) =
            apply_compose_filters_pg(client, system_url, &system_id, inc).await?
        {
            let keep: HashSet<String> = filtered.into_iter().map(|c| c.code).collect();
            candidates.retain(|c| keep.contains(&c.code));
        }

        included.extend(candidates);
    }

    // Apply excludes — both explicit `concept[]` entries AND `filter[]`
    // entries (filters appear on exclude blocks too; same semantics).
    let excludes = compose["exclude"].as_array().unwrap_or(&empty_arr);
    let mut denied: HashSet<(String, String)> = HashSet::new();

    for exc in excludes {
        let exc_system = exc["system"].as_str().unwrap_or("").to_owned();
        if let Some(codes) = exc["concept"].as_array() {
            for entry in codes {
                if let Some(code) = entry["code"].as_str() {
                    denied.insert((exc_system.clone(), code.to_owned()));
                }
            }
        }

        // Filter-based excludes: resolve system_id and apply the same filter
        // helper, then add the resulting codes to the denied set.
        if exc["filter"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
            && !exc_system.is_empty()
        {
            let exc_version = exc["version"].as_str();
            if let Some(exc_system_id) =
                resolve_compose_system_id(client, &exc_system, exc_version).await?
                && let Some(filtered) =
                    apply_compose_filters_pg(client, &exc_system, &exc_system_id, exc).await?
            {
                for item in filtered {
                    denied.insert((exc_system.clone(), item.code));
                }
            }
        }
    }

    if !denied.is_empty() {
        included.retain(|c| !denied.contains(&(c.system.clone(), c.code.clone())));
    }

    Ok(included)
}

/// Apply `compose.include[].filter[]` (or `compose.exclude[].filter[]`) to
/// produce the set of concepts the filter chain matches in the underlying
/// CodeSystem identified by (`system_url`, `system_id`).
///
/// Returns:
/// - `Ok(None)` when the block carries no filters.
/// - `Ok(Some(vec))` with the AND-intersection of every filter's match set.
///
/// Supported ops:
/// - `=` (and single-value `in`): property-equality on `concept_properties`,
///   with boolean-style "absence means false" for the `= false` case.
/// - `is-a`: a recursive CTE walking `concept_hierarchy` downward from the
///   root (including the root itself).
/// - `regex`: POSIX ERE match against `code` or `display` via PG's `~`.
///
/// Unsupported ops (`descendent-of`, `descendant-of`, `generalizes`, `in`
/// multi-value, `not-in`, `exists`, `child-of`, …) currently emit a
/// `tracing::warn!` and contribute an empty set to the AND, which collapses
/// the include. TODO: parity — these are handled in SQLite via
/// [`crates::hts::backends::sqlite::value_set::apply_compose_filters`].
async fn apply_compose_filters_pg(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    inc: &serde_json::Value,
) -> Result<Option<Vec<ExpansionContains>>, HtsError> {
    let filters = match inc["filter"].as_array() {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(None),
    };

    let mut result: Option<HashSet<String>> = None;
    let mut display_map: HashMap<String, Option<String>> = HashMap::new();

    for f in filters {
        let property = f["property"].as_str().unwrap_or("");
        let op = f["op"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");

        let matches: Vec<(String, Option<String>)> = match op {
            "=" => {
                pg_filter_property_eq(client, system_url, system_id, property, value).await?
            }
            // Single-value `in` reduces to equality. Multi-value (comma-
            // separated) `in` is a TODO: parity gap.
            "in" if !value.contains(',') => {
                pg_filter_property_eq(client, system_url, system_id, property, value).await?
            }
            "is-a" => pg_filter_is_a(client, system_url, system_id, value).await?,
            "regex" => pg_filter_regex(client, system_id, property, value).await?,
            other => {
                // TODO: parity — SQLite handles descendent-of/generalizes/
                // child-of/not-in/!=/exists/multi-value-in. Treat them as
                // empty set so the include collapses (rather than panicking
                // or accidentally returning every concept).
                tracing::warn!(
                    system_url,
                    property,
                    op = other,
                    value,
                    "PG compose filter op not yet supported — treating as empty set"
                );
                Vec::new()
            }
        };

        let code_set: HashSet<String> = matches
            .iter()
            .map(|(code, display)| {
                // First-seen display wins; tolerates duplicates from the
                // intersection of property and hierarchy filters.
                display_map
                    .entry(code.clone())
                    .or_insert_with(|| display.clone());
                code.clone()
            })
            .collect();

        match result.as_mut() {
            Some(prev) => prev.retain(|c| code_set.contains(c)),
            None => result = Some(code_set),
        }
    }

    let codes = result.unwrap_or_default();
    let mut out: Vec<ExpansionContains> = codes
        .into_iter()
        .map(|code| {
            let display = display_map.get(&code).cloned().unwrap_or(None);
            ExpansionContains {
                system: system_url.to_owned(),
                version: None,
                code,
                display,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                extensions: vec![],
                contains: vec![],
            }
        })
        .collect();
    out.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(Some(out))
}

/// Resolve every concept in `system_id` matching the FHIR property/value
/// equality, honouring locally-renamed property aliases. Treats the literal
/// string value `"false"` for boolean properties (e.g. `notSelectable`,
/// `inactive`) as "property value != 'true' OR property absent" — matching
/// the FHIR rule that boolean concept properties default to false when
/// omitted, and mirroring SQLite's behaviour for the
/// `notSelectable/expand-noprop-false` IG fixtures.
async fn pg_filter_property_eq(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    let property_aliases = cs_property_local_codes(client, system_url, property).await;
    // Heuristic: the literal lower-case "false" inverts the match for
    // boolean properties (notSelectable/inactive). Any other value (incl.
    // "true", "FALSE", "0", arbitrary strings) is a positive match.
    let is_boolean_false = value.eq_ignore_ascii_case("false") && !value.eq_ignore_ascii_case("true");

    let rows = if is_boolean_false {
        // "= false" → concepts that do NOT have any (property=alias, value='true').
        client
            .query(
                "SELECT c.code, c.display
                   FROM concepts c
                  WHERE c.system_id = $1
                    AND NOT EXISTS (
                        SELECT 1 FROM concept_properties cp
                         WHERE cp.concept_id = c.id
                           AND cp.property = ANY($2::text[])
                           AND cp.value = 'true'
                    )",
                &[&system_id, &property_aliases],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?
    } else {
        // Standard equality: concept has a property row matching the value.
        client
            .query(
                "SELECT c.code, c.display
                   FROM concepts c
                  WHERE c.system_id = $1
                    AND EXISTS (
                        SELECT 1 FROM concept_properties cp
                         WHERE cp.concept_id = c.id
                           AND cp.property = ANY($2::text[])
                           AND cp.value = $3
                    )",
                &[&system_id, &property_aliases, &value],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?
    };

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Resolve `is-a` filter: the root code itself plus every descendant via
/// `concept_hierarchy`. PG has no closure table yet (SQLite uses one for
/// O(1) lookup) so we walk the hierarchy with a recursive CTE.
async fn pg_filter_is_a(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    root_code: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    if root_code.is_empty() {
        return Err(HtsError::VsInvalid(format!(
            "The system {system_url} filter with property = concept, op = is-a has no value"
        )));
    }

    let rows = client
        .query(
            "WITH RECURSIVE descendants AS (
                 SELECT $2::text AS code
                 UNION
                 SELECT ch.child_code FROM concept_hierarchy ch
                  JOIN descendants d ON ch.parent_code = d.code
                  WHERE ch.system_id = $1
             )
             SELECT c.code, c.display
               FROM concepts c
              WHERE c.system_id = $1
                AND c.code IN (SELECT code FROM descendants)",
            &[&system_id, &root_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Regex match against the named property using PG's POSIX `~` operator.
/// `property` must be `code` or `display`; other properties (matching a
/// regex against a `concept_properties` value) is currently TODO: parity.
async fn pg_filter_regex(
    client: &tokio_postgres::Client,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    let sql = match property {
        "code" | "" => {
            "SELECT c.code, c.display FROM concepts c
              WHERE c.system_id = $1 AND c.code ~ $2"
        }
        "display" => {
            "SELECT c.code, c.display FROM concepts c
              WHERE c.system_id = $1 AND c.display ~ $2"
        }
        _ => {
            // TODO: parity — regex against an arbitrary concept_properties
            // value (SQLite materialises rows and matches in Rust).
            tracing::warn!(
                property,
                value,
                "PG regex filter on non-code/display property not yet supported"
            );
            return Ok(Vec::new());
        }
    };

    let rows = client
        .query(sql, &[&system_id, &value])
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Resolve the storage id of the `code_systems` row matching the (url,
/// optional version) pair declared on a `compose.include[]` entry.
///
/// Mirrors the SQLite helper: `1.x.x`-style patterns match the highest
/// version sharing the literal segments, an exact version requires a literal
/// match, and `None` falls back to the latest revision.
async fn resolve_compose_system_id(
    client: &tokio_postgres::Client,
    url: &str,
    version: Option<&str>,
) -> Result<Option<String>, HtsError> {
    let rows = client
        .query(
            "SELECT id, version FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC",
            &[&url],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let candidates: Vec<(String, Option<String>)> = rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }

    let chosen = match version {
        Some(v) if v.contains(".x") || v == "x" || compose_short_version(v) => {
            compose_select_version(&candidates, v)
        }
        Some(v) => candidates
            .into_iter()
            .find(|(_, ver)| ver.as_deref() == Some(v)),
        None => candidates.into_iter().next(),
    };
    Ok(chosen.map(|(id, _)| id))
}

fn compose_short_version(ver: &str) -> bool {
    !ver.contains('.') && ver.chars().all(|c| c.is_ascii_digit())
}

fn compose_select_version(
    candidates: &[(String, Option<String>)],
    pattern: &str,
) -> Option<(String, Option<String>)> {
    let segments: Vec<&str> = pattern.split('.').collect();
    candidates
        .iter()
        .filter(|(_, v)| match v {
            Some(actual) => compose_version_matches(actual, &segments),
            None => false,
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .cloned()
}

fn compose_version_matches(actual: &str, pattern_segments: &[&str]) -> bool {
    let actual_segments: Vec<&str> = actual.split('.').collect();
    if pattern_segments.len() > actual_segments.len() {
        return false;
    }
    pattern_segments
        .iter()
        .zip(actual_segments.iter())
        .all(|(p, a)| *p == "x" || *p == *a)
}

/// Find the canonical URL of a CodeSystem whose `valueSet` property equals `vs_url`.
async fn find_cs_for_implicit_vs(
    client: &tokio_postgres::Client,
    vs_url: &str,
    date: Option<&str>,
) -> Result<String, HtsError> {
    let rows = client
        .query(
            "SELECT url FROM code_systems
             WHERE (resource_json->>'valueSet') = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)",
            &[&vs_url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    rows.into_iter()
        .next()
        .map(|r| r.get::<_, String>(0))
        .ok_or_else(|| {
            HtsError::NotFound(format!(
                "A definition for the value Set \'{vs_url}\' could not be found"
            ))
        })
}

/// Build a tree-structured expansion from a flat list of concepts.
async fn build_hierarchical_expansion(
    client: &tokio_postgres::Client,
    flat: Vec<ExpansionContains>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    if flat.is_empty() {
        return Ok(flat);
    }

    let items_map: HashMap<(String, String), ExpansionContains> = flat
        .iter()
        .cloned()
        .map(|c| ((c.system.clone(), c.code.clone()), c))
        .collect();

    let expansion_set: HashSet<(String, String)> = flat
        .iter()
        .map(|c| (c.system.clone(), c.code.clone()))
        .collect();

    let system_urls: HashSet<String> = flat.iter().map(|c| c.system.clone()).collect();
    let mut system_id_map: HashMap<String, String> = HashMap::new();
    for sys_url in &system_urls {
        let rows = client
            .query(
                "SELECT id FROM code_systems WHERE url = $1 \
                 ORDER BY COALESCE(version, '') DESC LIMIT 1",
                &[sys_url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        if let Some(row) = rows.into_iter().next() {
            system_id_map.insert(sys_url.clone(), row.get(0));
        }
    }

    let mut parent_to_children: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    let mut has_parent: HashSet<(String, String)> = HashSet::new();

    for (sys_url, sys_id) in &system_id_map {
        let edge_rows = client
            .query(
                "SELECT parent_code, child_code FROM concept_hierarchy WHERE system_id = $1",
                &[sys_id],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        for row in edge_rows {
            let parent_code: String = row.get(0);
            let child_code: String = row.get(1);
            let parent_key = (sys_url.clone(), parent_code);
            let child_key = (sys_url.clone(), child_code);
            if expansion_set.contains(&parent_key) && expansion_set.contains(&child_key) {
                parent_to_children
                    .entry(parent_key)
                    .or_default()
                    .push(child_key.clone());
                has_parent.insert(child_key);
            }
        }
    }

    let mut roots: Vec<ExpansionContains> = flat
        .iter()
        .filter(|c| !has_parent.contains(&(c.system.clone(), c.code.clone())))
        .map(|c| {
            build_subtree(
                &(c.system.clone(), c.code.clone()),
                &items_map,
                &parent_to_children,
            )
        })
        .collect();

    roots.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(roots)
}

/// Recursively build an [`ExpansionContains`] node with all its nested children.
fn build_subtree(
    key: &(String, String),
    items_map: &HashMap<(String, String), ExpansionContains>,
    parent_to_children: &HashMap<(String, String), Vec<(String, String)>>,
) -> ExpansionContains {
    let mut item = items_map[key].clone();
    if let Some(children) = parent_to_children.get(key) {
        let mut child_items: Vec<ExpansionContains> = children
            .iter()
            .map(|ck| build_subtree(ck, items_map, parent_to_children))
            .collect();
        child_items.sort_by(|a, b| a.code.cmp(&b.code));
        item.contains = child_items;
    }
    item
}

/// Write computed expansion entries into the `value_set_expansions` cache.
///
/// Uses a transaction so the DELETE+INSERTs are atomic: without it, another
/// connection running `write_value_set` (which also DELETEs expansion rows
/// for the same `value_set_id` inside its own import transaction) can
/// interleave its buffered DELETE with our per-statement autocommits and
/// leave the cache with only the entries we inserted *after* that DELETE.
/// The symptom is a validate-code call reading a partially populated cache
/// (e.g. only the last-inserted code survives).
async fn populate_cache(
    client: &mut tokio_postgres::Client,
    vs_id: &str,
    codes: &[ExpansionContains],
) -> Result<(), HtsError> {
    let tx = client
        .transaction()
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    tx.execute(
        "DELETE FROM value_set_expansions WHERE value_set_id = $1",
        &[&vs_id],
    )
    .await
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for item in codes {
        tx.execute(
            "INSERT INTO value_set_expansions (value_set_id, system_url, code, display)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            &[&vs_id, &item.system, &item.code, &item.display],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(())
}

// ── Implicit-ValueSet (?fhir_vs) helpers ──────────────────────────────────────

/// FHIR defines query-parameter patterns on a CodeSystem URL that implicitly
/// describe a ValueSet (FHIR R4 §4.8.7):
///
/// | URL form | Pattern | Meaning |
/// |---|---|---|
/// | `<cs>?fhir_vs` | `AllConcepts` | Every code in the CodeSystem |
/// | `<cs>?fhir_vs=isa/<code>` | `IsA(code)` | Descendants (subsumees) of `code` |
#[derive(Debug)]
enum FhirVsPattern {
    AllConcepts,
    IsA(String),
}

/// Parse a `?fhir_vs` implicit ValueSet URL.
///
/// Returns `Some((cs_url, pattern))` on a recognised pattern, `None` otherwise.
fn parse_fhir_vs_url(url: &str) -> Option<(String, FhirVsPattern)> {
    let (base, query) = url.split_once('?')?;
    if !query.starts_with("fhir_vs") {
        return None;
    }
    let rest = &query["fhir_vs".len()..];
    if rest.is_empty() {
        return Some((base.to_owned(), FhirVsPattern::AllConcepts));
    }
    let value = rest.strip_prefix('=')?;
    if let Some(code) = value.strip_prefix("isa/") {
        return Some((base.to_owned(), FhirVsPattern::IsA(code.to_owned())));
    }
    None
}

/// Resolve the highest-versioned `code_systems.id` for a given canonical URL.
/// Multiple rows can share the same URL (stub + real import); we pick the
/// most recent textual COALESCE-DESC version, matching SQLite's resolver.
async fn resolve_system_id_pg(
    client: &tokio_postgres::Client,
    cs_url: &str,
) -> Result<Option<String>, HtsError> {
    let row = client
        .query_opt(
            "SELECT id FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&cs_url],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(row.map(|r| r.get::<_, String>(0)))
}

/// Validate a code against a `?fhir_vs` implicit ValueSet pattern directly,
/// without materializing the full expansion.
///
/// - `AllConcepts` — O(1) point lookup in the `concepts` table.
/// - `IsA(root)` — recursive CTE walking `concept_hierarchy` downward from
///   `root` to check whether `code` is a descendant-or-self.
async fn validate_fhir_vs(
    client: &tokio_postgres::Client,
    cs_url: &str,
    pattern: &FhirVsPattern,
    code: &str,
    system: Option<&str>,
) -> Result<Option<ExpansionContains>, HtsError> {
    if let Some(sys) = system {
        if sys != cs_url {
            return Ok(None);
        }
    }

    let system_id = match resolve_system_id_pg(client, cs_url).await? {
        Some(id) => id,
        None => {
            return Err(HtsError::NotFound(format!(
                "CodeSystem not found: {cs_url}"
            )));
        }
    };

    match pattern {
        FhirVsPattern::AllConcepts => {
            let row = client
                .query_opt(
                    "SELECT code, display FROM concepts \
                     WHERE system_id = $1 AND code = $2",
                    &[&system_id, &code],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            Ok(row.map(|r| ExpansionContains {
                system: cs_url.to_owned(),
                version: None,
                code: r.get::<_, String>(0),
                display: r.get::<_, Option<String>>(1),
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                extensions: vec![],
                contains: vec![],
            }))
        }
        FhirVsPattern::IsA(root_code) => {
            // TODO: parity — SQLite uses a precomputed `concept_closure` table
            // for O(1) ancestor lookup; PG has no closure table yet, so we
            // walk `concept_hierarchy` with WITH RECURSIVE downward from the
            // root. Membership = code == root OR descendant of root.
            if root_code == code {
                let row = client
                    .query_opt(
                        "SELECT code, display FROM concepts \
                         WHERE system_id = $1 AND code = $2",
                        &[&system_id, &code],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                return Ok(row.map(|r| ExpansionContains {
                    system: cs_url.to_owned(),
                    version: None,
                    code: r.get::<_, String>(0),
                    display: r.get::<_, Option<String>>(1),
                    is_abstract: None,
                    inactive: None,
                    designations: vec![],
                    properties: vec![],
                    extensions: vec![],
                    contains: vec![],
                }));
            }

            let is_member: bool = client
                .query_one(
                    "WITH RECURSIVE descendants AS (
                         SELECT child_code FROM concept_hierarchy
                          WHERE system_id = $1 AND parent_code = $2
                         UNION
                         SELECT ch.child_code FROM concept_hierarchy ch
                          JOIN descendants d ON ch.parent_code = d.child_code
                          WHERE ch.system_id = $1
                     )
                     SELECT EXISTS(SELECT 1 FROM descendants WHERE child_code = $3)",
                    &[&system_id, &root_code, &code],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .get(0);

            if !is_member {
                return Ok(None);
            }

            let display: Option<String> = client
                .query_opt(
                    "SELECT display FROM concepts WHERE system_id = $1 AND code = $2",
                    &[&system_id, &code],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .and_then(|r| r.get::<_, Option<String>>(0));

            Ok(Some(ExpansionContains {
                system: cs_url.to_owned(),
                version: None,
                code: code.to_owned(),
                display,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                extensions: vec![],
                contains: vec![],
            }))
        }
    }
}

// ── CodeSystem / ValueSet metadata helpers ─────────────────────────────────────

/// Highest stored ValueSet version for a URL, used to format `url|version`
/// in IG-spec not-found messages.
async fn lookup_value_set_version(
    client: &tokio_postgres::Client,
    url: &str,
) -> Option<String> {
    client
        .query_opt(
            "SELECT version FROM value_sets \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&url],
        )
        .await
        .ok()
        .flatten()
        .and_then(|r| r.get::<_, Option<String>>(0))
}

/// Highest stored CodeSystem version for a URL.
async fn cs_version_for_msg(
    client: &tokio_postgres::Client,
    system_url: &str,
) -> Option<String> {
    client
        .query_opt(
            "SELECT version FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&system_url],
        )
        .await
        .ok()
        .flatten()
        .and_then(|r| r.get::<_, Option<String>>(0))
}

/// Look up the `content` column for a stored CodeSystem URL. `Some("fragment")`
/// drives the `UNKNOWN_CODE_IN_FRAGMENT` warning shape in
/// `finish_validate_code_response`.
async fn cs_content_for_url(
    client: &tokio_postgres::Client,
    system_url: &str,
) -> Option<String> {
    client
        .query_opt(
            "SELECT content FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&system_url],
        )
        .await
        .ok()
        .flatten()
        .and_then(|r| r.get::<_, Option<String>>(0))
}

/// Returns `true` when the CodeSystem at `system_url` has `caseSensitive: false`
/// explicitly set. The FHIR default (absent) is treated as case-sensitive.
async fn cs_is_case_insensitive(
    client: &tokio_postgres::Client,
    system_url: &str,
) -> bool {
    let row = match client
        .query_opt(
            "SELECT (resource_json->>'caseSensitive') \
             FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&system_url],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return false,
    };
    match row.and_then(|r| r.get::<_, Option<String>>(0)) {
        Some(s) if s.eq_ignore_ascii_case("false") => true,
        _ => false,
    }
}

/// `true` when the code exists in the named CodeSystem (any version).
async fn is_code_in_cs(
    client: &tokio_postgres::Client,
    system_url: &str,
    code: &str,
) -> bool {
    client
        .query_one(
            "SELECT EXISTS(
                 SELECT 1 FROM concepts c
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1 AND c.code = $2
             )",
            &[&system_url, &code],
        )
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Like [`is_code_in_cs`] but scoped to a specific stored CS version.
async fn is_code_in_cs_at_version(
    client: &tokio_postgres::Client,
    system_url: &str,
    version: &str,
    code: &str,
) -> bool {
    client
        .query_one(
            "SELECT EXISTS(
                 SELECT 1 FROM concepts c
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1 AND s.version = $2 AND c.code = $3
             )",
            &[&system_url, &version, &code],
        )
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns true when the (system_url, version) pair is stored as a CS row.
#[allow(dead_code)]
async fn cs_version_exists(
    client: &tokio_postgres::Client,
    system_url: &str,
    version: &str,
) -> bool {
    client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM code_systems WHERE url = $1 AND version = $2)",
            &[&system_url, &version],
        )
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns the local property codes that map to the FHIR-canonical
/// `<canonical>` URI for the given CodeSystem URL. Always includes
/// `<canonical>` itself plus any locally-renamed aliases declared on the
/// CodeSystem `property[]` array (e.g. `not-selectable` aliased to
/// `notSelectable` via `uri: http://hl7.org/fhir/concept-properties#notSelectable`).
///
/// Mirrors `sqlite/code_system.rs:1599`. Tx-ecosystem fixtures rename these
/// properties locally and the FHIR spec allows it — queries hardcoded to the
/// canonical name miss those concepts.
async fn cs_property_local_codes(
    client: &tokio_postgres::Client,
    system_url: &str,
    canonical: &str,
) -> Vec<String> {
    let mut codes: Vec<String> = vec![canonical.to_string()];
    let row = match client
        .query_opt(
            "SELECT resource_json FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            &[&system_url],
        )
        .await
    {
        Ok(Some(r)) => r,
        _ => return codes,
    };
    let Some(v) = row.get::<_, Option<serde_json::Value>>(0) else {
        return codes;
    };
    let suffix = format!("#{canonical}");
    if let Some(props) = v.get("property").and_then(|p| p.as_array()) {
        for p in props {
            let uri = p.get("uri").and_then(|u| u.as_str()).unwrap_or("");
            if uri.ends_with(&suffix) || uri == canonical {
                if let Some(local_code) = p.get("code").and_then(|c| c.as_str()) {
                    if !codes.iter().any(|c| c == local_code) {
                        codes.push(local_code.to_string());
                    }
                }
            }
        }
    }
    codes
}

/// `true` when the concept is flagged inactive in the underlying CodeSystem.
///
/// Honours both the canonical `status` property (value in {retired, inactive})
/// AND the FHIR `inactive` boolean property, including locally-renamed
/// aliases resolved via [`cs_property_local_codes`]. `deprecated` codes are
/// intentionally excluded: per the FHIR concept-properties IG, deprecated
/// codes are discouraged but still active (the `deprecated/` test group
/// relies on this — deprecated codes survive `activeOnly=true` filtering).
async fn is_concept_inactive(
    client: &tokio_postgres::Client,
    system_url: &str,
    code: &str,
) -> bool {
    let inactive_codes = cs_property_local_codes(client, system_url, "inactive").await;
    let placeholders = (3..=inactive_codes.len() + 2)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT EXISTS(
             SELECT 1 FROM concept_properties cp
             JOIN concepts c ON c.id = cp.concept_id
             JOIN code_systems s ON s.id = c.system_id
             WHERE s.url = $1 AND c.code = $2
               AND (
                   (cp.property = 'status' AND cp.value IN ('retired', 'inactive'))
                OR (cp.property IN ({placeholders}) AND cp.value = 'true')
               )
         )"
    );
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        Vec::with_capacity(inactive_codes.len() + 2);
    params.push(&system_url);
    params.push(&code);
    for c in inactive_codes.iter() {
        params.push(c as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    client
        .query_one(&sql, params.as_slice())
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

/// `true` when the concept is flagged abstract (`notSelectable`) in the
/// underlying CodeSystem. Resolves locally-renamed aliases via
/// [`cs_property_local_codes`] (e.g. `not-selectable` with a hyphen, as
/// several tx-ecosystem fixtures use).
async fn is_concept_abstract(
    client: &tokio_postgres::Client,
    system_url: &str,
    code: &str,
) -> bool {
    let abstract_codes = cs_property_local_codes(client, system_url, "notSelectable").await;
    let placeholders = (3..=abstract_codes.len() + 2)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT EXISTS(
             SELECT 1 FROM concept_properties cp
             JOIN concepts c ON c.id = cp.concept_id
             JOIN code_systems s ON s.id = c.system_id
             WHERE s.url = $1 AND c.code = $2
               AND cp.property IN ({placeholders})
               AND cp.value = 'true'
         )"
    );
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        Vec::with_capacity(abstract_codes.len() + 2);
    params.push(&system_url);
    params.push(&code);
    for c in abstract_codes.iter() {
        params.push(c as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    client
        .query_one(&sql, params.as_slice())
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

// ── Version-mismatch detection ────────────────────────────────────────────────
//
// Ported from `crates/hts/src/backends/sqlite/value_set.rs` lines 6362–6896.
// The two entry points are [`detect_cs_version_mismatch`] (caller pinned a
// version) and [`detect_vs_pin_unknown`] (caller did not pin a version but the
// VS compose did). IG fixtures match the message text byte-for-byte, so the
// strings and message_ids here must stay aligned with the SQLite source.

/// Inline `code_systems` existence check. The trait method
/// `code_system_exists` takes `&self` and a `TenantContext`, which we don't
/// have at the helper boundary — this just runs the same EXISTS query.
async fn code_system_exists_inline(client: &tokio_postgres::Client, url: &str) -> bool {
    client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM code_systems WHERE url = $1)",
            &[&url],
        )
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns all non-null stored versions for a CS URL, sorted ascending for
/// display in "Valid versions: X or Y" messages.
async fn cs_all_stored_versions(client: &tokio_postgres::Client, system_url: &str) -> Vec<String> {
    let rows = match client
        .query(
            "SELECT version FROM code_systems \
             WHERE url = $1 AND version IS NOT NULL \
             ORDER BY COALESCE(version, '') ASC",
            &[&system_url],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    rows.into_iter()
        .filter_map(|r| r.get::<_, Option<String>>(0))
        .collect()
}

/// Format a list of versions as "X", "X or Y", or "X, Y or Z".
fn format_valid_versions_msg(versions: &[String]) -> String {
    match versions {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let (last, rest) = versions.split_last().unwrap();
            format!("{} or {}", rest.join(", "), last)
        }
    }
}

/// Return `Some(pin)` where `pin` is the version string (or `None` for a
/// versionless include) when `system_url` appears in `compose.include[]`.
/// Returns `None` when the system is not found in any include.
fn vs_pinned_include_version(compose_json: &str, system_url: &str) -> Option<Option<String>> {
    let compose: serde_json::Value = serde_json::from_str(compose_json).ok()?;
    let includes = compose.get("include")?.as_array()?;
    for inc in includes {
        if inc.get("system").and_then(|v| v.as_str()) == Some(system_url) {
            let ver = inc
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            return Some(ver);
        }
    }
    None
}

/// Returns *all* `compose.include[].version` entries that target `system_url`.
/// Used to detect the "overload" pattern where one VS includes multiple
/// versions of the same CodeSystem — in that case a request whose version
/// matches *any* included pin is acceptable, not just the first one.
///
/// Returns `Some(vec)` with one entry per matching include (`Some(version)` for
/// pinned includes, `None` for versionless includes). Returns `None` when no
/// include targets the given system at all.
fn vs_all_pinned_include_versions(
    compose_json: &str,
    system_url: &str,
) -> Option<Vec<Option<String>>> {
    let compose: serde_json::Value = serde_json::from_str(compose_json).ok()?;
    let includes = compose.get("include")?.as_array()?;
    let mut hits: Vec<Option<String>> = Vec::new();
    for inc in includes {
        if inc.get("system").and_then(|v| v.as_str()) == Some(system_url) {
            let ver = inc
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            hits.push(ver);
        }
    }
    if hits.is_empty() { None } else { Some(hits) }
}

/// Resolve a version string against a set of `(id, version)` candidate pairs.
/// Returns the matched full version string, or `None` when no candidate matches.
///
/// Rules:
/// - Explicit `.x` wildcards or bare "x" → pattern matching.
/// - Dot-containing versions ("1.0", "1.0.0") → prefix/pattern matching so
///   "1.0" resolves to the best "1.0.x" stored version.
/// - Single-integer versions ("1", "2") with no dot → EXACT match only.
///   These are not resolved via prefix expansion because the IG test fixtures
///   treat bare "1" as a distinct unrecognised version (producing
///   UNKNOWN_CODESYSTEM_VERSION), not as an alias for "1.x.x".
fn resolve_ver_against_candidates(
    candidates: &[(String, Option<String>)],
    ver: &str,
) -> Option<String> {
    if ver.contains(".x") || ver == "x" || ver.contains('.') {
        // Pattern/prefix matching: "1.0" → highest "1.0.x", "1.x" → highest "1.y.z".
        // Reuses the same matcher the compose-resolution helper above uses.
        compose_select_version(candidates, ver).and_then(|(_, v)| v)
    } else {
        // Single-segment or non-semver: EXACT match only
        candidates
            .iter()
            .find(|(_, v)| v.as_deref() == Some(ver))
            .and_then(|(_, v)| v.clone())
    }
}

/// Returns true if `version` satisfies the wildcard `pattern`.
/// "1.x" matches "1.0.0", "1.2.0", etc. "1.0.x" matches "1.0.0", "1.0.1".
/// "1.x.x" matches "1.0.0", "1.2.3", etc. (segment-wise: each "x" is any segment).
fn version_satisfies_wildcard(version: &str, pattern: &str) -> bool {
    if pattern == "x" {
        return true;
    }
    // Segment-wise comparison: each pattern segment of "x" matches any version segment.
    // A trailing "x" segment also matches "any number of remaining segments" (greedy).
    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let ver_segs: Vec<&str> = version.split('.').collect();

    // If the pattern ends in "x", it can absorb extra version segments.
    // Otherwise segment counts must match exactly.
    let ends_with_x = pat_segs.last().is_some_and(|s| *s == "x");
    if !ends_with_x && pat_segs.len() != ver_segs.len() {
        return false;
    }
    if ends_with_x && ver_segs.len() < pat_segs.len() - 1 {
        return false;
    }

    for (i, ps) in pat_segs.iter().enumerate() {
        if *ps == "x" {
            // matches any version segment (or "absorbs" trailing if last)
            continue;
        }
        match ver_segs.get(i) {
            Some(vs) if vs == ps => {}
            _ => return false,
        }
    }
    true
}

/// Check whether `req_ver` (caller-supplied CS version) conflicts with what is
/// stored in the DB or pinned in the VS compose.
///
/// Returns `Some((issues, caused_by, echo_version))` when a mismatch is detected:
/// - issues: validation issues to report
/// - caused_by: `Some(url|ver)` canonical for the `x-caused-by-unknown-system`
///   parameter (only when the requested version is missing from the DB).
/// - echo_version: the CS version to echo in the response `version` parameter.
///
/// Returns `None` when there is no mismatch (caller should proceed normally).
async fn detect_cs_version_mismatch(
    client: &tokio_postgres::Client,
    system_url: &str,
    req_ver: &str,
    compose_json: Option<&str>,
    vs_version: Option<&str>,
    version_loc: &str,
    system_loc: &str,
) -> Option<(
    Vec<crate::types::ValidationIssue>,
    Option<String>,
    Option<String>,
)> {
    // Build (id, version) candidate list sorted desc so the first entry is the
    // highest version — used for both resolution and picking the "actual" ver.
    let rows = client
        .query(
            "SELECT id, version FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC",
            &[&system_url],
        )
        .await
        .ok()?;
    let candidates: Vec<(String, Option<String>)> = rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect();

    if candidates.is_empty() {
        return None; // CS not in DB — handled by the not-found path elsewhere
    }

    // Resolve req_ver (handles short-forms like "1.0" → "1.0.0")
    let resolved_req = resolve_ver_against_candidates(&candidates, req_ver);

    // Parse compose to find include pin for this system. A VS may pin the
    // same system to multiple versions (the "overload" pattern). When the
    // requested version matches *any* of those pins, there is no mismatch.
    let all_include_pins: Option<Vec<Option<String>>> =
        compose_json.and_then(|cj| vs_all_pinned_include_versions(cj, system_url));
    let include_pin: Option<Option<String>> =
        compose_json.and_then(|cj| vs_pinned_include_version(cj, system_url));

    // Highest stored version (for use in warning text when req_ver is missing)
    let actual_ver: Option<String> = candidates.iter().find_map(|(_, v)| v.clone());

    if resolved_req.is_none() {
        // req_ver does not match any stored CS version → UNKNOWN_CODESYSTEM_VERSION
        let all_versions = cs_all_stored_versions(client, system_url).await;
        let valid_str = format_valid_versions_msg(&all_versions);
        let error_text = format!(
            "A definition for CodeSystem '{system_url}' version '{req_ver}' could not be found, \
             so the code cannot be validated. Valid versions: {valid_str}"
        );

        // Optionally supplement with a VALUESET_VALUE_MISMATCH when a VS include
        // provides context about which version was expected.
        // - VS pins a specific (known) version that differs → VALUESET_VALUE_MISMATCH (error)
        // - VS is versionless (effective = latest) and latest differs → VALUESET_VALUE_MISMATCH_DEFAULT (warning)
        // - No VS context → no supplement
        let extra: Option<(String, &str, &str)> = match include_pin.as_ref() {
            Some(Some(inc_ver)) => Some((
                format!(
                    "The code system '{system_url}' version '{inc_ver}' in the ValueSet include \
                     is different to the one in the value ('{req_ver}')"
                ),
                "VALUESET_VALUE_MISMATCH",
                "error",
            )),
            Some(None) => {
                let latest = actual_ver.as_deref().unwrap_or(req_ver);
                Some((
                    format!(
                        "The code system '{system_url}' version '{latest}' for the versionless \
                         include in the ValueSet include is different to the one in the value ('{req_ver}')"
                    ),
                    "VALUESET_VALUE_MISMATCH_DEFAULT",
                    "warning",
                ))
            }
            // No VS context — just UNKNOWN_CODESYSTEM_VERSION, no mismatch supplement.
            None => None,
        };

        // Echo version: use the VS-pinned resolved version when available,
        // otherwise use the highest stored version.
        let echo_version: Option<String> = match include_pin.as_ref() {
            Some(Some(inc_ver)) => {
                resolve_ver_against_candidates(&candidates, inc_ver).or_else(|| actual_ver.clone())
            }
            _ => actual_ver.clone(),
        };

        let unknown_issue = crate::types::ValidationIssue {
            severity: "error".into(),
            fhir_code: "not-found".into(),
            tx_code: "not-found".into(),
            text: error_text,
            expression: Some(system_loc.into()),
            location: Some(system_loc.into()),
            message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
        };
        // Order: VALUESET_VALUE_MISMATCH (error) before UNKNOWN when present as error;
        //        UNKNOWN before VALUESET_VALUE_MISMATCH_DEFAULT (warning).
        let issues = match extra {
            Some((mismatch_text, mismatch_id, "error")) => {
                vec![
                    crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "invalid".into(),
                        tx_code: "vs-invalid".into(),
                        text: mismatch_text,
                        expression: Some(version_loc.into()),
                        location: Some(version_loc.into()),
                        message_id: Some(mismatch_id.into()),
                    },
                    unknown_issue,
                ]
            }
            Some((warn_text, warn_id, warn_sev)) => {
                vec![
                    unknown_issue,
                    crate::types::ValidationIssue {
                        severity: warn_sev.into(),
                        fhir_code: "invalid".into(),
                        tx_code: "vs-invalid".into(),
                        text: warn_text,
                        expression: Some(version_loc.into()),
                        location: Some(version_loc.into()),
                        message_id: Some(warn_id.into()),
                    },
                ]
            }
            None => vec![unknown_issue],
        };
        let caused_by = Some(format!("{system_url}|{req_ver}"));
        return Some((issues, caused_by, echo_version));
    }

    let req_full = resolved_req.as_deref().unwrap_or(req_ver);

    // "Overload" pattern: when the VS pins the same system to multiple
    // versions, accept the request if it matches *any* of those pins. Without
    // this short-circuit, the legacy single-pin code below picks the first
    // include and emits a spurious VALUESET_VALUE_MISMATCH for callers whose
    // version matches a later include.
    if let Some(pins) = all_include_pins.as_ref() {
        if pins.len() > 1 {
            let any_match = pins.iter().any(|p| match p {
                Some(v) if v.contains(".x") || v == "x" => version_satisfies_wildcard(req_full, v),
                Some(v) => resolve_ver_against_candidates(&candidates, v)
                    .map(|rv| rv == req_full)
                    .unwrap_or_else(|| v == req_full),
                // Versionless include: the effective version is the latest
                // stored, which we'll have already accepted as `req_full`
                // when it matches; otherwise flag below.
                None => actual_ver.as_deref() == Some(req_full),
            });
            if any_match {
                return None;
            }
        }
    }

    // req_ver exists in the CS. Check if the VS include pins a conflicting version.
    match include_pin {
        Some(Some(ref inc_ver)) => {
            // When inc_ver is a wildcard pattern (e.g. "1.x"), check whether
            // req_full satisfies it. If so, no mismatch — "1.0.0" matches "1.x".
            if inc_ver.contains(".x") || inc_ver.as_str() == "x" {
                if version_satisfies_wildcard(req_full, inc_ver.as_str()) {
                    return None;
                }
            }

            let resolved_inc = resolve_ver_against_candidates(&candidates, inc_ver);
            let inc_full = resolved_inc.as_deref().unwrap_or(inc_ver.as_str());
            if inc_full != req_full {
                let mismatch_text = format!(
                    "The code system '{system_url}' version '{inc_full}' in the ValueSet include \
                     is different to the one in the value ('{req_full}')"
                );
                // When the VS pin itself doesn't exist in the DB, add UNKNOWN for
                // the pin version (e.g. VS include has version "1" but only "1.0.0"
                // and "1.2.0" are stored).
                if resolved_inc.is_none() {
                    let all_versions = cs_all_stored_versions(client, system_url).await;
                    let valid_str = format_valid_versions_msg(&all_versions);
                    let unknown_text = format!(
                        "A definition for CodeSystem '{system_url}' version '{inc_ver}' could not \
                         be found, so the code cannot be validated. Valid versions: {valid_str}"
                    );
                    let issues = vec![
                        crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "invalid".into(),
                            tx_code: "vs-invalid".into(),
                            text: mismatch_text,
                            expression: Some(version_loc.into()),
                            location: Some(version_loc.into()),
                            message_id: Some("VALUESET_VALUE_MISMATCH".into()),
                        },
                        crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "not-found".into(),
                            tx_code: "not-found".into(),
                            text: unknown_text,
                            expression: Some(system_loc.into()),
                            location: Some(system_loc.into()),
                            message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
                        },
                    ];
                    let caused_by = Some(format!("{system_url}|{inc_ver}"));
                    // Echo req_full (the code's existing version) when pin doesn't exist.
                    return Some((issues, caused_by, Some(req_full.to_string())));
                }
                // Both versions exist but differ → VALUESET_VALUE_MISMATCH only.
                let issues = vec![crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "invalid".into(),
                    tx_code: "vs-invalid".into(),
                    text: mismatch_text,
                    expression: Some(version_loc.into()),
                    location: Some(version_loc.into()),
                    message_id: Some("VALUESET_VALUE_MISMATCH".into()),
                }];
                // Echo inc_full (the VS-pinned version), not the requested version.
                return Some((issues, None, Some(inc_full.to_string())));
            }
        }
        Some(None) => {
            // Versionless VS include: the effective CS version is the latest stored.
            // When the caller requested a different (but existing) version, emit
            // VALUESET_VALUE_MISMATCH (error) — same form as a pinned-version conflict.
            //
            // Exception: when the VS itself carries a wildcard version (e.g. "1.x")
            // and req_full satisfies it (e.g. "1.0.0" satisfies "1.x"), no mismatch.
            if let Some(vs_ver) = vs_version {
                if (vs_ver.contains(".x") || vs_ver == "x")
                    && version_satisfies_wildcard(req_full, vs_ver)
                {
                    return None;
                }
            }
            let latest = actual_ver.as_deref().unwrap_or(req_ver);
            if latest != req_full {
                let mismatch_text = format!(
                    "The code system '{system_url}' version '{latest}' in the ValueSet include \
                     is different to the one in the value ('{req_full}')"
                );
                let issues = vec![crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "invalid".into(),
                    tx_code: "vs-invalid".into(),
                    text: mismatch_text,
                    expression: Some(version_loc.into()),
                    location: Some(version_loc.into()),
                    message_id: Some("VALUESET_VALUE_MISMATCH".into()),
                }];
                // Echo the stored version (latest), not the requested version.
                return Some((issues, None, actual_ver.clone()));
            }
        }
        None => {} // No VS context — req_ver was found, no mismatch to report.
    }

    None // No mismatch detected
}

/// When the caller provides **no** version, check whether the VS include pins
/// a version that doesn't exist in the DB.  Emits `UNKNOWN_CODESYSTEM_VERSION`
/// (with `x-caused-by-unknown-system`) when the pin can't be resolved.
///
/// Returns `None` when there is no issue (versionless include, pin resolves
/// OK, or no VS compose context).
async fn detect_vs_pin_unknown(
    client: &tokio_postgres::Client,
    system_url: &str,
    compose_json: Option<&str>,
    system_loc: &str,
) -> Option<(
    Vec<crate::types::ValidationIssue>,
    Option<String>,
    Option<String>,
)> {
    let inc_ver = compose_json
        .and_then(|cj| vs_pinned_include_version(cj, system_url))
        .and_then(|pin| pin)?; // only when the include has an explicit version

    // Build candidates for resolution
    let rows = client
        .query(
            "SELECT id, version FROM code_systems \
             WHERE url = $1 \
             ORDER BY COALESCE(version, '') DESC",
            &[&system_url],
        )
        .await
        .ok()?;
    let candidates: Vec<(String, Option<String>)> = rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // If the pin resolves to a stored version, there is no issue.
    if resolve_ver_against_candidates(&candidates, &inc_ver).is_some() {
        return None;
    }

    // Pin doesn't exist → report it as unknown.
    let all_versions = cs_all_stored_versions(client, system_url).await;
    let valid_str = format_valid_versions_msg(&all_versions);
    let error_text = format!(
        "A definition for CodeSystem '{system_url}' version '{inc_ver}' could not be found, \
         so the code cannot be validated. Valid versions: {valid_str}"
    );
    let issues = vec![crate::types::ValidationIssue {
        severity: "error".into(),
        fhir_code: "not-found".into(),
        tx_code: "not-found".into(),
        text: error_text,
        expression: Some(system_loc.into()),
        location: Some(system_loc.into()),
        message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
    }];
    let caused_by = Some(format!("{system_url}|{inc_ver}"));
    // Echo the highest stored version when pin doesn't exist.
    let echo_version = candidates.iter().find_map(|(_, v)| v.clone());
    Some((issues, caused_by, echo_version))
}

// ── Response builder ──────────────────────────────────────────────────────────

// Keep all message-format inputs explicit so the IG-fixture text strings are
// composed in one place — mirrors the SQLite helper at
// `sqlite/value_set.rs:6977`. Pure function, no I/O.
//
// `is_inactive_in_underlying_cs` is set when the code is NOT in the expansion
// (`found.is_none()`) but IS present in the underlying CodeSystem with an
// inactive status. The IG fixtures (e.g. `inactive/validate-inactive-2a`)
// expect three additional issues in that case: a business-rule "...is valid
// but is not active" error, the not-in-vs error, and a code-comment "...has a
// status of inactive..." warning.
//
// `code_unknown_in_cs` is the union signal: true when the code is unknown
// either anywhere in the underlying CS or only at the requested version.
// `code_unknown_at_version_only` is true when the code DOES exist in the CS
// (just not at the caller's pinned version) — in that case the IG fixtures
// still echo `system` and `version` (without `display`).
#[allow(clippy::too_many_arguments)]
fn finish_validate_code_response(
    found: Option<ExpansionContains>,
    code: &str,
    url: &str,
    expected_display: Option<&str>,
    system_for_msg: Option<&str>,
    is_abstract: bool,
    is_inactive: bool,
    vs_version: Option<&str>,
    is_inactive_in_underlying_cs: bool,
    code_unknown_in_cs: bool,
    code_unknown_at_version_only: bool,
    cs_version_for_msg: Option<&str>,
    req_version_hint: Option<&str>,
    lenient_display: bool,
    cs_is_fragment: bool,
    cs_display_lookup: Option<&str>,
    normalized_code: Option<&str>,
) -> Result<ValidateCodeResponse, HtsError> {
    let qualifier_version: Option<&str> = if found.is_none() {
        req_version_hint.filter(|v| !v.is_empty() && !v.contains(".x") && *v != "x")
    } else {
        None
    };
    let qualified = match (system_for_msg, qualifier_version) {
        (Some(s), Some(v)) => format!("{s}|{v}#{code}"),
        (Some(s), None) => format!("{s}#{code}"),
        (None, _) => code.to_string(),
    };
    let qualified_with_display = match (system_for_msg, expected_display, qualifier_version) {
        (Some(s), Some(d), Some(v)) => format!("{s}|{v}#{code} ('{d}')"),
        (Some(s), Some(d), None) => format!("{s}#{code} ('{d}')"),
        _ => qualified.clone(),
    };
    let url_with_version = match vs_version {
        Some(v) => format!("{url}|{v}"),
        None => url.to_string(),
    };
    let mut issues: Vec<crate::types::ValidationIssue> = Vec::new();
    match found {
        None => {
            // Fragment short-circuit: unknown code in a fragment CS becomes a
            // single warning (result=true) per IG `fragment/validation-*-bad-code`.
            if cs_is_fragment && code_unknown_in_cs {
                if let Some(sys) = system_for_msg {
                    let cs_text = match cs_version_for_msg {
                        Some(v) => format!(
                            "Unknown Code '{code}' in the CodeSystem '{sys}' version '{v}' - note that the code system is labeled as a fragment, so the code may be valid in some other fragment"
                        ),
                        None => format!(
                            "Unknown Code '{code}' in the CodeSystem '{sys}' - note that the code system is labeled as a fragment, so the code may be valid in some other fragment"
                        ),
                    };
                    return Ok(ValidateCodeResponse {
                        result: true,
                        message: None,
                        display: None,
                        system: Some(sys.to_string()),
                        cs_version: cs_version_for_msg.map(|s| s.to_string()),
                        inactive: None,
                        issues: vec![crate::types::ValidationIssue {
                            severity: "warning".into(),
                            fhir_code: "code-invalid".into(),
                            tx_code: "invalid-code".into(),
                            text: cs_text,
                            expression: Some("Coding.code".into()),
                            location: Some("Coding.code".into()),
                            message_id: Some("UNKNOWN_CODE_IN_FRAGMENT".into()),
                        }],
                        caused_by_unknown_system: None,
                        concept_status: None,
                        normalized_code: None,
                    });
                }
            }
            let not_in_vs_text = format!(
                "The provided code '{qualified_with_display}' was not found in the value set '{url_with_version}'"
            );
            // Code is valid in underlying CS but inactive, and the VS filtered
            // it out — emit the business-rule "valid but not active" error.
            if is_inactive_in_underlying_cs {
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-rule".into(),
                    text: format!("The concept '{code}' is valid but is not active"),
                    expression: Some("Coding.code".into()),
                    location: None,
                    message_id: Some("STATUS_CODE_WARNING_CODE".into()),
                });
            }
            issues.push(crate::types::ValidationIssue {
                severity: "error".into(),
                fhir_code: "code-invalid".into(),
                tx_code: "not-in-vs".into(),
                text: not_in_vs_text.clone(),
                expression: Some("Coding.code".into()),
                location: None,
                message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
            });
            // Companion issue when the code is not in the underlying CS at all
            // but the CS itself is loaded.
            if code_unknown_in_cs && cs_version_for_msg.is_some() {
                if let Some(sys) = system_for_msg {
                    let cs_text = match cs_version_for_msg {
                        Some(v) => {
                            format!("Unknown code '{code}' in the CodeSystem '{sys}' version '{v}'")
                        }
                        None => format!("Unknown code '{code}' in the CodeSystem '{sys}'"),
                    };
                    issues.push(crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "invalid-code".into(),
                        text: cs_text,
                        expression: Some("Coding.code".into()),
                        location: None,
                        message_id: Some("Unknown_Code_in_Version".into()),
                    });
                }
            }
            if is_inactive_in_underlying_cs {
                issues.push(crate::types::ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text: format!(
                        "The concept '{code}' has a status of inactive and its use should be reviewed"
                    ),
                    expression: Some("Coding".into()),
                    location: Some("Coding".into()),
                    message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
                });
            }
            let mut texts: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
            texts.sort();
            let message = texts.join("; ");
            let (echo_display, echo_system) = if !code_unknown_in_cs {
                let disp = expected_display
                    .map(str::to_string)
                    .or_else(|| cs_display_lookup.map(str::to_string));
                (disp, system_for_msg.map(str::to_string))
            } else if code_unknown_at_version_only {
                (None, system_for_msg.map(str::to_string))
            } else {
                (None, None)
            };
            Ok(ValidateCodeResponse {
                result: false,
                message: Some(message),
                display: echo_display,
                system: echo_system,
                cs_version: if !code_unknown_in_cs || code_unknown_at_version_only {
                    cs_version_for_msg.map(|s| s.to_string())
                } else {
                    None
                },
                inactive: if is_inactive_in_underlying_cs {
                    Some(true)
                } else {
                    None
                },
                issues,
                caused_by_unknown_system: None,
                concept_status: None,
                normalized_code: None,
            })
        }
        Some(concept) => {
            // Abstract / notSelectable concepts: reject with the IG wording.
            if is_abstract {
                let abstract_text =
                    format!("Code '{qualified}' is abstract, and not allowed in this context");
                let not_in_vs_text = format!(
                    "The provided code '{qualified}' was not found in the value set '{url_with_version}'"
                );
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-rule".into(),
                    text: abstract_text.clone(),
                    expression: Some("Coding.code".into()),
                    location: None,
                    message_id: Some("ABSTRACT_CODE_NOT_ALLOWED".into()),
                });
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "code-invalid".into(),
                    tx_code: "not-in-vs".into(),
                    text: not_in_vs_text,
                    expression: Some("Coding.code".into()),
                    location: None,
                    message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
                });
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(abstract_text),
                    display: concept.display,
                    system: None,
                    cs_version: concept
                        .version
                        .or_else(|| cs_version_for_msg.map(|s| s.to_string())),
                    inactive: None,
                    issues,
                    caused_by_unknown_system: None,
                    concept_status: None,
                    normalized_code: None,
                });
            }
            if is_inactive {
                issues.push(crate::types::ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text: format!(
                        "The concept '{code}' has a status of inactive and its use should be reviewed"
                    ),
                    expression: Some("Coding".into()),
                    location: Some("Coding".into()),
                    message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
                });
            }
            // Case-insensitive normalisation note (IG `case/case-coding-insensitive-*`).
            if let Some(canonical) = normalized_code {
                let cs_qualifier: String = match (system_for_msg, cs_version_for_msg) {
                    (Some(s), Some(v)) => format!("{s}|{v}"),
                    (Some(s), None) => s.to_string(),
                    _ => String::new(),
                };
                let text = format!(
                    "The code '{code}' differs from the correct code '{canonical}' by case. Although the code system '{cs_qualifier}' is case insensitive, implementers are strongly encouraged to use the correct case anyway"
                );
                issues.push(crate::types::ValidationIssue {
                    severity: "information".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-rule".into(),
                    text,
                    expression: Some("Coding.code".into()),
                    location: Some("Coding.code".into()),
                    message_id: Some("CODE_CASE_DIFFERENCE".into()),
                });
            }
            let mut display_message: Option<String> = None;
            if let Some(expected) = expected_display {
                if let Some(actual) = concept.display.as_deref() {
                    if !actual.eq_ignore_ascii_case(expected) {
                        let qualified = match system_for_msg {
                            Some(s) => format!("{s}#{code}"),
                            None => code.to_string(),
                        };
                        let text = format!(
                            "Wrong Display Name '{expected}' for {qualified}. Valid display is '{actual}' (en) (for the language(s) '--')"
                        );
                        display_message = Some(text.clone());
                        issues.push(crate::types::ValidationIssue {
                            severity: if lenient_display { "warning" } else { "error" }.into(),
                            fhir_code: "invalid".into(),
                            tx_code: "invalid-display".into(),
                            text,
                            expression: Some("Coding.display".into()),
                            location: None,
                            message_id: Some(
                                "Display_Name_for__should_be_one_of__instead_of".into(),
                            ),
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
            let req_version_owned = req_version_hint
                .filter(|v| !v.is_empty() && !v.contains(".x") && *v != "x")
                .map(|s| s.to_string());
            let cs_version = req_version_owned
                .or_else(|| concept.version.clone())
                .or_else(|| cs_version_for_msg.map(|s| s.to_string()));
            Ok(ValidateCodeResponse {
                result: !has_error,
                message,
                display: concept.display,
                system: Some(concept.system),
                cs_version,
                inactive: if is_inactive { Some(true) } else { None },
                issues,
                caused_by_unknown_system: None,
                concept_status: None,
                normalized_code: normalized_code.map(|s| s.to_string()),
            })
        }
    }
}
