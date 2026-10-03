//! Two-phase dependency-graph resolver and executor, shared by `$sql-run`
//! ([`super::sqlquery`]) and `$sql-export`
//! ([`super::export`] / [`crate::export::in_memory`]).
//!
//! A SQLQuery or SQLView Library names its table sources through
//! `relatedArtifact[type=depends-on]` entries. Before this module existed,
//! only the subject's *direct* dependencies were resolved, and every one had
//! to be a leaf ViewDefinition. A SQLView Library's own SQL result can now
//! serve as a table source for whoever depends on it, so the dependency
//! graph can be several levels deep (`SQLQuery -> SQLView -> SQLView ->
//! ViewDefinition`, etc.) — see design #567.
//!
//! Resolution happens in two phases:
//!
//! - **Phase 1 — plan** ([`build_plan`]): walk the graph iteratively (no
//!   recursion in the Rust call stack), fetching every artifact but running
//!   no SQL. Nodes are deduped by canonical URL, so a dependency shared by
//!   two branches (a diamond) is only visited once. Detects cycles, depth
//!   overflow (fixed at [`MAX_GRAPH_DEPTH`]), unresolvable URLs, artifacts of
//!   the wrong type, SQLView artifacts that violate the profile's
//!   `parameter 0..0` constraint, and label collisions. Every structural
//!   error found is collected — nothing is executed until the whole graph
//!   has been checked — and returned together. On success, produces a
//!   [`GraphPlan`] whose nodes are topologically ordered (leaves first).
//! - **Phase 2 — execution** ([`execute_plan`]): materializes the plan's
//!   nodes in order — leaf ViewDefinitions via the wired `SofRunner`,
//!   interior SQLView nodes by running their own (already-validated) SQL
//!   against the tables materialized so far — then runs the subject's own
//!   SQL. A node shared by multiple edges is materialized once; each edge's
//!   `relatedArtifact.label` becomes a table alias for whichever SQL
//!   references it.
//!
//! [`build_plan`] takes its artifact source through the [`ArtifactFetcher`]
//! trait, so it is fully testable without HTTP or a real storage backend —
//! see the `#[cfg(test)]` module below.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::{RowStream, SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use helios_sof::sqlquery::engine::ColumnSchema;
use helios_sof::sqlquery::{
    BoundParam, DependencyKind, DependsOnView, InMemorySqlEngine, QueryResult, SqlQueryError,
    TableSchema,
};
use serde_json::{Value, json};
use tracing::{debug, warn};

use super::references::{canonical_matches, resolve_resource_canonical_or_relative};
use super::sqlquery::{sqlquery_err_to_rest, validate_select_only};
use super::subject::{SubjectKind, classify_subject};
use crate::error::RestError;
use crate::state::AppState;

/// Maximum depth of the dependency graph, the subject's own direct
/// dependencies counting as depth 1. Fixed per design #567 — not
/// configurable; raising it, if ever needed, is a one-line change here.
pub(crate) const MAX_GRAPH_DEPTH: usize = 16;

/// Resource kinds this resolver accepts as a dependency
/// (`relatedArtifact.resource`), from either storage or the inline `context`
/// list — the check applies identically after the fetch, regardless of
/// origin (design #568). A SQLQuery Library may only be an operation
/// *subject*, never a dependency of something else, so it is deliberately
/// absent here. Extend this list — not the `match` in [`build_plan`] — when
/// the spec adds further artifact kinds (e.g. terminology) as legal
/// dependencies.
const ACCEPTED_DEPENDENCY_KINDS: [SubjectKind; 2] =
    [SubjectKind::ViewDefinition, SubjectKind::SqlView];

// ============================================================================
// Plan types
// ============================================================================

/// One dependency edge: the label a consumer's SQL selects from, and the
/// internal table name of the node it resolves to.
///
/// The label is per-edge (the same node may be referenced under different
/// labels by different consumers, or the same label by several), while the
/// internal name identifies the node itself. See the module docs.
#[derive(Debug, Clone)]
pub(crate) struct Edge {
    /// The SQL identifier this edge's consumer selects from
    /// (`relatedArtifact.label`).
    pub label: String,
    /// The canonical internal table name the referenced node was
    /// materialized under.
    pub target_internal_name: String,
}

/// One node in the resolved, topologically-ordered plan (leaves first). Does
/// not include the subject itself — the subject's own SQL and dependency
/// edges are carried separately (see [`GraphPlan::subject_edges`]) since the
/// subject is never itself a dependency of anything.
#[derive(Debug, Clone)]
pub(crate) enum PlanNode {
    /// A leaf ViewDefinition, materialized via the wired `SofRunner`.
    Leaf {
        /// Canonical internal table name this node's rows are stored under.
        internal_name: String,
        /// The resolved ViewDefinition JSON.
        view: Value,
    },
    /// An interior SQLView Library: a query whose result becomes a table for
    /// whichever node(s) reference it.
    SqlView {
        /// Canonical internal table name this node's result is stored under.
        internal_name: String,
        /// The validated (SELECT-only) SQL text.
        sql: String,
        /// This node's own `depends-on` edges, resolved.
        edges: Vec<Edge>,
        /// The Library's `name`, or its canonical URL when it has none —
        /// how an error (see [`SqlQueryError::DependencyRowCapExceeded`])
        /// names this node. Not used for resolution.
        name: String,
    },
}

/// The output of Phase 1: a topologically-ordered (leaves first) dependency
/// graph, ready for Phase 2 execution.
#[derive(Debug, Clone)]
pub(crate) struct GraphPlan {
    /// Every dependency node, in materialization order.
    pub nodes: Vec<PlanNode>,
    /// The subject's own `depends-on` edges, resolved the same way as any
    /// interior node's.
    pub subject_edges: Vec<Edge>,
}

impl GraphPlan {
    /// Total number of dependency nodes in the plan — what
    /// `sof_sqlquery_max_vds` bounds (the whole graph, not just the
    /// subject's direct dependencies).
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Describes the operation's subject for [`build_plan`]. A ViewDefinition
/// subject never reaches the resolver — it has no dependency graph to walk.
pub(crate) struct SubjectNode<'a> {
    /// Canonical identity used to detect a dependency cycling back to the
    /// subject itself. `None` when the subject was supplied inline
    /// (`subjectResource`) and carries no `url`; such a subject can still
    /// appear in a cycle among its *descendants*, just not one that closes
    /// back on the subject by name.
    pub identity: Option<&'a str>,
    /// Whether the subject is a SQLView Library. SQLView profiles Library
    /// with `parameter 0..0`; a SQLView subject that itself declares
    /// parameters is a Phase 1 structural error, exactly like an interior
    /// SQLView node that does.
    pub is_sql_view: bool,
    /// Whether `Library.parameter` is empty on the subject. Only consulted
    /// when `is_sql_view` is `true`.
    pub parameters_empty: bool,
    /// The subject's own `depends-on` edges.
    pub depends_on: &'a [DependsOnView],
}

// ============================================================================
// Artifact fetching
// ============================================================================

