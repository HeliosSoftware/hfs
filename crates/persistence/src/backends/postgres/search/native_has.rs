//! The one reverse-chain (`_has`) shape PostgreSQL answers natively (#1579).
//!
//! Every other chained and `_has` search is resolved by the shared
//! application-side resolver (`crate::search::chain_resolver`) before
//! `search()` runs. This module recognises a single measured shape,
//! `Patient?_has:Observation:patient:code=system|code`, and builds its
//! predicate, which `search()` ANDs into the ordinary query when
//! [`SearchProvider::supports_native_reverse_chains`] says so.
//!
//! [`SearchProvider::supports_native_reverse_chains`]: crate::core::SearchProvider::supports_native_reverse_chains

use crate::search::{IMPLICIT_TOKEN_SYSTEM, SearchParameterRegistry};
use crate::types::{ContainedMode, SearchParamType, SearchPrefix, SearchQuery};

use super::query_builder::{SqlFragment, SqlParam};

/// The measured, one-hop Patient `_has` shape. The exact applicable
/// expressions matter: tenant overrides can keep the parameter code and URL
/// while changing what gets extracted.
pub(crate) fn native_patient_code_token(
    query: &SearchQuery,
    registry: &SearchParameterRegistry,
) -> Option<(String, String)> {
    if query.resource_type != "Patient"
        || query.contained != ContainedMode::Off
        || query.compartment.is_some()
        || !query.list.is_empty()
        || !query.sort.is_empty()
        || !query.includes.is_empty()
        || query.parameters.iter().any(|param| {
            param.name != "_id"
                || param.param_type != SearchParamType::Token
                || param.modifier.is_some()
                || !param.chain.is_empty()
                || !param.components.is_empty()
                || param
                    .values
                    .iter()
                    .any(|value| value.prefix != SearchPrefix::Eq)
        })
    {
        return None;
    }
    let [reverse] = query.reverse_chains.as_slice() else {
        return None;
    };
    if reverse.source_type != "Observation"
        || reverse.reference_param != "patient"
        || reverse.search_param != "code"
        || reverse.modifier.is_some()
        || reverse.nested.is_some()
    {
        return None;
    }
    let value = reverse.value.as_ref()?;
    if value.prefix != SearchPrefix::Eq
        || value.value.contains(['\\', ','])
        || value.value.chars().any(char::is_whitespace)
    {
        return None;
    }
    let mut parts = value.value.split('|');
    let (system, code) = (parts.next()?, parts.next()?);
    if system.is_empty() || code.is_empty() || parts.next().is_some() {
        return None;
    }
    for (name, param_type, expression) in [
        (
            "patient",
            SearchParamType::Reference,
            "Observation.subject.where(resolve() is Patient)",
        ),
        ("code", SearchParamType::Token, "Observation.code"),
    ] {
        let definition = registry.get_param("Observation", name)?;
        if definition.param_type != param_type
            || (name == "patient"
                && !definition.target.as_ref().is_some_and(|targets| {
                    targets.len() == 2
                        && targets.iter().any(|target| target == "Patient")
                        && targets.iter().any(|target| target == "Group")
                }))
            || !core_observation_expression(&definition.expression, expression)
        {
            return None;
        }
    }
    Some((system.to_string(), code.to_string()))
}

fn core_observation_expression(expression: &str, expected: &str) -> bool {
    let mut applicable = 0;
    for member in crate::search::extractor::split_union_members(expression) {
        let unwrapped = member.trim_start_matches('(');
        let Some((resource_type, _)) = unwrapped.split_once('.') else {
            return false;
        };
        if matches!(resource_type, "Resource" | "DomainResource") {
            return false;
        }
        if resource_type == "Observation" {
            if member != expected {
                return false;
            }
            applicable += 1;
        } else if !resource_type.starts_with(char::is_uppercase)
            || !resource_type.chars().all(|c| c.is_ascii_alphanumeric())
        {
            // Unprefixed paths and complex expressions need the extractor.
            return false;
        }
    }
    applicable == 1
}

