//! Backend-agnostic resolution of chained and reverse-chained search.
//!
//! FHIR chained search (`Observation?subject.name=Smith`) and reverse chaining
//! (`Patient?_has:Observation:subject:code=1234-5`) require joins that the
//! per-backend `SearchProvider::search` does not perform. This module resolves
//! them by issuing iterative *plain* searches against any `SearchProvider` —
//! application-side joins — and rewrites the query into an `_id` filter that any
//! backend can execute. The cost is proportional to the chain depth, not the
//! intermediate fan-out (each hop is one search whose multi-value `OR` is
//! applied natively by the backend).

use std::collections::HashSet;

use crate::core::SearchProvider;
use crate::error::StorageResult;
use crate::tenant::TenantContext;
use crate::types::{
    ReverseChainedParameter, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

use super::{IndexValue, SearchParameterExtractor, resolve_param_type};

/// Returns true if the query contains any chained or reverse-chained parameter.
pub fn query_has_chains(query: &SearchQuery) -> bool {
    !query.reverse_chains.is_empty() || query.parameters.iter().any(|p| !p.chain.is_empty())
}

/// Resolves a query's chained and reverse-chained parameters into an `_id`
/// filter, returning a rewritten query that a plain `search()` can execute.
///
/// Multiple chains are intersected (`AND`). If any chain resolves to no
/// resources the rewritten query is forced to match nothing. Queries without
/// chains are returned unchanged.
pub async fn resolve_chains<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<SearchQuery>
where
    S: SearchProvider + ?Sized,
{
    if !query_has_chains(query) {
        return Ok(query.clone());
    }

    let base_type = query.resource_type.clone();
    let mut id_sets: Vec<HashSet<String>> = Vec::new();

    for param in &query.parameters {
        if param.chain.is_empty() {
            continue;
        }
        let chain = reconstruct_chain_string(param);
        let value = param
            .values
            .first()
            .map(|v| v.value.clone())
            .unwrap_or_default();
        let ids = resolve_forward_chain(storage, tenant, &base_type, &chain, &value).await?;
        id_sets.push(ids.into_iter().collect());
    }

    let max_reverse_depth = crate::types::ChainConfig::default().max_reverse_depth;
    for reverse_chain in &query.reverse_chains {
        if reverse_chain.depth() > max_reverse_depth {
            return Err(crate::error::StorageError::Search(
                crate::error::SearchError::QueryParseError {
                    message: format!(
                        "_has nesting depth {} exceeds the maximum of {}",
                        reverse_chain.depth(),
                        max_reverse_depth
                    ),
                },
            ));
        }
        let ids = resolve_reverse_chain(storage, tenant, &base_type, reverse_chain).await?;
        id_sets.push(ids.into_iter().collect());
    }

    let matched_ids = intersect(id_sets);

    // Rewrite: keep non-chained params, drop chained params + reverse chains,
    // and add an `_id` filter for the resolved base-resource ids.
    let mut rewritten = query.clone();
    rewritten.parameters.retain(|p| p.chain.is_empty());
    rewritten.reverse_chains.clear();

    let id_values: Vec<SearchValue> = if matched_ids.is_empty() {
        // Sentinel that cannot match a real id, forcing an empty result.
        vec![SearchValue::eq("__chained_search_no_match__")]
    } else {
        matched_ids.iter().map(SearchValue::eq).collect()
    };
    rewritten.parameters.push(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: id_values,
        chain: vec![],
        components: vec![],
    });

    Ok(rewritten)
}

/// Reconstructs the dotted chain string (`subject.organization.name`) from a
/// parameter's structured chain links.
fn reconstruct_chain_string(param: &SearchParameter) -> String {
    let mut parts = vec![param.name.clone()];
    for link in &param.chain {
        parts.push(link.target_param.clone());
    }
    parts.join(".")
}

/// Intersects a list of id sets (`AND` across chains). An empty input means no
/// chains contributed, which should not happen here, but yields an empty set.
fn intersect(mut sets: Vec<HashSet<String>>) -> HashSet<String> {
    let mut iter = sets.drain(..);
    let mut acc = match iter.next() {
        Some(s) => s,
        None => return HashSet::new(),
    };
    for s in iter {
        acc.retain(|id| s.contains(id));
    }
    acc
}

