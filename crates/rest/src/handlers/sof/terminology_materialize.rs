//! Write-path materialization of terminology dependencies (#1824, D1–D4,
//! D10, D12, D13).
//!
//! The governing rule: **expansions live in HFS storage; the query path
//! never calls a terminology server.** So the expansion has to exist by the
//! time a SQL Query runs, and this module produces it on the write path:
//!
//! - **T1 — a Library write.** When a SQLQuery or SQLView Library is written
//!   (POST, PUT, PATCH, batch entry, transaction entry), its *transitive*
//!   dependency graph is walked and every stored ValueSet without an
//!   `expansion` is expanded, in two phases: phase A
//!   ([`plan_library_materialization`]) computes every expansion in memory
//!   and validates every resource phase B would write; phase B
//!   ([`apply_materialization`]) persists them, immediately before the outer
//!   write. A canonical that neither storage nor the bundle holds is fetched
//!   from the terminology server (direct and batch writes only, D4), stored
//!   and expanded.
//! - **T2 — a ValueSet write.** When a ValueSet without an `expansion` is
//!   written and some stored (or in-bundle) SQL Library depends on it,
//!   [`materialize_value_set_candidate`] adds the expansion to the incoming
//!   candidate, which the caller then persists through its normal single
//!   write.
//!
//! The expansion engine is the configured terminology server whenever there
//! is one (D3); without one, only a compose that needs no terminology
//! knowledge is expanded locally
//! ([`helios_sof::sqlquery::local_expansion`]). Materialization acts under
//! the caller's SMART scopes (D12) and within a wall-clock budget
//! (`HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS`). `strict` mode fails the
//! write when materialization cannot complete; `lenient` warns and stores;
//! `off` never materializes (D10).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_auth::{Principal, SmartPermissions};
use helios_fhir::FhirVersion;
use helios_persistence::core::WriteKind;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::error::{BackendError, ConcurrencyError, StorageError};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchQuery, StoredResource};
use helios_sof::sqlquery::{
    DependsOnView, FlattenLimits, TerminologyContentError, estimate_json_bytes, local_expansion,
    parse_sqlquery_library,
};
use serde_json::{Value, json};
use tracing::{debug, warn};

use super::graph::{
    DEPENDENCY_RESOURCE_TYPES, DependencyArtifact, MAX_GRAPH_DEPTH, classify_dependency,
};
use super::references::{
    canonical_candidates, canonical_matches, resolve_resource_canonical_or_relative, select_newest,
};
use crate::config::ServerConfig;
use crate::error::RestError;
use crate::state::AppState;
use crate::terminology::{ExpandOptions, TerminologyError, TerminologyServiceClient};

/// The tag system HFS marks materialized terminology artifacts with.
pub(crate) const SOF_TERMINOLOGY_TAG_SYSTEM: &str =
    "http://heliossoftware.com/fhir/CodeSystem/sof-terminology";
/// Tag code: the `expansion` was produced by HFS on a write.
pub(crate) const TAG_MATERIALIZED_EXPANSION: &str = "materialized-expansion";
/// Tag code: the definition was fetched from the terminology server (D4).
pub(crate) const TAG_FETCHED_FROM_TERMINOLOGY_SERVER: &str = "fetched-from-terminology-server";
/// OperationOutcome coding: a conditional create committed the Library but
/// phase B failed, so the Library is stored and unmaterialized (D13 c).
pub(crate) const CODE_CREATED_UNMATERIALIZED: &str = "created-unmaterialized";

/// Longest single call to the terminology server, whatever the budget.
const MAX_HTS_CALL: Duration = Duration::from_secs(30);

/// Most stored Libraries T2 inspects when asking whether any depends on a
/// ValueSet. SQL Libraries are definitions, not data; a tenant with more is
/// told so in the log.
const MAX_LIBRARIES_SCANNED: u32 = 1_000;

/// `HFS_SOF_TERMINOLOGY_MATERIALIZE` (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterializeMode {
    Strict,
    Lenient,
    Off,
}

impl MaterializeMode {
    pub(crate) fn from_config(config: &ServerConfig) -> Self {
        match config.sof_terminology_materialize.as_str() {
            "lenient" => Self::Lenient,
            "off" => Self::Off,
            _ => Self::Strict,
        }
    }
}

/// The wall-clock budget one write may spend materializing all of its
/// dependencies, shared across every dependency of the write and every
/// entry of one bundle.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaterializeBudget {
    deadline: Instant,
    total_secs: u64,
}

impl MaterializeBudget {
    /// A fresh budget of `HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS`.
    pub(crate) fn start(config: &ServerConfig) -> Self {
        let total_secs = config.sof_terminology_materialize_budget_secs;
        Self {
            deadline: Instant::now() + Duration::from_secs(total_secs),
            total_secs,
        }
    }

    /// The timeout for the next terminology-server call: what is left of the
    /// budget, capped at [`MAX_HTS_CALL`]; `Err` once the budget is spent.
    fn call_timeout(&self) -> Result<Duration, MaterializeError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(MaterializeError::BudgetExceeded {
                secs: self.total_secs,
            });
        }
        Ok(remaining.min(MAX_HTS_CALL))
    }
}

/// What materialization did (or decided) about one dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The stored ValueSet already carries an expansion.
    AlreadyExpanded,
    /// Expanded by [`helios_sof::sqlquery::local_expansion`].
    ExpandedLocally,
    /// Expanded by the terminology server.
    ExpandedByServer,
    /// Definition fetched from the terminology server, then expanded there.
    FetchedAndExpanded,
    /// Definition (a ConceptMap) fetched from the terminology server.
    FetchedDefinition,
    /// A ConceptMap or an already-materialized relation: nothing to do.
    NoActionNeeded,
    /// Neither storage, the bundle nor the terminology server has it. Not an
    /// error: a Library write never validated its dependencies.
    NotResolvable,
    /// T2: no SQL Library depends on this ValueSet, so it is stored as sent.
    NotDependedOn,
    /// Lenient mode: the caller lacks the scope the write needs.
    SkippedNoScope,
    /// Lenient mode: materialization failed; the reason.
    SkippedLenient(String),
}

