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

use super::code_system::build_synthetic_resource;
use super::{HierarchyEdges, PostgresTerminologyBackend, hierarchy_cache};
use std::sync::Arc;

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

        // The `max_expansion_size` cap is intended as a guardrail against
        // unbounded materialisation: when the caller has already bounded the
        // request via `count`, SQLite's expand skips the cap (see
        // crates/hts/src/backends/sqlite/value_set.rs — `req.count.is_none()`
        // gates the same TooCostly branch). PG used to enforce the cap
        // regardless of `count`, which made bench scenarios EX02/EX04/EX07
        // 422 even though the caller only wanted a 10-200 code page. Gate the
        // check the same way SQLite does so a paginating client gets its page.
        let cap_enforced = req.count.is_none();

        // ── Pool-free warm-hit fast path ─────────────────────────────────
        // Check the in-memory inline_compose_cache before acquiring a pool
        // connection. The bench's warm path (90%+ of requests once caches
        // are populated) does not need DB access at all; holding a pool
        // connection on warm-hit was the bottleneck causing EX01 to stay
        // at 123 RPS in iter 4+5 even with full cache coverage.
        let warm_cache_key: Option<u64> = if let Some(url) = req.url.as_deref() {
            // URL-based: key is the same as the URL cold-miss path. Skip when
            // hierarchical (those bypass the cache).
            if req.hierarchical != Some(true) {
                Some(build_url_cache_key(
                    url,
                    req.value_set_version.as_deref(),
                    &req.force_system_versions,
                    &req.system_version_defaults,
                    &req.default_value_set_versions,
                ))
            } else {
                None
            }
        } else if let Some(vs) = req.value_set.as_ref() {
            // Inline VS: key is the same as the inline cold-miss path.
            if req.hierarchical != Some(true) && req.tx_resources.is_empty() {
                let compose_str_opt = vs.get("compose").map(|c| c.to_string());
                let contained_vec: Vec<serde_json::Value> = vs
                    .get("contained")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default();
                compose_str_opt.map(|compose_str| {
                    build_inline_cache_key(
                        &compose_str,
                        &req.force_system_versions,
                        &req.system_version_defaults,
                        &req.default_value_set_versions,
                        &contained_vec,
                    )
                })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(k) = warm_cache_key {
            if let Some(arc) = cache_get_initialized(&self.inline_compose_cache, k) {
                return Ok(serve_from_cached(&arc, &req));
            }
        }

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let mut compose_is_enumerated = false;
        let all_codes: Vec<ExpansionContains> = if let Some(url) = req.url.as_deref() {
            // ── `?fhir_vs` implicit ValueSet short-circuit (FHIR R4 §4.8.7) ──
            // A CodeSystem URL with `?fhir_vs` or `?fhir_vs=isa/<code>` is an
            // implicit ValueSet description. Resolving via the value_sets
            // table never matches (no stored VS has that URL); the legacy
            // `find_cs_for_implicit_vs` fallback only catches the
            // `?fhir_vs`-bare form when a CS's `.valueSet` field happens to
            // match, and never `?fhir_vs=isa/X`. Translate either form to a
            // synthetic compose so `compute_expansion` handles it via the
            // normal SNOMED hierarchy filter (mirrors sqlite's
            // `implicit_short_circuit` at backends/sqlite/value_set.rs).
            if let Some((cs_url, pattern)) = parse_fhir_vs_url(url) {
                if resolve_system_id_pg(&client, &cs_url).await?.is_none() {
                    return Err(HtsError::NotFound(format!(
                        "A definition for the value Set '{url}' could not be found"
                    )));
                }
                // Single-flight cache check — keyed by URL + version pins. EX01
                // hits the same `?fhir_vs=isa/<code>` per pool entry hundreds
                // of times during a 30s vu=50 run; cold-miss is a recursive
                // CTE on SNOMED. The OnceCell collapses simultaneous misses
                // onto one compute.
                let cache_key = build_url_cache_key(
                    url,
                    req.value_set_version.as_deref(),
                    &req.force_system_versions,
                    &req.system_version_defaults,
                    &req.default_value_set_versions,
                );
                let cache_eligible = req.hierarchical != Some(true);
                let cell = if cache_eligible {
                    cache_cell(&self.inline_compose_cache, cache_key)
                } else {
                    None
                };
                if let Some(cell) = cell.as_ref() {
                    let force_sv = &req.force_system_versions;
                    let sysv = &req.system_version_defaults;
                    let defvs = &req.default_value_set_versions;
                    let max_size = req.max_expansion_size;
                    let client_ref = &client;
                    let cs_url_ref = &cs_url;
                    let pattern_ref = &pattern;
                    let arc = cell
                        .get_or_try_init(|| async move {
                            let compose_val = build_implicit_compose_value(cs_url_ref, pattern_ref);
                            let compose_str = compose_val.to_string();
                            let codes = compute_expansion(
                                client_ref,
                                Some(&compose_str),
                                force_sv,
                                sysv,
                                defvs,
                            )
                            .await?;
                            enforce_cap(&codes, cap_enforced, max_size, /*implicit=*/ true)?;
                            Ok::<_, HtsError>(std::sync::Arc::new(codes))
                        })
                        .await?;
                    return Ok(serve_from_cached(arc, &req));
                }
                // Cache at capacity OR cache_eligible=false (hierarchical):
                // unprotected compute, no caching.
                let compose_val = build_implicit_compose_value(&cs_url, &pattern);
                let compose_str = compose_val.to_string();
                let codes = compute_expansion(
                    &client,
                    Some(&compose_str),
                    &req.force_system_versions,
                    &req.system_version_defaults,
                    &req.default_value_set_versions,
                ).await?;
                enforce_cap(&codes, cap_enforced, req.max_expansion_size, /*implicit=*/true)?;
                codes
            } else {
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
                    compose_is_enumerated = compose_is_enumerated_json(compose_json.as_deref());

                    // ── Iter 7c: pure-extensional fast path ────────────────────
                    // For VSAC ValueSets (compose.include[].concept[] lists with
                    // embedded display, no filter[], no valueSet[] refs), the
                    // expansion is in the JSON. Skip compute_expansion's full
                    // materialisation and the populate_cache UNNEST transaction
                    // — slice the requested page in memory. Mirrors SQLite's
                    // `compose_page_fast` (backends/sqlite/value_set.rs:5285).
                    //
                    // Only fires when the caller bounded the request via count,
                    // matches SQLite's call site, and skips when version pins
                    // would change the result (those fall through to the
                    // existing OnceCell + compute_expansion path below).
                    if let Some(count) = req.count.filter(|&c| c > 0) {
                        if req.force_system_versions.is_empty()
                            && req.system_version_defaults.is_empty()
                            && req.default_value_set_versions.is_empty()
                        {
                            let page_offset = req.offset.unwrap_or(0) as usize;
                            if let Some((page, total)) = compose_page_fast_pg(
                                &client,
                                compose_json.as_deref(),
                                page_offset,
                                count as usize,
                                req.filter.as_deref(),
                            )
                            .await?
                            {
                                return Ok(ExpandResponse {
                                    total: Some(total),
                                    offset: req.offset,
                                    contains: page,
                                    warnings: vec![],
                                });
                            }
                        }
                    }

                    // Bypass the value_set_expansions cache when the compose
                    // describes a multi-version overload — its PRIMARY KEY
                    // (vs_id, system_url, code) silently dedupes the second
                    // version's row, dropping half the expansion. Recomputing
                    // is cheap for these small overload ValueSets.
                    let multi_version =
                        compose_has_multi_version_pins(compose_json.as_deref());
                    // Also bypass when a default-valueset-version pin is in
                    // effect — the cached entry reflects unpinned resolution
                    // of any nested `compose.include[].valueSet[]` refs and
                    // would diverge from the pinned version's content.
                    let has_vs_pin = !req.default_value_set_versions.is_empty();

                    // Single-flight in-memory cache (EX04 hot path — VSAC
                    // URLs hit hundreds of times per 30s run). The OnceCell
                    // collapses simultaneous misses onto one compute +
                    // populate_cache, then warm-hit slices the cached Vec.
                    let cache_key = build_url_cache_key(
                        url,
                        req.value_set_version.as_deref(),
                        &req.force_system_versions,
                        &req.system_version_defaults,
                        &req.default_value_set_versions,
                    );
                    let cache_eligible = req.hierarchical != Some(true)
                        && !multi_version
                        && !has_vs_pin;
                    let cell = if cache_eligible {
                        cache_cell(&self.inline_compose_cache, cache_key)
                    } else {
                        None
                    };
                    if let Some(cell) = cell.as_ref() {
                        // Pre-borrow request fields into locals so the
                        // `async move` closure captures the borrows (not the
                        // owned `req`). `req` stays usable for the
                        // `serve_from_cached(arc, &req)` call below.
                        let force_sv = &req.force_system_versions;
                        let sysv = &req.system_version_defaults;
                        let defvs = &req.default_value_set_versions;
                        let max_size = req.max_expansion_size;
                        let client_ref = &mut client;
                        let vs_id_ref = vs_id.as_str();
                        let compose_json_ref = compose_json.as_deref();
                        let arc = cell
                            .get_or_try_init(|| async move {
                                let cached_rows = fetch_cache(&*client_ref, vs_id_ref).await?;
                                let codes = if cached_rows.is_empty() {
                                    let computed = compute_expansion(
                                        &*client_ref,
                                        compose_json_ref,
                                        force_sv,
                                        sysv,
                                        defvs,
                                    )
                                    .await?;
                                    enforce_cap(
                                        &computed,
                                        cap_enforced,
                                        max_size,
                                        /*implicit=*/ false,
                                    )?;
                                    populate_cache(client_ref, vs_id_ref, &computed).await?;
                                    computed
                                } else {
                                    cached_rows
                                };
                                Ok::<_, HtsError>(std::sync::Arc::new(codes))
                            })
                            .await?;
                        return Ok(serve_from_cached(arc, &req));
                    }

                    // Non-cacheable branch (multi_version / has_vs_pin /
                    // hierarchical) OR cache at capacity: unprotected compute.
                    let cached = if multi_version || has_vs_pin {
                        Vec::new()
                    } else {
                        fetch_cache(&client, &vs_id).await?
                    };
                    if cached.is_empty() {
                        let codes = compute_expansion(
                            &client,
                            compose_json.as_deref(),
                            &req.force_system_versions,
                            &req.system_version_defaults,
                            &req.default_value_set_versions,
                        ).await?;
                        enforce_cap(
                            &codes,
                            cap_enforced,
                            req.max_expansion_size,
                            /*implicit=*/ false,
                        )?;
                        if !multi_version && !has_vs_pin {
                            populate_cache(&mut client, &vs_id, &codes).await?;
                        }
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
                    let codes = compute_expansion(
                        &client,
                        Some(&compose),
                        &req.force_system_versions,
                        &req.system_version_defaults,
                        &req.default_value_set_versions,
                    ).await?;
                    if cap_enforced {
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
                    }
                    codes
                }
                Err(e) => return Err(e),
            }
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
            // Pull the inline VS's `contained[]` so `compose.include[].
            // valueSet[]: ["#fragment"]` references can resolve to inline
            // contained ValueSets — drives the IG `simple/simple-expand-
            // contained` and `validation/validate-contained-good` fixtures.
            let contained_vec: Vec<serde_json::Value> = vs
                .get("contained")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            if let Some(compose_val) = compose {
                compose_is_enumerated = compose_is_enumerated_value(compose_val);
                let compose_str = compose_val.to_string();

                // ── Iter 7c: pure-extensional fast path (inline VS) ────────
                // Same shape as the URL-resolved branch: bypass
                // compute_expansion's full materialisation when the inline
                // compose body is purely extensional and the caller is
                // paginating. Skipped when tx_resources are present (those
                // can reference resources that change the semantics).
                if let Some(count) = req.count.filter(|&c| c > 0) {
                    if req.force_system_versions.is_empty()
                        && req.system_version_defaults.is_empty()
                        && req.default_value_set_versions.is_empty()
                        && req.tx_resources.is_empty()
                        && contained_vec.is_empty()
                    {
                        let page_offset = req.offset.unwrap_or(0) as usize;
                        if let Some((page, total)) = compose_page_fast_pg(
                            &client,
                            Some(&compose_str),
                            page_offset,
                            count as usize,
                            req.filter.as_deref(),
                        )
                        .await?
                        {
                            return Ok(ExpandResponse {
                                total: Some(total),
                                offset: req.offset,
                                contains: page,
                                warnings: vec![],
                            });
                        }
                    }
                }

                // Single-flight inline-compose cache — EX02/05/06/07/08 POST
                // identical inline composes per pool entry. The OnceCell
                // collapses simultaneous misses onto one compute; warm-hit
                // skips the recursive-CTE / pg_filter_is_a expansion entirely.
                let cache_key = build_inline_cache_key(
                    &compose_str,
                    &req.force_system_versions,
                    &req.system_version_defaults,
                    &req.default_value_set_versions,
                    &contained_vec,
                );
                let cache_eligible = req.hierarchical != Some(true)
                    && req.tx_resources.is_empty();
                let cell = if cache_eligible {
                    cache_cell(&self.inline_compose_cache, cache_key)
                } else {
                    None
                };
                if let Some(cell) = cell.as_ref() {
                    let force_sv = &req.force_system_versions;
                    let sysv = &req.system_version_defaults;
                    let defvs = &req.default_value_set_versions;
                    let max_size = req.max_expansion_size;
                    let client_ref = &client;
                    let compose_str_ref = compose_str.as_str();
                    let contained_ref = contained_vec.as_slice();
                    let arc = cell
                        .get_or_try_init(|| async move {
                            let codes = compute_expansion_with_contained(
                                client_ref,
                                Some(compose_str_ref),
                                force_sv,
                                sysv,
                                defvs,
                                contained_ref,
                            )
                            .await?;
                            enforce_cap(
                                &codes,
                                cap_enforced,
                                max_size,
                                /*implicit=*/ false,
                            )?;
                            Ok::<_, HtsError>(std::sync::Arc::new(codes))
                        })
                        .await?;
                    return Ok(serve_from_cached(arc, &req));
                }
                // Non-cacheable branch (hierarchical / tx_resources) or
                // cache at capacity: unprotected compute, no caching.
                let codes = compute_expansion_with_contained(
                    &client,
                    Some(&compose_str),
                    &req.force_system_versions,
                    &req.system_version_defaults,
                    &req.default_value_set_versions,
                    &contained_vec,
                ).await?;
                enforce_cap(
                    &codes,
                    cap_enforced,
                    req.max_expansion_size,
                    /*implicit=*/ false,
                )?;
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

        // Suppress tree-building when the compose is fully enumerated — a
        // hand-curated explicit `concept[]` list shouldn't have its CS
        // hierarchy re-imposed even when hierarchical/excludeNested=false
        // requested it. The IG `parameters/parameters-expand-enum-*`
        // fixtures rely on this. Mirrors sqlite/value_set.rs:1144.
        if req.hierarchical == Some(true) && !compose_is_enumerated {
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
                    // Bypass the value_set_expansions cache when the compose
                    // describes a multi-version overload (see expand path for
                    // the same rationale).
                    let multi_version =
                        compose_has_multi_version_pins(compose_json.as_deref());
                    // Also bypass when a default-valueset-version pin is in
                    // effect — the cached expansion reflects the unpinned
                    // resolution of any nested `compose.include[].valueSet[]`
                    // refs (latest version), which would diverge from the
                    // pinned version's content. Mirrors sqlite/value_set.rs:1318-1324.
                    let has_vs_pin = !req.default_value_set_versions.is_empty();
                    let cached = if multi_version || has_vs_pin {
                        Vec::new()
                    } else {
                        fetch_cache(&client, &vs_id).await?
                    };
                    let codes = if cached.is_empty() {
                        // ValidateCodeRequest doesn't carry the
                        // force-system-version / system-version pins
                        // (those are $expand-only request params), so
                        // pass empty maps for those. The IG-level
                        // interaction between validate-code and
                        // force-system-version is handled separately via
                        // the version-mismatch detector below.
                        //
                        // It DOES carry default_value_set_versions, which
                        // controls how `compose.include[].valueSet[]` refs
                        // are version-pinned during indirect resolution
                        // (Cluster C). Without threading this, the
                        // `valueset-version/coding-indirect-zero-pinned`
                        // fixture regressed when indirect refs started
                        // resolving — they'd always pick the latest VS
                        // instead of honouring the request-side pin.
                        // validate-code is allowed to expand against a VS
                        // whose compose.include pins a CS-version that
                        // doesn't resolve — the validate-code response path
                        // emits its own UNKNOWN_CODESYSTEM_VERSION (no `_EXP`
                        // suffix) issue. Convert the
                        // `__UNKNOWN_CS_VERSION_EXP__` sentinel raised by
                        // compute_expansion into an empty include contribution
                        // here so the sentinel only escapes through the
                        // `$expand` handler (which renders the 4xx
                        // OperationOutcome with UNKNOWN_CODESYSTEM_VERSION_EXP
                        // the IG version/vs-expand-v-wb fixtures expect).
                        // Mirrors sqlite/value_set.rs:1318-1357.
                        let empty: HashMap<String, String> = HashMap::new();
                        let codes = match compute_expansion(
                            &client,
                            compose_json.as_deref(),
                            &empty,
                            &empty,
                            &req.default_value_set_versions,
                        )
                        .await
                        {
                            Ok(c) => c,
                            Err(HtsError::NotFound(msg))
                                if msg.starts_with("__UNKNOWN_CS_VERSION_EXP__:") =>
                            {
                                Vec::new()
                            }
                            Err(e) => return Err(e),
                        };
                        if !multi_version && !has_vs_pin {
                            populate_cache(&mut client, &vs_id, &codes).await?;
                        }
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
                            // No explicit VS, no `?fhir_vs` implicit form, no
                            // CodeSystem.valueSet link — the canonical is truly
                            // unresolvable. Bubble as `HtsError::NotFound` so
                            // the handler emits a top-level OperationOutcome
                            // (4xx) per the IG `version/*-vsbb-*` fixtures, not
                            // a `Parameters { result: false }` wrapper.
                            return Err(HtsError::NotFound(format!(
                                "A definition for the value Set '{url}' could not be found"
                            )));
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

        // inferSystem ambiguity: when the caller did not supply a system
        // and the bare code matches in two or more distinct CodeSystems
        // within the VS expansion, the system URI cannot be inferred. The
        // IG `errors/errors-combination-bad` fixture expects two issues:
        // not-in-vs + cannot-infer ("multiple matches"). Mirrors
        // sqlite/value_set.rs:1569-1644.
        if req.system.is_none() && !candidates.is_empty() {
            let distinct_systems: std::collections::BTreeSet<String> = candidates
                .iter()
                .map(|c| c.system.clone())
                .collect();
            if distinct_systems.len() >= 2 {
                let systems_list: Vec<String> = distinct_systems.into_iter().collect();
                let vs_v = lookup_value_set_version(&client, &url).await;
                let vs_canonical = match vs_v.as_deref() {
                    Some(v) if !v.is_empty() => format!("{url}|{v}"),
                    _ => url.clone(),
                };
                let cannot_infer_text = format!(
                    "The System URI could not be determined for the code '{}' in the ValueSet '{}': value set expansion has multiple matches: [{}]",
                    req.code, vs_canonical, systems_list.join(", ")
                );
                let not_in_vs_text = format!(
                    "The provided code '#{}' was not found in the value set '{}'",
                    req.code, vs_canonical
                );
                let issues = vec![
                    crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "not-in-vs".into(),
                        text: not_in_vs_text.clone(),
                        expression: Some("code".into()),
                        location: Some("code".into()),
                        message_id: Some(
                            "None_of_the_provided_codes_are_in_the_value_set_one".into(),
                        ),
                    },
                    crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "not-found".into(),
                        tx_code: "cannot-infer".into(),
                        text: cannot_infer_text.clone(),
                        expression: Some("code".into()),
                        location: Some("code".into()),
                        message_id: Some(
                            "Unable_to_resolve_system__value_set_has_multiple_matches".into(),
                        ),
                    },
                ];
                let mut texts: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
                texts.sort_unstable();
                let message = texts.join("; ");
                return Ok(crate::types::ValidateCodeResponse {
                    result: false,
                    message: Some(message),
                    display: None,
                    system: None,
                    cs_version: None,
                    inactive: None,
                    issues,
                    caused_by_unknown_system: None,
                    concept_status: None,
                    normalized_code: None,
                });
            }
        }

        let found: Option<ExpansionContains> = if candidates.is_empty() {
            None
        } else if let Some(req_v) = req_ver_exact {
            // (1) Explicit version pin. When a candidate exists at the
            // requested version use it directly.
            //
            // When no candidate matches the requested version we have to
            // discriminate between two scenarios:
            //   (a) Overload pattern (multiple candidates from different
            //       versions, the requested version simply lacks the code):
            //       returning None lets the not-in-VS + Unknown_Code_in_Version
            //       diagnostics fire — what `validate-bad-v1code4` /
            //       `validate-bad-v2code3` expect.
            //   (b) Single-include / single-candidate case where the
            //       requested version genuinely doesn't exist as a stored
            //       CS row (the failure is UNKNOWN_CODESYSTEM_VERSION, not
            //       Unknown_Code_in_Version): the IG `code-vbb-vs10`,
            //       `simple-code-bad-version1`, etc. expect to still echo
            //       the lone candidate's display so the consumer can see
            //       which code's metadata is being shown — fall back.
            //
            // Mirrors `sqlite/value_set.rs:1644-1702`.
            let exact_clone = candidates
                .iter()
                .find(|c| c.version.as_deref() == Some(req_v))
                .map(|c| (*c).clone());
            if let Some(c) = exact_clone {
                Some(c)
            } else if candidates.len() == 1 {
                let single = candidates.into_iter().next().cloned();
                let code_at_req = if let Some(c) = single.as_ref() {
                    is_code_in_cs_at_version(&client, &c.system, req_v, &c.code).await
                } else {
                    false
                };
                let req_version_exists = if let Some(c) = single.as_ref() {
                    cs_version_exists(&client, &c.system, req_v).await
                } else {
                    false
                };
                if code_at_req || !req_version_exists {
                    single
                } else {
                    None
                }
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

        // When compose.inactive=false the VS excludes inactive concepts.
        // The expansion may have been computed without applying this filter
        // (the cache path doesn't honour it), so apply it here: when the
        // matched concept is inactive in the underlying CS, treat it as
        // not-found. The IG `inactive/validate-inactive-2a` fixture relies
        // on this — code is in CS but inactive, VS pins inactive=false, so
        // validate-code returns result=false plus the STATUS_CODE_WARNING_CODE
        // issue. Mirrors sqlite/value_set.rs:1808-1822.
        let compose_inactive_false = compose_json_for_version
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("inactive").and_then(|b| b.as_bool()))
            == Some(false);
        let found = if compose_inactive_false {
            match found {
                Some(c) => {
                    if is_concept_inactive(&client, &c.system, &c.code).await {
                        None
                    } else {
                        Some(c)
                    }
                }
                None => None,
            }
        } else {
            found
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
        //
        // EXCEPTION: when the code is unknown in the CS at any version
        // (code_unknown_in_cs_anywhere=true), the caller's version pin
        // is meaningless — the IG `overload/validate-bad-unknown`
        // fixture expects the response to echo the LATEST stored CS
        // version rather than the (irrelevant) requested one. Override
        // by going straight to cs_version_for_msg in that case.
        let cs_version: Option<String> = match system_for_msg.as_deref() {
            Some(s) => {
                if code_unknown_in_cs_anywhere {
                    cs_version_for_msg(&client, s).await
                } else {
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
            }
            None => None,
        };
        let vs_version_owned = lookup_value_set_version(&client, &url).await;
        let cs_is_fragment = match system_for_msg.as_deref() {
            Some(s) => cs_content_for_url(&client, s).await.as_deref() == Some("fragment"),
            None => false,
        };
        // Echo display lookup: when the caller didn't provide a display but
        // the code lives in the underlying CS, look it up from `concepts` so
        // finish_validate_code_response can emit the `display` Parameters
        // entry. Drives IG `regex-bad/validate-regex-bad-2` (code in CS but
        // not in VS, expected response echoes the CS display).
        let cs_display_lookup: Option<String> = if !code_unknown_in_cs {
            match system_for_msg.as_deref() {
                Some(s) => pg_lookup_cs_display(
                    &client,
                    s,
                    cs_version.as_deref(),
                    &req.code,
                )
                .await,
                None => None,
            }
        } else {
            None
        };

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
            cs_display_lookup.as_deref(),
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
            "SELECT system_url, code, display, version
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
            version: row.get(3),
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

/// Returns true when every `compose.include[]` is a curated explicit
/// `concept[]` enumeration (no `filter[]`, no `valueSet[]` ref). The IG
/// `parameters/parameters-expand-enum-*` fixtures expect this shape to
/// stay flat in the response even when `excludeNested=false` (or
/// `hierarchical=true`) is requested — re-imposing the underlying CS
/// hierarchy on a hand-curated list contradicts the curator's intent.
///
/// Mirrors the detection at sqlite/value_set.rs:520-539.
fn compose_is_enumerated_value(compose: &serde_json::Value) -> bool {
    match compose.get("include").and_then(|v| v.as_array()) {
        Some(includes) if !includes.is_empty() => includes.iter().all(|inc| {
            let has_concept = inc
                .get("concept")
                .and_then(|c| c.as_array())
                .is_some_and(|a| !a.is_empty());
            let no_filter = inc
                .get("filter")
                .and_then(|f| f.as_array())
                .map(|a| a.is_empty())
                .unwrap_or(true);
            let no_vs_ref = inc
                .get("valueSet")
                .and_then(|v| v.as_array())
                .map(|a| a.is_empty())
                .unwrap_or(true);
            has_concept && no_filter && no_vs_ref
        }),
        _ => false,
    }
}

/// Same as [`compose_is_enumerated_value`] but accepts the compose as a
/// serialised JSON string (the URL-path branch carries it in that form).
fn compose_is_enumerated_json(compose_json: Option<&str>) -> bool {
    let Some(s) = compose_json else { return false };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
        return false;
    };
    compose_is_enumerated_value(&v)
}

/// Returns true when `compose_json` describes the "overload" pattern: at
/// least one `system` URL appearing in `include[]` (or `exclude[]`) at
/// multiple distinct `version` values. Used to bypass the
/// `value_set_expansions` cache for those ValueSets — its PRIMARY KEY does
/// not include `version`, so caching would silently dedupe `(system, code)`
/// pairs that legitimately differ across versions.
///
/// Mirrors `sqlite/value_set.rs:compose_has_multi_version_pins`.
fn compose_has_multi_version_pins(compose_json: Option<&str>) -> bool {
    let cj = match compose_json {
        Some(s) => s,
        None => return false,
    };
    let compose: serde_json::Value = match serde_json::from_str(cj) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut by_system: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for key in ["include", "exclude"] {
        if let Some(arr) = compose.get(key).and_then(|v| v.as_array()) {
            for inc in arr {
                let sys = match inc.get("system").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let ver = inc
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_default();
                by_system.entry(sys).or_default().insert(ver);
            }
        }
    }
    by_system.values().any(|s| s.len() > 1)
}

/// Compute an expansion from the raw `compose_json`. Entry point used by
/// `expand` and `validate_code` callers — sets up the recursion context
/// (depth 0, empty `visited` set).
async fn compute_expansion(
    client: &tokio_postgres::Client,
    compose_json: Option<&str>,
    force_system_versions: &HashMap<String, String>,
    system_version_defaults: &HashMap<String, String>,
    default_value_set_versions: &HashMap<String, String>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    compute_expansion_with_contained(
        client,
        compose_json,
        force_system_versions,
        system_version_defaults,
        default_value_set_versions,
        &[],
    )
    .await
}

/// Like [`compute_expansion`] but also accepts the inline VS body's
/// `contained[]` array so `compose.include[].valueSet[]: ["#fragment"]`
/// references resolve to inline contained ValueSets. Used by callers that
/// have the full inline VS body in hand (the `expand` inline-VS branch and
/// the `validate_code` inline-VS branch when forwarding to the inline
/// validator). Mirrors the SQLite `InlineResolutionContext` / Cluster 5c.
async fn compute_expansion_with_contained(
    client: &tokio_postgres::Client,
    compose_json: Option<&str>,
    force_system_versions: &HashMap<String, String>,
    system_version_defaults: &HashMap<String, String>,
    default_value_set_versions: &HashMap<String, String>,
    contained: &[serde_json::Value],
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut visited: HashSet<String> = HashSet::new();
    compute_expansion_inner(
        client,
        compose_json,
        force_system_versions,
        system_version_defaults,
        default_value_set_versions,
        contained,
        0,
        &mut visited,
    )
    .await
}

/// Recursive worker for `compute_expansion`. `depth` and `visited` thread
/// through `compose.include[].valueSet[]` references (Cluster C). The
/// reference handling intersects every referenced VS expansion (mirroring
/// FHIR R5 §4.9.5) with any local system/concept/filter base set on the
/// same include. Cycle detection raises `HtsError::VsInvalid`; depth >= 4
/// emits a warning and skips the nested ref. Mirrors
/// `sqlite/value_set.rs:expand_includes_per_clause` (3220-3354) +
/// `expand_vs_reference` (2944-3058).
fn compute_expansion_inner<'a>(
    client: &'a tokio_postgres::Client,
    compose_json: Option<&'a str>,
    force_system_versions: &'a HashMap<String, String>,
    system_version_defaults: &'a HashMap<String, String>,
    default_value_set_versions: &'a HashMap<String, String>,
    contained: &'a [serde_json::Value],
    depth: u8,
    visited: &'a mut HashSet<String>,
) -> futures::future::BoxFuture<'a, Result<Vec<ExpansionContains>, HtsError>> {
    Box::pin(compute_expansion_inner_body(
        client,
        compose_json,
        force_system_versions,
        system_version_defaults,
        default_value_set_versions,
        contained,
        depth,
        visited,
    ))
}

async fn compute_expansion_inner_body(
    client: &tokio_postgres::Client,
    compose_json: Option<&str>,
    force_system_versions: &HashMap<String, String>,
    system_version_defaults: &HashMap<String, String>,
    default_value_set_versions: &HashMap<String, String>,
    contained: &[serde_json::Value],
    depth: u8,
    visited: &mut HashSet<String>,
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
        let vs_refs_present = inc["valueSet"]
            .as_array()
            .is_some_and(|a| !a.is_empty());
        let has_local_system = inc["system"].as_str().is_some_and(|s| !s.is_empty());

        // ── compose.include[].valueSet[] handling (FHIR R5 §4.9.5) ─────────
        // Each entry is an additional condition on the include: a concept
        // matches only if it appears in EVERY referenced ValueSet. When the
        // include also has system / concept / filter, those local conditions
        // are intersected with the ref expansions. When the include has only
        // valueSet[] entries, the result is the intersection of the refs.
        if vs_refs_present {
            if depth >= 4 {
                tracing::warn!(
                    "Max ValueSet include depth (4) reached; skipping nested valueSet references"
                );
                continue;
            }

            let vs_refs = inc["valueSet"].as_array().unwrap();
            // Preserve the FIRST referenced VS's emission order (mirrors
            // sqlite/value_set.rs:3252-3262 — pagination determinism for
            // exclude/exclude-gender2 etc.).
            let mut ref_sets: Vec<HashSet<(String, String)>> = Vec::new();
            let mut display_index: HashMap<(String, String), Option<String>> = HashMap::new();
            let mut version_index: HashMap<(String, String), Option<String>> = HashMap::new();
            let mut first_ref_order: Vec<(String, String)> = Vec::new();
            let mut first_ref_seen: HashSet<(String, String)> = HashSet::new();

            for (idx, vs_ref) in vs_refs.iter().enumerate() {
                let ref_url = match vs_ref.as_str() {
                    Some(u) => u,
                    None => continue,
                };
                let codes = pg_expand_vs_reference(
                    client,
                    ref_url,
                    depth,
                    visited,
                    force_system_versions,
                    system_version_defaults,
                    default_value_set_versions,
                    contained,
                )
                .await?;
                let mut set: HashSet<(String, String)> = HashSet::new();
                for c in codes {
                    let key = (c.system.clone(), c.code.clone());
                    display_index
                        .entry(key.clone())
                        .or_insert_with(|| c.display.clone());
                    version_index
                        .entry(key.clone())
                        .or_insert_with(|| c.version.clone());
                    if idx == 0 && first_ref_seen.insert(key.clone()) {
                        first_ref_order.push(key.clone());
                    }
                    set.insert(key);
                }
                ref_sets.push(set);
            }

            let mut intersected: HashSet<(String, String)> = match ref_sets.first() {
                Some(first) => first.clone(),
                None => HashSet::new(),
            };
            for set in ref_sets.iter().skip(1) {
                intersected.retain(|k| set.contains(k));
            }

            // Build the local "base set" (system + concept + filter) and
            // intersect with the ref intersection. When the include has no
            // local system the result is just the ref intersection.
            let final_set: HashSet<(String, String)> = if has_local_system {
                let mut single_inc = inc.clone();
                if let Some(obj) = single_inc.as_object_mut() {
                    obj.remove("valueSet");
                }
                let single_compose = serde_json::json!({ "include": [single_inc] });
                let base_codes = compute_expansion_inner(
                    client,
                    Some(&single_compose.to_string()),
                    force_system_versions,
                    system_version_defaults,
                    default_value_set_versions,
                    contained,
                    depth + 1,
                    visited,
                )
                .await?;
                let mut bs: HashSet<(String, String)> = HashSet::new();
                for c in &base_codes {
                    let key = (c.system.clone(), c.code.clone());
                    bs.insert(key.clone());
                    display_index
                        .entry(key.clone())
                        .or_insert_with(|| c.display.clone());
                    version_index
                        .entry(key.clone())
                        .or_insert_with(|| c.version.clone());
                }
                intersected.intersection(&bs).cloned().collect()
            } else {
                intersected
            };

            // Emit in the first ref's order; survivors not in first_ref_order
            // (shouldn't happen — they must have been in ref_sets[0]) are
            // skipped silently.
            for key in &first_ref_order {
                if !final_set.contains(key) {
                    continue;
                }
                let (system, code) = key.clone();
                let display = display_index.get(key).cloned().unwrap_or(None);
                let version = version_index.get(key).cloned().unwrap_or(None);
                included.push(ExpansionContains {
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
                });
            }

            continue;
        }

        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        // Override order (mirrors `operations/expand.rs` + SQLite):
        //   force_system_versions[url] > include.version > system_version_defaults[url]
        // `force-system-version` overrides even an explicit include pin;
        // `system-version` only applies when the include omits version.
        let forced = force_system_versions.get(system_url).map(String::as_str);
        let raw_inc_version = inc["version"].as_str();
        let defaulted = system_version_defaults.get(system_url).map(String::as_str);
        let inc_version = forced.or(raw_inc_version).or(defaulted);

        let system_id = match resolve_compose_system_id(client, system_url, inc_version).await? {
            Some(id) => id,
            None => {
                // Distinguish two flavours of "not resolved":
                //   (a) the system URL itself isn't present in any
                //       CodeSystem row → silent warning + empty contribution
                //       (preserves the IG `*-not-found` fixtures).
                //   (b) the system exists, but the include's pinned version
                //       didn't match any stored CS version → bubble up as
                //       UNKNOWN_CODESYSTEM_VERSION_EXP per IG
                //       `version/vs-expand-v-wb` family. The operations
                //       layer (operations/expand.rs:UNKNOWN_CS_VERSION_EXP_PREFIX)
                //       recognises this sentinel and renders the proper 4xx
                //       OperationOutcome.
                //
                // Mirrors sqlite/value_set.rs:3392-3429.
                if let Some(inc_ver) = inc_version {
                    let any_row: bool = client
                        .query_one(
                            "SELECT EXISTS(SELECT 1 FROM code_systems WHERE url = $1)",
                            &[&system_url],
                        )
                        .await
                        .map(|r| r.get::<_, bool>(0))
                        .unwrap_or(false);
                    if any_row {
                        let all_versions = cs_all_stored_versions(client, system_url).await;
                        let valid_str = format_valid_versions_msg(&all_versions);
                        let text = format!(
                            "A definition for CodeSystem '{system_url}' version '{inc_ver}' \
                             could not be found, so the value set cannot be expanded. \
                             Valid versions: {valid_str}"
                        );
                        return Err(HtsError::NotFound(format!(
                            "__UNKNOWN_CS_VERSION_EXP__:{text}"
                        )));
                    }
                }
                tracing::warn!(
                    system_url,
                    inc_version,
                    "Skipping unknown code system in ValueSet compose"
                );
                continue;
            }
        };

        // Look up the resolved CS row's actual version, so each
        // ExpansionContains can carry it. The compose-pin (`inc_version`)
        // may be a wildcard pattern like "1.x" that resolved to the
        // concrete `1.0.0` — we want the concrete value to land on items
        // and feed `used-codesystem` deduplication in `operations/expand.rs`.
        // Mirrors the SQLite `cs_version.clone()` writes in
        // `sqlite/value_set.rs:compute_expansion_with_versions`.
        let cs_version: Option<String> = client
            .query_opt(
                "SELECT version FROM code_systems WHERE id = $1",
                &[&system_id],
            )
            .await
            .ok()
            .flatten()
            .and_then(|r| r.get::<_, Option<String>>(0));

        // Phase A: collect the candidate concepts dictated by `concept[]` or
        // by enumerating the whole CodeSystem.
        let mut candidates: Vec<ExpansionContains> =
            if let Some(explicit_codes) = inc["concept"].as_array() {
                let mut out = Vec::with_capacity(explicit_codes.len());
                let mut seen_codes: HashSet<String> = HashSet::new();
                for entry in explicit_codes {
                    let code = match entry["code"].as_str() {
                        Some(c) => c.to_owned(),
                        None => continue,
                    };

                    // Drop codes that don't actually exist in the CS — the
                    // IG `simple-expand-enum-bad` fixture asserts that an
                    // unknown code in compose.include[].concept[] is
                    // silently filtered out of the expansion rather than
                    // surfacing as a phantom entry. Mirrors the SQLite
                    // INNER JOIN at sqlite/value_set.rs:3441-3470.
                    let row = match client
                        .query_opt(
                            "SELECT display FROM concepts WHERE system_id = $1 AND code = $2",
                            &[&system_id, &code],
                        )
                        .await
                        .map_err(|e| HtsError::StorageError(e.to_string()))?
                    {
                        Some(r) => r,
                        None => continue,
                    };
                    let display: Option<String> = row.get(0);

                    seen_codes.insert(code.clone());
                    out.push(ExpansionContains {
                        system: system_url.to_owned(),
                        version: cs_version.clone(),
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
                // IG `parameters/parameters-expand-enum-*` semantics: when an
                // explicitly-enumerated concept is abstract (notSelectable=true),
                // its immediate children appear alongside it in the expansion
                // — the IG fixture lists the children flat at the top level.
                // Skip when depth > 0: a contained-VS / nested ref (Cluster C)
                // expansion is supposed to return EXACTLY the enumerated codes
                // so that intersections with sibling refs are well-defined.
                // The IG `simple/simple-expand-contained` fixture pins this:
                // its #vs1 enumerates code2 and the expected expansion is
                // exactly {code2} (not the auto-expanded {code2, code2a, code2b}).
                // Mirrors sqlite/value_set.rs:3502-3543 (`if depth == 0`).
                let mut abstract_codes_in_set: Vec<String> = Vec::new();
                if depth == 0 {
                    for c in &out {
                        if is_concept_abstract(client, &c.system, &c.code).await {
                            abstract_codes_in_set.push(c.code.clone());
                        }
                    }
                }
                for parent_code in abstract_codes_in_set {
                    let child_rows = client
                        .query(
                            "SELECT c.code, c.display
                               FROM concept_hierarchy h
                               JOIN concepts c
                                 ON c.system_id = h.system_id AND c.code = h.child_code
                              WHERE h.system_id = $1 AND h.parent_code = $2",
                            &[&system_id, &parent_code],
                        )
                        .await
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    for row in child_rows {
                        let child_code: String = row.get(0);
                        let child_display: Option<String> = row.get(1);
                        if seen_codes.insert(child_code.clone()) {
                            out.push(ExpansionContains {
                                system: system_url.to_owned(),
                                version: cs_version.clone(),
                                code: child_code,
                                display: child_display,
                                is_abstract: None,
                                inactive: None,
                                designations: vec![],
                                properties: vec![],
                                extensions: vec![],
                                contains: vec![],
                            });
                        }
                    }
                }
                out
            } else {
                // ORDER BY id preserves CodeSystem-defined insertion order
                // (concepts.id is BIGSERIAL → monotonic), which is what FHIR
                // expansion semantics require and what the IG
                // `exclude/exclude-gender2` fixture pins (`male` first, not
                // `female`). Concepts are inserted in the order they appear
                // in the source CodeSystem.concept[] array, so the
                // autoincrement column doubles as a stable definition-order
                // column. Mirrors sqlite/value_set.rs:3547-3556.
                let code_rows = client
                    .query(
                        "SELECT code, display FROM concepts WHERE system_id = $1 ORDER BY id",
                        &[&system_id],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                code_rows
                    .into_iter()
                    .map(|row| ExpansionContains {
                        system: system_url.to_owned(),
                        version: cs_version.clone(),
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
    //
    // Version-aware exclude: per the IG `overload-expand-exclude*` fixtures,
    // an `exclude.concept[]` listing with a `version` pin removes only the
    // pinned version's copies of that code (mirrors sqlite/value_set.rs:3712-3735).
    // The version-blind `denied` covers exc.concept[] without a pin and
    // exc.filter[] without versionsMatch=false. The
    // `denied_whole_system_versioned` set holds filter-based excludes with
    // version pins; their effect is whole-system version-aware ONLY when
    // the VS sets `versionsMatch=false` (otherwise collapsed into `denied`).
    //
    // versionsMatch=false detection mirrors sqlite/value_set.rs:3140-3169 —
    // looks for a `valueset-expansion-parameter` extension on `compose` whose
    // inner `name=versionsMatch, value=false`.
    let versions_match_false = compose
        .get("extension")
        .and_then(|e| e.as_array())
        .map(|exts| {
            exts.iter().any(|ext| {
                let url_match = ext.get("url").and_then(|u| u.as_str())
                    == Some("http://hl7.org/fhir/StructureDefinition/valueset-expansion-parameter");
                if !url_match {
                    return false;
                }
                let inner = ext.get("extension").and_then(|e| e.as_array());
                let mut name: Option<&str> = None;
                let mut value: Option<&str> = None;
                if let Some(arr) = inner {
                    for sub in arr {
                        match sub.get("url").and_then(|u| u.as_str()) {
                            Some("name") => {
                                name = sub.get("valueCode").and_then(|v| v.as_str());
                            }
                            Some("value") => {
                                value = sub.get("valueString").and_then(|v| v.as_str());
                            }
                            _ => {}
                        }
                    }
                }
                name == Some("versionsMatch") && value == Some("false")
            })
        })
        .unwrap_or(false);

    let excludes = compose["exclude"].as_array().unwrap_or(&empty_arr);
    let mut denied: HashSet<(String, String)> = HashSet::new();
    let mut denied_versioned: HashSet<(String, String, String)> = HashSet::new();
    let mut denied_whole_system_versioned: HashSet<(String, String, String)> = HashSet::new();

    for exc in excludes {
        let exc_vs_refs_present = exc["valueSet"]
            .as_array()
            .is_some_and(|a| !a.is_empty());

        // exclude.valueSet[] handling — mirror sqlite/value_set.rs:3631-3697.
        // Each ref's expansion contributes (system, code) pairs to the denied
        // set. Multiple refs are intersected. When the exclude also has a
        // local system condition, the ref intersection is further narrowed
        // by the local set.
        if exc_vs_refs_present {
            if depth >= 4 {
                tracing::warn!(
                    "Max ValueSet exclude depth (4) reached; skipping nested valueSet references"
                );
                continue;
            }
            let mut ref_sets: Vec<HashSet<(String, String)>> = Vec::new();
            for vs_ref in exc["valueSet"].as_array().unwrap() {
                let ref_url = match vs_ref.as_str() {
                    Some(u) => u,
                    None => continue,
                };
                let resolved = pg_expand_vs_reference(
                    client,
                    ref_url,
                    depth,
                    visited,
                    force_system_versions,
                    system_version_defaults,
                    default_value_set_versions,
                    contained,
                )
                .await?;
                let mut set = HashSet::new();
                for c in resolved {
                    set.insert((c.system, c.code));
                }
                ref_sets.push(set);
            }
            let mut intersected: HashSet<(String, String)> = match ref_sets.first() {
                Some(first) => first.clone(),
                None => HashSet::new(),
            };
            for set in ref_sets.iter().skip(1) {
                intersected.retain(|k| set.contains(k));
            }

            let has_exc_local_system =
                exc["system"].as_str().is_some_and(|s| !s.is_empty());
            if has_exc_local_system {
                let mut single_exc = exc.clone();
                if let Some(obj) = single_exc.as_object_mut() {
                    obj.remove("valueSet");
                }
                // Reuse the include path with a synthesized single-include
                // compose to materialise the local exclude condition's
                // codes — same shape as the SQLite expand_single_include_local
                // helper.
                let single_compose = serde_json::json!({ "include": [single_exc] });
                let local = compute_expansion_inner(
                    client,
                    Some(&single_compose.to_string()),
                    force_system_versions,
                    system_version_defaults,
                    default_value_set_versions,
                    contained,
                    depth + 1,
                    visited,
                )
                .await?;
                let local_set: HashSet<(String, String)> =
                    local.into_iter().map(|c| (c.system, c.code)).collect();
                intersected.retain(|k| local_set.contains(k));
            }

            for k in intersected {
                denied.insert(k);
            }
            continue;
        }

        let exc_system = exc["system"].as_str().unwrap_or("").to_owned();
        let exc_version_pin: Option<String> = exc["version"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if let Some(codes) = exc["concept"].as_array() {
            for entry in codes {
                if let Some(code) = entry["code"].as_str() {
                    match &exc_version_pin {
                        Some(v) => {
                            denied_versioned.insert((
                                exc_system.clone(),
                                v.clone(),
                                code.to_owned(),
                            ));
                        }
                        None => {
                            denied.insert((exc_system.clone(), code.to_owned()));
                        }
                    }
                }
            }
        }

        // Filter-based excludes: resolve system_id and apply the same filter
        // helper, then add the resulting codes to the appropriate denied
        // set. Whole-system version-aware exclude only fires when the VS has
        // `versionsMatch=false`; otherwise the version pin is collapsed to
        // version-blind. Mirrors sqlite/value_set.rs:3744-3755.
        let has_exc_filter = exc["filter"]
            .as_array()
            .is_some_and(|a| !a.is_empty());
        let has_exc_concept = exc["concept"]
            .as_array()
            .is_some_and(|a| !a.is_empty());
        // exclude { system: X } with no concept[], no filter[], no valueSet[]
        // means "remove every code from X". Mirrors sqlite/value_set.rs:3738-3754
        // (whole-system exclude branch). Without this PG silently dropped the
        // exclude, leaving total != 0 for the IG `exclude/exclude-all` fixture.
        if (has_exc_filter || (!has_exc_concept && !exc_system.is_empty()))
            && !exc_system.is_empty()
        {
            // Same override order as the include loop above.
            let exc_forced = force_system_versions.get(&exc_system).map(String::as_str);
            let exc_raw = exc["version"].as_str();
            let exc_def = system_version_defaults.get(&exc_system).map(String::as_str);
            let exc_version = exc_forced.or(exc_raw).or(exc_def);
            if let Some(exc_system_id) =
                resolve_compose_system_id(client, &exc_system, exc_version).await?
            {
                // Either apply the explicit filters or, when there are
                // none, expand the entire CodeSystem (whole-system exclude).
                let codes_to_deny: Vec<String> = if has_exc_filter {
                    let filtered = apply_compose_filters_pg(
                        client,
                        &exc_system,
                        &exc_system_id,
                        exc,
                    )
                    .await?;
                    filtered.unwrap_or_default().into_iter().map(|c| c.code).collect()
                } else {
                    let rows = client
                        .query(
                            "SELECT code FROM concepts WHERE system_id = $1",
                            &[&exc_system_id],
                        )
                        .await
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    rows.into_iter().map(|r| r.get::<_, String>(0)).collect()
                };
                for code in codes_to_deny {
                    match (&exc_version_pin, versions_match_false) {
                        (Some(v), true) => {
                            denied_whole_system_versioned.insert((
                                exc_system.clone(),
                                v.clone(),
                                code,
                            ));
                        }
                        _ => {
                            denied.insert((exc_system.clone(), code));
                        }
                    }
                }
            }
        }
    }

    if !denied.is_empty()
        || !denied_versioned.is_empty()
        || (versions_match_false && !denied_whole_system_versioned.is_empty())
    {
        included.retain(|c| {
            if denied.contains(&(c.system.clone(), c.code.clone())) {
                return false;
            }
            if let Some(ver) = c.version.as_deref() {
                if denied_versioned.contains(&(
                    c.system.clone(),
                    ver.to_owned(),
                    c.code.clone(),
                )) {
                    return false;
                }
                if versions_match_false
                    && denied_whole_system_versioned.contains(&(
                        c.system.clone(),
                        ver.to_owned(),
                        c.code.clone(),
                    ))
                {
                    return false;
                }
            }
            true
        });
    }

    Ok(included)
}

/// Resolve a `compose.include[].valueSet[]` reference to its expansion.
///
/// - Bubbles `HtsError::VsInvalid` when the URL is already in `visited` —
///   cycles are reported as hard errors so the operations layer can render
///   the FHIR-spec OperationOutcome (4xx). Mirrors sqlite/value_set.rs:2952-2980.
/// - Honours an explicit `|version` suffix on the ref, falling back to a
///   `default-valueset-version` request-level pin when the ref is bare
///   (mirrors sqlite/value_set.rs:3016-3026).
/// - Recurses via `compute_expansion_inner` at depth+1 so further nested
///   `valueSet[]` refs participate in cycle detection and depth limits.
/// - `#fragment` (contained-VS) refs are resolved against the supplied
///   `contained[]` array. The matched contained VS's compose is then
///   expanded recursively — same code path as a stored VS reference but
///   without a DB lookup.
async fn pg_expand_vs_reference(
    client: &tokio_postgres::Client,
    ref_url: &str,
    depth: u8,
    visited: &mut HashSet<String>,
    force_system_versions: &HashMap<String, String>,
    system_version_defaults: &HashMap<String, String>,
    default_value_set_versions: &HashMap<String, String>,
    contained: &[serde_json::Value],
) -> Result<Vec<ExpansionContains>, HtsError> {
    if visited.contains(ref_url) {
        return Err(HtsError::VsInvalid(format!(
            "Cyclic reference detected when excluding {ref_url} via [{ref_url}]"
        )));
    }
    visited.insert(ref_url.to_owned());

    if let Some(id) = ref_url.strip_prefix('#') {
        // Look up the matching contained ValueSet and recurse into its
        // compose. Mirrors sqlite/value_set.rs:lookup_compose +
        // expand_vs_reference.
        let compose_str = contained.iter().find_map(|r| {
            if r.get("id").and_then(|v| v.as_str()) == Some(id)
                && r.get("resourceType").and_then(|v| v.as_str()) == Some("ValueSet")
            {
                r.get("compose").map(|c| c.to_string())
            } else {
                None
            }
        });
        let result = if let Some(compose) = compose_str {
            compute_expansion_inner(
                client,
                Some(&compose),
                force_system_versions,
                system_version_defaults,
                default_value_set_versions,
                contained,
                depth + 1,
                visited,
            )
            .await
        } else {
            tracing::warn!(
                ref_url,
                "Contained ValueSet reference not found in inline contained[] array; treating as empty"
            );
            Ok(vec![])
        };
        visited.remove(ref_url);
        return result;
    }

    let (bare_url, ref_version) = match ref_url.split_once('|') {
        Some((u, v)) => (u, Some(v.to_string())),
        None => (ref_url, None),
    };
    let effective_version: Option<String> = ref_version.clone().or_else(|| {
        default_value_set_versions
            .get(bare_url)
            .cloned()
    });
    let pin_was_explicit = ref_version.is_some() || effective_version.is_some();

    let result = match resolve_value_set_versioned(
        client,
        bare_url,
        effective_version.as_deref(),
        None,
    )
    .await
    {
        Ok((_ref_vs_id, ref_compose)) => {
            // Recurse — the bypass for the cache mirrors compute_expansion's
            // own multi-version handling implicitly; for ref expansion the
            // cache is fine but we pay re-compute cost here for simplicity.
            compute_expansion_inner(
                client,
                ref_compose.as_deref(),
                force_system_versions,
                system_version_defaults,
                default_value_set_versions,
                contained,
                depth + 1,
                visited,
            )
            .await
        }
        Err(e) => {
            // Explicit version pin that doesn't resolve must surface as a
            // hard NotFound (mirrors SQLite valueset-version/expand-indirect-
            // expand-zero-pinned-wrong fixture). Otherwise, warn and
            // contribute an empty set so a missing optional ref doesn't
            // collapse the whole expansion.
            if pin_was_explicit && matches!(e, HtsError::NotFound(_)) {
                Err(e)
            } else {
                tracing::warn!(
                    ref_url,
                    "Referenced ValueSet not found; excluded from expansion"
                );
                Ok(vec![])
            }
        }
    };

    visited.remove(ref_url);
    result
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
    let filters_raw = match inc["filter"].as_array() {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(None),
    };

    // Normalise R4-encoded filter ops. The R5→R4 ValueSet converter clears
    // `op` when the operator has no R4 enum value (CHILDOF, DESCENDENTLEAF)
    // and stashes the original code in a cross-version extension. Restore
    // the op from `_op.extension[]` (HAPI converter canonical placement) or
    // `filter.extension[]` (legacy clients) before dispatch — drives the IG
    // `simple-expand-child-of` "R5/R4 transformation" test that arrives with
    // `op=null` when the server reports fhirVersion=4.x.
    //
    // Mirrors sqlite/value_set.rs:3812-3871.
    const EXT_FILTER_OP_URL: &str =
        "http://hl7.org/fhir/5.0/StructureDefinition/extension-ValueSet.compose.include.filter.op";
    fn find_filter_op_extension(exts: &serde_json::Value) -> Option<&str> {
        exts.as_array()?.iter().find_map(|ext| {
            let url = ext.get("url").and_then(|v| v.as_str())?;
            if url == EXT_FILTER_OP_URL {
                ext.get("valueCode").and_then(|v| v.as_str())
            } else {
                None
            }
        })
    }
    let filters_owned: Vec<serde_json::Value> = filters_raw
        .iter()
        .map(|f| {
            let mut f = f.clone();
            let needs_recovery = f
                .get("op")
                .and_then(|v| v.as_str())
                .map(str::is_empty)
                .unwrap_or(true);
            if needs_recovery {
                let recovered = f
                    .get("_op")
                    .and_then(|primitive| primitive.get("extension"))
                    .and_then(find_filter_op_extension)
                    .map(str::to_owned)
                    .or_else(|| {
                        f.get("extension")
                            .and_then(find_filter_op_extension)
                            .map(str::to_owned)
                    });
                if let Some(code) = recovered {
                    f["op"] = serde_json::Value::String(code);
                }
            }
            f
        })
        .collect();
    let filters: &[serde_json::Value] = &filters_owned;

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
            // `descendent-of` is `is-a` minus the root itself — strict subtree.
            // The IG `other/dual-filter` fixture uses
            // `op=descendent-of, value=AA` AND-intersected with property=A; PG
            // previously fell through to the "unsupported, treat as empty" branch
            // and the include collapsed.
            "descendent-of" => {
                let mut v = pg_filter_is_a(client, system_url, system_id, value).await?;
                v.retain(|(c, _)| c != value);
                v
            }
            "child-of" => pg_filter_child_of(client, system_url, system_id, value).await?,
            "regex" => pg_filter_regex(client, system_id, property, value).await?,
            // Single-value `not-in` reduces to `!=`; both return concepts
            // whose property is NOT this value (concepts without the
            // property pass). Drives notSelectable-prop-out* IG fixtures.
            "!=" => {
                pg_filter_property_ne(client, system_url, system_id, property, value).await?
            }
            "not-in" if !value.contains(',') => {
                pg_filter_property_ne(client, system_url, system_id, property, value).await?
            }
            other => {
                // TODO: parity — SQLite also handles descendent-of/generalizes/
                // exists/multi-value-in. Treat them as empty set so the
                // include collapses (rather than panicking or accidentally
                // returning every concept).
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
/// equality (strict). Honours locally-renamed property aliases. Returns the
/// concepts that have an EXPLICIT row in `concept_properties` matching
/// the (property-alias, value) pair — concepts that omit the property
/// entirely do not match (mirrors SQLite's `query_property_eq`). The IG
/// `notSelectable/notSelectable-{prop,noprop,reprop,unprop}-false`
/// fixtures depend on strict equality for the `=false` case (a previous
/// "absence means false" hack returned every code without an explicit
/// =true marker, which silently included unannotated codes the IG
/// expects filtered out).
async fn pg_filter_property_eq(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    let property_aliases = cs_property_local_codes(client, system_url, property).await;

    let rows = client
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
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Inverse of [`pg_filter_property_eq`]: returns every concept in
/// `system_id` MINUS those matching the (property-alias, value) equality.
/// Drives FHIR `op=!=` and single-value `op=not-in`. Concepts without the
/// property at all PASS — the IG `notSelectable-prop-out*` fixtures rely on
/// "select every concept whose `notSelectable` is NOT `true` (or absent)".
/// Mirrors sqlite/value_set.rs:3998-4021.
async fn pg_filter_property_ne(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    let property_aliases = cs_property_local_codes(client, system_url, property).await;

    let rows = client
        .query(
            "SELECT c.code, c.display
               FROM concepts c
              WHERE c.system_id = $1
                AND NOT EXISTS (
                    SELECT 1 FROM concept_properties cp
                     WHERE cp.concept_id = c.id
                       AND cp.property = ANY($2::text[])
                       AND cp.value = $3
                )",
            &[&system_id, &property_aliases, &value],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Load every `(parent_code, child_code)` edge for `system_id` into a
/// `HashMap<parent_code, Vec<child_code>>` and cache it in
/// [`super::hierarchy_cache`]. Single-flight via `OnceCell` so a 50-VU
/// stampede onto the first request runs one scan, not 50.
///
/// For SNOMED this query returns ~1.2 M rows and takes ~1-2 s on the bench
/// runner — paid exactly once per (server boot × system) — after which
/// every is-a / descendent-of walk in process memory takes 10-50 ms instead
/// of 400-700 ms per recursive-CTE call. The map sits at ~60 MB resident
/// for SNOMED.
async fn load_hierarchy_edges(
    client: &tokio_postgres::Client,
    system_id: &str,
) -> Result<Arc<HierarchyEdges>, HtsError> {
    let cell = {
        let mut guard = hierarchy_cache()
            .write()
            .map_err(|e| HtsError::StorageError(format!("hierarchy_cache poisoned: {e}")))?;
        guard
            .entry(system_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
            .clone()
    };

    let edges = cell
        .get_or_try_init(|| async {
            let started = std::time::Instant::now();
            let rows = client
                .query(
                    "SELECT parent_code, child_code FROM concept_hierarchy WHERE system_id = $1",
                    &[&system_id],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut children_of: HashMap<String, Vec<String>> = HashMap::new();
            for row in rows {
                let parent: String = row.get(0);
                let child: String = row.get(1);
                children_of.entry(parent).or_default().push(child);
            }
            tracing::info!(
                system_id,
                edges = children_of.values().map(Vec::len).sum::<usize>(),
                parents = children_of.len(),
                ms = started.elapsed().as_millis(),
                "HIER_LOAD: built in-memory hierarchy edge map"
            );
            Ok::<Arc<HierarchyEdges>, HtsError>(Arc::new(HierarchyEdges { children_of }))
        })
        .await?
        .clone();

    Ok(edges)
}

/// BFS descendants of `root_code` over the in-memory `children_of` map,
/// including the root itself. Mirrors the FHIR `is-a` semantics (the root
/// IS-A itself). Returns a `HashSet` so duplicate visits collapse cheaply
/// — DAG-shaped hierarchies (a concept with multiple parents) would
/// otherwise yield duplicates.
fn bfs_descendants_inmem(edges: &HierarchyEdges, root_code: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    out.insert(root_code.to_owned());
    let mut frontier: Vec<String> = vec![root_code.to_owned()];
    while let Some(code) = frontier.pop() {
        if let Some(children) = edges.children_of.get(&code) {
            for child in children {
                if out.insert(child.clone()) {
                    frontier.push(child.clone());
                }
            }
        }
    }
    out
}

/// Resolve `is-a` filter: the root code itself plus every descendant via
/// `concept_hierarchy`. Iter 7f path: load the system's edges into process
/// memory once (via [`load_hierarchy_edges`]), BFS in Rust, then issue a
/// single batched `SELECT code, display FROM concepts WHERE code = ANY(...)`
/// for the resulting code set. Replaces the per-request recursive CTE,
/// which was the EX01/EX05 cold-miss bottleneck (~600 ms p99 cold-miss
/// even when the in-memory `inline_compose_cache` was warming, because
/// the bench hits ~100 distinct SNOMED roots per 30 s window).
///
/// On any failure inside [`load_hierarchy_edges`] we propagate the error
/// rather than falling back to the recursive CTE — a failure here would
/// most likely be a transient pool error, and the OnceCell stays empty on
/// `Err` so the next request retries cleanly.
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

    let edges = load_hierarchy_edges(client, system_id).await?;
    let codes_set = bfs_descendants_inmem(&edges, root_code);
    let codes: Vec<String> = codes_set.into_iter().collect();

    if codes.is_empty() {
        return Ok(Vec::new());
    }

    let rows = client
        .query(
            "SELECT code, display FROM concepts \
             WHERE system_id = $1 AND code = ANY($2::text[])",
            &[&system_id, &codes],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Direct children of `parent_code` (one level only, parent NOT included).
/// Mirrors the SQLite `child-of` filter semantics: returns the codes whose
/// `concept_hierarchy.parent_code` literally equals the supplied code.
async fn pg_filter_child_of(
    client: &tokio_postgres::Client,
    system_url: &str,
    system_id: &str,
    parent_code: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    if parent_code.is_empty() {
        return Err(HtsError::VsInvalid(format!(
            "The system {system_url} filter with property = concept, op = child-of has no value"
        )));
    }

    let rows = client
        .query(
            "SELECT c.code, c.display
               FROM concept_hierarchy h
               JOIN concepts c
                 ON c.system_id = h.system_id AND c.code = h.child_code
              WHERE h.system_id = $1 AND h.parent_code = $2",
            &[&system_id, &parent_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
        .collect())
}

/// Regex match against the named property using PG's POSIX `~` operator.
/// `property` may be `code`, `display`, or any concept property name. For
/// non-code/display properties the SQL pre-narrows by `concept_properties`
/// rows for that property name (resolving FHIR-canonical aliases via
/// `cs_property_local_codes`), then anchored regex is applied to the value.
async fn pg_filter_regex(
    client: &tokio_postgres::Client,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<(String, Option<String>)>, HtsError> {
    // The FHIR `regex` filter requires a full match. PG's `~` is POSIX
    // regex and matches substrings unless anchored, and Rust's `regex`
    // crate likewise matches anywhere unless anchored. Wrap once here
    // (mirrors SQLite's `\A...\z` wrap at sqlite/value_set.rs:4727).
    let anchored_sql = format!("^(?:{value})$");

    match property {
        "code" | "" => {
            let rows = client
                .query(
                    "SELECT c.code, c.display FROM concepts c
                      WHERE c.system_id = $1 AND c.code ~ $2",
                    &[&system_id, &anchored_sql],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            Ok(rows
                .into_iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
                .collect())
        }
        "display" => {
            let rows = client
                .query(
                    "SELECT c.code, c.display FROM concepts c
                      WHERE c.system_id = $1 AND c.display ~ $2",
                    &[&system_id, &anchored_sql],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            Ok(rows
                .into_iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, Option<String>>(1)))
                .collect())
        }
        _ => {
            // Match against a named property value. Pre-narrow at SQL to
            // rows that carry the property, then anchored regex on the
            // value in Rust (PG's POSIX regex differs subtly from Rust's;
            // matching in Rust keeps results consistent with the
            // code/display branches above and with SQLite).
            //
            // Mirrors sqlite/value_set.rs:4783-4827.
            let regex = match regex::Regex::new(&anchored_sql) {
                Ok(r) => r,
                Err(e) => {
                    return Err(HtsError::VsInvalid(format!(
                        "Invalid regex '{value}' on property '{property}': {e}"
                    )));
                }
            };
            let rows = client
                .query(
                    "SELECT c.code, c.display, cp.value
                       FROM concept_properties cp
                       JOIN concepts c ON c.id = cp.concept_id AND c.system_id = $1
                      WHERE cp.property = $2",
                    &[&system_id, &property],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            // A concept may have multiple values for the same property; keep
            // it if any value matches. Dedupe by code in case more than one
            // value matches.
            let mut seen: HashSet<String> = HashSet::new();
            let mut out: Vec<(String, Option<String>)> = Vec::new();
            for r in rows {
                let code: String = r.get(0);
                let display: Option<String> = r.get(1);
                let value: String = r.get(2);
                if regex.is_match(&value) && seen.insert(code.clone()) {
                    out.push((code, display));
                }
            }
            Ok(out)
        }
    }
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
        // Pattern matching ONLY for `.x` wildcards / bare "x" / dot-containing
        // versions ("1.0" → highest "1.0.x"). Single-integer versions ("1",
        // "2") with no dot must EXACT-match — IG fixtures (vs-expand-v-wb)
        // treat bare "1" as a distinct unrecognised version that must
        // surface UNKNOWN_CODESYSTEM_VERSION rather than alias to "1.x.x".
        // Mirrors sqlite/value_set.rs:resolve_ver_against_candidates rules.
        Some(v) if v.contains(".x") || v == "x" || v.contains('.') => {
            compose_select_version(&candidates, v)
        }
        Some(v) => candidates
            .into_iter()
            .find(|(_, ver)| ver.as_deref() == Some(v)),
        None => candidates.into_iter().next(),
    };
    Ok(chosen.map(|(id, _)| id))
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

    // Bulk-insert via UNNEST over four parallel arrays. Replaces 5,975
    // per-row roundtrips with one query for a typical VSAC VS — measurable
    // on the EX04 cold-miss path. `ON CONFLICT DO NOTHING` is evaluated
    // per row, preserving the prior dedupe semantics.
    if !codes.is_empty() {
        let n = codes.len();
        let mut systems: Vec<&str> = Vec::with_capacity(n);
        let mut codes_arr: Vec<&str> = Vec::with_capacity(n);
        let mut displays: Vec<Option<&str>> = Vec::with_capacity(n);
        let mut versions: Vec<Option<&str>> = Vec::with_capacity(n);
        for item in codes {
            systems.push(item.system.as_str());
            codes_arr.push(item.code.as_str());
            displays.push(item.display.as_deref());
            versions.push(item.version.as_deref());
        }

        tx.execute(
            "INSERT INTO value_set_expansions (value_set_id, system_url, code, display, version)
             SELECT $1, sys, code, display, version
             FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[])
                  AS t(sys, code, display, version)
             ON CONFLICT DO NOTHING",
            &[&vs_id, &systems, &codes_arr, &displays, &versions],
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
/// Look up the display for a code in a CodeSystem, optionally pinned to a
/// specific version. When `version` is None, falls back to the highest
/// stored version. Returns None when the code isn't present in any version
/// of the system.
async fn pg_lookup_cs_display(
    client: &tokio_postgres::Client,
    system_url: &str,
    version: Option<&str>,
    code: &str,
) -> Option<String> {
    let row = match version {
        Some(v) => client
            .query_opt(
                "SELECT c.display FROM concepts c
                   JOIN code_systems s ON s.id = c.system_id
                  WHERE s.url = $1 AND s.version = $2 AND c.code = $3",
                &[&system_url, &v, &code],
            )
            .await
            .ok()
            .flatten(),
        None => client
            .query_opt(
                "SELECT c.display FROM concepts c
                   JOIN code_systems s ON s.id = c.system_id
                  WHERE s.url = $1 AND c.code = $2
                  ORDER BY COALESCE(s.version, '') DESC
                  LIMIT 1",
                &[&system_url, &code],
            )
            .await
            .ok()
            .flatten(),
    }?;
    row.get::<_, Option<String>>(0)
}

pub(super) async fn cs_version_for_msg(
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
pub(super) async fn cs_content_for_url(
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
pub(super) async fn cs_is_case_insensitive(
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
pub(super) async fn cs_property_local_codes(
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
pub(super) async fn is_concept_inactive(
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
pub(super) async fn is_concept_abstract(
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
pub(super) async fn detect_cs_version_mismatch(
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

// ── Inline-compose expansion cache helpers ───────────────────────────────────
//
// Mirrors backends/sqlite/value_set.rs:296+ (inline_compose_index). The cache
// stores the FULL unfiltered, unpaged code list for a canonicalised compose +
// version-pin set. Filter/count/offset are applied to the cached vec in Rust
// on each request, so repeated EX02/04/05/06/07/08 pool entries (which differ
// only in count/filter) reuse a single cold-miss expansion.

/// FNV-1a 64-bit hash. Matches `backends/sqlite/value_set.rs:fnv64`.
fn fnv64(data: &[u8]) -> u64 {
    const PRIME: u64 = 0x00000100000001B3;
    const OFFSET: u64 = 0xcbf29ce484222325;
    let mut h = OFFSET;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Canonicalise a `HashMap<String, String>` to a stable key suffix.
fn canon_map(m: &std::collections::HashMap<String, String>) -> String {
    if m.is_empty() {
        return String::new();
    }
    let mut entries: Vec<(&String, &String)> = m.iter().collect();
    entries.sort();
    entries
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Build a cache key for an inline-compose expansion. Folds the compose body
/// and every request input that changes which codes are emitted (version
/// pins, contained VSes). Filter, count, offset, hierarchical are NOT folded
/// in — they are post-cache slicing knobs.
fn build_inline_cache_key(
    compose_str: &str,
    force_system_versions: &std::collections::HashMap<String, String>,
    system_version_defaults: &std::collections::HashMap<String, String>,
    default_value_set_versions: &std::collections::HashMap<String, String>,
    contained: &[serde_json::Value],
) -> u64 {
    let contained_str = if contained.is_empty() {
        String::new()
    } else {
        serde_json::to_string(contained).unwrap_or_default()
    };
    let canonical = format!(
        "inline|{compose_str}|{}|{}|{}|{contained_str}",
        canon_map(force_system_versions),
        canon_map(system_version_defaults),
        canon_map(default_value_set_versions),
    );
    fnv64(canonical.as_bytes())
}

/// Build a cache key for a stored / `?fhir_vs` ValueSet expansion. Same idea
/// as [`build_inline_cache_key`] but keyed by canonical URL (+ optional
/// version pin) instead of an inline compose body.
fn build_url_cache_key(
    url: &str,
    value_set_version: Option<&str>,
    force_system_versions: &std::collections::HashMap<String, String>,
    system_version_defaults: &std::collections::HashMap<String, String>,
    default_value_set_versions: &std::collections::HashMap<String, String>,
) -> u64 {
    let canonical = format!(
        "url|{url}|{}|{}|{}|{}",
        value_set_version.unwrap_or(""),
        canon_map(force_system_versions),
        canon_map(system_version_defaults),
        canon_map(default_value_set_versions),
    );
    fnv64(canonical.as_bytes())
}

/// Build the synthetic compose value for a `?fhir_vs` implicit ValueSet
/// pattern. Factored out so the `OnceCell` cold-miss closure can build it
/// lazily (only the winning VU does the work).
fn build_implicit_compose_value(cs_url: &str, pattern: &FhirVsPattern) -> serde_json::Value {
    match pattern {
        FhirVsPattern::AllConcepts => serde_json::json!({
            "include": [{ "system": cs_url }]
        }),
        FhirVsPattern::IsA(code) => serde_json::json!({
            "include": [{
                "system": cs_url,
                "filter": [{
                    "property": "concept",
                    "op": "is-a",
                    "value": code,
                }],
            }]
        }),
    }
}

/// Apply the `max_expansion_size` guardrail to a freshly-computed code list.
/// Cold-page fast path for purely extensional ValueSets.
///
/// When a `compose` only carries `include[].concept[]` lists (no `filter[]`,
/// no nested `valueSet[]` refs), the expansion is just a flat list of
/// (system, code, display) triples already present in the JSON. We parse the
/// compose body, apply optional `exclude[]` and the request `filter`, then
/// slice the requested page — all in memory, no DB I/O.
///
/// This is the EX04 hot path. VSAC ValueSets are extensional with ~5 000
/// codes plus embedded `display` per concept, and PG's `compute_expansion`
/// previously took 10-30 s to materialise the full list and a `populate_cache`
/// transaction on every cold-miss. The fast path is ~1 ms.
///
/// Mirrors `backends/sqlite/value_set.rs:compose_page_fast`. Returns `None`
/// (caller falls through to `compute_expansion`) when:
/// * any include has a non-empty `filter[]` (intensional — needs SQL filter),
/// * any include has a non-empty `valueSet[]` ref (needs recursive expand),
/// * any include has no `concept[]` (also intensional — implicit "all codes"),
/// * the compose JSON fails to parse.
///
/// `client` is used for the rare display fallback: when an extensional
/// `include[].concept` entry has no embedded `display` but the (system, code)
/// row exists in the local `concepts` table. VSAC URLs always embed display,
/// so the typical fast path makes zero DB calls.
async fn compose_page_fast_pg(
    client: &tokio_postgres::Client,
    compose_json: Option<&str>,
    offset: usize,
    limit: usize,
    filter: Option<&str>,
) -> Result<Option<(Vec<ExpansionContains>, u32)>, HtsError> {
    let compose: serde_json::Value = match compose_json {
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        },
        None => return Ok(None),
    };

    let includes = match compose["include"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return Ok(None),
    };

    // Bail on anything that needs a real expansion engine.
    for inc in includes {
        if inc["concept"].as_array().is_none() {
            return Ok(None);
        }
        if inc["filter"].as_array().is_some_and(|f| !f.is_empty()) {
            return Ok(None);
        }
        if inc["valueSet"].as_array().is_some_and(|v| !v.is_empty()) {
            return Ok(None);
        }
    }

    // Collect (system_url, code, embedded_display) triples in compose order.
    let mut all_triples: Vec<(String, String, Option<String>)> = Vec::new();
    for inc in includes {
        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s.to_owned(),
            _ => continue,
        };
        if let Some(concepts) = inc["concept"].as_array() {
            for c in concepts {
                if let Some(code) = c["code"].as_str() {
                    let display = c["display"].as_str().map(|s| s.to_owned());
                    all_triples.push((system_url.clone(), code.to_owned(), display));
                }
            }
        }
    }

    // Apply exclusions (pure code-based, no DB).
    if let Some(excl) = compose["exclude"].as_array() {
        if !excl.is_empty() {
            let mut exclude_set: HashSet<(String, String)> = HashSet::new();
            for exc in excl {
                let sys = exc["system"].as_str().unwrap_or("").to_owned();
                if let Some(concepts) = exc["concept"].as_array() {
                    for c in concepts {
                        if let Some(code) = c["code"].as_str() {
                            exclude_set.insert((sys.clone(), code.to_owned()));
                        }
                    }
                }
            }
            all_triples
                .retain(|(sys, code, _)| !exclude_set.contains(&(sys.clone(), code.clone())));
        }
    }

    // Apply text filter against compose-embedded code and display — pure
    // in-memory, no DB required. Mirrors the SQLite path's behaviour for
    // VSAC ValueSets that the local DB doesn't carry every code for.
    if let Some(f) = filter {
        let lower = f.to_lowercase();
        all_triples.retain(|(_, code, display)| {
            code.to_lowercase().contains(&lower)
                || display
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&lower))
                    .unwrap_or(false)
        });
    }

    let total = all_triples.len() as u32;

    // Paginate.
    let page_triples: Vec<(String, String, Option<String>)> =
        all_triples.into_iter().skip(offset).take(limit).collect();

    if page_triples.is_empty() {
        return Ok(Some((vec![], total)));
    }

    // Build result. Use the compose-embedded display when present; only fall
    // back to a DB lookup when the compose entry omitted it (rare for VSAC).
    let mut result = Vec::with_capacity(page_triples.len());
    let mut system_id_cache: HashMap<String, Option<String>> = HashMap::new();

    for (system_url, code, embedded_display) in &page_triples {
        let display = if embedded_display.is_some() {
            embedded_display.clone()
        } else {
            let system_id = if let Some(sid) = system_id_cache.get(system_url) {
                sid.clone()
            } else {
                let sid = resolve_system_id_pg(client, system_url).await?;
                system_id_cache.insert(system_url.clone(), sid.clone());
                sid
            };
            if let Some(sid) = system_id {
                client
                    .query_opt(
                        "SELECT display FROM concepts WHERE system_id = $1 AND code = $2",
                        &[&sid, &code],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .and_then(|r| r.get::<_, Option<String>>(0))
            } else {
                None
            }
        };

        result.push(ExpansionContains {
            system: system_url.clone(),
            version: None,
            code: code.clone(),
            display,
            is_abstract: None,
            inactive: None,
            designations: vec![],
            properties: vec![],
            extensions: vec![],
            contains: vec![],
        });
    }

    Ok(Some((result, total)))
}

/// Only fires when `cap_enforced` (i.e. `req.count.is_none()` — see
/// `expand()` for why count-bounded requests skip the cap to match SQLite's
/// behaviour). `implicit` toggles the diagnostic message phrasing to match
/// the prior call-site error texts byte-for-byte.
fn enforce_cap(
    codes: &[ExpansionContains],
    cap_enforced: bool,
    max_expansion_size: Option<u32>,
    implicit: bool,
) -> Result<(), HtsError> {
    if !cap_enforced {
        return Ok(());
    }
    if let Some(limit) = max_expansion_size {
        if codes.len() as u64 > u64::from(limit) {
            let prefix = if implicit {
                "Implicit ValueSet expansion"
            } else {
                "ValueSet expansion"
            };
            return Err(HtsError::TooCostly(format!(
                "{prefix} contains {} codes which exceeds \
                     the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                codes.len(),
                limit
            )));
        }
    }
    Ok(())
}

/// Fast-path cache read — looks up `key` under the HashMap's read lock and
/// returns the cached `Arc<Vec<…>>` only when the cell exists AND has been
/// fully initialized. Never creates a new cell. Used to skip pool
/// acquisition on warm hits.
///
/// **Why this matters:** the bench dispatches 50 VUs at vu=50 against a
/// 16-connection deadpool. Without this helper, `expand()` acquired a pool
/// connection at the top of the function regardless of whether the cache
/// would hit; 50 VUs would compete for 16 slots even when every request
/// was a sub-millisecond warm hit. The iter-4+5 PG log shows EX01 was at
/// 123 RPS even though the cache was fully populated — the bottleneck was
/// pool contention on warm-hit, not compute.
fn cache_get_initialized(
    cache: &super::ComposeExpansionCache,
    key: u64,
) -> Option<std::sync::Arc<Vec<ExpansionContains>>> {
    let g = cache.read().ok()?;
    let cell = g.get(&key)?;
    cell.get().cloned()
}

/// Fetch (or create) the `OnceCell` for `key`. Returns `None` if the cache
/// is at [`super::PG_COMPOSE_CACHE_MAX`] capacity and the key isn't already
/// present — callers fall back to an unprotected compute in that branch.
///
/// Single-flight pattern: the cell is the shared rendezvous point. The first
/// caller to invoke `get_or_try_init` runs the compute closure; subsequent
/// callers race onto the same cell and `await` its result, then clone the
/// inner `Arc` in O(1).
fn cache_cell(
    cache: &super::ComposeExpansionCache,
    key: u64,
) -> Option<std::sync::Arc<tokio::sync::OnceCell<std::sync::Arc<Vec<ExpansionContains>>>>> {
    // Fast path: read lock, look up; clone Arc on hit.
    if let Ok(g) = cache.read() {
        if let Some(cell) = g.get(&key) {
            return Some(std::sync::Arc::clone(cell));
        }
    }
    // Slow path: write lock, re-check then insert empty cell.
    let mut g = cache.write().ok()?;
    if let Some(cell) = g.get(&key) {
        return Some(std::sync::Arc::clone(cell));
    }
    if g.len() >= super::PG_COMPOSE_CACHE_MAX {
        // At cap: silently bypass single-flight (cold-miss VU runs unprotected).
        // Matches the prior cache_put behaviour — warm set wins.
        return None;
    }
    let cell = std::sync::Arc::new(tokio::sync::OnceCell::new());
    g.insert(key, std::sync::Arc::clone(&cell));
    Some(cell)
}

/// Apply request-level filter (substring on code|display) + offset + count to
/// a cached, unfiltered code list. Returns a fresh `ExpandResponse` whose
/// `contains` holds at most `count` entries.
fn serve_from_cached(codes: &[ExpansionContains], req: &ExpandRequest) -> ExpandResponse {
    let filter_lower = req.filter.as_deref().map(str::to_lowercase);
    let offset = req.offset.unwrap_or(0) as usize;
    let count = req.count.map(|c| c as usize).unwrap_or(usize::MAX);

    if let Some(lower) = filter_lower.as_deref() {
        // Two-pass: count total matching, then page. Single allocator pass via
        // an iterator over indices keeps memory bounded by `count`.
        let mut total: u32 = 0;
        let mut page: Vec<ExpansionContains> = Vec::new();
        let mut seen: usize = 0;
        for c in codes.iter() {
            let m = c.code.to_lowercase().contains(lower)
                || c.display
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(lower))
                    .unwrap_or(false);
            if m {
                total = total.saturating_add(1);
                if seen >= offset && page.len() < count {
                    page.push(c.clone());
                }
                seen += 1;
            }
        }
        ExpandResponse {
            total: Some(total),
            offset: req.offset,
            contains: page,
            warnings: vec![],
        }
    } else {
        let total = codes.len() as u32;
        let page: Vec<ExpansionContains> = codes.iter().skip(offset).take(count).cloned().collect();
        ExpandResponse {
            total: Some(total),
            offset: req.offset,
            contains: page,
            warnings: vec![],
        }
    }
}