/// Fetches a dependency artifact by canonical URL. Implemented against
/// storage for real requests ([`StorageArtifactFetcher`]) and against a fixed
/// in-memory map for this module's own dry unit tests.
#[async_trait]
pub(crate) trait ArtifactFetcher: Send + Sync {
    /// Resolves `url`. `Err(RestError::NotFound { .. })` means nothing at
    /// that URL matches; any other `Err` is an infrastructure fault that
    /// aborts resolution immediately rather than being aggregated with
    /// structural errors.
    async fn fetch(&self, url: &str) -> Result<Value, RestError>;
}

/// Storage-backed [`ArtifactFetcher`] used by the real HTTP handlers.
///
/// A dependency URL may name either a leaf ViewDefinition or an interior
/// SQLView Library, and neither URL form says which, so both resource types
/// are tried — ViewDefinition first, since it is the more common dependency
/// (mirrors [`super::subject::resolve_subject`]'s own dual-type lookup for
/// the subject itself).
pub(crate) struct StorageArtifactFetcher<'a, S> {
    state: &'a AppState<S>,
    tenant: &'a TenantContext,
}

impl<'a, S> StorageArtifactFetcher<'a, S> {
    /// Builds a fetcher scoped to one request's state and tenant.
    pub(crate) fn new(state: &'a AppState<S>, tenant: &'a TenantContext) -> Self {
        Self { state, tenant }
    }
}

#[async_trait]
impl<S> ArtifactFetcher for StorageArtifactFetcher<'_, S>
where
    S: SearchProvider + Send + Sync + 'static,
{
    async fn fetch(&self, url: &str) -> Result<Value, RestError> {
        match resolve_resource_canonical_or_relative(self.state, self.tenant, "ViewDefinition", url)
            .await
        {
            Ok(v) => Ok(v),
            Err(RestError::NotFound { .. }) => {
                resolve_resource_canonical_or_relative(self.state, self.tenant, "Library", url)
                    .await
            }
            Err(other) => Err(other),
        }
    }
}

/// Resolves one dependency's artifact: the server's own storage is checked
/// first via `fetcher`; only when storage has nothing at `url` is the inline
/// `context` list consulted, matched with the same [`canonical_matches`] rule
/// `resolve_by_canonical_url` uses for storage (including `url|version`
/// treatment). This order is deliberate (design #568): `context` exists to
/// fill gaps the server cannot resolve, not to override what it already has,
/// so a `context` entry whose URL the server also resolves is silently
/// ignored — there is nowhere to surface a warning in a streamed response.
/// Returns `Err(RestError::NotFound)` when neither source has a match.
async fn fetch_dependency(
    fetcher: &dyn ArtifactFetcher,
    inline: &[Value],
    url: &str,
) -> Result<Value, RestError> {
    match fetcher.fetch(url).await {
        Ok(v) => Ok(v),
        Err(RestError::NotFound { .. }) => inline
            .iter()
            .find(|artifact| canonical_matches(artifact, url))
            .cloned()
            .ok_or_else(|| RestError::NotFound {
                resource_type: "ViewDefinition or Library".to_string(),
                id: url.to_string(),
            }),
        Err(other) => Err(other),
    }
}

// ============================================================================
// Phase 1 — plan
// ============================================================================

/// One frame of the explicit (non-recursive) DFS stack: a node currently
/// being expanded, awaiting its own `depends_on` children before it can be
/// finalized into the plan.
struct Frame {
    /// Canonical identity of this node (its own `depends_on` URL, or the
    /// subject's identity for the root frame).
    identity: String,
    depends_on: Vec<DependsOnView>,
    next_idx: usize,
    /// This node's SQL. Unused for the root frame (the subject's SQL is
    /// executed by the caller, not by [`execute_plan`]).
    sql: String,
    resolved_edges: Vec<Edge>,
    /// The label the parent used to reference this node. `None` for the
    /// root frame, which has no parent.
    label_for_parent: Option<String>,
    /// Display name of this node (see [`PlanNode::SqlView::name`]). Unused
    /// for the root frame.
    name: String,
}

fn next_internal_name(counter: &mut usize) -> String {
    let name = format!("__sof_node_{counter}");
    *counter += 1;
    name
}

fn sqlview_parameter_error(identity: &str) -> RestError {
    RestError::BadRequest {
        message: format!(
            "'{identity}' is a SQLView Library; the SQLView profile constrains \
             Library.parameter to 0..0, but this Library declares one or more parameters"
        ),
    }
}

/// Registers `label` as resolving to `internal_name`. Returns `Some(error)`
/// if `label` was already registered against a *different* target — the same
/// label used for two different dependencies anywhere in the request, which
/// the flat, single-namespace SQL engine cannot represent.
fn register_label(
    registry: &mut HashMap<String, String>,
    label: &str,
    internal_name: &str,
) -> Option<RestError> {
    match registry.get(label) {
        Some(existing) if existing != internal_name => Some(RestError::BadRequest {
            message: format!(
                "label '{label}' is used for two different dependencies in this request; \
                 each label must resolve to exactly one artifact"
            ),
        }),
        Some(_) => None,
        None => {
            registry.insert(label.to_string(), internal_name.to_string());
            None
        }
    }
}