/// One write phase B performs.
#[derive(Debug)]
pub(crate) enum PlannedWrite {
    /// Add an `expansion` to a stored ValueSet (a new history version).
    UpdateExpansion {
        url: String,
        current: Box<StoredResource>,
        content: Value,
    },
    /// Store a definition fetched from the terminology server.
    CreateDefinition {
        url: String,
        resource_type: String,
        content: Value,
    },
}

impl PlannedWrite {
    fn url(&self) -> &str {
        match self {
            Self::UpdateExpansion { url, .. } | Self::CreateDefinition { url, .. } => url,
        }
    }
}

/// Phase A's output: everything phase B will write, already validated, plus
/// the per-dependency outcomes and lenient-mode warnings.
#[derive(Debug, Default)]
pub(crate) struct MaterializationPlan {
    pub writes: Vec<PlannedWrite>,
    pub outcomes: Vec<(String, Outcome)>,
    pub warnings: Vec<String>,
}

impl MaterializationPlan {
    /// Whether phase B has anything to do.
    pub(crate) fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// The URLs phase B would write, for diagnostics.
    pub(crate) fn pending_urls(&self) -> Vec<String> {
        self.writes.iter().map(|w| w.url().to_string()).collect()
    }
}

/// Where the walk resolves a dependency reference (`Type/id`, `url`,
/// `url|version`). The direct paths use [`StorageSource`]; a transaction
/// uses its bundle overlay first (`handlers::batch`).
#[async_trait]
pub(crate) trait DependencySource: Send + Sync {
    async fn lookup(&self, reference: &str) -> Result<Lookup, RestError>;
}

/// What a dependency reference resolved to.
#[derive(Debug)]
pub(crate) enum Lookup {
    /// A stored resource.
    Stored(StoredResource),
    /// An in-bundle candidate not yet persisted (transactions only).
    Candidate(Value),
    NotFound,
}

/// [`DependencySource`] over the tenant's storage: the four dependency types
/// in [`DEPENDENCY_RESOURCE_TYPES`] order, a relative reference only its own
/// type, a pinned canonical only that version.
pub(crate) struct StorageSource<'a, S> {
    pub state: &'a AppState<S>,
    pub tenant: &'a TenantContext,
}

#[async_trait]
impl<S> DependencySource for StorageSource<'_, S>
where
    S: SearchProvider + Send + Sync + 'static,
{
    async fn lookup(&self, reference: &str) -> Result<Lookup, RestError> {
        lookup_in_storage(self.state, self.tenant, reference).await
    }
}

/// The storage half of every [`DependencySource`]: resolves `reference` to
/// the newest stored match among the four dependency types.
pub(crate) async fn lookup_in_storage<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    reference: &str,
) -> Result<Lookup, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    for resource_type in DEPENDENCY_RESOURCE_TYPES {
        let prefix = format!("{resource_type}/");
        if let Some(id) = reference.trim().strip_prefix(prefix.as_str()) {
            let id = id.split('/').next().unwrap_or("");
            return match state.storage().read(tenant, resource_type, id).await {
                Ok(Some(stored)) => Ok(Lookup::Stored(stored)),
                Ok(None) => Ok(Lookup::NotFound),
                Err(StorageError::Resource(_)) => Ok(Lookup::NotFound),
                Err(e) => Err(e.into()),
            };
        }
    }
    if DEPENDENCY_RESOURCE_TYPES
        .iter()
        .any(|t| reference.starts_with(&format!("{t}/")))
    {
        return Ok(Lookup::NotFound);
    }
    for resource_type in DEPENDENCY_RESOURCE_TYPES {
        let candidates = canonical_candidates(state, tenant, resource_type, reference).await?;
        if let Some(chosen) = select_newest(candidates) {
            return match state
                .storage()
                .read(tenant, resource_type, &chosen.id)
                .await
            {
                Ok(Some(stored)) => Ok(Lookup::Stored(stored)),
                Ok(None) => continue,
                Err(StorageError::Resource(_)) => continue,
                Err(e) => Err(e.into()),
            };
        }
    }
    Ok(Lookup::NotFound)
}

/// Why materialization could not complete. Strict mode turns it into the
/// write's error; lenient mode into a warning.
#[derive(Debug)]
pub(crate) enum MaterializeError {
    /// The caller lacks the scope a planned write needs (D12).
    Forbidden { message: String },
    /// Inside a transaction, a dependency would need a write outside the
    /// bundle (D4).
    NeedsSideWrite { url: String, remedy: String },
    /// The artifact's content cannot be expanded or represented.
    Content {
        url: String,
        error: TerminologyContentError,
    },
    /// The terminology server failed.
    Server {
        url: String,
        error: TerminologyError,
    },
    /// `HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS` is spent.
    BudgetExceeded { secs: u64 },
    /// A structural problem in the dependency graph (cycle, depth, limits).
    Graph(RestError),
    /// A planned resource failed write validation (§6.15).
    Validation { url: String, error: RestError },
    /// Storage failed.
    Storage(RestError),
}

impl MaterializeError {
    fn is_forbidden(&self) -> bool {
        matches!(self, Self::Forbidden { .. })
    }