/// Fence the resource-scoped code slice before filtering the terminal token,
/// then use its qualified id to drive the live source/body guard. All qualifying
/// ids equal source.id, so LIMIT 1 also bounds lookups for duplicate index rows.
/// The JSON reference preserves the shared resolver's relative-reference semantics.
pub(crate) fn native_patient_code_predicate(
    system: &str,
    code: &str,
    offset: usize,
) -> SqlFragment {
    native_patient_code_predicate_with_guard(system, code, native_patient_code_source_guard(offset))
}

/// A broad exact count visits every Patient's references, unlike a limited page
/// or an `_id` intersection. Reject code-negative sources before opening their
/// exact code slices. An uncorrelated scalar subquery becomes one InitPlan;
/// unlike `IN`, it cannot fall back to rescanning candidates at low work_mem.
///
/// The map is only a prefilter. Hash collisions and contained/stale index rows
/// can admit extra sources, which the shared exact guard must still recheck.
/// Limit input rows, including duplicates, and use bounded-length hash keys so
/// neither ID length nor token density makes the aggregate grow without bound.
/// Overflow returns NULL and disables only the prefilter; an empty map rejects
/// every source. `tenant_param` names an existing bound tenant, adding no value.
pub(crate) fn native_patient_code_count_predicate(
    system: &str,
    code: &str,
    offset: usize,
    tenant_param: usize,
) -> SqlFragment {
    const MAX_CANDIDATES: usize = 262_144;
    let guard = native_patient_code_source_guard(offset);
    let membership = format!(
        "(SELECT CASE WHEN count(*) > {MAX_CANDIDATES} THEN NULL::jsonb \
         ELSE COALESCE(jsonb_object_agg(candidate.key, TRUE), '{{}}'::jsonb) END \
         FROM (SELECT hashtext(resource_id)::text AS key FROM search_index code_candidates \
         WHERE code_candidates.tenant_id = ${tenant_param} AND code_candidates.resource_type = 'Observation' \
         AND code_candidates.param_name = 'code' \
         AND code_candidates.value_token_system IN (${}, '{IMPLICIT_TOKEN_SYSTEM}') \
         AND code_candidates.value_token_code = ${} LIMIT {}) candidate)",
        offset + 1,
        offset + 2,
        MAX_CANDIDATES + 1,
    );
    native_patient_code_predicate_with_guard(
        system,
        code,
        format!(
            "CASE WHEN COALESCE({membership} ? hashtext(source.id)::text, TRUE) \
             THEN {guard} ELSE FALSE END"
        ),
    )
}

fn native_patient_code_predicate_with_guard(
    system: &str,
    code: &str,
    guard: String,
) -> SqlFragment {
    SqlFragment::with_params(
        format!(
            "EXISTS (WITH referenced_observations AS MATERIALIZED \
             (SELECT patient.resource_id AS id FROM search_index patient \
             WHERE patient.tenant_id = resources.tenant_id AND patient.resource_type = 'Observation' \
             AND patient.param_name = 'patient' AND patient.is_contained = FALSE \
             AND patient.value_reference = 'Patient/' || resources.id) \
             SELECT 1 FROM referenced_observations source WHERE {guard})",
        ),
        vec![SqlParam::text(system), SqlParam::text(code)],
    )
}