/// Phase 1: walks the dependency graph rooted at `subject`, producing a
/// topologically-ordered [`GraphPlan`] or the aggregated list of every
/// structural problem found. Runs no SQL and touches the in-memory engine
/// not at all — only `fetcher` (and `inline`) perform I/O.
///
/// Iterative: the traversal is an explicit stack of [`Frame`]s, not Rust call
/// recursion, so the graph's depth is bounded only by [`MAX_GRAPH_DEPTH`] and
/// not by the Rust call stack.
pub(crate) async fn build_plan(
    fetcher: &dyn ArtifactFetcher,
    inline: &[Value],
    subject: SubjectNode<'_>,
) -> Result<GraphPlan, Vec<RestError>> {
    let mut errors: Vec<RestError> = Vec::new();

    if subject.is_sql_view && !subject.parameters_empty {
        errors.push(sqlview_parameter_error(
            subject.identity.unwrap_or("<subject>"),
        ));
    }

    let root_identity = subject.identity.unwrap_or("<subject>").to_string();
    let mut on_stack: HashSet<String> = HashSet::new();
    on_stack.insert(root_identity.clone());
    let mut path: Vec<String> = vec![root_identity.clone()];

    let mut stack: Vec<Frame> = vec![Frame {
        identity: root_identity,
        depends_on: subject.depends_on.to_vec(),
        next_idx: 0,
        sql: String::new(),
        resolved_edges: Vec::new(),
        label_for_parent: None,
        name: String::new(),
    }];

    let mut nodes: Vec<PlanNode> = Vec::new();
    let mut finished: HashMap<String, String> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    let mut label_registry: HashMap<String, String> = HashMap::new();
    let mut next_internal_id: usize = 0;
    let mut subject_edges: Vec<Edge> = Vec::new();

    loop {
        let is_finished_child = {
            let Some(top) = stack.last() else { break };
            top.next_idx >= top.depends_on.len()
        };

        if is_finished_child {
            let frame = stack.pop().expect("checked above");
            on_stack.remove(&frame.identity);
            path.pop();

            if stack.is_empty() {
                subject_edges = frame.resolved_edges;
                break;
            }

            let internal_name = next_internal_name(&mut next_internal_id);
            finished.insert(frame.identity.clone(), internal_name.clone());
            let label = frame
                .label_for_parent
                .clone()
                .expect("non-root frame always has a parent label");
            nodes.push(PlanNode::SqlView {
                internal_name: internal_name.clone(),
                sql: frame.sql,
                edges: frame.resolved_edges,
                name: frame.name,
            });

            if let Some(err) = register_label(&mut label_registry, &label, &internal_name) {
                errors.push(err);
            } else {
                let parent = stack.last_mut().expect("checked above");
                parent.resolved_edges.push(Edge {
                    label,
                    target_internal_name: internal_name,
                });
            }
            continue;
        }

        let dep = {
            let top = stack.last_mut().expect("checked above");
            let dep = top.depends_on[top.next_idx].clone();
            top.next_idx += 1;
            dep
        };

        if failed.contains(&dep.url) {
            continue;
        }

        if let Some(internal_name) = finished.get(&dep.url).cloned() {
            if let Some(err) = register_label(&mut label_registry, &dep.label, &internal_name) {
                errors.push(err);
            } else {
                let top = stack.last_mut().expect("checked above");
                top.resolved_edges.push(Edge {
                    label: dep.label.clone(),
                    target_internal_name: internal_name,
                });
            }
            continue;
        }

        if on_stack.contains(&dep.url) {
            let mut cycle_path = path.clone();
            cycle_path.push(dep.url.clone());
            errors.push(RestError::BadRequest {
                message: format!(
                    "dependency cycle detected: {}",
                    cycle_path.join(" \u{2192} ")
                ),
            });
            failed.insert(dep.url.clone());
            continue;
        }

        let depth = stack.len();
        if depth > MAX_GRAPH_DEPTH {
            errors.push(RestError::BadRequest {
                message: format!(
                    "dependency graph exceeds the maximum depth of {MAX_GRAPH_DEPTH} levels \
                     at '{}'",
                    dep.url
                ),
            });
            failed.insert(dep.url.clone());
            continue;
        }

        let artifact = match fetch_dependency(fetcher, inline, &dep.url).await {
            Ok(v) => v,
            Err(RestError::NotFound { .. }) => {
                errors.push(RestError::NotFound {
                    resource_type: "ViewDefinition or Library".to_string(),
                    id: dep.url.clone(),
                });
                failed.insert(dep.url.clone());
                continue;
            }
            Err(other) => return Err(vec![other]),
        };

        let kind = match classify_subject(&artifact) {
            Ok(k) => k,
            Err(_) => {
                let rt = artifact
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<absent>");
                errors.push(RestError::BadRequest {
                    message: format!(
                        "dependency '{}' must be a ViewDefinition or a SQLView Library, got \
                         resourceType='{rt}'",
                        dep.url
                    ),
                });
                failed.insert(dep.url.clone());
                continue;
            }
        };

        if !ACCEPTED_DEPENDENCY_KINDS.contains(&kind) {
            errors.push(RestError::BadRequest {
                message: format!(
                    "dependency '{}' is a SQLQuery Library; only a ViewDefinition or a \
                     SQLView Library may appear as a dependency",
                    dep.url
                ),
            });
            failed.insert(dep.url.clone());
            continue;
        }

        match kind {
            SubjectKind::ViewDefinition => {
                let internal_name = next_internal_name(&mut next_internal_id);
                finished.insert(dep.url.clone(), internal_name.clone());
                nodes.push(PlanNode::Leaf {
                    internal_name: internal_name.clone(),
                    view: artifact,
                });
                if let Some(err) = register_label(&mut label_registry, &dep.label, &internal_name) {
                    errors.push(err);
                } else {
                    let top = stack.last_mut().expect("checked above");
                    top.resolved_edges.push(Edge {
                        label: dep.label.clone(),
                        target_internal_name: internal_name,
                    });
                }
            }
            SubjectKind::SqlView => {
                let library = match helios_sof::sqlquery::parse_sqlquery_library(&artifact) {
                    Ok(l) => l,
                    Err(e) => {
                        errors.push(sqlquery_err_to_rest(e));
                        failed.insert(dep.url.clone());
                        continue;
                    }
                };
                if !library.parameters.is_empty() {
                    errors.push(sqlview_parameter_error(&dep.url));
                    failed.insert(dep.url.clone());
                    continue;
                }
                // Uniform validation regardless of origin (storage or
                // `context`): the same `sql-must-be-sql-expressions`
                // SELECT-only check the subject's own SQL passes before
                // `build_plan` is called applies here too, since this SQL
                // will run exactly the same way in Phase 2.
                if let Err(e) = validate_select_only(&library.sql) {
                    errors.push(e);
                    failed.insert(dep.url.clone());
                    continue;
                }
                on_stack.insert(dep.url.clone());
                path.push(dep.url.clone());
                let name = artifact_display_name(&artifact, &dep.url).to_string();
                stack.push(Frame {
                    identity: dep.url.clone(),
                    depends_on: library.depends_on,
                    next_idx: 0,
                    sql: library.sql,
                    resolved_edges: Vec::new(),
                    label_for_parent: Some(dep.label.clone()),
                    name,
                });
            }
            SubjectKind::SqlQuery => {
                unreachable!("rejected above by ACCEPTED_DEPENDENCY_KINDS")
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(GraphPlan {
        nodes,
        subject_edges,
    })
}

/// Enforces `sof_sqlquery_max_vds` against the whole resolved plan (every
/// node the graph reached, not just the subject's direct dependencies).
pub(crate) fn check_max_nodes(plan: &GraphPlan, max_vds: usize) -> Result<(), RestError> {
    let count = plan.node_count();
    if count > max_vds {
        return Err(RestError::UnprocessableEntity {
            message: format!("dependency graph has {count} nodes; max allowed is {max_vds}"),
        });
    }
    Ok(())
}

/// Collapses one or more Phase 1 structural errors into a single
/// [`RestError`]. A single error keeps its own precise status code
/// unchanged; multiple simultaneous errors — e.g. an unresolved dependency
/// alongside a cycle — are combined into one [`RestError::MultiIssue`]
/// carrying one OperationOutcome `issue` per problem, since the request as a
/// whole cannot proceed until every one of them is fixed.
pub(crate) fn errors_to_rest_error(mut errors: Vec<RestError>) -> RestError {
    if errors.len() == 1 {
        return errors.pop().expect("len checked above");
    }
    let issues: Vec<Value> = errors
        .iter()
        .map(|e| {
            let (_, code, message) = e.client_response();
            json!({"severity": "error", "code": code, "diagnostics": message})
        })
        .collect();
    RestError::MultiIssue {
        outcome: json!({"resourceType": "OperationOutcome", "issue": issues}),
    }
}

// ============================================================================
// Phase 2 — execution
// ============================================================================

/// Row-cap and timeout budget for Phase 2 execution, shared by `$sql-run`
/// and `$sql-export`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExecLimits {
    /// Maximum rows materialized per dependency node (leaf ViewDefinition or
    /// interior SQLView result) — a hard error when exceeded, since silently
    /// truncating a table other SQL joins/aggregates over would silently
    /// change query semantics.
    pub max_source_rows_per_vd: usize,
    /// Maximum rows returned by the subject's own SQL — a silent cap per SoF
    /// v2 PR #353.
    pub max_rows: usize,
    /// SQL execution timeout in seconds, applied per statement (each
    /// interior SQLView's own SQL, and the subject's).
    pub timeout_secs: u64,
}

/// Materializes every node in `plan`, then executes `subject_sql` against
/// the resulting tables, and returns the result plus the schema of every
/// leaf ViewDefinition materialized (in plan order) — callers use the latter
/// to refine output column types against VD-declared FHIR types
/// (`$sql-run`'s `_format=fhir`).
///
/// Leaves are materialized via `runner.run_view` (async I/O against
/// storage). Interior SQLView nodes and the subject's own SQL run on a
/// blocking thread under the same watchdog-timeout/interrupt mechanism, one
/// statement at a time — `limits.timeout_secs` bounds each of them
/// individually, not the whole call.
///
/// A node referenced by more than one label (a diamond, or simply two
/// consumers using different names for the same dependency) is computed
/// once; each label is a physical copy of that one result, so it is
/// addressable at every point where a consumer's SQL selects from it. Phase
/// 1 guarantees every label maps to exactly one node.
///
/// Returns a typed [`SqlQueryError`]. In particular, a
/// [`SqlQueryError::SourceStream`] means the *source* failed — a storage
/// error, a backend statement timeout, or a lost connection while streaming
/// a leaf ViewDefinition's rows — never that the Library or its
/// ViewDefinitions were malformed; callers must not treat it as a client
/// error.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_plan(
    mut engine: InMemorySqlEngine,
    runner: &Arc<dyn SofRunner>,
    tenant: &TenantContext,
    filters: &ViewFilters,
    plan: &GraphPlan,
    subject_sql: &str,
    subject_bindings: &[BoundParam],
    limits: ExecLimits,
) -> Result<(QueryResult, Vec<TableSchema>), SqlQueryError> {
    let labels_by_target = labels_by_target(plan);
    let mut leaf_schemas: Vec<TableSchema> = Vec::new();

    for node in &plan.nodes {
        match node {
            PlanNode::Leaf {
                internal_name,
                view,
            } => {
                // One column per declared name: `from_view_definition` repeats
                // every `unionAll` branch's columns, which `CREATE TABLE`
                // rejects as duplicates. Rows are inserted by column name, so
                // each union row's values land in their own columns. A view
                // without `unionAll` gets the same columns, in the same order.
                let schema = TableSchema::sql_output_layout(view);
                engine.create_table(internal_name, &schema)?;
                let view_label = artifact_display_name(view, internal_name);
                let start = std::time::Instant::now();
                let row_stream = match runner.run_view(tenant, view.clone(), filters.clone()).await
                {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(
                            table = %internal_name,
                            view = %view_label,
                            error = %e,
                            "SQL query dependency failed to materialize"
                        );
                        return Err(SqlQueryError::SourceStream(format!(
                            "dependency failed to materialize: {e}"
                        )));
                    }
                };
                let row_stream = adapt_row_stream(row_stream);
                let (returned_engine, inserted) = match engine
                    .insert_rows(
                        internal_name,
                        &schema,
                        Box::pin(row_stream),
                        limits.max_source_rows_per_vd,
                    )
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        // The engine's bare row-cap error names neither the
                        // dependency nor the setting; name both (#1473).
                        let e = match e {
                            SqlQueryError::RowCapExceeded { max } => {
                                SqlQueryError::DependencyRowCapExceeded {
                                    label: dependency_label(
                                        &labels_by_target,
                                        internal_name,
                                        view_label,
                                    )
                                    .to_string(),
                                    kind: DependencyKind::ViewDefinition,
                                    view: view_label.to_string(),
                                    max,
                                }
                            }
                            other => other,
                        };
                        warn!(
                            table = %internal_name,
                            view = %view_label,
                            error = %e,
                            "SQL query dependency failed to materialize"
                        );
                        return Err(e);
                    }
                };
                engine = returned_engine;
                let elapsed_ms = start.elapsed().as_millis();
                debug!(
                    table = %internal_name,
                    view = %view_label,
                    rows = inserted,
                    elapsed_ms,
                    "materialized SQL query dependency"
                );
                leaf_schemas.push(schema);
                engine = materialize_labels(engine, internal_name, &labels_by_target).await?;
            }
            PlanNode::SqlView {
                internal_name,
                sql,
                name,
                ..
            } => {
                let start = std::time::Instant::now();
                let (returned_engine, result) = run_select_with_timeout(
                    engine,
                    sql.clone(),
                    Vec::new(),
                    limits.max_source_rows_per_vd.saturating_add(1),
                    limits.timeout_secs,
                )
                .await?;
                engine = returned_engine;
                if result.rows.len() > limits.max_source_rows_per_vd {
                    return Err(SqlQueryError::DependencyRowCapExceeded {
                        label: dependency_label(&labels_by_target, internal_name, name).to_string(),
                        kind: DependencyKind::SqlView,
                        view: name.clone(),
                        max: limits.max_source_rows_per_vd,
                    });
                }
                let rows = result.rows.len();
                engine = materialize_query_result(engine, internal_name, &result).await?;
                let elapsed_ms = start.elapsed().as_millis();
                debug!(
                    table = %internal_name,
                    rows,
                    elapsed_ms,
                    "materialized SQL view node"
                );
                engine = materialize_labels(engine, internal_name, &labels_by_target).await?;
            }
        }
    }

    let (_, result) = run_select_with_timeout(
        engine,
        subject_sql.to_string(),
        subject_bindings.to_vec(),
        limits.max_rows,
        limits.timeout_secs,
    )
    .await?;

    Ok((result, leaf_schemas))
}