    /// The diagnostic text, without status.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Forbidden { message } => message.clone(),
            Self::NeedsSideWrite { url, remedy } => {
                format!("dependency '{url}' needs a write outside this transaction: {remedy}")
            }
            Self::Content { url, error } => format!("dependency '{url}': {error}"),
            Self::Server { url, error } => {
                format!("dependency '{url}': the terminology server could not expand it: {error}")
            }
            Self::BudgetExceeded { secs } => format!(
                "materialization exceeded {secs}s; raise \
                 HFS_SOF_TERMINOLOGY_MATERIALIZE_BUDGET_SECS and HFS_REQUEST_TIMEOUT together, \
                 or pre-store the expansion"
            ),
            Self::Graph(e) | Self::Storage(e) => e.client_response().2,
            Self::Validation { url, error } => {
                format!(
                    "dependency '{url}' did not validate: {}",
                    error.client_response().2
                )
            }
        }
    }

    /// The OperationOutcome issue code (§6.8).
    fn issue_code(&self) -> &'static str {
        match self {
            Self::Forbidden { .. } => "forbidden",
            Self::Content { error, .. } => error.issue_code(),
            Self::Server { error, .. } => match error {
                TerminologyError::Network(_) => "exception",
                TerminologyError::ServerError { status, .. } if *status >= 500 => "exception",
                TerminologyError::TooLarge { .. } => "too-costly",
                _ => "invalid",
            },
            Self::NeedsSideWrite { .. } | Self::BudgetExceeded { .. } => "invalid",
            Self::Graph(e) | Self::Storage(e) => e.client_response().1,
            Self::Validation { .. } => "invalid",
        }
    }

    /// Strict mode: the error the write answers with.
    pub(crate) fn into_rest_error(self) -> RestError {
        match self {
            Self::Forbidden { message } => RestError::Forbidden { message },
            Self::Graph(e) | Self::Storage(e) => e,
            Self::Validation { url, error } => match error {
                RestError::ValidationFailed { mut outcome } => {
                    let label = json!({
                        "severity": "error",
                        "code": "invalid",
                        "diagnostics": format!(
                            "the expansion materialized for dependency '{url}' did not validate; \
                             the issues below are the validator's"
                        )
                    });
                    if let Some(issues) = outcome.get_mut("issue").and_then(Value::as_array_mut) {
                        issues.insert(0, label);
                    }
                    RestError::ValidationFailed { outcome }
                }
                other => other,
            },
            other => {
                let code = other.issue_code();
                RestError::ValidationFailed {
                    outcome: json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{"severity": "error", "code": code, "diagnostics": other.message()}]
                    }),
                }
            }
        }
    }
}

impl From<StorageError> for MaterializeError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e.into())
    }
}

// ============================================================================
// Phase A — plan
// ============================================================================

/// What the walk found that phase A must act on.
enum Need {
    /// A stored ValueSet without an expansion.
    Expand {
        url: String,
        label: String,
        current: Box<StoredResource>,
    },
    /// A canonical nothing holds; the terminology server may.
    Fetch { url: String, label: String },
}

