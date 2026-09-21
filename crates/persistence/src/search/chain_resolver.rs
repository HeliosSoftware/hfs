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
use crate::error::{SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue,
};

use super::{
    IndexValue, SearchParameterExtractor, SearchParameterRegistry, param_requires_terminology,
    parse_typed_values, resolve_param_type, split_unescaped_commas, validate_modifier,
};

/// What a [`TerminologyExpander`] made of a terminal parameter's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminologyExpansion {
    /// The modifier was resolved into a plain token search: the raw value to
    /// search for instead — a comma-separated OR list of `system|code` tokens,
    /// as a client would write it — with the modifier removed.
    Tokens(String),
    /// Nothing to expand (a hierarchy modifier whose value is not a
    /// `system|code`): the terminal search runs with the value and the
    /// modifier as written.
    Unchanged,
    /// The expansion failed and the caller's policy is to go on without the
    /// filter: the whole chained / `_has` parameter is dropped from the search.
    Dropped,
}

/// Expands a terminology-backed modifier (`:in`, token `:above` / `:below`) on
/// the terminal parameter of a chained / `_has` search.
///
/// The resolver is the only place the terminal parameter's type is known, so
/// it is the only place that can tell `subject.gender:in` (valid, to expand)
/// from `subject.name:in` (invalid, a `400`) — a caller rewriting the key
/// before the resolver runs cannot (#1365). It therefore asks the caller, who
/// owns the terminology client, to expand the value once the modifier has been
/// validated against the terminal's type.
#[async_trait::async_trait]
pub trait TerminologyExpander: Send + Sync {
    /// Expands `value` — the parameter's raw value as the client wrote it,
    /// commas included — for `modifier`, which is `:in`, `:above` or `:below`.
    async fn expand(&self, modifier: &SearchModifier, value: &str) -> TerminologyExpansion;
}

/// What the caller can do for the resolver.
#[derive(Clone, Copy, Default)]
pub struct ChainResolveOptions<'a> {
    /// The caller's terminology server, if it has one — the REST search
    /// handler's, when `HFS_TERMINOLOGY_SERVER` is set. A terminology-backed
    /// modifier (`:in`, token `:above` / `:below`) on a chain's terminal
    /// parameter is expanded through it, after the modifier has been checked
    /// against the terminal's type.
    ///
    /// When `None` — the default — such a modifier is rejected with
    /// [`SearchError::TerminologyRequired`], as it is on a direct parameter:
    /// no backend can answer it, and the terminal search would otherwise match
    /// literally or not at all (#1317).
    pub terminology: Option<&'a dyn TerminologyExpander>,
}

impl std::fmt::Debug for ChainResolveOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainResolveOptions")
            .field("terminology", &self.terminology.is_some())
            .finish()
    }
}

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
///
/// Uses the default [`ChainResolveOptions`]; see [`resolve_chains_with`].
pub async fn resolve_chains<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<SearchQuery>
where
    S: SearchProvider + ?Sized,
{
    resolve_chains_with(storage, tenant, query, ChainResolveOptions::default()).await
}

/// [`resolve_chains`], told what the caller can do for it.
pub async fn resolve_chains_with<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
    options: ChainResolveOptions<'_>,
) -> StorageResult<SearchQuery>
where
    S: SearchProvider + ?Sized,
{
    if !query_has_chains(query) {
        return Ok(query.clone());
    }

    let base_type = query.resource_type.clone();
    let mut id_sets: Vec<HashSet<String>> = Vec::new();

    let max_forward_depth = crate::types::ChainConfig::default().max_forward_depth;
    for param in &query.parameters {
        if param.chain.is_empty() {
            continue;
        }
        // Forward-chain depth = number of reference hops. Cap it (mirroring the
        // reverse `_has` cap) so a pathological chain can't fan out unboundedly.
        let depth = param.chain.len();
        if depth > max_forward_depth {
            return Err(crate::error::StorageError::Search(
                crate::error::SearchError::QueryParseError {
                    message: format!(
                        "forward chain depth {} exceeds the maximum of {}",
                        depth, max_forward_depth
                    ),
                },
            ));
        }
        // `None`: the terminal's terminology expansion failed open, and the
        // chain no longer constrains the search.
        if let Some(ids) =
            resolve_forward_chain_opt(storage, tenant, &base_type, param, options).await?
        {
            id_sets.push(ids.into_iter().collect());
        }
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
        if let Some(ids) =
            resolve_reverse_chain_level(storage, tenant, &base_type, reverse_chain, options, "")
                .await?
        {
            id_sets.push(ids.into_iter().collect());
        }
    }

    // Rewrite: keep non-chained params, drop chained params + reverse chains,
    // and add an `_id` filter for the resolved base-resource ids.
    let mut rewritten = query.clone();
    rewritten.parameters.retain(|p| p.chain.is_empty());
    rewritten.reverse_chains.clear();

    // Every chain was dropped (see above): nothing left to filter on.
    if id_sets.is_empty() {
        return Ok(rewritten);
    }
    let matched_ids = intersect(id_sets);

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

/// The resolver's internal page size. A balance between round trips and
/// per-page memory; correctness never depends on it — [`search_all_pages`]
/// drains every page.
const RESOLVER_PAGE: u32 = 1000;

/// Maximum number of reference values passed to a single intermediate hop
/// search. A backend expands a multi-value reference param into an OR of one
/// condition per value; the SQLite builder folds them with a helper that
/// parenthesizes both operands (`(X) OR (Y)`), and SQLite counts each
/// parenthesized expression as its own tree node, so every term adds ~2 to the
/// parse-tree depth — a chain wide enough to reach depth 1000 fails to prepare
/// ("Expression tree is too large", #943). 250 keeps a hop's tree near ~500
/// deep with comfortable margin, no matter how wide the intermediate set is;
/// correctness never depends on the size — the chunks' matches are unioned.
const CHAIN_VALUE_CHUNK: usize = 250;

/// Every match of `query`, across every page. The resolver's intermediate
/// searches feed id rewrites, so letting a backend apply its default page
/// size silently truncated every hop of every chain at 100 matches — both
/// `patient.gender=female` and `=male` answered total=100 on a 21k-row
/// Synthea set (#645).
async fn search_all_pages<S>(
    storage: &S,
    tenant: &TenantContext,
    query: SearchQuery,
) -> StorageResult<Vec<crate::types::StoredResource>>
where
    S: SearchProvider + ?Sized,
{
    let mut items = Vec::new();
    let mut offset: u32 = 0;
    loop {
        let mut page = query.clone().with_count(RESOLVER_PAGE);
        page.offset = Some(offset);
        let result = storage.search(tenant, &page).await?;
        let got = result.resources.items.len();
        items.extend(result.resources.items);
        if got < RESOLVER_PAGE as usize {
            return Ok(items);
        }
        offset += RESOLVER_PAGE;
    }
}

/// Resolves a forward chain (e.g. `subject.organization.name=Hospital`) to a
/// set of `base_type` resource ids, walking from the deepest target back out.
///
/// Target types per hop come from an explicit `:Type` qualifier when the
/// request carried one, else from the registry's declared targets — all of
/// them: FHIR's untyped chain searches every target type, so a multi-target
/// reference like `Patient.general-practitioner` fans out to Practitioner,
/// Organization, and PractitionerRole rather than guessing one. The name
/// heuristic remains only for parameters the registry does not know.
pub(crate) async fn resolve_forward_chain<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    param: &SearchParameter,
    options: ChainResolveOptions<'_>,
) -> StorageResult<Vec<String>>
where
    S: SearchProvider + ?Sized,
{
    // A dropped chain (`None`) has no place in an id list; it takes a
    // terminology expander that fails open to get one.
    resolve_forward_chain_opt(storage, tenant, base_type, param, options)
        .await
        .map(Option::unwrap_or_default)
}