/// Picks a human-readable name for a dependency artifact (a ViewDefinition
/// or a SQL View Library), for log lines and error messages: `artifact.name`
/// if it is a string, else `artifact.url` if it is a string, else `fallback`
/// (the plan's internal table name or the canonical URL the artifact was
/// resolved from), which is always present.
fn artifact_display_name<'a>(artifact: &'a Value, fallback: &'a str) -> &'a str {
    artifact
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| artifact.get("url").and_then(|v| v.as_str()))
        .unwrap_or(fallback)
}

/// The label a dependency's consumer selects it by, for error messages.
/// A node aliased by several labels (a diamond, or two names for one
/// dependency) is named by the alphabetically first one so the message is
/// deterministic; `fallback` covers a node no edge points at, which a plan
/// built by [`build_plan`] never contains.
fn dependency_label<'a>(
    labels_by_target: &'a HashMap<String, Vec<String>>,
    internal_name: &str,
    fallback: &'a str,
) -> &'a str {
    labels_by_target
        .get(internal_name)
        .and_then(|labels| labels.iter().min())
        .map_or(fallback, String::as_str)
}

/// Groups every distinct label used anywhere in the plan by the internal
/// table name it aliases. Phase 1's [`register_label`] guarantees a label
/// never targets two different nodes, so each label appears under exactly
/// one target — but the *same* label may legitimately be declared by more
/// than one consumer for that target (e.g. two SQLView nodes each depending
/// on a shared node under the same label name), so this dedupes per target
/// to materialize each label table once.
fn labels_by_target(plan: &GraphPlan) -> HashMap<String, Vec<String>> {
    let mut by_target: HashMap<String, HashSet<String>> = HashMap::new();
    for node in &plan.nodes {
        if let PlanNode::SqlView { edges, .. } = node {
            for e in edges {
                by_target
                    .entry(e.target_internal_name.clone())
                    .or_default()
                    .insert(e.label.clone());
            }
        }
    }
    for e in &plan.subject_edges {
        by_target
            .entry(e.target_internal_name.clone())
            .or_default()
            .insert(e.label.clone());
    }
    by_target
        .into_iter()
        .map(|(target, labels)| (target, labels.into_iter().collect()))
        .collect()
}