/// Trigger T1, phase A only: walks `library`'s **transitive** dependency
/// graph through `source` (SQLViews descended into; cycles, [`MAX_GRAPH_DEPTH`]
/// and `sof_sqlquery_max_vds` apply as in `build_plan`), computes every
/// expansion that must be written, fetches definitions only the terminology
/// server holds when `allow_side_writes` (direct and batch writes, D4; inside
/// a transaction such a definition is reported as a needed side write), and
/// validates every planned resource with `check_write` (§6.15). Reads only;
/// writes nothing. Checks `principal`'s scopes before contacting the server
/// (D12).
///
/// A resource that is not a SQLQuery/SQLView Library, or `off` mode, yields
/// an empty plan. In `lenient` mode every failure becomes a warning and an
/// outcome instead of an error.
pub(crate) async fn plan_library_materialization<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    principal: Option<&Principal>,
    library: &Value,
    source: &dyn DependencySource,
    allow_side_writes: bool,
    budget: &MaterializeBudget,
) -> Result<MaterializationPlan, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let mode = MaterializeMode::from_config(state.config());
    if mode == MaterializeMode::Off
        || library.get("resourceType").and_then(Value::as_str) != Some("Library")
    {
        return Ok(MaterializationPlan::default());
    }
    let Ok(parsed) = parse_sqlquery_library(library) else {
        // Not a SQLQuery/SQLView Library (a CQL Library, say): nothing to do.
        return Ok(MaterializationPlan::default());
    };
    let identity = library
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("<library>")
        .to_string();

    match plan_inner(
        state,
        tenant,
        principal,
        &identity,
        &parsed.depends_on,
        source,
        allow_side_writes,
        budget,
    )
    .await
    .map_err(|failure| *failure)
    {
        Ok(plan) => Ok(plan),
        Err((partial, error)) => match mode {
            MaterializeMode::Strict => Err(error.into_rest_error()),
            MaterializeMode::Lenient | MaterializeMode::Off => {
                let message = error.message();
                warn!(library = %identity, error = %message, "terminology materialization skipped (lenient)");
                let mut plan = partial;
                plan.warnings.push(message.clone());
                let outcome = if error.is_forbidden() {
                    Outcome::SkippedNoScope
                } else {
                    Outcome::SkippedLenient(message)
                };
                plan.outcomes.push((identity, outcome));
                // Lenient never writes a partial plan: the Library is stored
                // as sent and the warning says why.
                plan.writes.clear();
                Ok(plan)
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn plan_inner<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    principal: Option<&Principal>,
    identity: &str,
    depends_on: &[DependsOnView],
    source: &dyn DependencySource,
    allow_side_writes: bool,
    budget: &MaterializeBudget,
) -> Result<MaterializationPlan, Box<(MaterializationPlan, MaterializeError)>>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let mut plan = MaterializationPlan::default();
    let needs = match walk(state, tenant, identity, depends_on, source, &mut plan).await {
        Ok(needs) => needs,
        Err(e) => return Err(Box::new((plan, e))),
    };
    if needs.is_empty() {
        return Ok(plan);
    }

    // D12: the caller's scopes must permit every write phase B would do,
    // checked before the terminology server is contacted.
    let needs_update = needs.iter().any(|n| matches!(n, Need::Expand { .. }));
    let needs_fetch = allow_side_writes && needs.iter().any(|n| matches!(n, Need::Fetch { .. }));
    if needs_update && let Err(e) = require_scope(principal, "ValueSet", SmartPermissions::UPDATE) {
        return Err(Box::new((plan, e)));
    }
    if needs_fetch
        && !permits(principal, "ValueSet", SmartPermissions::CREATE)
        && !permits(principal, "ConceptMap", SmartPermissions::CREATE)
    {
        return Err(Box::new((
            plan,
            MaterializeError::Forbidden {
                message: "materializing this Library would store a ValueSet or ConceptMap \
                          fetched from the terminology server, which needs a create scope on \
                          ValueSet or ConceptMap"
                    .to_string(),
            },
        )));
    }

    let config = state.config();
    let hts = state
        .terminology_server_url()
        .map(|url| TerminologyServiceClient::with_timeout(url.to_string(), MAX_HTS_CALL));

    for need in needs {
        match need {
            Need::Expand {
                url,
                label,
                current,
            } => {
                let (content, outcome) =
                    match expand(state, hts.as_ref(), current.content(), &url, budget).await {
                        Ok(v) => v,
                        Err(e) => return Err(Box::new((plan, e))),
                    };
                let content = match finish_for_storage(
                    state,
                    tenant,
                    &url,
                    current.fhir_version(),
                    "ValueSet",
                    content,
                    TAG_MATERIALIZED_EXPANSION,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => return Err(Box::new((plan, e))),
                };
                debug!(library = %identity, label = %label, url = %url, ?outcome, "expansion planned");
                plan.outcomes.push((url.clone(), outcome));
                plan.writes.push(PlannedWrite::UpdateExpansion {
                    url,
                    current,
                    content,
                });
            }
            Need::Fetch { url, label } => {
                // Neither storage nor the bundle holds it. Without a
                // terminology server nothing could supply it, and a Library
                // write never validated its dependencies: left alone (D4).
                let Some(client) = hts.as_ref() else {
                    plan.outcomes.push((url, Outcome::NotResolvable));
                    continue;
                };
                let fetched = match fetch_definition(client, config, &url, budget).await {
                    Ok(f) => f,
                    Err(e) => return Err(Box::new((plan, e))),
                };
                let Some((resource_type, definition)) = fetched else {
                    plan.outcomes.push((url, Outcome::NotResolvable));
                    continue;
                };
                // Inside a transaction the definition the server holds would
                // have to be stored outside the bundle, which it never is.
                if !allow_side_writes {
                    return Err(Box::new((
                        plan,
                        MaterializeError::NeedsSideWrite {
                            url,
                            remedy: format!(
                                "the terminology server holds this {resource_type}; include it \
                                 in the transaction, or store it first"
                            ),
                        },
                    )));
                }
                if let Err(e) = require_scope(principal, resource_type, SmartPermissions::CREATE) {
                    return Err(Box::new((plan, e)));
                }
                let (content, outcome) =
                    if resource_type == "ValueSet" && definition.get("expansion").is_none() {
                        match expand(state, Some(client), &definition, &url, budget).await {
                            Ok((c, _)) => (c, Outcome::FetchedAndExpanded),
                            Err(e) => return Err(Box::new((plan, e))),
                        }
                    } else {
                        (definition, Outcome::FetchedDefinition)
                    };
                let content = match finish_for_storage(
                    state,
                    tenant,
                    &url,
                    config.default_fhir_version,
                    resource_type,
                    content,
                    TAG_FETCHED_FROM_TERMINOLOGY_SERVER,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => return Err(Box::new((plan, e))),
                };
                debug!(library = %identity, label = %label, url = %url, ?outcome, "definition planned");
                plan.outcomes.push((url.clone(), outcome));
                plan.writes.push(PlannedWrite::CreateDefinition {
                    url,
                    resource_type: resource_type.to_string(),
                    content,
                });
            }
        }
    }
    Ok(plan)
}

/// The bounded, iterative walk of a Library's transitive dependency graph
/// (the same bounds `build_plan` applies: [`MAX_GRAPH_DEPTH`], the
/// per-Library `depends-on` limit, `sof_sqlquery_max_vds`, and cycle
/// detection with the same diagnostic). Records what every terminology
/// dependency needs.
async fn walk<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    identity: &str,
    depends_on: &[DependsOnView],
    source: &dyn DependencySource,
    plan: &mut MaterializationPlan,
) -> Result<Vec<Need>, MaterializeError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let _ = tenant;
    let max_vds = state.config().sof_sqlquery_max_vds;
    let max_depends_on = super::input_limits::max_depends_on(max_vds);
    if depends_on.len() > max_depends_on {
        return Err(MaterializeError::Graph(
            super::input_limits::depends_on_limit_error(identity, depends_on.len(), max_depends_on),
        ));
    }

    struct Frame {
        identity: String,
        depends_on: Vec<DependsOnView>,
        next: usize,
    }
    let mut stack = vec![Frame {
        identity: identity.to_string(),
        depends_on: depends_on.to_vec(),
        next: 0,
    }];
    let mut on_stack: HashSet<String> = HashSet::from([identity.to_string()]);
    let mut path: Vec<String> = vec![identity.to_string()];
    let mut visited: HashSet<String> = HashSet::new();
    let mut nodes = 0usize;
    let mut needs = Vec::new();

    while let Some(top) = stack.last_mut() {
        if top.next >= top.depends_on.len() {
            let frame = stack.pop().expect("checked");
            on_stack.remove(&frame.identity);
            // A SQLView is finished only once its children are: until then
            // a reference back to it is a cycle, not a revisit.
            visited.insert(frame.identity);
            path.pop();
            continue;
        }
        let dep = top.depends_on[top.next].clone();
        top.next += 1;

        if visited.contains(&dep.url) {
            continue;
        }
        if on_stack.contains(&dep.url) {
            let mut cycle = path.clone();
            cycle.push(dep.url.clone());
            return Err(MaterializeError::Graph(RestError::BadRequest {
                message: format!("dependency cycle detected: {}", cycle.join(" \u{2192} ")),
            }));
        }
        if stack.len() > MAX_GRAPH_DEPTH {
            return Err(MaterializeError::Graph(RestError::BadRequest {
                message: format!(
                    "dependency graph exceeds the maximum depth of {MAX_GRAPH_DEPTH} levels at '{}'",
                    dep.url
                ),
            }));
        }
        nodes += 1;
        if nodes > max_vds {
            return Err(MaterializeError::Graph(RestError::UnprocessableEntity {
                message: format!("dependency graph has {nodes} nodes; max allowed is {max_vds}"),
            }));
        }

        let (artifact, stored): (Value, Option<StoredResource>) = match source
            .lookup(&dep.url)
            .await
            .map_err(MaterializeError::Storage)?
        {
            Lookup::Stored(stored) => (stored.content().clone(), Some(stored)),
            Lookup::Candidate(candidate) => (candidate, None),
            Lookup::NotFound => {
                visited.insert(dep.url.clone());
                needs.push(Need::Fetch {
                    url: dep.url.clone(),
                    label: dep.label.clone(),
                });
                continue;
            }
        };

        let kind = classify_dependency(&artifact);
        if kind != DependencyArtifact::SqlView {
            visited.insert(dep.url.clone());
        }
        match kind {
            DependencyArtifact::ValueSet => {
                if artifact.get("expansion").is_some() {
                    plan.outcomes
                        .push((dep.url.clone(), Outcome::AlreadyExpanded));
                } else if let Some(current) = stored {
                    needs.push(Need::Expand {
                        url: dep.url.clone(),
                        label: dep.label.clone(),
                        current: Box::new(current),
                    });
                } else {
                    // An in-bundle candidate the pre-pass could not expand
                    // (it is transformed before Libraries are planned).
                    return Err(MaterializeError::NeedsSideWrite {
                        url: dep.url.clone(),
                        remedy: "the ValueSet in this transaction carries no expansion and could \
                                 not be expanded"
                            .to_string(),
                    });
                }
            }
            DependencyArtifact::ConceptMap => {
                plan.outcomes
                    .push((dep.url.clone(), Outcome::NoActionNeeded));
            }
            DependencyArtifact::SqlView => {
                let Ok(library) = parse_sqlquery_library(&artifact) else {
                    continue;
                };
                if library.depends_on.len() > max_depends_on {
                    return Err(MaterializeError::Graph(
                        super::input_limits::depends_on_limit_error(
                            &dep.url,
                            library.depends_on.len(),
                            max_depends_on,
                        ),
                    ));
                }
                on_stack.insert(dep.url.clone());
                path.push(dep.url.clone());
                stack.push(Frame {
                    identity: dep.url.clone(),
                    depends_on: library.depends_on,
                    next: 0,
                });
            }
            DependencyArtifact::ViewDefinition
            | DependencyArtifact::SqlQuery
            | DependencyArtifact::Other(_) => {}
        }
    }
    Ok(needs)
}