/// [`resolve_forward_chain`], with `None` for a chain whose terminal parameter
/// was dropped by the terminology expander ([`TerminologyExpansion::Dropped`]).
async fn resolve_forward_chain_opt<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    param: &SearchParameter,
    options: ChainResolveOptions<'_>,
) -> StorageResult<Option<Vec<String>>>
where
    S: SearchProvider + ?Sized,
{
    let hops = &param.chain;
    if hops.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let terminal_param = &hops[hops.len() - 1].target_param;

    // Candidate parent/target types per hop, and the terminal param's type
    // and parsed values.
    let (parent_types_per_hop, terminal_types, mut terminal, expand) = {
        let reg = storage.search_param_registry(tenant);
        let registry = reg.read();

        let mut parents: Vec<Vec<String>> = Vec::with_capacity(hops.len());
        let mut current: Vec<String> = vec![base_type.to_string()];
        for hop in hops {
            parents.push(current.clone());
            current = match &hop.target_type {
                Some(t) => vec![t.clone()],
                None => {
                    let mut targets: Vec<String> = Vec::new();
                    for parent in &current {
                        if let Some(def) = registry.get_param(parent, &hop.reference_param) {
                            for t in def.target.as_deref().unwrap_or_default() {
                                if !targets.contains(t) {
                                    targets.push(t.clone());
                                }
                            }
                        }
                    }
                    if targets.is_empty() {
                        vec![infer_target_type(&hop.reference_param)]
                    } else {
                        targets
                    }
                }
            };
        }

        // Only search terminal types that define the terminal param — the
        // others cannot match. Keep the unfiltered set if that empties the
        // list (an unregistered custom param still gets a permissive try).
        let defined: Vec<String> = current
            .iter()
            .filter(|t| registry.get_param(t, terminal_param).is_some())
            .cloned()
            .collect();
        let mut terminal_types = if defined.is_empty() { current } else { defined };
        // An untyped hop over a polymorphic reference can reach target types
        // that type the terminal param differently (`Provenance?target.context`
        // is a token on Questionnaire, a reference on ChargeItem). A modifier is
        // accepted if it is valid on at least one of them, and the terminal
        // search then only covers the types that type the param that way: the
        // others contribute nothing, like the types that do not define the
        // param at all. Invalid everywhere, it is the first type's error — a
        // "needs terminology" (`501`) before an "invalid" (`400`), which is
        // what the client would get by naming the type it is valid on.
        if let Some(m) = param.modifier.as_ref() {
            let type_of = |t: &String| {
                parse_terminal_values(&registry, t, terminal_param, &param.values, Some(m), false).0
            };
            let check = |t: &String| {
                check_terminal_modifier(
                    &registry,
                    t,
                    terminal_param,
                    type_of(t),
                    Some(m),
                    || forward_chain_display(param),
                    options,
                )
            };
            if let Some(accepted) = terminal_types.iter().find(|t| check(t).is_ok()) {
                let accepted_type = type_of(accepted);
                terminal_types.retain(|t| type_of(t) == accepted_type && check(t).is_ok());
            } else if let Some(needs_terminology) = terminal_types.iter().find(|t| {
                matches!(
                    check(t),
                    Err(StorageError::Search(
                        SearchError::TerminologyRequired { .. }
                    ))
                )
            }) {
                terminal_types = vec![needs_terminology.clone()];
            }
        }
        // The chained parameter arrives typed as its *reference* hop, so its
        // values are still raw (already comma-split into the OR list). Only
        // here is the terminal param's type known, so this is where they are
        // parsed — all of them, not just the first (#1292).
        let (terminal_type, terminal_values) = parse_terminal_values(
            &registry,
            &terminal_types[0],
            terminal_param,
            &param.values,
            param.modifier.as_ref(),
            false,
        );
        // Likewise the modifier: a chained parameter's modifier belongs to the
        // terminal param, and only now can it be checked against that type.
        let expand = check_terminal_modifier(
            &registry,
            &terminal_types[0],
            terminal_param,
            terminal_type,
            param.modifier.as_ref(),
            || forward_chain_display(param),
            options,
        )?;
        let terminal = TerminalSearch {
            param_type: terminal_type,
            modifier: param.modifier.clone(),
            values: terminal_values,
        };
        (parents, terminal_types, terminal, expand)
    };
    if let Some(expander) = expand {
        // The values were split into their OR alternatives by the query
        // builder; the expander takes the value as the client wrote it.
        let raw = param
            .values
            .iter()
            .map(|v| v.value.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let expanded = expand_terminal(
            storage,
            tenant,
            expander,
            &terminal_types[0],
            terminal_param,
            &raw,
            &mut terminal,
        )
        .await;
        if !expanded {
            return Ok(None);
        }
    }

    // Deepest hop: search each candidate terminal type, union the refs.
    let mut current_refs: Vec<String> = Vec::new();
    for terminal_target in &terminal_types {
        let terminal_query = SearchQuery::new(terminal_target).with_parameter(SearchParameter {
            name: terminal_param.clone(),
            param_type: terminal.param_type,
            // Set on the terminal search itself, so every check a backend runs
            // on a direct `name:exact` / `birthdate:missing` sees this one too.
            modifier: terminal.modifier.clone(),
            values: terminal.values.clone(),
            chain: vec![],
            components: vec![],
        });
        let items = search_all_pages(storage, tenant, terminal_query).await?;
        current_refs.extend(
            items
                .into_iter()
                .map(|r| format!("{}/{}", r.resource_type(), r.id())),
        );
    }
    if current_refs.is_empty() {
        return Ok(Some(Vec::new()));
    }

    // Walk back out: for each reference hop, find parents pointing at the
    // refs, across every candidate parent type.
    for i in (0..hops.len()).rev() {
        let ref_param = &hops[i].reference_param;
        // Search the refs in chunks. A backend renders a multi-value reference
        // param as an OR of one condition per value, which nests one level per
        // term; past ~1000 terms SQLite refuses to prepare the statement
        // ("Expression tree is too large", #943). Chunking bounds the per-query
        // term count regardless of how wide the intermediate set is; the union
        // of the chunks is the same parent set. Dedup between chunks so a parent
        // referencing refs in two chunks does not inflate the next hop.
        let mut seen: HashSet<String> = HashSet::new();
        let mut next_refs: Vec<String> = Vec::new();
        for parent_type in &parent_types_per_hop[i] {
            for chunk in current_refs.chunks(CHAIN_VALUE_CHUNK) {
                let values: Vec<SearchValue> = chunk.iter().map(SearchValue::eq).collect();
                let query = SearchQuery::new(parent_type).with_parameter(SearchParameter {
                    name: ref_param.clone(),
                    param_type: SearchParamType::Reference,
                    modifier: None,
                    values,
                    chain: vec![],
                    components: vec![],
                });
                let items = search_all_pages(storage, tenant, query).await?;
                for res in items {
                    let r = if i == 0 {
                        res.id().to_string()
                    } else {
                        format!("{}/{}", res.resource_type(), res.id())
                    };
                    if seen.insert(r.clone()) {
                        next_refs.push(r);
                    }
                }
            }
        }
        current_refs = next_refs;
        if current_refs.is_empty() {
            return Ok(Some(Vec::new()));
        }
    }

    Ok(Some(current_refs))
}

/// Resolves a reverse chain (`_has:Source:refParam:searchParam=value`) to a set
/// of `base_type` ids by finding matching source resources and collecting the
/// references they make to `base_type`.
///
/// Nested `_has` (`_has:Source:refParam:_has:...`) is resolved recursively: the
/// inner chain selects the qualifying `Source` resources by id, then this level
/// collects the references those resources make to `base_type`.
pub(crate) async fn resolve_reverse_chain<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    reverse_chain: &ReverseChainedParameter,
    options: ChainResolveOptions<'_>,
) -> StorageResult<Vec<String>>
where
    S: SearchProvider + ?Sized,
{
    // As in `resolve_forward_chain`: `None` takes an expander that fails open.
    resolve_reverse_chain_level(storage, tenant, base_type, reverse_chain, options, "")
        .await
        .map(Option::unwrap_or_default)
}