/// Exposes `internal_name` under every label that aliases it, so a
/// consumer's SQL — which addresses its dependencies by label, not by
/// internal name — finds it. Each label is a view over the internal table
/// (`CREATE VIEW "<label>" AS SELECT * FROM "<internal>"`): no rows are
/// copied, and the DDL runs on a blocking thread like every other SQLite
/// statement in the plan. A node with no referencing label is a no-op
/// (never happens in practice: every plan node exists only because some
/// edge pointed at it).
async fn materialize_labels(
    engine: InMemorySqlEngine,
    internal_name: &str,
    labels_by_target: &HashMap<String, Vec<String>>,
) -> Result<InMemorySqlEngine, SqlQueryError> {
    let Some(labels) = labels_by_target.get(internal_name) else {
        return Ok(engine);
    };
    let internal_name = internal_name.to_string();
    let labels = labels.clone();
    tokio::task::spawn_blocking(move || {
        for label in &labels {
            engine.create_view(label, &internal_name)?;
        }
        Ok(engine)
    })
    .await
    .map_err(|e| SqlQueryError::Internal(format!("sqlquery worker panicked: {e}")))?
}

/// Builds a [`TableSchema`] from a [`QueryResult`]'s columns and inferred
/// types — used for interior SQLView nodes, which have no VD-declared column
/// types to draw on.
fn schema_from_query_result(result: &QueryResult) -> TableSchema {
    TableSchema {
        columns: result
            .columns
            .iter()
            .zip(result.column_types.iter())
            .map(|(name, ty)| ColumnSchema {
                name: name.clone(),
                fhir_type: ty.clone(),
            })
            .collect(),
    }
}

/// Materializes a [`QueryResult`] into a fresh physical table named
/// `table_name`.
async fn materialize_query_result(
    engine: InMemorySqlEngine,
    table_name: &str,
    result: &QueryResult,
) -> Result<InMemorySqlEngine, SqlQueryError> {
    let schema = schema_from_query_result(result);
    engine.create_table(table_name, &schema)?;
    let rows: Vec<Result<Value, String>> = result
        .rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in result.columns.iter().enumerate() {
                let v = row.get(i).cloned().flatten().unwrap_or(Value::Null);
                obj.insert(col.clone(), v);
            }
            Ok(Value::Object(obj))
        })
        .collect();
    let cap = result.rows.len();
    let stream = futures::stream::iter(rows);
    let (returned_engine, _inserted) = engine
        .insert_rows(table_name, &schema, Box::pin(stream), cap)
        .await?;
    Ok(returned_engine)
}

/// Runs one SELECT statement on a blocking thread under a watchdog timeout
/// (mirrors the exact mechanism `$sql-run` has always used for the
/// subject's own query), returning ownership of `engine` alongside the
/// result so the caller can keep using it for the next node.
async fn run_select_with_timeout(
    engine: InMemorySqlEngine,
    sql: String,
    bindings: Vec<BoundParam>,
    max_rows: usize,
    timeout_secs: u64,
) -> Result<(InMemorySqlEngine, QueryResult), SqlQueryError> {
    let interrupt = engine.interrupt_handle();
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
        interrupt.interrupt();
    });
    let (engine, exec_result) = tokio::task::spawn_blocking(move || {
        let r = engine.execute_select(&sql, &bindings, max_rows);
        (engine, r)
    })
    .await
    .map_err(|e| SqlQueryError::Internal(format!("sqlquery worker panicked: {e}")))?;
    watchdog.abort();
    match exec_result {
        Ok(r) => Ok((engine, r)),
        Err(e) if e.to_string().contains("interrupted") => {
            Err(SqlQueryError::Timeout { secs: timeout_secs })
        }
        Err(e) => Err(e),
    }
}

/// Converts a `RowStream<Result<Value, SofError>>` into a
/// `Stream<Item = Result<Value, String>>` for the engine (it doesn't know the
/// persistence crate's error type).
fn adapt_row_stream(stream: RowStream) -> impl Stream<Item = Result<Value, String>> + Send {
    use futures::StreamExt;
    stream.map(|r| r.map_err(|e| e.to_string()))
}

// ============================================================================
// Dry unit tests — Phase 1 only, no HTTP, no storage, no SQL execution.
// ============================================================================

#[cfg(test)]
mod tests {
    use helios_persistence::core::sof_runner::SofError;
    use helios_persistence::tenant::{TenantId, TenantPermissions};

    use super::*;

    /// A fixed in-memory [`ArtifactFetcher`] for dry Phase 1 tests.
    struct MapFetcher(HashMap<String, Value>);

    #[async_trait]
    impl ArtifactFetcher for MapFetcher {
        async fn fetch(&self, url: &str) -> Result<Value, RestError> {
            self.0.get(url).cloned().ok_or_else(|| RestError::NotFound {
                resource_type: "ViewDefinition or Library".to_string(),
                id: url.to_string(),
            })
        }
    }