fn permits(
    principal: Option<&Principal>,
    resource_type: &str,
    permission: SmartPermissions,
) -> bool {
    principal.is_none_or(|p| p.scopes.is_permitted(resource_type, permission))
}

fn require_scope(
    principal: Option<&Principal>,
    resource_type: &str,
    permission: SmartPermissions,
) -> Result<(), MaterializeError> {
    if permits(principal, resource_type, permission) {
        return Ok(());
    }
    let verb = if permission == SmartPermissions::CREATE {
        "create"
    } else {
        "update"
    };
    Err(MaterializeError::Forbidden {
        message: format!(
            "materializing this Library's terminology dependencies would {verb} a \
             {resource_type}, which the caller's scopes do not permit (a {resource_type} \
             {verb} scope is required)"
        ),
    })
}

/// Expands `value_set` (D3): through the terminology server when configured,
/// else locally when the compose is enumerated. Returns the ValueSet with the
/// expansion set.
async fn expand<S>(
    state: &AppState<S>,
    hts: Option<&TerminologyServiceClient>,
    value_set: &Value,
    url: &str,
    budget: &MaterializeBudget,
) -> Result<(Value, Outcome), MaterializeError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let config = state.config();
    let max_members = config.sof_terminology_max_expansion_members;
    if let Some(client) = hts {
        let opts = ExpandOptions {
            timeout: budget.call_timeout()?,
            max_members,
            max_artifact_bytes: config.sof_terminology_max_artifact_bytes,
        };
        let answer = client
            .expand_value_set_resource(value_set, &opts)
            .await
            .map_err(|error| MaterializeError::Server {
                url: url.to_string(),
                error,
            })?;
        let Some(expansion) = answer.get("expansion").cloned() else {
            return Err(MaterializeError::Server {
                url: url.to_string(),
                error: TerminologyError::Parse("the answer carries no expansion".to_string()),
            });
        };
        // The server expanded the definition HFS stores; keep that
        // definition and take the server's expansion.
        let mut expanded = value_set.clone();
        expanded["expansion"] = expansion;
        return Ok((expanded, Outcome::ExpandedByServer));
    }
    let limits = FlattenLimits::for_members(max_members, config.sof_terminology_max_job_bytes);
    match local_expansion(
        value_set,
        chrono::Utc::now(),
        &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        limits,
    ) {
        Ok(expanded) => Ok((expanded, Outcome::ExpandedLocally)),
        Err(TerminologyContentError::NotEnumerated { reason }) => Err(MaterializeError::Content {
            url: url.to_string(),
            error: TerminologyContentError::NotEnumerated {
                reason: format!(
                    "{reason}; configure HFS_TERMINOLOGY_SERVER so a terminology server expands \
                     it, or store the ValueSet with an expansion"
                ),
            },
        }),
        Err(error) => Err(MaterializeError::Content {
            url: url.to_string(),
            error,
        }),
    }
}

/// D4: asks the terminology server for a definition HFS does not hold —
/// a ValueSet first, then a ConceptMap.
async fn fetch_definition(
    client: &TerminologyServiceClient,
    config: &ServerConfig,
    url: &str,
    budget: &MaterializeBudget,
) -> Result<Option<(&'static str, Value)>, MaterializeError> {
    let (canonical, version) = super::references::split_canonical_version(url);
    let max_bytes = config.sof_terminology_max_artifact_bytes;
    let server = |error| MaterializeError::Server {
        url: url.to_string(),
        error,
    };
    if let Some(value_set) = client
        .read_value_set_by_canonical(
            &canonical,
            version.as_deref(),
            budget.call_timeout()?,
            max_bytes,
        )
        .await
        .map_err(server)?
    {
        return Ok(Some(("ValueSet", value_set)));
    }
    if let Some(map) = client
        .read_concept_map_by_canonical(
            &canonical,
            version.as_deref(),
            budget.call_timeout()?,
            max_bytes,
        )
        .await
        .map_err(server)?
    {
        return Ok(Some(("ConceptMap", map)));
    }
    Ok(None)
}

/// Tags, size-checks (D9) and validates (§6.15) a resource phase B will
/// write. Server-assigned fields a fetched definition may carry are
/// dropped so the create assigns its own.
async fn finish_for_storage<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
    fhir_version: FhirVersion,
    resource_type: &str,
    mut content: Value,
    tag: &str,
) -> Result<Value, MaterializeError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    add_tag(&mut content, tag);
    if tag == TAG_FETCHED_FROM_TERMINOLOGY_SERVER
        && let Some(obj) = content.as_object_mut()
    {
        obj.remove("id");
        if let Some(meta) = obj.get_mut("meta").and_then(Value::as_object_mut) {
            meta.remove("versionId");
            meta.remove("lastUpdated");
        }
    }
    let cap = state.config().sof_terminology_max_artifact_bytes;
    estimate_json_bytes(&content, cap).map_err(|error| MaterializeError::Content {
        url: url.to_string(),
        error,
    })?;
    state
        .validation()
        .check_write(
            tenant.tenant_id().as_str(),
            fhir_version,
            resource_type,
            &content,
        )
        .await
        .map_err(|error| MaterializeError::Validation {
            url: url.to_string(),
            error,
        })?;
    Ok(content)
}