fn native_patient_code_source_guard(offset: usize) -> String {
    format!(
        "EXISTS (WITH scoped_code AS MATERIALIZED \
         (SELECT resource_id, value_token_system, value_token_code FROM search_index \
         WHERE tenant_id = resources.tenant_id AND resource_type = 'Observation' \
         AND resource_id = source.id AND param_name = 'code' AND is_contained = FALSE) \
         SELECT 1 FROM (SELECT resource_id FROM scoped_code \
         WHERE value_token_system IN (${}, '{IMPLICIT_TOKEN_SYSTEM}') AND value_token_code = ${} LIMIT 1) matched \
         JOIN resources observation ON observation.tenant_id = resources.tenant_id \
         AND observation.resource_type = 'Observation' AND observation.id = matched.resource_id \
         WHERE observation.is_deleted = FALSE \
         AND observation.data #>> '{{subject,reference}}' = 'Patient/' || resources.id)",
        offset + 1,
        offset + 2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchParameterDefinition;
    use crate::types::{ReverseChainedParameter, SearchValue};

    fn native_1579_query() -> SearchQuery {
        let mut query = SearchQuery::new("Patient");
        query.reverse_chains.push(ReverseChainedParameter::terminal(
            "Observation",
            "patient",
            "code",
            SearchValue::eq("http://loinc.org|8302-2"),
        ));
        query
    }

    #[cfg(feature = "R4")]
    fn native_1579_registry() -> SearchParameterRegistry {
        let mut registry = SearchParameterRegistry::new();
        let loader = crate::search::SearchParameterLoader::new(helios_fhir::FhirVersion::R4);
        for definition in loader
            .load_from_spec_file(
                &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"),
            )
            .unwrap()
        {
            registry.register(definition).unwrap();
        }
        registry
    }

    #[cfg(feature = "R4")]
    #[test]
    fn native_1579_uses_effective_core_expression_and_bounded_shape() {
        let registry = native_1579_registry();
        let query = native_1579_query();
        assert_eq!(
            native_patient_code_token(&query, &registry),
            Some(("http://loinc.org".into(), "8302-2".into()))
        );
        let mut paged = query.clone();
        paged.count = Some(1);
        paged.offset = Some(1);
        paged.cursor = Some("cursor".into());
        paged.total = Some(crate::types::TotalMode::Accurate);
        paged.summary = Some(crate::types::SummaryMode::Count);
        paged.parameters.push(crate::types::SearchParameter {
            name: "_id".into(),
            param_type: SearchParamType::Token,
            modifier: None,
            chain: vec![],
            components: vec![],
            values: vec![SearchValue::eq("patient")],
        });
        assert!(native_patient_code_token(&paged, &registry).is_some());
        let mut unsupported = vec![];
        for value in [
            "8302-2",
            "|8302-2",
            "http://loinc.org|",
            "a|b,c",
            "a|b|c",
            "a|b\\,c",
            " a|b",
        ] {
            let mut q = query.clone();
            q.reverse_chains[0].value = Some(SearchValue::eq(value));
            unsupported.push(q);
        }
        let mut q = query.clone();
        q.reverse_chains[0].modifier = Some(crate::types::SearchModifier::Not);
        unsupported.push(q);
        let mut q = query.clone();
        q.reverse_chains[0].reference_param = "subject".into();
        unsupported.push(q);
        let mut q = query.clone();
        q.reverse_chains.push(q.reverse_chains[0].clone());
        unsupported.push(q);
        let mut q = query.clone();
        q.reverse_chains[0].nested = Some(Box::new(q.reverse_chains[0].clone()));
        unsupported.push(q);
        let mut q = query.clone();
        q.contained = ContainedMode::Both;
        unsupported.push(q);
        let mut q = query.clone();
        q.list.push("list".into());
        unsupported.push(q);
        let mut q = paged.clone();
        q.parameters[0].name = "identifier".into();
        unsupported.push(q);
        for q in unsupported {
            assert!(native_patient_code_token(&q, &registry).is_none(), "{q:?}");
        }
        for expression in [
            "Observation.component.code",
            "Observation.code | Observation.component.code",
            "Observation.code | Resource.meta.tag",
            "Observation.code | DomainResource.extension.value",
            "code",
            "Observation.code.where(text = 'Observation.code')",
        ] {
            let mut overridden = native_1579_registry();
            overridden
                .register(
                    SearchParameterDefinition::new(
                        "http://example.org/native-code",
                        "code",
                        SearchParamType::Token,
                        expression,
                    )
                    .with_base(["Observation"])
                    .with_source(crate::search::SearchParameterSource::Stored),
                )
                .unwrap();
            assert!(
                native_patient_code_token(&query, &overridden).is_none(),
                "{expression}"
            );
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn native_1579_rejects_patient_target_and_expression_overrides() {
        let query = native_1579_query();
        for (expression, targets) in [
            (
                "Observation.subject.where(resolve() is Patient)",
                vec!["Group"],
            ),
            (
                "Observation.subject.where(resolve() is Patient)",
                vec!["Patient"],
            ),
            (
                "Observation.subject.where(resolve() is Patient)",
                vec!["Patient", "Group", "Encounter"],
            ),
            (
                "Observation.performer.where(resolve() is Patient)",
                vec!["Patient", "Group"],
            ),
            (
                "Observation.subject.where(resolve() is Patient) | Observation.performer",
                vec!["Patient", "Group"],
            ),
        ] {
            let mut registry = native_1579_registry();
            registry
                .register(
                    SearchParameterDefinition::new(
                        "http://example.org/native-patient",
                        "patient",
                        SearchParamType::Reference,
                        expression,
                    )
                    .with_base(["Observation"])
                    .with_targets(targets)
                    .with_source(crate::search::SearchParameterSource::Stored),
                )
                .unwrap();
            assert!(
                native_patient_code_token(&query, &registry).is_none(),
                "{expression}"
            );
        }
    }

    #[test]
    fn native_1579_predicate_scopes_sources_and_preserves_binds() {
        for offset in [2, 4, 7] {
            let fragment = native_patient_code_predicate("http://loinc.org", "8302-2", offset);
            assert_eq!(
                placeholders(&fragment.sql)
                    .into_iter()
                    .filter(|n| *n > 2)
                    .collect::<Vec<_>>(),
                vec![offset + 1, offset + 2]
            );
            assert_eq!(
                format!("{:?}", fragment.params),
                format!(
                    "{:?}",
                    vec![SqlParam::text("http://loinc.org"), SqlParam::text("8302-2")]
                )
            );
            assert!(fragment.sql.contains("is_contained = FALSE"));
            assert!(fragment.sql.contains("observation.is_deleted = FALSE"));
            assert!(fragment.sql.contains(
                "observation.data #>> '{subject,reference}' = 'Patient/' || resources.id"
            ));
            assert!(!fragment.sql.contains("http://loinc.org"));
        }
    }

    #[test]
    fn native_2075_count_reuses_tenant_and_token_binds_with_exact_guard() {
        for (tenant_param, offset) in [(1, 2), (3, 4), (1, 7)] {
            let page = native_patient_code_predicate("a'|$9", "b'|$8", offset);
            let count = native_patient_code_count_predicate("a'|$9", "b'|$8", offset, tenant_param);
            assert_eq!(
                placeholders(&count.sql),
                [tenant_param, offset + 1, offset + 2]
            );
            assert_eq!(format!("{:?}", page.params), format!("{:?}", count.params));
            assert!(!count.sql.contains("a'|$9") && !count.sql.contains("b'|$8"));
            let exact_guard = native_patient_code_source_guard(offset);
            assert!(page.sql.contains(&exact_guard));
            assert!(
                count
                    .sql
                    .contains(&format!("THEN {exact_guard} ELSE FALSE END"))
            );
            assert!(!page.sql.contains("code_candidates"));
        }
    }

    #[test]
    fn native_2075_count_bounds_input_and_keeps_prefilter_a_superset() {
        let fragment = native_patient_code_count_predicate("system", "code", 2, 1);
        assert!(fragment.sql.contains("count(*) > 262144 THEN NULL::jsonb"));
        assert!(fragment.sql.contains("LIMIT 262145"));
        assert!(fragment.sql.contains("hashtext(resource_id)::text AS key"));
        assert!(
            fragment
                .sql
                .contains("jsonb_object_agg(candidate.key, TRUE), '{}'::jsonb")
        );
        assert!(fragment.sql.contains("? hashtext(source.id)::text, TRUE)"));
        let candidate_sql = fragment
            .sql
            .split("FROM search_index code_candidates")
            .nth(1)
            .unwrap()
            .split(") candidate")
            .next()
            .unwrap();
        assert!(candidate_sql.contains("code_candidates.tenant_id = $1"));
        assert!(!candidate_sql.contains("is_contained"));
        assert!(!candidate_sql.contains("DISTINCT"));
    }

    /// The `$N` placeholders a fragment uses, in order of first appearance.
    fn placeholders(sql: &str) -> Vec<usize> {
        let mut seen = Vec::new();
        for (i, _) in sql.match_indices('$') {
            let digits: String = sql[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<usize>() {
                if !seen.contains(&n) {
                    seen.push(n);
                }
            }
        }
        seen
    }
}