    fn view_definition(url: &str) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "url": url,
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id", "type": "id"}]}]
        })
    }

    fn sql_view(url: &str, sql: &str, depends_on: &[(&str, &str)]) -> Value {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(sql.as_bytes());
        json!({
            "resourceType": "Library",
            "url": url,
            "status": "active",
            "type": {"coding": [{
                "system": helios_sof::canonical::LIBRARY_TYPES_CODE_SYSTEM,
                "code": "sql-view"
            }]},
            "content": [{"contentType": "application/sql", "data": data}],
            "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
                "type": "depends-on",
                "label": label,
                "resource": target
            })).collect::<Vec<_>>()
        })
    }

    fn depends_on(label: &str, url: &str) -> DependsOnView {
        DependsOnView {
            label: label.to_string(),
            url: url.to_string(),
        }
    }

    fn subject_node<'a>(depends_on: &'a [DependsOnView]) -> SubjectNode<'a> {
        SubjectNode {
            identity: Some("http://example.org/subject"),
            is_sql_view: false,
            parameters_empty: true,
            depends_on,
        }
    }

    #[tokio::test]
    async fn three_level_chain_resolves_leaves_first() {
        // SQLQuery(subject) -> SQLView(mid) -> SQLView(inner) -> ViewDefinition(leaf)
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/mid".to_string(),
                sql_view(
                    "http://example.org/mid",
                    "SELECT * FROM inner_t",
                    &[("inner_t", "http://example.org/inner")],
                ),
            ),
            (
                "http://example.org/inner".to_string(),
                sql_view(
                    "http://example.org/inner",
                    "SELECT * FROM leaf_t",
                    &[("leaf_t", "http://example.org/leaf")],
                ),
            ),
            (
                "http://example.org/leaf".to_string(),
                view_definition("http://example.org/leaf"),
            ),
        ]));

        let deps = vec![depends_on("mid_t", "http://example.org/mid")];
        let plan = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect("plan should resolve");

        assert_eq!(plan.node_count(), 3, "leaf + 2 SQLView nodes");
        // Leaves-first: the first node must be the ViewDefinition leaf.
        assert!(matches!(plan.nodes[0], PlanNode::Leaf { .. }));
        assert!(matches!(plan.nodes[1], PlanNode::SqlView { .. }));
        assert!(matches!(plan.nodes[2], PlanNode::SqlView { .. }));
        assert_eq!(plan.subject_edges.len(), 1);
        assert_eq!(plan.subject_edges[0].label, "mid_t");
    }

    #[tokio::test]
    async fn diamond_dependency_is_materialized_once() {
        // subject -> a -> shared, subject -> b -> shared
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/a".to_string(),
                sql_view(
                    "http://example.org/a",
                    "SELECT * FROM s",
                    &[("s", "http://example.org/shared")],
                ),
            ),
            (
                "http://example.org/b".to_string(),
                sql_view(
                    "http://example.org/b",
                    "SELECT * FROM s",
                    &[("s", "http://example.org/shared")],
                ),
            ),
            (
                "http://example.org/shared".to_string(),
                view_definition("http://example.org/shared"),
            ),
        ]));

        let deps = vec![
            depends_on("a_t", "http://example.org/a"),
            depends_on("b_t", "http://example.org/b"),
        ];
        let plan = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect("plan should resolve");

        // shared + a + b = 3 nodes, not 4: the shared leaf is materialized once.
        assert_eq!(plan.node_count(), 3);
        let leaf_count = plan
            .nodes
            .iter()
            .filter(|n| matches!(n, PlanNode::Leaf { .. }))
            .count();
        assert_eq!(leaf_count, 1, "the shared leaf must appear exactly once");
    }

    #[tokio::test]
    async fn cycle_is_reported_with_the_full_path() {
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/a".to_string(),
                sql_view(
                    "http://example.org/a",
                    "SELECT * FROM b_t",
                    &[("b_t", "http://example.org/b")],
                ),
            ),
            (
                "http://example.org/b".to_string(),
                sql_view(
                    "http://example.org/b",
                    "SELECT * FROM a_t",
                    &[("a_t", "http://example.org/a")],
                ),
            ),
        ]));

        let deps = vec![depends_on("a_t", "http://example.org/a")];
        let errors = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect_err("a cycle must be rejected");

        assert_eq!(errors.len(), 1);
        let RestError::BadRequest { message } = &errors[0] else {
            panic!("expected a 400, got {:?}", errors[0]);
        };
        assert!(message.contains("cycle"), "{message}");
        assert!(message.contains("http://example.org/a"), "{message}");
        assert!(message.contains("http://example.org/b"), "{message}");
    }

    #[tokio::test]
    async fn depth_seventeen_is_rejected_but_sixteen_is_not() {
        // Build a chain totaling `depth` edges from the subject down to a
        // leaf ViewDefinition (`depth - 1` interior SQLView levels, then the
        // final leaf edge) — `depth` matches exactly what the resolver's own
        // depth counter measures (subject's first edge = depth 1, ...,
        // final edge into the leaf = depth `depth`). Each level uses its own
        // distinct label: every `relatedArtifact.label` in the whole request
        // is materialized as a physical table under that exact name for the
        // lifetime of the execution (see `execute_plan`'s
        // `labels_by_target`/`materialize_labels`), so labels form a single
        // flat namespace across the *entire* graph, not just within one
        // node.
        fn chain(fetcher_map: &mut HashMap<String, Value>, depth: usize) -> Vec<DependsOnView> {
            let leaf_url = "http://example.org/level-leaf".to_string();
            fetcher_map.insert(leaf_url.clone(), view_definition(&leaf_url));
            let mut next_target = leaf_url;
            for i in (1..depth).rev() {
                let url = format!("http://example.org/level-{i}");
                let label = format!("t{i}");
                fetcher_map.insert(
                    url.clone(),
                    sql_view(
                        &url,
                        &format!("SELECT * FROM {label}"),
                        &[(label.as_str(), &next_target)],
                    ),
                );
                next_target = url;
            }
            vec![DependsOnView {
                label: "t0".to_string(),
                url: next_target,
            }]
        }

        let mut map16 = HashMap::new();
        let deps16 = chain(&mut map16, 16);
        let fetcher16 = MapFetcher(map16);
        let plan16 = build_plan(&fetcher16, &[], subject_node(&deps16)).await;
        assert!(plan16.is_ok(), "16 levels must resolve: {plan16:?}");

        let mut map17 = HashMap::new();
        let deps17 = chain(&mut map17, 17);
        let fetcher17 = MapFetcher(map17);
        let errors17 = build_plan(&fetcher17, &[], subject_node(&deps17))
            .await
            .expect_err("17 levels must be rejected");
        assert_eq!(errors17.len(), 1);
        let RestError::BadRequest { message } = &errors17[0] else {
            panic!("expected a 400, got {:?}", errors17[0]);
        };
        assert!(message.contains("16"), "{message}");
    }

    #[tokio::test]
    async fn sqlview_dependency_with_parameters_is_rejected() {
        let bad = json!({
            "resourceType": "Library",
            "url": "http://example.org/bad",
            "status": "active",
            "type": {"coding": [{
                "system": helios_sof::canonical::LIBRARY_TYPES_CODE_SYSTEM,
                "code": "sql-view"
            }]},
            "content": [{"contentType": "application/sql", "data": "U0VMRUNUIDE="}],
            "parameter": [{"name": "p", "use": "in", "type": "string"}]
        });
        let fetcher = MapFetcher(HashMap::from([("http://example.org/bad".to_string(), bad)]));
        let deps = vec![depends_on("t", "http://example.org/bad")];
        let errors = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect_err("a parameterized SQLView must be rejected");
        assert_eq!(errors.len(), 1);
        let RestError::BadRequest { message } = &errors[0] else {
            panic!("expected a 400, got {:?}", errors[0]);
        };
        assert!(message.contains("parameter"), "{message}");
    }

    #[tokio::test]
    async fn unresolved_url_and_cycle_are_both_reported_together() {
        let fetcher = MapFetcher(HashMap::from([(
            "http://example.org/a".to_string(),
            sql_view(
                "http://example.org/a",
                "SELECT * FROM a_t",
                &[("a_t", "http://example.org/a")],
            ),
        )]));

        let deps = vec![
            depends_on("a_t", "http://example.org/a"),
            depends_on("missing_t", "http://example.org/missing"),
        ];
        let errors = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect_err("both problems must be reported");

        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(
            |e| matches!(e, RestError::NotFound { id, .. } if id == "http://example.org/missing")
        ));
        assert!(
            errors.iter().any(
                |e| matches!(e, RestError::BadRequest { message } if message.contains("cycle"))
            )
        );
    }

    #[tokio::test]
    async fn label_collision_across_different_targets_is_rejected() {
        // Two different SQLView nodes, one referenced by a label another
        // node has already bound to a *different* target.
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/a".to_string(),
                view_definition("http://example.org/a"),
            ),
            (
                "http://example.org/b".to_string(),
                view_definition("http://example.org/b"),
            ),
        ]));
        let deps = vec![
            depends_on("t", "http://example.org/a"),
            depends_on("t", "http://example.org/b"),
        ];
        // `parse_sqlquery_library` would already reject a literal duplicate
        // label within one Library's own relatedArtifact list, so this
        // scenario is exercised directly against the resolver instead.
        let errors = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect_err("same label, different targets, must be rejected");
        assert!(
            errors.iter().any(
                |e| matches!(e, RestError::BadRequest { message } if message.contains("label"))
            )
        );
    }

    #[tokio::test]
    async fn inline_context_artifact_satisfies_a_dependency_the_fetcher_cannot_resolve() {
        let fetcher = MapFetcher(HashMap::new());
        let inline = vec![view_definition("http://example.org/inline-leaf")];
        let deps = vec![depends_on("t", "http://example.org/inline-leaf")];
        let plan = build_plan(&fetcher, &inline, subject_node(&deps))
            .await
            .expect("inline artifact should resolve when the fetcher has nothing at that url");
        assert_eq!(plan.node_count(), 1);
    }

    /// Design #568: `context` fills gaps the server cannot resolve; it never
    /// overrides an artifact the server already has. The fetcher and the
    /// inline list carry distinguishable artifacts at the same URL — the
    /// fetcher's copy must be the one that lands in the plan.
    #[tokio::test]
    async fn server_storage_wins_over_a_context_entry_for_the_same_url() {
        let url = "http://example.org/dup";
        let mut server_vd = view_definition(url);
        server_vd["name"] = json!("from-storage");
        let mut context_vd = view_definition(url);
        context_vd["name"] = json!("from-context");

        let fetcher = MapFetcher(HashMap::from([(url.to_string(), server_vd)]));
        let inline = vec![context_vd];
        let deps = vec![depends_on("t", url)];
        let plan = build_plan(&fetcher, &inline, subject_node(&deps))
            .await
            .expect("plan should resolve");

        assert_eq!(plan.node_count(), 1);
        let PlanNode::Leaf { view, .. } = &plan.nodes[0] else {
            panic!("expected a leaf node");
        };
        assert_eq!(view["name"], json!("from-storage"));
    }

    /// A `context` entry can satisfy a dependency reached at any depth, not
    /// only the subject's direct ones: here the subject's direct dependency
    /// ("mid") comes from storage, but "mid"'s own dependency ("inner") is
    /// only available inline.
    #[tokio::test]
    async fn context_entry_satisfies_a_dependency_reached_through_an_intermediate_sqlview() {
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/mid".to_string(),
                sql_view(
                    "http://example.org/mid",
                    "SELECT * FROM inner_t",
                    &[("inner_t", "http://example.org/inner")],
                ),
            ),
            (
                "http://example.org/leaf".to_string(),
                view_definition("http://example.org/leaf"),
            ),
        ]));
        let inline = vec![sql_view(
            "http://example.org/inner",
            "SELECT * FROM leaf_t",
            &[("leaf_t", "http://example.org/leaf")],
        )];

        let deps = vec![depends_on("mid_t", "http://example.org/mid")];
        let plan = build_plan(&fetcher, &inline, subject_node(&deps))
            .await
            .expect("plan should resolve via the context-supplied intermediate SQLView");

        assert_eq!(plan.node_count(), 3, "leaf + 2 SQLView nodes");
    }

    /// An interior SQLView's SQL is validated SELECT-only exactly like the
    /// subject's own SQL, regardless of whether the Library came from
    /// storage or `context` — the resolver does not branch by origin.
    #[tokio::test]
    async fn interior_sqlview_with_non_select_sql_is_rejected() {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(b"DELETE FROM t" as &[u8]);
        let bad = json!({
            "resourceType": "Library",
            "url": "http://example.org/bad-sql",
            "status": "active",
            "type": {"coding": [{
                "system": helios_sof::canonical::LIBRARY_TYPES_CODE_SYSTEM,
                "code": "sql-view"
            }]},
            "content": [{"contentType": "application/sql", "data": data}]
        });
        let fetcher = MapFetcher(HashMap::from([(
            "http://example.org/bad-sql".to_string(),
            bad,
        )]));
        let deps = vec![depends_on("t", "http://example.org/bad-sql")];
        let errors = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect_err("a non-SELECT interior SQLView must be rejected");
        assert_eq!(errors.len(), 1);
        let RestError::BadRequest { message } = &errors[0] else {
            panic!("expected a 400, got {:?}", errors[0]);
        };
        assert!(message.to_uppercase().contains("SELECT"), "{message}");
    }

    #[tokio::test]
    async fn max_nodes_cap_counts_the_whole_plan() {
        let fetcher = MapFetcher(HashMap::from([
            (
                "http://example.org/a".to_string(),
                view_definition("http://example.org/a"),
            ),
            (
                "http://example.org/b".to_string(),
                view_definition("http://example.org/b"),
            ),
        ]));
        let deps = vec![
            depends_on("a_t", "http://example.org/a"),
            depends_on("b_t", "http://example.org/b"),
        ];
        let plan = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect("plan should resolve");
        assert!(check_max_nodes(&plan, 2).is_ok());
        assert!(check_max_nodes(&plan, 1).is_err());
    }

    /// A SQLView dependency is named by its Library `name`, falling back to
    /// its canonical URL; the name is what a row-cap error reports (#1473).
    #[tokio::test]
    async fn sqlview_node_records_its_name_or_url() {
        let named_url = "http://example.org/named-view";
        let unnamed_url = "http://example.org/unnamed-view";
        let leaf_url = "http://example.org/leaf";
        let mut named = sql_view(named_url, "SELECT * FROM l", &[("l", leaf_url)]);
        named["name"] = json!("female_patients");
        let unnamed = sql_view(unnamed_url, "SELECT * FROM l", &[("l", leaf_url)]);
        let fetcher = MapFetcher(HashMap::from([
            (leaf_url.to_string(), view_definition(leaf_url)),
            (named_url.to_string(), named),
            (unnamed_url.to_string(), unnamed),
        ]));
        let deps = vec![
            depends_on("named_t", named_url),
            depends_on("unnamed_t", unnamed_url),
        ];
        let plan = build_plan(&fetcher, &[], subject_node(&deps))
            .await
            .expect("plan should resolve");
        let names: Vec<&str> = plan
            .nodes
            .iter()
            .filter_map(|n| match n {
                PlanNode::SqlView { name, .. } => Some(name.as_str()),
                PlanNode::Leaf { .. } => None,
            })
            .collect();
        assert_eq!(names, vec!["female_patients", unnamed_url]);
    }

    // ------------------------------------------------------------------
    // Phase 2 — the per-dependency row cap (#1473). Runs `execute_plan`
    // against a stub `SofRunner`; no HTTP and no storage.
    // ------------------------------------------------------------------

    /// A `SofRunner` that yields `rows` rows of `{"id": "p<i>"}`, then ends.
    struct RowsRunner {
        rows: usize,
    }

    #[async_trait]
    impl SofRunner for RowsRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let rows = (0..self.rows).map(|i| Ok(json!({"id": format!("p{i}")})));
            Ok(Box::pin(futures::stream::iter(rows)))
        }

        fn runner_name(&self) -> &'static str {
            "rows-test-runner"
        }
    }

    fn named_view_definition(name: &str) -> Value {
        let mut view = view_definition(&format!("http://example.org/{name}"));
        view["name"] = json!(name);
        view
    }

    fn edge(label: &str, target: &str) -> Edge {
        Edge {
            label: label.to_string(),
            target_internal_name: target.to_string(),
        }
    }

    async fn run_plan(
        runner_rows: usize,
        limits: ExecLimits,
        plan: &GraphPlan,
        subject_sql: &str,
    ) -> Result<(QueryResult, Vec<TableSchema>), SqlQueryError> {
        let runner: Arc<dyn SofRunner> = Arc::new(RowsRunner { rows: runner_rows });
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        execute_plan(
            InMemorySqlEngine::open().expect("open engine"),
            &runner,
            &tenant,
            &ViewFilters::default(),
            plan,
            subject_sql,
            &[],
            limits,
        )
        .await
    }

    fn limits(max_source_rows_per_vd: usize, max_rows: usize) -> ExecLimits {
        ExecLimits {
            max_source_rows_per_vd,
            max_rows,
            timeout_secs: 5,
        }
    }

    fn leaf_plan(label_edges: Vec<Edge>) -> GraphPlan {
        GraphPlan {
            nodes: vec![PlanNode::Leaf {
                internal_name: "__sof_node_0".to_string(),
                view: named_view_definition("observation_flat"),
            }],
            subject_edges: label_edges,
        }
    }

    #[tokio::test]
    async fn leaf_dependency_over_the_cap_names_label_view_cap_and_setting() {
        let plan = leaf_plan(vec![edge("obs", "__sof_node_0")]);
        let err = run_plan(10, limits(3, 1000), &plan, "SELECT * FROM obs")
            .await
            .err()
            .expect("a 10-row dependency must exceed a cap of 3");
        let SqlQueryError::DependencyRowCapExceeded {
            label,
            kind,
            view,
            max,
        } = &err
        else {
            panic!("expected DependencyRowCapExceeded, got {err:?}");
        };
        assert_eq!(label, "obs");
        assert_eq!(*kind, DependencyKind::ViewDefinition);
        assert_eq!(view, "observation_flat");
        assert_eq!(*max, 3);

        let text = err.to_string();
        for needle in [
            "dependency 'obs'",
            "(ViewDefinition observation_flat)",
            "more than 3 rows",
            "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in: {text}");
        }
    }

    /// The cap is inclusive: a dependency of exactly `max` rows is fine.
    #[tokio::test]
    async fn leaf_dependency_at_the_cap_still_runs() {
        let plan = leaf_plan(vec![edge("obs", "__sof_node_0")]);
        let (result, _) = run_plan(3, limits(3, 1000), &plan, "SELECT * FROM obs")
            .await
            .expect("a dependency of exactly the cap must run");
        assert_eq!(result.rows.len(), 3);
    }

    /// A node reached under two labels is named by the alphabetically first,
    /// so the message does not depend on hash order.
    #[tokio::test]
    async fn dependency_with_two_labels_is_named_by_the_first_alphabetically() {
        let plan = leaf_plan(vec![
            edge("zeta", "__sof_node_0"),
            edge("alpha", "__sof_node_0"),
        ]);
        let err = run_plan(10, limits(3, 1000), &plan, "SELECT * FROM alpha")
            .await
            .err()
            .expect("must exceed the cap");
        assert!(
            matches!(&err, SqlQueryError::DependencyRowCapExceeded { label, .. } if label == "alpha"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn sqlview_dependency_over_the_cap_names_label_view_cap_and_setting() {
        // `patients` yields 3 rows (within a cap of 5); the SQLView built on
        // it cross-joins them with themselves into 9 rows (over it).
        let plan = GraphPlan {
            nodes: vec![
                PlanNode::Leaf {
                    internal_name: "__sof_node_0".to_string(),
                    view: named_view_definition("patients"),
                },
                PlanNode::SqlView {
                    internal_name: "__sof_node_1".to_string(),
                    sql: "SELECT a.id FROM base a, base b".to_string(),
                    edges: vec![edge("base", "__sof_node_0")],
                    name: "pairs".to_string(),
                },
            ],
            subject_edges: vec![edge("pairs_t", "__sof_node_1")],
        };
        let err = run_plan(3, limits(5, 1000), &plan, "SELECT * FROM pairs_t")
            .await
            .err()
            .expect("the 9-row SQLView must exceed a cap of 5");
        let SqlQueryError::DependencyRowCapExceeded {
            label,
            kind,
            view,
            max,
        } = &err
        else {
            panic!("expected DependencyRowCapExceeded, got {err:?}");
        };
        assert_eq!(label, "pairs_t");
        assert_eq!(*kind, DependencyKind::SqlView);
        assert_eq!(view, "pairs");
        assert_eq!(*max, 5);

        let text = err.to_string();
        for needle in [
            "dependency 'pairs_t'",
            "(SQL View pairs)",
            "more than 5 rows",
            "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in: {text}");
        }
        assert!(!text.contains("(ViewDefinition pairs)"), "{text}");
    }

    /// The subject's own result is not a dependency: it is silently
    /// truncated at `max_rows` (SoF v2 PR #353), never an error.
    #[tokio::test]
    async fn subject_result_over_max_rows_is_truncated_not_an_error() {
        let plan = leaf_plan(vec![edge("obs", "__sof_node_0")]);
        let (result, _) = run_plan(10, limits(100, 4), &plan, "SELECT * FROM obs")
            .await
            .expect("the subject's own cap truncates");
        assert_eq!(result.rows.len(), 4);
    }
}