/// Adds `{SOF_TERMINOLOGY_TAG_SYSTEM, code}` to `meta.tag` unless present.
pub(crate) fn add_tag(resource: &mut Value, code: &str) {
    let Some(obj) = resource.as_object_mut() else {
        return;
    };
    let meta = obj
        .entry("meta")
        .or_insert_with(|| Value::Object(Default::default()));
    if !meta.is_object() {
        *meta = Value::Object(Default::default());
    }
    let tags = meta
        .as_object_mut()
        .expect("object")
        .entry("tag")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !tags.is_array() {
        *tags = Value::Array(Vec::new());
    }
    let tags = tags.as_array_mut().expect("array");
    let present = tags.iter().any(|t| {
        t.get("system").and_then(Value::as_str) == Some(SOF_TERMINOLOGY_TAG_SYSTEM)
            && t.get("code").and_then(Value::as_str) == Some(code)
    });
    if !present {
        tags.push(json!({"system": SOF_TERMINOLOGY_TAG_SYSTEM, "code": code}));
    }
}

// ============================================================================
// Phase B — apply
// ============================================================================

/// Phase B stopped at a write; what was and was not written (D13).
#[derive(Debug)]
pub(crate) struct ApplyFailure {
    pub error: RestError,
    pub written: Vec<String>,
    pub remaining: Vec<String>,
}

impl ApplyFailure {
    /// The 422 an ordinary write answers with: the failure, which
    /// dependencies were materialized (an idempotent cache the retry
    /// consumes) and which were not.
    pub(crate) fn into_rest_error(self) -> RestError {
        let (_, code, message) = self.error.client_response();
        RestError::ValidationFailed {
            outcome: json!({
                "resourceType": "OperationOutcome",
                "issue": [
                    {"severity": "error", "code": code, "diagnostics": format!(
                        "materializing terminology dependencies failed: {message}; already \
                         materialized: [{}]; still to materialize: [{}]. Re-issue the write to retry.",
                        self.written.join(", "), self.remaining.join(", ")
                    )}
                ]
            }),
        }
    }
}

/// Phase B: persists `plan`'s writes through the storage layer, emitting
/// write events as a direct write would, so subscriptions and audit see
/// them. Stops at the first failure (D13): the writes before it stay, as an
/// idempotent cache. A version conflict on an expansion update is re-read
/// once; an expansion that is now present is [`Outcome::AlreadyExpanded`].
pub(crate) async fn apply_materialization<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    plan: MaterializationPlan,
) -> Result<Vec<(String, Outcome)>, Box<ApplyFailure>>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let mut outcomes = plan.outcomes;
    let mut written: Vec<String> = Vec::new();
    let mut pending: Vec<PlannedWrite> = plan.writes;
    while !pending.is_empty() {
        let write = pending.remove(0);
        let url = write.url().to_string();
        let result: Result<(), RestError> = match write {
            PlannedWrite::UpdateExpansion {
                current, content, ..
            } => {
                let fhir_version = current.fhir_version();
                match state.storage().update(tenant, &current, content).await {
                    Ok(stored) => {
                        super::super::write_event::report(
                            state,
                            tenant,
                            fhir_version,
                            "ValueSet",
                            0,
                            Some(super::super::write_event::stored_notice(
                                WriteKind::Update,
                                &stored,
                            )),
                        );
                        Ok(())
                    }
                    Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                        ..
                    })) => match state.storage().read(tenant, "ValueSet", current.id()).await {
                        Ok(Some(now)) if now.content().get("expansion").is_some() => {
                            outcomes.retain(|(u, _)| u != &url);
                            outcomes.push((url.clone(), Outcome::AlreadyExpanded));
                            Ok(())
                        }
                        Ok(_) => Err(RestError::Conflict {
                            message: format!(
                                "ValueSet {} changed while its expansion was being materialized",
                                current.id()
                            ),
                        }),
                        Err(e) => Err(e.into()),
                    },
                    Err(e) => Err(e.into()),
                }
            }
            PlannedWrite::CreateDefinition {
                resource_type,
                content,
                ..
            } => {
                let fhir_version = state.config().default_fhir_version;
                match state
                    .storage()
                    .create(tenant, &resource_type, content, fhir_version)
                    .await
                {
                    Ok(stored) => {
                        super::super::write_event::report(
                            state,
                            tenant,
                            fhir_version,
                            &resource_type,
                            1,
                            Some(super::super::write_event::stored_notice(
                                WriteKind::Create,
                                &stored,
                            )),
                        );
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                }
            }
        };
        match result {
            Ok(()) => written.push(url),
            Err(error) => {
                let mut remaining = vec![url];
                remaining.extend(pending.iter().map(|w| w.url().to_string()));
                return Err(Box::new(ApplyFailure {
                    error,
                    written,
                    remaining,
                }));
            }
        }
    }
    Ok(outcomes)
}

// ============================================================================
// Trigger T2 — a ValueSet write
// ============================================================================

/// Trigger T2: returns `candidate` with an `expansion` added when (a) it has
/// none, (b) some stored SQLQuery/SQLView Library in the tenant — or one of
/// `bundle_libraries` — depends on its canonical, and (c) expansion succeeds.
/// The caller persists the returned candidate through its normal single
/// write (so `If-None-Exist`, `If-Match` and validation apply to the final
/// content). Never writes itself. Strict mode fails on (c); lenient returns
/// the candidate unchanged with [`Outcome::SkippedLenient`].
pub(crate) async fn materialize_value_set_candidate<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    candidate: Value,
    bundle_libraries: &[Value],
    budget: &MaterializeBudget,
) -> Result<(Value, Outcome), RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    materialize_value_set_candidate_with(state, tenant, candidate, bundle_libraries, true, budget)
        .await
}