/// One level of [`resolve_reverse_chain`]. `outer` is the part of the key
/// written before this level — `_has:Encounter:subject:` for the inner level of
/// `_has:Encounter:subject:_has:Observation:encounter:code` — so an error on
/// the terminal parameter can name the whole key, not just its last level.
///
/// `None` when the terminal parameter was dropped by the terminology expander
/// ([`TerminologyExpansion::Dropped`]), at this level or a nested one: the
/// whole `_has` then no longer constrains the search.
async fn resolve_reverse_chain_level<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    reverse_chain: &ReverseChainedParameter,
    options: ChainResolveOptions<'_>,
    outer: &str,
) -> StorageResult<Option<Vec<String>>>
where
    S: SearchProvider + ?Sized,
{
    // This level as written, less what follows the reference parameter.
    let level = format!(
        "{outer}_has:{}:{}:",
        reverse_chain.source_type, reverse_chain.reference_param
    );
    // Build a query selecting the matching `source_type` resources.
    let source_query = if let Some(inner) = &reverse_chain.nested {
        // Nested: the inner chain decides which source resources qualify. Its
        // base type is *this* level's source type.
        let Some(inner_ids) = Box::pin(resolve_reverse_chain_level(
            storage,
            tenant,
            &reverse_chain.source_type,
            inner,
            options,
            &level,
        ))
        .await?
        else {
            return Ok(None);
        };
        if inner_ids.is_empty() {
            return Ok(Some(Vec::new()));
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
        // Terminal: match source resources by `search_param=value`. A `_has`
        // carries its value as one raw string, so the OR list is split here
        // too, then parsed for the terminal param's type (#1292).
        //
        // `search_param` may end in a modifier (`code:not`), which applies to
        // the terminal search as it would to a direct one (#1302).
        let raw: Vec<SearchValue> = reverse_chain.value.iter().cloned().collect();
        let (search_param, modifier) = reverse_chain.terminal_param();
        let modifier = match modifier {
            Some(m) => Some(SearchModifier::parse(m).ok_or_else(|| {
                query_error(format!(
                    "unknown search modifier ':{m}' on _has parameter '{level}{search_param}'"
                ))
            })?),
            None => None,
        };
        let (mut terminal, expand) = {
            let reg = storage.search_param_registry(tenant);
            let registry = reg.read();
            let (search_param_type, values) = parse_terminal_values(
                &registry,
                &reverse_chain.source_type,
                search_param,
                &raw,
                modifier.as_ref(),
                true,
            );
            let expand = check_terminal_modifier(
                &registry,
                &reverse_chain.source_type,
                search_param,
                search_param_type,
                modifier.as_ref(),
                || format!("{level}{search_param}"),
                options,
            )?;
            let terminal = TerminalSearch {
                param_type: search_param_type,
                modifier,
                values,
            };
            (terminal, expand)
        };
        if let Some(expander) = expand {
            let raw_value = reverse_chain
                .value
                .as_ref()
                .map_or("", |v| v.value.as_str());
            let expanded = expand_terminal(
                storage,
                tenant,
                expander,
                &reverse_chain.source_type,
                search_param,
                raw_value,
                &mut terminal,
            )
            .await;
            if !expanded {
                return Ok(None);
            }
        }
        SearchQuery::new(&reverse_chain.source_type).with_parameter(SearchParameter {
            name: search_param.to_string(),
            param_type: terminal.param_type,
            modifier: terminal.modifier,
            values: terminal.values,
            chain: vec![],
            components: vec![],
        })
    };

    let items = search_all_pages(storage, tenant, source_query).await?;

    let extractor = SearchParameterExtractor::new(storage.search_param_registry(tenant));
    let mut ids = Vec::new();
    for resource in items {
        let refs = extract_references(
            &extractor,
            storage,
            tenant,
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
    Ok(Some(ids))
}

/// Parses the values of a chain's terminal parameter exactly as a direct search
/// on that parameter would, via [`parse_typed_values`]: a comparator prefix is
/// split off for date/number/quantity terminals only, so `birthdate=ge1980` is a
/// comparison while `family=Lee` stays the literal "Lee".
///
/// `values` are raw — the query builder cannot type them, since it only knows
/// the reference hop. Forward-chain values arrive already split into their OR
/// alternatives (and unescaped), so `split_commas` is false for them; a `_has`
/// value arrives as a single unsplit string, so it is split here.
///
/// A value that already carries a non-`eq` prefix was parsed by the caller
/// (programmatic queries, or an unregistered reference hop whose date-shaped
/// value the type heuristic claimed) and is passed through untouched.
///
/// With the `:missing` modifier the value is the boolean `true` / `false`
/// whatever the parameter's type, so it is neither prefix-parsed nor split.
fn parse_terminal_values(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    param_name: &str,
    values: &[SearchValue],
    modifier: Option<&SearchModifier>,
    split_commas: bool,
) -> (SearchParamType, Vec<SearchValue>) {
    if matches!(modifier, Some(SearchModifier::Missing))
        || values.iter().any(|v| v.prefix != SearchPrefix::Eq)
    {
        let param_type = resolve_param_type(registry, resource_type, param_name, values);
        return (param_type, values.to_vec());
    }
    let raw_values: Vec<String> = values
        .iter()
        .flat_map(|v| {
            if split_commas {
                split_unescaped_commas(&v.value)
            } else {
                vec![v.value.clone()]
            }
        })
        .collect();
    parse_typed_values(registry, resource_type, param_name, &raw_values)
}

/// Rejects a modifier the terminal parameter cannot take, with the same checks
/// — in the same order, and with the same wording — a direct search on it gets:
///
/// * one the parameter's type does not define ([`validate_modifier`]), which
///   the REST layer maps to a `400`. Checked first, as the REST handler does
///   for a direct parameter: `name:in` is a client error whether or not there
///   is a terminology server to ask (#1339);
/// * one that is valid but needs a terminology server the caller does not have
///   ([`param_requires_terminology`]), which the REST layer maps to a `501`.
///   `:not-in` is always that: negated value-set membership cannot be turned
///   into a token list, so a terminology server does not help.
///
/// Returns the caller's terminology expander when the modifier is valid, needs
/// one and there is one: the terminal's value is then to be expanded with
/// [`expand_terminal`] — once the registry guard this runs under is released.
///
/// `display` names the parameter as the client wrote it, less the modifier —
/// the whole chain or `_has` key, every level of a nested one. Both errors
/// carry it: the terminal parameter's own name alone does not tell the client
/// which part of the request was wrong.
fn check_terminal_modifier<'a>(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    param_name: &str,
    param_type: SearchParamType,
    modifier: Option<&SearchModifier>,
    display: impl Fn() -> String,
    options: ChainResolveOptions<'a>,
) -> StorageResult<Option<&'a dyn TerminologyExpander>> {
    let Some(m) = modifier else {
        return Ok(None);
    };
    validate_modifier(registry, resource_type, param_name, param_type, m)
        .map_err(|message| query_error(format!("{message} (in '{}:{m}')", display())))?;
    if !param_requires_terminology(registry, resource_type, param_name, m) {
        return Ok(None);
    }
    match options.terminology {
        Some(expander) if *m != SearchModifier::NotIn => Ok(Some(expander)),
        _ => Err(StorageError::Search(SearchError::TerminologyRequired {
            modifier: m.to_string(),
            param: display(),
        })),
    }
}

/// The terminal search of a chain: what is searched for on the terminal
/// parameter.
struct TerminalSearch {
    param_type: SearchParamType,
    modifier: Option<SearchModifier>,
    values: Vec<SearchValue>,
}

/// Expands the terminology-backed modifier of `terminal` through `expander`,
/// turning it into the plain token search a direct `code:in` becomes: the
/// expansion is parsed like any terminal value. `raw` is the value as the
/// client wrote it. Returns false when the expander dropped the parameter.
async fn expand_terminal<S>(
    storage: &S,
    tenant: &TenantContext,
    expander: &dyn TerminologyExpander,
    resource_type: &str,
    param_name: &str,
    raw: &str,
    terminal: &mut TerminalSearch,
) -> bool
where
    S: SearchProvider + ?Sized,
{
    let Some(modifier) = terminal.modifier.clone() else {
        return true;
    };
    match expander.expand(&modifier, raw).await {
        TerminologyExpansion::Tokens(tokens) => {
            let reg = storage.search_param_registry(tenant);
            let registry = reg.read();
            let (param_type, values) = parse_terminal_values(
                &registry,
                resource_type,
                param_name,
                &[SearchValue::eq(tokens)],
                None,
                true,
            );
            *terminal = TerminalSearch {
                param_type,
                modifier: None,
                values,
            };
            true
        }
        TerminologyExpansion::Unchanged => true,
        TerminologyExpansion::Dropped => false,
    }
}

/// A forward chain as the client wrote it, less the terminal modifier:
/// `subject:Patient.general-practitioner.name`.
fn forward_chain_display(param: &SearchParameter) -> String {
    let mut path = String::new();
    for hop in &param.chain {
        path.push_str(&hop.reference_param);
        if let Some(t) = &hop.target_type {
            path.push(':');
            path.push_str(t);
        }
        path.push('.');
    }
    if let Some(last) = param.chain.last() {
        path.push_str(&last.target_param);
    }
    path
}

fn query_error(message: String) -> StorageError {
    StorageError::Search(SearchError::QueryParseError { message })
}

/// Extracts reference values for `search_param` from a resource, using the
/// registry's FHIRPath expression when the parameter is registered.
fn extract_references<S>(
    extractor: &SearchParameterExtractor,
    storage: &S,
    tenant: &TenantContext,
    resource: &crate::types::StoredResource,
    search_param: &str,
) -> Vec<String>
where
    S: SearchProvider + ?Sized,
{
    let content = resource.content();
    let resource_type = resource.resource_type();

    let registered = {
        let reg = storage.search_param_registry(tenant);
        let registry = reg.read();
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

    // ---- #1292: the terminal parameter is parsed like a direct parameter ----

    /// Two patients (born 1980-05-06 "Lee" and 1990-01-01 "Gert"), two
    /// Procedures each, plus an Encounter/Observation/DiagnosticReport/
    /// RiskAssessment per patient for the multi-hop, quantity and number cases.
    async fn seed_dated(b: &SqliteBackend, t: &TenantContext) {
        let resources = [
            json!({ "resourceType": "Patient", "id": "p80", "birthDate": "1980-05-06",
                    "name": [{ "family": "Lee" }] }),
            json!({ "resourceType": "Patient", "id": "p90", "birthDate": "1990-01-01",
                    "name": [{ "family": "Gert" }] }),
            json!({ "resourceType": "Procedure", "id": "pr1", "status": "completed",
                    "subject": { "reference": "Patient/p80" },
                    "performedDateTime": "2013-04-05T09:20:00-04:00" }),
            json!({ "resourceType": "Procedure", "id": "pr2", "status": "completed",
                    "subject": { "reference": "Patient/p80" },
                    "performedDateTime": "2013-04-05" }),
            json!({ "resourceType": "Procedure", "id": "pr3", "status": "completed",
                    "subject": { "reference": "Patient/p90" },
                    "performedDateTime": "2020-06-01T10:00:00+05:30" }),
            json!({ "resourceType": "Procedure", "id": "pr4", "status": "completed",
                    "subject": { "reference": "Patient/p90" },
                    "performedDateTime": "2021-02-03" }),
            json!({ "resourceType": "Encounter", "id": "e80", "status": "finished",
                    "class": { "code": "AMB" },
                    "subject": { "reference": "Patient/p80" } }),
            json!({ "resourceType": "Encounter", "id": "e90", "status": "finished",
                    "class": { "code": "AMB" },
                    "subject": { "reference": "Patient/p90" } }),
            json!({ "resourceType": "Observation", "id": "ob80", "status": "final",
                    "code": { "text": "hr" },
                    "subject": { "reference": "Patient/p80" },
                    "encounter": { "reference": "Encounter/e80" },
                    "valueQuantity": { "value": 60, "unit": "bpm" } }),
            json!({ "resourceType": "Observation", "id": "ob90", "status": "final",
                    "code": { "text": "hr" },
                    "subject": { "reference": "Patient/p90" },
                    "encounter": { "reference": "Encounter/e90" },
                    "valueQuantity": { "value": 90, "unit": "bpm" } }),
            json!({ "resourceType": "DiagnosticReport", "id": "dr80", "status": "final",
                    "code": { "text": "panel" },
                    "result": [{ "reference": "Observation/ob80" }] }),
            json!({ "resourceType": "DiagnosticReport", "id": "dr90", "status": "final",
                    "code": { "text": "panel" },
                    "result": [{ "reference": "Observation/ob90" }] }),
            json!({ "resourceType": "RiskAssessment", "id": "ra80", "status": "final",
                    "subject": { "reference": "Patient/p80" },
                    "prediction": [{ "probabilityDecimal": 0.2 }] }),
            json!({ "resourceType": "RiskAssessment", "id": "ra90", "status": "final",
                    "subject": { "reference": "Patient/p90" },
                    "prediction": [{ "probabilityDecimal": 0.8 }] }),
        ];
        for resource in resources {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            b.create(t, &resource_type, resource, FhirVersion::default())
                .await
                .unwrap();
        }
    }

    /// A forward-chained parameter exactly as the REST query builder emits it:
    /// typed as the reference hop, values raw and already comma-split.
    fn forward(base: &str, hops: &[(&str, &str, &str)], raw_values: &[&str]) -> SearchQuery {
        SearchQuery::new(base).with_parameter(SearchParameter {
            name: hops[0].0.to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: raw_values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: hops
                .iter()
                .map(|(reference, target_type, target_param)| ChainedParameter {
                    reference_param: reference.to_string(),
                    target_type: Some(target_type.to_string()),
                    target_param: target_param.to_string(),
                })
                .collect(),
            components: vec![],
        })
    }

    /// A `_has` exactly as the REST query builder emits it: one raw value.
    fn has(base: &str, source: &str, reference: &str, param: &str, raw: &str) -> SearchQuery {
        let mut query = SearchQuery::new(base);
        query.reverse_chains.push(ReverseChainedParameter::terminal(
            source,
            reference,
            param,
            SearchValue::eq(raw),
        ));
        query
    }

    async fn run(b: &SqliteBackend, t: &TenantContext, query: &SearchQuery) -> Vec<String> {
        let rewritten = resolve_chains(b, t, query).await.unwrap();
        let result = b.search(t, &rewritten).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        ids
    }

    fn pairs(values: &[SearchValue]) -> Vec<(SearchPrefix, &str)> {
        values
            .iter()
            .map(|v| (v.prefix, v.value.as_str()))
            .collect()
    }

    const SUBJECT_BIRTHDATE: &[(&str, &str, &str)] = &[("subject", "Patient", "birthdate")];

    #[tokio::test]
    async fn forward_chain_date_prefixes() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Procedure?subject:Patient.birthdate=ge1980-01-01
        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["ge1980-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2", "pr3", "pr4"]);

        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["ge1985-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["pr3", "pr4"]);

        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["lt1985-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);

        // An explicit `eq` is a prefix too, not part of the date.
        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["eq1980-05-06"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);

        // Unprefixed values keep working.
        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["1980-05-06"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);

        // A prefixed value nobody satisfies still yields the empty result.
        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["gt2000-01-01"]);
        assert!(run(&b, &t, &q).await.is_empty());
    }

    #[tokio::test]
    async fn forward_chain_or_list_returns_the_union() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Procedure?subject:Patient.birthdate=1980-05-06,1990-01-01
        let q = forward(
            "Procedure",
            SUBJECT_BIRTHDATE,
            &["1980-05-06", "1990-01-01"],
        );
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2", "pr3", "pr4"]);

        // Prefixes apply per alternative.
        let q = forward("Procedure", SUBJECT_BIRTHDATE, &["lt1970", "ge1990"]);
        assert_eq!(run(&b, &t, &q).await, ["pr3", "pr4"]);
    }

    #[tokio::test]
    async fn forward_chain_quantity_prefix() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // DiagnosticReport?result.value-quantity=gt70
        let hops = &[("result", "Observation", "value-quantity")];
        let q = forward("DiagnosticReport", hops, &["gt70"]);
        assert_eq!(run(&b, &t, &q).await, ["dr90"]);

        let q = forward("DiagnosticReport", hops, &["le60"]);
        assert_eq!(run(&b, &t, &q).await, ["dr80"]);
    }

    #[tokio::test]
    async fn forward_multi_hop_chain_with_prefix() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Observation?encounter.subject.birthdate=ge1985-01-01
        let hops = &[
            ("encounter", "Encounter", "subject"),
            ("subject", "Patient", "birthdate"),
        ];
        let q = forward("Observation", hops, &["ge1985-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["ob90"]);

        let q = forward("Observation", hops, &["lt1985-01-01", "ge1990-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["ob80", "ob90"]);
    }

    #[tokio::test]
    async fn forward_chain_last_updated_prefix() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Procedure?subject:Patient._lastUpdated=ge2000-01-01
        let hops = &[("subject", "Patient", "_lastUpdated")];
        let q = forward("Procedure", hops, &["ge2000-01-01"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2", "pr3", "pr4"]);

        let q = forward("Procedure", hops, &["lt2000-01-01"]);
        assert!(run(&b, &t, &q).await.is_empty());
    }

    /// Prefix parsing is type-aware: a string or token terminal that merely
    /// starts with the letters of a comparator keeps them.
    #[tokio::test]
    async fn string_terminal_starting_with_prefix_letters_is_untouched() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        let hops = &[("subject", "Patient", "family")];
        let q = forward("Procedure", hops, &["Lee"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);

        let q = forward("Procedure", hops, &["gert"]);
        assert_eq!(run(&b, &t, &q).await, ["pr3", "pr4"]);

        // Same for a `_has` token terminal: not `ne` + "cessary".
        let (ty, values) = {
            let reg = b.search_param_registry(&t);
            let registry = reg.read();
            parse_terminal_values(
                &registry,
                "Procedure",
                "status",
                &[SearchValue::eq("necessary")],
                None,
                true,
            )
        };
        assert_eq!(ty, SearchParamType::Token);
        assert_eq!(pairs(&values), [(SearchPrefix::Eq, "necessary")]);
    }

    #[tokio::test]
    async fn has_date_prefix() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Patient?_has:Procedure:subject:date=ge2013-01-01
        let q = has("Patient", "Procedure", "subject", "date", "ge2013-01-01");
        assert_eq!(run(&b, &t, &q).await, ["p80", "p90"]);

        let q = has("Patient", "Procedure", "subject", "date", "ge2020-01-01");
        assert_eq!(run(&b, &t, &q).await, ["p90"]);

        let q = has("Patient", "Procedure", "subject", "date", "lt2014");
        assert_eq!(run(&b, &t, &q).await, ["p80"]);

        let q = has("Patient", "Procedure", "subject", "date", "eq2013-04-05");
        assert_eq!(run(&b, &t, &q).await, ["p80"]);
    }

    #[tokio::test]
    async fn has_or_list() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Patient?_has:Procedure:subject:date=2013-04-05T09:20:00-04:00,2020
        let q = has(
            "Patient",
            "Procedure",
            "subject",
            "date",
            "2013-04-05T09:20:00-04:00,2020",
        );
        assert_eq!(run(&b, &t, &q).await, ["p80", "p90"]);

        // Prefixes apply per alternative.
        let q = has("Patient", "Procedure", "subject", "date", "lt2000,ge2021");
        assert_eq!(run(&b, &t, &q).await, ["p90"]);

        // An escaped comma is part of the value, not an OR separator.
        let (_, values) = {
            let reg = b.search_param_registry(&t);
            let registry = reg.read();
            parse_terminal_values(
                &registry,
                "Patient",
                "family",
                &[SearchValue::eq("Lee\\, Jr,Gert")],
                None,
                true,
            )
        };
        assert_eq!(
            pairs(&values),
            [(SearchPrefix::Eq, "Lee, Jr"), (SearchPrefix::Eq, "Gert")]
        );
    }

    #[tokio::test]
    async fn has_number_and_quantity_prefixes() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Patient?_has:RiskAssessment:subject:probability=gt0.5
        let q = has(
            "Patient",
            "RiskAssessment",
            "subject",
            "probability",
            "gt0.5",
        );
        assert_eq!(run(&b, &t, &q).await, ["p90"]);
        let q = has(
            "Patient",
            "RiskAssessment",
            "subject",
            "probability",
            "le0.5",
        );
        assert_eq!(run(&b, &t, &q).await, ["p80"]);

        // Patient?_has:Observation:subject:value-quantity=gt70
        let q = has(
            "Patient",
            "Observation",
            "subject",
            "value-quantity",
            "gt70",
        );
        assert_eq!(run(&b, &t, &q).await, ["p90"]);
    }

    /// `forward`, with the modifier the REST query builder takes from the
    /// chain's terminal parameter (`subject:Patient.family:exact`).
    fn forward_modified(
        base: &str,
        hops: &[(&str, &str, &str)],
        modifier: SearchModifier,
        raw_values: &[&str],
    ) -> SearchQuery {
        let mut query = forward(base, hops, raw_values);
        query.parameters[0].modifier = Some(modifier);
        query
    }

    /// #1302: a chained parameter's modifier applies to the terminal search.
    #[tokio::test]
    async fn forward_chain_terminal_modifier() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;
        let family = &[("subject", "Patient", "family")];
        let all = ["pr1", "pr2", "pr3", "pr4"];

        // Default string matching is a prefix match; :exact is not.
        let q = forward("Procedure", family, &["Le"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);
        let q = forward_modified("Procedure", family, SearchModifier::Exact, &["Le"]);
        assert!(run(&b, &t, &q).await.is_empty());
        let q = forward_modified("Procedure", family, SearchModifier::Exact, &["Lee"]);
        assert_eq!(run(&b, &t, &q).await, ["pr1", "pr2"]);

        // "er" is a prefix of neither name.
        let q = forward("Procedure", family, &["er"]);
        assert!(run(&b, &t, &q).await.is_empty());
        let q = forward_modified("Procedure", family, SearchModifier::Contains, &["er"]);
        assert_eq!(run(&b, &t, &q).await, ["pr3", "pr4"]);

        // :missing on a date terminal: the value is a boolean, not a date.
        let birthdate = &[("subject", "Patient", "birthdate")];
        let q = forward_modified("Procedure", birthdate, SearchModifier::Missing, &["false"]);
        assert_eq!(run(&b, &t, &q).await, all);
        let q = forward_modified("Procedure", birthdate, SearchModifier::Missing, &["true"]);
        assert!(run(&b, &t, &q).await.is_empty());

        // Multi-hop.
        let hops = &[
            ("encounter", "Encounter", "subject"),
            ("subject", "Patient", "family"),
        ];
        let q = forward_modified("Observation", hops, SearchModifier::Exact, &["Gert"]);
        assert_eq!(run(&b, &t, &q).await, ["ob90"]);
        let q = forward_modified("Observation", hops, SearchModifier::Exact, &["Ger"]);
        assert!(run(&b, &t, &q).await.is_empty());
    }

    /// #1302: `_has:…:param:modifier`, carried in `search_param`.
    #[tokio::test]
    async fn has_terminal_modifier() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Patient?_has:Procedure:subject:status:not=completed
        let q = has("Patient", "Procedure", "subject", "status", "completed");
        assert_eq!(run(&b, &t, &q).await, ["p80", "p90"]);
        let q = has("Patient", "Procedure", "subject", "status:not", "completed");
        assert!(run(&b, &t, &q).await.is_empty());
        let q = has("Patient", "Procedure", "subject", "status:not", "stopped");
        assert_eq!(run(&b, &t, &q).await, ["p80", "p90"]);

        // Patient?_has:Observation:subject:encounter:missing=true
        let q = has(
            "Patient",
            "Observation",
            "subject",
            "encounter:missing",
            "false",
        );
        assert_eq!(run(&b, &t, &q).await, ["p80", "p90"]);
        let q = has(
            "Patient",
            "Observation",
            "subject",
            "encounter:missing",
            "true",
        );
        assert!(run(&b, &t, &q).await.is_empty());
    }

    /// #1302: with `:missing` the value is `true`/`false`, whatever the
    /// terminal's type — it is never prefix-parsed.
    #[tokio::test]
    async fn missing_terminal_value_is_not_prefix_parsed() {
        let b = backend();
        let t = tenant();
        let reg = b.search_param_registry(&t);
        let registry = reg.read();
        for split_commas in [false, true] {
            let (ty, values) = parse_terminal_values(
                &registry,
                "Patient",
                "birthdate",
                &[SearchValue::eq("true")],
                Some(&SearchModifier::Missing),
                split_commas,
            );
            assert_eq!(ty, SearchParamType::Date);
            assert_eq!(pairs(&values), [(SearchPrefix::Eq, "true")]);
        }
    }

    /// #1302: a modifier the terminal's type does not define is the same
    /// error a direct search gets, not a silently unmodified search.
    #[tokio::test]
    async fn terminal_modifier_invalid_for_type_is_rejected() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        let message = |q: SearchQuery| {
            let (b, t) = (&b, &t);
            async move {
                match resolve_chains(b, t, &q).await {
                    Err(StorageError::Search(SearchError::QueryParseError { message })) => message,
                    other => panic!("expected a query parse error, got {other:?}"),
                }
            }
        };

        // Procedure?subject:Patient.birthdate:exact=1980-05-06
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "birthdate")],
            SearchModifier::Exact,
            &["1980-05-06"],
        );
        assert_eq!(
            message(q).await,
            "search modifier ':exact' is not supported for date parameter 'birthdate' \
             (in 'subject:Patient.birthdate:exact')"
        );

        // Patient?_has:Procedure:subject:status:exact=completed
        let q = has(
            "Patient",
            "Procedure",
            "subject",
            "status:exact",
            "completed",
        );
        assert_eq!(
            message(q).await,
            "search modifier ':exact' is not supported for token parameter 'status' \
             (in '_has:Procedure:subject:status:exact')"
        );

        // Not a modifier at all.
        let q = has(
            "Patient",
            "Procedure",
            "subject",
            "status:bogus",
            "completed",
        );
        assert!(
            message(q)
                .await
                .contains("unknown search modifier ':bogus'")
        );
    }

    /// #1317: a terminology-backed modifier on the terminal parameter is
    /// rejected unless the caller has a terminology server — the terminal
    /// search would otherwise run it literally. `:above` / `:below` count only
    /// on a token terminal.
    #[tokio::test]
    async fn terminal_modifier_needing_terminology_is_rejected() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        let required = |q: SearchQuery| {
            let (b, t) = (&b, &t);
            async move {
                match resolve_chains(b, t, &q).await {
                    Err(StorageError::Search(SearchError::TerminologyRequired {
                        modifier,
                        param,
                    })) => (modifier, param),
                    other => panic!("expected a terminology-required error, got {other:?}"),
                }
            }
        };
        let of = |m: &str, p: &str| (m.to_string(), p.to_string());

        // Procedure?subject:Patient.gender:in=… / :below=…
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "gender")],
            SearchModifier::In,
            &["http://example.org/vs"],
        );
        assert_eq!(required(q).await, of("in", "subject:Patient.gender"));
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "gender")],
            SearchModifier::Below,
            &["http://hl7.org/fhir/administrative-gender|male"],
        );
        assert_eq!(required(q).await, of("below", "subject:Patient.gender"));

        // Patient?_has:Procedure:subject:status:in=… / :above=…
        let q = has("Patient", "Procedure", "subject", "status:in", "http://vs");
        assert_eq!(required(q).await, of("in", "_has:Procedure:subject:status"));
        let q = has("Patient", "Procedure", "subject", "status:above", "s|c");
        assert_eq!(
            required(q).await,
            of("above", "_has:Procedure:subject:status")
        );

        // Structural on a reference terminal: never needs terminology.
        // Procedure?subject:Patient.general-practitioner:below=Practitioner/x
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "general-practitioner")],
            SearchModifier::Below,
            &["Practitioner/x"],
        );
        assert!(resolve_chains(&b, &t, &q).await.is_ok());

        // #1339: a terminology modifier the terminal's type does not define is
        // a parse error (`400`), not a missing terminology server (`501`) —
        // with or without one.
        let expander = FakeExpander::new(TerminologyExpansion::Tokens("completed".into()));
        for options in [
            ChainResolveOptions::default(),
            ChainResolveOptions {
                terminology: Some(&expander),
            },
        ] {
            for q in [
                // Procedure?subject:Patient.birthdate:in=… / .family:below=…
                forward_modified(
                    "Procedure",
                    &[("subject", "Patient", "birthdate")],
                    SearchModifier::In,
                    &["http://example.org/vs"],
                ),
                forward_modified(
                    "Procedure",
                    &[("subject", "Patient", "family")],
                    SearchModifier::Below,
                    &["Smith"],
                ),
                has("Patient", "Procedure", "subject", "date:in", "http://vs"),
                has("Patient", "Procedure", "subject", "date:above", "2020"),
            ] {
                match resolve_chains_with(&b, &t, &q, options).await {
                    Err(StorageError::Search(SearchError::QueryParseError { message })) => {
                        assert!(message.contains("is not supported for"), "{message}")
                    }
                    other => panic!("expected a query parse error, got {other:?}"),
                }
            }
        }
        // … and the terminology server is never asked about it (#1365).
        assert_eq!(expander.calls(), vec![]);
    }

    /// #1365: over an untyped polymorphic hop the terminal param can be typed
    /// differently per target type. A modifier valid on at least one of them is
    /// accepted, and only those types are searched.
    #[tokio::test]
    async fn terminal_modifier_valid_on_one_polymorphic_target_is_accepted() {
        let b = backend();
        let t = tenant();
        for resource in [
            json!({ "resourceType": "Questionnaire", "id": "q1", "status": "active",
                    "useContext": [{ "code": { "code": "focus" },
                                     "valueCodeableConcept": { "coding": [
                                         { "system": "http://cs", "code": "focus" }] } }] }),
            json!({ "resourceType": "Provenance", "id": "prov1", "recorded": "2020-01-01T00:00:00Z",
                    "target": [{ "reference": "Questionnaire/q1" }],
                    "agent": [{ "who": { "display": "x" } }] }),
        ] {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            b.create(&t, &resource_type, resource, FhirVersion::default())
                .await
                .unwrap();
        }
        let untyped = |modifier: Option<SearchModifier>, raw: &str| {
            SearchQuery::new("Provenance").with_parameter(SearchParameter {
                name: "target".to_string(),
                param_type: SearchParamType::Reference,
                modifier,
                values: vec![SearchValue::eq(raw)],
                chain: vec![crate::types::ChainedParameter {
                    reference_param: "target".to_string(),
                    target_type: None,
                    target_param: "context".to_string(),
                }],
                components: vec![],
            })
        };

        // Positive control: `context` is a token on Questionnaire …
        assert_eq!(run(&b, &t, &untyped(None, "focus")).await, ["prov1"]);

        // … and a reference on ChargeItem / MedicationStatement, where alone
        // `:identifier` is valid. The first target type defining `context`
        // (ActivityDefinition, a token) used to decide for all of them.
        let q = untyped(Some(SearchModifier::Identifier), "http://ids|e1");
        assert!(run(&b, &t, &q).await.is_empty());

        // `:in` is valid on the token targets: a `501` without terminology,
        // expanded with it.
        let q = untyped(Some(SearchModifier::In), "http://vs");
        assert!(matches!(
            resolve_chains(&b, &t, &q).await,
            Err(StorageError::Search(
                SearchError::TerminologyRequired { .. }
            ))
        ));
        let expander = FakeExpander::new(TerminologyExpansion::Tokens("http://cs|focus".into()));
        assert_eq!(run_with(&b, &t, &q, &expander).await.unwrap(), ["prov1"]);
        assert_eq!(expander.calls().len(), 1);

        // Valid nowhere (`:exact` is a string modifier): still a parse error.
        let q = untyped(Some(SearchModifier::Exact), "focus");
        assert!(matches!(
            resolve_chains(&b, &t, &q).await,
            Err(StorageError::Search(SearchError::QueryParseError { .. }))
        ));
    }

    /// A terminology expander answering every call the same way, and recording
    /// what it was asked.
    struct FakeExpander {
        answer: TerminologyExpansion,
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl FakeExpander {
        fn new(answer: TerminologyExpansion) -> Self {
            Self {
                answer,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TerminologyExpander for FakeExpander {
        async fn expand(&self, modifier: &SearchModifier, value: &str) -> TerminologyExpansion {
            self.calls
                .lock()
                .unwrap()
                .push((modifier.to_string(), value.to_string()));
            self.answer.clone()
        }
    }

    async fn run_with(
        b: &SqliteBackend,
        t: &TenantContext,
        query: &SearchQuery,
        expander: &FakeExpander,
    ) -> StorageResult<Vec<String>> {
        let options = ChainResolveOptions {
            terminology: Some(expander),
        };
        let rewritten = resolve_chains_with(b, t, query, options).await?;
        let mut ids: Vec<String> = b
            .search(t, &rewritten)
            .await?
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// #1365: a valid terminology modifier on a terminal parameter is expanded
    /// through the caller's expander — once, after the terminal has been typed
    /// — and the terminal search runs on the expansion.
    #[tokio::test]
    async fn terminal_terminology_modifier_is_expanded_by_the_caller() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;
        b.create(
            &t,
            "Patient",
            json!({ "resourceType": "Patient", "id": "pf", "gender": "female" }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
        b.create(
            &t,
            "Procedure",
            json!({ "resourceType": "Procedure", "id": "prf", "status": "completed",
                    "subject": { "reference": "Patient/pf" } }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

        // Procedure?subject:Patient.gender:in=http://vs → gender=female,other
        let tokens = TerminologyExpansion::Tokens("female,other".into());
        let expander = FakeExpander::new(tokens.clone());
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "gender")],
            SearchModifier::In,
            &["http://vs"],
        );
        assert_eq!(run_with(&b, &t, &q, &expander).await.unwrap(), ["prf"]);
        assert_eq!(
            expander.calls(),
            [("in".to_string(), "http://vs".to_string())]
        );

        // Patient?_has:Procedure:subject:status:below=s|c — nobody has that status.
        let expander = FakeExpander::new(tokens);
        let q = has("Patient", "Procedure", "subject", "status:below", "s|c");
        assert!(run_with(&b, &t, &q, &expander).await.unwrap().is_empty());
        assert_eq!(expander.calls(), [("below".to_string(), "s|c".to_string())]);
        let expander = FakeExpander::new(TerminologyExpansion::Tokens("completed".into()));
        assert_eq!(
            run_with(&b, &t, &q, &expander).await.unwrap(),
            ["p80", "p90", "pf"]
        );

        // Dropped (the caller fails open): the chain no longer constrains.
        let expander = FakeExpander::new(TerminologyExpansion::Dropped);
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "gender")],
            SearchModifier::In,
            &["http://vs"],
        );
        assert_eq!(
            run_with(&b, &t, &q, &expander).await.unwrap(),
            ["pr1", "pr2", "pr3", "pr4", "prf"]
        );

        // A structural `:below` (reference terminal) is not the expander's.
        let expander = FakeExpander::new(TerminologyExpansion::Dropped);
        let q = forward_modified(
            "Procedure",
            &[("subject", "Patient", "general-practitioner")],
            SearchModifier::Below,
            &["Practitioner/x"],
        );
        assert!(run_with(&b, &t, &q, &expander).await.unwrap().is_empty());
        assert_eq!(expander.calls(), vec![]);

        // `:not-in` cannot be expanded: unanswerable with a server too, but
        // only once it is known to be valid for the terminal's type.
        for (q, valid) in [
            (
                has(
                    "Patient",
                    "Procedure",
                    "subject",
                    "status:not-in",
                    "http://vs",
                ),
                true,
            ),
            (
                has(
                    "Patient",
                    "Procedure",
                    "subject",
                    "date:not-in",
                    "http://vs",
                ),
                false,
            ),
        ] {
            match (run_with(&b, &t, &q, &expander).await, valid) {
                (
                    Err(StorageError::Search(SearchError::TerminologyRequired {
                        modifier, ..
                    })),
                    true,
                ) => {
                    assert_eq!(modifier, "not-in")
                }
                (Err(StorageError::Search(SearchError::QueryParseError { .. })), false) => {}
                (other, _) => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(expander.calls(), vec![]);
    }

    /// #1339: an error on the terminal parameter of a nested `_has` names the
    /// whole key, not just its innermost level.
    #[tokio::test]
    async fn nested_has_terminal_errors_name_the_full_key() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        // Patient?_has:Encounter:subject:_has:Procedure:encounter:<param>=…
        let nested = |param: &str| {
            let mut query = SearchQuery::new("Patient");
            query.reverse_chains.push(ReverseChainedParameter::nested(
                "Encounter",
                "subject",
                ReverseChainedParameter::terminal(
                    "Procedure",
                    "encounter",
                    param,
                    SearchValue::eq("http://example.org/vs"),
                ),
            ));
            query
        };

        match resolve_chains(&b, &t, &nested("status:in")).await {
            Err(StorageError::Search(SearchError::TerminologyRequired { modifier, param })) => {
                assert_eq!(modifier, "in");
                assert_eq!(
                    param,
                    "_has:Encounter:subject:_has:Procedure:encounter:status"
                );
            }
            other => panic!("expected a terminology-required error, got {other:?}"),
        }
        for param in ["status:exact", "date:in"] {
            match resolve_chains(&b, &t, &nested(param)).await {
                Err(StorageError::Search(SearchError::QueryParseError { message })) => {
                    assert!(
                        message.contains(&format!(
                            "'_has:Encounter:subject:_has:Procedure:encounter:{param}'"
                        )),
                        "{message}"
                    );
                }
                other => panic!("expected a query parse error, got {other:?}"),
            }
        }
    }

    /// Values a caller already parsed (non-`eq` prefix) pass through as given.
    #[tokio::test]
    async fn pre_parsed_values_pass_through() {
        let b = backend();
        let t = tenant();
        seed_dated(&b, &t).await;

        let mut q = forward("Procedure", SUBJECT_BIRTHDATE, &[]);
        q.parameters[0].values = vec![SearchValue::new(SearchPrefix::Ge, "1985-01-01")];
        assert_eq!(run(&b, &t, &q).await, ["pr3", "pr4"]);
    }

    /// A two-level chain over a wide intermediate set used to 500 with
    /// "Expression tree is too large (maximum depth 1000)" (#943): each hop
    /// searched one query carrying every intermediate reference as an OR of
    /// equals, and the final rewrite injected every matched id as an `_id` OR,
    /// both of which nest one level per value and trip SQLite's parse-depth
    /// limit past ~1000 terms. The resolver now chunks the reference hops and
    /// the `_id` filter is a flat `IN (...)`, so a wide chain resolves.
    #[tokio::test]
    async fn two_level_chain_over_wide_intermediate_set() {
        let b = backend();
        let t = tenant();

        // Comfortably past the depth-1000 limit so the un-chunked path fails.
        const N: usize = 1200;
        for i in 0..N {
            let pid = format!("p{i}");
            let eid = format!("e{i}");
            let oid = format!("o{i}");
            b.create(
                &t,
                "Patient",
                json!({ "resourceType": "Patient", "id": pid, "gender": "female" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
            b.create(
                &t,
                "Encounter",
                json!({ "resourceType": "Encounter", "id": eid, "status": "finished",
                        "subject": { "reference": format!("Patient/{pid}") } }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
            b.create(
                &t,
                "Observation",
                json!({ "resourceType": "Observation", "id": oid, "status": "final",
                        "encounter": { "reference": format!("Encounter/{eid}") } }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        }

        // Observation?encounter.subject.gender=female (a two-hop forward chain;
        // `subject` is the reference param the sibling tests exercise).
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "encounter".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("female")],
            chain: vec![
                ChainedParameter {
                    reference_param: "encounter".to_string(),
                    target_type: Some("Encounter".to_string()),
                    target_param: "subject".to_string(),
                },
                ChainedParameter {
                    reference_param: "subject".to_string(),
                    target_type: Some("Patient".to_string()),
                    target_param: "gender".to_string(),
                },
            ],
            components: vec![],
        });

        let mut rewritten = resolve_chains(&b, &t, &query)
            .await
            .expect("wide chain resolves without a depth-1000 failure");
        // One page big enough to hold every match, so the assertion sees the
        // full result rather than a default-capped page.
        rewritten.count = Some((N + 10) as u32);
        let result = b
            .search(&t, &rewritten)
            .await
            .expect("the rewritten _id filter prepares as a flat IN list");
        assert_eq!(
            result.resources.items.len(),
            N,
            "every observation whose encounter's patient is female matches"
        );
    }
}

/// Heuristic fallback for inferring the target resource type of a reference
/// search parameter when the registry has no (or an ambiguous) target list.
///
/// This is the single source of truth shared by every backend's chain builder
/// (SQLite, PostgreSQL) and the composite storage layer, so all of them agree on
/// which type an untyped chain link resolves to. Callers reach it via
/// `crate::search::chain_resolver::infer_target_type`.
///
/// Note: this remains a hand-maintained heuristic rather than spec-derived data;
/// the longer-term fix is to pick the first declared target from the
/// SearchParameter registry. Keeping it in one place is the prerequisite for that
/// migration.
pub(crate) fn infer_target_type(ref_param: &str) -> String {
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