/// Resolves a forward chain (e.g. `subject.organization.name=Hospital`) to a
/// set of `base_type` resource ids, walking from the deepest target back out.
async fn resolve_forward_chain<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    chain: &str,
    value: &str,
) -> StorageResult<Vec<String>>
where
    S: SearchProvider + ?Sized,
{
    let parts: Vec<&str> = chain.split('.').collect();
    if parts.len() < 2 {
        return Ok(Vec::new());
    }

    // Per-link target types, resolved from the registry (single-target params)
    // with a heuristic fallback for ambiguous references.
    let target_types: Vec<String> = {
        let registry = storage.search_param_registry().read();
        let mut types = Vec::with_capacity(parts.len() - 1);
        let mut current = base_type.to_string();
        for ref_param in parts.iter().take(parts.len() - 1) {
            let next = registry
                .get_param(&current, ref_param)
                .and_then(|def| {
                    def.target.as_ref().and_then(|t| {
                        if t.len() == 1 {
                            Some(t[0].clone())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| infer_target_type(ref_param));
            types.push(next.clone());
            current = next;
        }
        types
    };

    // Deepest hop: search the terminal target type for the terminal param.
    let terminal_param = parts[parts.len() - 1];
    let deepest_type = target_types.last().map(String::as_str).unwrap_or(base_type);
    let terminal_type = {
        let registry = storage.search_param_registry().read();
        resolve_param_type(
            &registry,
            deepest_type,
            terminal_param,
            &[SearchValue::eq(value)],
        )
    };
    let terminal_query = SearchQuery::new(deepest_type).with_parameter(SearchParameter {
        name: terminal_param.to_string(),
        param_type: terminal_type,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    });
    let result = storage.search(tenant, &terminal_query).await?;
    let mut current_refs: Vec<String> = result
        .resources
        .items
        .into_iter()
        .map(|r| format!("{}/{}", r.resource_type(), r.id()))
        .collect();
    if current_refs.is_empty() {
        return Ok(Vec::new());
    }

    // Walk back out: for each reference hop, find parents pointing at the refs.
    for i in (0..parts.len() - 1).rev() {
        let ref_param = parts[i];
        let parent_type = if i == 0 {
            base_type
        } else {
            &target_types[i - 1]
        };
        let values: Vec<SearchValue> = current_refs.iter().map(SearchValue::eq).collect();
        let query = SearchQuery::new(parent_type).with_parameter(SearchParameter {
            name: ref_param.to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        });
        let r = storage.search(tenant, &query).await?;
        current_refs = r
            .resources
            .items
            .into_iter()
            .map(|res| {
                if i == 0 {
                    res.id().to_string()
                } else {
                    format!("{}/{}", res.resource_type(), res.id())
                }
            })
            .collect();
        if current_refs.is_empty() {
            return Ok(Vec::new());
        }
    }

    Ok(current_refs)
}

/// Resolves a reverse chain (`_has:Source:refParam:searchParam=value`) to a set
/// of `base_type` ids by finding matching source resources and collecting the
/// references they make to `base_type`.
///
/// Nested `_has` (`_has:Source:refParam:_has:...`) is resolved recursively: the
/// inner chain selects the qualifying `Source` resources by id, then this level
/// collects the references those resources make to `base_type`.
async fn resolve_reverse_chain<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    reverse_chain: &ReverseChainedParameter,
) -> StorageResult<Vec<String>>
where
    S: SearchProvider + ?Sized,
{
    // Build a query selecting the matching `source_type` resources.
    let source_query = if let Some(inner) = &reverse_chain.nested {
        // Nested: the inner chain decides which source resources qualify. Its
        // base type is *this* level's source type.
        let inner_ids = Box::pin(resolve_reverse_chain(
            storage,
            tenant,
            &reverse_chain.source_type,
            inner,
        ))
        .await?;
        if inner_ids.is_empty() {
            return Ok(Vec::new());
        }
        SearchQuery::new(&reverse_chain.source_type).with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: inner_ids.iter().map(SearchValue::eq).collect(),
            chain: vec![],
            components: vec![],
        })
    } else {
        // Terminal: match source resources by `search_param=value`.
        let values = match &reverse_chain.value {
            Some(v) => vec![v.clone()],
            None => vec![],
        };
        let search_param_type = {
            let registry = storage.search_param_registry().read();
            resolve_param_type(
                &registry,
                &reverse_chain.source_type,
                &reverse_chain.search_param,
                &values,
            )
        };
        SearchQuery::new(&reverse_chain.source_type).with_parameter(SearchParameter {
            name: reverse_chain.search_param.clone(),
            param_type: search_param_type,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        })
    };

    let result = storage.search(tenant, &source_query).await?;

    let extractor = SearchParameterExtractor::new(storage.search_param_registry().clone());
    let mut ids = Vec::new();
    for resource in result.resources.items {
        let refs = extract_references(
            &extractor,
            storage,
            &resource,
            &reverse_chain.reference_param,
        );
        for reference in refs {
            if let Some((ref_type, ref_id)) = reference.split_once('/') {
                if ref_type == base_type {
                    ids.push(ref_id.to_string());
                }
            }
        }
    }
    Ok(ids)
}

/// Extracts reference values for `search_param` from a resource, using the
/// registry's FHIRPath expression when the parameter is registered.
fn extract_references<S>(
    extractor: &SearchParameterExtractor,
    storage: &S,
    resource: &crate::types::StoredResource,
    search_param: &str,
) -> Vec<String>
where
    S: SearchProvider + ?Sized,
{
    let content = resource.content();
    let resource_type = resource.resource_type();

    let registered = {
        let registry = storage.search_param_registry().read();
        registry
            .get_param(resource_type, search_param)
            .or_else(|| registry.get_param("Resource", search_param))
    };

    if let Some(param_def) = registered {
        if let Ok(values) = extractor.extract_for_param(content, &param_def) {
            return values
                .into_iter()
                .filter_map(|v| match v.value {
                    IndexValue::Reference { reference, .. } => Some(reference),
                    _ => None,
                })
                .collect();
        }
    }
    Vec::new()
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantId, TenantPermissions};
    use crate::types::ChainedParameter;
    use helios_fhir::FhirVersion;
    use serde_json::json;
    use std::path::PathBuf;

    fn backend() -> SqliteBackend {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap();
        let config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let b = SqliteBackend::with_config(":memory:", config).unwrap();
        b.init_schema().unwrap();
        b
    }

    async fn seed(b: &SqliteBackend, tenant: &TenantContext) {
        for (id, family) in [("smith", "Smith"), ("jones", "Jones")] {
            b.create(
                tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": id, "name": [{ "family": family }] }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        }
        b.create(
            tenant,
            "Observation",
            json!({ "resourceType": "Observation", "id": "o1", "status": "final",
                    "subject": { "reference": "Patient/smith" },
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] } }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
        b.create(
            tenant,
            "Observation",
            json!({ "resourceType": "Observation", "id": "o2", "status": "final",
                    "subject": { "reference": "Patient/jones" } }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t"), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn forward_chain_subject_name() {
        let b = backend();
        let t = tenant();
        seed(&b, &t).await;

        // Observation?subject.name=Smith
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
            }],
            components: vec![],
        });

        let rewritten = resolve_chains(&b, &t, &query).await.unwrap();
        let result = b.search(&t, &rewritten).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(ids, vec!["o1"], "only Smith's observation matches");
    }

    #[tokio::test]
    async fn reverse_chain_has() {
        let b = backend();
        let t = tenant();
        seed(&b, &t).await;

        // Patient?_has:Observation:subject:code=8867-4
        let mut query = SearchQuery::new("Patient");
        query.reverse_chains.push(ReverseChainedParameter {
            source_type: "Observation".to_string(),
            reference_param: "subject".to_string(),
            search_param: "code".to_string(),
            value: Some(SearchValue::eq("8867-4")),
            nested: None,
        });

        let rewritten = resolve_chains(&b, &t, &query).await.unwrap();
        let result = b.search(&t, &rewritten).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["smith"],
            "only Smith is referenced by a matching obs"
        );
    }

    #[tokio::test]
    async fn nested_reverse_chain_has() {
        let b = backend();
        let t = tenant();
        seed(&b, &t).await;

        // A Provenance targets Smith's observation (o1) with a known agent.
        b.create(
            &t,
            "Provenance",
            json!({ "resourceType": "Provenance", "id": "prov1",
                    "target": [{ "reference": "Observation/o1" }],
                    "recorded": "2020-01-01T00:00:00Z",
                    "agent": [{ "who": { "reference": "Practitioner/prac-1" } }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

        // Patient?_has:Observation:subject:_has:Provenance:target:agent=Practitioner/prac-1
        // -> patients whose observation is the target of a provenance with that agent.
        let inner = ReverseChainedParameter::terminal(
            "Provenance",
            "target",
            "agent",
            SearchValue::eq("Practitioner/prac-1"),
        );
        let mut query = SearchQuery::new("Patient");
        query.reverse_chains.push(ReverseChainedParameter::nested(
            "Observation",
            "subject",
            inner,
        ));

        let rewritten = resolve_chains(&b, &t, &query).await.unwrap();
        let result = b.search(&t, &rewritten).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["smith"],
            "only Smith's observation is targeted by the matching provenance"
        );
    }

    #[tokio::test]
    async fn no_match_chain_yields_empty() {
        let b = backend();
        let t = tenant();
        seed(&b, &t).await;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Nobody")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
            }],
            components: vec![],
        });
        let rewritten = resolve_chains(&b, &t, &query).await.unwrap();
        let result = b.search(&t, &rewritten).await.unwrap();
        assert!(result.resources.items.is_empty(), "no patient named Nobody");
    }
}

/// Heuristic fallback for ambiguous reference targets, matching the SQLite /
/// PostgreSQL chain builders so all backends agree.
fn infer_target_type(ref_param: &str) -> String {
    match ref_param {
        "patient" | "subject" => "Patient".to_string(),
        "practitioner" | "performer" | "requester" | "author" => "Practitioner".to_string(),
        "organization" | "managingOrganization" | "custodian" => "Organization".to_string(),
        "encounter" | "context" => "Encounter".to_string(),
        "location" => "Location".to_string(),
        "device" => "Device".to_string(),
        "specimen" => "Specimen".to_string(),
        "medication" => "Medication".to_string(),
        "condition" => "Condition".to_string(),
        _ => {
            let mut chars = ref_param.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => ref_param.to_string(),
            }
        }
    }
}