/// [`materialize_value_set_candidate`] with the dependant check made
/// explicit: `dependants` are the Libraries already known (in-bundle
/// candidates, or a snapshot of stored ones), and `read_stored` says whether
/// the tenant's stored Libraries are read as well. A transaction executor
/// passes `false`: it must not read storage while its transaction is open.
pub(crate) async fn materialize_value_set_candidate_with<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    candidate: Value,
    dependants: &[Value],
    read_stored: bool,
    budget: &MaterializeBudget,
) -> Result<(Value, Outcome), RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let mode = MaterializeMode::from_config(state.config());
    if mode == MaterializeMode::Off
        || candidate.get("resourceType").and_then(Value::as_str) != Some("ValueSet")
    {
        return Ok((candidate, Outcome::NoActionNeeded));
    }
    if candidate.get("expansion").is_some() {
        return Ok((candidate, Outcome::AlreadyExpanded));
    }
    let Some(url) = candidate
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok((candidate, Outcome::NotDependedOn));
    };
    let mut depended_on = dependants
        .iter()
        .any(|lib| library_depends_on(lib, &candidate));
    if !depended_on && read_stored {
        depended_on = stored_sql_libraries(state, tenant)
            .await?
            .iter()
            .any(|lib| library_depends_on(lib, &candidate));
    }
    if !depended_on {
        return Ok((candidate, Outcome::NotDependedOn));
    }

    let hts = state
        .terminology_server_url()
        .map(|u| TerminologyServiceClient::with_timeout(u.to_string(), MAX_HTS_CALL));
    let expanded = match expand(state, hts.as_ref(), &candidate, &url, budget).await {
        Ok((mut content, outcome)) => {
            add_tag(&mut content, TAG_MATERIALIZED_EXPANSION);
            let cap = state.config().sof_terminology_max_artifact_bytes;
            match estimate_json_bytes(&content, cap) {
                Ok(_) => Ok((content, outcome)),
                Err(error) => Err(MaterializeError::Content { url, error }),
            }
        }
        Err(e) => Err(e),
    };
    match expanded {
        Ok(v) => Ok(v),
        Err(error) => match mode {
            MaterializeMode::Strict => Err(error.into_rest_error()),
            MaterializeMode::Lenient | MaterializeMode::Off => {
                let message = error.message();
                warn!(error = %message, "ValueSet expansion skipped (lenient)");
                Ok((candidate, Outcome::SkippedLenient(message)))
            }
        },
    }
}

/// Whether `library` is a SQLQuery/SQLView Library with a `depends-on`
/// entry that `candidate`'s canonical identity satisfies.
pub(crate) fn library_depends_on(library: &Value, candidate: &Value) -> bool {
    if library.get("resourceType").and_then(Value::as_str) != Some("Library") {
        return false;
    }
    let Ok(parsed) = parse_sqlquery_library(library) else {
        return false;
    };
    parsed
        .depends_on
        .iter()
        .any(|dep| canonical_matches(candidate, &dep.url))
}

/// Every stored SQLQuery/SQLView Library of the tenant (bounded by
/// [`MAX_LIBRARIES_SCANNED`]).
pub(crate) async fn stored_sql_libraries<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
) -> Result<Vec<Value>, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let mut query = SearchQuery::new("Library");
    query.count = Some(MAX_LIBRARIES_SCANNED);
    let libraries: Vec<Value> = match state.storage().search(tenant, &query).await {
        Ok(result) => result
            .resources
            .items
            .into_iter()
            .map(|r| r.content().clone())
            .collect(),
        Err(StorageError::Backend(BackendError::UnsupportedCapability { .. })) => {
            let Some(scan) = state.storage().resource_scan() else {
                return Ok(Vec::new());
            };
            use futures::StreamExt;
            let mut stream = scan.scan_resources(tenant, "Library").await.map_err(|e| {
                RestError::InternalError {
                    message: format!("scanning Libraries failed: {e}"),
                }
            })?;
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                out.push(item.map_err(|e| RestError::InternalError {
                    message: format!("scanning Libraries failed: {e}"),
                })?);
                if out.len() as u32 >= MAX_LIBRARIES_SCANNED {
                    break;
                }
            }
            out
        }
        Err(e) => return Err(e.into()),
    };
    if libraries.len() as u32 >= MAX_LIBRARIES_SCANNED {
        warn!(
            "tenant has {MAX_LIBRARIES_SCANNED} or more Libraries; a ValueSet write checks only \
             the first {MAX_LIBRARIES_SCANNED} for dependants"
        );
    }
    Ok(libraries
        .into_iter()
        .filter(|lib| parse_sqlquery_library(lib).is_ok())
        .collect())
}

/// The response for a conditional create that committed the Library and then
/// failed phase B (D13 c, `[R6-2]`): `Location`, `ETag` and `Last-Modified`
/// exactly as the 201 would carry them, and an OperationOutcome whose second
/// issue says the create committed and names the only remedy.
pub(crate) fn created_unmaterialized_outcome(
    resource_type: &str,
    id: &str,
    version_id: &str,
    location: &str,
    failure: &ApplyFailure,
) -> Value {
    let (_, code, message) = failure.error.client_response();
    json!({
        "resourceType": "OperationOutcome",
        "issue": [
            {"severity": "error", "code": code, "diagnostics": format!(
                "materializing terminology dependencies failed: {message}; already materialized: \
                 [{}]; still to materialize: [{}]",
                failure.written.join(", "), failure.remaining.join(", ")
            )},
            {"severity": "information", "code": "informational",
             "details": {"coding": [{"system": SOF_TERMINOLOGY_TAG_SYSTEM, "code": CODE_CREATED_UNMATERIALIZED}]},
             "diagnostics": format!(
                "{resource_type}/{id} (version {version_id}) was created; its terminology \
                 dependencies were not materialized: {}. Retry with PUT {location} (the same \
                 conditional POST would match the stored {resource_type} and return 200 without \
                 materializing).",
                failure.remaining.join(", ")
             )}
        ]
    })
}

// ============================================================================
// Direct-path helpers (create / update / patch handlers)
// ============================================================================

/// Phase A for a direct write of `resource`: `Some(plan)` for a SQL Library
/// (possibly empty), `None` for any other resource type or `off` mode.
pub(crate) async fn plan_for_direct_write<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    principal: Option<&Principal>,
    resource_type: &str,
    resource: &Value,
    budget: &MaterializeBudget,
) -> Result<Option<MaterializationPlan>, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    if resource_type != "Library"
        || MaterializeMode::from_config(state.config()) == MaterializeMode::Off
    {
        return Ok(None);
    }
    let source = StorageSource { state, tenant };
    plan_library_materialization(state, tenant, principal, resource, &source, true, budget)
        .await
        .map(Some)
}

/// Phase B for a direct write, answering an ordinary write's 422 on failure
/// (D13 b).
pub(crate) async fn apply_for_direct_write<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    plan: Option<MaterializationPlan>,
) -> Result<(), RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let Some(plan) = plan else { return Ok(()) };
    if plan.is_empty() {
        return Ok(());
    }
    apply_materialization(state, tenant, plan)
        .await
        .map(|_| ())
        .map_err(|failure| failure.into_rest_error())
}

/// T2 for a direct write: a ValueSet candidate is transformed, anything else
/// is returned as is.
pub(crate) async fn transform_for_direct_write<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    resource: Value,
    budget: &MaterializeBudget,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    if resource_type != "ValueSet" {
        return Ok(resource);
    }
    materialize_value_set_candidate(state, tenant, resource, &[], budget)
        .await
        .map(|(candidate, _)| candidate)
}

/// Resolves a reference the way the direct paths do, for callers that hold
/// a `Value` and not a [`DependencySource`]: kept for parity with
/// [`resolve_resource_canonical_or_relative`].
#[allow(dead_code)]
pub(crate) async fn resolve_direct<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    resolve_resource_canonical_or_relative(state, tenant, resource_type, reference).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_tag_is_idempotent_and_tolerates_odd_meta() {
        let mut vs = json!({"resourceType": "ValueSet", "meta": {"tag": "not-an-array"}});
        add_tag(&mut vs, TAG_MATERIALIZED_EXPANSION);
        add_tag(&mut vs, TAG_MATERIALIZED_EXPANSION);
        assert_eq!(
            vs["meta"]["tag"],
            json!([{"system": SOF_TERMINOLOGY_TAG_SYSTEM, "code": TAG_MATERIALIZED_EXPANSION}])
        );
        let mut no_meta = json!({"resourceType": "ValueSet"});
        add_tag(&mut no_meta, TAG_FETCHED_FROM_TERMINOLOGY_SERVER);
        assert_eq!(
            no_meta["meta"]["tag"][0]["code"],
            TAG_FETCHED_FROM_TERMINOLOGY_SERVER
        );
    }

    #[test]
    fn library_depends_on_matches_pinned_and_unpinned_canonicals() {
        use base64::Engine as _;
        let lib = json!({
            "resourceType": "Library", "status": "active",
            "type": {"coding": [{"system": helios_sof::canonical::LIBRARY_TYPES_CODE_SYSTEM, "code": "sql-query"}]},
            "content": [{"contentType": "application/sql",
                         "data": base64::engine::general_purpose::STANDARD.encode("SELECT 1")}],
            "relatedArtifact": [{"type": "depends-on", "label": "vs", "resource": "http://example.org/vs|2"}]
        });
        let v2 =
            json!({"resourceType": "ValueSet", "url": "http://example.org/vs", "version": "2"});
        let v1 =
            json!({"resourceType": "ValueSet", "url": "http://example.org/vs", "version": "1"});
        assert!(library_depends_on(&lib, &v2));
        assert!(!library_depends_on(&lib, &v1));
        let cql =
            json!({"resourceType": "Library", "type": {"coding": [{"code": "logic-library"}]}});
        assert!(!library_depends_on(&cql, &v2));
    }

    #[test]
    fn strict_errors_map_to_the_spec_codes() {
        let content = MaterializeError::Content {
            url: "u".into(),
            error: TerminologyContentError::TooManyMembers { max: 1 },
        }
        .into_rest_error();
        match content {
            RestError::ValidationFailed { outcome } => {
                assert_eq!(outcome["issue"][0]["code"], "too-costly")
            }
            other => panic!("{other:?}"),
        }
        let network = MaterializeError::Server {
            url: "u".into(),
            error: TerminologyError::Network("refused".into()),
        }
        .into_rest_error();
        match network {
            RestError::ValidationFailed { outcome } => {
                assert_eq!(outcome["issue"][0]["code"], "exception");
                assert!(
                    outcome["issue"][0]["diagnostics"]
                        .as_str()
                        .unwrap()
                        .contains("refused")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            MaterializeError::Forbidden {
                message: "m".into()
            }
            .into_rest_error(),
            RestError::Forbidden { .. }
        ));
        let budget = MaterializeError::BudgetExceeded { secs: 20 }.into_rest_error();
        match budget {
            RestError::ValidationFailed { outcome } => {
                assert_eq!(outcome["issue"][0]["code"], "invalid");
                assert!(
                    outcome["issue"][0]["diagnostics"]
                        .as_str()
                        .unwrap()
                        .contains("HFS_REQUEST_TIMEOUT")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn created_unmaterialized_outcome_names_the_put_remedy() {
        let failure = ApplyFailure {
            error: RestError::InternalError {
                message: "disk full".into(),
            },
            written: vec!["http://example.org/a".into()],
            remaining: vec!["http://example.org/b".into()],
        };
        let outcome = created_unmaterialized_outcome(
            "Library",
            "l1",
            "1",
            "http://localhost/Library/l1",
            &failure,
        );
        assert_eq!(
            outcome["issue"][1]["details"]["coding"][0]["code"],
            CODE_CREATED_UNMATERIALIZED
        );
        let text = outcome["issue"][1]["diagnostics"].as_str().unwrap();
        assert!(
            text.contains("Library/l1 (version 1) was created"),
            "{text}"
        );
        assert!(text.contains("PUT http://localhost/Library/l1"), "{text}");
        assert!(text.contains("http://example.org/b"), "{text}");
    }

    #[test]
    fn mode_parses_with_strict_as_the_fallback() {
        let mut config = ServerConfig::for_testing();
        assert_eq!(
            MaterializeMode::from_config(&config),
            MaterializeMode::Strict
        );
        config.sof_terminology_materialize = "lenient".into();
        assert_eq!(
            MaterializeMode::from_config(&config),
            MaterializeMode::Lenient
        );
        config.sof_terminology_materialize = "off".into();
        assert_eq!(MaterializeMode::from_config(&config), MaterializeMode::Off);
    }
}
